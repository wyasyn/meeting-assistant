//! Plays a stretch of a recorded track back (FR-3.8, ADR-022): decrypts the chunks it
//! covers, decodes them to 16 kHz mono and returns a WAV file in memory. Plain audio is
//! never written to disk (ADR-021).

use std::io::Cursor;

use ogg::PacketReader;
use opus::{Channels, Decoder};

use super::recorder::{read_chunk, RecordingTarget};
use super::{CaptureError, Track, SAMPLE_RATE};

/// Chunks are wideband (ADR-014), so 16 kHz loses nothing and is a third of 48 kHz.
pub const PLAYBACK_RATE: u32 = 16_000;
/// Longest stretch returned at once; longer ranges are cut.
pub const MAX_PLAYBACK_MS: i64 = 10 * 60 * 1_000;
const CHUNK_MS: i64 = 10_000;
const SAMPLES_PER_MS: i64 = PLAYBACK_RATE as i64 / 1_000;
/// Ogg Opus granule positions count 48 kHz samples (RFC 7845 section 4).
const GRANULE_PER_SAMPLE: u64 = (SAMPLE_RATE / PLAYBACK_RATE) as u64;
/// 120 ms, the longest Opus packet, at 16 kHz.
const MAX_PACKET_SAMPLES: usize = 1_920;
const WAV_HEADER: usize = 44;

/// `[start_ms, end_ms)` of `track` as a 16-bit mono WAV. Missing chunks play as silence.
pub fn track_wav(
    target: &RecordingTarget,
    track: Track,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<u8>, CaptureError> {
    let start_ms = start_ms.max(0);
    let end_ms = end_ms.clamp(start_ms, start_ms + MAX_PLAYBACK_MS);
    let len = usize::try_from((end_ms - start_ms) * SAMPLES_PER_MS).unwrap_or(0);
    let mut out = vec![0f32; len];
    if len > 0 {
        let first = start_ms / CHUNK_MS + 1;
        let last = (end_ms - 1) / CHUNK_MS + 1;
        for index in first..=last {
            let index = u32::try_from(index).map_err(|e| CaptureError::Storage(e.to_string()))?;
            if !target.chunk_path(track, index).exists() {
                continue;
            }
            let samples = decode(&read_chunk(target, track, index)?)?;
            // Position of the chunk's first sample in `out`; negative when it starts earlier.
            let offset = (i64::from(index - 1) * CHUNK_MS - start_ms) * SAMPLES_PER_MS;
            for (i, sample) in samples.into_iter().enumerate() {
                let Ok(at) = usize::try_from(offset + i as i64) else {
                    continue;
                };
                match out.get_mut(at) {
                    Some(slot) => *slot = sample,
                    None => break,
                }
            }
        }
    }
    Ok(wav(&out))
}

/// One chunk at 16 kHz: pre-skip dropped, trimmed to the last granule position.
fn decode(ogg: &[u8]) -> Result<Vec<f32>, CaptureError> {
    let damaged = |detail: String| CaptureError::Storage(format!("damaged chunk: {detail}"));
    let mut reader = PacketReader::new(Cursor::new(ogg));
    let head = reader
        .read_packet()
        .map_err(|e| damaged(e.to_string()))?
        .ok_or_else(|| damaged("empty".into()))?;
    if head.data.len() < 12 || &head.data[..8] != b"OpusHead" {
        return Err(damaged("no OpusHead".into()));
    }
    let pre_skip = u64::from(u16::from_le_bytes([head.data[10], head.data[11]]));
    // OpusTags.
    reader.read_packet().map_err(|e| damaged(e.to_string()))?;

    let mut decoder =
        Decoder::new(PLAYBACK_RATE, Channels::Mono).map_err(|e| damaged(e.to_string()))?;
    let mut out = Vec::new();
    let mut buf = [0f32; MAX_PACKET_SAMPLES];
    let mut end = None;
    while let Some(packet) = reader.read_packet().map_err(|e| damaged(e.to_string()))? {
        let n = decoder
            .decode_float(&packet.data, &mut buf, false)
            .map_err(|e| damaged(e.to_string()))?;
        out.extend_from_slice(&buf[..n]);
        if packet.last_in_stream() {
            end = Some(packet.absgp_page());
        }
    }
    if let Some(end) = end {
        out.truncate(usize::try_from(end / GRANULE_PER_SAMPLE).unwrap_or(usize::MAX));
    }
    let skip = usize::try_from(pre_skip / GRANULE_PER_SAMPLE).unwrap_or(0);
    Ok(out.split_off(skip.min(out.len())))
}

/// 16-bit PCM mono WAV at `PLAYBACK_RATE`.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(WAV_HEADER + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&PLAYBACK_RATE.to_le_bytes());
    out.extend_from_slice(&(PLAYBACK_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        // Saturating float-to-int cast keeps clipped samples at full scale.
        out.extend_from_slice(&((sample * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::encoder::ChunkEncoder;
    use crate::capture::recorder::tests::{target, TempDir};
    use crate::capture::AudioFrame;
    use crate::store::crypto::{chunk_aad, encrypt_chunk};

    /// Seals `seconds` of a tone as the recorder would, skipping the chunks in `missing`.
    fn record(target: &RecordingTarget, seconds: u64, missing: &[u32]) {
        let mut enc = ChunkEncoder::new(7).unwrap();
        let samples = 480;
        let mut chunks = Vec::new();
        for i in 0..seconds * 100 {
            let frame = AudioFrame {
                track: Track::Mic,
                offset_samples: i * samples,
                samples: (0..samples)
                    .map(|n| {
                        let t = (i * samples + n) as f32 / SAMPLE_RATE as f32;
                        0.3 * (t * 440.0 * std::f32::consts::TAU).sin()
                    })
                    .collect(),
            };
            chunks.extend(enc.push(&frame).unwrap());
        }
        chunks.extend(enc.finish().unwrap());
        std::fs::create_dir_all(target.dir.join("mic")).unwrap();
        for chunk in chunks.into_iter().filter(|c| !missing.contains(&c.index)) {
            let aad = chunk_aad(&target.meeting_id, "mic", chunk.index);
            let sealed = encrypt_chunk(&target.key, &aad, &chunk.ogg).unwrap();
            std::fs::write(target.chunk_path(Track::Mic, chunk.index), sealed).unwrap();
        }
    }

    fn pcm(wav: &[u8]) -> Vec<i16> {
        wav[WAV_HEADER..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect()
    }

    fn loud(samples: &[i16]) -> bool {
        samples.iter().any(|s| s.unsigned_abs() > 3_000)
    }

    #[test]
    fn fr_3_8_plays_a_range_across_a_chunk_boundary() {
        let dir = TempDir::new("playback-boundary");
        let target = target(&dir.0);
        record(&target, 25, &[]);

        let wav = track_wav(&target, Track::Mic, 9_000, 11_500).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        let samples = pcm(&wav);
        assert_eq!(samples.len(), 2_500 * 16);
        assert_eq!(
            u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
            samples.len() * 2
        );
        // Sound on both sides of the 10 s boundary.
        assert!(loud(&samples[..16_000]));
        assert!(loud(&samples[16_000..]));
    }

    #[test]
    fn missing_chunks_and_the_end_of_the_track_are_silence() {
        let dir = TempDir::new("playback-gaps");
        let target = target(&dir.0);
        record(&target, 25, &[2]);

        let samples = pcm(&track_wav(&target, Track::Mic, 9_000, 21_000).unwrap());
        assert_eq!(samples.len(), 12_000 * 16);
        assert!(loud(&samples[..16_000]));
        assert!(!loud(&samples[16_000..176_000]));
        assert!(loud(&samples[176_000..]));

        // Past the 25 s of audio, and a reversed range.
        assert!(!loud(&pcm(
            &track_wav(&target, Track::Mic, 26_000, 27_000).unwrap()
        )));
        assert!(pcm(&track_wav(&target, Track::Mic, 5_000, 4_000).unwrap()).is_empty());
        // Ranges are capped.
        let long = track_wav(&target, Track::Mic, 0, MAX_PLAYBACK_MS * 2).unwrap();
        assert_eq!(
            pcm(&long).len(),
            (MAX_PLAYBACK_MS * SAMPLES_PER_MS) as usize
        );
    }

    #[test]
    fn a_damaged_chunk_is_an_error() {
        let dir = TempDir::new("playback-damaged");
        let target = target(&dir.0);
        std::fs::create_dir_all(target.dir.join("mic")).unwrap();
        std::fs::write(target.chunk_path(Track::Mic, 1), b"MAC1 not really").unwrap();
        assert!(track_wav(&target, Track::Mic, 0, 1_000).is_err());
    }
}
