//! Audio capture per OS behind the `AudioBackend` trait (FR-2.1, FR-2.3, rule 10).
//! Every backend delivers two tracks, mic and system output, as 48 kHz mono f32 frames.

pub mod encoder;
#[cfg(test)]
pub mod fake;
#[cfg(target_os = "linux")]
mod pipewire;
pub mod recorder;
pub mod recovery;

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;

use crate::error::AppError;

pub const SAMPLE_RATE: u32 = 48_000;

/// Frames buffered between the capture thread and the consumer (about 20 s at 10 ms buffers).
pub const FRAME_QUEUE: usize = 2_048;

/// Which side of the call a frame belongs to. Mic is always the user (FR-2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Track {
    Mic,
    Sys,
}

impl Track {
    /// Folder name and `segments.track` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::Sys => "sys",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Input,
    Output,
}

/// Shape for `list_audio_devices` (wired up in roadmap 1.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// Backend id; on Linux the PipeWire `node.name`.
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub is_default: bool,
}

/// `None` follows the system default, including when it changes mid-capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureConfig {
    pub mic: Option<String>,
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrame {
    pub track: Track,
    /// Position of the first sample since the session started, per track.
    pub offset_samples: u64,
    pub samples: Vec<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("audio capture is not supported on this system yet")]
    Unsupported,
    #[error("could not connect to the audio server: {0}")]
    Connect(String),
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("audio stream failed: {0}")]
    Stream(String),
    #[error("opus encoding failed: {0}")]
    Encode(String),
    #[error("could not save audio: {0}")]
    Storage(String),
}

impl From<CaptureError> for AppError {
    fn from(err: CaptureError) -> Self {
        tracing::debug!(error = %err, "capture error");
        let message = match err {
            CaptureError::Unsupported => "Recording is not supported on this system yet.",
            CaptureError::Connect(_) => {
                "Could not reach the system audio service. Check that PipeWire is running."
            }
            CaptureError::DeviceNotFound(_) => {
                "The selected audio device is not available. Choose another one in settings."
            }
            CaptureError::Stream(_) | CaptureError::Encode(_) => {
                "Audio capture stopped unexpectedly."
            }
            CaptureError::Storage(_) => {
                "Could not save the recording. Check that the disk has free space."
            }
        };
        AppError::AudioDevice(message.to_owned())
    }
}

pub trait AudioBackend: Send + Sync {
    fn list_devices(&self) -> Result<Vec<AudioDevice>, CaptureError>;
    fn start(&self, config: CaptureConfig) -> Result<CaptureSession, CaptureError>;
}

/// A running capture. Frames arrive on `recv`; `stop` (or drop) ends it.
pub struct CaptureSession {
    frames: Receiver<AudioFrame>,
    stop: Option<Box<dyn FnOnce() + Send>>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureSession {
    pub fn new(
        frames: Receiver<AudioFrame>,
        stop: Box<dyn FnOnce() + Send>,
        thread: Option<JoinHandle<()>>,
    ) -> Self {
        Self {
            frames,
            stop: Some(stop),
            thread,
        }
    }

    /// Next frame, `Ok(None)` on timeout, `Err` once capture has ended and the queue is empty.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<AudioFrame>, CaptureError> {
        match self.frames.recv_timeout(timeout) {
            Ok(frame) => Ok(Some(frame)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(CaptureError::Stream("capture ended".into()))
            }
        }
    }

    /// Stops capture and waits for the capture thread. Frames already queued stay readable.
    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                tracing::warn!("capture thread panicked");
            }
        }
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Gaps shorter than this are buffer jitter, not lost audio (100 ms).
pub const GAP_TOLERANCE_SAMPLES: u64 = SAMPLE_RATE as u64 / 10;

/// Gives each frame its offset since the session start, per track.
/// Offsets follow the sample count, and jump forward to wall-clock time when a device
/// stops delivering for longer than `GAP_TOLERANCE_SAMPLES` (Bluetooth does; ADR-013),
/// so mic and system tracks stay aligned.
#[derive(Debug, Default)]
pub struct TrackClock {
    mic: u64,
    sys: u64,
}

impl TrackClock {
    /// Frame without a wall-clock check (scripted sources).
    pub fn frame(&mut self, track: Track, samples: Vec<f32>) -> AudioFrame {
        let offset_samples = *self.counter(track);
        self.emit(track, offset_samples, samples)
    }

    /// Frame that ended `elapsed_samples` after the session started.
    pub fn frame_at(
        &mut self,
        track: Track,
        samples: Vec<f32>,
        elapsed_samples: u64,
    ) -> AudioFrame {
        let counted = *self.counter(track);
        let by_clock = elapsed_samples.saturating_sub(samples.len() as u64);
        let offset_samples = if by_clock > counted + GAP_TOLERANCE_SAMPLES {
            tracing::debug!(?track, gap_samples = by_clock - counted, "capture gap");
            by_clock
        } else {
            counted
        };
        self.emit(track, offset_samples, samples)
    }

    fn counter(&mut self, track: Track) -> &mut u64 {
        match track {
            Track::Mic => &mut self.mic,
            Track::Sys => &mut self.sys,
        }
    }

    fn emit(&mut self, track: Track, offset_samples: u64, samples: Vec<f32>) -> AudioFrame {
        *self.counter(track) = offset_samples + samples.len() as u64;
        AudioFrame {
            track,
            offset_samples,
            samples,
        }
    }
}

/// The backend for this OS.
pub fn default_backend() -> Box<dyn AudioBackend> {
    #[cfg(target_os = "linux")]
    {
        Box::new(pipewire::PipeWireBackend::new())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(UnsupportedBackend)
    }
}

#[cfg(not(target_os = "linux"))]
struct UnsupportedBackend;

#[cfg(not(target_os = "linux"))]
impl AudioBackend for UnsupportedBackend {
    fn list_devices(&self) -> Result<Vec<AudioDevice>, CaptureError> {
        Err(CaptureError::Unsupported)
    }
    fn start(&self, _config: CaptureConfig) -> Result<CaptureSession, CaptureError> {
        Err(CaptureError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_2_1_track_clock_counts_each_track_separately() {
        let mut clock = TrackClock::default();
        assert_eq!(clock.frame(Track::Mic, vec![0.0; 480]).offset_samples, 0);
        assert_eq!(clock.frame(Track::Sys, vec![0.0; 256]).offset_samples, 0);
        assert_eq!(clock.frame(Track::Mic, vec![0.0; 480]).offset_samples, 480);
        assert_eq!(clock.frame(Track::Sys, vec![0.0; 256]).offset_samples, 256);
    }

    #[test]
    fn fr_2_1_track_clock_jumps_over_gaps_but_not_jitter() {
        let mut clock = TrackClock::default();
        // On time.
        assert_eq!(
            clock
                .frame_at(Track::Sys, vec![0.0; 1024], 1024)
                .offset_samples,
            0
        );
        // A little late: jitter, keep counting.
        assert_eq!(
            clock
                .frame_at(Track::Sys, vec![0.0; 1024], 4096)
                .offset_samples,
            1024
        );
        // One second of nothing: jump to the clock.
        let late = clock.frame_at(Track::Sys, vec![0.0; 1024], 2048 + 48_000 + 1024);
        assert_eq!(late.offset_samples, 2048 + 48_000);
        // Early callbacks never move the offset backwards.
        let next = clock.frame_at(Track::Sys, vec![0.0; 1024], 0);
        assert_eq!(next.offset_samples, 2048 + 48_000 + 1024);
        // The other track is unaffected.
        assert_eq!(
            clock.frame_at(Track::Mic, vec![0.0; 10], 10).offset_samples,
            0
        );
    }

    #[test]
    fn capture_errors_become_audio_device_errors() {
        let err: AppError = CaptureError::Connect("socket refused".into()).into();
        assert_eq!(err.code(), "audio_device");
        assert!(!err.to_string().contains("socket"));
    }
}
