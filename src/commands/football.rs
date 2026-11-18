//! `/football` (fixtures, results, tables, scorers, teams, squads, players,
//! match details: cached, may be minutes old) and `/live` (the live
//! scoreboard and live match details).
//!
//! Commands only read the football service's cache. When the cached copy is
//! too old they ask the service to refresh it, which shares one request among
//! everyone asking and respects each provider's budget; when that fails the
//! reply says how old the data it shows is, or that there is none.

use crate::football::{
    self, Football, Lookup,
    competitions::{self, COMPETITIONS, Competition, Kind},
    model::*,
    providers::FetchError,
    quota::{Blocked, Purpose},
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use serenity::all::{
    AutocompleteChoice, ButtonStyle, CommandInteraction, CommandOptionType, ComponentInteraction, Context,
    CreateActionRow, CreateAutocompleteResponse, CreateButton, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateEmbedFooter, CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse,
    ResolvedOption, ResolvedValue,
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tracing::info;

pub const PAGE_PREFIX: &str = "fb:";
const COLOR: u32 = 0x2ecc71;
const LIVE_COLOR: u32 = 0xe74c3c;
const PAGE_CHARS: usize = 3300;
const PAGE_LINES: usize = 24;
/// Live data older than this is labelled as cached.
const LIVE_FRESH: Duration = Duration::from_secs(3 * 60);

// ─────────────────────────────────────────────
//  Definitions
// ─────────────────────────────────────────────

fn opt(kind: CommandOptionType, name: &str, desc: &str) -> CreateCommandOption {
    CreateCommandOption::new(kind, name, desc)
}

fn fixture_opt() -> CreateCommandOption {
    opt(CommandOptionType::String, "fixture", "The match: type a team name and pick it from the list")
        .required(true)
        .set_autocomplete(true)
}

fn team_opt() -> CreateCommandOption {
    opt(CommandOptionType::String, "team", "The team: type its name").required(true).set_autocomplete(true)
}

fn competition_opt(required: bool) -> CreateCommandOption {
    opt(CommandOptionType::String, "competition", "The competition: type its name")
        .required(required)
        .set_autocomplete(true)
}

fn sub(name: &str, desc: &str, options: Vec<CreateCommandOption>) -> CreateCommandOption {
    options.into_iter().fold(opt(CommandOptionType::SubCommand, name, desc), |o, s| o.add_sub_option(s))
}

fn group(name: &str, desc: &str, subs: Vec<CreateCommandOption>) -> CreateCommandOption {
    subs.into_iter().fold(opt(CommandOptionType::SubCommandGroup, name, desc), |o, s| o.add_sub_option(s))
}

pub fn register() -> Vec<CreateCommand> {
    let football = CreateCommand::new("football")
        .description("Football fixtures, results, tables, teams and players (not live: see /live)")
        .add_option(group(
            "match",
            "A match: score, events, line-ups, statistics",
            vec![
                sub("details", "Score, status, kick-off and key events", vec![fixture_opt()]),
                sub("events", "Goals, cards, substitutions and VAR", vec![fixture_opt()]),
                sub("lineup", "Starting XI, substitutes, formations", vec![fixture_opt()]),
                sub("stats", "Team statistics, when the data source has them", vec![fixture_opt()]),
            ],
        ))
        .add_option(group(
            "team",
            "A team: info, squad, fixtures, statistics",
            vec![
                sub("info", "League positions, recent results and next matches", vec![team_opt()]),
                sub("squad", "Current registered players by position", vec![team_opt()]),
                sub("fixtures", "Recent results and upcoming matches", vec![team_opt()]),
                sub("stats", "The team's record in a competition", vec![team_opt(), competition_opt(true)]),
            ],
        ))
        .add_option(group(
            "competition",
            "A competition: fixtures, table, top scorers",
            vec![
                sub("list", "Competitions I follow and the data available for each", vec![]),
                sub(
                    "fixtures",
                    "Matches and results on a day",
                    vec![
                        competition_opt(true),
                        opt(CommandOptionType::String, "date", "today, tomorrow, yesterday or YYYY-MM-DD (default: today)"),
                    ],
                ),
                sub("table", "Standings", vec![competition_opt(true)]),
                sub(
                    "scorers",
                    "Top goalscorers or assists",
                    vec![
                        competition_opt(true),
                        opt(CommandOptionType::String, "stat", "Goals or assists (default: goals)")
                            .add_string_choice("Goals", "goals")
                            .add_string_choice("Assists", "assists"),
                    ],
                ),
            ],
        ))
        .add_option(sub(
            "player",
            "A player's season statistics",
            vec![opt(CommandOptionType::String, "name", "The player: type a name").required(true).set_autocomplete(true)],
        ));

    let live = CreateCommand::new("live")
        .description("Live football scores")
        .add_option(sub("all", "All live matches, by competition", vec![competition_opt(false)]))
        .add_option(sub("match", "One live match: score, clock, latest events", vec![fixture_opt()]))
        .add_option(sub("competition", "Live matches in one competition", vec![competition_opt(true)]));

    vec![football, live]
}

// ─────────────────────────────────────────────
//  Option parsing
// ─────────────────────────────────────────────

struct Args {
    path: Vec<String>,
    values: HashMap<String, String>,
}

impl Args {
    fn from(cmd: &CommandInteraction) -> Self {
        fn walk(opts: &[ResolvedOption], path: &mut Vec<String>, values: &mut HashMap<String, String>) {
            for o in opts {
                match &o.value {
                    ResolvedValue::SubCommandGroup(sub) | ResolvedValue::SubCommand(sub) => {
                        path.push(o.name.to_string());
                        walk(sub, path, values);
                    }
                    ResolvedValue::String(s) if !s.trim().is_empty() => {
                        values.insert(o.name.to_string(), s.trim().to_string());
                    }
                    _ => {}
                }
            }
        }
        let mut a = Args { path: vec![], values: HashMap::new() };
        walk(&cmd.data.options(), &mut a.path, &mut a.values);
        a
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

/// A reply: embed plus optional page buttons.
struct Reply {
    embed: CreateEmbed,
    rows: Vec<CreateActionRow>,
}

impl From<CreateEmbed> for Reply {
    fn from(embed: CreateEmbed) -> Self {
        Reply { embed, rows: vec![] }
    }
}

fn service() -> Result<Arc<Football>> {
    football::get().ok_or_else(|| anyhow!("football service not started"))
}

fn notice(text: impl Into<String>) -> Reply {
    CreateEmbed::new().description(text.into()).color(0x95a5a6).into()
}

// ─────────────────────────────────────────────
//  Entry points
// ─────────────────────────────────────────────

pub async fn handle_football(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let fb = service()?;
    cmd.defer(&ctx.http).await?;
    let args = Args::from(cmd);
    let path: Vec<&str> = args.path.iter().map(String::as_str).collect();
    let reply = match path.as_slice() {
        ["match", view] => match_view(&fb, &args, view, false).await,
        ["team", "info"] => team_info(&fb, &args).await,
        ["team", "squad"] => team_squad(&fb, &args).await,
        ["team", "fixtures"] => match resolve_team(&fb, args.get("team").unwrap_or_default()).await {
            Ok(t) => team_fixtures(&fb, &t.slug, 0).await,
            Err(r) => Ok(r),
        },
        ["team", "stats"] => team_stats(&fb, &args).await,
        ["competition", "list"] => Ok(competition_list(&fb)),
        ["competition", "fixtures"] => match (pick_competition(&args), parse_date(args.get("date"))) {
            (Ok(c), Ok(d)) => competition_fixtures(&fb, c, d, 0).await,
            (Err(r), _) | (_, Err(r)) => Ok(r),
        },
        ["competition", "table"] => match pick_competition(&args) {
            Ok(c) => table(&fb, c).await,
            Err(r) => Ok(r),
        },
        ["competition", "scorers"] => match pick_competition(&args) {
            Ok(c) => scorers(&fb, c, args.get("stat") == Some("assists")).await,
            Err(r) => Ok(r),
        },
        ["player"] => player(&fb, args.get("name").unwrap_or_default()).await,
        _ => Err(anyhow!("unknown /football subcommand {:?}", args.path)),
    }?;
    send(ctx, cmd, reply).await?;
    info!(command = "football", path = args.path.join(" "), "Football reply delivered");
    Ok(())
}

pub async fn handle_live(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let fb = service()?;
    cmd.defer(&ctx.http).await?;
    let args = Args::from(cmd);
    let reply = match args.path.first().map(String::as_str) {
        Some("all") => match args.get("competition") {
            None => live_list(&fb, None, 0).await,
            Some(_) => match pick_competition(&args) {
                Ok(c) => live_list(&fb, Some(c), 0).await,
                Err(r) => Ok(r),
            },
        },
        Some("competition") => match pick_competition(&args) {
            Ok(c) => live_list(&fb, Some(c), 0).await,
            Err(r) => Ok(r),
        },
        Some("match") => match_view(&fb, &args, "live", true).await,
        _ => Err(anyhow!("unknown /live subcommand")),
    }?;
    send(ctx, cmd, reply).await?;
    info!(command = "live", path = args.path.join(" "), "Live reply delivered");
    Ok(())
}

async fn send(ctx: &Context, cmd: &CommandInteraction, reply: Reply) -> Result<()> {
    cmd.edit_response(&ctx.http, EditInteractionResponse::new().embed(reply.embed).components(reply.rows))
        .await?;
    Ok(())
}

/// Page buttons: `fb:<kind>:<arg>:<arg>:<page>`.
pub async fn handle_page(ctx: &Context, comp: &ComponentInteraction) -> Result<()> {
    let fb = service()?;
    let parts: Vec<&str> = comp.data.custom_id.trim_start_matches(PAGE_PREFIX).split(':').collect();
    let page = |i: usize| parts.get(i).and_then(|p| p.parse::<usize>().ok()).unwrap_or(0);
    let reply = match parts.as_slice() {
        ["la", c, _] => live_list(&fb, competitions::by_key(c), page(2)).await?,
        ["cf", c, d, _] => match (competitions::by_key(c), NaiveDate::parse_from_str(d, "%Y-%m-%d")) {
            (Some(c), Ok(d)) => competition_fixtures(&fb, c, d, page(3)).await?,
            _ => notice("This list has expired: run the command again."),
        },
        ["tf", slug, _] => team_fixtures(&fb, slug, page(2)).await?,
        _ => notice("This list has expired: run the command again."),
    };
    comp.create_response(
        &ctx.http,
        CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new().embed(reply.embed).components(reply.rows),
        ),
    )
    .await?;
    Ok(())
}

// ─────────────────────────────────────────────
//  Autocomplete (cache only: never calls a provider)
// ─────────────────────────────────────────────

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

fn utc_label(t: i64) -> String {
    DateTime::from_timestamp(t, 0).map(|d| d.format("%a %d %b %H:%M UTC").to_string()).unwrap_or_default()
}

fn comp_label(key: &str, fallback: &str) -> String {
    competitions::by_key(key).map(|c| c.name.to_string()).unwrap_or_else(|| fallback.to_string())
}

pub fn autocomplete_choices(fb: &Football, option: &str, typed: &str) -> Vec<AutocompleteChoice> {
    let q = football::fold(typed);
    let mut out: Vec<AutocompleteChoice> = vec![];
    match option {
        "competition" => {
            for c in COMPETITIONS {
                let hay = football::fold(&format!("{} {} {}", c.name, c.region, c.sportscore_name));
                if q.is_empty() || hay.contains(&q) {
                    out.push(AutocompleteChoice::new(format!("{} · {}", c.name, c.region), c.key));
                }
            }
        }
        "fixture" => {
            let now = Utc::now().timestamp();
            let mut list: Vec<Fixture> = fb
                .recent_fixtures()
                .into_iter()
                .map(|(f, _)| f)
                .filter(|f| !f.slug.is_empty() && f.slug.len() <= 100)
                .filter(|f| {
                    q.is_empty()
                        || football::fold(&format!("{} {} {}", f.home, f.away, comp_label(&f.competition, &f.competition_name)))
                            .contains(&q)
                })
                .collect();
            list.sort_by_key(|f| (f.status != Status::Live, (f.kickoff - now).abs()));
            for f in list {
                let what = match (f.status, f.score()) {
                    (Status::Live, Some((h, a))) => format!("🔴 {} {h}–{a} {}", f.home, f.away),
                    (Status::Finished, Some((h, a))) => format!("FT {} {h}–{a} {}", f.home, f.away),
                    _ => format!("{} vs {} · {}", f.home, f.away, utc_label(f.kickoff)),
                };
                let name = clip(&format!("{what} · {}", comp_label(&f.competition, &f.competition_name)), 100);
                out.push(AutocompleteChoice::new(name, f.slug));
            }
        }
        "team" => {
            for (name, slug, c) in fb.known_teams() {
                if (q.is_empty() || football::fold(&name).contains(&q)) && slug.len() <= 100 {
                    out.push(AutocompleteChoice::new(clip(&format!("{name} · {}", c.name), 100), slug));
                }
            }
        }
        "name" => {
            let mut seen = std::collections::HashSet::new();
            for c in COMPETITIONS {
                let Some(s) = fb.cached_now::<Vec<Scorer>>(&format!("scorers:{}:goals", c.key)) else { continue };
                for p in s.data {
                    if let Some(slug) = p.player_slug
                        && (q.is_empty() || football::fold(&p.player).contains(&q))
                        && slug.len() <= 100
                        && seen.insert(slug.clone())
                    {
                        out.push(AutocompleteChoice::new(clip(&format!("{} · {}", p.player, p.team), 100), slug));
                    }
                }
            }
        }
        _ => {}
    }
    out.truncate(25);
    out
}

pub async fn handle_autocomplete(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let choices = match (football::get(), cmd.data.autocomplete()) {
        (Some(fb), Some(focused)) => autocomplete_choices(&fb, focused.name, focused.value),
        _ => vec![],
    };
    cmd.create_response(
        &ctx.http,
        CreateInteractionResponse::Autocomplete(CreateAutocompleteResponse::new().set_choices(choices)),
    )
    .await?;
    Ok(())
}

// ─────────────────────────────────────────────
//  Shared rendering
// ─────────────────────────────────────────────

/// What a user should know when data couldn't be fetched.
fn explain(e: &FetchError) -> String {
    match e {
        FetchError::Blocked(Blocked::Budget) => {
            "the data provider's request budget for today is used up; automatic refresh resumes when the quota resets (00:00 UTC)".into()
        }
        FetchError::Blocked(Blocked::Cooldown { state, until }) if state.contains("daily") => format!(
            "the data provider has reached its request limit; automatic refresh resumes when the quota resets ({})",
            until.format("%H:%M UTC")
        ),
        FetchError::Blocked(Blocked::Cooldown { until, .. }) => {
            format!("the data provider is rate-limited or unavailable until {}", until.format("%H:%M UTC"))
        }
        FetchError::Blocked(Blocked::Minute) => "too many requests this minute; try again shortly".into(),
        FetchError::Blocked(Blocked::Unauthorized) => "the data provider rejected the bot's API key".into(),
        FetchError::Blocked(Blocked::NotConfigured) => "this data source isn't configured on this bot".into(),
        FetchError::NotFound => "the data provider doesn't have it".into(),
        FetchError::PlanRestricted(_) => "it isn't included in the data provider's free plan".into(),
        FetchError::Failed(_) => "the data provider is unavailable right now".into(),
    }
}

fn unavailable(what: &str, e: &FetchError) -> Reply {
    notice(format!("⚠️ {what} is temporarily unavailable: {}, and no cached copy exists.", explain(e)))
}

/// Source and freshness footer for `/football` replies (cached data, not live).
fn source_note<T>(l: &Lookup<T>) -> String {
    let v = &l.value;
    let mut s = format!("🕒 Updated <t:{}:R> · not live (use `/live` for live scores)", v.fetched_at);
    if v.provider == Provider::FootballData {
        s.push_str("\n⏳ football-data.org scores on the free tier may be delayed.");
    }
    if l.stale {
        s.push_str(&format!(
            "\n⚠️ Couldn't refresh: {}. Showing the last saved copy.",
            l.note.as_deref().map(friendly_note).unwrap_or("unknown error".into())
        ));
    }
    s.push('\n');
    s.push_str(v.provider.attribution());
    s
}

/// Freshness for `/live` replies.
fn live_note<T>(l: &Lookup<T>) -> String {
    let v = &l.value;
    let mut s = if l.stale || v.age() > LIVE_FRESH {
        format!(
            "⚠️ Live data is currently served from cache. Last refreshed <t:{at}:t> (<t:{at}:R>).{}",
            l.note.as_deref().map(|n| format!(" Couldn't refresh: {}.", friendly_note(n))).unwrap_or_default(),
            at = v.fetched_at
        )
    } else {
        format!("🔴 Live · updated <t:{}:R>", v.fetched_at)
    };
    s.push('\n');
    s.push_str(v.provider.attribution());
    s
}

/// Refresh-failure notes come from `FetchError`'s Display; keep them short.
fn friendly_note(n: &str) -> String {
    if n.contains("budget") || n.contains("quota") {
        "the provider's request limit was reached; refresh resumes when the quota resets".into()
    } else if n.contains("rate") || n.contains("limit") {
        "the provider is rate-limiting requests".into()
    } else {
        "the provider is unavailable".into()
    }
}

fn score_text(f: &Fixture) -> String {
    match f.score() {
        Some((h, a)) => format!("{h}–{a}"),
        None => "vs".into(),
    }
}

fn fixture_line(f: &Fixture, with_date: bool) -> String {
    let when = if with_date { format!("<t:{}:f>", f.kickoff) } else { format!("<t:{}:t>", f.kickoff) };
    match f.status {
        Status::Live => format!("🔴 **{} {} {}** · {}", f.home, score_text(f), f.away, f.status_text),
        Status::Finished => format!("✅ {} **{}** {} · FT", f.home, score_text(f), f.away),
        Status::Upcoming => format!("🕒 {when} · {} vs {}", f.home, f.away),
        Status::Postponed | Status::Cancelled | Status::Suspended | Status::Other => {
            format!("⏸ {when} · {} vs {} · {}", f.home, f.away, f.status_text)
        }
    }
}

/// Split blocks (heading, lines) into pages, repeating a heading that
/// continues onto the next page.
fn paginate(blocks: &[(String, Vec<String>)], page: usize) -> (String, usize) {
    let mut pages: Vec<String> = vec![];
    let mut cur = String::new();
    let mut lines = 0;
    for (heading, items) in blocks {
        let mut first = true;
        for item in items {
            let head = if first { format!("**{heading}**\n") } else { String::new() };
            if !cur.is_empty() && (cur.len() + head.len() + item.len() > PAGE_CHARS || lines >= PAGE_LINES) {
                pages.push(std::mem::take(&mut cur));
                lines = 0;
                if !first {
                    cur.push_str(&format!("**{heading}** (cont.)\n"));
                }
            }
            if first && !cur.is_empty() && !cur.ends_with("\n\n") {
                cur.push('\n');
            }
            cur.push_str(&head);
            cur.push_str(item);
            cur.push('\n');
            lines += 1;
            first = false;
        }
    }
    if !cur.is_empty() {
        pages.push(cur);
    }
    let total = pages.len().max(1);
    (pages.get(page.min(total - 1)).cloned().unwrap_or_default(), total)
}

fn page_buttons(prefix: &str, page: usize, total: usize) -> Vec<CreateActionRow> {
    if total <= 1 {
        return vec![];
    }
    let page = page.min(total - 1);
    vec![CreateActionRow::Buttons(vec![
        CreateButton::new(format!("{PAGE_PREFIX}{prefix}:{}", page.saturating_sub(1)))
            .label("◀")
            .style(ButtonStyle::Secondary)
            .disabled(page == 0),
        CreateButton::new(format!("{PAGE_PREFIX}{prefix}:noop:{page}"))
            .label(format!("{}/{}", page + 1, total))
            .style(ButtonStyle::Secondary)
            .disabled(true),
        CreateButton::new(format!("{PAGE_PREFIX}{prefix}:{}", page + 1))
            .label("▶")
            .style(ButtonStyle::Secondary)
            .disabled(page + 1 >= total),
    ])]
}

// The error is the reply to send; it's built once per command.
#[allow(clippy::result_large_err)]
fn pick_competition(args: &Args) -> Result<&'static Competition, Reply> {
    let typed = args.get("competition").unwrap_or_default();
    competitions::find(typed).ok_or_else(|| {
        notice(format!(
            "❌ I don't follow a competition called **{}**. See `/football competition list`.",
            clip(typed, 60)
        ))
    })
}

// The error is the reply to send; it's built once per command.
#[allow(clippy::result_large_err)]
fn parse_date(input: Option<&str>) -> Result<NaiveDate, Reply> {
    let today = Utc::now().date_naive();
    let Some(s) = input else { return Ok(today) };
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "today" | "hoje" => Ok(today),
        "tomorrow" | "amanha" | "amanhã" => Ok(today + ChronoDuration::days(1)),
        "yesterday" | "ontem" => Ok(today - ChronoDuration::days(1)),
        _ => NaiveDate::parse_from_str(&s, "%Y-%m-%d")
            .or_else(|_| NaiveDate::parse_from_str(&s, "%d/%m/%Y"))
            .map_err(|_| notice("❌ Use a date like `2026-11-22`, `today`, `tomorrow` or `yesterday`.")),
    }
}

// ─────────────────────────────────────────────
//  Matches
// ─────────────────────────────────────────────

/// Find the match a user picked: a slug from autocomplete, or text matched
/// against recent fixtures.
fn resolve_fixture(fb: &Football, input: &str) -> Option<String> {
    let input = input.trim();
    if !input.is_empty() && !input.contains(' ') {
        return Some(input.to_string());
    }
    let q = football::fold(input);
    fb.recent_fixtures()
        .into_iter()
        .map(|(f, _)| f)
        .find(|f| !f.slug.is_empty() && football::fold(&format!("{} {}", f.home, f.away)).contains(&q))
        .map(|f| f.slug)
}

fn match_title(f: &Fixture) -> String {
    format!("{} {} {}", f.home, score_text(f), f.away)
}

fn status_line(d: &MatchDetail) -> String {
    let f = &d.fixture;
    match f.status {
        Status::Live => {
            let minute = d.minute.as_deref().map(|m| format!(" · {m}'")).unwrap_or_default();
            format!("🔴 **{}**{minute}", f.status_text)
        }
        Status::Finished => "✅ **Full time**".into(),
        Status::Upcoming => format!("🕒 Kick-off <t:{}:R>", f.kickoff),
        _ => format!("⏸ **{}**", f.status_text),
    }
}

fn event_line(e: &Event, d: &MatchDetail) -> String {
    let minute = match (e.minute, e.extra) {
        (Some(m), Some(x)) if x > 0 => format!("{m}+{x}'"),
        (Some(m), _) => format!("{m}'"),
        _ => "–".into(),
    };
    let team = match e.side {
        Some(Side::Home) => d.fixture.home.as_str(),
        Some(Side::Away) => d.fixture.away.as_str(),
        None => "",
    };
    let score = e.score.map(|(h, a)| format!(" ({h}–{a})")).unwrap_or_default();
    let who = if e.player.is_empty() { "–".to_string() } else { e.player.clone() };
    let what = match &e.kind {
        EventKind::Goal => format!("⚽ **{who}**{score}"),
        EventKind::PenaltyGoal => format!("⚽ **{who}** (pen){score}"),
        EventKind::OwnGoal => format!("⚽ **{who}** (own goal){score}"),
        EventKind::PenaltyMissed => format!("❌ {who} (missed penalty)"),
        EventKind::YellowCard => format!("🟨 {who}"),
        EventKind::SecondYellow => format!("🟨🟥 {who}"),
        EventKind::RedCard => format!("🟥 {who}"),
        EventKind::Substitution => format!(
            "🔄 ⬆ {} ⬇ {who}",
            e.player_in.as_deref().unwrap_or("–")
        ),
        EventKind::Var => format!("📺 VAR{}", if e.player.is_empty() { String::new() } else { format!(" · {who}") }),
        EventKind::Other(k) => format!("• {k} {who}"),
    };
    if team.is_empty() { format!("`{minute:>6}` {what}") } else { format!("`{minute:>6}` {what} · {team}") }
}

fn lineup_field(t: &TeamLineup, starting: bool) -> String {
    let players = if starting { &t.starting } else { &t.substitutes };
    let mut s = String::new();
    if starting {
        let head: Vec<String> = [t.formation.clone(), t.coach.as_ref().map(|c| format!("Coach: {c}"))]
            .into_iter()
            .flatten()
            .collect();
        if !head.is_empty() {
            s.push_str(&format!("**{}**\n", head.join(" · ")));
        }
    }
    for p in players {
        let num = p.number.map(|n| format!("`{n:>2}` ")).unwrap_or_default();
        let pos = p.position.as_deref().map(|p| format!(" ({p})")).unwrap_or_default();
        let cap = if p.captain { " ©" } else { "" };
        s.push_str(&format!("{num}{}{pos}{cap}\n", p.name));
    }
    if s.is_empty() { "–".into() } else { clip(&s, 1020) }
}

async fn match_view(fb: &Football, args: &Args, view: &str, live_cmd: bool) -> Result<Reply> {
    let input = args.get("fixture").unwrap_or_default();
    let Some(slug) = resolve_fixture(fb, input) else {
        return Ok(notice("❌ I couldn't find that match. Start typing a team name and pick the match from the list."));
    };
    let lookup = match fb.match_detail(&slug, Purpose::User).await {
        Ok(l) => l,
        Err(FetchError::NotFound) => {
            return Ok(notice("❌ The data source doesn't know that match. Pick one from the list as you type."));
        }
        Err(e) if live_cmd => {
            return Ok(notice(format!(
                "⚠️ Live data is temporarily unavailable: {}, and no cached result exists for this fixture.",
                explain(&e)
            )));
        }
        Err(e) => return Ok(unavailable("This match", &e)),
    };
    let d = &lookup.value.data;
    let f = &d.fixture;
    if matches!(f.status, Status::Live | Status::Upcoming) {
        fb.note_demand(&slug);
    }
    let comp = comp_label(&f.competition, &f.competition_name);
    let mut header = format!("**{comp}** · <t:{}:F>\n{}", f.kickoff, status_line(d));
    if let Some((h, a)) = d.half_time
        && f.status != Status::Upcoming
    {
        header.push_str(&format!(" · HT {h}–{a}"));
    }

    if live_cmd && f.status != Status::Live {
        let state = match f.status {
            Status::Finished => format!("It has finished: **{}**.", match_title(f)),
            Status::Upcoming => format!("It kicks off <t:{}:R>.", f.kickoff),
            _ => format!("Last known status: **{}**.", f.status_text),
        };
        let embed = CreateEmbed::new()
            .title(match_title(f))
            .description(format!("ℹ️ This match isn't live. {state}\nUse `/football match details` for the full report.\n\n{}", live_note(&lookup)))
            .color(COLOR);
        return Ok(embed.into());
    }

    let mut embed = CreateEmbed::new()
        .title(match_title(f))
        .color(if f.status == Status::Live { LIVE_COLOR } else { COLOR });
    let note = if live_cmd { live_note(&lookup) } else { source_note(&lookup) };

    let body = match view {
        "details" | "live" => {
            let key: Vec<String> = d
                .events
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        EventKind::Goal | EventKind::OwnGoal | EventKind::PenaltyGoal | EventKind::RedCard | EventKind::SecondYellow
                    )
                })
                .map(|e| event_line(e, d))
                .collect();
            let mut s = header.clone();
            if view == "live" {
                let latest: Vec<String> = d.events.iter().rev().take(8).map(|e| event_line(e, d)).collect();
                if !latest.is_empty() {
                    s.push_str(&format!("\n\n**Latest events**\n{}", latest.join("\n")));
                }
            } else if !key.is_empty() {
                s.push_str(&format!("\n\n**Key events**\n{}", key.join("\n")));
            } else if f.status != Status::Upcoming {
                s.push_str("\n\nNo goals or red cards reported.");
            }
            if let Some(l) = &d.lineups {
                let fm = |t: &TeamLineup| t.formation.clone().unwrap_or_else(|| "?".into());
                s.push_str(&format!(
                    "\n\n**Formations:** {} {} · {} {}{}",
                    f.home,
                    fm(&l.home),
                    f.away,
                    fm(&l.away),
                    if l.confirmed { "" } else { " (expected)" }
                ));
            }
            s
        }
        "events" => {
            if d.events.is_empty() {
                format!("{header}\n\nNo events reported by the data source{}.", if f.status == Status::Upcoming { " yet" } else { "" })
            } else {
                let lines: Vec<String> = d.events.iter().map(|e| event_line(e, d)).collect();
                clip(&format!("{header}\n\n{}", lines.join("\n")), 3500)
            }
        }
        "lineup" => match &d.lineups {
            Some(l) => {
                embed = embed
                    .field(format!("{} · starting XI", f.home), lineup_field(&l.home, true), true)
                    .field(format!("{} · starting XI", f.away), lineup_field(&l.away, true), true)
                    .field("\u{200b}", "\u{200b}", false)
                    .field(format!("{} · substitutes", f.home), lineup_field(&l.home, false), true)
                    .field(format!("{} · substitutes", f.away), lineup_field(&l.away, false), true);
                format!(
                    "{header}\n\n{}",
                    if l.confirmed { "✅ Official line-ups" } else { "📝 Expected line-ups (not confirmed yet)" }
                )
            }
            None => format!("{header}\n\nLine-ups are not provided by the current data source for this match (yet)."),
        },
        "stats" => {
            if d.stats.is_empty() {
                format!("{header}\n\n📊 Match statistics are not provided by the current data source for this match.")
            } else {
                let w = d.stats.iter().map(|s| s.name.chars().count()).max().unwrap_or(10).min(22);
                let rows: Vec<String> = d
                    .stats
                    .iter()
                    .map(|s| format!("{:>7}  {:<w$}  {:<7}", s.home, clip(&s.name, w), s.away))
                    .collect();
                clip(&format!("{header}\n```\n{:>7}  {:<w$}  {}\n{}\n```", clip(&f.home, 7), "", clip(&f.away, 7), rows.join("\n")), 3800)
            }
        }
        _ => header.clone(),
    };
    Ok(embed.description(format!("{body}\n\n{note}")).into())
}

// ─────────────────────────────────────────────
//  /live lists
// ─────────────────────────────────────────────

async fn live_list(fb: &Football, filter: Option<&'static Competition>, page: usize) -> Result<Reply> {
    let lookup = match fb.live(Purpose::User).await {
        Ok(l) => l,
        Err(e) => {
            return Ok(notice(format!(
                "⚠️ Live data is temporarily unavailable: {}, and no cached live scoreboard exists.",
                explain(&e)
            )));
        }
    };
    let title = match filter {
        Some(c) => format!("🔴 Live · {}", c.name),
        None => "🔴 Live football".to_string(),
    };
    let matches: Vec<&Fixture> = lookup
        .value
        .data
        .iter()
        .filter(|f| f.status == Status::Live && filter.is_none_or(|c| f.competition == c.key))
        .collect();
    let note = live_note(&lookup);
    if matches.is_empty() {
        let now = Utc::now().timestamp();
        let next = fb
            .recent_fixtures()
            .into_iter()
            .map(|(f, _)| f)
            .filter(|f| f.status == Status::Upcoming && f.kickoff > now && filter.is_none_or(|c| f.competition == c.key))
            .min_by_key(|f| f.kickoff);
        let next = next
            .map(|f| format!("\nNext kick-off: **{} vs {}** ({}) <t:{}:R>", f.home, f.away, comp_label(&f.competition, ""), f.kickoff))
            .unwrap_or_default();
        let scope = filter.map(|c| c.name).unwrap_or("the competitions I follow");
        return Ok(CreateEmbed::new()
            .title(title)
            .description(format!("No live matches in {scope} right now.{next}\n\n{note}"))
            .color(COLOR)
            .into());
    }
    let mut blocks: Vec<(String, Vec<String>)> = vec![];
    for f in &matches {
        let heading = competitions::by_key(&f.competition)
            .map(|c| format!("{} {}", c.region.split(' ').next().unwrap_or(""), c.name))
            .unwrap_or_else(|| f.competition_name.clone());
        match blocks.iter_mut().find(|(h, _)| *h == heading) {
            Some((_, lines)) => lines.push(fixture_line(f, false)),
            None => blocks.push((heading, vec![fixture_line(f, false)])),
        }
    }
    let (text, total) = paginate(&blocks, page);
    let embed = CreateEmbed::new()
        .title(format!("{title} ({})", matches.len()))
        .description(format!("{text}\n{note}"))
        .color(LIVE_COLOR);
    Ok(Reply { embed, rows: page_buttons(&format!("la:{}", filter.map(|c| c.key).unwrap_or("-")), page, total) })
}

// ─────────────────────────────────────────────
//  Competitions
// ─────────────────────────────────────────────

fn competition_list(fb: &Football) -> Reply {
    let mut embed = CreateEmbed::new()
        .title("🏆 Competitions I follow")
        .color(COLOR)
        .description(format!(
            "Fixtures, results, live scores, match events and line-ups for all of them. \
             ⭐ = match details followed live by default.{}\n\n{}",
            if fb.has_squads() { " Squads for every team." } else { "" },
            Provider::SportScore.attribution()
        ));
    let mut regions: Vec<&str> = vec![];
    for c in COMPETITIONS {
        if !regions.contains(&c.region) {
            regions.push(c.region);
        }
    }
    for r in regions {
        let lines: Vec<String> = COMPETITIONS
            .iter()
            .filter(|c| c.region == r)
            .map(|c| {
                let mut extras = vec![];
                match c.kind {
                    Kind::League => extras.push("table"),
                    Kind::Groups => extras.push("group tables"),
                    Kind::Cup => extras.push("knockout"),
                }
                extras.push("scorers");
                if c.football_data.is_some() && fb.has_fallback() {
                    extras.push("backup source");
                }
                format!("{}{} · {}", if fb.is_priority(c.key) { "⭐ " } else { "" }, c.name, extras.join(", "))
            })
            .collect();
        embed = embed.field(r, lines.join("\n"), false);
    }
    embed.into()
}

async fn competition_fixtures(fb: &Football, c: &'static Competition, date: NaiveDate, page: usize) -> Result<Reply> {
    let lookup = match fb.fixtures(c, date, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return Ok(unavailable("This fixture list", &e)),
    };
    let mut list = lookup.value.data.clone();
    // Newer scores from the live scoreboard, clearly marked as live.
    if let Some(live) = fb.cached_now::<Vec<Fixture>>("live")
        && live.fetched_at > lookup.value.fetched_at
        && live.age() < LIVE_FRESH
    {
        for f in list.iter_mut() {
            if let Some(l) = live.data.iter().find(|l| l.id == f.id) {
                *f = l.clone();
            }
        }
    }
    let title = format!("📅 {} · {}", c.name, date.format("%a %d %b %Y"));
    let note = source_note(&lookup);
    if list.is_empty() {
        return Ok(CreateEmbed::new()
            .title(title)
            .description(format!("No matches on this day.\n\n{note}"))
            .color(COLOR)
            .into());
    }
    let lines: Vec<String> = list.iter().map(|f| fixture_line(f, false)).collect();
    let (text, total) = paginate(&[(format!("{} matches", list.len()), lines)], page);
    let embed = CreateEmbed::new().title(title).description(format!("{text}\n{note}")).color(COLOR);
    Ok(Reply { embed, rows: page_buttons(&format!("cf:{}:{date}", c.key), page, total) })
}

fn table_block(rows: &[StandingRow]) -> String {
    let mut s = String::from("```\n #  Team             P   W  D  L   GD  Pts\n");
    for r in rows {
        let gd = r.goals_for - r.goals_against;
        s.push_str(&format!(
            "{:>2}  {:<15} {:>2} {:>3} {:>2} {:>2} {:>4} {:>4}\n",
            r.position,
            clip(r.team.split(" - ").next().unwrap_or(&r.team), 15),
            r.played,
            r.won,
            r.drawn,
            r.lost,
            format!("{gd:+}"),
            r.points
        ));
    }
    s.push_str("```");
    s
}

async fn table(fb: &Football, c: &'static Competition) -> Result<Reply> {
    if c.kind == Kind::Cup {
        return Ok(notice(format!(
            "🏆 **{}** is a knockout competition, so it has no table. Try `/football competition fixtures`.",
            c.name
        )));
    }
    let lookup = match fb.standings(c, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return Ok(unavailable("This table", &e)),
    };
    let s = &lookup.value.data;
    let note = source_note(&lookup);
    let mut embed = CreateEmbed::new().title(format!("📊 {}", c.name)).color(COLOR);
    if s.groups.is_empty() {
        return Ok(embed.description(format!("No table published yet.\n\n{note}")).into());
    }
    if s.groups.len() == 1 {
        let g = &s.groups[0];
        let zones: Vec<String> = {
            let mut z: Vec<String> = vec![];
            for r in &g.rows {
                if let Some(n) = &r.note
                    && !z.iter().any(|x| x.ends_with(n.as_str()))
                {
                    z.push(format!("{}: {n}", r.position));
                }
            }
            z
        };
        let zones = if zones.is_empty() { String::new() } else { format!("\nZones from position {}", zones.join(" · ")) };
        embed = embed.description(clip(&format!("{}{zones}\n\n{note}", table_block(&g.rows)), 4000));
    } else {
        for g in s.groups.iter().take(24) {
            let name = if g.name.is_empty() { "Table".to_string() } else { g.name.clone() };
            embed = embed.field(name, clip(&table_block(&g.rows), 1024), false);
        }
        embed = embed.description(note);
    }
    Ok(embed.into())
}

async fn scorers(fb: &Football, c: &'static Competition, assists: bool) -> Result<Reply> {
    let lookup = match fb.scorers(c, assists, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return Ok(unavailable("This list", &e)),
    };
    let note = source_note(&lookup);
    let title = format!("{} {} · {}", if assists { "🅰️" } else { "⚽" }, if assists { "Top assists" } else { "Top scorers" }, c.name);
    let lines: Vec<String> = lookup
        .value
        .data
        .iter()
        .take(15)
        .map(|s| {
            let main = if assists { s.assists } else { s.goals };
            let other = if assists { s.goals.map(|g| format!(", {}", plural(g, "goal"))) } else { s.assists.map(|a| format!(", {}", plural(a, "assist"))) };
            let games = s.matches.map(|m| format!(" in {}", plural(m, "match").replace("matchs", "matches"))).unwrap_or_default();
            format!(
                "**{}.** {} · {} — **{}**{}{games}",
                s.rank,
                s.player,
                s.team.split(" - ").next().unwrap_or(&s.team),
                main.map(|m| m.to_string()).unwrap_or("–".into()),
                other.unwrap_or_default()
            )
        })
        .collect();
    let body = if lines.is_empty() { "No statistics published yet.".to_string() } else { lines.join("\n") };
    Ok(CreateEmbed::new().title(title).description(format!("{body}\n\n{note}")).color(COLOR).into())
}

// ─────────────────────────────────────────────
//  Teams
// ─────────────────────────────────────────────

struct TeamRef {
    name: String,
    slug: String,
    country: &'static str,
}

/// A team from autocomplete (a slug in a cached table) or typed text (looked up).
async fn resolve_team(fb: &Football, input: &str) -> Result<TeamRef, Reply> {
    let known = fb.known_teams();
    if let Some((name, slug, c)) = known.iter().find(|(_, s, _)| s == input) {
        return Ok(TeamRef { name: name.clone(), slug: slug.clone(), country: c.country });
    }
    let q = football::fold(input);
    if let Some((name, slug, c)) = known.iter().find(|(n, _, _)| football::fold(n.split(" - ").next().unwrap_or(n)) == q) {
        return Ok(TeamRef { name: name.clone(), slug: slug.clone(), country: c.country });
    }
    match fb.search(input, Purpose::User).await {
        Ok(l) => match best_match(&l.value.data.teams, input) {
            Some((name, slug)) => Ok(TeamRef { name: name.clone(), slug: slug.clone(), country: "" }),
            None => Err(notice(format!("❌ I couldn't find a team called **{}**.", clip(input, 60)))),
        },
        Err(e) => Err(unavailable("Team search", &e)),
    }
}

/// The search result closest to what was typed: the exact name, then a name
/// starting with it, then the shortest name containing it, then the first.
fn best_match<'a>(results: &'a [(String, String)], typed: &str) -> Option<&'a (String, String)> {
    let q = football::fold(typed);
    let base = |n: &str| football::fold(n.split(" - ").next().unwrap_or(n));
    results
        .iter()
        .find(|(n, _)| base(n) == q)
        .or_else(|| results.iter().find(|(n, _)| base(n).starts_with(&q)))
        .or_else(|| results.iter().filter(|(n, _)| base(n).contains(&q)).min_by_key(|(n, _)| n.len()))
        .or_else(|| results.first())
}

fn plural(n: u32, word: &str) -> String {
    if n == 1 { format!("{n} {word}") } else { format!("{n} {word}s") }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn result_letter(f: &Fixture, team: &str) -> Option<char> {
    let (h, a) = f.score()?;
    let home = f.home == team;
    Some(match (h.cmp(&a), home) {
        (std::cmp::Ordering::Equal, _) => 'D',
        (std::cmp::Ordering::Greater, true) | (std::cmp::Ordering::Less, false) => 'W',
        _ => 'L',
    })
}

async fn team_info(fb: &Football, args: &Args) -> Result<Reply> {
    let t = match resolve_team(fb, args.get("team").unwrap_or_default()).await {
        Ok(t) => t,
        Err(r) => return Ok(r),
    };
    let lookup = match fb.team(&t.slug, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return Ok(unavailable("This team", &e)),
    };
    let team = &lookup.value.data;
    let now = Utc::now().timestamp();
    let mut positions = vec![];
    for c in COMPETITIONS {
        if let Some(s) = fb.cached_now::<Standings>(&format!("standings:{}", c.key)) {
            for g in &s.data.groups {
                if let Some(r) = g.rows.iter().find(|r| r.team_slug.as_deref() == Some(t.slug.as_str())) {
                    let grp = if g.name.is_empty() || c.kind != Kind::Groups { String::new() } else { format!(" ({})", g.name) };
                    positions.push(format!("**{}** in {}{grp} · {} pts from {} games", ordinal(r.position), c.name, r.points, r.played));
                }
            }
        }
    }
    let recent: Vec<&Fixture> = team.matches.iter().filter(|m| m.status == Status::Finished).rev().take(5).collect();
    let next: Vec<&Fixture> = team.matches.iter().filter(|m| m.kickoff > now - 3 * 3600 && !m.status.is_terminal()).take(5).collect();
    let line = |f: &Fixture| format!("{} · {}", fixture_line(f, true), comp_label(&f.competition, &f.competition_name));
    let form: String = recent.iter().rev().filter_map(|f| result_letter(f, &team.name)).collect();
    let mut embed = CreateEmbed::new().title(format!("🛡️ {}", team.name)).color(COLOR);
    if let Some(logo) = &team.logo
        && logo.ends_with(".png")
    {
        embed = embed.thumbnail(logo);
    }
    if !positions.is_empty() {
        embed = embed.field("Standings", positions.join("\n"), false);
    }
    if !form.is_empty() {
        embed = embed.field("Form (oldest → newest)", format!("`{form}`"), false);
    }
    embed = embed
        .field(
            "Recent results",
            if recent.is_empty() { "None in the data source's recent list.".into() } else { clip(&recent.iter().map(|f| line(f)).collect::<Vec<_>>().join("\n"), 1024) },
            false,
        )
        .field(
            "Next matches",
            if next.is_empty() { "None scheduled in the data source.".into() } else { clip(&next.iter().map(|f| line(f)).collect::<Vec<_>>().join("\n"), 1024) },
            false,
        )
        .description(source_note(&lookup));
    Ok(embed.into())
}

async fn team_fixtures(fb: &Football, slug: &str, page: usize) -> Result<Reply> {
    let lookup = match fb.team(slug, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return Ok(unavailable("This team's fixtures", &e)),
    };
    let team = &lookup.value.data;
    let now = Utc::now().timestamp();
    let (past, future): (Vec<&Fixture>, Vec<&Fixture>) = team.matches.iter().partition(|f| f.kickoff < now - 3 * 3600 || f.status == Status::Finished);
    let fmt = |f: &&Fixture| format!("{} · {}", fixture_line(f, true), comp_label(&f.competition, &f.competition_name));
    let mut blocks = vec![];
    if !future.is_empty() {
        blocks.push(("Upcoming".to_string(), future.iter().map(fmt).collect()));
    }
    if !past.is_empty() {
        blocks.push(("Results".to_string(), past.iter().rev().map(fmt).collect()));
    }
    let (text, total) = paginate(&blocks, page);
    let text = if text.is_empty() { "No matches in the data source.".to_string() } else { text };
    let embed = CreateEmbed::new()
        .title(format!("📅 {}", team.name))
        .description(format!("{text}\n{}", source_note(&lookup)))
        .color(COLOR);
    Ok(Reply { embed, rows: page_buttons(&format!("tf:{slug}"), page, total) })
}

fn position_rank(p: Option<&str>) -> (u8, String) {
    let p = p.unwrap_or("Other");
    let rank = match p {
        "Goalkeeper" => 0,
        "Defender" => 1,
        "Midfielder" => 2,
        "Attacker" | "Forward" => 3,
        _ => 4,
    };
    (rank, p.to_string())
}

async fn team_squad(fb: &Football, args: &Args) -> Result<Reply> {
    if !fb.has_squads() {
        return Ok(notice("⚠️ Squads aren't available on this bot (no API-Football key is configured)."));
    }
    let t = match resolve_team(fb, args.get("team").unwrap_or_default()).await {
        Ok(t) => t,
        Err(r) => return Ok(r),
    };
    let lookup = match fb.squad(&t.slug, &t.name, t.country, Purpose::User).await {
        Ok(l) => l,
        Err(FetchError::NotFound) => {
            return Ok(notice(format!("❌ The squad data source has no team matching **{}**.", t.name)));
        }
        Err(e) => return Ok(unavailable("This squad", &e)),
    };
    let squad = &lookup.value.data;
    let mut by_pos: Vec<((u8, String), Vec<&SquadPlayer>)> = vec![];
    for p in &squad.players {
        let k = position_rank(p.position.as_deref());
        match by_pos.iter_mut().find(|(pk, _)| *pk == k) {
            Some((_, list)) => list.push(p),
            None => by_pos.push((k, vec![p])),
        }
    }
    by_pos.sort_by(|a, b| a.0.cmp(&b.0));
    let mut embed = CreateEmbed::new()
        .title(format!("👥 {} · squad", t.name))
        .color(COLOR)
        .description(format!("{} players (as registered at {}).\n\n{}", squad.players.len(), lookup.value.provider, source_note(&lookup)));
    for ((_, name), mut players) in by_pos {
        players.sort_by_key(|p| p.number.unwrap_or(999));
        let lines: Vec<String> = players
            .iter()
            .map(|p| {
                let n = p.number.map(|n| format!("`{n:>2}` ")).unwrap_or_else(|| "`  ` ".into());
                let age = p.age.map(|a| format!(" · {a}")).unwrap_or_default();
                format!("{n}{}{age}", p.name)
            })
            .collect();
        embed = embed.field(format!("{name}s"), clip(&lines.join("\n"), 1024), true);
    }
    Ok(embed.into())
}

async fn team_stats(fb: &Football, args: &Args) -> Result<Reply> {
    let c = match pick_competition(args) {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let t = match resolve_team(fb, args.get("team").unwrap_or_default()).await {
        Ok(t) => t,
        Err(r) => return Ok(r),
    };
    let lookup = match fb.standings(c, Purpose::User).await {
        Ok(l) => l,
        Err(e) => return Ok(unavailable("This competition's table", &e)),
    };
    let wanted = football::fold(t.name.split(" - ").next().unwrap_or(&t.name));
    let row = lookup.value.data.groups.iter().find_map(|g| {
        g.rows
            .iter()
            .find(|r| r.team_slug.as_deref() == Some(t.slug.as_str()) || football::fold(r.team.split(" - ").next().unwrap_or(&r.team)) == wanted)
            .map(|r| (g.name.clone(), r.clone()))
    });
    let Some((group, r)) = row else {
        return Ok(notice(format!(
            "ℹ️ **{}** has no table entry in **{}** (the data source has no statistics for it there).",
            t.name, c.name
        )));
    };
    let games = r.played.max(1) as f64;
    let form = match fb.team(&t.slug, Purpose::User).await {
        Ok(l) => {
            let name = l.value.data.name.clone();
            let f: Vec<char> = l
                .value
                .data
                .matches
                .iter()
                .filter(|m| m.competition == c.key && m.status == Status::Finished)
                .rev()
                .take(5)
                .filter_map(|m| result_letter(m, &name))
                .collect();
            f.into_iter().rev().collect::<String>()
        }
        Err(_) => String::new(),
    };
    let mut embed = CreateEmbed::new()
        .title(format!("📈 {} · {}", t.name, c.name))
        .color(COLOR)
        .field("Position", format!("{}{}", ordinal(r.position), if group.is_empty() { String::new() } else { format!(" ({group})") }), true)
        .field("Points", format!("{} ({:.2} per game)", r.points, r.points as f64 / games), true)
        .field("Played", format!("{} · {}W {}D {}L", r.played, r.won, r.drawn, r.lost), true)
        .field("Goals", format!("{} scored, {} conceded ({:+})", r.goals_for, r.goals_against, r.goals_for - r.goals_against), true)
        .field("Per game", format!("{:.2} scored, {:.2} conceded", r.goals_for as f64 / games, r.goals_against as f64 / games), true);
    if !form.is_empty() {
        embed = embed.field("Form (last 5)", format!("`{form}`"), true);
    }
    if let Some(n) = &r.note {
        embed = embed.field("Zone", n.clone(), true);
    }
    Ok(embed.description(source_note(&lookup)).into())
}

// ─────────────────────────────────────────────
//  Players
// ─────────────────────────────────────────────

async fn player(fb: &Football, input: &str) -> Result<Reply> {
    let input = input.trim();
    let mut lookup = if !input.contains(' ') { fb.player(input, Purpose::User).await } else { Err(FetchError::NotFound) };
    if matches!(lookup, Err(FetchError::NotFound)) {
        lookup = match fb.search(input, Purpose::User).await {
            Ok(s) => match best_match(&s.value.data.players, input) {
                Some((_, slug)) => fb.player(slug, Purpose::User).await,
                None => Err(FetchError::NotFound),
            },
            Err(e) => Err(e),
        };
    }
    let lookup = match lookup {
        Ok(l) => l,
        Err(FetchError::NotFound) => return Ok(notice(format!("❌ I couldn't find a player called **{}**.", clip(input, 60)))),
        Err(e) => return Ok(unavailable("This player", &e)),
    };
    let p = &lookup.value.data;
    let mut embed = CreateEmbed::new().title(format!("👤 {}", p.name)).color(COLOR);
    if let Some(photo) = &p.photo
        && photo.ends_with(".png")
    {
        embed = embed.thumbnail(photo);
    }
    let context: Vec<String> = [p.team.clone(), p.competition.clone()].into_iter().flatten().collect();
    let mut text = if context.is_empty() { String::new() } else { format!("**{}**\n", context.join(" · ")) };
    if p.stats.is_empty() {
        text.push_str("Season statistics are not provided by the current data source for this player.\n");
    }
    for (k, v) in p.stats.iter().take(24) {
        embed = embed.field(k, v, true);
    }
    Ok(embed.description(format!("{text}\n{}", source_note(&lookup))).footer(CreateEmbedFooter::new("Statistics for the player's current main competition")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(status: Status, score: Option<(u32, u32)>) -> Fixture {
        Fixture {
            id: "x1".into(),
            slug: "a-vs-b".into(),
            competition: "brazilian-serie-a".into(),
            competition_name: "Brazilian Serie A".into(),
            home: "Flamengo - RJ".into(),
            away: "Palmeiras - SP".into(),
            home_score: score.map(|s| s.0),
            away_score: score.map(|s| s.1),
            status,
            status_text: "2nd half".into(),
            kickoff: 1_795_000_000,
        }
    }

    #[test]
    fn definitions_fit_discord_limits() {
        for cmd in register() {
            let v = serde_json::to_value(&cmd).unwrap();
            let name = v["name"].as_str().unwrap();
            assert!(v["description"].as_str().unwrap().len() <= 100, "{name}");
            fn check(opts: &serde_json::Value) {
                let opts = opts.as_array().map(Vec::as_slice).unwrap_or_default();
                assert!(opts.len() <= 25);
                for o in opts {
                    let n = o["name"].as_str().unwrap();
                    assert!(n.len() <= 32 && n.chars().all(|c| c.is_ascii_lowercase() || c == '-' || c == '_'), "{n}");
                    assert!(o["description"].as_str().unwrap().len() <= 100, "{n}");
                    check(&o["options"]);
                }
            }
            check(&v["options"]);
        }
    }

    #[test]
    fn fixture_lines() {
        assert!(fixture_line(&fixture(Status::Live, Some((1, 0))), false).contains("**Flamengo - RJ 1–0 Palmeiras - SP**"));
        assert!(fixture_line(&fixture(Status::Finished, Some((2, 2))), false).contains("FT"));
        assert!(fixture_line(&fixture(Status::Upcoming, None), false).contains("<t:1795000000:t>"));
    }

    #[test]
    fn pages() {
        let lines: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
        let blocks = vec![("A".to_string(), lines[..30].to_vec()), ("B".to_string(), lines[30..].to_vec())];
        let (first, total) = paginate(&blocks, 0);
        assert_eq!(total, 3);
        assert!(first.starts_with("**A**"));
        let (second, _) = paginate(&blocks, 1);
        assert!(second.starts_with("**A** (cont.)"));
        let (last, _) = paginate(&blocks, 99);
        assert!(last.contains("line 59"));
        assert!(page_buttons("la:-", 0, 1).is_empty());
        let ids: Vec<String> = (0..3).map(|p| format!("{PAGE_PREFIX}cf:brazilian-campeonato-mineiro-division-1:2026-11-22:{p}")).collect();
        assert!(ids.iter().all(|i| i.len() <= 100));
    }

    #[test]
    fn dates() {
        let today = Utc::now().date_naive();
        assert_eq!(parse_date(None).ok(), Some(today));
        assert_eq!(parse_date(Some("tomorrow")).ok(), Some(today + ChronoDuration::days(1)));
        assert_eq!(parse_date(Some("2026-11-22")).ok(), NaiveDate::from_ymd_opt(2026, 11, 22));
        assert_eq!(parse_date(Some("22/11/2026")).ok(), NaiveDate::from_ymd_opt(2026, 11, 22));
        assert!(parse_date(Some("someday")).is_err());
    }

    #[test]
    fn search_picks_the_closest_name() {
        let r = vec![
            ("Abrantes e Benfica".to_string(), "abrantes-e-benfica".to_string()),
            ("Benfica".to_string(), "benfica".to_string()),
            ("Benfica B Women".to_string(), "benfica-b-women".to_string()),
        ];
        assert_eq!(best_match(&r, "benfica").unwrap().1, "benfica");
        assert_eq!(best_match(&r, "Benfica B").unwrap().1, "benfica-b-women");
        assert_eq!(plural(1, "goal"), "1 goal");
        assert_eq!(plural(3, "assist"), "3 assists");
    }

    #[test]
    fn form_and_ordinals() {
        let f = fixture(Status::Finished, Some((2, 1)));
        assert_eq!(result_letter(&f, "Flamengo - RJ"), Some('W'));
        assert_eq!(result_letter(&f, "Palmeiras - SP"), Some('L'));
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(12), "12th");
        assert_eq!(ordinal(23), "23rd");
    }

    #[test]
    fn competition_autocomplete() {
        let fb = Football::from_env(None);
        let all = autocomplete_choices(&fb, "competition", "");
        assert_eq!(all.len(), 25, "capped at Discord's limit");
        let copa = autocomplete_choices(&fb, "competition", "copa");
        assert!(copa.len() >= 4);
        assert!(autocomplete_choices(&fb, "fixture", "flamengo").is_empty(), "nothing cached yet");
    }

    #[test]
    fn errors_are_explained() {
        assert!(explain(&FetchError::Blocked(Blocked::Budget)).contains("quota resets"));
        assert!(explain(&FetchError::PlanRestricted("x".into())).contains("free plan"));
        assert!(friendly_note("not requested: today's request budget is used up").contains("quota"));
    }

    /// Embed limits Discord enforces: a reply that breaks them fails to send.
    fn check_limits(label: &str, r: &Reply) {
        let v = serde_json::to_value(&r.embed).unwrap();
        let desc = v["description"].as_str().unwrap_or_default();
        assert!(desc.chars().count() <= 4096, "{label}: description too long");
        let fields = v["fields"].as_array().cloned().unwrap_or_default();
        assert!(fields.len() <= 25, "{label}: too many fields");
        let mut total = desc.chars().count() + v["title"].as_str().unwrap_or_default().chars().count();
        for f in &fields {
            let (n, val) = (f["name"].as_str().unwrap(), f["value"].as_str().unwrap());
            assert!(!n.is_empty() && n.chars().count() <= 256, "{label}: field name");
            assert!(!val.is_empty() && val.chars().count() <= 1024, "{label}: field '{n}' value {}", val.chars().count());
            total += n.chars().count() + val.chars().count();
        }
        assert!(total <= 6000, "{label}: embed total {total}");
        println!("===== {label} ({total} chars, {} fields)\n{}", fields.len(), v["title"].as_str().unwrap_or_default());
        println!("{desc}");
        for f in &fields {
            println!("--- {}\n{}", f["name"].as_str().unwrap(), f["value"].as_str().unwrap());
        }
    }

    fn args(path: &[&str], values: &[(&str, &str)]) -> Args {
        Args {
            path: path.iter().map(|s| s.to_string()).collect(),
            values: values.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    /// Every view against the real providers. Needs network; uses the keys in
    /// the environment (`FOOTBALL_DATA_TOKEN`, `API_FOOTBALL_KEY`: one squad
    /// costs 2 API-Football calls the first time). Run with
    /// `cargo test live_football -- --ignored --nocapture --test-threads=1`.
    #[tokio::test]
    #[ignore]
    async fn live_football() {
        let fb = Football::from_env(None);
        let serie_a = competitions::by_key("brazilian-serie-a").unwrap();
        let libertadores = competitions::by_key("conmebol-copa-libertadores").unwrap();
        let today = Utc::now().date_naive();
        // What the scheduler would have cached.
        for c in [serie_a, libertadores, competitions::by_key("english-premier-league").unwrap()] {
            for d in [today - ChronoDuration::days(1), today, today + ChronoDuration::days(1)] {
                fb.fixtures(c, d, Purpose::Scheduled).await.unwrap();
            }
            fb.standings(c, Purpose::Metadata).await.unwrap();
        }
        fb.scorers(serie_a, false, Purpose::Metadata).await.unwrap();
        check_limits("competition list", &competition_list(&fb));
        check_limits("live all", &live_list(&fb, None, 0).await.unwrap());
        check_limits("live serie a", &live_list(&fb, Some(serie_a), 0).await.unwrap());
        check_limits("fixtures 2026-11-22", &competition_fixtures(&fb, serie_a, NaiveDate::from_ymd_opt(2026, 11, 22).unwrap(), 0).await.unwrap());
        check_limits("table", &table(&fb, serie_a).await.unwrap());
        check_limits("table groups", &table(&fb, libertadores).await.unwrap());
        check_limits("table cup", &table(&fb, competitions::by_key("brazilian-cup").unwrap()).await.unwrap());
        check_limits("scorers", &scorers(&fb, serie_a, false).await.unwrap());
        check_limits("assists", &scorers(&fb, serie_a, true).await.unwrap());

        let teams = autocomplete_choices(&fb, "team", "flamengo");
        assert!(!teams.is_empty(), "Flamengo is in a cached table");
        check_limits("team info", &team_info(&fb, &args(&["team", "info"], &[("team", "flamengo-rj")])).await.unwrap());
        check_limits("team fixtures", &team_fixtures(&fb, "flamengo-rj", 0).await.unwrap());
        check_limits("team stats", &team_stats(&fb, &args(&["team", "stats"], &[("team", "flamengo-rj"), ("competition", "brazilian-serie-a")])).await.unwrap());
        check_limits("team typed", &team_info(&fb, &args(&["team", "info"], &[("team", "Benfica")])).await.unwrap());
        if fb.has_squads() {
            check_limits("squad", &team_squad(&fb, &args(&["team", "squad"], &[("team", "flamengo-rj")])).await.unwrap());
        }
        check_limits("player", &player(&fb, "kevin-viveros").await.unwrap());
        check_limits("player typed", &player(&fb, "Lionel Messi").await.unwrap());

        // A finished Premier League match and one from the fixture list.
        for view in ["details", "events", "lineup", "stats"] {
            let r = match_view(&fb, &args(&["match", view], &[("fixture", "bournemouth-afc-vs-ipswich-town")]), view, false).await.unwrap();
            check_limits(&format!("match {view}"), &r);
        }
        check_limits("live match (finished)", &match_view(&fb, &args(&["match"], &[("fixture", "bournemouth-afc-vs-ipswich-town")]), "live", true).await.unwrap());
        let picks = autocomplete_choices(&fb, "fixture", "");
        println!("fixture choices: {}", picks.len());
        if let Some(live) = fb.cached_now::<Vec<Fixture>>("live").and_then(|l| l.data.into_iter().find(|f| !f.slug.is_empty())) {
            check_limits("live match", &match_view(&fb, &args(&["match"], &[("fixture", &live.slug)]), "live", true).await.unwrap());
        }
        check_limits("unknown match", &match_view(&fb, &args(&["match"], &[("fixture", "no-such-match-zzz")]), "details", false).await.unwrap());
        if let Ok(token) = std::env::var("FOOTBALL_DATA_TOKEN") {
            let fd = crate::football::providers::football_data::FootballData::new(token);
            let day = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
            let m = fd.fixtures(&fb.up, serie_a, "BSA", day, Purpose::User).await.unwrap();
            assert!(!m.is_empty() && m.iter().all(|f| f.status == Status::Finished && f.score().is_some()));
            // No Libertadores table there (404): its fallback covers fixtures only.
            assert_eq!(fd.standings(&fb.up, "CLI", Purpose::User).await.unwrap_err(), FetchError::NotFound);
            let s = fd.standings(&fb.up, "BSA", Purpose::User).await.unwrap();
            assert!(!s.groups.is_empty());
            let sc = fd.scorers(&fb.up, "PL", Purpose::User).await.unwrap();
            assert!(!sc.is_empty());
            println!("football-data.org: {} BSA results, {} BSA table(s), PL top scorer {}", m.len(), s.groups.len(), sc[0].player);
        }
        for p in Provider::ALL {
            fb.up.with_gate(p, |g| println!("{}: {} calls used, health {}", p.id(), g.used, g.health.id()));
        }
    }
}
