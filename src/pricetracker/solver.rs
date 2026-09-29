//! Clients for challenge-solving browsers that speak the FlareSolverr API
//! (`POST /v1` with `request.get`):
//!
//! - FlareSolverr (https://github.com/FlareSolverr/FlareSolverr): Chrome.
//!   Gets through Akamai (given extra wait time) and many Cloudflare pages.
//! - Byparr (https://github.com/ThePhaseless/Byparr): Camoufox (Firefox).
//!   Gets through Cloudflare challenges that FlareSolverr times out on.
//!
//! Both return the final rendered HTML, so JavaScript-only prices work too.

use super::scrape::ScrapeError;
use serde::Deserialize;
use serde_json::json;
use std::{sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;
use url::Url;

/// How long a solver may spend on one page (challenge solving included).
const MAX_TIMEOUT: Duration = Duration::from_secs(40);

/// Extra tries when the browser crashes mid-page (Byparr often crashes a few
/// times on a Cloudflare-protected page before succeeding).
const CRASH_RETRIES: u32 = 4;
const CRASH_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
struct Response {
    status: String,
    #[serde(default)]
    message: String,
    solution: Option<Solution>,
}

#[derive(Debug, Deserialize)]
struct Solution {
    url: String,
    status: u16,
    #[serde(default)]
    response: String,
}

pub struct Page {
    pub html: String,
    pub url: Url,
}

pub struct Solver {
    pub name: &'static str,
    endpoint: Url,
    proxy: Option<String>,
    http: reqwest::Client,
    /// Each request runs a browser instance; cap how many run at once.
    permits: Semaphore,
}

impl Solver {
    pub fn new(name: &'static str, base: &str, proxy: Option<String>, concurrency: usize) -> anyhow::Result<Self> {
        let endpoint = Url::parse(base)?.join("v1")?;
        Ok(Self {
            name,
            endpoint,
            proxy,
            http: reqwest::Client::builder()
                .timeout(MAX_TIMEOUT + Duration::from_secs(60))
                .build()?,
            permits: Semaphore::new(concurrency.max(1)),
        })
    }

    /// The solver's status line, e.g. `FlareSolverr is ready! (3.5.2)`.
    pub async fn health(&self) -> anyhow::Result<String> {
        let response = self.http.get(self.endpoint.join("/")?).send().await?.error_for_status()?;
        let text = response.text().await?;
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        Ok(match (v["msg"].as_str(), v["version"].as_str()) {
            (Some(msg), Some(version)) => format!("{msg} ({version})"),
            (Some(msg), None) => msg.to_string(),
            _ => "reachable".to_string(),
        })
    }

    /// Load `url` in the browser. `wait` keeps the page open a little longer
    /// before returning, for challenges that solve themselves in JavaScript
    /// and then reload (e.g. Akamai's).
    pub async fn get(&self, url: &Url, wait: Option<Duration>) -> Result<Page, ScrapeError> {
        let mut body = json!({
            "cmd": "request.get",
            "url": url.as_str(),
            "maxTimeout": MAX_TIMEOUT.as_millis() as u64,
            "disableMedia": true,
        });
        if let Some(wait) = wait {
            body["waitInSeconds"] = json!(wait.as_secs());
        }
        if let Some(proxy) = &self.proxy {
            body["proxy"] = json!({ "url": proxy });
        }

        let name = self.name;
        let _permit = self.permits.acquire().await.map_err(|e| ScrapeError::Network(e.to_string()))?;
        let mut attempts_left = CRASH_RETRIES;
        let response: Response = loop {
            let http_response = self
                .http
                .post(self.endpoint.clone())
                .json(&body)
                .send()
                .await
                .map_err(|e| ScrapeError::Network(format!("{name} unavailable: {}", e.without_url())))?;
            let status = http_response.status();
            let text = http_response
                .text()
                .await
                .map_err(|e| ScrapeError::Network(format!("{name}: {}", e.without_url())))?;
            // Solvers report errors (e.g. challenge timeouts) with HTTP 500 and
            // a JSON body, so parse the body regardless of the status code.
            match serde_json::from_str(&text) {
                Ok(r) => break r,
                // Anything else is the browser crashing mid-page (Byparr answers
                // 502 "Target page, context or browser has been closed"):
                // transient, so try again.
                Err(_) if status.is_server_error() && attempts_left > 0 => {
                    attempts_left -= 1;
                    tracing::debug!(solver = name, %status, "Browser crashed; retrying");
                    tokio::time::sleep(CRASH_RETRY_DELAY).await;
                }
                Err(_) => {
                    let snippet: String = text.chars().take(120).collect();
                    return Err(ScrapeError::Network(format!("{name} answered HTTP {status}: {snippet}")));
                }
            }
        };

        if response.status != "ok" {
            return Err(ScrapeError::Network(format!("{name}: {}", response.message)));
        }
        let solution = response
            .solution
            .ok_or_else(|| ScrapeError::Network(format!("{name} returned no page")))?;
        if !(200..300).contains(&solution.status) {
            return Err(ScrapeError::Status(
                reqwest::StatusCode::from_u16(solution.status).unwrap_or(reqwest::StatusCode::BAD_GATEWAY),
            ));
        }
        let final_url = Url::parse(&solution.url).unwrap_or_else(|_| url.clone());
        Ok(Page { html: solution.response, url: final_url })
    }
}

fn from_env(name: &'static str, url_var: &str, concurrency_var: &str) -> Option<Solver> {
    let base = std::env::var(url_var).ok().filter(|s| !s.trim().is_empty())?;
    let concurrency = std::env::var(concurrency_var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    match Solver::new(name, base.trim(), super::scrape::proxy(), concurrency) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!(error = %e, "Invalid {url_var}; {name} disabled");
            None
        }
    }
}

/// Configured solvers, tried in this order: `FLARESOLVERR_URL`, `BYPARR_URL`
/// (each with a `*_CONCURRENCY`, default 1). `SCRAPER_PROXY` applies to both.
pub static SOLVERS: LazyLock<Vec<Solver>> = LazyLock::new(|| {
    [
        from_env("FlareSolverr", "FLARESOLVERR_URL", "FLARESOLVERR_CONCURRENCY"),
        from_env("Byparr", "BYPARR_URL", "BYPARR_CONCURRENCY"),
    ]
    .into_iter()
    .flatten()
    .collect()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_solution() {
        let r: Response = serde_json::from_str(
            r#"{"status":"ok","message":"Challenge solved!","solution":{"url":"https://x.com/p","status":200,
               "cookies":[],"userAgent":"UA","response":"<html></html>"},"startTimestamp":1,"endTimestamp":2,"version":"3.5.2"}"#,
        )
        .unwrap();
        let s = r.solution.unwrap();
        assert_eq!((s.status, s.url.as_str(), s.response.as_str()), (200, "https://x.com/p", "<html></html>"));

        let r: Response =
            serde_json::from_str(r#"{"status":"error","message":"Error: Error solving the challenge. Timeout after 60.0 seconds."}"#)
                .unwrap();
        assert_eq!(r.status, "error");
        assert!(r.solution.is_none());
    }

    #[test]
    fn endpoint() {
        let s = Solver::new("FlareSolverr", "http://flaresolverr:8191", None, 1).unwrap();
        assert_eq!(s.endpoint.as_str(), "http://flaresolverr:8191/v1");
    }
}
