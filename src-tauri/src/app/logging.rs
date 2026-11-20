//! Daily rolling log files in `%LOCALAPPDATA%\LLMTranslate\logs`, 7 days kept.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use super::paths;

pub fn init() -> Option<WorkerGuard> {
    let dir = paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("app")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("LLMT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(writer).with_ansi(false))
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();
    Some(guard)
}
