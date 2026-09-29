pub mod currency;
pub mod solver;
pub mod mercadolivre;
pub mod scrape;
mod sites;
pub mod store;

use crate::metrics;
use serenity::all::{CreateEmbed, CreateMessage, Http, UserId};
use std::{sync::Arc, time::Duration};
use store::{Store, TrackedProduct};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

pub use currency::format as format_price;

/// How often the scheduler looks for products whose hourly check is due.
const POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Products checked in parallel.
const MAX_CONCURRENT_CHECKS: usize = 4;

/// A claimed product is re-offered to other workers after this long, in case
/// the worker holding it died mid-check.
const LEASE: Duration = Duration::from_secs(10 * 60);

/// Warn the owner once after this many consecutive failed hourly checks.
const FAILURE_NOTIFY_THRESHOLD: i32 = 24;

/// Check every product once an hour, counted from when it was added, so the
/// checks are spread across the hour instead of all running at once.
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
    let started = std::time::Instant::now();
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
