use axum::{
    http::{header, HeaderValue, StatusCode},
    routing::get,
    Router,
};
use once_cell::sync::Lazy;
use prometheus::{
    register_counter_vec, register_histogram_vec, CounterVec, Encoder, HistogramVec, TextEncoder,
};
use tracing::{error, info};

// ─────────────────────────────────────────────
//  Global metric instances
// ─────────────────────────────────────────────

/// Number of commands received / completed / failed
pub static COMMAND_COUNTER: Lazy<CounterVec> = Lazy::new(|| {
    register_counter_vec!(
        "revolt_command_total",
        "Number of bot commands executed, by command name and status",
        &["command", "status"]
    )
    .expect("failed to register revolt_command_total")
});

/// Number of outbound weather API calls
pub static WEATHER_API_COUNTER: Lazy<CounterVec> = Lazy::new(|| {
    register_counter_vec!(
        "weather_api_requests_total",
        "Number of requests made to weather or geocoding APIs",
        &["type", "status"]
    )
    .expect("failed to register weather_api_requests_total")
});

/// End-to-end command latency
pub static RESPONSE_TIME: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "revolt_command_response_time_seconds",
        "Latency for handling a bot command, in seconds",
        &["command", "status"],
        vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .expect("failed to register revolt_command_response_time_seconds")
});

// ─────────────────────────────────────────────
//  HTTP server
// ─────────────────────────────────────────────

/// Spawn an Axum HTTP server that serves Prometheus metrics at `/metrics`.
pub fn start_metrics_server(port: u16) {
    // Force static initialisation so all metrics are registered before the
    // first scrape (avoids "metric not found" errors in Prometheus).
    let _ = &*COMMAND_COUNTER;
    let _ = &*WEATHER_API_COUNTER;
    let _ = &*RESPONSE_TIME;

    tokio::spawn(async move {
        let app = Router::new().route("/metrics", get(metrics_handler));

        let addr = format!("0.0.0.0:{port}");
        info!("Metrics server listening at http://{addr}/metrics");

        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, "Failed to bind metrics server on {addr}");
                return;
            }
        };

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

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        buf,
    )
}
