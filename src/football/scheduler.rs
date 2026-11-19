//! The only place that calls the football providers on a schedule:
//!
//! - Fixture lists of today and tomorrow every 6 hours; yesterday's results
//!   and every competition's table once a day.
//! - The live scoreboard about once a minute, but only while a tracked match
//!   is about to start, under way, or just ended (SportScore caches for 60 s,
//!   so polling faster gains nothing).
//! - Match details every ~2.5 minutes while live, for priority competitions
//!   and matches users asked about in the last 20 minutes.
//! - When a match leaves the live list: its final details, its competition's
//!   fixture list and table are refreshed once (final verification).
//!
//! Everything goes through the shared budget, so when a provider runs low the
//! optional work (tables, details) stops before the live scoreboard does.

use super::{Football, competitions, model::*, quota::Purpose};
use crate::metrics;
use chrono::{Duration as ChronoDuration, Utc};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tracing::{debug, info, warn};

const TICK: Duration = Duration::from_secs(20);
const LIVE_EVERY: Duration = Duration::from_secs(60);
const FIXTURES_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Detail refreshes per tick, so a busy evening can't burst the budget.
const DETAILS_PER_TICK: usize = 6;
/// Gap between the background refreshes of a batch, to stay well inside
/// per-minute limits (a cached result costs nothing but this pause).
const PACE: Duration = Duration::from_millis(300);

pub async fn run(fb: Arc<Football>) {
    // Let the bot finish connecting before the first refresh.
    tokio::time::sleep(Duration::from_secs(10)).await;
    info!("Football scheduler started");
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_fixtures: Option<Instant> = None;
    let mut last_daily: Option<chrono::NaiveDate> = None;
    let mut last_live: Option<Instant> = None;
    // Live matches at the previous poll, to notice when one ends.
    let mut previous_live: HashMap<String, Fixture> = HashMap::new();
    // The first scoreboard after a start only sets the baseline: matches
    // already under way aren't announced as kick-offs.
    let mut primed = false;

    loop {
        tick.tick().await;
        let today = Utc::now().date_naive();

        if last_daily != Some(today) {
            // New UTC day: yesterday's final results, the new today/tomorrow,
            // and every table.
            let started = Instant::now();
            refresh_fixtures(&fb, &[today - ChronoDuration::days(1), today, today + ChronoDuration::days(1)]).await;
            // The week ahead, for "important upcoming matches" in follow feeds.
            let week: Vec<_> = (2..=7).map(|d| today + ChronoDuration::days(d)).collect();
            refresh_fixtures(&fb, &week).await;
            for c in competitions::COMPETITIONS {
                // Yesterday's table stays available to show position changes.
                fb.keep_previous_standings(c).await;
                if let Err(e) = fb.standings(c, Purpose::Metadata).await {
                    debug!(competition = c.key, error = %e, "Standings refresh skipped");
                }
                tokio::time::sleep(PACE).await;
            }
            // Followed teams' schedules (next and last matches for follow feeds).
            if let Some(store) = fb.follows.get() {
                match store.followed_team_slugs().await {
                    Ok(slugs) => {
                        for slug in slugs {
                            if let Err(e) = fb.team(&slug, Purpose::Metadata).await {
                                debug!(team = slug, error = %e, "Followed team refresh skipped");
                            }
                            tokio::time::sleep(PACE).await;
                        }
                    }
                    Err(e) => warn!(error = format!("{e:#}"), "Could not list followed teams"),
                }
            }
            observe("daily", started);
            last_daily = Some(today);
            last_fixtures = Some(Instant::now());
        } else if last_fixtures.is_none_or(|t| t.elapsed() >= FIXTURES_EVERY) {
            let started = Instant::now();
            refresh_fixtures(&fb, &[today, today + ChronoDuration::days(1)]).await;
            observe("fixtures", started);
            last_fixtures = Some(Instant::now());
        }

        let window = fb.in_watch_window() || !previous_live.is_empty();
        metrics::FOOTBALL_LIVE_WINDOW.set(if window { 1.0 } else { 0.0 });
        if !window {
            continue;
        }

        if last_live.is_none_or(|t| t.elapsed() >= LIVE_EVERY) {
            last_live = Some(Instant::now());
            let started = Instant::now();
            match fb.live(Purpose::Live).await {
                Ok(l) if !l.stale => {
                    metrics::FOOTBALL_LIVE_LAST_SUCCESS.set(l.value.fetched_at as f64);
                    metrics::FOOTBALL_LIVE_MATCHES.set(l.value.data.len() as f64);
                    let now_live: HashMap<String, Fixture> =
                        l.value.data.iter().map(|f| (f.id.clone(), f.clone())).collect();
                    if primed {
                        fb.publish_events(super::live_changes(&previous_live, &l.value.data));
                    }
                    for (id, f) in &previous_live {
                        if !now_live.contains_key(id)
                            && let Some(done) = verify_final(&fb, f).await
                        {
                            fb.publish_events(vec![super::LiveEvent { kind: super::LiveEventKind::FullTime, fixture: done }]);
                        }
                    }
                    previous_live = now_live;
                    primed = true;
                }
                Ok(l) => warn!(reason = l.note.unwrap_or_default(), "Live scoreboard refresh failed; serving cached data"),
                Err(e) => warn!(error = %e, "Live scoreboard refresh failed"),
            }
            observe("live", started);
        }

        let started = Instant::now();
        let mut done = 0;
        for f in previous_live.values() {
            if done >= DETAILS_PER_TICK {
                break;
            }
            if f.slug.is_empty() || !(fb.is_priority(&f.competition) || fb.is_demanded(&f.slug)) {
                continue;
            }
            // match_detail's own TTL (2.5 min while live) paces this.
            let due = fb
                .cached_now::<MatchDetail>(&format!("match:{}", f.slug))
                .is_none_or(|d| d.age() >= Duration::from_secs(150) || d.data.fixture.status != Status::Live);
            if due {
                if let Err(e) = fb.match_detail(&f.slug, Purpose::Detail).await {
                    debug!(slug = f.slug, error = %e, "Match detail refresh skipped");
                }
                done += 1;
            }
        }
        if done > 0 {
            observe("details", started);
        }
    }
}

fn observe(job: &str, started: Instant) {
    metrics::FOOTBALL_POLL_DURATION.with_label_values(&[job]).observe(started.elapsed().as_secs_f64());
}

async fn refresh_fixtures(fb: &Football, dates: &[chrono::NaiveDate]) {
    let mut failed = 0;
    for c in competitions::COMPETITIONS {
        for d in dates {
            if fb.fixtures(c, *d, Purpose::Scheduled).await.is_err() {
                failed += 1;
            }
            tokio::time::sleep(PACE).await;
        }
    }
    if failed > 0 {
        warn!(failed, "Some football fixture lists could not be refreshed");
    }
}

/// A match left the live list: confirm how it ended and refresh what depends
/// on it. Returns the match when it's confirmed finished.
async fn verify_final(fb: &Football, f: &Fixture) -> Option<Fixture> {
    info!(home = f.home, away = f.away, competition = f.competition, "Match left the live list; verifying the final score");
    let mut finished = None;
    if !f.slug.is_empty() {
        fb.expire(&format!("match:{}", f.slug));
        if let Ok(d) = fb.match_detail(&f.slug, Purpose::Detail).await
            && !d.stale
            && d.value.data.fixture.status == Status::Finished
        {
            finished = Some(d.value.data.fixture.clone());
        }
    }
    if let Some(c) = competitions::by_key(&f.competition) {
        let date = chrono::DateTime::from_timestamp(f.kickoff, 0).map(|d| d.date_naive()).unwrap_or_else(|| Utc::now().date_naive());
        let key = super::fixtures_key(c, date);
        fb.expire(&key);
        if let Ok(list) = fb.fixtures(c, date, Purpose::Detail).await
            && finished.is_none()
        {
            finished = list.value.data.into_iter().find(|x| x.id == f.id && x.status == Status::Finished);
        }
        // Tables and scorers change after a result; refresh them on next use.
        fb.expire(&format!("standings:{}", c.key));
        fb.expire(&format!("scorers:{}:goals", c.key));
        fb.expire(&format!("scorers:{}:assists", c.key));
    }
    finished
}
