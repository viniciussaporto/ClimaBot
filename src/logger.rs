use std::{env, io::IsTerminal};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Number of daily log files kept on disk.
const MAX_LOG_FILES: usize = 7;

/// Initialise the global `tracing` subscriber.
///
/// - Structured **JSON** output rotated **daily** into
///   `$LOG_DIR/climabot.log.YYYY-MM-DD` (default `LOG_DIR`: `/var/log/climabot`),
///   keeping the last 7 days
/// - Human-readable output on `stdout` (visible with `docker logs`)
///
/// The log level can be controlled with the `RUST_LOG` environment variable
/// (default: `info`).  Example:
///   `RUST_LOG=climabot=debug,warn`
pub fn init() {
    let env_filter = || {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    let log_dir = env::var("LOG_DIR").unwrap_or_else(|_| "/var/log/climabot".into());
    let file_appender = std::fs::create_dir_all(&log_dir)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix("climabot.log")
                .max_log_files(MAX_LOG_FILES)
                .build(&log_dir)
                .map_err(|e| e.to_string())
        });

    let file_layer = match file_appender {
        Ok(appender) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            // Keep the guard alive for the entire process lifetime so the
            // background flusher thread is never dropped prematurely.
            std::mem::forget(guard);
            Some(
                fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_writer(non_blocking)
                    .with_filter(env_filter()),
            )
        }
        Err(e) => {
            eprintln!("File logging disabled: cannot write to {log_dir}: {e}");
            None
        }
    };

    tracing_subscriber::registry()
        .with(file_layer)
        .with(
            fmt::layer()
                .with_ansi(std::io::stdout().is_terminal())
                .with_writer(std::io::stdout)
                .with_filter(env_filter()),
        )
        .init();
}
