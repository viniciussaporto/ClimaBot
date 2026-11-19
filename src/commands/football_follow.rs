//! Following teams and competitions (`/football follow …`), server-wide
//! posts set up by moderators (`/football server …`), and the notifier that
//! delivers them.
//!
//! Nothing here calls a provider: the feed is built from the football
//! service's cache, live alerts come from changes the scheduler already
//! noticed, and the daily summary reuses the feed. Every notification is
//! off until a user (for their DMs) or a moderator (for a server channel)
//! switches it on.

use super::football::{
    Args, COLOR, LIVE_COLOR, Reply, clip, comp_label, fixture_line, notice, ordinal, pick_competition, resolve_team,
};
use crate::{
    football::{
        Football, LiveEvent, LiveEventKind,
        competitions::{self, COMPETITIONS, Kind},
        follows::{FollowStore, FollowedTeam, Owner, Subscription},
        model::*,
    },
    metrics,
};
use anyhow::{Result, anyhow};
use chrono::{Duration as ChronoDuration, Timelike, Utc};
use serenity::all::{
    ButtonStyle, ChannelId, ChannelType, CommandInteraction, ComponentInteraction, ComponentInteractionDataKind,
    Context, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption,
    EditInteractionResponse, Http, Permissions, UserId,
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tracing::{info, warn};

/// Who may change a server's football posts: moderators and above.
pub const MANAGERS: Permissions = Permissions::ADMINISTRATOR
    .union(Permissions::MANAGE_GUILD)
    .union(Permissions::MANAGE_CHANNELS)
    .union(Permissions::MODERATE_MEMBERS);

fn store(fb: &Football) -> Result<&FollowStore> {
    fb.follows.get().ok_or_else(|| anyhow!("follow storage isn't available"))
}

fn can_manage(perms: Option<Permissions>) -> bool {
    perms.is_some_and(|p| p.intersects(MANAGERS))
}

const NO_PERMISSION: &str = "🔒 Only moderators can change this server's football posts: members with **Manage Server**, \
     **Manage Channels**, **Moderate Members** or **Administrator**. Use `/football follow` for your own alerts.";

// ─────────────────────────────────────────────
//  Commands
// ─────────────────────────────────────────────

pub(super) async fn handle(ctx: &Context, cmd: &CommandInteraction, fb: &Football, args: &Args) -> Result<()> {
    cmd.defer_ephemeral(&ctx.http).await?;
    let store = store(fb)?;
    let path: Vec<&str> = args.path.iter().map(String::as_str).collect();
    let reply = match path.as_slice() {
        ["follow", "team"] => match resolve_team(fb, args.get("team").unwrap_or_default()).await {
            Ok(t) => {
                let mut sub = store.get(Owner::User(cmd.user.id.get())).await?;
                let text = toggle_team(&mut sub, t.slug.clone(), t.name.clone(), "You");
                save(store, &mut sub, cmd.user.id.get()).await?;
                follow_ack(&text)
            }
            Err(r) => r,
        },
        ["follow", "competition"] => match pick_competition(args) {
            Ok(c) => {
                let mut sub = store.get(Owner::User(cmd.user.id.get())).await?;
                let now = sub.toggle_competition(c.key);
                save(store, &mut sub, cmd.user.id.get()).await?;
                follow_ack(&format!(
                    "{} **{}**.",
                    if now { "⭐ You now follow" } else { "You no longer follow" },
                    c.name
                ))
            }
            Err(r) => r,
        },
        ["follow", "manage"] => panel(&store.get(Owner::User(cmd.user.id.get())).await?, None),
        ["follow", "feed"] => {
            let sub = store.get(Owner::User(cmd.user.id.get())).await?;
            feed_reply(fb, &sub, "⭐ Your football").await
        }
        ["server", rest] => {
            let Some(guild) = cmd.guild_id else {
                return edit(ctx, cmd, notice("This only works in a server.")).await;
            };
            let owner = Owner::Guild(guild.get());
            match *rest {
                "preview" => {
                    let sub = store.get(owner).await?;
                    feed_reply(fb, &sub, "📣 This server's daily football summary").await
                }
                _ if !can_manage(cmd.member.as_ref().and_then(|m| m.permissions)) => notice(NO_PERMISSION),
                "setup" => panel(&store.get(owner).await?, None),
                "team" => match resolve_team(fb, args.get("team").unwrap_or_default()).await {
                    Ok(t) => {
                        let mut sub = store.get(owner).await?;
                        let text = toggle_team(&mut sub, t.slug.clone(), t.name.clone(), "This server");
                        save(store, &mut sub, cmd.user.id.get()).await?;
                        panel(&sub, Some(text))
                    }
                    Err(r) => r,
                },
                _ => notice("Unknown option."),
            }
        }
        _ => notice("Unknown option."),
    };
    edit(ctx, cmd, reply).await
}

async fn edit(ctx: &Context, cmd: &CommandInteraction, r: Reply) -> Result<()> {
    cmd.edit_response(&ctx.http, EditInteractionResponse::new().embed(r.embed).components(r.rows)).await?;
    Ok(())
}

fn toggle_team(sub: &mut Subscription, slug: String, name: String, who: &str) -> String {
    match sub.toggle_team(FollowedTeam { slug, name: name.clone() }) {
        Some(true) => format!("⭐ {who} now follow{} **{name}**.", if who == "You" { "" } else { "s" }),
        Some(false) => format!("{who} no longer follow{} **{name}**.", if who == "You" { "" } else { "s" }),
        None => format!(
            "❌ {who} already follow{} {} teams, the maximum.",
            if who == "You" { "" } else { "s" },
            crate::football::follows::MAX_TEAMS
        ),
    }
}

async fn save(store: &FollowStore, sub: &mut Subscription, by: u64) -> Result<()> {
    sub.updated_by = Some(by as i64);
    sub.updated_at = Some(mongodb::bson::DateTime::now());
    store.save(sub).await
}

fn follow_ack(text: &str) -> Reply {
    Reply {
        embed: CreateEmbed::new()
            .description(format!(
                "{text}\n\nIt shows up in `/football follow feed`. Alerts by DM are **off** unless you switch them on in \
                 **Manage**."
            ))
            .color(COLOR),
        rows: vec![CreateActionRow::Buttons(vec![
            CreateButton::new("fbf:open:manage").label("Manage").style(ButtonStyle::Secondary),
            CreateButton::new("fbf:open:feed").label("My feed").style(ButtonStyle::Primary),
        ])],
    }
}

// ─────────────────────────────────────────────
//  Settings panel (users and servers)
// ─────────────────────────────────────────────

fn onoff(b: bool) -> &'static str {
    if b { "On" } else { "Off" }
}

fn comp_menu(
    prefix: &str,
    id: &str,
    placeholder: &str,
    sub: &Subscription,
    filter: impl Fn(&str) -> bool,
) -> CreateActionRow {
    let options: Vec<CreateSelectMenuOption> = COMPETITIONS
        .iter()
        .filter(|c| filter(c.region))
        .map(|c| {
            CreateSelectMenuOption::new(c.name, c.key)
                .description(c.region)
                .default_selection(sub.competitions.iter().any(|k| k == c.key))
        })
        .collect();
    let n = options.len() as u8;
    CreateActionRow::SelectMenu(
        CreateSelectMenu::new(format!("{prefix}:comps:{id}"), CreateSelectMenuKind::String { options })
            .placeholder(placeholder)
            .min_values(0)
            .max_values(n),
    )
}

fn is_americas(region: &str) -> bool {
    region.contains("Brazil") || region.contains("South America")
}

/// The settings panel. Users get team removal; servers get a channel picker.
fn panel(sub: &Subscription, message: Option<String>) -> Reply {
    let guild = matches!(sub.owner(), Some(Owner::Guild(_)));
    let p = if guild { "fbs" } else { "fbf" };
    let teams = if sub.teams.is_empty() {
        "None".to_string()
    } else {
        sub.teams.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join(", ")
    };
    let comps = if sub.competitions.is_empty() {
        "None".to_string()
    } else {
        sub.competitions.iter().map(|k| comp_label(k, k)).collect::<Vec<_>>().join(", ")
    };
    let channel = sub.channel_id.map(|c| format!("<#{c}>")).unwrap_or_else(|| "not set".into());
    let mut desc = if guild {
        format!(
            "**Server football posts** (off by default). Choose a channel, what the server follows, and what to post.\n\
             Teams: add or remove with `/football server team`.\n\n\
             **Channel:** {channel}\n**Daily summary:** {} at {:02}:00 UTC\n**Live alerts** (kick-off, goals, full time): {}",
            onoff(sub.digest),
            sub.digest_hour,
            onoff(sub.live_alerts)
        )
    } else {
        format!(
            "**What you follow.** Pick competitions below; add teams with `/football follow team` and deselect them \
             here to unfollow.\n\n**Daily summary by DM:** {} at {:02}:00 UTC\n**Live alerts by DM** (kick-off, goals, \
             full time): {}",
            onoff(sub.digest),
            sub.digest_hour,
            onoff(sub.live_alerts)
        )
    };
    if let Some(m) = message {
        desc = format!("{m}\n\n{desc}");
    }
    if guild && (sub.digest || sub.live_alerts) && sub.channel_id.is_none() {
        desc.push_str("\n\n⚠️ Pick a channel: nothing is posted until one is set.");
    }
    if (sub.digest || sub.live_alerts) && sub.is_empty() {
        desc.push_str("\n\n⚠️ Nothing is followed yet, so there is nothing to send.");
    }
    let embed = CreateEmbed::new()
        .title(if guild { "📣 Server football posts" } else { "⭐ Following" })
        .description(desc)
        .field("Teams", clip(&teams, 1024), false)
        .field("Competitions", clip(&comps, 1024), false)
        .color(COLOR)
        .footer(CreateEmbedFooter::new(
            "Changes are saved as you make them. /help follow and /help server explain more.",
        ));

    let mut rows = vec![];
    if guild {
        rows.push(CreateActionRow::SelectMenu(
            CreateSelectMenu::new(
                "fbs:channel",
                CreateSelectMenuKind::Channel {
                    channel_types: Some(vec![ChannelType::Text, ChannelType::News]),
                    default_channels: sub.channel_id.map(|c| vec![ChannelId::new(c as u64)]),
                },
            )
            .placeholder("Channel for football posts"),
        ));
    }
    rows.push(comp_menu(p, "sa", "Competitions: Brazil and South America", sub, is_americas));
    rows.push(comp_menu(p, "eu", "Competitions: Europe", sub, |r| !is_americas(r)));
    if !guild && !sub.teams.is_empty() {
        let options: Vec<CreateSelectMenuOption> = sub
            .teams
            .iter()
            .map(|t| CreateSelectMenuOption::new(clip(&t.name, 100), clip(&t.slug, 100)).default_selection(true))
            .collect();
        let n = options.len() as u8;
        rows.push(CreateActionRow::SelectMenu(
            CreateSelectMenu::new("fbf:teams", CreateSelectMenuKind::String { options })
                .placeholder("Teams you follow (deselect to unfollow)")
                .min_values(0)
                .max_values(n),
        ));
    }
    let hours: Vec<CreateSelectMenuOption> = (0..24u8)
        .map(|h| {
            CreateSelectMenuOption::new(format!("{h:02}:00 UTC"), h.to_string()).default_selection(h == sub.digest_hour)
        })
        .collect();
    // At most 4 menus: [channel or teams], two competition menus, the hour.
    rows.push(CreateActionRow::SelectMenu(
        CreateSelectMenu::new(format!("{p}:hour"), CreateSelectMenuKind::String { options: hours })
            .placeholder("Time of the daily summary"),
    ));
    let toggle = |id: &str, label: &str, on: bool| {
        CreateButton::new(format!("{p}:{id}")).label(format!("{label}: {}", onoff(on))).style(if on {
            ButtonStyle::Success
        } else {
            ButtonStyle::Secondary
        })
    };
    let mut buttons = vec![
        toggle("digest", if guild { "Daily summary" } else { "Daily DM" }, sub.digest),
        toggle("live", if guild { "Live alerts" } else { "Live alerts DM" }, sub.live_alerts),
    ];
    if guild {
        buttons.push(CreateButton::new("fbs:test").label("Send a test post").style(ButtonStyle::Primary));
        buttons.push(CreateButton::new("fbs:off").label("Turn everything off").style(ButtonStyle::Danger));
    } else {
        buttons.push(CreateButton::new("fbf:open:feed").label("My feed").style(ButtonStyle::Primary));
    }
    rows.push(CreateActionRow::Buttons(buttons));
    Reply { embed, rows }
}

// ─────────────────────────────────────────────
//  Components
// ─────────────────────────────────────────────

pub(super) async fn handle_component(ctx: &Context, comp: &ComponentInteraction, fb: &Football) -> Result<()> {
    let store = store(fb)?;
    let id = comp.data.custom_id.as_str();
    let guild_panel = id.starts_with("fbs:");
    let owner = if guild_panel {
        let Some(g) = comp.guild_id else { return Ok(()) };
        if !can_manage(comp.member.as_ref().and_then(|m| m.permissions)) {
            let r = CreateInteractionResponseMessage::new().content(NO_PERMISSION).ephemeral(true);
            comp.create_response(&ctx.http, CreateInteractionResponse::Message(r)).await?;
            return Ok(());
        }
        Owner::Guild(g.get())
    } else {
        Owner::User(comp.user.id.get())
    };
    let mut sub = store.get(owner).await?;
    let values: Vec<String> = match &comp.data.kind {
        ComponentInteractionDataKind::StringSelect { values } => values.clone(),
        ComponentInteractionDataKind::ChannelSelect { values } => values.iter().map(|c| c.get().to_string()).collect(),
        _ => vec![],
    };
    let action = id.split_once(':').map(|(_, a)| a).unwrap_or("");
    let mut message = None;

    // Buttons that open something else, as a new private message.
    if let Some(slug) = action.strip_prefix("toggle:") {
        let t = super::football::team_from_slug(fb, slug).await;
        let text = toggle_team(&mut sub, t.slug, t.name, "You");
        save(store, &mut sub, comp.user.id.get()).await?;
        let r = follow_ack(&text);
        let msg = CreateInteractionResponseMessage::new().embed(r.embed).components(r.rows).ephemeral(true);
        comp.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await?;
        return Ok(());
    }

    let reply = match action {
        "open:feed" => feed_reply(fb, &sub, "⭐ Your football").await,
        "open:manage" => panel(&sub, None),
        a if a.starts_with("comps:") => {
            let region_is_americas = a.ends_with(":sa");
            sub.competitions
                .retain(|k| competitions::by_key(k).is_some_and(|c| is_americas(c.region) != region_is_americas));
            sub.competitions.extend(values.iter().filter(|v| competitions::by_key(v).is_some()).cloned());
            save(store, &mut sub, comp.user.id.get()).await?;
            panel(&sub, Some("✅ Competitions saved.".into()))
        }
        "teams" => {
            sub.teams.retain(|t| values.contains(&t.slug));
            save(store, &mut sub, comp.user.id.get()).await?;
            panel(&sub, Some("✅ Teams saved.".into()))
        }
        "hour" => {
            if let Some(h) = values.first().and_then(|v| v.parse::<u8>().ok()).filter(|h| *h < 24) {
                sub.digest_hour = h;
                // A new time today: allow today's summary at the new hour.
                sub.last_digest = None;
                save(store, &mut sub, comp.user.id.get()).await?;
            }
            panel(&sub, Some(format!("✅ The daily summary is sent at {:02}:00 UTC.", sub.digest_hour)))
        }
        "channel" => {
            sub.channel_id = values.first().and_then(|v| v.parse::<i64>().ok());
            save(store, &mut sub, comp.user.id.get()).await?;
            panel(&sub, Some("✅ Channel saved. Use **Send a test post** to check I can post there.".into()))
        }
        "digest" | "live" => {
            let turning_on = if action == "digest" { !sub.digest } else { !sub.live_alerts };
            // DMs only work when the user accepts them: check before switching on.
            if turning_on
                && !guild_panel
                && let Err(e) = dm(&ctx.http, comp.user.id, welcome()).await
            {
                warn!(error = %e, user = %comp.user.id, "Could not DM user enabling football alerts");
                message = Some(
                    "❌ I couldn't DM you. Allow direct messages from server members (Privacy Settings) and try again."
                        .to_string(),
                );
            } else {
                if action == "digest" {
                    sub.digest = turning_on;
                } else {
                    sub.live_alerts = turning_on;
                }
                save(store, &mut sub, comp.user.id.get()).await?;
            }
            panel(&sub, message.take())
        }
        "test" => {
            let text = match sub.channel_id {
                None => "⚠️ Pick a channel first.".to_string(),
                Some(c) => {
                    let embed = feed_embed(fb, &sub, "📣 Test: this server's football summary").await;
                    match ChannelId::new(c as u64).send_message(&ctx.http, CreateMessage::new().embed(embed)).await {
                        Ok(_) => format!("✅ Test post sent to <#{c}>."),
                        Err(e) => format!(
                            "❌ I can't post in <#{c}> ({e}). Give me **View Channel**, **Send Messages** and **Embed Links** there."
                        ),
                    }
                }
            };
            panel(&sub, Some(text))
        }
        "off" => {
            sub.digest = false;
            sub.live_alerts = false;
            save(store, &mut sub, comp.user.id.get()).await?;
            panel(&sub, Some("✅ All server football posts are off.".into()))
        }
        _ => panel(&sub, None),
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

fn welcome() -> CreateEmbed {
    CreateEmbed::new()
        .description("✅ ClimaBot football alerts are on. Change or stop them any time with `/football follow manage`.")
        .color(COLOR)
}

async fn dm(http: &Http, user: UserId, embed: CreateEmbed) -> Result<()> {
    let channel = user.create_dm_channel(http).await?;
    channel.send_message(http, CreateMessage::new().embed(embed)).await?;
    Ok(())
}

// ─────────────────────────────────────────────
//  Feed (also the daily summary)
// ─────────────────────────────────────────────

async fn feed_reply(fb: &Football, sub: &Subscription, title: &str) -> Reply {
    if sub.is_empty() {
        let guild = matches!(sub.owner(), Some(Owner::Guild(_)));
        return notice(if guild {
            "This server doesn't follow anything yet. A moderator can set it up with `/football server setup`."
        } else {
            "You don't follow anything yet: use `/football follow team` or `/football follow competition`."
        });
    }
    Reply { embed: feed_embed(fb, sub, title).await, rows: vec![] }
}

fn standings_positions(s: &Standings) -> HashMap<String, (u32, String)> {
    let mut out = HashMap::new();
    for g in &s.groups {
        for r in &g.rows {
            out.insert(r.team.clone(), (r.position, g.name.clone()));
        }
    }
    out
}

fn arrow(prev: Option<u32>, now: u32) -> String {
    match prev {
        Some(p) if p > now => format!(" ▲{}", p - now),
        Some(p) if p < now => format!(" ▼{}", now - p),
        Some(_) => " =".into(),
        None => String::new(),
    }
}

/// Teams in the top 6 of a competition's table (for "important" matches).
fn top_teams(fb: &Football, comp: &str) -> Vec<String> {
    fb.cached_now::<Standings>(&format!("standings:{comp}"))
        .map(|s| {
            s.data
                .groups
                .iter()
                .flat_map(|g| g.rows.iter().filter(|r| r.position <= 6).map(|r| r.team.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn section(lines: Vec<String>, max: usize) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let extra = lines.len().saturating_sub(max);
    let mut s = lines.into_iter().take(max).collect::<Vec<_>>().join("\n");
    if extra > 0 {
        s.push_str(&format!("\n…and {extra} more"));
    }
    Some(clip(&s, 1024))
}

/// Everything about what `sub` follows, from the cache only.
pub async fn feed_embed(fb: &Football, sub: &Subscription, title: &str) -> CreateEmbed {
    let now = Utc::now();
    let ts = now.timestamp();
    let today = now.date_naive();
    let day_of = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.date_naive()).unwrap_or(today);
    let with_comp = |f: &Fixture, date: bool| {
        format!("{} · {}", fixture_line(f, date), comp_label(&f.competition, &f.competition_name))
    };
    let concerns = |f: &Fixture| sub.concerns(&f.competition, &f.home, &f.away);

    let live: Vec<Fixture> = fb
        .cached_now::<Vec<Fixture>>("live")
        .filter(|l| l.age() < Duration::from_secs(10 * 60))
        .map(|l| l.data.into_iter().filter(|f| f.status == Status::Live && concerns(f)).collect())
        .unwrap_or_default();
    let recent: Vec<Fixture> = fb.recent_fixtures().into_iter().map(|(f, _)| f).filter(|f| concerns(f)).collect();
    let yesterday: Vec<String> = recent
        .iter()
        .filter(|f| day_of(f.kickoff) == today - ChronoDuration::days(1) && f.status == Status::Finished)
        .map(|f| with_comp(f, false))
        .collect();
    let soon: Vec<&Fixture> = recent
        .iter()
        .filter(|f| {
            f.kickoff >= ts - 3 * 3600
                && day_of(f.kickoff) <= today + ChronoDuration::days(1)
                && f.status != Status::Live
                && f.status != Status::Finished
        })
        .collect();
    let soon_ids: Vec<&str> = soon.iter().map(|f| f.id.as_str()).collect();

    // Important: involving a followed team, a cup match, or a top-6 clash.
    let mut tops: HashMap<String, Vec<String>> = HashMap::new();
    let important: Vec<String> = fb
        .upcoming_fixtures(7)
        .into_iter()
        .filter(|f| f.kickoff > ts && f.status == Status::Upcoming && concerns(f) && !soon_ids.contains(&f.id.as_str()))
        .filter(|f| {
            let cup = competitions::by_key(&f.competition).is_some_and(|c| c.kind == Kind::Cup);
            let top = tops.entry(f.competition.clone()).or_insert_with(|| top_teams(fb, &f.competition));
            sub.follows_team(&f.home, &f.away) || cup || (top.contains(&f.home) && top.contains(&f.away))
        })
        .map(|f| format!("⭐ {}", with_comp(&f, true)))
        .collect();

    // Next match of each followed team, from their cached schedules.
    let mut next = vec![];
    for t in &sub.teams {
        let line = match fb.cached::<TeamSchedule>(&format!("team:{}", t.slug)).await {
            Some(s) => match s.data.matches.iter().find(|m| m.kickoff > ts && !m.status.is_terminal()) {
                Some(m) => format!("**{}** · {}", t.name.split(" - ").next().unwrap_or(&t.name), with_comp(m, true)),
                None => format!("**{}** · no match scheduled in the data source", t.name),
            },
            None => format!("**{}** · schedule not loaded yet (refreshed daily)", t.name),
        };
        next.push(line);
    }

    // Table changes since yesterday's snapshot.
    let mut table = vec![];
    for c in COMPETITIONS {
        let Some(cur) = fb.cached::<Standings>(&format!("standings:{}", c.key)).await else { continue };
        let prev =
            fb.cached::<Standings>(&format!("standings_prev:{}", c.key)).await.map(|p| standings_positions(&p.data));
        let pos = standings_positions(&cur.data);
        let followed_comp = sub.competitions.iter().any(|k| k == c.key);
        for t in &sub.teams {
            if let Some((p, group)) = pos.get(&t.name) {
                let was = prev.as_ref().and_then(|m| m.get(&t.name)).map(|x| x.0);
                let grp = if group.is_empty() { String::new() } else { format!(" ({group})") };
                table.push(format!("**{}** · {} in {}{grp}{}", t.name, ordinal(*p), c.name, arrow(was, *p)));
            }
        }
        if followed_comp
            && c.kind == Kind::League
            && let Some(g) = cur.data.groups.first()
        {
            {
                let leader =
                    g.rows.first().map(|r| format!("{} leads with {} pts", r.team, r.points)).unwrap_or_default();
                let mut movers: Vec<(i64, String)> = g
                    .rows
                    .iter()
                    .filter_map(|r| {
                        let was = prev.as_ref()?.get(&r.team)?.0 as i64;
                        let d = was - r.position as i64;
                        (d.abs() >= 2).then(|| (d, format!("{}{}", r.team, arrow(Some(was as u32), r.position))))
                    })
                    .collect();
                movers.sort_by_key(|(d, _)| std::cmp::Reverse(d.abs()));
                let moves = movers.into_iter().take(3).map(|(_, s)| s).collect::<Vec<_>>().join(", ");
                table.push(format!(
                    "**{}:** {leader}{}",
                    c.name,
                    if moves.is_empty() { String::new() } else { format!(" · moved: {moves}") }
                ));
            }
        }
    }

    let mut e = CreateEmbed::new().title(title).color(if live.is_empty() { COLOR } else { LIVE_COLOR });
    let mut any = false;
    let mut add = |e: CreateEmbed, name: &str, body: Option<String>| -> CreateEmbed {
        match body {
            Some(b) => {
                any = true;
                e.field(name, b, false)
            }
            None => e,
        }
    };
    e = add(e, "🔴 Live now", section(live.iter().map(|f| with_comp(f, false)).collect(), 10));
    e = add(e, "📅 Today and tomorrow", section(soon.iter().map(|f| with_comp(f, true)).collect(), 12));
    e = add(e, "✅ Yesterday's results", section(yesterday, 12));
    e = add(e, "⭐ Important matches this week", section(important, 8));
    e = add(e, "⏭ Next match", section(next, 10));
    e = add(e, "📊 Tables", section(table, 10));
    let mut desc = format!(
        "What you follow, as of <t:{ts}:t>. Not live: tables change once a day, scores every minute during matches."
    );
    if !any {
        desc.push_str("\n\nNothing to report right now.");
    }
    desc.push_str(&format!("\n\n{}", Provider::SportScore.attribution()));
    e.description(desc)
}

// ─────────────────────────────────────────────
//  Notifier
// ─────────────────────────────────────────────

fn event_line(e: &LiveEvent) -> String {
    let f = &e.fixture;
    let score = f
        .score()
        .map(|(h, a)| format!("{} {h}–{a} {}", f.home, f.away))
        .unwrap_or_else(|| format!("{} vs {}", f.home, f.away));
    let comp = comp_label(&f.competition, &f.competition_name);
    match e.kind {
        LiveEventKind::Kickoff => format!("▶️ **Kick-off** · {} vs {} · {comp}", f.home, f.away),
        LiveEventKind::Goal => format!("⚽ **Goal** · {score} · {} · {comp}", f.status_text),
        LiveEventKind::FullTime => format!("🏁 **Full time** · {score} · {comp}"),
    }
}

async fn deliver(http: &Http, sub: &Subscription, embed: CreateEmbed) -> Result<()> {
    match sub.owner() {
        Some(Owner::User(id)) => dm(http, UserId::new(id), embed).await,
        Some(Owner::Guild(_)) => {
            let channel = sub.channel_id.ok_or_else(|| anyhow!("no channel set"))?;
            ChannelId::new(channel as u64).send_message(http, CreateMessage::new().embed(embed)).await?;
            Ok(())
        }
        None => Err(anyhow!("bad subscription key {}", sub.key)),
    }
}

fn settings_hint(sub: &Subscription) -> &'static str {
    match sub.owner() {
        Some(Owner::Guild(_)) => "Moderators: /football server setup",
        _ => "Change or stop: /football follow manage",
    }
}

async fn send_live(http: &Http, fb: &Football, events: &[LiveEvent]) {
    let Ok(store) = store(fb) else { return };
    let subs = match store.notifying().await {
        Ok(s) => s,
        Err(e) => return warn!(error = format!("{e:#}"), "Could not load football subscriptions"),
    };
    for sub in subs.iter().filter(|s| s.live_alerts) {
        let lines: Vec<String> = events
            .iter()
            .filter(|e| sub.concerns(&e.fixture.competition, &e.fixture.home, &e.fixture.away))
            .map(event_line)
            .collect();
        if lines.is_empty() {
            continue;
        }
        let embed = CreateEmbed::new()
            .title("🔔 Football")
            .description(format!("{}\n\n{}", clip(&lines.join("\n"), 3800), Provider::SportScore.attribution()))
            .color(LIVE_COLOR)
            .footer(CreateEmbedFooter::new(settings_hint(sub)));
        let status = match deliver(http, sub, embed).await {
            Ok(()) => "success",
            Err(e) => {
                warn!(error = format!("{e:#}"), subscription = sub.key, "Football live alert not delivered");
                "error"
            }
        };
        metrics::FOOTBALL_NOTIFICATIONS.with_label_values(&["live", status]).inc();
    }
}

async fn send_digests(http: &Http, fb: &Football) {
    let Ok(store) = store(fb) else { return };
    let now = Utc::now();
    let day = now.format("%Y-%m-%d").to_string();
    let subs = match store.notifying().await {
        Ok(s) => s,
        Err(e) => return warn!(error = format!("{e:#}"), "Could not load football subscriptions"),
    };
    for sub in subs {
        let due = sub.digest
            && now.hour() as u8 == sub.digest_hour
            && sub.last_digest.as_deref() != Some(day.as_str())
            && !sub.is_empty();
        if !due {
            continue;
        }
        let title = match sub.owner() {
            Some(Owner::Guild(_)) => "📣 Today's football",
            _ => "⭐ Your football today",
        };
        let embed = feed_embed(fb, &sub, title).await.footer(CreateEmbedFooter::new(settings_hint(&sub)));
        // Mark first: a failing channel shouldn't be retried every minute.
        if let Err(e) = store.mark_digest_sent(&sub.key, &day).await {
            warn!(error = format!("{e:#}"), "Could not mark football summary as sent");
            continue;
        }
        let status = match deliver(http, &sub, embed).await {
            Ok(()) => "success",
            Err(e) => {
                warn!(error = format!("{e:#}"), subscription = sub.key, "Football daily summary not delivered");
                "error"
            }
        };
        metrics::FOOTBALL_NOTIFICATIONS.with_label_values(&["digest", status]).inc();
    }
}

/// Deliver live alerts as the scheduler notices changes, and the daily
/// summaries at their hour. Runs for the bot's lifetime.
pub async fn run_notifier(http: Arc<Http>, fb: Arc<Football>) {
    let Some(mut events) = fb.take_events() else { return };
    info!("Football notifier started");
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            Some(first) = events.recv() => {
                // Gather what arrived with it (one poll's worth) into one message.
                tokio::time::sleep(Duration::from_secs(2)).await;
                let mut batch = vec![first];
                while let Ok(e) = events.try_recv() {
                    batch.push(e);
                }
                send_live(&http, &fb, &batch).await;
            }
            _ = tick.tick() => send_digests(&http, &fb).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moderators_only() {
        assert!(can_manage(Some(Permissions::MANAGE_GUILD)));
        assert!(can_manage(Some(Permissions::MODERATE_MEMBERS | Permissions::SEND_MESSAGES)));
        assert!(can_manage(Some(Permissions::ADMINISTRATOR)));
        assert!(!can_manage(Some(Permissions::SEND_MESSAGES | Permissions::MANAGE_ROLES)));
        assert!(!can_manage(None));
    }

    fn check(r: &Reply) {
        assert!(r.rows.len() <= 5, "at most 5 component rows");
        let v = serde_json::to_value(&r.embed).unwrap();
        for f in v["fields"].as_array().cloned().unwrap_or_default() {
            assert!(f["value"].as_str().unwrap().chars().count() <= 1024);
        }
        for row in &r.rows {
            let row = serde_json::to_value(row).unwrap();
            for c in row["components"].as_array().unwrap() {
                assert!(c["custom_id"].as_str().unwrap().len() <= 100);
                if let Some(o) = c["options"].as_array() {
                    assert!(o.len() <= 25);
                }
            }
        }
    }

    #[test]
    fn panels_fit_discord_limits() {
        let mut user = Subscription::new(Owner::User(1));
        check(&panel(&user, None));
        for i in 0..25 {
            user.toggle_team(FollowedTeam { slug: format!("team-{i}"), name: format!("Team {i}") });
        }
        user.competitions = COMPETITIONS.iter().map(|c| c.key.to_string()).collect();
        check(&panel(&user, Some("x".into())));
        let mut guild = Subscription::new(Owner::Guild(2));
        guild.digest = true;
        check(&panel(&guild, None));
        assert!(serde_json::to_string(&panel(&guild, None).embed).unwrap().contains("Pick a channel"));
    }

    #[test]
    fn alert_lines() {
        let f = Fixture {
            id: "1".into(),
            slug: "a-vs-b".into(),
            competition: "brazilian-serie-a".into(),
            competition_name: "Brazilian Serie A".into(),
            home: "Flamengo - RJ".into(),
            away: "Palmeiras - SP".into(),
            home_score: Some(1),
            away_score: Some(0),
            status: Status::Live,
            status_text: "2nd half".into(),
            kickoff: 0,
        };
        assert!(
            event_line(&LiveEvent { kind: LiveEventKind::Goal, fixture: f.clone() })
                .contains("Flamengo - RJ 1–0 Palmeiras - SP")
        );
        assert!(event_line(&LiveEvent { kind: LiveEventKind::Kickoff, fixture: f }).starts_with("▶️"));
        assert_eq!(arrow(Some(3), 1), " ▲2");
        assert_eq!(arrow(Some(1), 4), " ▼3");
        assert_eq!(arrow(Some(2), 2), " =");
    }
}
