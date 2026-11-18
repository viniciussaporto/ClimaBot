//! football-data.org v4 (https://docs.football-data.org/): free tier covers
//! 13 competitions (Brasileirão, Libertadores, the big five European leagues,
//! Primeira Liga, Champions League…) at 10 requests/minute. Scores on the
//! free tier are delayed, so this is only a fallback for fixtures, results,
//! tables and scorers — never for `/live`.

use super::{FetchResult, Upstream, num, str_or_empty, text, unix, unum};
use crate::football::{competitions::Competition, model::*, quota::Purpose};
use chrono::NaiveDate;
use serde_json::Value;

const BASE: &str = "https://api.football-data.org/v4";

pub struct FootballData {
    token: String,
}

impl FootballData {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    async fn get(&self, up: &Upstream, endpoint: &'static str, path: &str, purpose: Purpose) -> FetchResult<Value> {
        let url = format!("{BASE}{path}");
        up.get_json(Provider::FootballData, endpoint, &url, &[("X-Auth-Token", &self.token)], purpose).await
    }

    pub async fn fixtures(
        &self,
        up: &Upstream,
        comp: &Competition,
        code: &str,
        date: NaiveDate,
        purpose: Purpose,
    ) -> FetchResult<Vec<Fixture>> {
        let path = format!("/competitions/{code}/matches?dateFrom={date}&dateTo={date}");
        Ok(parse_matches(&self.get(up, "matches", &path, purpose).await?, comp.key))
    }

    pub async fn standings(&self, up: &Upstream, code: &str, purpose: Purpose) -> FetchResult<Standings> {
        let path = format!("/competitions/{code}/standings");
        Ok(parse_standings(&self.get(up, "standings", &path, purpose).await?))
    }

    pub async fn scorers(&self, up: &Upstream, code: &str, purpose: Purpose) -> FetchResult<Vec<Scorer>> {
        let path = format!("/competitions/{code}/scorers?limit=20");
        Ok(parse_scorers(&self.get(up, "scorers", &path, purpose).await?))
    }
}

fn team_name(t: &Value) -> Option<String> {
    text(&t["shortName"]).or_else(|| text(&t["name"]))
}

fn status(s: &str) -> (Status, &'static str) {
    match s {
        "SCHEDULED" | "TIMED" => (Status::Upcoming, "Scheduled"),
        // The free tier's scores are delayed: never present these as live.
        "IN_PLAY" => (Status::Live, "In play (delayed)"),
        "PAUSED" => (Status::Live, "Half time (delayed)"),
        "FINISHED" | "AWARDED" => (Status::Finished, "Finished"),
        "POSTPONED" => (Status::Postponed, "Postponed"),
        "SUSPENDED" => (Status::Suspended, "Suspended"),
        "CANCELLED" => (Status::Cancelled, "Cancelled"),
        _ => (Status::Other, "Unknown"),
    }
}

pub fn parse_matches(v: &Value, competition: &str) -> Vec<Fixture> {
    v["matches"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let (st, st_text) = status(m["status"].as_str().unwrap_or_default());
                    let full = &m["score"]["fullTime"];
                    let started = st != Status::Upcoming;
                    Some(Fixture {
                        id: format!("fd:{}", num(&m["id"])?),
                        slug: String::new(),
                        competition: competition.to_string(),
                        competition_name: str_or_empty(&m["competition"]["name"]),
                        home: team_name(&m["homeTeam"])?,
                        away: team_name(&m["awayTeam"])?,
                        home_score: if started { unum(&full["home"]) } else { None },
                        away_score: if started { unum(&full["away"]) } else { None },
                        status: st,
                        status_text: st_text.to_string(),
                        kickoff: unix(&m["utcDate"])?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_standings(v: &Value) -> Standings {
    let groups = v["standings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| s["type"].as_str() == Some("TOTAL"))
                .map(|s| StandingGroup {
                    name: text(&s["group"]).map(|g| g.replace('_', " ")).unwrap_or_default(),
                    rows: s["table"]
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .filter_map(|r| {
                                    Some(StandingRow {
                                        position: unum(&r["position"])?,
                                        team: team_name(&r["team"])?,
                                        team_slug: None,
                                        played: unum(&r["playedGames"]).unwrap_or(0),
                                        won: unum(&r["won"]).unwrap_or(0),
                                        drawn: unum(&r["draw"]).unwrap_or(0),
                                        lost: unum(&r["lost"]).unwrap_or(0),
                                        goals_for: num(&r["goalsFor"]).unwrap_or(0),
                                        goals_against: num(&r["goalsAgainst"]).unwrap_or(0),
                                        points: num(&r["points"]).unwrap_or(0),
                                        note: None,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .filter(|g| !g.rows.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Standings { competition_name: str_or_empty(&v["competition"]["name"]), groups }
}

pub fn parse_scorers(v: &Value) -> Vec<Scorer> {
    v["scorers"]
        .as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    Some(Scorer {
                        rank: i as u32 + 1,
                        player: text(&s["player"]["name"])?,
                        player_slug: None,
                        team: team_name(&s["team"]).unwrap_or_default(),
                        goals: unum(&s["goals"]),
                        assists: unum(&s["assists"]),
                        matches: unum(&s["playedMatches"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches() {
        let v: Value = serde_json::from_str(include_str!("../testdata/football_data_matches.json")).unwrap();
        let m = parse_matches(&v, "brazilian-serie-a");
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].home, "Bragantino");
        assert_eq!(m[0].score(), Some((2, 1)));
        assert_eq!(m[0].status, Status::Finished);
        assert!(m[0].id.starts_with("fd:") && m[0].slug.is_empty());
    }

    #[test]
    fn standings_and_scorers() {
        let v: Value = serde_json::from_str(include_str!("../testdata/football_data_standings.json")).unwrap();
        let s = parse_standings(&v);
        assert_eq!(s.groups.len(), 1);
        assert_eq!((s.groups[0].rows[0].team.as_str(), s.groups[0].rows[0].points), ("Flamengo", 76));
        let v: Value = serde_json::from_str(include_str!("../testdata/football_data_scorers.json")).unwrap();
        let sc = parse_scorers(&v);
        assert_eq!(sc[0].player, "Kevin Viveros");
        assert_eq!(sc[0].goals, Some(21));
    }

    #[test]
    fn delayed_statuses_are_labelled() {
        assert_eq!(status("IN_PLAY"), (Status::Live, "In play (delayed)"));
        assert_eq!(status("TIMED").0, Status::Upcoming);
    }
}
