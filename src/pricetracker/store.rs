//! MongoDB persistence for tracked products and their price history.
//!
//! Each product has an hourly slot on the anniversary of when it was added
//! (`slot_at`), and is checked at a random time within ±15 minutes of it
//! (`next_check_at`), so checks don't follow a machine-regular pattern. Workers claim a due product with an atomic
//! `findOneAndUpdate` that sets a short lease, so several workers (or bot
//! replicas) never check the same product at the same time.
//!
//! It also remembers which fetch method last worked for each shop
//! (`fetch_methods`), so a restart doesn't make every shop's first check try
//! every method again.

use anyhow::{Context as _, Result};
use futures_util::TryStreamExt;
use mongodb::{
    Client, Collection, IndexModel,
    bson::{DateTime, doc, oid::ObjectId},
    error::{ErrorKind, WriteFailure},
    options::{IndexOptions, ReturnDocument},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Upper bound on products per user, to keep the hourly load bounded.
pub const MAX_PRODUCTS_PER_USER: u64 = 25;

/// Interval between checks of the same product, on average.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(3600);

/// Each check happens at a random time within this much of its hourly slot,
/// so the gap between two checks of a product varies from 30 to 90 minutes.
pub const CHECK_JITTER: Duration = Duration::from_secs(15 * 60);

/// A (jittered) check is never scheduled sooner than this from now.
const MIN_LEAD: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedProduct {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub user_id: i64,
    pub url: String,
    pub name: String,
    pub currency: Option<String>,
    pub last_price: Option<f64>,
    pub created_at: DateTime,
    pub last_checked: Option<DateTime>,
    /// When the next check runs: `slot_at` plus a random jitter.
    pub next_check_at: DateTime,
    /// The hourly slot the next check belongs to (no jitter). Missing on
    /// products saved before jitter existed; `next_check_at` stands in.
    #[serde(default)]
    pub slot_at: Option<DateTime>,
    /// Consecutive failed checks (reset on success).
    pub failures: i32,
    pub lease_until: Option<DateTime>,
}

impl TrackedProduct {
    pub fn owner(&self) -> u64 {
        self.user_id as u64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceEntry {
    pub product_id: ObjectId,
    pub price: f64,
    pub currency: Option<String>,
    pub observed_at: DateTime,
}

/// The fetch method that last worked for a host (see `scrape`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchMethod {
    #[serde(rename = "_id")]
    pub host: String,
    pub method: String,
    pub updated_at: DateTime,
}

#[derive(Debug, PartialEq)]
pub enum AddOutcome {
    Added,
    AlreadyTracked,
    LimitReached,
}

#[derive(Clone)]
pub struct Store {
    products: Collection<TrackedProduct>,
    history: Collection<PriceEntry>,
    fetch_methods: Collection<FetchMethod>,
}

/// Next hourly slot after `now`, keeping the product's phase: a product
/// added at 10:17 is checked at 11:17, 12:17, … even if a check ran late.
pub fn next_slot(scheduled: DateTime, now: DateTime) -> DateTime {
    let interval = CHECK_INTERVAL.as_millis() as i64;
    let scheduled = scheduled.timestamp_millis();
    let now = now.timestamp_millis();
    let elapsed_slots = if now < scheduled { 0 } else { (now - scheduled) / interval + 1 };
    DateTime::from_millis(scheduled + elapsed_slots.max(1) * interval)
}

/// `slot` moved by `offset_ms`, but never earlier than `MIN_LEAD` from now.
pub fn jittered(slot: DateTime, now: DateTime, offset_ms: i64) -> DateTime {
    let earliest = now.timestamp_millis() + MIN_LEAD.as_millis() as i64;
    DateTime::from_millis((slot.timestamp_millis() + offset_ms).max(earliest))
}

fn random_offset_ms() -> i64 {
    let j = CHECK_JITTER.as_millis() as i64;
    fastrand::i64(-j..=j)
}

/// The slot after this check, and the randomized time to run it.
fn schedule_after(product: &TrackedProduct, now: DateTime) -> (DateTime, DateTime) {
    let slot = next_slot(product.slot_at.unwrap_or(product.next_check_at), now);
    (slot, jittered(slot, now, random_offset_ms()))
}

fn add(t: DateTime, d: Duration) -> DateTime {
    DateTime::from_millis(t.timestamp_millis() + d.as_millis() as i64)
}

fn is_duplicate_key(e: &mongodb::error::Error) -> bool {
    matches!(&*e.kind, ErrorKind::Write(WriteFailure::WriteError(w)) if w.code == 11000)
}

impl Store {
    pub async fn connect(uri: &str, database: &str) -> Result<Self> {
        let client = Client::with_uri_str(uri).await.context("parsing MONGODB_URI")?;
        let db = client.database(database);
        db.run_command(doc! { "ping": 1 })
            .await
            .with_context(|| format!("cannot reach MongoDB at {}", redact(uri)))?;

        let store = Self {
            products: db.collection("products"),
            history: db.collection("price_history"),
            fetch_methods: db.collection("fetch_methods"),
        };
        store.create_indexes().await?;
        Ok(store)
    }

    /// Whether MongoDB answers (for the health status).
    pub async fn ping(&self) -> Result<()> {
        let db = self.products.client().database(self.products.namespace().db.as_str());
        db.run_command(doc! { "ping": 1 }).await?;
        Ok(())
    }

    async fn create_indexes(&self) -> Result<()> {
        self.products
            .create_indexes([
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "url": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder().keys(doc! { "user_id": 1, "created_at": 1 }).build(),
                IndexModel::builder().keys(doc! { "next_check_at": 1 }).build(),
            ])
            .await?;
        self.history
            .create_index(IndexModel::builder().keys(doc! { "product_id": 1, "observed_at": 1 }).build())
            .await?;
        Ok(())
    }

    /// Drop everything in this store's database (used by `selfcheck` on its
    /// scratch database only).
    pub async fn drop_database(&self) -> Result<()> {
        let db = self.products.client().database(self.products.namespace().db.as_str());
        db.drop().await?;
        Ok(())
    }

    /// A user's products, in the order they were added (positions are 1-based
    /// indexes into this list).
    pub async fn list_products(&self, user_id: u64) -> Result<Vec<TrackedProduct>> {
        Ok(self
            .products
            .find(doc! { "user_id": user_id as i64 })
            .sort(doc! { "created_at": 1, "_id": 1 })
            .await?
            .try_collect()
            .await?)
    }

    pub async fn count_products(&self) -> Result<u64> {
        Ok(self.products.count_documents(doc! {}).await?)
    }

    /// Products whose scheduled check is more than `grace` late: a sign the
    /// scheduler can't keep up.
    pub async fn count_overdue(&self, grace: Duration) -> Result<u64> {
        let cutoff = DateTime::from_millis(DateTime::now().timestamp_millis() - grace.as_millis() as i64);
        Ok(self.products.count_documents(doc! { "next_check_at": { "$lt": cutoff } }).await?)
    }

    pub async fn add_product(
        &self,
        user_id: u64,
        url: String,
        name: String,
        price: f64,
        currency: Option<String>,
    ) -> Result<AddOutcome> {
        let uid = user_id as i64;
        if self.products.count_documents(doc! { "user_id": uid, "url": &url }).await? > 0 {
            return Ok(AddOutcome::AlreadyTracked);
        }
        if self.products.count_documents(doc! { "user_id": uid }).await? >= MAX_PRODUCTS_PER_USER {
            return Ok(AddOutcome::LimitReached);
        }

        let now = DateTime::now();
        let product = TrackedProduct {
            id: ObjectId::new(),
            user_id: uid,
            url,
            name,
            currency: currency.clone(),
            last_price: Some(price),
            created_at: now,
            last_checked: Some(now),
            next_check_at: jittered(add(now, CHECK_INTERVAL), now, random_offset_ms()),
            slot_at: Some(add(now, CHECK_INTERVAL)),
            failures: 0,
            lease_until: None,
        };
        match self.products.insert_one(&product).await {
            Ok(_) => {}
            // Lost a race with a concurrent add of the same URL.
            Err(e) if is_duplicate_key(&e) => return Ok(AddOutcome::AlreadyTracked),
            Err(e) => return Err(e.into()),
        }
        self.history
            .insert_one(PriceEntry { product_id: product.id, price, currency, observed_at: now })
            .await?;
        Ok(AddOutcome::Added)
    }

    /// Delete a product owned by `user_id`. Returns whether it existed.
    pub async fn remove_product(&self, user_id: u64, product_id: ObjectId) -> Result<bool> {
        let deleted = self
            .products
            .delete_one(doc! { "_id": product_id, "user_id": user_id as i64 })
            .await?
            .deleted_count;
        if deleted > 0 {
            self.history.delete_many(doc! { "product_id": product_id }).await?;
        }
        Ok(deleted > 0)
    }

    /// Full price history for a product, oldest first.
    pub async fn history(&self, product_id: ObjectId) -> Result<Vec<PriceEntry>> {
        Ok(self
            .history
            .find(doc! { "product_id": product_id })
            .sort(doc! { "observed_at": 1, "_id": 1 })
            .await?
            .try_collect()
            .await?)
    }

    /// Atomically claim the most overdue product whose check time has come,
    /// leasing it for `lease` so no other worker picks it up meanwhile.
    pub async fn claim_due(&self, lease: Duration) -> Result<Option<TrackedProduct>> {
        let now = DateTime::now();
        Ok(self
            .products
            .find_one_and_update(
                doc! {
                    "next_check_at": { "$lte": now },
                    "$or": [ { "lease_until": null }, { "lease_until": { "$lt": now } } ],
                },
                doc! { "$set": { "lease_until": add(now, lease) } },
            )
            .sort(doc! { "next_check_at": 1 })
            .return_document(ReturnDocument::After)
            .await?)
    }

    /// Record a successful check and schedule the next one. Appends to the
    /// history only when the price changed, and returns the previous price in
    /// that case.
    pub async fn record_check(
        &self,
        product: &TrackedProduct,
        name: &str,
        price: f64,
        currency: Option<&str>,
    ) -> Result<Option<f64>> {
        let now = DateTime::now();
        let (slot, next) = schedule_after(product, now);
        let currency = currency.or(product.currency.as_deref());
        let updated = self
            .products
            .update_one(
                doc! { "_id": product.id },
                doc! {
                    "$set": {
                        "name": name,
                        "currency": currency,
                        "last_price": price,
                        "last_checked": now,
                        "next_check_at": next,
                        "slot_at": slot,
                        "failures": 0,
                    },
                    "$unset": { "lease_until": "" },
                },
            )
            .await?;
        if updated.matched_count == 0 {
            // Removed by its owner while the check was running.
            return Ok(None);
        }

        let changed = product.last_price.is_none_or(|old| (old - price).abs() >= 1e-9);
        if !changed {
            return Ok(None);
        }
        self.history
            .insert_one(PriceEntry {
                product_id: product.id,
                price,
                currency: currency.map(str::to_string),
                observed_at: now,
            })
            .await?;
        Ok(product.last_price)
    }

    /// Every remembered fetch method, as (host, method label) pairs.
    pub async fn fetch_methods(&self) -> Result<Vec<(String, String)>> {
        let all: Vec<FetchMethod> = self.fetch_methods.find(doc! {}).await?.try_collect().await?;
        Ok(all.into_iter().map(|m| (m.host, m.method)).collect())
    }

    /// Remember the fetch method that just worked for `host`.
    pub async fn save_fetch_method(&self, host: &str, method: &str) -> Result<()> {
        self.fetch_methods
            .update_one(
                doc! { "_id": host },
                doc! { "$set": { "method": method, "updated_at": DateTime::now() } },
            )
            .upsert(true)
            .await?;
        Ok(())
    }

    /// Record a failed check and schedule the next one; returns the new
    /// consecutive failure count.
    pub async fn record_failure(&self, product: &TrackedProduct) -> Result<i32> {
        let now = DateTime::now();
        let (slot, next) = schedule_after(product, now);
        let updated = self
            .products
            .find_one_and_update(
                doc! { "_id": product.id },
                doc! {
                    "$inc": { "failures": 1 },
                    "$set": {
                        "last_checked": now,
                        "next_check_at": next,
                        "slot_at": slot,
                    },
                    "$unset": { "lease_until": "" },
                },
            )
            .return_document(ReturnDocument::After)
            .await?;
        Ok(updated.map_or(0, |p| p.failures))
    }
}

/// Hide credentials when logging a connection string.
fn redact(uri: &str) -> String {
    match (uri.find("://"), uri.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => format!("{}://***{}", &uri[..scheme], &uri[at..]),
        _ => uri.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    #[test]
    fn next_slot_keeps_phase() {
        let t0 = DateTime::from_millis(10 * H + 17 * 60_000); // 10:17
        // On time or slightly late: the following hour.
        assert_eq!(next_slot(t0, DateTime::from_millis(t0.timestamp_millis() + 5_000)).timestamp_millis(), t0.timestamp_millis() + H);
        // Hours late (e.g. bot was down): the next future slot, same minute.
        let late = DateTime::from_millis(t0.timestamp_millis() + 3 * H + 10 * 60_000);
        assert_eq!(next_slot(t0, late).timestamp_millis(), t0.timestamp_millis() + 4 * H);
        // Early (clock skew): still one interval ahead.
        let early = DateTime::from_millis(t0.timestamp_millis() - 1_000);
        assert_eq!(next_slot(t0, early).timestamp_millis(), t0.timestamp_millis() + H);
    }

    #[test]
    fn jitter_stays_around_the_slot() {
        let now = DateTime::from_millis(100 * H);
        let slot = DateTime::from_millis(100 * H + H);
        let j = CHECK_JITTER.as_millis() as i64;
        assert_eq!(jittered(slot, now, -j).timestamp_millis(), slot.timestamp_millis() - j);
        assert_eq!(jittered(slot, now, j).timestamp_millis(), slot.timestamp_millis() + j);
        // Never scheduled in the past, even with a large negative offset.
        let soon = DateTime::from_millis(100 * H + 60_000);
        assert_eq!(jittered(soon, now, -j).timestamp_millis(), now.timestamp_millis() + 60_000);
        for _ in 0..1000 {
            let o = random_offset_ms();
            assert!((-j..=j).contains(&o));
        }
    }

    #[test]
    fn schedule_keeps_the_hourly_anchor() {
        let t0 = DateTime::from_millis(10 * H + 17 * 60_000); // slot 10:17
        let mut p = TrackedProduct {
            id: ObjectId::new(), user_id: 1, url: "u".into(), name: "n".into(), currency: None,
            last_price: None, created_at: t0, last_checked: None, next_check_at: t0, slot_at: Some(t0),
            failures: 0, lease_until: None,
        };
        // Checked 12 minutes early (negative jitter): the next slot is still 11:17.
        let (slot, next) = schedule_after(&p, DateTime::from_millis(t0.timestamp_millis() - 12 * 60_000));
        assert_eq!(slot.timestamp_millis(), t0.timestamp_millis() + H);
        assert!((next.timestamp_millis() - slot.timestamp_millis()).abs() <= CHECK_JITTER.as_millis() as i64);
        // Checked 14 minutes late: also 11:17, no drift.
        let (slot, _) = schedule_after(&p, DateTime::from_millis(t0.timestamp_millis() + 14 * 60_000));
        assert_eq!(slot.timestamp_millis(), t0.timestamp_millis() + H);
        // Products saved before jitter existed fall back to next_check_at.
        p.slot_at = None;
        let (slot, _) = schedule_after(&p, DateTime::from_millis(t0.timestamp_millis() + 60_000));
        assert_eq!(slot.timestamp_millis(), t0.timestamp_millis() + H);
    }

    #[test]
    fn redacts_credentials() {
        assert_eq!(redact("mongodb://user:pw@mongo:27017/x"), "mongodb://***@mongo:27017/x");
        assert_eq!(redact("mongodb://mongo:27017"), "mongodb://mongo:27017");
    }
}
