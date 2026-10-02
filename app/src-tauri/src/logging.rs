//! Tracing setup: daily rolling log files in `<app_data>/logs/`, plus stderr in debug builds.
//! `RUST_LOG` overrides the default `info` filter.
//! Never log API keys, transcripts or audio paths at info level or above (AGENTS.md rule 8).

use std::path::Path;
use std::sync::Mutex;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

const DEFAULT_FILTER: &str = "info";
const LOG_FILE_PREFIX: &str = "app";
const MAX_LOG_FILES: usize = 7;

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("could not create the log directory: {0}")]
    CreateDir(#[from] std::io::Error),
    #[error("could not open the log file: {0}")]
    Appender(#[from] tracing_appender::rolling::InitError),
    #[error("logging is already initialised: {0}")]
    Subscriber(#[from] tracing_subscriber::util::TryInitError),
}

/// Keeps the background log writer alive. Call `flush` on exit so buffered lines are written.
pub struct LogGuard(Mutex<Option<WorkerGuard>>);

impl LogGuard {
    pub fn flush(&self) {
        if let Ok(mut guard) = self.0.lock() {
            guard.take();
        }
    }
}

/// Installs the global subscriber and a panic hook that logs panics. Call once.
pub fn init(log_dir: &Path) -> Result<LogGuard, LoggingError> {
    std::fs::create_dir_all(log_dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(log_dir)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let file_layer = fmt::layer().with_writer(writer).with_ansi(false);
    let stderr_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()?;

    install_panic_hook();
    Ok(LogGuard(Mutex::new(Some(guard))))
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(%info, "panic");
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    // The subscriber is process-global, so this is the only test that calls `init`.
    #[test]
    fn writes_to_rolling_file_in_log_dir() {
        let dir = std::env::temp_dir().join(format!("ma-logging-test-{}", std::process::id()));
        let guard = init(&dir).unwrap();
        tracing::info!("hello from the test");
        guard.flush();

        let contents: String = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(contents.contains("hello from the test"));
    }
}
