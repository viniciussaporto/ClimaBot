//! Football data: fixtures, live scores, match details, tables, scorers,
//! teams, squads and players for Brazilian, South American and European
//! competitions (see `competitions`).
//!
//! Data flows one way: the scheduler (and, on a cache miss, a command) asks
//! the providers; results are normalized, cached in memory and MongoDB, and
//! Discord commands read the cache. Each resource has one refresh in flight
//! at a time however many users ask, and every request is budgeted per
//! provider (`quota`), so usage doesn't grow with the number of users.
//!
//! Providers:
//! - SportScore: everything, including live scores (primary).
//! - football-data.org: fixtures, tables and scorers for its 13 free
//!   competitions when SportScore fails (delayed, never used for `/live`).
//! - API-Football: squads.

pub mod competitions;
pub mod follows;
pub mod model;
pub mod providers;
pub mod quota;
pub mod scheduler;
pub mod store;

use crate::metrics;
use chrono::{NaiveDate, Utc};
use competitions::{COMPETITIONS, Competition};
use model::*;
use providers::{
    FetchError, FetchResult, Upstream, api_football::ApiFootball, football_data::FootballData, sportscore::SportScore,
};
use quota::{Limits, Purpose};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    env,
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use store::{CacheDoc, FootballStore};
use tracing::{info, warn};

static FOOTBALL: OnceLock<Arc<Football>> = OnceLock::new();

/// The running football service, once `init` has run.
pub fn get() -> Option<Arc<Football>> {
    FOOTBALL.get().cloned()
}

/// Lowercase and strip accents, for matching names typed by users.
pub fn fold(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            'ß' => 's',
            'ć' | 'č' => 'c',
            'š' => 's',
            'ž' => 'z',
            'đ' => 'd',
            other => other,
        })
        .collect()
}

/// A cached result: the data, where it came from and when.
#[derive(Clone, Debug)]
pub struct Cached<T> {
    pub data: T,
    pub provider: Provider,
    /// Unix seconds.
    pub fetched_at: i64,
}

impl<T> Cached<T> {
    pub fn age(&self) -> Duration {
        Duration::from_secs((Utc::now().timestamp() - self.fetched_at).max(0) as u64)
    }
}

/// What a command gets back: the newest data we have, and whether a refresh
/// was needed but failed (then `stale` is set and `note` says why).
#[derive(Clone, Debug)]
pub struct Lookup<T> {
    pub value: Cached<T>,
    pub stale: bool,
    pub note: Option<String>,
}

struct MemEntry {
    provider: Provider,
    fetched_at: i64,
    json: Arc<str>,
}

const MEM_LIMIT: usize = 4000;
/// Seasons API-Football's free plan serves player statistics for.
pub const SEASONS_AVAILABLE: [i32; 3] = [2024, 2023, 2022];
/// A refresh triggered by a command never makes the user wait longer than this.
const FETCH_TIMEOUT: Duration = Duration::from_secs(12);
/// Fixtures a user asked about stay "followed" (details polled while live) this long.
const DEMAND_WINDOW: Duration = Duration::from_secs(20 * 60);

pub struct Football {
    pub up: Upstream,
    ss: SportScore,
    fd: Option<FootballData>,
    af: Option<ApiFootball>,
    store: Option<FootballStore>,
    mem: Mutex<HashMap<String, MemEntry>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    demand: Mutex<HashMap<String, Instant>>,
    priority: HashSet<String>,
    /// Who follows what (None without a database, e.g. in tests).
    pub follows: OnceLock<follows::FollowStore>,
    /// Live-match changes seen by the scheduler, for the notifier.
    events_tx: tokio::sync::mpsc::UnboundedSender<LiveEvent>,
    events_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<LiveEvent>>>,
}

/// A change the scheduler noticed in the live scoreboard.
#[derive(Clone, Debug)]
pub struct LiveEvent {
    pub kind: LiveEventKind,
    pub fixture: Fixture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveEventKind {
    Kickoff,
    Goal,
    FullTime,
}

/// What changed between two live scoreboards: kick-offs and goals. (Full
/// time is reported after the final score is verified.)
pub fn live_changes(before: &HashMap<String, Fixture>, now: &[Fixture]) -> Vec<LiveEvent> {
    let mut out = vec![];
    for f in now {
        match before.get(&f.id) {
            None => out.push(LiveEvent { kind: LiveEventKind::Kickoff, fixture: f.clone() }),
            Some(old) => {
                let total = |x: &Fixture| x.home_score.unwrap_or(0) + x.away_score.unwrap_or(0);
                if total(f) > total(old) {
                    out.push(LiveEvent { kind: LiveEventKind::Goal, fixture: f.clone() });
                }
            }
        }
    }
    out
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn env_secret(name: &str) -> Option<String> {
    env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Football {
    /// Build the service from the environment. `store` is None in tests.
    pub fn from_env(store: Option<FootballStore>) -> Self {
        let fd_token = env_secret("FOOTBALL_DATA_TOKEN");
        let af_key = env_secret("API_FOOTBALL_KEY");
        let mut limits = vec![(
            Provider::SportScore,
            Limits {
                daily: env_u32("SPORTSCORE_DAILY_BUDGET", 9000),
                per_minute: env_u32("SPORTSCORE_PER_MINUTE", 300),
            },
        )];
        if fd_token.is_some() {
            // No daily cap on the free tier; 10 per minute.
            limits.push((
                Provider::FootballData,
                Limits { daily: env_u32("FOOTBALL_DATA_DAILY_BUDGET", 5000), per_minute: 9 },
            ));
        }
        if af_key.is_some() {
            limits.push((
                Provider::ApiFootball,
                Limits { daily: env_u32("API_FOOTBALL_DAILY_BUDGET", 100), per_minute: 9 },
            ));
        }
        let priority = match env::var("FOOTBALL_PRIORITY") {
            Ok(v) if !v.trim().is_empty() => v
                .split(',')
                .map(str::trim)
                .filter(|k| {
                    let known = competitions::by_key(k).is_some();
                    if !known {
                        warn!(competition = k, "Unknown competition in FOOTBALL_PRIORITY");
                    }
                    known
                })
                .map(String::from)
                .collect(),
            _ => COMPETITIONS.iter().filter(|c| c.priority).map(|c| c.key.to_string()).collect(),
        };
        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            up: Upstream::new(limits, store.clone()),
            ss: SportScore::new(env_secret("SPORTSCORE_API_KEY")),
            fd: fd_token.map(FootballData::new),
            af: af_key.map(ApiFootball::new),
            store,
            mem: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            demand: Mutex::new(HashMap::new()),
            priority,
            follows: OnceLock::new(),
            events_tx,
            events_rx: Mutex::new(Some(events_rx)),
        }
    }

    /// Hand live-match changes to the notifier (dropped when none is running).
    pub fn publish_events(&self, events: Vec<LiveEvent>) {
        for e in events {
            let _ = self.events_tx.send(e);
        }
    }

    /// The notifier's end of the live-event channel (only the first caller gets it).
    pub fn take_events(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<LiveEvent>> {
        self.events_rx.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn has_careers(&self) -> bool {
        self.af.is_some()
    }

    pub fn has_squads(&self) -> bool {
        self.af.is_some()
    }

    pub fn has_fallback(&self) -> bool {
        self.fd.is_some()
    }

    pub fn is_priority(&self, competition: &str) -> bool {
        self.priority.contains(competition)
    }

    // ── Cache ────────────────────────────────────────────────

    fn mem_get(&self, key: &str) -> Option<(Provider, i64, Arc<str>)> {
        let mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
        mem.get(key).map(|e| (e.provider, e.fetched_at, e.json.clone()))
    }

    fn mem_put(&self, key: &str, provider: Provider, fetched_at: i64, json: Arc<str>) {
        let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
        if mem.len() >= MEM_LIMIT && !mem.contains_key(key) {
            let cutoff = Utc::now().timestamp() - 86400;
            mem.retain(|_, e| e.fetched_at > cutoff);
            if mem.len() >= MEM_LIMIT {
                mem.clear();
            }
        }
        mem.insert(key.to_string(), MemEntry { provider, fetched_at, json });
    }

    /// Cached data only, from memory (then MongoDB). Never calls a provider.
    pub async fn cached<T: DeserializeOwned>(&self, key: &str) -> Option<Cached<T>> {
        let (provider, fetched_at, json) = match self.mem_get(key) {
            Some(e) => e,
            None => {
                let doc = self.store.as_ref()?.load(key).await.ok().flatten()?;
                let provider = Provider::ALL.into_iter().find(|p| p.id() == doc.provider)?;
                let fetched_at = doc.fetched_at.timestamp_millis() / 1000;
                let json: Arc<str> = doc.data.into();
                self.mem_put(key, provider, fetched_at, json.clone());
                (provider, fetched_at, json)
            }
        };
        match serde_json::from_str(&json) {
            Ok(data) => Some(Cached { data, provider, fetched_at }),
            Err(e) => {
                // A format change between versions: treat as a miss.
                warn!(key, error = %e, "Ignoring unreadable cached football data");
                None
            }
        }
    }

    /// Memory-only, synchronous read (for autocomplete, which must answer fast).
    pub fn cached_now<T: DeserializeOwned>(&self, key: &str) -> Option<Cached<T>> {
        let (provider, fetched_at, json) = self.mem_get(key)?;
        serde_json::from_str(&json).ok().map(|data| Cached { data, provider, fetched_at })
    }

    async fn put<T: Serialize>(&self, key: &str, value: &Cached<T>, retention: Duration) {
        let Ok(json) = serde_json::to_string(&value.data) else { return };
        let json: Arc<str> = json.into();
        self.mem_put(key, value.provider, value.fetched_at, json.clone());
        if let Some(store) = &self.store {
            let doc = CacheDoc {
                key: key.to_string(),
                provider: value.provider.id().to_string(),
                fetched_at: mongodb::bson::DateTime::from_millis(value.fetched_at * 1000),
                expires_at: mongodb::bson::DateTime::from_millis(
                    (Utc::now().timestamp() + retention.as_secs() as i64) * 1000,
                ),
                data: json.to_string(),
            };
            if let Err(e) = store.save(doc).await {
                warn!(key, error = format!("{e:#}"), "Could not save football data");
            }
        }
    }

    /// Mark cached data as due for a refresh, keeping it as a fallback.
    pub fn expire(&self, key: &str) {
        let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = mem.get_mut(key) {
            e.fetched_at = 0;
        }
    }

    fn lock_for(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        if locks.len() > 2000 {
            locks.retain(|_, l| Arc::strong_count(l) > 1);
        }
        locks.entry(key.to_string()).or_default().clone()
    }

    /// The newest data for `key`: from the cache while younger than `ttl`,
    /// otherwise refreshed with `fetch` (one refresh at a time per key). If
    /// the refresh fails, the old data is returned marked stale.
    async fn lookup<T, F, Fut>(
        &self,
        kind: &'static str,
        key: &str,
        ttl: impl Fn(&T) -> Duration,
        retention: Duration,
        fetch: F,
    ) -> FetchResult<Lookup<T>>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = FetchResult<(T, Provider)>>,
    {
        let fresh = |c: &Cached<T>| c.age() < ttl(&c.data);
        if let Some(c) = self.cached::<T>(key).await
            && fresh(&c)
        {
            metrics::FOOTBALL_CACHE_COUNTER.with_label_values(&[kind, "hit"]).inc();
            return Ok(Lookup { value: c, stale: false, note: None });
        }
        let lock = self.lock_for(key);
        let _guard = lock.lock().await;
        // Someone else may have refreshed it while we waited.
        let old = self.cached::<T>(key).await;
        if let Some(c) = &old
            && fresh(c)
        {
            metrics::FOOTBALL_CACHE_COUNTER.with_label_values(&[kind, "hit"]).inc();
            return Ok(Lookup { value: old.unwrap(), stale: false, note: None });
        }
        let result = tokio::time::timeout(FETCH_TIMEOUT, fetch())
            .await
            .unwrap_or_else(|_| Err(FetchError::Failed("timed out".into())));
        match (result, old) {
            (Ok((data, provider)), _) => {
                metrics::FOOTBALL_CACHE_COUNTER.with_label_values(&[kind, "refreshed"]).inc();
                let value = Cached { data, provider, fetched_at: Utc::now().timestamp() };
                self.put(key, &value, retention).await;
                Ok(Lookup { value, stale: false, note: None })
            }
            (Err(e), Some(old)) => {
                metrics::FOOTBALL_CACHE_COUNTER.with_label_values(&[kind, "stale"]).inc();
                Ok(Lookup { value: old, stale: true, note: Some(e.to_string()) })
            }
            (Err(e), None) => {
                metrics::FOOTBALL_CACHE_COUNTER.with_label_values(&[kind, "miss"]).inc();
                Err(e)
            }
        }
    }

    /// Load recently used data from MongoDB into memory after a restart, so
    /// autocomplete and the scheduler see it right away.
    pub async fn warm(&self) {
        let today = Utc::now().date_naive();
        let mut keys = vec!["live".to_string()];
        for c in COMPETITIONS {
            keys.push(format!("standings:{}", c.key));
            for d in [today.pred_opt(), Some(today), today.succ_opt()].into_iter().flatten() {
                keys.push(fixtures_key(c, d));
            }
        }
        let mut loaded = 0;
        for k in &keys {
            if self.cached::<serde_json::Value>(k).await.is_some() {
                loaded += 1;
            }
        }
        info!(loaded, "Football cache warmed from MongoDB");
    }

    /// Restore today's request counts after a restart.
    pub async fn restore_usage(&self) {
        let Some(store) = &self.store else { return };
        let day = quota::today(Utc::now());
        for p in Provider::ALL {
            if !self.up.is_configured(p) {
                continue;
            }
            match store.usage_today(p, &day).await {
                Ok(used) => {
                    self.up.with_gate(p, |g| {
                        if g.day == day {
                            g.used = g.used.max(used);
                        }
                    });
                    self.up.publish(p, false);
                }
                Err(e) => warn!(provider = p.id(), error = format!("{e:#}"), "Could not read football API usage"),
            }
        }
    }

    // ── Demand ───────────────────────────────────────────────

    /// A user asked about this match: poll its details while it's live.
    pub fn note_demand(&self, slug: &str) {
        let mut d = self.demand.lock().unwrap_or_else(|e| e.into_inner());
        d.retain(|_, t| t.elapsed() < DEMAND_WINDOW);
        d.insert(slug.to_string(), Instant::now());
    }

    pub fn is_demanded(&self, slug: &str) -> bool {
        let d = self.demand.lock().unwrap_or_else(|e| e.into_inner());
        d.get(slug).is_some_and(|t| t.elapsed() < DEMAND_WINDOW)
    }

    // ── Data ─────────────────────────────────────────────────

    pub async fn fixtures(
        &self,
        comp: &'static Competition,
        date: NaiveDate,
        purpose: Purpose,
    ) -> FetchResult<Lookup<Vec<Fixture>>> {
        let today = Utc::now().date_naive();
        let days = (date - today).num_days();
        let ttl = match days {
            ..-1 => Duration::from_secs(7 * 86400),
            -1 => Duration::from_secs(3 * 3600),
            0 => Duration::from_secs(20 * 60),
            _ => Duration::from_secs(6 * 3600),
        };
        let key = fixtures_key(comp, date);
        self.lookup(
            "fixtures",
            &key,
            |_| ttl,
            Duration::from_secs(60 * 86400),
            move || async move {
                match self.ss.fixtures(&self.up, comp, date, purpose).await {
                    Ok(f) => Ok((f, Provider::SportScore)),
                    Err(e) => match (&self.fd, comp.football_data) {
                        (Some(fd), Some(code)) if e != FetchError::NotFound => {
                            let f = fd.fixtures(&self.up, comp, code, date, purpose).await?;
                            Ok((f, Provider::FootballData))
                        }
                        _ => Err(e),
                    },
                }
            },
        )
        .await
    }

    /// Live matches in the tracked competitions.
    pub async fn live(&self, purpose: Purpose) -> FetchResult<Lookup<Vec<Fixture>>> {
        self.lookup(
            "live",
            "live",
            |_| Duration::from_secs(55),
            Duration::from_secs(86400),
            move || async move {
                let (all, truncated) = self.ss.live(&self.up, purpose).await?;
                let mut tracked: Vec<Fixture> = all.into_iter().filter(|f| !f.competition.is_empty()).collect();
                if truncated {
                    // More live matches than one page: ask per competition that has
                    // matches on right now, so none of ours are cut off.
                    let mut seen: HashSet<String> = tracked.iter().map(|f| f.id.clone()).collect();
                    for comp in self.competitions_in_window() {
                        if let Ok(extra) = self.ss.live_in(&self.up, comp, purpose).await {
                            for f in extra {
                                if seen.insert(f.id.clone()) {
                                    tracked.push(f);
                                }
                            }
                        }
                    }
                }
                tracked.sort_by(|a, b| (a.competition.as_str(), a.kickoff).cmp(&(b.competition.as_str(), b.kickoff)));
                Ok((tracked, Provider::SportScore))
            },
        )
        .await
    }

    pub async fn match_detail(&self, slug: &str, purpose: Purpose) -> FetchResult<Lookup<MatchDetail>> {
        let key = format!("match:{slug}");
        let now = Utc::now().timestamp();
        let ttl = move |d: &MatchDetail| match d.fixture.status {
            Status::Live => Duration::from_secs(150),
            Status::Finished | Status::Cancelled => {
                // Allow for late corrections in the first hours after the final whistle.
                if now - d.fixture.kickoff < 4 * 3600 {
                    Duration::from_secs(15 * 60)
                } else {
                    Duration::from_secs(30 * 86400)
                }
            }
            // Line-ups appear about an hour before kick-off.
            _ if (d.fixture.kickoff - now).abs() < 2 * 3600 => Duration::from_secs(10 * 60),
            _ => Duration::from_secs(3 * 3600),
        };
        self.lookup("match", &key, ttl, Duration::from_secs(60 * 86400), move || async move {
            Ok((self.ss.match_detail(&self.up, slug, purpose).await?, Provider::SportScore))
        })
        .await
    }

    pub async fn standings(&self, comp: &'static Competition, purpose: Purpose) -> FetchResult<Lookup<Standings>> {
        let key = format!("standings:{}", comp.key);
        self.lookup(
            "standings",
            &key,
            |_| Duration::from_secs(3 * 3600),
            Duration::from_secs(60 * 86400),
            move || async move {
                match self.ss.standings(&self.up, comp, purpose).await {
                    Ok(s) if !s.groups.is_empty() => Ok((s, Provider::SportScore)),
                    result => match (&self.fd, comp.football_data) {
                        (Some(fd), Some(code)) => {
                            Ok((fd.standings(&self.up, code, purpose).await?, Provider::FootballData))
                        }
                        _ => result.map(|s| (s, Provider::SportScore)),
                    },
                }
            },
        )
        .await
    }

    pub async fn scorers(
        &self,
        comp: &'static Competition,
        assists: bool,
        purpose: Purpose,
    ) -> FetchResult<Lookup<Vec<Scorer>>> {
        let key = format!("scorers:{}:{}", comp.key, if assists { "assists" } else { "goals" });
        self.lookup(
            "scorers",
            &key,
            |_| Duration::from_secs(6 * 3600),
            Duration::from_secs(60 * 86400),
            move || async move {
                match self.ss.scorers(&self.up, comp, assists, purpose).await {
                    Ok(s) if !s.is_empty() => Ok((s, Provider::SportScore)),
                    result => match (&self.fd, comp.football_data) {
                        (Some(fd), Some(code)) => {
                            let mut s = fd.scorers(&self.up, code, purpose).await?;
                            if assists {
                                s.sort_by_key(|x| std::cmp::Reverse(x.assists.unwrap_or(0)));
                                for (i, x) in s.iter_mut().enumerate() {
                                    x.rank = i as u32 + 1;
                                }
                            }
                            Ok((s, Provider::FootballData))
                        }
                        _ => result.map(|s| (s, Provider::SportScore)),
                    },
                }
            },
        )
        .await
    }

    pub async fn team(&self, slug: &str, purpose: Purpose) -> FetchResult<Lookup<TeamSchedule>> {
        let key = format!("team:{slug}");
        self.lookup(
            "team",
            &key,
            |_| Duration::from_secs(6 * 3600),
            Duration::from_secs(30 * 86400),
            move || async move { Ok((self.ss.team(&self.up, slug, purpose).await?, Provider::SportScore)) },
        )
        .await
    }

    pub async fn player(&self, slug: &str, purpose: Purpose) -> FetchResult<Lookup<PlayerProfile>> {
        let key = format!("player:{slug}");
        self.lookup(
            "player",
            &key,
            |_| Duration::from_secs(86400),
            Duration::from_secs(30 * 86400),
            move || async move { Ok((self.ss.player(&self.up, slug, purpose).await?, Provider::SportScore)) },
        )
        .await
    }

    pub async fn search(&self, query: &str, purpose: Purpose) -> FetchResult<Lookup<SearchResults>> {
        let key = format!("search:{}", fold(query));
        self.lookup(
            "search",
            &key,
            |_| Duration::from_secs(86400),
            Duration::from_secs(7 * 86400),
            move || async move { Ok((self.ss.search(&self.up, query, purpose).await?, Provider::SportScore)) },
        )
        .await
    }

    /// The current squad of a team (by its SportScore slug and name), from
    /// API-Football. The team's API-Football id is looked up once and kept.
    pub async fn squad(
        &self,
        team_slug: &str,
        team_name: &str,
        country: &str,
        purpose: Purpose,
    ) -> FetchResult<Lookup<Squad>> {
        let af = self.af.as_ref().ok_or(FetchError::Blocked(quota::Blocked::NotConfigured))?;
        let key = format!("squad:{team_slug}");
        self.lookup(
            "squad",
            &key,
            |_| Duration::from_secs(7 * 86400),
            Duration::from_secs(60 * 86400),
            move || async move {
                let map_key = format!("api_football:{team_slug}");
                let known = match &self.store {
                    Some(s) => s.team_id(&map_key).await.ok().flatten().map(|d| d.api_football_id),
                    None => None,
                };
                let id = match known {
                    Some(id) => id,
                    None => {
                        let candidates = af.search_teams(&self.up, team_name, purpose).await?;
                        let team = providers::api_football::pick_team(&candidates, team_name, country)
                            .ok_or(FetchError::NotFound)?;
                        if let Some(s) = &self.store
                            && let Err(e) = s.save_team_id(&map_key, team.id, &team.name).await
                        {
                            warn!(error = format!("{e:#}"), "Could not save API-Football team id");
                        }
                        team.id
                    }
                };
                Ok((af.squad(&self.up, id, purpose).await?, Provider::ApiFootball))
            },
        )
        .await
    }

    /// Head-to-head of two teams (SportScore slugs), in a stable key order.
    pub async fn h2h(&self, team1: &str, team2: &str, purpose: Purpose) -> FetchResult<Lookup<HeadToHead>> {
        let key = format!("h2h:{team1}:{team2}");
        self.lookup(
            "h2h",
            &key,
            |_| Duration::from_secs(12 * 3600),
            Duration::from_secs(30 * 86400),
            move || async move { Ok((self.ss.h2h(&self.up, team1, team2, purpose).await?, Provider::SportScore)) },
        )
        .await
    }

    /// API-Football players matching a name, best first (at most 10). Each
    /// one is remembered so later views can show who it is.
    pub async fn find_players(&self, name: &str, purpose: Purpose) -> FetchResult<Lookup<Vec<PlayerRef>>> {
        let af = self.af.as_ref().ok_or(FetchError::Blocked(quota::Blocked::NotConfigured))?;
        let query = providers::api_football::profile_query(name).ok_or(FetchError::NotFound)?;
        let key = format!("players:{}", fold(name));
        let result = self
            .lookup(
                "players",
                &key,
                |_| Duration::from_secs(7 * 86400),
                Duration::from_secs(30 * 86400),
                move || async move {
                    let found = af.search_players(&self.up, &query, purpose).await?;
                    let mut ranked = providers::api_football::rank_players(found, name);
                    ranked.truncate(10);
                    Ok((ranked, Provider::ApiFootball))
                },
            )
            .await?;
        for p in &result.value.data {
            self.remember_player(p).await;
        }
        Ok(result)
    }

    pub async fn remember_player(&self, p: &PlayerRef) {
        let value = Cached { data: p.clone(), provider: Provider::ApiFootball, fetched_at: Utc::now().timestamp() };
        self.put(&format!("player_ref:{}", p.id), &value, Duration::from_secs(90 * 86400)).await;
    }

    /// A player seen in a search or a squad (cache only).
    pub async fn player_ref(&self, id: i64) -> Option<PlayerRef> {
        self.cached::<PlayerRef>(&format!("player_ref:{id}")).await.map(|c| c.data)
    }

    /// Injuries, illnesses and suspensions of a player, newest first.
    pub async fn absences(&self, player: i64, purpose: Purpose) -> FetchResult<Lookup<Vec<Absence>>> {
        let af = self.af.as_ref().ok_or(FetchError::Blocked(quota::Blocked::NotConfigured))?;
        let key = format!("absences:{player}");
        self.lookup(
            "absences",
            &key,
            |_| Duration::from_secs(86400),
            Duration::from_secs(30 * 86400),
            move || async move { Ok((af.sidelined(&self.up, player, purpose).await?, Provider::ApiFootball)) },
        )
        .await
    }

    /// Absences already cached (never calls the provider).
    pub fn cached_absences(&self, player: i64) -> Option<Cached<Vec<Absence>>> {
        self.cached_now(&format!("absences:{player}"))
    }

    pub async fn career_teams(&self, player: &PlayerRef, purpose: Purpose) -> FetchResult<Lookup<Vec<CareerTeam>>> {
        let af = self.af.as_ref().ok_or(FetchError::Blocked(quota::Blocked::NotConfigured))?;
        let key = format!("career_teams:{}", player.id);
        self.lookup(
            "career",
            &key,
            |_| Duration::from_secs(14 * 86400),
            Duration::from_secs(90 * 86400),
            move || async move { Ok((af.player_teams(&self.up, player, purpose).await?, Provider::ApiFootball)) },
        )
        .await
    }

    pub async fn transfers(&self, player: i64, purpose: Purpose) -> FetchResult<Lookup<Vec<Transfer>>> {
        let af = self.af.as_ref().ok_or(FetchError::Blocked(quota::Blocked::NotConfigured))?;
        let key = format!("transfers:{player}");
        self.lookup(
            "career",
            &key,
            |_| Duration::from_secs(14 * 86400),
            Duration::from_secs(90 * 86400),
            move || async move { Ok((af.transfers(&self.up, player, purpose).await?, Provider::ApiFootball)) },
        )
        .await
    }

    pub async fn trophies(&self, player: i64, purpose: Purpose) -> FetchResult<Lookup<Vec<Trophy>>> {
        let af = self.af.as_ref().ok_or(FetchError::Blocked(quota::Blocked::NotConfigured))?;
        let key = format!("trophies:{player}");
        self.lookup(
            "career",
            &key,
            |_| Duration::from_secs(14 * 86400),
            Duration::from_secs(90 * 86400),
            move || async move { Ok((af.trophies(&self.up, player, purpose).await?, Provider::ApiFootball)) },
        )
        .await
    }

    /// Season-by-season statistics for the seasons the free plan serves
    /// (`SEASONS_AVAILABLE`), limited to those the player played: one
    /// request per season, cached together for a month.
    pub async fn season_lines(
        &self,
        player: &PlayerRef,
        played: &[i32],
        purpose: Purpose,
    ) -> FetchResult<Lookup<Vec<SeasonLine>>> {
        let af = self.af.as_ref().ok_or(FetchError::Blocked(quota::Blocked::NotConfigured))?;
        let key = format!("seasons:{}", player.id);
        let seasons: Vec<i32> = SEASONS_AVAILABLE.iter().copied().filter(|s| played.contains(s)).collect();
        self.lookup(
            "career",
            &key,
            |_| Duration::from_secs(30 * 86400),
            Duration::from_secs(90 * 86400),
            move || async move {
                let mut lines = vec![];
                for s in &seasons {
                    lines.extend(af.season(&self.up, player.id, *s, purpose).await?);
                }
                Ok((lines, Provider::ApiFootball))
            },
        )
        .await
    }

    /// Before the daily table refresh: keep the current table as yesterday's,
    /// so position changes can be shown.
    pub async fn keep_previous_standings(&self, comp: &Competition) {
        let key = format!("standings:{}", comp.key);
        if let Some(cur) = self.cached::<Standings>(&key).await
            && cur.age() > Duration::from_secs(12 * 3600)
        {
            self.put(&format!("standings_prev:{}", comp.key), &cur, Duration::from_secs(30 * 86400)).await;
            self.expire(&key);
        }
    }

    /// Fixtures from today up to `days` ahead in the tracked competitions
    /// (cache only), sorted by kick-off.
    pub fn upcoming_fixtures(&self, days: i64) -> Vec<Fixture> {
        let today = Utc::now().date_naive();
        let mut out: Vec<Fixture> = vec![];
        for c in COMPETITIONS {
            for d in 0..=days {
                if let Some(list) = self.cached_now::<Vec<Fixture>>(&fixtures_key(c, today + chrono::Duration::days(d)))
                {
                    out.extend(list.data);
                }
            }
        }
        out.sort_by_key(|f| f.kickoff);
        out.dedup_by(|a, b| a.id == b.id);
        out
    }

    pub async fn api_football_status(&self) -> Option<FetchResult<String>> {
        Some(self.af.as_ref()?.status(&self.up).await)
    }

    // ── Indexes over the cache (no provider calls) ───────────

    /// Fixtures of yesterday, today and tomorrow in all tracked competitions,
    /// with the live scoreboard applied on top when it's newer.
    pub fn recent_fixtures(&self) -> Vec<(Fixture, i64)> {
        let today = Utc::now().date_naive();
        let mut by_id: HashMap<String, (Fixture, i64)> = HashMap::new();
        for c in COMPETITIONS {
            for d in [today.pred_opt(), Some(today), today.succ_opt()].into_iter().flatten() {
                if let Some(list) = self.cached_now::<Vec<Fixture>>(&fixtures_key(c, d)) {
                    for f in list.data {
                        by_id.insert(f.id.clone(), (f, list.fetched_at));
                    }
                }
            }
        }
        if let Some(live) = self.cached_now::<Vec<Fixture>>("live") {
            for f in live.data {
                match by_id.get(&f.id) {
                    Some((_, at)) if *at > live.fetched_at => {}
                    _ => {
                        by_id.insert(f.id.clone(), (f, live.fetched_at));
                    }
                }
            }
        }
        let mut all: Vec<(Fixture, i64)> = by_id.into_values().collect();
        all.sort_by_key(|(f, _)| f.kickoff);
        all
    }

    /// Whether any tracked match is about to start, under way, or just ended.
    pub fn in_watch_window(&self) -> bool {
        !self.competitions_in_window().is_empty()
            || self.cached_now::<Vec<Fixture>>("live").is_some_and(|l| !l.data.is_empty())
    }

    /// Competitions with a match from 15 minutes before kick-off until it ends
    /// (or 3 hours after kick-off if the end wasn't seen).
    pub fn competitions_in_window(&self) -> Vec<&'static Competition> {
        let now = Utc::now().timestamp();
        let mut keys: Vec<&'static Competition> = vec![];
        for (f, _) in self.recent_fixtures() {
            let open = now >= f.kickoff - 15 * 60 && now <= f.kickoff + 3 * 3600 && !f.status.is_terminal();
            if open
                && let Some(c) = competitions::by_key(&f.competition)
                && !keys.iter().any(|k| k.key == c.key)
            {
                keys.push(c);
            }
        }
        keys
    }

    /// Teams seen in the cached tables: (name, slug, competition).
    pub fn known_teams(&self) -> Vec<(String, String, &'static Competition)> {
        let mut out: Vec<(String, String, &'static Competition)> = vec![];
        for c in COMPETITIONS {
            if let Some(s) = self.cached_now::<Standings>(&format!("standings:{}", c.key)) {
                for g in s.data.groups {
                    for r in g.rows {
                        if let Some(slug) = r.team_slug
                            && !out.iter().any(|(_, s, _)| *s == slug)
                        {
                            out.push((r.team, slug, c));
                        }
                    }
                }
            }
        }
        out
    }
}

pub fn fixtures_key(comp: &Competition, date: NaiveDate) -> String {
    format!("fixtures:{}:{date}", comp.key)
}

/// Start the service: connect its collections, restore state, start the scheduler.
pub async fn init(db: &mongodb::Database) -> Arc<Football> {
    let store = match FootballStore::new(db).await {
        Ok(s) => Some(s),
        Err(e) => {
            warn!(error = format!("{e:#}"), "Football data won't be persisted (MongoDB indexes failed)");
            None
        }
    };
    let fb = Arc::new(Football::from_env(store));
    fb.restore_usage().await;
    fb.warm().await;
    for p in Provider::ALL {
        fb.up.publish(p, false);
    }
    info!(
        fallback = fb.has_fallback(),
        squads = fb.has_squads(),
        priority = fb.priority.len(),
        "Football data service ready"
    );
    let _ = fb.follows.set(follows::FollowStore::new(db));
    let _ = FOOTBALL.set(fb.clone());
    tokio::spawn(scheduler::run(fb.clone()));
    fb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        assert_eq!(fold("  São Paulo "), "sao paulo");
        assert_eq!(fold("Grêmio"), "gremio");
        assert_eq!(fold("Atlético-MG"), "atletico-mg");
    }

    fn service() -> Football {
        Football::from_env(None)
    }

    #[tokio::test]
    async fn single_flight_and_stale_fallback() {
        let fb = service();
        let counter = std::sync::atomic::AtomicU32::new(0);
        let calls = &counter;
        let fetch = move || async move {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok((vec![1u32, 2, 3], Provider::SportScore))
        };
        let ttl = |_: &Vec<u32>| Duration::from_secs(60);
        let (a, b) = tokio::join!(
            fb.lookup("test", "k", ttl, Duration::from_secs(60), fetch),
            fb.lookup("test", "k", ttl, Duration::from_secs(60), fetch)
        );
        assert_eq!(a.unwrap().value.data, vec![1, 2, 3]);
        assert_eq!(b.unwrap().value.data, vec![1, 2, 3]);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1, "one refresh for two requests");

        // Expired, and the provider fails: the old data comes back, marked stale.
        fb.expire("k");
        let failing = move || async move { Err::<(Vec<u32>, Provider), _>(FetchError::Failed("down".into())) };
        let l = fb.lookup("test", "k", ttl, Duration::from_secs(60), failing).await.unwrap();
        assert!(l.stale);
        assert_eq!(l.value.data, vec![1, 2, 3]);
        assert_eq!(l.note.as_deref(), Some("down"));

        // Nothing cached and the provider fails: an error, not made-up data.
        let miss = fb.lookup("test", "other", ttl, Duration::from_secs(60), failing).await;
        assert!(miss.is_err());
    }

    #[test]
    fn watch_window() {
        let fb = service();
        let now = Utc::now().timestamp();
        let mk = |id: &str, kickoff: i64, status: Status| Fixture {
            id: id.into(),
            slug: id.into(),
            competition: "brazilian-serie-a".into(),
            competition_name: "Brazilian Serie A".into(),
            home: "A".into(),
            away: "B".into(),
            home_score: None,
            away_score: None,
            status,
            status_text: String::new(),
            kickoff,
        };
        let comp = competitions::by_key("brazilian-serie-a").unwrap();
        let key = fixtures_key(comp, Utc::now().date_naive());
        let put = |list: Vec<Fixture>| {
            fb.mem_put(&key, Provider::SportScore, now, serde_json::to_string(&list).unwrap().into());
        };
        put(vec![mk("1", now + 3 * 3600, Status::Upcoming)]);
        assert!(!fb.in_watch_window(), "kick-off is hours away");
        put(vec![mk("1", now + 10 * 60, Status::Upcoming)]);
        assert!(fb.in_watch_window(), "kick-off in 10 minutes");
        put(vec![mk("1", now - 2 * 3600, Status::Finished)]);
        assert!(!fb.in_watch_window(), "already finished");
        assert_eq!(fb.recent_fixtures().len(), 1);
    }
}
