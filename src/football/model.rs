//! Provider-neutral football data. Every provider adapter converts its JSON
//! into these types; commands only ever see these. Fields a source does not
//! supply stay `None`/empty: they are never filled in by guessing.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Where a piece of data came from. Shown on every reply that uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    SportScore,
    FootballData,
    ApiFootball,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::SportScore, Provider::FootballData, Provider::ApiFootball];

    /// Short id for metrics, logs and database keys.
    pub fn id(self) -> &'static str {
        match self {
            Provider::SportScore => "sportscore",
            Provider::FootballData => "football_data",
            Provider::ApiFootball => "api_football",
        }
    }

    /// Attribution as Discord markdown. SportScore's free tier requires a
    /// visible link to sportscore.com wherever its data is shown.
    pub fn attribution(self) -> &'static str {
        match self {
            Provider::SportScore => "[Powered by SportScore](https://sportscore.com/)",
            Provider::FootballData => "Data by [football-data.org](https://www.football-data.org/)",
            Provider::ApiFootball => "Data by [API-Football](https://www.api-football.com/)",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Provider::SportScore => "SportScore",
            Provider::FootballData => "football-data.org",
            Provider::ApiFootball => "API-Football",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Upcoming,
    Live,
    Finished,
    Postponed,
    Cancelled,
    Suspended,
    /// A status the source reports but we don't recognise: shown verbatim.
    Other,
}

impl Status {
    /// No further changes expected (the match is over or won't be played as scheduled).
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Finished | Status::Cancelled | Status::Postponed)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    /// The provider's match id (SportScore: the id at the end of the match
    /// URL; football-data.org: `fd:<id>`).
    pub id: String,
    /// SportScore match slug, for the detail endpoint. Empty for other providers.
    pub slug: String,
    /// Registry key of the competition, or empty when it isn't one we track.
    pub competition: String,
    /// The competition name as the provider spells it.
    pub competition_name: String,
    pub home: String,
    pub away: String,
    pub home_score: Option<u32>,
    pub away_score: Option<u32>,
    pub status: Status,
    /// The provider's own status wording ("2nd half", "Half time", …).
    pub status_text: String,
    /// Kick-off, Unix seconds (UTC).
    pub kickoff: i64,
}

impl Fixture {
    pub fn score(&self) -> Option<(u32, u32)> {
        Some((self.home_score?, self.away_score?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Home,
    Away,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Goal,
    OwnGoal,
    PenaltyGoal,
    PenaltyMissed,
    YellowCard,
    SecondYellow,
    RedCard,
    Substitution,
    Var,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub minute: Option<i64>,
    /// Stoppage time added to `minute` (45+2 → 45, 2), when the source gives it.
    pub extra: Option<i64>,
    pub kind: EventKind,
    pub side: Option<Side>,
    pub player: String,
    /// Substitutions: the player coming on (`player` is the one going off).
    pub player_in: Option<String>,
    /// The score after a goal, when the source gives it.
    pub score: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LineupPlayer {
    pub name: String,
    pub number: Option<u32>,
    /// G, D, M, F (SportScore's codes) when known.
    pub position: Option<String>,
    pub captain: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamLineup {
    pub formation: Option<String>,
    pub coach: Option<String>,
    pub starting: Vec<LineupPlayer>,
    pub substitutes: Vec<LineupPlayer>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lineups {
    /// The source says these are the official line-ups (not predicted).
    pub confirmed: bool,
    pub home: TeamLineup,
    pub away: TeamLineup,
}

impl Lineups {
    pub fn is_empty(&self) -> bool {
        self.home.starting.is_empty() && self.away.starting.is_empty()
    }
}

/// One team-statistic row ("Possession", "55%", "45%").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    pub name: String,
    pub home: String,
    pub away: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchDetail {
    pub fixture: Fixture,
    /// The match clock as the source reports it ("67", "45+2").
    pub minute: Option<String>,
    pub half_time: Option<(u32, u32)>,
    pub events: Vec<Event>,
    pub lineups: Option<Lineups>,
    pub stats: Vec<Stat>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StandingRow {
    pub position: u32,
    pub team: String,
    pub team_slug: Option<String>,
    pub played: u32,
    pub won: u32,
    pub drawn: u32,
    pub lost: u32,
    pub goals_for: i64,
    pub goals_against: i64,
    pub points: i64,
    /// Qualification / relegation zone, when the source gives one.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StandingGroup {
    /// Empty for a single league table.
    pub name: String,
    pub rows: Vec<StandingRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Standings {
    pub competition_name: String,
    pub groups: Vec<StandingGroup>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scorer {
    pub rank: u32,
    pub player: String,
    pub player_slug: Option<String>,
    pub team: String,
    pub goals: Option<u32>,
    pub assists: Option<u32>,
    pub matches: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TeamSchedule {
    pub name: String,
    pub slug: String,
    pub logo: Option<String>,
    pub matches: Vec<Fixture>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SquadPlayer {
    /// API-Football player id (missing in squads cached by older versions).
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    pub number: Option<u32>,
    pub position: Option<String>,
    pub age: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Squad {
    pub team: String,
    pub players: Vec<SquadPlayer>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub name: String,
    pub slug: String,
    pub photo: Option<String>,
    pub team: Option<String>,
    pub competition: Option<String>,
    /// Statistic name and value, in the source's order.
    pub stats: Vec<(String, String)>,
}

/// A player as API-Football knows them (for careers and absences).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerRef {
    pub id: i64,
    pub name: String,
    pub full_name: Option<String>,
    pub nationality: Option<String>,
    pub birth: Option<String>,
    pub position: Option<String>,
    pub photo: Option<String>,
}

/// One injury, illness or suspension, as reported by the source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Absence {
    pub kind: String,
    /// YYYY-MM-DD
    pub start: Option<String>,
    /// YYYY-MM-DD; None while it's ongoing with no known return date.
    pub end: Option<String>,
}

impl Absence {
    pub fn is_suspension(&self) -> bool {
        let k = self.kind.to_lowercase();
        k.contains("suspen") || k.contains("red card") || k.contains("yellow card")
    }

    /// Whether it covers the day `today` (YYYY-MM-DD; such dates compare as strings).
    pub fn is_current(&self, today: &str) -> bool {
        self.start.as_deref().is_some_and(|s| s <= today) && self.end.as_deref().is_none_or(|e| e >= today)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CareerTeam {
    pub team: String,
    /// Seasons by start year, newest first.
    pub seasons: Vec<i32>,
    pub national: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transfer {
    pub date: String,
    pub from: String,
    pub to: String,
    /// Fee or kind ("€ 80M", "Loan", "Free").
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trophy {
    pub competition: String,
    pub country: String,
    pub season: String,
    pub place: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeasonLine {
    pub season: i32,
    pub team: String,
    pub competition: String,
    pub appearances: Option<u32>,
    pub minutes: Option<u32>,
    pub goals: Option<u32>,
    pub assists: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadToHead {
    pub team1: String,
    pub team2: String,
    pub team1_wins: u32,
    pub team2_wins: u32,
    pub draws: u32,
    pub matches: Vec<Fixture>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    /// (name, slug)
    pub teams: Vec<(String, String)>,
    pub players: Vec<(String, String)>,
}
