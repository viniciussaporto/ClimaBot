pub mod currency;
pub mod solver;
pub mod mercadolivre;
pub mod scrape;
mod sites;
pub mod store;

use crate::metrics;
use serenity::all::{CreateEmbed, CreateMessage, Http, UserId};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Duration,
};
// tokio's clock (identical in production) so tests can run on virtual time.
use tokio::time::Instant;
use store::{Store, TrackedProduct};
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, error, info, warn};

pub use currency::format as format_price;

/// How often the scheduler looks for products whose hourly check is due.
const POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Products checked in parallel.
const MAX_CONCURRENT_CHECKS: usize = 4;

/// A claimed product is re-offered to other workers after this long, in case
/// the worker holding it died mid-check.
const LEASE: Duration = Duration::from_secs(10 * 60);

/// Random pause between two scheduled fetches from the same shop, so several
/// products on one site don't hit it in a burst.
const SAME_SITE_GAP_SECS: std::ops::RangeInclusive<u64> = 15..=45;

/// Earliest time each site may be fetched again by the scheduler.
static NEXT_ALLOWED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// The site a URL belongs to: its registrable domain, roughly
/// (`produto.mercadolivre.com.br` and `www.mercadolivre.com.br` are one site).
fn site_of(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    if n < 3 || host.parse::<std::net::IpAddr>().is_ok() {
        return Some(host);
    }
    // Country domains with a second level: example.com.br, example.co.uk, …
    let second_level = labels[n - 1].len() == 2
        && matches!(labels[n - 2], "com" | "co" | "net" | "org" | "gov" | "edu" | "ac" | "gob" | "or" | "ne");
    let keep = if second_level { 3 } else { 2 };
    Some(labels[n - keep..].join("."))
}

/// Wait until the scheduler may fetch from this URL's site again, and book the
/// next slot for it.
async fn wait_for_site(url: &str) {
    let Some(site) = site_of(url) else { return };
    loop {
        let wait = {
            let mut next_allowed = NEXT_ALLOWED.lock().await;
            let now = Instant::now();
            match next_allowed.get(&site) {
                Some(&at) if at > now => at - now,
                _ => {
                    let gap = Duration::from_secs(fastrand::u64(SAME_SITE_GAP_SECS));
                    next_allowed.insert(site, now + gap);
                    return;
                }
            }
        };
        debug!(site = %site, wait_secs = wait.as_secs(), "Spacing out requests to the same site");
        tokio::time::sleep(wait).await;
    }
}

/// Load the fetch methods saved by earlier runs, and save every change from
/// now on, so a restart doesn't make each shop's first check try every method.
pub async fn remember_fetch_methods(store: Store) {
    match store.fetch_methods().await {
        Ok(saved) => {
            let restored = scrape::restore_preferred(saved);
            info!(shops = restored, "Restored remembered fetch methods");
        }
        Err(e) => warn!(error = %e, "Could not load remembered fetch methods"),
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    scrape::report_preferred_changes(tx);
    tokio::spawn(async move {
        while let Some((host, method)) = rx.recv().await {
            match store.save_fetch_method(&host, &method).await {
                Ok(()) => debug!(host = %host, method = %method, "Saved fetch method"),
                Err(e) => warn!(error = %e, host = %host, "Could not save fetch method"),
            }
        }
    });
}

/// Warn the owner once after this many consecutive failed hourly checks.
const FAILURE_NOTIFY_THRESHOLD: i32 = 24;

/// Check every product about once an hour: at a random time within ±15 minutes
/// of its hourly slot (counted from when it was added), with random gaps
/// between requests to the same site, so the traffic doesn't look scheduled.
pub async fn run_scheduler(store: Store, http: Arc<Http>) {
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CHECKS));
    loop {
        update_gauges(&store).await;
        loop {
            let Ok(permit) = permits.clone().acquire_owned().await else {
                return;
            };
            let product = match store.claim_due(LEASE).await {
                Ok(Some(p)) => p,
                Ok(None) => break,
                Err(e) => {
                    error!(error = %e, "Price check: failed to claim due products");
                    break;
                }
            };
            let (store, http) = (store.clone(), http.clone());
            tokio::spawn(async move {
                check_one(&store, &http, &product).await;
                drop(permit);
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Checks later than this count as overdue in the metrics.
const OVERDUE_GRACE: Duration = Duration::from_secs(5 * 60);

async fn update_gauges(store: &Store) {
    match tokio::try_join!(store.count_products(), store.count_overdue(OVERDUE_GRACE)) {
        Ok((total, overdue)) => {
            metrics::TRACKED_PRODUCTS.set(total as i64);
            metrics::OVERDUE_CHECKS.set(overdue as i64);
        }
        Err(e) => warn!(error = %e, "Could not count tracked products"),
    }
}

async fn check_one(store: &Store, http: &Http, product: &TrackedProduct) {
    wait_for_site(&product.url).await;
    let started = Instant::now();
    let ok = check_and_record(store, http, product).await;
    metrics::PRICE_CHECK_DURATION
        .with_label_values(&[if ok { "success" } else { "error" }])
        .observe(started.elapsed().as_secs_f64());
}

/// Returns whether the price could be read.
async fn check_and_record(store: &Store, http: &Http, product: &TrackedProduct) -> bool {
    debug!(url = %product.url, "Checking price");
    match scrape::fetch_product(&product.url).await {
        Ok(found) => {
            // Keep the currency we already know if the page stopped showing one.
            let currency = found.currency.clone().or_else(|| product.currency.clone());
            match store
                .record_check(product, &found.name, found.price, currency.as_deref())
                .await
            {
                Ok(Some(old)) => {
                    metrics::PRICE_CHECK_COUNTER.with_label_values(&["changed"]).inc();
                    info!(url = %product.url, old, new = found.price, "Price change detected");
                    notify_change(http, product, &found.name, old, found.price, currency.as_deref()).await;
                }
                Ok(None) => {
                    metrics::PRICE_CHECK_COUNTER.with_label_values(&["unchanged"]).inc();
                }
                Err(e) => error!(error = %e, url = %product.url, "Price check: save failed"),
            }
            true
        }
        Err(e) => {
            metrics::PRICE_CHECK_COUNTER.with_label_values(&["error"]).inc();
            warn!(error = %e, url = %product.url, "Price check: scrape failed");
            match store.record_failure(product).await {
                Ok(n) if n == FAILURE_NOTIFY_THRESHOLD => {
                    let embed = CreateEmbed::new()
                        .title("⚠️ Price tracking problem")
                        .description(format!(
                            "I haven't been able to read the price of **{}** for {n} hours ({e}).\n{}\n\n\
                             I'll keep trying. Use `/pt remove` if the product is gone.",
                            product.name, product.url
                        ))
                        .color(0xf1c40f);
                    send_dm(http, product.owner(), embed).await;
                }
                Ok(_) => {}
                Err(e) => error!(error = %e, url = %product.url, "Price check: save failed"),
            }
            false
        }
    }
}

async fn notify_change(
    http: &Http,
    product: &TrackedProduct,
    name: &str,
    old: f64,
    new: f64,
    currency: Option<&str>,
) {
    let (icon, verb, color) = if new < old {
        ("📉", "dropped", 0x2ecc71)
    } else {
        ("📈", "went up", 0xe74c3c)
    };
    let pct = (new - old) / old * 100.0;
    let embed = CreateEmbed::new()
        .title(format!("{icon} Price {verb}: {name}"))
        .url(&product.url)
        .description(format!(
            "**{}** → **{}** ({pct:+.1}%)",
            format_price(old, currency),
            format_price(new, currency)
        ))
        .color(color);
    send_dm(http, product.owner(), embed).await;
}

async fn send_dm(http: &Http, user_id: u64, embed: CreateEmbed) {
    let user = UserId::new(user_id);
    let result = async {
        let channel = user.create_dm_channel(http).await?;
        channel.send_message(http, CreateMessage::new().embed(embed)).await
    }
    .await;
    if let Err(e) = result {
        // Usually the user has DMs from server members disabled.
        warn!(error = %e, user = user_id, "Could not DM price notification");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites() {
        assert_eq!(site_of("https://www.kabum.com.br/produto/1").as_deref(), Some("kabum.com.br"));
        assert_eq!(site_of("https://produto.mercadolivre.com.br/MLB-1").as_deref(), Some("mercadolivre.com.br"));
        assert_eq!(site_of("https://www.amazon.co.uk/dp/X").as_deref(), Some("amazon.co.uk"));
        assert_eq!(site_of("https://pt.aliexpress.com/item/1.html").as_deref(), Some("aliexpress.com"));
        assert_eq!(site_of("https://shop.example.de/p").as_deref(), Some("example.de"));
        assert_eq!(site_of("https://example.com/").as_deref(), Some("example.com"));
        assert_eq!(site_of("not a url"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn same_site_requests_are_spaced() {
        let start = Instant::now();
        wait_for_site("https://a.example-shop.com/1").await;
        wait_for_site("https://b.example-shop.com/2").await;
        let waited = start.elapsed();
        assert!(waited >= Duration::from_secs(15) && waited <= Duration::from_secs(46), "{waited:?}");
        // A different site doesn't wait.
        let t = Instant::now();
        wait_for_site("https://other-shop.org/x").await;
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}
