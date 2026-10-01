//! Public health status, served at `/status` next to `/metrics`.
//!
//! Meant for an uptime monitor outside the server, so the bot being down is
//! noticed even when Grafana (which sends every other alert) is down too.
//! Requests only read a snapshot refreshed in the background, so hitting the
//! endpoint never touches Discord or MongoDB.

use crate::pricetracker::store::Store;
use axum::{
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use serde::Serialize;
use serenity::all::{ConnectionStage, ShardManager};
use std::{
    sync::{Arc, LazyLock, RwLock},
    time::{Duration, Instant},
};
use tracing::warn;

/// How often the snapshot is refreshed.
const PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// A snapshot older than this means the probe itself is stuck: report not ok.
const MAX_AGE: Duration = Duration::from_secs(120);

const DB_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Default)]
struct Snapshot {
    discord: bool,
    database: bool,
    at: Option<Instant>,
}

static SNAPSHOT: LazyLock<RwLock<Snapshot>> = LazyLock::new(Default::default);

#[derive(Serialize)]
struct Body {
    ok: bool,
    discord: bool,
    database: bool,
    version: &'static str,
}

/// Keep the snapshot up to date: is a shard connected to Discord's gateway,
/// and does MongoDB answer a ping?
pub async fn run_probes(store: Store, shard_manager: Arc<ShardManager>) {
    let mut interval = tokio::time::interval(PROBE_INTERVAL);
    loop {
        interval.tick().await;
        let discord = {
            let runners = shard_manager.runners.lock().await;
            runners.values().any(|r| r.stage == ConnectionStage::Connected)
        };
        let database = match tokio::time::timeout(DB_TIMEOUT, store.ping()).await {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                warn!(error = %e, "Status: MongoDB ping failed");
                false
            }
            Err(_) => {
                warn!("Status: MongoDB ping timed out");
                false
            }
        };
        *SNAPSHOT.write().unwrap_or_else(|p| p.into_inner()) =
            Snapshot { discord, database, at: Some(Instant::now()) };
    }
}

fn body(snapshot: Snapshot, now: Instant) -> Body {
    let fresh = snapshot.at.is_some_and(|at| now.duration_since(at) <= MAX_AGE);
    Body {
        ok: fresh && snapshot.discord && snapshot.database,
        discord: fresh && snapshot.discord,
        database: fresh && snapshot.database,
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// `200 {"ok":true,…}` when everything is healthy, `503 {"ok":false,…}` otherwise.
pub async fn handler() -> impl IntoResponse {
    let snapshot = *SNAPSHOT.read().unwrap_or_else(|p| p.into_inner());
    let body = body(snapshot, Instant::now());
    let code = if body.ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (
        code,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        serde_json::to_string(&body).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_only_when_fresh_and_healthy() {
        let now = Instant::now();
        let healthy = Snapshot { discord: true, database: true, at: Some(now) };
        assert!(body(healthy, now).ok);
        // Not probed yet (just started).
        assert!(!body(Snapshot::default(), now).ok);
        // Either dependency down.
        assert!(!body(Snapshot { discord: false, ..healthy }, now).ok);
        assert!(!body(Snapshot { database: false, ..healthy }, now).ok);
        // The probe stopped updating.
        let stale = body(healthy, now + MAX_AGE + Duration::from_secs(1));
        assert!(!stale.ok && !stale.discord && !stale.database);
    }

    #[test]
    fn body_has_the_ok_field_monitors_match() {
        let now = Instant::now();
        let json = serde_json::to_string(&body(Snapshot { discord: true, database: true, at: Some(now) }, now))
            .unwrap();
        assert!(json.starts_with(r#"{"ok":true,"#), "{json}");
    }
}
