//! `/help [topic]`: an overview, and one page per command explaining how to
//! use it, where its data comes from and how the reply is built. Replies are
//! ephemeral (only the person asking sees them).

use anyhow::Result;
use serenity::all::{
    CommandInteraction, CommandOptionType, Context, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateEmbedFooter, CreateInteractionResponse, CreateInteractionResponseMessage, ResolvedValue,
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
                "`/weather <location> [units]` — Current conditions with an animated precipitation map\n\
                 `/forecast <location> [units]` — 5-day forecast",
            ),
            (
                "🔴 Live football · `/help live`",
                "`/live all [competition]` — Every live match in the competitions I follow\n\
                 `/live competition <competition>` — Live matches in one competition\n\
                 `/live match <match>` — Score, clock and latest events of one match",
            ),
            (
                "⚽ Football · `/help football`",
                "`/football competition list|fixtures|table|scorers` — Competitions, matches, standings, top scorers\n\
                 `/football match details|events|lineup|stats <match>` — Match report\n\
                 `/football team info|fixtures|squad|stats <team>` — Teams\n\
                 `/football player <name>` — Player statistics",
            ),
            ("🎭 Roles · `/help roles`", "`/roles` — Pick self-assignable roles from a menu"),
            (
                "📦 Price tracking · `/help pt`",
                "`/pt add <url>` — Track a product page\n\
                 `/pt list` · `/pt history <item>` · `/pt remove <item>`",
            ),
            ("ℹ️ About · `/help about`", "Data sources, privacy, status and source code"),
        ],
    },
    Topic {
        name: "weather",
        description: "How /weather works",
        title: "🌤 /weather",
        intro: "`/weather <location> [units]` — the weather right now anywhere on Earth, with a map of rain, \
                snow and thunderstorms around it.",
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
        description: "How /forecast works",
        title: "🌦 /forecast",
        intro: "`/forecast <location> [units]` — the next 5 days, starting today.",
        sections: &[
            ("How to use", "Same options as `/weather`: a `location`, and optionally **Imperial** `units`."),
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
        description: "How /live works",
        title: "🔴 /live",
        intro: "Live football scores in the competitions I follow (see `/football competition list`).",
        sections: &[
            (
                "How to use",
                "• `/live all [competition]`: every live match, grouped by competition.\n\
                 • `/live competition <competition>`: one competition.\n\
                 • `/live match <match>`: start typing a team name and pick the match from the list.",
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
                 and points you to `/football match details`.",
            ),
        ],
    },
    Topic {
        name: "football",
        description: "How /football works",
        title: "⚽ /football",
        intro: "Fixtures, results, tables, scorers, match reports, teams, squads and players. Not live: every reply \
                says how old its data is (use `/live` for live scores).",
        sections: &[
            (
                "Competitions",
                "`/football competition list` shows them all: Brasileirão A/B/C, Copa do Brasil, Supercopa, \
                 Copa do Nordeste, Copa Verde, Paulistão, Carioca, Mineiro, Gaúcho, Libertadores, Sudamericana, \
                 the UEFA competitions, and the leagues and cups of England, Spain, Germany, Italy, France and Portugal.\n\
                 • `fixtures <competition> [date]`: matches on a day (`today`, `tomorrow`, `yesterday`, `YYYY-MM-DD`).\n\
                 • `table <competition>`: standings, one table per group where there are groups. Knockout cups have none.\n\
                 • `scorers <competition> [stat]`: top 15 for goals or assists.",
            ),
            (
                "Matches",
                "Pick a match of yesterday, today or tomorrow from the list as you type.\n\
                 • `details`: score, status, half-time score, goals and red cards, formations.\n\
                 • `events`: the whole timeline.\n\
                 • `lineup`: starting XIs and substitutes with numbers and positions (G, D, M, F), © captain; \
                 marked official or expected.\n\
                 • `stats`: team statistics, when the source has them.",
            ),
            (
                "Teams and players",
                "• `team info`: league positions, form (W/D/L, oldest first), last 5 results, next 5 matches.\n\
                 • `team fixtures`: all recent results and upcoming matches.\n\
                 • `team squad`: current registered players by position, with numbers and ages.\n\
                 • `team stats <team> <competition>`: position, points, record, goals, per-game averages, form.\n\
                 • `player <name>`: season statistics in the player's main competition.",
            ),
            (
                "Where the data comes from",
                "• **SportScore**: everything except squads.\n\
                 • **football-data.org**: backup for fixtures, tables and scorers of the main leagues when \
                 SportScore fails (its scores may be delayed; the reply says so).\n\
                 • **API-Football**: squads.\n\
                 Data is refreshed on a schedule (fixtures every 6 hours, tables daily, after each final \
                 whistle) and kept, so replies are fast and the free request limits aren't exceeded.",
            ),
            (
                "How to read a reply",
                "• The end of every reply names the source and says when the data was updated.\n\
                 • ⚠️ means a refresh failed and you're seeing the last saved copy.\n\
                 • \"Not provided by the current data source\" means the source has no such data for it \
                 (often match statistics): nothing is guessed.",
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
        name: "about",
        description: "Data sources, privacy and status",
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
    let chosen = cmd
        .data
        .options()
        .into_iter()
        .find_map(|o| matches!(o.value, ResolvedValue::SubCommand(_)).then_some(o.name));
    chosen.and_then(|n| TOPICS.iter().find(|t| t.name == n)).unwrap_or(&TOPICS[0])
}

pub async fn handle(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let msg = CreateInteractionResponseMessage::new().embed(embed(topic_of(cmd))).ephemeral(true);
    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await?;
    Ok(())
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
    fn every_command_has_a_topic() {
        for name in ["weather", "forecast", "live", "football", "roles", "pt"] {
            assert!(TOPICS.iter().any(|t| t.name == name), "{name}");
        }
        assert_eq!(TOPICS[0].name, "overview");
    }
}
