use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialise the global `tracing` subscriber.
///
/// - Structured **JSON** output rotated **daily** into `/var/log/climabot/climabot.log`
///   (mirrors the Winston `DailyRotateFile` transport)
/// - Human-readable **coloured** output on `stdout`
///
/// The log level can be controlled with the `RUST_LOG` environment variable
/// (default: `info`).  Example:
///   `RUST_LOG=climabot=debug,warn`
pub fn init() {
    // Daily-rotating JSON log file  →  /var/log/climabot/climabot.YYYY-MM-DD
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "/var/log/climabot", "climabot.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Keep the guard alive for the entire process lifetime so the background
    // flusher thread is never dropped prematurely.
    std::mem::forget(guard);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        // JSON layer → log file
        .with(
            fmt::layer()
                .json()
                .with_current_span(true)
                .with_writer(non_blocking),
        )
        // Pretty layer → stdout
        .with(fmt::layer().pretty().with_writer(std::io::stdout))
        .init();
}
