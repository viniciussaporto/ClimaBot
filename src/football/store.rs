//! MongoDB persistence for football data, in the bot's existing database:
//!
//! - `football_cache`: the latest normalized result of every request
//!   (fixture lists, live scoreboard, match details, tables, squads…), keyed
//!   by resource, with its provider and fetch time. It survives restarts; old
//!   entries expire through a TTL index.
//! - `football_usage`: requests per provider and UTC day, so a restart can't
//!   reset the daily budget.
//! - `football_provider_health`: last known state of each provider.
//! - `football_team_map`: which API-Football team id a SportScore team is.
//!
//! Writes of usage/health are fire-and-forget: a slow database never delays a
//! Discord reply.

use super::model::Provider;
use anyhow::Result;
use chrono::{DateTime as ChronoDateTime, Utc};
use mongodb::{
    Collection, Database, IndexModel,
    bson::{DateTime, doc},
    options::IndexOptions,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheDoc {
    #[serde(rename = "_id")]
    pub key: String,
    pub provider: String,
    pub fetched_at: DateTime,
    pub expires_at: DateTime,
    /// The normalized data, as JSON.
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UsageDoc {
    #[serde(rename = "_id")]
    id: String,
    provider: String,
    day: String,
    used: i64,
    updated_at: DateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TeamMapDoc {
    #[serde(rename = "_id")]
    pub key: String,
    pub api_football_id: i64,
    pub api_football_name: String,
    pub updated_at: DateTime,
}

#[derive(Clone)]
pub struct FootballStore {
    cache: Collection<CacheDoc>,
    usage: Collection<UsageDoc>,
    health: Collection<mongodb::bson::Document>,
    team_map: Collection<TeamMapDoc>,
}

fn bson_time(t: ChronoDateTime<Utc>) -> DateTime {
    DateTime::from_millis(t.timestamp_millis())
}

impl FootballStore {
    pub async fn new(db: &Database) -> Result<Self> {
        let store = Self {
            cache: db.collection("football_cache"),
            usage: db.collection("football_usage"),
            health: db.collection("football_provider_health"),
            team_map: db.collection("football_team_map"),
        };
        store
            .cache
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "expires_at": 1 })
                    .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                    .build(),
            )
            .await?;
        // Usage is only needed for a few days.
        store
            .usage
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "updated_at": 1 })
                    .options(IndexOptions::builder().expire_after(Duration::from_secs(30 * 86400)).build())
                    .build(),
            )
            .await?;
        Ok(store)
    }

    pub async fn load(&self, key: &str) -> Result<Option<CacheDoc>> {
        Ok(self.cache.find_one(doc! { "_id": key }).await?)
    }

    pub async fn save(&self, doc: CacheDoc) -> Result<()> {
        self.cache.replace_one(doc! { "_id": &doc.key }, &doc).upsert(true).await?;
        Ok(())
    }

    /// Requests already made today, per provider.
    pub async fn usage_today(&self, provider: Provider, day: &str) -> Result<u32> {
        let id = format!("{}:{day}", provider.id());
        Ok(self.usage.find_one(doc! { "_id": id }).await?.map_or(0, |d| d.used.max(0) as u32))
    }

    pub fn save_usage(&self, provider: Provider, used: u32) {
        let usage = self.usage.clone();
        tokio::spawn(async move {
            let now = Utc::now();
            let day = super::quota::today(now);
            let doc = UsageDoc {
                id: format!("{}:{day}", provider.id()),
                provider: provider.id().into(),
                day,
                used: i64::from(used),
                updated_at: bson_time(now),
            };
            // `used` only grows within a day, so a late write can't undercount
            // by much; $max keeps the larger value if writes race.
            let result = usage
                .update_one(
                    doc! { "_id": &doc.id },
                    doc! {
                        "$max": { "used": doc.used },
                        "$set": { "provider": &doc.provider, "day": &doc.day, "updated_at": doc.updated_at },
                    },
                )
                .upsert(true)
                .await;
            if let Err(e) = result {
                warn!(error = %e, "Could not save football API usage");
            }
        });
    }

    pub fn save_health(
        &self,
        provider: Provider,
        state: &str,
        until: Option<ChronoDateTime<Utc>>,
        last_success: Option<ChronoDateTime<Utc>>,
        last_error: Option<String>,
    ) {
        let health = self.health.clone();
        let state = state.to_string();
        tokio::spawn(async move {
            let doc = doc! {
                "_id": provider.id(),
                "state": state,
                "until": until.map(bson_time),
                "last_success": last_success.map(bson_time),
                "last_error": last_error,
                "updated_at": DateTime::now(),
            };
            if let Err(e) = health.replace_one(doc! { "_id": provider.id() }, doc).upsert(true).await {
                warn!(error = %e, "Could not save football provider health");
            }
        });
    }

    pub async fn team_id(&self, key: &str) -> Result<Option<TeamMapDoc>> {
        Ok(self.team_map.find_one(doc! { "_id": key }).await?)
    }

    pub async fn save_team_id(&self, key: &str, id: i64, name: &str) -> Result<()> {
        let doc = TeamMapDoc {
            key: key.to_string(),
            api_football_id: id,
            api_football_name: name.to_string(),
            updated_at: DateTime::now(),
        };
        self.team_map.replace_one(doc! { "_id": key }, doc).upsert(true).await?;
        Ok(())
    }
}
