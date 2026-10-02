//! Scripted `AudioBackend` for tests of later pipeline steps (1.2, 1.9).

use std::sync::mpsc;

use super::{
    AudioBackend, AudioDevice, CaptureConfig, CaptureError, CaptureSession, Track, TrackClock,
};

/// Emits `script` as frames, then ends the session.
pub struct FakeBackend {
    pub devices: Vec<AudioDevice>,
    pub script: Vec<(Track, Vec<f32>)>,
}

impl AudioBackend for FakeBackend {
    fn list_devices(&self) -> Result<Vec<AudioDevice>, CaptureError> {
        Ok(self.devices.clone())
    }

    fn start(&self, _config: CaptureConfig) -> Result<CaptureSession, CaptureError> {
        let (tx, rx) = mpsc::sync_channel(self.script.len().max(1));
        let mut clock = TrackClock::default();
        for (track, samples) in &self.script {
            tx.send(clock.frame(*track, samples.clone()))
                .map_err(|e| CaptureError::Stream(e.to_string()))?;
        }
        drop(tx);
        Ok(CaptureSession::new(rx, Box::new(|| {}), None))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn delivers_script_in_order_then_ends() {
        let backend = FakeBackend {
            devices: vec![],
            script: vec![(Track::Mic, vec![0.1; 4]), (Track::Mic, vec![0.2; 4])],
        };
        let mut session = backend.start(CaptureConfig::default()).unwrap();
        let wait = Duration::from_millis(10);
        let first = session.recv_timeout(wait).unwrap().unwrap();
        let second = session.recv_timeout(wait).unwrap().unwrap();
        assert_eq!((first.offset_samples, second.offset_samples), (0, 4));
        assert!(session.recv_timeout(wait).is_err());
        session.stop();
    }
}
