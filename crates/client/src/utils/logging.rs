//! File logging (`~/.zensidus/logs/YYYY-MM-DD.log`, daily rotation, WARN and
//! above) plus the helpers the Settings → Logs page uses to list, read and
//! open those files.

use anyhow::Context;
use chrono::NaiveDate;
use tracing::level_filters::LevelFilter;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Install the tracing subscriber: a daily-rotating file layer under
/// `log_dir` (WARN+), and optionally a stderr layer filtered by `RUST_LOG`
/// (default `info`). The returned guard flushes the file writer on drop, so
/// keep it alive for the whole run.
pub fn init(log_dir: &std::path::Path, console: bool) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create log dir: {}", log_dir.display()))?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_suffix("log")
        // Daily files otherwise accumulate forever; a month is generous.
        .max_log_files(30)
        .build(log_dir)
        .with_context(|| format!("failed to build file appender: {}", log_dir.display()))?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_filter(LevelFilter::WARN);

    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let console_layer = if console {
        Some(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(true)
                .with_target(true)
                .with_line_number(true)
                .with_filter(console_filter),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(file_layer)
        .with(console_layer)
        .init();

    Ok(guard)
}

/// Directory holding the rotated daily log files.
pub fn log_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(crate::utils::app_dir()?.join("logs"))
}

/// Path of one day's log file (`YYYY-MM-DD.log`); the file may not exist.
pub fn day_log_path(date: NaiveDate) -> anyhow::Result<std::path::PathBuf> {
    Ok(log_dir()?.join(format!("{}.log", date.format("%Y-%m-%d"))))
}

/// Read one day's log file; `Ok(None)` when it does not exist.
pub fn read_day_log(date: NaiveDate) -> anyhow::Result<Option<String>> {
    let path = day_log_path(date)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read log file: {}", path.display()))?;
    Ok(Some(content))
}

/// Open one day's log file with the OS default program.
pub fn open_day_log(date: NaiveDate) -> anyhow::Result<()> {
    let path = day_log_path(date)?;
    if !path.exists() {
        anyhow::bail!("log file does not exist: {}", path.display());
    }
    open_with_default_app(&path)
}

/// Open a path with the OS default program.
fn open_with_default_app(path: &std::path::Path) -> anyhow::Result<()> {
    // explorer opens the file through the shell association and tolerates the
    // file being held open by the tracing appender. `cmd /C start` from a GUI
    // process hits title-quoting pitfalls and prints shell errors to the
    // inherited stderr instead of surfacing them through spawn's Result.
    #[cfg(target_os = "windows")]
    let spawned = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(path).spawn();
    spawned.context("failed to spawn the default file opener")?;
    Ok(())
}
