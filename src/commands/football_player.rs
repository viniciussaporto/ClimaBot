//! `/football player career|injuries`, `/football team injuries|compare`.
//!
//! Careers, absences and the player lookup come from API-Football (free
//! plan: 100 requests a day), through the football service's cache: each
//! player is looked up once and kept for weeks, and every view says what the
//! free plan can't provide.

use super::football::{
    COLOR, Reply, TeamRef, clip, comp_label, explain, fixture_line, notice, ordinal, resolve_team, result_letter,
    source_note, team_buttons, unavailable,
};
use crate::football::{
    Football, SEASONS_AVAILABLE,
    competitions::COMPETITIONS,
    model::{Absence, CareerTeam, PlayerRef, Provider, SquadPlayer, Standings, Status, Trophy},
    providers::FetchError,
    quota::Purpose,
};
use anyhow::Result;
use chrono::{NaiveDate, Utc};
use serenity::all::{
    ButtonStyle, ComponentInteraction, ComponentInteractionDataKind, CreateActionRow, CreateButton, CreateEmbed,
    CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption,
};

const VIEWS: [(&str, &str); 5] = [
    ("clubs", "Clubs"),
    ("seasons", "Seasons"),
    ("transfers", "Transfers"),
    ("trophies", "Trophies"),
    ("injuries", "Injuries"),
];

fn day(d: &str) -> String {
    NaiveDate::parse_from_str(d, "%Y-%m-%d").map_or_else(|_| d.to_string(), |d| d.format("%d %b %Y").to_string())
}

fn no_key() -> Reply {
    notice("⚠️ Careers and injuries need API-Football, which isn't configured on this bot.")
}

fn view_buttons(id: i64, current: &str) -> Vec<CreateActionRow> {
    vec![CreateActionRow::Buttons(
        VIEWS
            .iter()
            .map(|(v, label)| {
                CreateButton::new(format!("fbc:{v}:{id}"))
                    .label(*label)
                    .style(if *v == current { ButtonStyle::Primary } else { ButtonStyle::Secondary })
                    .disabled(*v == current)
            })
            .collect(),
    )]
}

fn header(p: &PlayerRef) -> String {
    let birth = p.birth.as_deref().map(|b| format!("born {}", day(b)));
    let parts: Vec<String> =
        [p.full_name.clone(), p.nationality.clone(), p.position.clone(), birth].into_iter().flatten().collect();
    parts.join(" · ")
}

fn player_embed(p: &PlayerRef, section: &str) -> CreateEmbed {
    let mut e = CreateEmbed::new().title(format!("👤 {} · {section}", p.name)).color(COLOR);
    if let Some(photo) = &p.photo {
        e = e.thumbnail(photo);
    }
    e
}

fn candidate_label(p: &PlayerRef) -> String {
    let year = p.birth.as_deref().and_then(|b| b.get(..4));
    let parts: Vec<&str> =
        [Some(p.name.as_str()), p.nationality.as_deref(), p.position.as_deref(), year].into_iter().flatten().collect();
    clip(&parts.join(" · "), 100)
}

/// `/football player career|injuries <name>`: find the player, then show `view`.
pub(super) async fn career(fb: &Football, input: &str, view: &str) -> Result<Reply> {
    if !fb.has_careers() {
        return Ok(no_key());
    }
    // Autocomplete values are SportScore slugs ("kevin-viveros").
    let name = if input.contains(' ') { input.to_string() } else { input.replace('-', " ") };
    let found = match fb.find_players(&name, Purpose::User).await {
        Ok(l) => l.value.data,
        Err(FetchError::NotFound) => vec![],
        Err(e) => return Ok(unavailable("Player search", &e)),
    };
    let Some(best) = found.first() else {
        return Ok(notice(format!(
            "❌ The career data source has no player called **{}**. Try their full name.",
            clip(&name, 60)
        )));
    };
    let score = |p: &PlayerRef| crate::football::providers::api_football::name_score(p, &name);
    let ambiguous = found.len() > 1 && score(&found[1]) == score(best);
    if !ambiguous {
        return show(fb, best.id, view).await;
    }
    let options =
        found.iter().take(10).map(|p| CreateSelectMenuOption::new(candidate_label(p), p.id.to_string())).collect();
    let menu = CreateSelectMenu::new(format!("fbc:pick:{view}"), CreateSelectMenuKind::String { options })
        .placeholder("Which player?");
    Ok(Reply {
        embed: CreateEmbed::new()
            .title("👤 Which player?")
            .description(format!(
                "Several players match **{}**. Pick one below.\n\n{}",
                clip(&name, 60),
                Provider::ApiFootball.attribution()
            ))
            .color(COLOR),
        rows: vec![CreateActionRow::SelectMenu(menu)],
    })
}

/// One view of a player's profile, by API-Football id.
pub(super) async fn show(fb: &Football, id: i64, view: &str) -> Result<Reply> {
    if !fb.has_careers() {
        return Ok(no_key());
    }
    let Some(p) = fb.player_ref(id).await else {
        return Ok(notice("This list has expired: run the command again."));
    };
    let mut reply = match view {
        "injuries" => injuries(fb, &p).await,
        "seasons" => seasons(fb, &p).await,
        "transfers" => transfers(fb, &p).await,
        "trophies" => trophies(fb, &p).await,
        _ => clubs(fb, &p).await,
    };
    reply.rows.extend(view_buttons(id, view));
    Ok(reply)
}

fn season_span(seasons: &[i32]) -> String {
    let mut s = seasons.to_vec();
    s.sort_unstable();
    // Collapse consecutive years: 2009–2013, 2025–2026.
    let mut parts = vec![];
    let mut i = 0;
    while i < s.len() {
        let start = s[i];
        while i + 1 < s.len() && s[i + 1] == s[i] + 1 {
            i += 1;
        }
        parts.push(if s[i] == start { start.to_string() } else { format!("{start}–{}", s[i]) });
        i += 1;
    }
    parts.reverse();
    parts.join(", ")
}

async fn clubs(fb: &Football, p: &PlayerRef) -> Reply {
    let lookup = match fb.career_teams(p, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return unavailable("This career", &e),
    };
    let teams = &lookup.value.data;
    let (national, clubs): (Vec<&CareerTeam>, Vec<&CareerTeam>) = teams.iter().partition(|t| t.national);
    let mut clubs = clubs;
    clubs.sort_by_key(|t| std::cmp::Reverse(t.seasons.first().copied().unwrap_or(0)));
    let current = clubs.first().map(|t| t.team.clone());
    let club_lines: Vec<String> =
        clubs.iter().map(|t| format!("**{}** · {}", t.team, season_span(&t.seasons))).collect();
    let nat_lines: Vec<String> = national
        .iter()
        .map(|t| {
            format!(
                "**{}** · {} season{} ({})",
                t.team,
                t.seasons.len(),
                if t.seasons.len() == 1 { "" } else { "s" },
                season_span(&t.seasons)
            )
        })
        .collect();
    let mut e = player_embed(p, "career").description(format!(
        "{}\n{}\n\n{}",
        header(p),
        current.map(|c| format!("Latest club: **{c}**")).unwrap_or_default(),
        source_note(&lookup)
    ));
    e = e.field(
        "Clubs (seasons by start year)",
        if club_lines.is_empty() {
            "None reported by the data source.".into()
        } else {
            clip(&club_lines.join("\n"), 1024)
        },
        false,
    );
    e = e.field(
        "National teams",
        if nat_lines.is_empty() {
            "None reported by the data source.".into()
        } else {
            clip(
                &format!(
                    "{}\nAppearance counts aren't in the free plan; per-season numbers for 2022–2024 are under **Seasons**.",
                    nat_lines.join("\n")
                ),
                1024,
            )
        },
        false,
    );
    Reply { embed: e, rows: vec![] }
}

async fn seasons(fb: &Football, p: &PlayerRef) -> Reply {
    let played: Vec<i32> = match fb.career_teams(p, Purpose::User).await {
        Ok(l) => l.value.data.iter().flat_map(|t| t.seasons.clone()).collect(),
        Err(e) => return unavailable("This career", &e),
    };
    let lookup = match fb.season_lines(p, &played, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return unavailable("Season statistics", &e),
    };
    let mut lines = lookup.value.data.clone();
    lines.sort_by(|a, b| b.season.cmp(&a.season).then(b.appearances.cmp(&a.appearances)));
    let mut by_season: Vec<(i32, Vec<String>)> = vec![];
    for l in lines.iter().filter(|l| l.appearances.unwrap_or(0) > 0) {
        let text = format!(
            "{} · {} — {} apps, {} goals, {} assists{}",
            l.team,
            l.competition,
            l.appearances.unwrap_or(0),
            l.goals.unwrap_or(0),
            l.assists.unwrap_or(0),
            l.minutes.map(|m| format!(", {m}'")).unwrap_or_default()
        );
        match by_season.iter_mut().find(|(s, _)| *s == l.season) {
            Some((_, v)) => v.push(text),
            None => by_season.push((l.season, vec![text])),
        }
    }
    let mut e = player_embed(p, "seasons").description(format!(
        "{}\n\nPer competition, for the seasons the free plan covers ({}). This season: `/football player stats`.\n\n{}",
        header(p),
        SEASONS_AVAILABLE.iter().rev().map(ToString::to_string).collect::<Vec<_>>().join(", "),
        source_note(&lookup)
    ));
    if by_season.is_empty() {
        e = e.field("No data", "The data source has no appearances for this player in those seasons.", false);
    }
    for (season, rows) in by_season.into_iter().take(6) {
        let total_goals: u32 = lines.iter().filter(|l| l.season == season).filter_map(|l| l.goals).sum();
        let total_assists: u32 = lines.iter().filter(|l| l.season == season).filter_map(|l| l.assists).sum();
        e = e.field(
            format!("{season} · {total_goals} goals, {total_assists} assists"),
            clip(&rows.join("\n"), 1024),
            false,
        );
    }
    Reply { embed: e, rows: vec![] }
}

async fn transfers(fb: &Football, p: &PlayerRef) -> Reply {
    let lookup = match fb.transfers(p.id, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return unavailable("Transfers", &e),
    };
    let lines: Vec<String> = lookup
        .value
        .data
        .iter()
        .take(20)
        .map(|t| {
            let kind = if t.kind.is_empty() || t.kind == "N/A" { String::new() } else { format!(" ({})", t.kind) };
            format!("`{}` {} → **{}**{kind}", day(&t.date), t.from, t.to)
        })
        .collect();
    let body =
        if lines.is_empty() { "No transfers reported by the data source.".to_string() } else { lines.join("\n") };
    Reply {
        embed: player_embed(p, "transfers")
            .description(clip(&format!("{}\n\n{body}\n\n{}", header(p), source_note(&lookup)), 4000)),
        rows: vec![],
    }
}

async fn trophies(fb: &Football, p: &PlayerRef) -> Reply {
    let lookup = match fb.trophies(p.id, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return unavailable("Trophies", &e),
    };
    let all = &lookup.value.data;
    let won: Vec<&Trophy> = all.iter().filter(|t| t.place.eq_ignore_ascii_case("winner")).collect();
    let others: Vec<&Trophy> = all.iter().filter(|t| !t.place.eq_ignore_ascii_case("winner")).collect();
    // One line per competition: "Ligue 1 (France) ×5 · 2022/2023, 2021/2022, …"
    let grouped = |list: &[&Trophy], with_place: bool| -> String {
        let mut groups: Vec<(String, Vec<String>)> = vec![];
        for t in list {
            let key = if with_place && !t.place.is_empty() {
                format!("{} ({}) · {}", t.competition, t.country, t.place)
            } else {
                format!("{} ({})", t.competition, t.country)
            };
            let season = t.season.trim().to_string();
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, s)) => {
                    if !season.is_empty() && !s.contains(&season) {
                        s.push(season);
                    }
                }
                None => groups.push((key, if season.is_empty() { vec![] } else { vec![season] })),
            }
        }
        let lines: Vec<String> = groups
            .iter()
            .map(|(k, seasons)| match seasons.len() {
                0 => k.clone(),
                1 => format!("{k} {}", seasons[0]),
                n => format!("{k} ×{n} · {}", seasons.join(", ")),
            })
            .collect();
        let mut out = String::new();
        for (i, l) in lines.iter().enumerate() {
            if out.len() + l.len() > 960 {
                out.push_str(&format!("…and {} more", lines.len() - i));
                break;
            }
            out.push_str(l);
            out.push('\n');
        }
        out
    };
    let mut e = player_embed(p, "trophies").description(format!(
        "{}\n\n**{} title{}**, {} other final placing{}.\n\n{}",
        header(p),
        won.len(),
        if won.len() == 1 { "" } else { "s" },
        others.len(),
        if others.len() == 1 { "" } else { "s" },
        source_note(&lookup)
    ));
    if all.is_empty() {
        e = e.field("Trophies", "None reported by the data source.", false);
    }
    if !won.is_empty() {
        e = e.field("🏆 Won", grouped(&won, false), false);
    }
    if !others.is_empty() {
        e = e.field("🥈 Runner-up and other placings", grouped(&others, true), false);
    }
    Reply { embed: e, rows: vec![] }
}

// ─────────────────────────────────────────────
//  Injuries and suspensions
// ─────────────────────────────────────────────

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn absence_icon(a: &Absence) -> &'static str {
    if a.is_suspension() { "🟥" } else { "🩹" }
}

/// The absence that applies today: the newest entry, if it covers today. The
/// source sometimes leaves older entries open (no end date) after a newer
/// one started; those don't count.
pub(super) fn current_absence<'a>(abs: &'a [Absence], today: &str) -> Option<&'a Absence> {
    abs.first().filter(|a| a.is_current(today))
}

fn absence_line(a: &Absence, today: &str, newest: bool) -> String {
    let start = a.start.as_deref().map_or_else(|| "?".into(), day);
    let end = match (&a.end, newest && a.is_current(today)) {
        (Some(e), true) => format!("expected back **{}**", day(e)),
        (None, true) => "ongoing, **return date unknown**".to_string(),
        (Some(e), false) => format!("until {}", day(e)),
        (None, false) => "end not reported".to_string(),
    };
    format!("{} **{}** · from {start}, {end}", absence_icon(a), a.kind)
}

/// Availability in one line: out (with why and until when) or available.
pub(super) fn availability(abs: &[Absence], today: &str) -> String {
    match current_absence(abs, today) {
        Some(a) if a.is_suspension() => format!("🟥 **Suspended** · {}", absence_line(a, today, true)),
        Some(a) => format!("❌ **Out** · {}", absence_line(a, today, true)),
        None => "✅ **No current absence reported**".to_string(),
    }
}

/// Whether the source seems to have stopped tracking the player.
fn coverage_warning(abs: &[Absence]) -> Option<String> {
    let latest = abs.iter().filter_map(|a| a.end.clone().or_else(|| a.start.clone())).max();
    let cutoff = (Utc::now() - chrono::Duration::days(365)).format("%Y-%m-%d").to_string();
    match latest {
        None => Some("The data source has no absences on record for this player: it may not track them.".into()),
        Some(d) if d < cutoff => Some(format!(
            "The data source's latest entry for this player is from {}: recent absences may be missing \
             (coverage of Brazilian competitions is often incomplete).",
            day(&d)
        )),
        _ => None,
    }
}

async fn injuries(fb: &Football, p: &PlayerRef) -> Reply {
    let lookup = match fb.absences(p.id, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return unavailable("Injury data", &e),
    };
    let today = today();
    let abs = &lookup.value.data;
    let mut text = format!("{}\n\n**Availability:** {}", header(p), availability(abs, &today));
    if let Some(w) = coverage_warning(abs) {
        text.push_str(&format!("\n⚠️ {w}"));
    }
    let history: Vec<String> = abs.iter().take(12).enumerate().map(|(i, a)| absence_line(a, &today, i == 0)).collect();
    let mut e = player_embed(p, "injuries and suspensions").description(format!(
        "{text}\n\nSuspensions only appear when the data source reports them; bans for accumulated cards are usually missing.\n\n{}",
        source_note(&lookup)
    ));
    if !history.is_empty() {
        e = e.field("History (newest first)", clip(&history.join("\n"), 1024), false);
    }
    Reply { embed: e, rows: vec![] }
}

/// `/football team injuries`: the free sources have no team injury lists, so
/// show what's known (players already checked) and let users check a player.
pub(super) async fn team_injuries(fb: &Football, t: &TeamRef) -> Result<Reply> {
    if !fb.has_squads() {
        return Ok(no_key());
    }
    let squad = match fb.squad(&t.slug, &t.name, t.country, Purpose::User).await {
        Ok(l) => l,
        Err(FetchError::NotFound) => {
            return Ok(notice(format!("❌ The squad data source has no team matching **{}**.", t.name)));
        }
        Err(e) => return Ok(unavailable("This squad", &e)),
    };
    let today = today();
    let mut out_lines = vec![];
    let mut checked = 0;
    let mut players: Vec<&SquadPlayer> = squad.value.data.players.iter().filter(|p| p.id.is_some()).collect();
    players.sort_by_key(|p| (super::football::position_rank(p.position.as_deref()).0, p.number.unwrap_or(999)));
    for p in &players {
        let id = p.id.unwrap_or_default();
        if fb.player_ref(id).await.is_none() {
            fb.remember_player(&PlayerRef {
                id,
                name: p.name.clone(),
                full_name: None,
                nationality: None,
                birth: None,
                position: p.position.clone(),
                photo: None,
            })
            .await;
        }
        if let Some(abs) = fb.cached_absences(id) {
            checked += 1;
            if current_absence(&abs.data, &today).is_some() {
                out_lines.push(format!("**{}** · {}", p.name, availability(&abs.data, &today)));
            }
        }
    }
    let mut e = CreateEmbed::new().title(format!("🩹 {} · injuries and suspensions", t.name)).color(COLOR).description(
        format!(
            "ℹ️ The free data sources have **no current injury or suspension list** for teams, so this is \
             built player by player: pick a player below to check their availability (from their injury \
             history). Suspensions for accumulated cards are usually missing, and coverage of Brazilian \
             competitions is often incomplete.\n\nChecked so far: {checked} of {} players.\n\n{}",
            players.len(),
            Provider::ApiFootball.attribution()
        ),
    );
    e = e.field(
        "Out right now (among checked players)",
        if out_lines.is_empty() { "Nobody reported out.".to_string() } else { clip(&out_lines.join("\n"), 1024) },
        false,
    );
    let mut rows = vec![];
    for (i, chunk) in players.chunks(25).take(2).enumerate() {
        let options = chunk
            .iter()
            .map(|p| {
                let num = p.number.map(|n| format!("{n} · ")).unwrap_or_default();
                let mark = match fb.cached_absences(p.id.unwrap_or_default()) {
                    Some(a) if current_absence(&a.data, &today).is_some() => " ❌",
                    Some(_) => " ✅",
                    None => "",
                };
                CreateSelectMenuOption::new(
                    clip(&format!("{num}{}{mark}", p.name), 100),
                    p.id.unwrap_or_default().to_string(),
                )
                .description(p.position.clone().unwrap_or_else(|| "Player".into()))
            })
            .collect();
        let placeholder = if i == 0 { "Check a player's availability…" } else { "…more players" };
        rows.push(CreateActionRow::SelectMenu(
            CreateSelectMenu::new(format!("fbc:pick:injuries:{i}"), CreateSelectMenuKind::String { options })
                .placeholder(placeholder),
        ));
    }
    rows.extend(team_buttons(&t.slug, "injuries"));
    Ok(Reply { embed: e, rows })
}

// ─────────────────────────────────────────────
//  Team comparison
// ─────────────────────────────────────────────

pub(super) async fn compare(fb: &Football, args: &super::football::Args) -> Result<Reply> {
    let a = match resolve_team(fb, args.get("team").unwrap_or_default()).await {
        Ok(t) => t,
        Err(r) => return Ok(r),
    };
    let b = match resolve_team(fb, args.get("other").unwrap_or_default()).await {
        Ok(t) => t,
        Err(r) => return Ok(r),
    };
    if a.slug == b.slug {
        return Ok(notice("❌ Pick two different teams."));
    }
    let h = match fb.h2h(&a.slug, &b.slug, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return Ok(notice(format!("⚠️ The head-to-head record is unavailable: {}.", explain(&e)))),
    };
    let rec = &h.value.data;
    let mut e = CreateEmbed::new().title(format!("⚔️ {} vs {}", a.name, b.name)).color(COLOR);
    let meetings = rec.team1_wins + rec.team2_wins + rec.draws;
    e = e.description(format!(
        "**Last {meetings} meetings:** {} **{}** · draws **{}** · {} **{}**\n\n{}",
        a.name,
        rec.team1_wins,
        rec.draws,
        b.name,
        rec.team2_wins,
        source_note(&h)
    ));
    let lines: Vec<String> = rec
        .matches
        .iter()
        .take(8)
        .map(|f| format!("{} · {}", fixture_line(f, true), comp_label(&f.competition, &f.competition_name)))
        .collect();
    e = e.field(
        "Recent meetings",
        if lines.is_empty() { "None in the data source.".into() } else { clip(&lines.join("\n"), 1024) },
        false,
    );
    for t in [&a, &b] {
        let mut s = vec![];
        for c in COMPETITIONS {
            if let Some(st) = fb.cached_now::<Standings>(&format!("standings:{}", c.key)) {
                for g in &st.data.groups {
                    if let Some(r) = g.rows.iter().find(|r| r.team_slug.as_deref() == Some(t.slug.as_str())) {
                        s.push(format!(
                            "{} in {} · {} pts, {:+} GD",
                            ordinal(r.position),
                            c.name,
                            r.points,
                            r.goals_for - r.goals_against
                        ));
                    }
                }
            }
        }
        if let Ok(team) = fb.team(&t.slug, Purpose::User).await {
            let form: String = team
                .value
                .data
                .matches
                .iter()
                .filter(|m| m.status == Status::Finished)
                .rev()
                .take(5)
                .filter_map(|m| result_letter(m, &team.value.data.name))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !form.is_empty() {
                s.push(format!("Form: `{form}`"));
            }
        }
        e = e.field(t.name.clone(), if s.is_empty() { "No table or form data.".into() } else { s.join("\n") }, true);
    }
    let rows = if a.slug.len() <= 80 && b.slug.len() <= 80 {
        vec![CreateActionRow::Buttons(vec![
            CreateButton::new(format!("fbt:info:{}", a.slug)).label(clip(&a.name, 70)).style(ButtonStyle::Secondary),
            CreateButton::new(format!("fbt:info:{}", b.slug)).label(clip(&b.name, 70)).style(ButtonStyle::Secondary),
        ])]
    } else {
        vec![]
    };
    Ok(Reply { embed: e, rows })
}

// ─────────────────────────────────────────────
//  Components
// ─────────────────────────────────────────────

/// `fbc:<view>:<id>` buttons, `fbc:pick:<view>[:n]` select menus.
pub(super) async fn handle_component(fb: &Football, comp: &ComponentInteraction) -> Result<Reply> {
    let parts: Vec<&str> = comp.data.custom_id.split(':').collect();
    match (parts.get(1).copied(), &comp.data.kind) {
        (Some("pick"), ComponentInteractionDataKind::StringSelect { values }) => {
            let view = parts.get(2).copied().unwrap_or("clubs");
            match values.first().and_then(|v| v.parse::<i64>().ok()) {
                Some(id) => show(fb, id, view).await,
                None => Ok(notice("Pick a player from the list.")),
            }
        }
        (Some(view), _) => match parts.get(2).and_then(|v| v.parse::<i64>().ok()) {
            Some(id) => show(fb, id, view).await,
            None => Ok(notice("This list has expired: run the command again.")),
        },
        _ => Ok(notice("This list has expired: run the command again.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::football::model::Squad;

    fn abs(kind: &str, start: &str, end: Option<&str>) -> Absence {
        Absence { kind: kind.into(), start: Some(start.into()), end: end.map(String::from) }
    }

    #[test]
    fn availability_lines() {
        let today = "2026-11-19";
        let out = vec![abs("Thigh Injury", "2026-11-01", Some("2026-12-05"))];
        assert!(availability(&out, today).contains("expected back **05 Dec 2026**"));
        let open = vec![abs("Knee Injury", "2026-11-10", None)];
        assert!(availability(&open, today).contains("return date unknown"));
        let ban = vec![abs("Red Card Suspension", "2026-11-18", Some("2026-11-25"))];
        assert!(availability(&ban, today).starts_with("🟥 **Suspended**"));
        // An older entry left open behind a newer one doesn't count.
        let stale = vec![abs("Knock", "2026-10-01", Some("2026-10-05")), abs("Thigh Injury", "2025-09-19", None)];
        assert!(availability(&stale, today).starts_with("✅"));
        assert!(absence_line(&stale[1], today, false).contains("end not reported"));
        let old = vec![abs("Knock", "2023-01-01", Some("2023-01-10"))];
        assert!(availability(&old, today).starts_with("✅"));
        assert!(coverage_warning(&old).unwrap().contains("10 Jan 2023"));
        assert!(coverage_warning(&[]).is_some());
    }

    #[test]
    fn season_spans() {
        assert_eq!(season_span(&[2026, 2025, 2013, 2012, 2011, 2010, 2009]), "2025–2026, 2009–2013");
        assert_eq!(season_span(&[2020]), "2020");
    }

    /// Injury views against API-Football (about 7 requests). Run with
    /// `cargo test live_injuries -- --ignored --nocapture` and API_FOOTBALL_KEY set.
    #[tokio::test]
    #[ignore = "calls API-Football (about 7 requests)"]
    async fn live_injuries() {
        let fb = Football::from_env(None);
        let print = |label: &str, r: &Reply| {
            let v = serde_json::to_value(&r.embed).unwrap();
            println!("===== {label} ({} rows)\n{}", r.rows.len(), v["description"].as_str().unwrap_or_default());
            for f in v["fields"].as_array().cloned().unwrap_or_default() {
                assert!(f["value"].as_str().unwrap().chars().count() <= 1024);
                println!("--- {}\n{}", f["name"].as_str().unwrap(), f["value"].as_str().unwrap());
            }
            assert!(r.rows.len() <= 5);
        };
        let neymar = fb.find_players("Neymar", Purpose::User).await.unwrap().value.data[0].id;
        print("injuries", &show(&fb, neymar, "injuries").await.unwrap());
        print("trophies", &show(&fb, neymar, "trophies").await.unwrap());
        print("ambiguous", &career(&fb, "Pedro", "injuries").await.unwrap());
        let fla = TeamRef { name: "Flamengo - RJ".into(), slug: "flamengo-rj".into(), country: "Brazil" };
        print("team", &team_injuries(&fb, &fla).await.unwrap());
        let squad = fb.cached_now::<Squad>("squad:flamengo-rj").unwrap();
        let leo = squad.data.players.iter().find(|p| p.name.contains("Pereira")).and_then(|p| p.id).unwrap();
        print("squad player", &show(&fb, leo, "injuries").await.unwrap());
        print("team after a check", &team_injuries(&fb, &fla).await.unwrap());
    }

    #[test]
    fn custom_ids_fit() {
        for (v, _) in VIEWS {
            assert!(format!("fbc:{v}:{}", i64::MAX).len() <= 100);
        }
    }
}
