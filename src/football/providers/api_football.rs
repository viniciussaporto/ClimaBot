//! API-Football (api-sports.io), free plan: 100 requests/day, 10/minute.
//! The free plan can't read current-season fixtures (only 2022–2024, checked
//! 2026-11-18), but team search and current squads are not season-bound, so
//! this provider is used for squads only.

use super::{FetchResult, Upstream, num, str_or_empty, text, unum};
use crate::football::{
    fold,
    model::{Absence, CareerTeam, PlayerRef, Provider, SeasonLine, Squad, SquadPlayer, Transfer, Trophy},
    quota::Purpose,
};
use serde_json::Value;

const BASE: &str = "https://v3.football.api-sports.io";

pub struct ApiFootball {
    key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TeamCandidate {
    pub id: i64,
    pub name: String,
    pub country: String,
}

impl ApiFootball {
    pub fn new(key: String) -> Self {
        Self { key }
    }

    async fn get(&self, up: &Upstream, endpoint: &'static str, path: &str, purpose: Purpose) -> FetchResult<Value> {
        let url = format!("{BASE}{path}");
        up.get_json(Provider::ApiFootball, endpoint, &url, &[("x-apisports-key", &self.key)], purpose).await
    }

    pub async fn search_teams(&self, up: &Upstream, name: &str, purpose: Purpose) -> FetchResult<Vec<TeamCandidate>> {
        let path = format!("/teams?search={}", super::sportscore::urlencode(&search_term(name)));
        Ok(parse_teams(&self.get(up, "teams", &path, purpose).await?))
    }

    pub async fn squad(&self, up: &Upstream, team_id: i64, purpose: Purpose) -> FetchResult<Squad> {
        let path = format!("/players/squads?team={team_id}");
        parse_squad(&self.get(up, "squads", &path, purpose).await?)
    }

    /// Players whose last name matches (3+ letters). One page (up to 250).
    pub async fn search_players(&self, up: &Upstream, query: &str, purpose: Purpose) -> FetchResult<Vec<PlayerRef>> {
        let path = format!("/players/profiles?search={}", super::sportscore::urlencode(query));
        Ok(parse_profiles(&self.get(up, "profiles", &path, purpose).await?))
    }

    /// Injuries, illnesses and suspensions, newest first. Not season-bound.
    pub async fn sidelined(&self, up: &Upstream, player: i64, purpose: Purpose) -> FetchResult<Vec<Absence>> {
        Ok(parse_sidelined(&self.get(up, "sidelined", &format!("/sidelined?player={player}"), purpose).await?))
    }

    /// Clubs and national teams, with the seasons played for each.
    pub async fn player_teams(
        &self,
        up: &Upstream,
        player: &PlayerRef,
        purpose: Purpose,
    ) -> FetchResult<Vec<CareerTeam>> {
        let v = self.get(up, "player_teams", &format!("/players/teams?player={}", player.id), purpose).await?;
        Ok(parse_player_teams(&v, player.nationality.as_deref().unwrap_or("")))
    }

    pub async fn transfers(&self, up: &Upstream, player: i64, purpose: Purpose) -> FetchResult<Vec<Transfer>> {
        Ok(parse_transfers(&self.get(up, "transfers", &format!("/transfers?player={player}"), purpose).await?))
    }

    pub async fn trophies(&self, up: &Upstream, player: i64, purpose: Purpose) -> FetchResult<Vec<Trophy>> {
        Ok(parse_trophies(&self.get(up, "trophies", &format!("/trophies?player={player}"), purpose).await?))
    }

    /// Per-competition statistics of one season. The free plan only serves
    /// seasons 2022 to 2024.
    pub async fn season(
        &self,
        up: &Upstream,
        player: i64,
        season: i32,
        purpose: Purpose,
    ) -> FetchResult<Vec<SeasonLine>> {
        let v = self.get(up, "player_season", &format!("/players?id={player}&season={season}"), purpose).await?;
        Ok(parse_season(&v, season))
    }

    /// Whether the key works (the status endpoint doesn't count against the quota).
    pub async fn status(&self, up: &Upstream) -> FetchResult<String> {
        let v = self.get(up, "status", "/status", Purpose::User).await?;
        let r = &v["response"];
        Ok(format!(
            "plan {}, {} of {} requests used today",
            str_or_empty(&r["subscription"]["plan"]),
            num(&r["requests"]["current"]).unwrap_or(0),
            num(&r["requests"]["limit_day"]).unwrap_or(0)
        ))
    }
}

/// API-Football's search accepts letters, digits and spaces only (3+ chars).
/// SportScore names carry a state suffix ("Chapecoense - SC"); drop it.
pub fn search_term(name: &str) -> String {
    let base = name.split(" - ").next().unwrap_or(name);
    let base = base.split('(').next().unwrap_or(base);
    let cleaned: String = fold(base).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_teams(v: &Value) -> Vec<TeamCandidate> {
    v["response"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let t = &r["team"];
                    Some(TeamCandidate {
                        id: num(&t["id"])?,
                        name: text(&t["name"])?,
                        country: str_or_empty(&t["country"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_reserve_side(name: &str) -> bool {
    let n = format!(" {} ", fold(name).replace(['-', '_'], " "));
    [
        " u17 ",
        " u18 ",
        " u19 ",
        " u20 ",
        " u21 ",
        " u23 ",
        " b ",
        " ii ",
        " w ",
        " women ",
        " fem ",
        " feminino ",
        " sub 20 ",
    ]
    .iter()
    .any(|m| n.contains(m))
}

/// The senior team best matching `name` (in `country` when known).
pub fn pick_team(candidates: &[TeamCandidate], name: &str, country: &str) -> Option<TeamCandidate> {
    let want = search_term(name);
    let senior: Vec<&TeamCandidate> = candidates
        .iter()
        .filter(|c| !is_reserve_side(&c.name))
        .filter(|c| country.is_empty() || c.country.eq_ignore_ascii_case(country))
        .collect();
    senior
        .iter()
        .find(|c| search_term(&c.name) == want)
        .or_else(|| senior.iter().find(|c| search_term(&c.name).starts_with(&want)))
        .or_else(|| senior.first())
        .map(|c| (*c).clone())
}

pub fn parse_squad(v: &Value) -> FetchResult<Squad> {
    let Some(entry) = v["response"].as_array().and_then(|a| a.first()) else {
        return Err(super::FetchError::NotFound);
    };
    Ok(Squad {
        team: str_or_empty(&entry["team"]["name"]),
        players: entry["players"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some(SquadPlayer {
                            id: num(&p["id"]),
                            name: text(&p["name"])?,
                            number: unum(&p["number"]),
                            position: text(&p["position"]),
                            age: unum(&p["age"]),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

pub fn parse_profiles(v: &Value) -> Vec<PlayerRef> {
    v["response"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let p = &r["player"];
                    let full: Vec<String> =
                        [text(&p["firstname"]), text(&p["lastname"])].into_iter().flatten().collect();
                    Some(PlayerRef {
                        id: num(&p["id"])?,
                        name: text(&p["name"])?,
                        full_name: (!full.is_empty()).then(|| full.join(" ")),
                        nationality: text(&p["nationality"]),
                        birth: text(&p["birth"]["date"]),
                        position: text(&p["position"]),
                        photo: text(&p["photo"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn words(s: &str) -> Vec<String> {
    fold(s).split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(String::from).collect()
}

/// How well an API-Football profile matches a name: shared words, and a bonus
/// when the short name matches exactly.
pub fn name_score(p: &PlayerRef, name: &str) -> usize {
    let want = words(name);
    let mut have = words(&p.name);
    if let Some(f) = &p.full_name {
        have.extend(words(f));
    }
    let shared = want.iter().filter(|w| have.contains(w)).count();
    shared * 2 + usize::from(words(&p.name) == want)
}

/// Candidates sharing a word with `name`, best match first.
pub fn rank_players(mut list: Vec<PlayerRef>, name: &str) -> Vec<PlayerRef> {
    list.retain(|p| name_score(p, name) > 0);
    list.sort_by_key(|p| std::cmp::Reverse(name_score(p, name)));
    list
}

/// The word to search API-Football's profiles by: its search matches last
/// names, so use the name's longest word (3+ letters).
pub fn profile_query(name: &str) -> Option<String> {
    words(name).into_iter().filter(|w| w.len() >= 3).max_by_key(String::len)
}

pub fn parse_sidelined(v: &Value) -> Vec<Absence> {
    let mut out: Vec<Absence> = v["response"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(Absence { kind: text(&r["type"])?, start: text(&r["start"]), end: text(&r["end"]) })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.start.cmp(&a.start));
    out
}

pub fn parse_player_teams(v: &Value, nationality: &str) -> Vec<CareerTeam> {
    let nat = fold(nationality);
    v["response"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let team = text(&r["team"]["name"])?.split_whitespace().collect::<Vec<_>>().join(" ");
                    let mut seasons: Vec<i32> = r["seasons"]
                        .as_array()
                        .map(|s| s.iter().filter_map(|x| num(x).map(|n| n as i32)).collect())
                        .unwrap_or_default();
                    seasons.sort_unstable_by(|a, b| b.cmp(a));
                    let t = fold(&team);
                    // National teams carry the country's name (Brazil, Brazil U23).
                    let national = !nat.is_empty() && (t == nat || t.starts_with(&format!("{nat} ")));
                    Some(CareerTeam { team, seasons, national })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_transfers(v: &Value) -> Vec<Transfer> {
    let mut out: Vec<Transfer> = v["response"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r["transfers"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    Some(Transfer {
                        date: text(&t["date"])?,
                        from: text(&t["teams"]["out"]["name"]).unwrap_or_else(|| "?".into()),
                        to: text(&t["teams"]["in"]["name"]).unwrap_or_else(|| "?".into()),
                        kind: text(&t["type"]).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.date.cmp(&a.date));
    out
}

pub fn parse_trophies(v: &Value) -> Vec<Trophy> {
    v["response"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    Some(Trophy {
                        competition: text(&t["league"])?,
                        country: str_or_empty(&t["country"]),
                        season: str_or_empty(&t["season"]),
                        place: str_or_empty(&t["place"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_season(v: &Value, season: i32) -> Vec<SeasonLine> {
    v["response"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r["statistics"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(SeasonLine {
                        season,
                        team: text(&s["team"]["name"])?,
                        competition: str_or_empty(&s["league"]["name"]),
                        appearances: unum(&s["games"]["appearences"]),
                        minutes: unum(&s["games"]["minutes"]),
                        goals: unum(&s["goals"]["total"]),
                        assists: unum(&s["goals"]["assists"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn career_parsers() {
        let teams = parse_player_teams(&sample(include_str!("../testdata/api_football_player_teams.json")), "Brazil");
        let brazil = teams.iter().find(|t| t.team == "Brazil").unwrap();
        assert!(brazil.national && brazil.seasons[0] >= brazil.seasons[1]);
        assert!(teams.iter().any(|t| t.team == "Brazil U23" && t.national));
        assert!(teams.iter().any(|t| t.team == "Barcelona" && !t.national));
        let tr = parse_transfers(&sample(include_str!("../testdata/api_football_transfers.json")));
        assert_eq!(tr[0].to, "Al-Hilal Saudi FC");
        assert!(tr.windows(2).all(|w| w[0].date >= w[1].date));
        let tp = parse_trophies(&sample(include_str!("../testdata/api_football_trophies.json")));
        assert_eq!(tp[0].place, "Winner");
        let s = parse_season(&sample(include_str!("../testdata/api_football_player_season.json")), 2023);
        assert!(s.iter().any(|l| l.competition == "Pro League" && l.appearances == Some(3)));
    }

    #[test]
    fn absences() {
        let a = parse_sidelined(&sample(include_str!("../testdata/api_football_sidelined.json")));
        assert_eq!(a[0].kind, "Thigh Injury");
        assert!(a[0].end.is_none(), "ongoing, no return date");
        assert!(a[0].is_current("2026-01-01"));
        assert!(!a[1].is_current("2026-01-01"));
        assert!(Absence { kind: "Red Card Suspension".into(), start: None, end: None }.is_suspension());
    }

    #[test]
    fn profile_matching() {
        let list = parse_profiles(&sample(include_str!("../testdata/api_football_profiles.json")));
        assert!(!list.is_empty());
        let ranked = rank_players(list.clone(), "Pedro Neto");
        assert_eq!(ranked[0].name, "Pedro Neto");
        assert!(rank_players(list, "Zzyzx").is_empty());
        assert_eq!(profile_query("Giorgian de Arrascaeta").as_deref(), Some("arrascaeta"));
        assert_eq!(profile_query("Jo"), None);
    }

    #[test]
    fn search_terms() {
        assert_eq!(search_term("Chapecoense - SC"), "chapecoense");
        assert_eq!(search_term("São Paulo - SP"), "sao paulo");
        assert_eq!(search_term("Flamengo (AA)"), "flamengo");
        assert_eq!(search_term("Atlético-MG"), "atletico mg");
    }

    #[test]
    fn picks_the_senior_team() {
        let c = vec![
            TeamCandidate { id: 12995, name: "Chapecoense U20".into(), country: "Brazil".into() },
            TeamCandidate { id: 132, name: "Chapecoense-sc".into(), country: "Brazil".into() },
            TeamCandidate { id: 22722, name: "Chapecoense B".into(), country: "Brazil".into() },
        ];
        assert_eq!(pick_team(&c, "Chapecoense - SC", "Brazil").unwrap().id, 132);
        assert_eq!(pick_team(&c, "Chapecoense - SC", "England"), None);
        assert_eq!(pick_team(&c, "Chapecoense - SC", "").unwrap().id, 132);
    }

    #[test]
    fn squad() {
        let v: Value = serde_json::from_str(include_str!("../testdata/api_football_squad.json")).unwrap();
        let s = parse_squad(&v).unwrap();
        assert_eq!(s.team, "Chapecoense-sc");
        assert_eq!(s.players.len(), 4);
        assert_eq!(s.players[0].position.as_deref(), Some("Goalkeeper"));
        assert!(parse_squad(&serde_json::json!({"response": []})).is_err());
    }
}
