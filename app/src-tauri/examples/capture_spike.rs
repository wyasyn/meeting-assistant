//! Dev tool for roadmap 1.1: lists devices and records both tracks to raw f32 files.
//! Never run by the app. Files go outside the repo; recordings must not be committed.
//!
//!   cargo run --example capture_spike -- [seconds] [out_dir] [mic_id|-] [output_id|-]
//!   pw-play --format f32 --rate 48000 --channels 1 <out_dir>/mic.f32
//!
//! `RUST_LOG=debug` shows stream states and negotiated formats.

use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use meeting_assistant_lib::capture::{default_backend, CaptureConfig, Track, SAMPLE_RATE};

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let seconds: u64 = args.next().map_or(Ok(10), |s| s.parse())?;
    let out_dir = args
        .next()
        .map_or_else(|| std::env::temp_dir().join("capture-spike"), PathBuf::from);
    std::fs::create_dir_all(&out_dir)?;
    let mut device = || args.next().filter(|id| id != "-");
    let config = CaptureConfig {
        mic: device(),
        output: device(),
    };

    let backend = default_backend();
    for device in backend.list_devices()? {
        println!(
            "{:<7} {} {} ({})",
            format!("{:?}", device.kind),
            if device.is_default { "*" } else { " " },
            device.name,
            device.id
        );
    }

    let mut mic = BufWriter::new(File::create(out_dir.join("mic.f32"))?);
    let mut sys = BufWriter::new(File::create(out_dir.join("sys.f32"))?);
    // samples written (gaps filled with silence), sum of squares, frames, gap samples
    let mut stats = [(0u64, 0f64, 0u64, 0u64), (0u64, 0f64, 0u64, 0u64)];

    println!("recording {seconds} s to {}", out_dir.display());
    let mut session = backend.start(config)?;
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        let Some(frame) = session.recv_timeout(Duration::from_millis(200))? else {
            continue;
        };
        let (writer, stat) = match frame.track {
            Track::Mic => (&mut mic, &mut stats[0]),
            Track::Sys => (&mut sys, &mut stats[1]),
        };
        if frame.offset_samples > stat.0 {
            let gap = frame.offset_samples - stat.0;
            for _ in 0..gap {
                writer.write_all(&0f32.to_le_bytes())?;
            }
            stat.0 += gap;
            stat.3 += gap;
        }
        for sample in &frame.samples {
            writer.write_all(&sample.to_le_bytes())?;
            stat.1 += f64::from(*sample) * f64::from(*sample);
        }
        stat.0 += frame.samples.len() as u64;
        stat.2 += 1;
    }
    session.stop();
    mic.flush()?;
    sys.flush()?;

    for (label, (samples, squares, frames, gaps)) in ["mic", "sys"].iter().zip(stats) {
        let rms = if samples == 0 {
            0.0
        } else {
            (squares / samples as f64).sqrt()
        };
        let db = 20.0 * rms.max(1e-9).log10();
        println!(
            "{label}: {frames} frames, {:.2} s with {:.2} s of gaps filled, rms {db:.1} dBFS",
            samples as f64 / f64::from(SAMPLE_RATE),
            gaps as f64 / f64::from(SAMPLE_RATE)
        );
    }
    Ok(())
}
