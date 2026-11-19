//! Who follows what: one document per user and per server in
//! `football_follows`. Everything is opt-in: a new document has every
//! notification off.

use anyhow::Result;
use futures_util::TryStreamExt;
use mongodb::{
    Collection, Database,
    bson::{DateTime, doc},
};
use serde::{Deserialize, Serialize};

pub const MAX_TEAMS: usize = 25;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FollowedTeam {
    /// SportScore team slug.
    pub slug: String,
    /// The name as SportScore spells it in fixtures ("Flamengo - RJ").
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    User(u64),
    Guild(u64),
}

impl Owner {
    pub fn key(self) -> String {
        match self {
            Owner::User(id) => format!("user:{id}"),
            Owner::Guild(id) => format!("guild:{id}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Owner> {
        let (kind, id) = key.split_once(':')?;
        let id = id.parse().ok()?;
        match kind {
            "user" => Some(Owner::User(id)),
            "guild" => Some(Owner::Guild(id)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    #[serde(rename = "_id")]
    pub key: String,
    #[serde(default)]
    pub teams: Vec<FollowedTeam>,
    /// Competition keys.
    #[serde(default)]
    pub competitions: Vec<String>,
    /// Daily summary (DM for users, a channel post for servers).
    #[serde(default)]
    pub digest: bool,
    /// Kick-off, goal and full-time alerts.
    #[serde(default)]
    pub live_alerts: bool,
    /// UTC hour of the daily summary.
    #[serde(default = "default_hour")]
    pub digest_hour: u8,
    /// Servers only: where posts go.
    #[serde(default)]
    pub channel_id: Option<i64>,
    /// UTC day (YYYY-MM-DD) the last daily summary was sent.
    #[serde(default)]
    pub last_digest: Option<String>,
    #[serde(default)]
    pub updated_by: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<DateTime>,
}

fn default_hour() -> u8 {
    9
}

impl Subscription {
    pub fn new(owner: Owner) -> Self {
        Subscription {
            key: owner.key(),
            teams: vec![],
            competitions: vec![],
            digest: false,
            live_alerts: false,
            digest_hour: default_hour(),
            channel_id: None,
            last_digest: None,
            updated_by: None,
            updated_at: None,
        }
    }

    pub fn owner(&self) -> Option<Owner> {
        Owner::from_key(&self.key)
    }

    pub fn is_empty(&self) -> bool {
        self.teams.is_empty() && self.competitions.is_empty()
    }

    /// Whether a fixture concerns this subscription: a followed team plays,
    /// or it's in a followed competition.
    pub fn concerns(&self, competition: &str, home: &str, away: &str) -> bool {
        self.competitions.iter().any(|c| c == competition) || self.teams.iter().any(|t| t.name == home || t.name == away)
    }

    pub fn follows_team(&self, home: &str, away: &str) -> bool {
        self.teams.iter().any(|t| t.name == home || t.name == away)
    }

    /// Add or remove a team. Returns Some(true) when now followed, Some(false)
    /// when removed, None when the limit is reached.
    pub fn toggle_team(&mut self, team: FollowedTeam) -> Option<bool> {
        if let Some(i) = self.teams.iter().position(|t| t.slug == team.slug) {
            self.teams.remove(i);
            return Some(false);
        }
        if self.teams.len() >= MAX_TEAMS {
            return None;
        }
        self.teams.push(team);
        Some(true)
    }

    pub fn toggle_competition(&mut self, key: &str) -> bool {
        if let Some(i) = self.competitions.iter().position(|c| c == key) {
            self.competitions.remove(i);
            false
        } else {
            self.competitions.push(key.to_string());
            true
        }
    }
}

#[derive(Clone)]
pub struct FollowStore {
    col: Collection<Subscription>,
}

impl FollowStore {
    pub fn new(db: &Database) -> Self {
        Self { col: db.collection("football_follows") }
    }

    /// The subscription, or a new empty one (not saved until `save`).
    pub async fn get(&self, owner: Owner) -> Result<Subscription> {
        Ok(self.col.find_one(doc! { "_id": owner.key() }).await?.unwrap_or_else(|| Subscription::new(owner)))
    }

    pub async fn save(&self, sub: &Subscription) -> Result<()> {
        self.col.replace_one(doc! { "_id": &sub.key }, sub).upsert(true).await?;
        Ok(())
    }

    /// Subscriptions with any notification switched on.
    pub async fn notifying(&self) -> Result<Vec<Subscription>> {
        Ok(self
            .col
            .find(doc! { "$or": [ { "digest": true }, { "live_alerts": true } ] })
            .await?
            .try_collect()
            .await?)
    }

    /// Every team someone follows (to keep their schedules fresh).
    pub async fn followed_team_slugs(&self) -> Result<Vec<String>> {
        let subs: Vec<Subscription> = self.col.find(doc! { "teams.0": { "$exists": true } }).await?.try_collect().await?;
        let mut slugs: Vec<String> = subs.into_iter().flat_map(|s| s.teams.into_iter().map(|t| t.slug)).collect();
        slugs.sort();
        slugs.dedup();
        Ok(slugs)
    }

    pub async fn mark_digest_sent(&self, key: &str, day: &str) -> Result<()> {
        self.col.update_one(doc! { "_id": key }, doc! { "$set": { "last_digest": day } }).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_and_matching() {
        let mut s = Subscription::new(Owner::User(1));
        assert!(!s.digest && !s.live_alerts, "everything starts off");
        let fla = FollowedTeam { slug: "flamengo-rj".into(), name: "Flamengo - RJ".into() };
        assert_eq!(s.toggle_team(fla.clone()), Some(true));
        assert!(s.concerns("x", "Flamengo - RJ", "Y"));
        assert_eq!(s.toggle_team(fla), Some(false));
        assert!(s.toggle_competition("brazilian-cup"));
        assert!(s.concerns("brazilian-cup", "A", "B"));
        assert!(!s.toggle_competition("brazilian-cup"));
        assert!(s.is_empty());
        for i in 0..MAX_TEAMS {
            s.toggle_team(FollowedTeam { slug: format!("t{i}"), name: format!("T{i}") });
        }
        assert_eq!(s.toggle_team(FollowedTeam { slug: "x".into(), name: "X".into() }), None);
        assert_eq!(Owner::from_key("guild:42"), Some(Owner::Guild(42)));
        assert_eq!(Owner::from_key("nope"), None);
    }
}
