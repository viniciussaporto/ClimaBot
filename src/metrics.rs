use axum::{
    Router,
    http::{HeaderValue, StatusCode, header},
    routing::get,
};
use prometheus::{
    CounterVec, Encoder, Gauge, GaugeVec, HistogramVec, IntGauge, TextEncoder, register_counter_vec, register_gauge,
    register_gauge_vec, register_histogram_vec, register_int_gauge,
};
use std::sync::LazyLock;
use tracing::{error, info};

// ─────────────────────────────────────────────
//  Global metric instances
//
//  Names and labels match the previous TypeScript bot so the existing
//  Prometheus alert rules and Grafana dashboards keep working.
// ─────────────────────────────────────────────

/// Number of commands received / completed / failed
pub static COMMAND_COUNTER: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!("discord_command_total", "Count of Discord commands executed", &["command", "status"])
        .expect("failed to register discord_command_total")
});

/// Number of self-assigned role changes
pub static ROLE_ASSIGNMENT_COUNTER: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!("discord_role_assignments_total", "Count of role assignments", &["action", "role"])
        .expect("failed to register discord_role_assignments_total")
});

/// Number of outbound weather / geocoding API calls
pub static WEATHER_API_COUNTER: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!("weather_api_requests_total", "Count of weather API requests", &["type", "status"])
        .expect("failed to register weather_api_requests_total")
});

/// End-to-end command latency
pub static RESPONSE_TIME: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "discord_command_response_time_seconds",
        "Response time for Discord commands",
        &["command", "status"],
        vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .expect("failed to register discord_command_response_time_seconds")
});

/// Number of product price checks performed by the hourly job
pub static PRICE_CHECK_COUNTER: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!("price_checks_total", "Count of product price checks, by outcome", &["status"])
        .expect("failed to register price_checks_total")
});

/// Page fetches by the price scraper, by method and outcome
pub static SCRAPE_FETCH_COUNTER: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "price_scrape_fetches_total",
        "Count of price-scraper page fetches, by method (direct, flaresolverr, byparr, mercadolivre_api) and result",
        &["method", "result"]
    )
    .expect("failed to register price_scrape_fetches_total")
});

/// Servers (guilds) the bot is in
pub static GUILDS: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!("discord_guilds", "Number of Discord servers the bot is in")
        .expect("failed to register discord_guilds")
});

/// Heartbeat round trip to Discord's gateway
pub static GATEWAY_LATENCY: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!("discord_gateway_latency_seconds", "Heartbeat latency of the Discord gateway connection")
        .expect("failed to register discord_gateway_latency_seconds")
});

/// Products currently tracked, across all users
pub static TRACKED_PRODUCTS: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!("price_tracked_products", "Number of products being price-tracked")
        .expect("failed to register price_tracked_products")
});

/// Products whose hourly check is more than 5 minutes late
pub static OVERDUE_CHECKS: LazyLock<IntGauge> = LazyLock::new(|| {
    register_int_gauge!("price_checks_overdue", "Products whose scheduled price check is more than 5 minutes late")
        .expect("failed to register price_checks_overdue")
});

/// Time to check one product (all fetch attempts included)
pub static PRICE_CHECK_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "price_check_duration_seconds",
        "Time to check one product's price, including every fetch method tried",
        &["result"],
        vec![1.0, 2.5, 5.0, 10.0, 20.0, 40.0, 60.0, 120.0, 240.0]
    )
    .expect("failed to register price_check_duration_seconds")
});

// ── Football data ─────────────────────────────

/// Requests to the football data providers, by outcome (success, not_found,
/// blocked = not sent because of budget/cooldown, rate_limited, error, …)
pub static FOOTBALL_API_COUNTER: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "football_api_requests_total",
        "Requests to football data providers, by provider, endpoint and outcome",
        &["provider", "endpoint", "status"]
    )
    .expect("failed to register football_api_requests_total")
});

pub static FOOTBALL_API_LATENCY: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "football_api_request_duration_seconds",
        "Latency of football data provider requests",
        &["provider"],
        vec![0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .expect("failed to register football_api_request_duration_seconds")
});

/// Requests counted against today's (UTC) budget, per provider
pub static FOOTBALL_QUOTA_USED: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "football_quota_used",
        "Football provider requests used today (UTC), as counted by the bot and corrected from provider headers",
        &["provider"]
    )
    .expect("failed to register football_quota_used")
});

pub static FOOTBALL_QUOTA_LIMIT: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!("football_quota_limit", "Daily request budget per football provider", &["provider"])
        .expect("failed to register football_quota_limit")
});

/// 1 while the provider is healthy, 0 during a cooldown, outage or with a rejected key
pub static FOOTBALL_PROVIDER_UP: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "football_provider_up",
        "Whether a football data provider is usable (1) or in a cooldown / failing (0)",
        &["provider"]
    )
    .expect("failed to register football_provider_up")
});

/// 1 while a tracked match is about to start, live, or just ended
pub static FOOTBALL_LIVE_WINDOW: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "football_live_window_active",
        "1 while tracked matches are live or about to start (the live scoreboard is being polled)"
    )
    .expect("failed to register football_live_window_active")
});

pub static FOOTBALL_LIVE_LAST_SUCCESS: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "football_live_last_success_timestamp_seconds",
        "Unix time of the last successful live scoreboard refresh"
    )
    .expect("failed to register football_live_last_success_timestamp_seconds")
});

pub static FOOTBALL_LIVE_MATCHES: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!("football_live_matches", "Live matches in the tracked competitions")
        .expect("failed to register football_live_matches")
});

/// Cache lookups: hit, refreshed, stale (refresh failed, old data served), miss
pub static FOOTBALL_CACHE_COUNTER: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "football_cache_requests_total",
        "Football data cache lookups by kind and result",
        &["kind", "result"]
    )
    .expect("failed to register football_cache_requests_total")
});

/// Follow notifications: live alerts and daily summaries, by outcome
pub static FOOTBALL_NOTIFICATIONS: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "football_notifications_total",
        "Football follow notifications (DMs and server posts), by kind and outcome",
        &["kind", "status"]
    )
    .expect("failed to register football_notifications_total")
});

pub static FOOTBALL_POLL_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "football_poll_duration_seconds",
        "Duration of the football scheduler's jobs",
        &["job"],
        vec![0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0]
    )
    .expect("failed to register football_poll_duration_seconds")
});

// ─────────────────────────────────────────────
//  HTTP server
// ─────────────────────────────────────────────

/// Spawn an Axum HTTP server that serves Prometheus metrics at `/metrics` and
/// the public health status at `/status`.
pub fn start_metrics_server(port: u16) {
    // Force static initialisation so all metrics are registered before the
    // first scrape (avoids "metric not found" errors in Prometheus).
    LazyLock::force(&COMMAND_COUNTER);
    LazyLock::force(&ROLE_ASSIGNMENT_COUNTER);
    LazyLock::force(&WEATHER_API_COUNTER);
    LazyLock::force(&RESPONSE_TIME);
    LazyLock::force(&PRICE_CHECK_COUNTER);
    LazyLock::force(&SCRAPE_FETCH_COUNTER);
    LazyLock::force(&GUILDS);
    LazyLock::force(&GATEWAY_LATENCY);
    LazyLock::force(&TRACKED_PRODUCTS);
    LazyLock::force(&OVERDUE_CHECKS);
    LazyLock::force(&PRICE_CHECK_DURATION);
    LazyLock::force(&FOOTBALL_API_COUNTER);
    LazyLock::force(&FOOTBALL_API_LATENCY);
    LazyLock::force(&FOOTBALL_QUOTA_USED);
    LazyLock::force(&FOOTBALL_QUOTA_LIMIT);
    LazyLock::force(&FOOTBALL_PROVIDER_UP);
    LazyLock::force(&FOOTBALL_LIVE_WINDOW);
    LazyLock::force(&FOOTBALL_LIVE_LAST_SUCCESS);
    LazyLock::force(&FOOTBALL_LIVE_MATCHES);
    LazyLock::force(&FOOTBALL_CACHE_COUNTER);
    LazyLock::force(&FOOTBALL_POLL_DURATION);
    LazyLock::force(&FOOTBALL_NOTIFICATIONS);

    tokio::spawn(async move {
        let app = Router::new().route("/metrics", get(metrics_handler)).route("/status", get(crate::status::handler));

        let addr = format!("0.0.0.0:{port}");
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, "Failed to bind metrics server on {addr}");
                return;
            }
        };
        info!("Metrics server listening at http://{addr}/metrics");

        if let Err(e) = axum::serve(listener, app).await {
            error!(error = %e, "Metrics server error");
        }
    });
}

// ─────────────────────────────────────────────
//  Handler
// ─────────────────────────────────────────────

async fn metrics_handler() -> impl axum::response::IntoResponse {
    let encoder = TextEncoder::new();
    let families = prometheus::gather();
    let mut buf = Vec::new();

    if let Err(e) = encoder.encode(&families, &mut buf) {
        error!(error = %e, "Failed to encode Prometheus metrics");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
            b"internal error".to_vec(),
        );
    }

    let content_type = HeaderValue::from_str(encoder.format_type())
        .unwrap_or_else(|_| HeaderValue::from_static("text/plain; version=0.0.4"));

    (StatusCode::OK, [(header::CONTENT_TYPE, content_type)], buf)
}
