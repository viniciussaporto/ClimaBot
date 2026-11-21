//! SportScore (https://sportscore.com/developers/): live scores, fixtures,
//! match details (events, line-ups), standings, top scorers, team schedules
//! and player statistics. Free with attribution; ~10,000 requests/day/IP, or
//! more with a registered key (sent as `X-Api-Key`). Responses are cached
//! for 60 s at their edge, so polling faster than that gains nothing.

use super::{FetchError, FetchResult, Upstream, num, str_or_empty, text, unix, unum};
use crate::football::{
    competitions::{self, Competition},
    model::{
        Event, EventKind, Fixture, HeadToHead, LineupPlayer, Lineups, MatchDetail, PlayerProfile, Provider, Scorer,
        SearchResults, Side, StandingGroup, StandingRow, Standings, Stat, Status, TeamLineup, TeamSchedule,
    },
    quota::Purpose,
};
use chrono::NaiveDate;
use serde_json::Value;

const BASE: &str = "https://sportscore.com";

/// SportScore returns at most this many matches per list request.
pub const LIST_LIMIT: usize = 200;

pub struct SportScore {
    key: Option<String>,
}

impl SportScore {
    pub fn new(key: Option<String>) -> Self {
        Self { key: key.filter(|k| !k.trim().is_empty()) }
    }

    async fn get(&self, up: &Upstream, endpoint: &'static str, path: &str, purpose: Purpose) -> FetchResult<Value> {
        let url = format!("{BASE}{path}");
        let headers: Vec<(&str, &str)> = self.key.as_deref().map(|k| ("X-Api-Key", k)).into_iter().collect();
        up.get_json(Provider::SportScore, endpoint, &url, &headers, purpose).await
    }

    pub async fn fixtures(
        &self,
        up: &Upstream,
        comp: &Competition,
        date: NaiveDate,
        purpose: Purpose,
    ) -> FetchResult<Vec<Fixture>> {
        let path = format!("/api/v1/fixtures/?sport=football&date={date}&competition={}&limit={LIST_LIMIT}", comp.key);
        Ok(parse_matches(&self.get(up, "fixtures", &path, purpose).await?))
    }

    /// Every match live right now, worldwide. The second value is true when
    /// the list was cut at [`LIST_LIMIT`].
    pub async fn live(&self, up: &Upstream, purpose: Purpose) -> FetchResult<(Vec<Fixture>, bool)> {
        let path = format!("/api/v1/fixtures/?sport=football&status=live&limit={LIST_LIMIT}");
        let all = parse_matches(&self.get(up, "live", &path, purpose).await?);
        let truncated = all.len() >= LIST_LIMIT;
        Ok((all, truncated))
    }

    /// Live matches of one competition (used when the global list is truncated).
    pub async fn live_in(&self, up: &Upstream, comp: &Competition, purpose: Purpose) -> FetchResult<Vec<Fixture>> {
        let path = format!("/api/v1/fixtures/?sport=football&status=live&competition={}&limit={LIST_LIMIT}", comp.key);
        Ok(parse_matches(&self.get(up, "live", &path, purpose).await?))
    }

    pub async fn match_detail(&self, up: &Upstream, slug: &str, purpose: Purpose) -> FetchResult<MatchDetail> {
        let path = format!("/api/v1/match/?sport=football&slug={}", urlencode(slug));
        parse_match(&self.get(up, "match", &path, purpose).await?)
    }

    pub async fn standings(&self, up: &Upstream, comp: &Competition, purpose: Purpose) -> FetchResult<Standings> {
        let path = format!("/api/v1/standings/?sport=football&slug={}", comp.key);
        parse_standings(&self.get(up, "standings", &path, purpose).await?)
    }

    pub async fn scorers(
        &self,
        up: &Upstream,
        comp: &Competition,
        assists: bool,
        purpose: Purpose,
    ) -> FetchResult<Vec<Scorer>> {
        let stat = if assists { "assists" } else { "goals" };
        let path = format!("/api/v1/topscorers/?sport=football&slug={}&limit=20&stat={stat}", comp.key);
        Ok(parse_scorers(&self.get(up, "topscorers", &path, purpose).await?))
    }

    pub async fn team(&self, up: &Upstream, slug: &str, purpose: Purpose) -> FetchResult<TeamSchedule> {
        let path = format!("/api/v1/team/?sport=football&slug={}&limit=30", urlencode(slug));
        parse_team(&self.get(up, "team", &path, purpose).await?)
    }

    pub async fn player(&self, up: &Upstream, slug: &str, purpose: Purpose) -> FetchResult<PlayerProfile> {
        let path = format!("/api/widget/player/?sport=football&slug={}", urlencode(slug));
        parse_player(&self.get(up, "player", &path, purpose).await?)
    }

    /// Recent meetings of two teams (by slug) and the win/draw summary.
    pub async fn h2h(&self, up: &Upstream, team1: &str, team2: &str, purpose: Purpose) -> FetchResult<HeadToHead> {
        let path =
            format!("/api/v1/h2h/?sport=football&team1={}&team2={}&limit=10", urlencode(team1), urlencode(team2));
        Ok(parse_h2h(&self.get(up, "h2h", &path, purpose).await?))
    }

    pub async fn search(&self, up: &Upstream, query: &str, purpose: Purpose) -> FetchResult<SearchResults> {
        let path = format!("/api/v1/search/?sport=football&limit=10&q={}", urlencode(query));
        Ok(parse_search(&self.get(up, "search", &path, purpose).await?))
    }
}

pub fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// `/football/match/<slug>/<id>/` → (slug, id)
fn match_ids(url: &str) -> (String, String) {
    let parts: Vec<&str> = url.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        ["football", "match", slug, id, ..] => (slug.to_string(), id.to_string()),
        ["football", "match", slug] => (slug.to_string(), slug.to_string()),
        _ => (String::new(), String::new()),
    }
}

pub fn status_of(status: &str, status_text: &str) -> Status {
    let t = status_text.to_lowercase();
    match status {
        "live" => Status::Live,
        "finished" => Status::Finished,
        _ if t.contains("postpon") => Status::Postponed,
        _ if t.contains("cancel") || t.contains("abandon") => Status::Cancelled,
        _ if t.contains("suspend") || t.contains("interrupt") => Status::Suspended,
        "upcoming" | "scheduled" | "notstarted" => Status::Upcoming,
        _ => Status::Other,
    }
}

fn parse_fixture(m: &Value) -> Option<Fixture> {
    let home = text(&m["home"])?;
    let away = text(&m["away"])?;
    let kickoff = unix(&m["time"])?;
    let (mut slug, id) = match_ids(m["url"].as_str().unwrap_or_default());
    if let Some(s) = text(&m["slug"]) {
        slug = s;
    }
    let competition_name = str_or_empty(&m["competition"]);
    let status_text = str_or_empty(&m["status_text"]);
    let status = status_of(m["status"].as_str().unwrap_or_default(), &status_text);
    Some(Fixture {
        id: if id.is_empty() { slug.clone() } else { id },
        slug,
        competition: competitions::by_sportscore_name(&competition_name).map(|c| c.key.to_string()).unwrap_or_default(),
        competition_name,
        home,
        away,
        // Scores of matches that haven't started are meaningless ("0" or "").
        home_score: if status == Status::Upcoming { None } else { unum(&m["home_score"]) },
        away_score: if status == Status::Upcoming { None } else { unum(&m["away_score"]) },
        status,
        status_text,
        kickoff,
    })
}

pub fn parse_matches(v: &Value) -> Vec<Fixture> {
    v["matches"].as_array().map(|a| a.iter().filter_map(parse_fixture).collect()).unwrap_or_default()
}

fn malformed(what: &str) -> FetchError {
    FetchError::Failed(format!("unexpected SportScore {what} response"))
}

fn event_kind(kind: &str) -> EventKind {
    match kind.to_lowercase().as_str() {
        "goal" => EventKind::Goal,
        "own goal" => EventKind::OwnGoal,
        "penalty goal" | "penalty" => EventKind::PenaltyGoal,
        "missed penalty" | "penalty missed" => EventKind::PenaltyMissed,
        "yellow card" => EventKind::YellowCard,
        "second yellow" | "yellow red card" | "second yellow card" => EventKind::SecondYellow,
        "red card" => EventKind::RedCard,
        "substitution" => EventKind::Substitution,
        "var" => EventKind::Var,
        other => EventKind::Other(other.to_string()),
    }
}

fn lineup_players(v: &Value) -> Vec<LineupPlayer> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(LineupPlayer {
                        name: text(&p["name"])?,
                        number: unum(&p["number"]),
                        position: text(&p["position"]),
                        captain: p["captain"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Team statistics: the format isn't documented (and was empty in every
/// sample), so accept the usual shapes and skip anything else.
fn parse_stats(v: &Value) -> Vec<Stat> {
    let Some(items) = v.as_array() else { return vec![] };
    items
        .iter()
        .filter_map(|s| {
            let name = ["name", "type", "stat", "title"].iter().find_map(|k| text(&s[*k]))?;
            let home = ["home", "home_value"].iter().find_map(|k| text(&s[*k]))?;
            let away = ["away", "away_value"].iter().find_map(|k| text(&s[*k]))?;
            Some(Stat { name, home, away })
        })
        .collect()
}

pub fn parse_match(v: &Value) -> FetchResult<MatchDetail> {
    let m = &v["match"];
    let fixture = parse_fixture(m).ok_or_else(|| malformed("match"))?;
    let events = m["incidents"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| {
                    let side = match i["side"].as_str() {
                        Some("home") => Some(Side::Home),
                        Some("away") => Some(Side::Away),
                        _ => None,
                    };
                    let kind = event_kind(i["type"].as_str().unwrap_or_default());
                    let (player, player_in) = if kind == EventKind::Substitution {
                        (str_or_empty(&i["player_out"]), text(&i["player_in"]))
                    } else {
                        (str_or_empty(&i["player"]), None)
                    };
                    Event {
                        minute: num(&i["time"]),
                        extra: ["addtime", "add_time", "extra", "added_time"].iter().find_map(|k| num(&i[*k])),
                        kind,
                        side,
                        player,
                        player_in,
                        score: unum(&i["home_score"]).zip(unum(&i["away_score"])),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let l = &m["lineups"];
    let lineups = l.is_object().then(|| Lineups {
        confirmed: l["confirmed"].as_bool().unwrap_or(false),
        home: TeamLineup {
            formation: text(&l["home_formation"]),
            coach: text(&l["home_coach"]),
            starting: lineup_players(&l["home_xi"]),
            substitutes: lineup_players(&l["home_subs"]),
        },
        away: TeamLineup {
            formation: text(&l["away_formation"]),
            coach: text(&l["away_coach"]),
            starting: lineup_players(&l["away_xi"]),
            substitutes: lineup_players(&l["away_subs"]),
        },
    });
    Ok(MatchDetail {
        minute: if fixture.status == Status::Live { text(&m["live_minute"]) } else { None },
        half_time: unum(&m["home_ht_score"]).zip(unum(&m["away_ht_score"])),
        events,
        lineups: lineups.filter(|l| !l.is_empty()),
        stats: parse_stats(&m["stats"]),
        fixture,
    })
}

pub fn parse_standings(v: &Value) -> FetchResult<Standings> {
    let tables = v["tables"].as_array().ok_or_else(|| malformed("standings"))?;
    let groups = tables
        .iter()
        .map(|t| StandingGroup {
            name: str_or_empty(&t["group"]),
            rows: t["rows"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|r| {
                            Some(StandingRow {
                                position: unum(&r["pos"])?,
                                team: text(&r["team"])?,
                                team_slug: text(&r["team_slug"]),
                                played: unum(&r["p"]).unwrap_or(0),
                                won: unum(&r["w"]).unwrap_or(0),
                                drawn: unum(&r["d"]).unwrap_or(0),
                                lost: unum(&r["l"]).unwrap_or(0),
                                goals_for: num(&r["gf"]).unwrap_or(0),
                                goals_against: num(&r["ga"]).unwrap_or(0),
                                points: num(&r["pts"]).unwrap_or(0),
                                note: text(&r["promo_name"]),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .filter(|g| !g.rows.is_empty())
        .collect();
    Ok(Standings { competition_name: str_or_empty(&v["competition"]), groups })
}

pub fn parse_scorers(v: &Value) -> Vec<Scorer> {
    v["scorers"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(Scorer {
                        rank: unum(&s["rank"])?,
                        player: text(&s["player"])?,
                        player_slug: text(&s["player_slug"]),
                        team: str_or_empty(&s["team"]),
                        goals: unum(&s["goals"]),
                        assists: unum(&s["assists"]),
                        matches: unum(&s["matches"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_team(v: &Value) -> FetchResult<TeamSchedule> {
    let t = &v["team"];
    let mut matches = parse_matches(v);
    matches.sort_by_key(|m| m.kickoff);
    Ok(TeamSchedule {
        name: text(&t["name"]).ok_or_else(|| malformed("team"))?,
        slug: str_or_empty(&t["slug"]),
        logo: text(&t["logo"]),
        matches,
    })
}

const PLAYER_STATS: &[(&str, &str)] = &[
    ("matches", "Matches"),
    ("goals", "Goals"),
    ("assists", "Assists"),
    ("minutes", "Minutes"),
    ("shots", "Shots"),
    ("shots_on_target", "Shots on target"),
    ("key_passes", "Key passes"),
    ("passes", "Passes"),
    ("dribbles", "Dribbles"),
    ("tackles", "Tackles"),
    ("interceptions", "Interceptions"),
    ("yellow_cards", "Yellow cards"),
    ("red_cards", "Red cards"),
];

pub fn parse_player(v: &Value) -> FetchResult<PlayerProfile> {
    let p = &v["player"];
    let s = &v["stats"];
    Ok(PlayerProfile {
        name: text(&p["name"]).ok_or_else(|| malformed("player"))?,
        slug: str_or_empty(&p["slug"]),
        photo: text(&p["logo"]),
        team: text(&s["team"]),
        competition: text(&s["competition"]),
        stats: PLAYER_STATS.iter().filter_map(|(k, label)| text(&s[*k]).map(|v| (label.to_string(), v))).collect(),
    })
}

pub fn parse_h2h(v: &Value) -> HeadToHead {
    let s = &v["summary"];
    HeadToHead {
        team1: str_or_empty(&v["team1"]),
        team2: str_or_empty(&v["team2"]),
        team1_wins: unum(&s["team1_wins"]).unwrap_or(0),
        team2_wins: unum(&s["team2_wins"]).unwrap_or(0),
        draws: unum(&s["draws"]).unwrap_or(0),
        matches: parse_matches(v),
    }
}

pub fn parse_search(v: &Value) -> SearchResults {
    let pairs = |k: &str| {
        v[k].as_array()
            .map(|a| a.iter().filter_map(|t| Some((text(&t["name"])?, text(&t["slug"])?))).collect())
            .unwrap_or_default()
    };
    SearchResults { teams: pairs("teams"), players: pairs("players") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Value {
        let s = match name {
            "live" => include_str!("../testdata/sportscore_live.json"),
            "fixtures" => include_str!("../testdata/sportscore_fixtures.json"),
            "match" => include_str!("../testdata/sportscore_match.json"),
            "standings" => include_str!("../testdata/sportscore_standings.json"),
            "groups" => include_str!("../testdata/sportscore_standings_groups.json"),
            "scorers" => include_str!("../testdata/sportscore_topscorers.json"),
            "team" => include_str!("../testdata/sportscore_team.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn fixtures() {
        let f = parse_matches(&sample("fixtures"));
        assert_eq!(f.len(), 3);
        let serie_a = &f[0];
        assert_eq!(serie_a.competition, "brazilian-serie-a");
        assert_eq!(serie_a.home, "Chapecoense - SC");
        assert_eq!(serie_a.status, Status::Upcoming);
        assert_eq!(serie_a.score(), None, "no score before kick-off");
        assert!(!serie_a.slug.is_empty() && !serie_a.id.is_empty() && serie_a.slug != serie_a.id);
        // Not a tracked competition.
        assert_eq!(f[2].competition, "", "not a tracked competition");
    }

    #[test]
    fn live() {
        let f = parse_matches(&sample("live"));
        assert_ne!(f.len(), 0);
        assert!(f.iter().all(|m| m.status == Status::Live && m.score().is_some()));
    }

    #[test]
    fn match_detail() {
        let d = parse_match(&sample("match")).unwrap();
        assert_eq!(d.fixture.home, "Ipswich Town");
        assert_eq!(d.fixture.score(), Some((1, 2)));
        assert_eq!(d.fixture.status, Status::Finished);
        assert_eq!(d.fixture.competition, "english-premier-league");
        assert_eq!(d.half_time, Some((0, 1)));
        assert_eq!(d.minute, None);
        let goal = &d.events[0];
        assert_eq!(goal.kind, EventKind::Goal);
        assert_eq!(goal.side, Some(Side::Away));
        assert_eq!(goal.score, Some((0, 1)));
        assert!(d.events.iter().any(|e| e.kind == EventKind::OwnGoal));
        let sub = d.events.iter().find(|e| e.kind == EventKind::Substitution).unwrap();
        assert!(sub.player_in.is_some() && !sub.player.is_empty());
        let l = d.lineups.unwrap();
        assert!(l.confirmed);
        assert_eq!(l.home.formation.as_deref(), Some("4-2-3-1"));
        assert_eq!(l.home.starting.len(), 3);
        assert!(d.stats.is_empty(), "SportScore sent no statistics for this match");
    }

    #[test]
    fn standings() {
        let s = parse_standings(&sample("standings")).unwrap();
        assert_eq!(s.groups.len(), 1);
        let top = &s.groups[0].rows[0];
        assert_eq!((top.position, top.team.as_str(), top.points), (1, "Flamengo - RJ", 76));
        assert_eq!(top.team_slug.as_deref(), Some("flamengo-rj"));
        let g = parse_standings(&sample("groups")).unwrap();
        assert_eq!(g.groups.len(), 2);
        assert_eq!(g.groups[0].name, "Group 1");
    }

    #[test]
    fn scorers_team_search() {
        let s = parse_scorers(&sample("scorers"));
        assert_eq!(s[0].player, "Kevin Viveros");
        assert_eq!(s[0].goals, Some(21));
        let t = parse_team(&sample("team")).unwrap();
        assert_ne!(t.matches.len(), 0);
        assert!(t.matches.windows(2).all(|w| w[0].kickoff <= w[1].kickoff));
        let r = parse_search(&serde_json::json!({"teams":[{"name":"Benfica","slug":"benfica"}],"players":[]}));
        assert_eq!(r.teams, vec![("Benfica".into(), "benfica".into())]);
    }

    #[test]
    fn player() {
        let v = serde_json::json!({"player":{"name":"Lionel Messi","slug":"lionel-messi","logo":"x"},
            "stats":{"team":"Inter Miami CF","competition":"MLS","matches":31,"goals":26,"assists":13,"red_cards":0}});
        let p = parse_player(&v).unwrap();
        assert_eq!(p.team.as_deref(), Some("Inter Miami CF"));
        assert_eq!(p.stats[0], ("Matches".into(), "31".into()));
        assert!(p.stats.contains(&("Red cards".into(), "0".into())));
        let none = parse_player(&serde_json::json!({"player":{"name":"X","slug":"x"},"stats":null})).unwrap();
        assert_eq!(none.stats, vec![]);
    }

    #[test]
    fn head_to_head() {
        let h = parse_h2h(&serde_json::from_str(include_str!("../testdata/sportscore_h2h.json")).unwrap());
        assert_eq!((h.team1_wins, h.team2_wins, h.draws), (3, 1, 1));
        assert_eq!(h.matches.len(), 3);
        assert!(h.matches.iter().all(|m| m.score().is_some()));
    }

    #[test]
    fn statuses() {
        assert_eq!(status_of("upcoming", "Postponed"), Status::Postponed);
        assert_eq!(status_of("live", "Half time"), Status::Live);
        assert_eq!(status_of("x", "Abandoned"), Status::Cancelled);
        assert_eq!(status_of("weird", "?"), Status::Other);
    }
}
