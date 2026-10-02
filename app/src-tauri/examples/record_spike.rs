//! Dev tool for roadmap 1.2: records both tracks into encrypted 10 s Opus chunks, then
//! decrypts them to playable files. Never run by the app. Use a folder outside the repo;
//! the key is a throwaway written next to the chunks, never the keychain key.
//!
//!   cargo run --example record_spike -- record [seconds] [out_dir] [mic_id|-] [output_id|-]
//!   cargo run --example record_spike -- decrypt [out_dir]     # writes out_dir/plain/*.opus
//!   pw-play out_dir/plain/mic-000001.opus

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use meeting_assistant_lib::capture::encoder::chunk_samples;
use meeting_assistant_lib::capture::recorder::{
    parse_chunk_index, read_chunk, Recorder, RecordingTarget,
};
use meeting_assistant_lib::capture::{default_backend, CaptureConfig, Track, SAMPLE_RATE};
use meeting_assistant_lib::store::crypto::AudioKey;

const MEETING_ID: &str = "spike";

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "record".into());
    if mode == "decrypt" {
        return decrypt(&out_dir(args.next()));
    }
    let seconds: u64 = args.next().map_or(Ok(30), |s| s.parse())?;
    let dir = out_dir(args.next());
    std::fs::create_dir_all(&dir)?;
    let mut key = [0u8; 32];
    getrandom::fill(&mut key)?;
    std::fs::write(dir.join("key.bin"), key)?;
    let mut device = || args.next().filter(|id| id != "-");
    let config = CaptureConfig {
        mic: device(),
        output: device(),
    };

    let target = RecordingTarget {
        meeting_id: MEETING_ID.into(),
        dir: dir.clone(),
        key: AudioKey::new(&key)?,
    };
    println!("recording {seconds} s to {}", dir.display());
    let recorder = Recorder::start(&*default_backend(), config, target, Box::new(|_| {}))?;
    for _ in 0..seconds * 10 {
        std::thread::sleep(Duration::from_millis(100));
        if !recorder.is_running() {
            break;
        }
    }
    let summary = recorder.stop()?;
    println!(
        "saved {} chunks, {:.1} s",
        summary.chunks,
        summary.duration_ms as f64 / 1_000.0
    );
    Ok(())
}

fn out_dir(arg: Option<String>) -> PathBuf {
    arg.map_or_else(|| std::env::temp_dir().join("record-spike"), PathBuf::from)
}

fn decrypt(dir: &Path) -> Result<(), Box<dyn Error>> {
    let target = RecordingTarget {
        meeting_id: MEETING_ID.into(),
        dir: dir.to_path_buf(),
        key: AudioKey::new(&std::fs::read(dir.join("key.bin"))?)?,
    };
    let plain = dir.join("plain");
    std::fs::create_dir_all(&plain)?;
    for track in [Track::Mic, Track::Sys] {
        let mut indexes: Vec<u32> = std::fs::read_dir(dir.join(track.as_str()))?
            .filter_map(|e| e.ok()?.file_name().to_str().and_then(parse_chunk_index))
            .collect();
        indexes.sort_unstable();
        let mut total = 0;
        for index in indexes {
            let ogg = read_chunk(&target, track, index)?;
            total += chunk_samples(&ogg).ok_or("not an Ogg Opus chunk")?;
            let name = format!("{}-{index:06}.opus", track.as_str());
            std::fs::write(plain.join(name), ogg)?;
        }
        println!(
            "{}: {:.2} s decrypted",
            track.as_str(),
            total as f64 / f64::from(SAMPLE_RATE)
        );
    }
    Ok(())
}
