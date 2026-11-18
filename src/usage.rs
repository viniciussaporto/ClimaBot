//! Today's (UTC) use of the weather and map APIs, for `/help status`.
//!
//! The football providers keep their own budgets (`football::quota`). This
//! only counts: it never blocks a request. Counts live in memory, so after a
//! restart they start again from zero (`/help status` says since when).
//! OpenCage reports its own quota in every response; that value wins.

use chrono::{DateTime, Utc};
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    OpenCage,
    OpenMeteo,
    OsmTiles,
}

impl Service {
    fn index(self) -> usize {
        match self {
            Service::OpenCage => 0,
            Service::OpenMeteo => 1,
            Service::OsmTiles => 2,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub day: String,
    /// Calls as the provider counts them (an Open-Meteo request for 108
    /// locations counts as 108).
    pub calls: u64,
    pub errors: u64,
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<DateTime<Utc>>,
    /// From the provider itself (OpenCage): (limit, remaining, reset time).
    pub reported: Option<(u64, u64, Option<DateTime<Utc>>)>,
}

struct State {
    started: DateTime<Utc>,
    services: [Usage; 3],
}

static STATE: LazyLock<Mutex<State>> =
    LazyLock::new(|| Mutex::new(State { started: Utc::now(), services: Default::default() }));

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn with<R>(service: Service, f: impl FnOnce(&mut Usage) -> R) -> R {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let u = &mut state.services[service.index()];
    let day = today();
    if u.day != day {
        *u = Usage { day, ..Default::default() };
    }
    f(u)
}

/// Count `weight` calls to `service`.
pub fn record(service: Service, weight: u64, ok: bool) {
    with(service, |u| {
        u.calls += weight;
        if ok {
            u.last_success = Some(Utc::now());
        } else {
            u.errors += 1;
            u.last_error = Some(Utc::now());
        }
    });
}

/// The provider's own report of its quota.
pub fn report(service: Service, limit: u64, remaining: u64, reset: Option<DateTime<Utc>>) {
    with(service, |u| u.reported = Some((limit, remaining, reset)));
}

pub fn get(service: Service) -> Usage {
    with(service, |u| u.clone())
}

/// When counting started (the bot's start).
pub fn since() -> DateTime<Utc> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).started
}

/// Make sure the start time is the process start, not the first `/help status`.
pub fn init() {
    LazyLock::force(&STATE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_weighted_calls_and_errors() {
        let before = get(Service::OsmTiles).calls;
        record(Service::OsmTiles, 3, true);
        record(Service::OsmTiles, 1, false);
        let u = get(Service::OsmTiles);
        assert_eq!(u.calls, before + 4);
        assert!(u.errors >= 1 && u.last_success.is_some() && u.last_error.is_some());
        report(Service::OsmTiles, 100, 42, None);
        assert_eq!(get(Service::OsmTiles).reported, Some((100, 42, None)));
    }
}
