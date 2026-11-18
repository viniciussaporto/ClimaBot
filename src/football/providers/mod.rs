//! Provider adapters and the request path they share: every call reserves
//! budget at the provider's [`Gate`], is classified into an [`Outcome`] that
//! updates the provider's health, and is counted in the metrics.

pub mod api_football;
pub mod football_data;
pub mod sportscore;

use super::{
    model::Provider,
    quota::{Blocked, Gate, Limits, Outcome, Purpose},
    store::FootballStore,
};
use crate::metrics;
use chrono::Utc;
use reqwest::{StatusCode, header::HeaderMap};
use serde_json::Value;
use std::{collections::HashMap, fmt, sync::Mutex, time::Duration};
use tracing::{debug, warn};

#[derive(Clone, Debug, PartialEq)]
pub enum FetchError {
    /// The request was not sent (budget, cooldown, bad key, not configured).
    Blocked(Blocked),
    /// The provider has no such match / team / competition.
    NotFound,
    /// The key is valid but this data isn't included in the plan.
    PlanRestricted(String),
    /// The request failed (network, 5xx, unexpected answer).
    Failed(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Blocked(b) => write!(f, "not requested: {b}"),
            FetchError::NotFound => f.write_str("not found"),
            FetchError::PlanRestricted(m) => write!(f, "not included in the free plan: {m}"),
            FetchError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for FetchError {}

pub type FetchResult<T> = Result<T, FetchError>;

/// The shared HTTP client, budgets and health of all providers.
pub struct Upstream {
    http: reqwest::Client,
    gates: HashMap<Provider, Mutex<Gate>>,
    store: Option<FootballStore>,
}

impl Upstream {
    pub fn new(limits: Vec<(Provider, Limits)>, store: Option<FootballStore>) -> Self {
        let now = Utc::now();
        let http = reqwest::Client::builder()
            .user_agent(concat!("ClimaBot/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("HTTP client");
        let gates = limits.into_iter().map(|(p, l)| (p, Mutex::new(Gate::new(l, now)))).collect();
        Self { http, gates, store }
    }

    pub fn is_configured(&self, p: Provider) -> bool {
        self.gates.contains_key(&p)
    }

    /// Run `f` on a provider's gate.
    pub fn with_gate<R>(&self, p: Provider, f: impl FnOnce(&mut Gate) -> R) -> Option<R> {
        self.gates.get(&p).map(|g| f(&mut g.lock().unwrap_or_else(|e| e.into_inner())))
    }

    /// GET a JSON document. `endpoint` is a short label for metrics.
    pub async fn get_json(
        &self,
        provider: Provider,
        endpoint: &'static str,
        url: &str,
        headers: &[(&str, &str)],
        purpose: Purpose,
    ) -> FetchResult<Value> {
        let reserved = self
            .with_gate(provider, |g| g.reserve(purpose, Utc::now()).map(|()| g.used))
            .unwrap_or(Err(Blocked::NotConfigured));
        let used = match reserved {
            Ok(used) => used,
            Err(b) => {
                metrics::FOOTBALL_API_COUNTER.with_label_values(&[provider.id(), endpoint, "blocked"]).inc();
                debug!(provider = provider.id(), endpoint, purpose = purpose.id(), reason = %b, "Football request not sent");
                return Err(FetchError::Blocked(b));
            }
        };
        if let Some(store) = &self.store {
            store.save_usage(provider, used);
        }

        let mut req = self.http.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let started = std::time::Instant::now();
        let response = req.send().await;
        metrics::FOOTBALL_API_LATENCY
            .with_label_values(&[provider.id()])
            .observe(started.elapsed().as_secs_f64());

        let (outcome, result) = match response {
            Err(e) => (Outcome::Unavailable, Err(FetchError::Failed(format!("request failed: {e}")))),
            Ok(r) => {
                let status = r.status();
                let headers = r.headers().clone();
                self.reconcile(provider, &headers);
                match r.bytes().await {
                    Err(e) => (Outcome::Unavailable, Err(FetchError::Failed(format!("reading body: {e}")))),
                    Ok(body) => classify(provider, status, &headers, &body),
                }
            }
        };

        let label = match &outcome {
            Outcome::Success => "success",
            Outcome::NotFound => "not_found",
            Outcome::PlanRestricted => "plan_restricted",
            Outcome::Unauthorized => "unauthorized",
            Outcome::MinuteLimit { .. } => "rate_limited",
            Outcome::DailyLimit => "quota_exhausted",
            Outcome::Unavailable => "error",
            Outcome::Malformed => "malformed",
        };
        metrics::FOOTBALL_API_COUNTER.with_label_values(&[provider.id(), endpoint, label]).inc();
        if !matches!(outcome, Outcome::Success | Outcome::NotFound) {
            warn!(provider = provider.id(), endpoint, outcome = label, url = %redact(url), "Football provider request failed");
        }
        let changed = self.with_gate(provider, |g| g.record(&outcome, Utc::now())).unwrap_or(false);
        self.publish(provider, changed);
        result
    }

    fn reconcile(&self, provider: Provider, h: &HeaderMap) {
        let num = |name: &str| h.get(name).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u32>().ok());
        if provider == Provider::ApiFootball {
            let remaining = num("x-ratelimit-requests-remaining");
            let limit = num("x-ratelimit-requests-limit");
            self.with_gate(provider, |g| g.reconcile(remaining, limit));
        }
    }

    /// Export the gate's state, and persist it when the health changed.
    pub fn publish(&self, provider: Provider, health_changed: bool) {
        let Some((used, daily, health, last_success, last_error, until)) = self.with_gate(provider, |g| {
            (g.used, g.limits.daily, g.health.id(), g.last_success, g.last_error.clone(), g.health.until())
        }) else {
            return;
        };
        metrics::FOOTBALL_QUOTA_USED.with_label_values(&[provider.id()]).set(used as f64);
        metrics::FOOTBALL_QUOTA_LIMIT.with_label_values(&[provider.id()]).set(daily as f64);
        metrics::FOOTBALL_PROVIDER_UP
            .with_label_values(&[provider.id()])
            .set(if health == "healthy" { 1.0 } else { 0.0 });
        if health_changed && let Some(store) = &self.store {
            store.save_health(provider, health, until, last_success, last_error);
        }
    }
}

/// Remove keys from URLs before logging them (none of ours carry one, but be safe).
fn redact(url: &str) -> String {
    match url.split_once('?') {
        Some((base, q)) => {
            let q: Vec<String> = q
                .split('&')
                .map(|kv| match kv.split_once('=') {
                    Some((k, _)) if k.contains("key") || k.contains("token") => format!("{k}=***"),
                    _ => kv.to_string(),
                })
                .collect();
            format!("{base}?{}", q.join("&"))
        }
        None => url.to_string(),
    }
}

/// Turn a provider's response into an outcome (for its health) and a result.
pub fn classify(
    provider: Provider,
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
) -> (Outcome, FetchResult<Value>) {
    let text = String::from_utf8_lossy(body);
    let snippet: String = text.chars().take(200).collect();
    let retry_after = ["retry-after", "x-requestcounter-reset"]
        .iter()
        .find_map(|h| headers.get(*h)?.to_str().ok()?.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let lower = text.to_lowercase();

    match status.as_u16() {
        200..=299 => {}
        401 => return (Outcome::Unauthorized, Err(FetchError::Failed("API key rejected".into()))),
        400 if lower.contains("token") || lower.contains("api key") => {
            return (Outcome::Unauthorized, Err(FetchError::Failed("API key rejected".into())));
        }
        403 => {
            return match provider {
                // football-data.org: valid token, competition/resource outside the plan.
                Provider::FootballData if !lower.contains("token") => {
                    (Outcome::PlanRestricted, Err(FetchError::PlanRestricted(snippet)))
                }
                Provider::SportScore => (Outcome::Unavailable, Err(FetchError::Failed(format!("HTTP 403: {snippet}")))),
                _ => (Outcome::Unauthorized, Err(FetchError::Failed("API key rejected".into()))),
            };
        }
        404 => return (Outcome::NotFound, Err(FetchError::NotFound)),
        429 => {
            let outcome = if lower.contains("day") || lower.contains("daily") {
                Outcome::DailyLimit
            } else {
                Outcome::MinuteLimit { retry_after }
            };
            return (outcome, Err(FetchError::Failed(format!("rate limited: {snippet}"))));
        }
        s => return (Outcome::Unavailable, Err(FetchError::Failed(format!("HTTP {s}: {snippet}")))),
    }

    let json: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return (Outcome::Malformed, Err(FetchError::Failed(format!("bad JSON: {e}")))),
    };

    // API-Football reports most problems as HTTP 200 with an `errors` object.
    if provider == Provider::ApiFootball {
        let errors: Vec<(String, String)> = match &json["errors"] {
            Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect(),
            Value::Array(a) => a.iter().map(|v| (String::new(), v.to_string())).collect(),
            _ => vec![],
        };
        if let Some((k, msg)) = errors.first() {
            let outcome = match k.as_str() {
                "token" => Outcome::Unauthorized,
                "requests" => Outcome::DailyLimit,
                "rateLimit" => Outcome::MinuteLimit { retry_after },
                "plan" => Outcome::PlanRestricted,
                _ => Outcome::Success,
            };
            let err = match outcome {
                Outcome::PlanRestricted => FetchError::PlanRestricted(msg.clone()),
                _ => FetchError::Failed(format!("{k}: {msg}")),
            };
            return (outcome, Err(err));
        }
    }
    // SportScore: {"error": "Match not found"}.
    if provider == Provider::SportScore
        && let Some(e) = json.get("error").and_then(Value::as_str)
    {
        return if e.to_lowercase().contains("not found") {
            (Outcome::NotFound, Err(FetchError::NotFound))
        } else {
            (Outcome::Success, Err(FetchError::Failed(e.to_string())))
        };
    }
    (Outcome::Success, Ok(json))
}

// ── Lenient JSON helpers shared by the adapters ─────────────────

/// A number the provider may send as a number or a string ("2", "").
pub fn num(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn unum(v: &Value) -> Option<u32> {
    num(v).and_then(|n| u32::try_from(n).ok())
}

/// A non-empty string, or None.
pub fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn str_or_empty(v: &Value) -> String {
    text(v).unwrap_or_default()
}

pub fn unix(v: &Value) -> Option<i64> {
    let s = v.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(*k, v.parse().unwrap());
        }
        m
    }

    #[test]
    fn classify_errors() {
        let ok = |p, s: u16, body: &str| classify(p, StatusCode::from_u16(s).unwrap(), &HeaderMap::new(), body.as_bytes()).0;
        assert_eq!(ok(Provider::SportScore, 200, r#"{"matches":[]}"#), Outcome::Success);
        assert_eq!(ok(Provider::SportScore, 200, r#"{"error":"Match not found"}"#), Outcome::NotFound);
        assert_eq!(ok(Provider::SportScore, 200, "<html>"), Outcome::Malformed);
        assert_eq!(ok(Provider::SportScore, 502, "bad gateway"), Outcome::Unavailable);
        assert_eq!(ok(Provider::FootballData, 403, r#"{"message":"The resource you are looking for is restricted"}"#), Outcome::PlanRestricted);
        assert_eq!(ok(Provider::FootballData, 400, r#"{"message":"Your API token is invalid."}"#), Outcome::Unauthorized);
        assert_eq!(ok(Provider::ApiFootball, 403, r#"{"message":"missing key"}"#), Outcome::Unauthorized);
        assert_eq!(
            ok(Provider::ApiFootball, 200, r#"{"errors":{"plan":"Free plans do not have access to this season"}}"#),
            Outcome::PlanRestricted
        );
        assert_eq!(
            ok(Provider::ApiFootball, 200, r#"{"errors":{"requests":"You have reached the request limit for the day"}}"#),
            Outcome::DailyLimit
        );
        assert_eq!(ok(Provider::ApiFootball, 200, r#"{"errors":{"token":"Missing application key"}}"#), Outcome::Unauthorized);
        assert_eq!(ok(Provider::ApiFootball, 200, r#"{"errors":[],"response":[]}"#), Outcome::Success);
        assert_eq!(ok(Provider::ApiFootball, 429, "Too many requests per day"), Outcome::DailyLimit);
    }

    #[test]
    fn minute_limit_uses_reset_header() {
        let (o, _) = classify(
            Provider::FootballData,
            StatusCode::TOO_MANY_REQUESTS,
            &h(&[("x-requestcounter-reset", "37")]),
            br#"{"message":"You reached your request limit. Wait 37 seconds."}"#,
        );
        assert_eq!(o, Outcome::MinuteLimit { retry_after: Some(Duration::from_secs(37)) });
    }

    #[test]
    fn lenient_numbers() {
        assert_eq!(num(&serde_json::json!("2")), Some(2));
        assert_eq!(num(&serde_json::json!("")), None);
        assert_eq!(unum(&serde_json::json!(-1)), None);
        assert_eq!(text(&serde_json::json!("  ")), None);
        assert_eq!(redact("https://x/y?a=1&api_key=s3cret"), "https://x/y?a=1&api_key=***");
    }
}
