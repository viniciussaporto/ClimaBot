//! Per-provider request budget and health.
//!
//! Every upstream request first reserves one call here. A call is refused
//! when the provider is in a cooldown (minute limit, daily quota, outage), has
//! a bad key, or when the day's remaining budget is below the floor kept for
//! more important work: optional refreshes stop first, the live scoreboard
//! last, and part of the budget is only ever spent on user requests.
//!
//! The daily count is persisted (see `store`), so a restart doesn't reset it,
//! and is corrected from the provider's own quota headers when it sends them.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::{fmt, time::Duration};

/// Why a request is made, from most to least important.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The global live scoreboard.
    Live,
    /// A Discord command found no fresh data.
    User,
    /// Match details of live priority fixtures, final-score verification.
    Detail,
    /// Daily / 6-hourly fixture lists.
    Scheduled,
    /// Standings, squads and other slow-changing data refreshed in the background.
    Metadata,
}

impl Purpose {
    /// Share of the daily limit that must still be left for this purpose to
    /// spend a call. User requests may use everything.
    fn floor(self) -> f64 {
        match self {
            Purpose::User => 0.0,
            Purpose::Live => 0.05,
            Purpose::Detail => 0.15,
            Purpose::Scheduled => 0.20,
            Purpose::Metadata => 0.30,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Purpose::Live => "live",
            Purpose::User => "user",
            Purpose::Detail => "detail",
            Purpose::Scheduled => "scheduled",
            Purpose::Metadata => "metadata",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Health {
    Healthy,
    /// Per-minute limit hit: wait until `until`.
    RateLimitedMinute { until: DateTime<Utc> },
    /// Daily quota used up: wait for the reset.
    DailyQuotaExhausted { until: DateTime<Utc> },
    /// The key was rejected. Needs a configuration change (and a restart).
    Unauthorized,
    /// Timeouts / 5xx: back off until `until`.
    Unavailable { until: DateTime<Utc>, failures: u32 },
}

impl Health {
    pub fn id(&self) -> &'static str {
        match self {
            Health::Healthy => "healthy",
            Health::RateLimitedMinute { .. } => "rate_limited_minute",
            Health::DailyQuotaExhausted { .. } => "daily_quota_exhausted",
            Health::Unauthorized => "unauthorized",
            Health::Unavailable { .. } => "unavailable",
        }
    }

    /// When requests may resume, if this is a temporary state.
    pub fn until(&self) -> Option<DateTime<Utc>> {
        match self {
            Health::RateLimitedMinute { until }
            | Health::DailyQuotaExhausted { until }
            | Health::Unavailable { until, .. } => Some(*until),
            _ => None,
        }
    }
}

/// Why a call wasn't allowed.
#[derive(Clone, Debug, PartialEq)]
pub enum Blocked {
    NotConfigured,
    Cooldown { state: &'static str, until: DateTime<Utc> },
    Unauthorized,
    /// The day's budget left for this purpose is used up.
    Budget,
    /// Our own per-minute pacing.
    Minute,
}

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Blocked::NotConfigured => f.write_str("not configured"),
            Blocked::Cooldown { state, until } => write!(f, "{state} until {}", until.format("%H:%M UTC")),
            Blocked::Unauthorized => f.write_str("the API key was rejected"),
            Blocked::Budget => f.write_str("today's request budget is used up"),
            Blocked::Minute => f.write_str("per-minute request limit"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub daily: u32,
    pub per_minute: u32,
}

#[derive(Debug)]
pub struct Gate {
    pub limits: Limits,
    /// UTC day the counter belongs to (YYYY-MM-DD).
    pub day: String,
    pub used: u32,
    minute_start: DateTime<Utc>,
    minute_used: u32,
    pub health: Health,
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// What a finished request tells us about the provider.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Success,
    /// A 404 or an empty result: the provider is fine.
    NotFound,
    /// The key is fine but this resource/season isn't in the plan.
    PlanRestricted,
    Unauthorized,
    MinuteLimit { retry_after: Option<Duration> },
    DailyLimit,
    /// Timeout, connection error or 5xx.
    Unavailable,
    /// The response didn't parse: keep the last good data.
    Malformed,
}

pub fn today(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

fn next_midnight(now: DateTime<Utc>) -> DateTime<Utc> {
    (now.date_naive() + ChronoDuration::days(1)).and_hms_opt(0, 0, 0).unwrap().and_utc()
}

impl Gate {
    pub fn new(limits: Limits, now: DateTime<Utc>) -> Self {
        Self {
            limits,
            day: today(now),
            used: 0,
            minute_start: now,
            minute_used: 0,
            health: Health::Healthy,
            last_success: None,
            last_error: None,
        }
    }

    fn roll(&mut self, now: DateTime<Utc>) {
        let day = today(now);
        if day != self.day {
            self.day = day;
            self.used = 0;
            if matches!(self.health, Health::DailyQuotaExhausted { .. }) {
                self.health = Health::Healthy;
            }
        }
        if now - self.minute_start >= ChronoDuration::seconds(60) {
            self.minute_start = now;
            self.minute_used = 0;
        }
        if let Some(until) = self.health.until()
            && now >= until
            && !matches!(self.health, Health::Unavailable { .. })
        {
            self.health = Health::Healthy;
        }
    }

    pub fn remaining(&self) -> u32 {
        self.limits.daily.saturating_sub(self.used)
    }

    /// Reserve one call, or say why not.
    pub fn reserve(&mut self, purpose: Purpose, now: DateTime<Utc>) -> Result<(), Blocked> {
        self.roll(now);
        match &self.health {
            Health::Unauthorized => return Err(Blocked::Unauthorized),
            Health::Unavailable { until, .. } if now < *until => {
                return Err(Blocked::Cooldown { state: "unavailable", until: *until });
            }
            h @ (Health::RateLimitedMinute { until } | Health::DailyQuotaExhausted { until }) if now < *until => {
                return Err(Blocked::Cooldown { state: h.id(), until: *until });
            }
            _ => {}
        }
        let floor = (self.limits.daily as f64 * purpose.floor()).ceil() as u32;
        if self.remaining() <= floor {
            return Err(Blocked::Budget);
        }
        if self.minute_used >= self.limits.per_minute {
            return Err(Blocked::Minute);
        }
        self.used += 1;
        self.minute_used += 1;
        Ok(())
    }

    /// Record how a request went. Returns true when the health state changed.
    pub fn record(&mut self, outcome: &Outcome, now: DateTime<Utc>) -> bool {
        let before = self.health.clone();
        match outcome {
            Outcome::Success | Outcome::NotFound | Outcome::PlanRestricted => {
                self.last_success = Some(now);
                self.health = Health::Healthy;
            }
            Outcome::Malformed => {
                self.last_error = Some("malformed response".into());
            }
            Outcome::Unauthorized => {
                self.last_error = Some("unauthorized".into());
                self.health = Health::Unauthorized;
            }
            Outcome::MinuteLimit { retry_after } => {
                let wait = retry_after.unwrap_or(Duration::from_secs(60)).min(Duration::from_secs(3600));
                self.last_error = Some("per-minute limit".into());
                self.health = Health::RateLimitedMinute {
                    until: now + ChronoDuration::from_std(wait).unwrap_or(ChronoDuration::seconds(60)),
                };
            }
            Outcome::DailyLimit => {
                self.last_error = Some("daily quota exhausted".into());
                self.used = self.used.max(self.limits.daily);
                self.health = Health::DailyQuotaExhausted { until: next_midnight(now) };
            }
            Outcome::Unavailable => {
                let failures = match self.health {
                    Health::Unavailable { failures, .. } => failures + 1,
                    _ => 1,
                };
                self.last_error = Some("unavailable".into());
                // Up to two quick retries, then exponential backoff (max 15 min).
                let wait = if failures < 3 { 0 } else { (30u64 << (failures - 3).min(5)).min(900) };
                self.health = Health::Unavailable { until: now + ChronoDuration::seconds(wait as i64), failures };
            }
        }
        before.id() != self.health.id()
    }

    /// Correct the daily counter from the provider's own report.
    pub fn reconcile(&mut self, remaining_today: Option<u32>, limit_today: Option<u32>) {
        if let Some(limit) = limit_today
            && limit > 0
        {
            self.limits.daily = self.limits.daily.min(limit);
        }
        if let Some(remaining) = remaining_today {
            self.used = self.limits.daily.saturating_sub(remaining);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(daily: u32, per_minute: u32) -> (Gate, DateTime<Utc>) {
        let now = DateTime::parse_from_rfc3339("2026-11-18T12:00:00Z").unwrap().to_utc();
        (Gate::new(Limits { daily, per_minute }, now), now)
    }

    #[test]
    fn optional_work_stops_before_user_requests() {
        let (mut g, now) = gate(100, 1000);
        g.used = 70; // 30 left
        assert_eq!(g.reserve(Purpose::Metadata, now), Err(Blocked::Budget));
        assert!(g.reserve(Purpose::Scheduled, now).is_ok());
        g.used = 95;
        assert_eq!(g.reserve(Purpose::Live, now), Err(Blocked::Budget));
        assert!(g.reserve(Purpose::User, now).is_ok());
        g.used = 100;
        assert_eq!(g.reserve(Purpose::User, now), Err(Blocked::Budget));
    }

    #[test]
    fn per_minute_pacing_and_reset() {
        let (mut g, now) = gate(100, 2);
        assert!(g.reserve(Purpose::User, now).is_ok());
        assert!(g.reserve(Purpose::User, now).is_ok());
        assert_eq!(g.reserve(Purpose::User, now), Err(Blocked::Minute));
        assert!(g.reserve(Purpose::User, now + ChronoDuration::seconds(61)).is_ok());
    }

    #[test]
    fn daily_limit_waits_for_midnight() {
        let (mut g, now) = gate(100, 10);
        g.record(&Outcome::DailyLimit, now);
        assert!(matches!(g.reserve(Purpose::User, now), Err(Blocked::Cooldown { .. })));
        let tomorrow = now + ChronoDuration::hours(12);
        assert!(g.reserve(Purpose::User, tomorrow).is_ok());
        assert_eq!(g.used, 1);
    }

    #[test]
    fn minute_limit_honours_retry_after() {
        let (mut g, now) = gate(100, 10);
        g.record(&Outcome::MinuteLimit { retry_after: Some(Duration::from_secs(20)) }, now);
        assert!(g.reserve(Purpose::User, now + ChronoDuration::seconds(10)).is_err());
        assert!(g.reserve(Purpose::User, now + ChronoDuration::seconds(21)).is_ok());
    }

    #[test]
    fn outages_back_off_and_recover() {
        let (mut g, now) = gate(100, 10);
        g.record(&Outcome::Unavailable, now);
        g.record(&Outcome::Unavailable, now);
        assert!(g.reserve(Purpose::User, now).is_ok(), "first failures retry at once");
        g.record(&Outcome::Unavailable, now);
        assert!(matches!(g.reserve(Purpose::User, now), Err(Blocked::Cooldown { .. })));
        assert!(g.reserve(Purpose::User, now + ChronoDuration::seconds(31)).is_ok());
        g.record(&Outcome::Success, now);
        assert_eq!(g.health, Health::Healthy);
    }

    #[test]
    fn bad_key_stops_everything() {
        let (mut g, now) = gate(100, 10);
        assert!(g.record(&Outcome::Unauthorized, now));
        assert_eq!(g.reserve(Purpose::User, now), Err(Blocked::Unauthorized));
    }

    #[test]
    fn plan_restriction_is_not_an_outage() {
        let (mut g, now) = gate(100, 10);
        g.record(&Outcome::PlanRestricted, now);
        assert_eq!(g.health, Health::Healthy);
    }

    #[test]
    fn provider_headers_win() {
        let (mut g, _) = gate(100, 10);
        g.used = 3;
        g.reconcile(Some(40), Some(100));
        assert_eq!(g.used, 60);
        g.reconcile(Some(0), None);
        assert_eq!(g.remaining(), 0);
    }
}
