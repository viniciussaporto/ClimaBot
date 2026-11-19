//! `/help [topic]`: an overview, and one page per command explaining how to
//! use it, where its data comes from and how the reply is built. Replies are
//! ephemeral (only the person asking sees them).

use crate::{
    football::{
        self,
        model::{Fixture, Provider},
        quota::Health,
    },
    metrics, usage,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serenity::all::{
    CommandInteraction, CommandOptionType, Context, CreateCommand, CreateCommandOption, CreateEmbed, CreateEmbedFooter,
    CreateInteractionResponse, CreateInteractionResponseMessage, ResolvedValue,
};

const COLOR: u32 = 0x0099ff;
const FOOTER: &str = "ClimaBot · open source: github.com/viniciussaporto/ClimaBot";

struct Topic {
    name: &'static str,
    description: &'static str,
    title: &'static str,
    intro: &'static str,
    sections: &'static [(&'static str, &'static str)],
}

const TOPICS: &[Topic] = &[
    Topic {
        name: "overview",
        description: "All commands at a glance",
        title: "🤖 ClimaBot — Commands",
        intro: "Use `/help <topic>` for how each command works, where its data comes from and how the reply is built.",
        sections: &[
            (
                "🌤 Weather · `/help weather`, `/help forecast`",
                "`/weather now <location> [units]` — Current conditions with an animated precipitation map\n\
                 `/weather forecast <location> [units]` — 5-day forecast",
            ),
            (
                "🔴 Live football · `/help live`",
                "`/football live all|competition|match` — Live scores, clock and latest events",
            ),
            (
                "⚽ Football · `/help football`",
                "`/football competition list|fixtures|table|scorers` — Competitions, matches, standings, top scorers\n\
                 `/football match details|events|lineup|stats` — Match report\n\
                 `/football team info|fixtures|squad|injuries|stats|compare` — Teams\n\
                 `/football player stats|career|injuries` — Players",
            ),
            (
                "⭐ Following · `/help follow`",
                "`/football follow team|competition|manage|feed` — Your teams and competitions, your feed, optional DM alerts",
            ),
            (
                "📣 Server posts · `/help server`",
                "`/football server setup|team|preview` — Daily summaries and live alerts in a channel (moderators; off by default)",
            ),
            ("🎭 Roles · `/help roles`", "`/roles` — Pick self-assignable roles from a menu"),
            (
                "📦 Price tracking · `/help pt`",
                "`/pt add <url>` — Track a product page\n\
                 `/pt list` · `/pt history <item>` · `/pt remove <item>`",
            ),
            (
                "📶 Status · `/help status`",
                "Live status of the bot and its data sources, and how much of each API's limit is used today",
            ),
            ("ℹ️ About · `/help about`", "Data sources, privacy and source code"),
        ],
    },
    Topic {
        name: "weather",
        description: "How /weather now works",
        title: "🌤 /weather now",
        intro: "`/weather now <location> [units]` — the weather right now anywhere on Earth, with a map of \
                rain, snow and thunderstorms around it.",
        sections: &[
            (
                "How to use",
                "• `location`: a city, address, landmark or postcode. Add a state or country when a name is \
                 ambiguous (`Springfield, Illinois`).\n\
                 • `units`: **Metric** (°C, km/h, hPa, the default) or **Imperial** (°F, mph, inHg).",
            ),
            (
                "Where the data comes from",
                "• **OpenCage** turns the location into coordinates (its best match is used).\n\
                 • **Open-Meteo** gives the current conditions and the precipitation forecast.\n\
                 • **OpenStreetMap** tiles make the map's background.",
            ),
            (
                "How the reply is built",
                "• **Title:** the place as OpenCage resolved it, with an icon for the current weather.\n\
                 • **Left column:** temperature, feels-like, humidity, cloud cover.\n\
                 • **Right column:** wind speed, wind direction (degrees and compass point), sea-level pressure.\n\
                 • **Map:** an animated GIF about 290 × 220 km around the place, with the location marked. \
                 Frame 1 is now; the next 3 frames are the forecast for each of the next 3 hours.",
            ),
            (
                "Reading the map",
                "• Colours go from blue (0.1 mm/h, drizzle) through green and yellow to red and magenta \
                 (16+ mm/h); the bar in the corner is the scale.\n\
                 • Snow is drawn whiter. ⚡ marks thunderstorms.\n\
                 • The precipitation comes from a forecast model on a grid of points, smoothed between \
                 them: it shows where rain is likely, not a radar picture.\n\
                 • When no precipitation is likely nearby in the next hours, the map is drawn dry and the \
                 caption says so.",
            ),
            (
                "Good to know",
                "• Maps are reused for 10 minutes for the same place.\n\
                 • If the map can't be made in time, you get the weather without it.\n\
                 • \"I couldn't find a location\" means OpenCage had no match: add a country.",
            ),
        ],
    },
    Topic {
        name: "forecast",
        description: "How /weather forecast works",
        title: "🌦 /weather forecast",
        intro: "`/weather forecast <location> [units]` — the next 5 days, starting today.",
        sections: &[
            ("How to use", "Same options as `/weather now`: a `location`, and optionally **Imperial** `units`."),
            (
                "Where the data comes from",
                "OpenCage finds the place; **Open-Meteo**'s daily forecast gives the highest and lowest \
                 temperature and the highest chance of precipitation of each day, in the place's own time zone.",
            ),
            (
                "How the reply is built",
                "One box per day, dated day-first (`Tue, 29/09`): ⬆ highest and ⬇ lowest temperature, and the \
                 day's highest chance of rain. A `–` means the model has no value for that day.",
            ),
        ],
    },
    Topic {
        name: "live",
        description: "How live scores work",
        title: "🔴 /football live",
        intro: "Live football scores in the competitions I follow (see `/football competition list`).",
        sections: &[
            (
                "How to use",
                "• `/football live all [competition]`: every live match, grouped by competition.\n\
                 • `/football live competition <competition>`: one competition.\n\
                 • `/football live match <match>`: start typing a team name and pick the match from the list. \
                 Buttons switch between the live view, events, line-ups and stats.",
            ),
            (
                "Where the data comes from",
                "**SportScore**. While a followed match is about to start, under way or just ended, I refresh \
                 the live scoreboard about once a minute (SportScore itself updates every 60 seconds). Details \
                 of live matches (clock, events) are refreshed every 2–3 minutes for the main Brazilian and \
                 South American competitions, and for any match someone asked about recently.",
            ),
            (
                "How the reply is built",
                "• **Lists:** `🔴 Home 1–0 Away · 2nd half` per match; ◀ ▶ buttons when they don't fit one page.\n\
                 • **One match:** score, status and minute, half-time score, and the latest 8 events \
                 (⚽ goals, 🟨🟥 cards, 🔄 substitutions, 📺 VAR).\n\
                 • **Freshness:** every reply ends with when it was updated. If the scores couldn't be \
                 refreshed, it says so and shows the last saved copy with its age.",
            ),
            (
                "Good to know",
                "• No live matches? The reply says when the next one kicks off.\n\
                 • A finished or upcoming match is never shown as live: the reply tells you its status \
                 and offers the full report.\n\
                 • Want alerts instead of checking? See `/help follow`.",
            ),
        ],
    },
    Topic {
        name: "football",
        description: "How /football works",
        title: "⚽ /football",
        intro: "Fixtures, results, tables, scorers, match reports, teams, squads and players. Not live: every reply \
                says how old its data is (`/football live` is). Buttons under a reply switch between views.",
        sections: &[
            (
                "Competitions",
                "`/football competition list` shows them all: Brasileirão A/B/C, Copa do Brasil, Supercopa, \
                 Copa do Nordeste, Copa Verde, Paulistão, Carioca, Mineiro, Gaúcho, Libertadores, Sudamericana, \
                 the UEFA competitions, and the leagues and cups of England, Spain, Germany, Italy, France and Portugal.\n\
                 • `fixtures <competition> [date]`: matches on a day (`today`, `tomorrow`, `yesterday`, `YYYY-MM-DD`).\n\
                 • `table`: standings, one table per group where there are groups. Knockout cups have none.\n\
                 • `scorers [stat]`: top 15 for goals or assists.",
            ),
            (
                "Matches",
                "Pick a match of yesterday, today or tomorrow from the list as you type. `details` (score, key \
                 events, formations), `events` (the whole timeline), `lineup` (XIs and substitutes, official or \
                 expected) and `stats` (when the source has them) are also buttons under each other.",
            ),
            (
                "Teams",
                "• `info`: league positions, form (W/D/L, oldest first), last 5 results, next 5 matches.\n\
                 • `fixtures`, `squad` (by position, numbers, ages), `stats <competition>` (record, goals, per game).\n\
                 • `injuries`: see below.\n\
                 • `compare <team> <other>`: head-to-head record, recent meetings, positions and form side by side.\n\
                 Buttons: Info · Fixtures · Squad · Injuries · ⭐ Follow.",
            ),
            (
                "Players",
                "• `player stats`: this season, in the player's main competition.\n\
                 • `player career`: clubs and national teams by season, season statistics (2022–2024 only, a \
                 limit of the free plan), transfers with fees, trophies. International appearance counts aren't available.\n\
                 • `player injuries`: availability (out, suspended, or available), current absence with its \
                 type and expected return, and past absences.\n\
                 When several players share a name, pick the right one from a menu.",
            ),
            (
                "Injuries and suspensions: what to expect",
                "The free sources publish **no current injury list per team**. `team injuries` lets you pick \
                 players from the squad and checks each one's absence history (once a day per player). Absences \
                 are only as complete as the source: **Brazilian competitions are often missing recent entries**, \
                 and suspensions for accumulated cards are usually not listed. The reply says when data looks \
                 out of date; it never guesses.",
            ),
            (
                "Where the data comes from",
                "• **SportScore**: fixtures, live scores, events, line-ups, tables, scorers, teams, head-to-head.\n\
                 • **football-data.org**: backup for fixtures, tables and scorers (delayed; the reply says so).\n\
                 • **API-Football**: squads, careers, transfers, trophies, injuries (100 requests a day, so each \
                 player is looked up once and kept for weeks).\n\
                 The bot refreshes data on a schedule and keeps it; commands read that copy.",
            ),
            (
                "How to read a reply",
                "• The end of every reply names the source and says when the data was updated.\n\
                 • ⚠️ means a refresh failed and you're seeing the last saved copy.\n\
                 • \"Not provided by the current data source\" means the source doesn't have it: nothing is guessed.",
            ),
        ],
    },
    Topic {
        name: "follow",
        description: "Follow teams and competitions",
        title: "⭐ /football follow",
        intro: "Follow teams and competitions to get one feed for all of them, and optionally alerts by DM.",
        sections: &[
            (
                "How to use",
                "• `/football follow team <team>` or the **⭐ Follow** button under a team: follow or unfollow it.\n\
                 • `/football follow competition <competition>`: the same for a competition.\n\
                 • `/football follow manage`: menus to pick competitions, unfollow teams, choose the time of your \
                 daily summary, and buttons to switch DM alerts on or off.\n\
                 • `/football follow feed`: your feed now.\n\
                 Up to 25 teams and every competition. Only you see these replies.",
            ),
            (
                "What the feed shows",
                "🔴 live matches · 📅 today's and tomorrow's matches · ✅ yesterday's results · ⭐ important matches \
                 this week (your teams, cup ties, top-6 clashes) · ⏭ each team's next match · 📊 table positions \
                 with changes since yesterday (▲▼), leaders and big movers of followed leagues.",
            ),
            (
                "DM alerts (off by default)",
                "• **Daily DM**: the feed once a day, at the hour you pick (UTC).\n\
                 • **Live alerts DM**: kick-off, each goal and the final score of matches you follow, a minute or \
                 two after they happen.\n\
                 Switching one on sends you a test DM first: allow direct messages from server members if it fails.",
            ),
            (
                "Good to know",
                "Alerts and the feed reuse the data the bot already refreshes on schedule, so following more \
                 doesn't slow anything down. Only teams in the competitions I follow appear in the feed.",
            ),
        ],
    },
    Topic {
        name: "server",
        description: "Set up football posts for a server (moderators)",
        title: "📣 /football server",
        intro: "Daily football summaries and live alerts posted in a channel of this server. **Off until a \
                moderator switches them on.**",
        sections: &[
            (
                "Who can set it up",
                "Members with **Manage Server**, **Manage Channels**, **Moderate Members** or **Administrator**. \
                 Everyone else can see `/football server preview` but can't change anything.",
            ),
            (
                "Setting it up",
                "1. `/football server setup` opens the settings panel (only you see it).\n\
                 2. Pick the **channel** for the posts.\n\
                 3. Pick **competitions** in the two menus, and add **teams** with `/football server team <team>` \
                 (run it again to remove one).\n\
                 4. Choose the **time** of the daily summary (UTC).\n\
                 5. Switch on **Daily summary** and/or **Live alerts**.\n\
                 6. Press **Send a test post** to check the bot can post there.",
            ),
            (
                "Permissions the bot needs",
                "In the chosen channel: **View Channel**, **Send Messages** and **Embed Links**. The test post \
                 tells you if one is missing.",
            ),
            (
                "What gets posted",
                "• **Daily summary**: once a day at the chosen hour, the same feed as `/help follow` describes, \
                 for what the server follows.\n\
                 • **Live alerts**: kick-off, goals and final scores of followed matches, grouped into one message \
                 per update.\n\
                 **Turn everything off** in the panel stops all posts at once.",
            ),
        ],
    },
    Topic {
        name: "roles",
        description: "How /roles works",
        title: "🎭 /roles",
        intro: "`/roles` — pick your own roles in this server from a menu.",
        sections: &[
            (
                "How to use",
                "Run `/roles` in a server, open the menu and select the roles you want; deselect a role to \
                 remove it. ◀ ▶ switch pages when there are more than 25 roles. Only you see the menu.",
            ),
            (
                "Which roles are offered",
                "Every role the bot can manage, except roles with moderation or administrator permissions, \
                 roles managed by integrations or bots (boosters, bot roles), `@everyone`, and roles at or \
                 above the bot's own highest role. To offer a role, move it below the bot's role.",
            ),
            (
                "If it doesn't work",
                "The bot needs the **Manage Roles** permission, and its role must be above the roles it hands out.",
            ),
        ],
    },
    Topic {
        name: "pt",
        description: "How price tracking (/pt) works",
        title: "📦 /pt — price tracking",
        intro: "Track product pages and get a DM when the price changes. Replies are only visible to you.",
        sections: &[
            (
                "How to use",
                "• `/pt add <url>`: the product page's address. I read the price right away and confirm it.\n\
                 • `/pt list`: your products with their current price and next check.\n\
                 • `/pt history <item>`: every price seen, with the lowest and highest.\n\
                 • `/pt remove <item>`: stop tracking. `item` is the number from `/pt list` or the URL.\n\
                 You can track up to 25 products.",
            ),
            (
                "How prices are checked",
                "Each product is checked about once an hour, at a slightly random time. The page is loaded \
                 directly or, for shops with bot protection, in a real browser; the price is read from the \
                 page's product data. Any currency works.",
            ),
            (
                "What you get",
                "• A DM when the price changes: old price, new price and the change in %.\n\
                 • A one-time warning if the page can't be read for about 24 hours (moved, sold out, blocked).\n\
                 • Both DMs have a **❌ Stop tracking** button. Allow DMs from server members to receive them.",
            ),
            (
                "Shops",
                "Tested: KaBuM, Amazon Brasil, Magazine Luiza, Pichau, Terabyte, AliExpress. Other shops may \
                 work. Prices of shops that adapt to your country (AliExpress) are the server's country prices.",
            ),
        ],
    },
    Topic {
        name: "status",
        description: "Live status of the bot and its APIs' limits",
        title: "📶 Status",
        intro: "Built live when you ask.",
        sections: &[],
    },
    Topic {
        name: "about",
        description: "Data sources, privacy and links",
        title: "ℹ️ About ClimaBot",
        intro: "An open-source Discord bot written in Rust.",
        sections: &[
            (
                "Data sources",
                "Weather: [Open-Meteo](https://open-meteo.com) · Geocoding: [OpenCage](https://opencagedata.com) · \
                 Map: © [OpenStreetMap](https://www.openstreetmap.org/copyright) contributors\n\
                 Football: [Powered by SportScore](https://sportscore.com/), \
                 [football-data.org](https://www.football-data.org/), [API-Football](https://www.api-football.com/)\n\
                 Weather icons: [Icons8](https://icons8.com)",
            ),
            (
                "Privacy",
                "• Weather and football commands store nothing about you; locations are sent to OpenCage \
                 and Open-Meteo to answer.\n\
                 • `/pt` stores your Discord user id, the product URLs you add and their prices, until you \
                 remove them.\n\
                 • Server logs (commands used, places looked up, errors, with your user id) are kept                  for up to 30 days.",
            ),
            (
                "Links",
                "[Add ClimaBot to a server](https://discord.com/oauth2/authorize?client_id=1109598244884979792) · \
                 [Source code](https://github.com/viniciussaporto/ClimaBot) · \
                 [Report a bug](https://github.com/viniciussaporto/ClimaBot/issues) · \
                 [Website](https://vinisaporto.de)",
            ),
        ],
    },
];

pub fn register() -> CreateCommand {
    TOPICS.iter().fold(CreateCommand::new("help").description("How ClimaBot's commands work"), |cmd, t| {
        cmd.add_option(CreateCommandOption::new(CommandOptionType::SubCommand, t.name, t.description))
    })
}

fn embed(topic: &Topic) -> CreateEmbed {
    topic
        .sections
        .iter()
        .fold(CreateEmbed::new().title(topic.title).description(topic.intro).color(COLOR), |e, (name, value)| {
            e.field(*name, *value, false)
        })
        .footer(CreateEmbedFooter::new(FOOTER))
}

fn topic_of(cmd: &CommandInteraction) -> &'static Topic {
    let chosen =
        cmd.data.options().into_iter().find_map(|o| matches!(o.value, ResolvedValue::SubCommand(_)).then_some(o.name));
    chosen.and_then(|n| TOPICS.iter().find(|t| t.name == n)).unwrap_or(&TOPICS[0])
}

pub async fn handle(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let topic = topic_of(cmd);
    let page = if topic.name == "status" { status_embed() } else { embed(topic) };
    let msg = CreateInteractionResponseMessage::new().embed(page).ephemeral(true);
    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await?;
    Ok(())
}

// ─────────────────────────────────────────────
//  /help status
// ─────────────────────────────────────────────

fn ago(t: Option<DateTime<Utc>>) -> String {
    t.map_or_else(|| "not yet".into(), |t| format!("<t:{}:R>", t.timestamp()))
}

fn ok(b: bool) -> &'static str {
    if b { "✅" } else { "❌" }
}

/// Icon for a service from its last success and last error.
fn usage_icon(u: &usage::Usage) -> &'static str {
    match (u.last_success, u.last_error) {
        (None, None) => "⚪",
        (Some(s), Some(e)) if e > s => "⚠️",
        (None, Some(_)) => "⚠️",
        _ => "✅",
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn weather_status() -> String {
    let oc = usage::get(usage::Service::OpenCage);
    let om = usage::get(usage::Service::OpenMeteo);
    let osm = usage::get(usage::Service::OsmTiles);
    let opencage = match oc.reported {
        Some((limit, remaining, reset)) => format!(
            "{} **OpenCage** (geocoding): {} of {} requests used today, {} left{}",
            usage_icon(&oc),
            thousands(limit.saturating_sub(remaining)),
            thousands(limit),
            thousands(remaining),
            reset.map(|r| format!(" · resets <t:{}:R>", r.timestamp())).unwrap_or_default()
        ),
        None => format!(
            "{} **OpenCage** (geocoding): {} requests today · free plan: 2,500/day",
            usage_icon(&oc),
            thousands(oc.calls)
        ),
    };
    format!(
        "{opencage}\n\
         {} **Open-Meteo** (weather): ≈ {} of 10,000 calls today · a precipitation map costs 108, \
         so maps are only made when rain is likely\n\
         {} **OpenStreetMap** (map tiles): {} downloaded today, {} cached for a week\n\
         Last successful weather request: {}",
        usage_icon(&om),
        thousands(om.calls),
        usage_icon(&osm),
        thousands(osm.calls),
        super::radar::cached_tiles(),
        ago(om.last_success),
    )
}

fn football_status() -> String {
    let Some(fb) = football::get() else {
        return "⚪ Not started yet.".into();
    };
    let mut lines = vec![];
    for p in Provider::ALL {
        let role = match p {
            Provider::SportScore => "everything, live included",
            Provider::FootballData => "backup",
            Provider::ApiFootball => "squads",
        };
        let line = fb
            .up
            .with_gate(p, |g| {
                let (icon, state) = match &g.health {
                    Health::Healthy => ("✅", String::new()),
                    Health::RateLimitedMinute { until } => {
                        ("⏳", format!(" · rate-limited until <t:{}:t>", until.timestamp()))
                    }
                    Health::DailyQuotaExhausted { until } => {
                        ("⛔", format!(" · daily limit reached, resets <t:{}:R>", until.timestamp()))
                    }
                    Health::Unavailable { until, .. } => {
                        ("⚠️", format!(" · failing, next try <t:{}:R>", until.timestamp()))
                    }
                    Health::Unauthorized => ("❌", " · API key rejected".to_string()),
                };
                format!(
                    "{icon} **{p}** ({role}): {} of {} requests today, max {}/min{state} · last success {}",
                    thousands(u64::from(g.used)),
                    thousands(u64::from(g.limits.daily)),
                    g.limits.per_minute,
                    ago(g.last_success)
                )
            })
            .unwrap_or_else(|| format!("⚪ **{p}** ({role}): not configured"));
        lines.push(line);
    }
    let live = fb.cached_now::<Vec<Fixture>>("live");
    let updated = live.as_ref().map_or_else(|| "never".into(), |l| format!("<t:{}:R>", l.fetched_at));
    lines.push(if fb.in_watch_window() {
        format!(
            "🔴 Live scoreboard: polled every minute, {} live match(es) · updated {updated}",
            live.map_or(0, |l| l.data.len())
        )
    } else {
        format!("💤 Live scoreboard: idle, no followed match on right now · last updated {updated}")
    });
    lines.join("\n")
}

fn status_embed() -> CreateEmbed {
    let (discord, database) = crate::status::current();
    let latency = metrics::GATEWAY_LATENCY.get();
    let latency = if latency > 0.0 { format!(" · {:.0} ms latency", latency * 1000.0) } else { String::new() };
    let since = usage::since();
    let bot = format!(
        "{} Discord gateway{latency}\n{} Database\n📡 In {} servers · v{} · running since <t:{}:R>",
        ok(discord),
        ok(database),
        metrics::GUILDS.get(),
        env!("CARGO_PKG_VERSION"),
        since.timestamp()
    );
    let prices = format!(
        "{} products tracked · {} checks running late",
        metrics::TRACKED_PRODUCTS.get(),
        metrics::OVERDUE_CHECKS.get()
    );
    let counted = if since.date_naive() == Utc::now().date_naive() {
        format!("Weather counts start at the bot's last restart (<t:{}:t>). ", since.timestamp())
    } else {
        String::new()
    };
    CreateEmbed::new()
        .title("📶 Status")
        .description(format!(
            "Live status of the bot and the services it uses. Limits are the free plans'; days reset \
             at 00:00 UTC. {counted}Football limits are the bot's own budgets, kept below the providers'."
        ))
        .field("🤖 Bot", bot, false)
        .field("🌤 Weather and maps", weather_status(), false)
        .field("⚽ Football data", football_status(), false)
        .field("📦 Price tracking", prices, false)
        .color(COLOR)
        .footer(CreateEmbedFooter::new(FOOTER))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_fit_discord_limits() {
        assert!(TOPICS.len() <= 25);
        for t in TOPICS {
            assert!(t.name.len() <= 32 && t.name.chars().all(|c| c.is_ascii_lowercase()), "{}", t.name);
            assert!(t.description.len() <= 100);
            assert!(t.intro.chars().count() <= 4096);
            assert!(t.sections.len() <= 25);
            let mut total = t.title.chars().count() + t.intro.chars().count() + FOOTER.chars().count();
            for (name, value) in t.sections {
                assert!(name.chars().count() <= 256, "{}: {name}", t.name);
                assert!(value.chars().count() <= 1024, "{}: '{name}' is {} chars", t.name, value.chars().count());
                total += name.chars().count() + value.chars().count();
            }
            assert!(total <= 6000, "{}: {total} chars", t.name);
        }
    }

    #[test]
    fn status_page_fits() {
        let v = serde_json::to_value(status_embed()).unwrap();
        for f in v["fields"].as_array().unwrap() {
            assert!(f["value"].as_str().unwrap().chars().count() <= 1024);
        }
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(12), "12");
    }

    #[test]
    fn every_command_has_a_topic() {
        for name in ["weather", "forecast", "live", "football", "follow", "server", "roles", "pt"] {
            assert!(TOPICS.iter().any(|t| t.name == name), "{name}");
        }
        assert_eq!(TOPICS[0].name, "overview");
    }
}
