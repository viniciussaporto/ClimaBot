//! API-Football (api-sports.io), free plan: 100 requests/day, 10/minute.
//! The free plan can't read current-season fixtures (only 2022–2024, checked
//! 2026-11-18), but team search and current squads are not season-bound, so
//! this provider is used for squads only.

use super::{FetchResult, Upstream, num, str_or_empty, text, unum};
use crate::football::{fold, model::*, quota::Purpose};
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
                    Some(TeamCandidate { id: num(&t["id"])?, name: text(&t["name"])?, country: str_or_empty(&t["country"]) })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_reserve_side(name: &str) -> bool {
    let n = format!(" {} ", fold(name).replace(['-', '_'], " "));
    [" u17 ", " u18 ", " u19 ", " u20 ", " u21 ", " u23 ", " b ", " ii ", " w ", " women ", " fem ", " feminino ", " sub 20 "]
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

#[cfg(test)]
mod tests {
    use super::*;

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
