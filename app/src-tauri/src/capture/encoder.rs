//! Splits one track into 10 s chunks and encodes each as a self-contained Ogg Opus stream
//! (FR-2.4, NFR-6, ADR-014). Chunk N (from 1) holds track samples [(N-1) x 10 s, N x 10 s).

use std::io::Cursor;

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use opus::{Application, Bandwidth, Bitrate, Channels, Encoder};

use super::{AudioFrame, CaptureError, SAMPLE_RATE};

/// 10 s at 48 kHz.
pub const CHUNK_SAMPLES: u64 = SAMPLE_RATE as u64 * 10;
/// 20 ms at 48 kHz. `CHUNK_SAMPLES` is an exact multiple, so frames never straddle chunks.
const FRAME_SAMPLES: usize = 960;
const FRAMES_PER_CHUNK: usize = (CHUNK_SAMPLES / FRAME_SAMPLES as u64) as usize;
/// Speech at wideband (16 kHz audio, docs/03) needs no more.
const BITRATE: i32 = 24_000;
const MAX_PACKET: usize = 4_000;
/// End an Ogg page about once a second so players can seek.
const PACKETS_PER_PAGE: usize = 50;
const VENDOR: &[u8] = b"meeting-assistant";

/// One finished chunk, not yet encrypted.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedChunk {
    /// 1-based, matches the file name `000001.opus.enc`.
    pub index: u32,
    /// Real samples in the chunk; the last chunk of a recording is usually short.
    pub samples: u64,
    pub ogg: Vec<u8>,
}

impl EncodedChunk {
    pub fn start_ms(&self) -> u64 {
        u64::from(self.index - 1) * CHUNK_SAMPLES * 1_000 / u64::from(SAMPLE_RATE)
    }
}

pub struct ChunkEncoder {
    encoder: Encoder,
    pre_skip: u16,
    serial: u32,
    /// Track position of the next sample to be buffered.
    next_sample: u64,
    pending: Vec<f32>,
    packets: Vec<Vec<u8>>,
    index: u32,
}

impl ChunkEncoder {
    pub fn new(serial: u32) -> Result<Self, CaptureError> {
        let mut encoder =
            Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip).map_err(opus_error)?;
        encoder
            .set_max_bandwidth(Bandwidth::Wideband)
            .map_err(opus_error)?;
        encoder
            .set_bitrate(Bitrate::Bits(BITRATE))
            .map_err(opus_error)?;
        encoder.set_dtx(true).map_err(opus_error)?;
        let lookahead = encoder.get_lookahead().map_err(opus_error)?;
        Ok(Self {
            encoder,
            pre_skip: u16::try_from(lookahead).unwrap_or(0),
            serial,
            next_sample: 0,
            pending: Vec::with_capacity(FRAME_SAMPLES),
            packets: Vec::with_capacity(FRAMES_PER_CHUNK + 1),
            index: 1,
        })
    }

    /// Adds a frame at its offset, filling any gap before it with silence.
    /// Returns the chunks it completed.
    pub fn push(&mut self, frame: &AudioFrame) -> Result<Vec<EncodedChunk>, CaptureError> {
        let mut done = Vec::new();
        if frame.offset_samples > self.next_sample {
            let mut gap = frame.offset_samples - self.next_sample;
            let silence = [0f32; FRAME_SAMPLES];
            while gap > 0 {
                let n = gap.min(FRAME_SAMPLES as u64) as usize;
                self.feed(&silence[..n], &mut done)?;
                gap -= n as u64;
            }
        }
        // Offsets never go backwards (TrackClock); drop any overlap defensively.
        let skip = usize::try_from(self.next_sample.saturating_sub(frame.offset_samples))
            .unwrap_or(usize::MAX)
            .min(frame.samples.len());
        self.feed(&frame.samples[skip..], &mut done)?;
        Ok(done)
    }

    /// Track position of the next sample: everything before it is buffered or saved.
    pub fn position(&self) -> u64 {
        self.next_sample
    }

    /// Encodes what is buffered as the last, short chunk. `None` when nothing is buffered.
    pub fn finish(&mut self) -> Result<Option<EncodedChunk>, CaptureError> {
        if self.pending.is_empty() && self.packets.is_empty() {
            return Ok(None);
        }
        let real = (self.packets.len() * FRAME_SAMPLES + self.pending.len()) as u64;
        if !self.pending.is_empty() {
            self.pending.resize(FRAME_SAMPLES, 0.0);
            self.encode_pending()?;
        }
        self.seal(real).map(Some)
    }

    fn feed(
        &mut self,
        mut samples: &[f32],
        done: &mut Vec<EncodedChunk>,
    ) -> Result<(), CaptureError> {
        while !samples.is_empty() {
            let n = (FRAME_SAMPLES - self.pending.len()).min(samples.len());
            self.pending.extend_from_slice(&samples[..n]);
            self.next_sample += n as u64;
            samples = &samples[n..];
            if self.pending.len() == FRAME_SAMPLES {
                self.encode_pending()?;
                if self.packets.len() == FRAMES_PER_CHUNK {
                    done.push(self.seal(CHUNK_SAMPLES)?);
                }
            }
        }
        Ok(())
    }

    fn encode_pending(&mut self) -> Result<(), CaptureError> {
        let packet = self
            .encoder
            .encode_vec_float(&self.pending, MAX_PACKET)
            .map_err(opus_error)?;
        self.packets.push(packet);
        self.pending.clear();
        Ok(())
    }

    /// Flushes the encoder's lookahead with one frame of silence, writes the Ogg stream
    /// and resets for the next chunk, so every chunk decodes on its own.
    fn seal(&mut self, real_samples: u64) -> Result<EncodedChunk, CaptureError> {
        let flush = self
            .encoder
            .encode_vec_float(&[0f32; FRAME_SAMPLES], MAX_PACKET)
            .map_err(opus_error)?;
        let mut packets = std::mem::take(&mut self.packets);
        packets.push(flush);
        let ogg = write_ogg(
            self.serial.wrapping_add(self.index),
            self.pre_skip,
            &packets,
            real_samples,
        )
        .map_err(|e| CaptureError::Encode(e.to_string()))?;
        self.encoder.reset_state().map_err(opus_error)?;
        let chunk = EncodedChunk {
            index: self.index,
            samples: real_samples,
            ogg,
        };
        self.index += 1;
        self.packets = Vec::with_capacity(FRAMES_PER_CHUNK + 1);
        Ok(chunk)
    }
}

/// Real samples in an encoded chunk: the last granule position minus pre-skip.
/// `None` when the bytes are not an Ogg Opus stream.
pub fn chunk_samples(ogg: &[u8]) -> Option<u64> {
    let mut reader = ogg::PacketReader::new(Cursor::new(ogg));
    let head = reader.read_packet().ok()??;
    if head.data.len() < 12 || &head.data[..8] != b"OpusHead" {
        return None;
    }
    let pre_skip = u64::from(u16::from_le_bytes([head.data[10], head.data[11]]));
    let mut granule = None;
    while let Ok(Some(packet)) = reader.read_packet() {
        if packet.last_in_page() {
            granule = Some(packet.absgp_page());
        }
    }
    granule.map(|g| g.saturating_sub(pre_skip))
}

fn opus_error(err: opus::Error) -> CaptureError {
    CaptureError::Encode(err.to_string())
}

/// OpusHead (RFC 7845 section 5.1): mono, 48 kHz input, no gain, mapping family 0.
fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut head = b"OpusHead".to_vec();
    head.push(1);
    head.push(1);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes());
    head.push(0);
    head
}

/// OpusTags with the vendor string and no comments.
fn opus_tags() -> Vec<u8> {
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    tags.extend_from_slice(VENDOR);
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags
}

/// The last page's granule position is `pre_skip + real_samples`, which tells players to
/// drop the padding and the flush frame (RFC 7845 section 4.4).
fn write_ogg(
    serial: u32,
    pre_skip: u16,
    packets: &[Vec<u8>],
    real_samples: u64,
) -> std::io::Result<Vec<u8>> {
    let mut writer = PacketWriter::new(Cursor::new(Vec::new()));
    writer.write_packet(opus_head(pre_skip), serial, PacketWriteEndInfo::EndPage, 0)?;
    writer.write_packet(opus_tags(), serial, PacketWriteEndInfo::EndPage, 0)?;
    let last = packets.len().saturating_sub(1);
    for (i, packet) in packets.iter().enumerate() {
        let (end, granule) = if i == last {
            (
                PacketWriteEndInfo::EndStream,
                u64::from(pre_skip) + real_samples,
            )
        } else if (i + 1) % PACKETS_PER_PAGE == 0 {
            (
                PacketWriteEndInfo::EndPage,
                ((i + 1) * FRAME_SAMPLES) as u64,
            )
        } else {
            (
                PacketWriteEndInfo::NormalPacket,
                ((i + 1) * FRAME_SAMPLES) as u64,
            )
        };
        writer.write_packet(packet.clone(), serial, end, granule)?;
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
pub mod tests {
    use ogg::PacketReader;

    use super::*;
    use crate::capture::Track;

    /// Decodes a chunk the way a player would: skip pre-skip, trim to the last granule.
    pub fn decode(ogg: &[u8]) -> Vec<f32> {
        let mut reader = PacketReader::new(Cursor::new(ogg));
        let head = reader.read_packet_expected().unwrap();
        assert_eq!(&head.data[..8], b"OpusHead");
        let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as usize;
        let tags = reader.read_packet_expected().unwrap();
        assert_eq!(&tags.data[..8], b"OpusTags");

        let mut decoder = opus::Decoder::new(SAMPLE_RATE, Channels::Mono).unwrap();
        let mut out = Vec::new();
        let mut last_granule = 0;
        while let Some(packet) = reader.read_packet().unwrap() {
            let mut buf = [0f32; 5_760];
            let n = decoder.decode_float(&packet.data, &mut buf, false).unwrap();
            out.extend_from_slice(&buf[..n]);
            if packet.last_in_stream() {
                last_granule = packet.absgp_page() as usize;
            }
        }
        out.truncate(last_granule);
        out.split_off(pre_skip)
    }

    fn frame(offset: u64, samples: Vec<f32>) -> AudioFrame {
        AudioFrame {
            track: Track::Mic,
            offset_samples: offset,
            samples,
        }
    }

    fn tone(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| 0.3 * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn fr_2_4_splits_at_exactly_ten_seconds() {
        let mut enc = ChunkEncoder::new(1).unwrap();
        let mut chunks = Vec::new();
        // 25 s in 10 ms frames.
        for i in 0..2_500u64 {
            chunks.extend(enc.push(&frame(i * 480, tone(480))).unwrap());
        }
        assert_eq!(chunks.len(), 2);
        chunks.extend(enc.finish().unwrap());
        assert_eq!(
            chunks
                .iter()
                .map(|c| (c.index, c.samples))
                .collect::<Vec<_>>(),
            vec![
                (1, CHUNK_SAMPLES),
                (2, CHUNK_SAMPLES),
                (3, CHUNK_SAMPLES / 2)
            ]
        );
        assert_eq!(chunks[2].start_ms(), 20_000);
        for chunk in &chunks {
            assert_eq!(decode(&chunk.ogg).len() as u64, chunk.samples);
            assert_eq!(chunk_samples(&chunk.ogg), Some(chunk.samples));
        }
        assert_eq!(chunk_samples(b"not ogg"), None);
        assert_eq!(enc.finish().unwrap(), None);
    }

    #[test]
    fn nfr_6_gaps_are_filled_with_silence_to_keep_the_timeline() {
        let mut enc = ChunkEncoder::new(1).unwrap();
        enc.push(&frame(0, tone(4_800))).unwrap();
        // 12 s gap (a Bluetooth stall), then 0.1 s of audio.
        let done = enc.push(&frame(4_800 + 12 * 48_000, tone(4_800))).unwrap();
        assert_eq!(done.len(), 1);
        let last = enc.finish().unwrap().unwrap();
        assert_eq!(last.index, 2);
        assert_eq!(last.samples, 4_800 + 12 * 48_000 + 4_800 - CHUNK_SAMPLES);
        let decoded = decode(&last.ogg);
        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
        assert!(rms(&decoded[..48_000]) < 0.01, "gap is not silent");
        assert!(
            rms(&decoded[decoded.len() - 2_400..]) > 0.05,
            "tone missing"
        );
    }

    #[test]
    fn overlapping_frames_do_not_duplicate_samples() {
        let mut enc = ChunkEncoder::new(1).unwrap();
        enc.push(&frame(0, tone(960))).unwrap();
        enc.push(&frame(480, tone(960))).unwrap();
        assert_eq!(enc.finish().unwrap().unwrap().samples, 1_440);
    }

    #[test]
    fn opus_head_matches_rfc_7845() {
        let head = opus_head(312);
        assert_eq!(head.len(), 19);
        assert_eq!(&head[..8], b"OpusHead");
        assert_eq!((head[8], head[9]), (1, 1));
        assert_eq!(u16::from_le_bytes([head[10], head[11]]), 312);
        assert_eq!(
            u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
            48_000
        );
    }
}
