pub mod football;
pub mod football_follow;
pub mod football_player;
pub mod help;
pub mod pricetracking;
pub mod radar;
pub mod roles;
pub mod weather;

use crate::{
    metrics,
    pricetracker::{STOP_BUTTON_PREFIX, store::Store},
};
use anyhow::{Result, anyhow};
use serenity::all::{
    CommandInteraction, CommandOptionType, ComponentInteraction, Context, CreateCommand,
    CreateCommandOption, CreateInteractionResponse, CreateInteractionResponseMessage,
    EditInteractionResponse, InteractionContext, UserId,
};
use std::{sync::OnceLock, time::Instant};
use tracing::{debug, error, info, warn};

static BOT_USER_ID: OnceLock<UserId> = OnceLock::new();

pub fn set_bot_user_id(id: UserId) {
    let _ = BOT_USER_ID.set(id);
}

pub async fn bot_user_id(ctx: &Context) -> Result<UserId> {
    if let Some(id) = BOT_USER_ID.get() {
        return Ok(*id);
    }
    let id = ctx.http.get_current_user().await?.id;
    set_bot_user_id(id);
    Ok(id)
}

// ─────────────────────────────────────────────
//  Slash command definitions
// ─────────────────────────────────────────────

pub fn definitions() -> Vec<CreateCommand> {
    let location = |what: &str| {
        CreateCommandOption::new(
            CommandOptionType::String,
            "location",
            format!("The location to get the {what} for"),
        )
        .required(true)
    };
    let units = || {
        CreateCommandOption::new(CommandOptionType::String, "units", "Units to show the results in (default: metric)")
            .add_string_choice("Metric (°C, km/h, hPa)", "metric")
            .add_string_choice("Imperial (°F, mph, inHg)", "imperial")
    };

    let weather_sub = |name: &str, desc: &str, what: &str| {
        CreateCommandOption::new(CommandOptionType::SubCommand, name, desc)
            .add_sub_option(location(what))
            .add_sub_option(units())
    };

    vec![
        CreateCommand::new("weather")
            .description("Weather: conditions now with a rain map, or a 5-day forecast")
            .add_option(weather_sub("now", "Current conditions and a precipitation map", "weather information"))
            .add_option(weather_sub("forecast", "5-day forecast: highs, lows, chance of rain", "weather forecast")),
        football::register(),
        CreateCommand::new("roles")
            .description("Manage self-assignable roles in this server")
            .contexts(vec![InteractionContext::Guild]),
        pricetracking::register(),
        help::register(),
    ]
}

// ─────────────────────────────────────────────
//  Dispatch
// ─────────────────────────────────────────────

/// A string option, at the top level or inside the chosen subcommand.
fn string_option<'a>(cmd: &'a CommandInteraction, name: &str) -> Option<&'a str> {
    use serenity::all::CommandDataOptionValue;
    let top = &cmd.data.options;
    let nested = top.iter().find_map(|o| match &o.value {
        CommandDataOptionValue::SubCommand(opts) => Some(opts.as_slice()),
        _ => None,
    });
    nested
        .unwrap_or(top.as_slice())
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| o.value.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The first subcommand's name (`/weather now` → "now").
fn subcommand(cmd: &CommandInteraction) -> Option<&str> {
    cmd.data.options.first().map(|o| o.name.as_str())
}

pub async fn handle_command(ctx: &Context, cmd: &CommandInteraction, store: &Store) {
    let name = match (cmd.data.name.as_str(), subcommand(cmd)) {
        ("weather", Some("forecast")) => "forecast",
        (n, _) => n,
    };
    let label = match name {
        "pt" => "pricetracking",
        other => other,
    };
    debug!(command = name, user = %cmd.user.name, "Incoming command");
    metrics::COMMAND_COUNTER.with_label_values(&[label, "received"]).inc();
    let start = Instant::now();

    let result = match name {
        "weather" => weather_command(ctx, cmd).await,
        "forecast" => forecast_command(ctx, cmd).await,
        "roles" => roles::handle_command(ctx, cmd).await,
        "pt" => pricetracking::handle(ctx, cmd, store).await,
        "help" => help::handle(ctx, cmd).await,
        "football" => football::handle_football(ctx, cmd).await,
        _ => Err(anyhow!("unknown command")),
    };

    let status = match &result {
        Ok(()) => "success",
        Err(e) => {
            error!(error = format!("{e:#}"), command = name, user = %cmd.user.id, "Command failed");
            let message = match name {
                "weather" => "❌ Unable to retrieve weather information.",
                "forecast" => "❌ Unable to retrieve forecast information.",
                "roles" => "❌ Failed to process roles command. Check that I have the **Manage Roles** permission.",
                "pt" => "⚠️ Something went wrong with price tracking.",
                "football" => "⚠️ Something went wrong fetching football data.",
                _ => "❌ Something went wrong.",
            };
            reply_error(ctx, cmd, message).await;
            "error"
        }
    };

    metrics::COMMAND_COUNTER.with_label_values(&[label, status]).inc();
    metrics::RESPONSE_TIME
        .with_label_values(&[label, status])
        .observe(start.elapsed().as_secs_f64());
}

/// Tell the user something failed, whether or not the interaction was
/// already acknowledged.
async fn reply_error(ctx: &Context, cmd: &CommandInteraction, message: &str) {
    let fresh = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content(message).ephemeral(true),
    );
    if cmd.create_response(&ctx.http, fresh).await.is_err() {
        let edit = EditInteractionResponse::new()
            .content(message)
            .embeds(vec![])
            .components(vec![]);
        if let Err(e) = cmd.edit_response(&ctx.http, edit).await {
            error!(error = %e, "Could not deliver error message to user");
        }
    }
}

/// Suggestions while a user types a `/football` option. Read
/// from the cache only, so it answers within Discord's 3-second limit.
pub async fn handle_autocomplete(ctx: &Context, cmd: &CommandInteraction) {
    if let Err(e) = football::handle_autocomplete(ctx, cmd).await {
        debug!(error = %e, command = %cmd.data.name, "Autocomplete reply failed");
    }
}

pub async fn handle_component(ctx: &Context, comp: &ComponentInteraction, store: &Store) {
    let id = comp.data.custom_id.as_str();

    if id.starts_with("fb") {
        let status = match football::handle_component(ctx, comp).await {
            Ok(()) => "success",
            Err(e) => {
                error!(error = format!("{e:#}"), custom_id = id, "Football page button failed");
                let reply = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("⚠️ Couldn't load that page. Run the command again.")
                        .ephemeral(true),
                );
                let _ = comp.create_response(&ctx.http, reply).await;
                "error"
            }
        };
        metrics::COMMAND_COUNTER.with_label_values(&["football_page", status]).inc();
        return;
    }

    if id.starts_with(STOP_BUTTON_PREFIX) {
        let status = match pricetracking::handle_stop_button(ctx, comp, store).await {
            Ok(()) => "success",
            Err(e) => {
                error!(error = format!("{e:#}"), custom_id = id, user = %comp.user.id, "Stop-tracking button failed");
                let reply = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("⚠️ Couldn't stop tracking that product. Try `/pt remove` instead."),
                );
                let _ = comp.create_response(&ctx.http, reply).await;
                "error"
            }
        };
        metrics::COMMAND_COUNTER.with_label_values(&["pt_stop_button", status]).inc();
        return;
    }

    let result = if id.starts_with(roles::SELECT_ID) {
        roles::handle_select(ctx, comp).await
    } else if id.starts_with(roles::PAGE_PREFIX) {
        roles::handle_pagination(ctx, comp).await
    } else {
        debug!(custom_id = id, "Ignoring unknown component");
        return;
    };

    if let Err(e) = result {
        metrics::ROLE_ASSIGNMENT_COUNTER.with_label_values(&["error", "unknown"]).inc();
        error!(error = format!("{e:#}"), custom_id = id, user = %comp.user.id, "Component handler failed");
        let reply = CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("❌ Failed to update roles. Please check bot permissions!")
                .ephemeral(true),
        );
        let _ = comp.create_response(&ctx.http, reply).await;
    }
}

// ─────────────────────────────────────────────
//  /weather now, /weather forecast
// ─────────────────────────────────────────────

/// Reply for a location OpenCage could not resolve. Not a bot failure, so the
/// command still counts as successful.
fn not_found_reply(location: &str) -> EditInteractionResponse {
    EditInteractionResponse::new().content(format!(
        "❌ I couldn't find a location matching **{location}**. Try adding a state or country."
    ))
}

async fn weather_command(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let Some(location) = string_option(cmd, "location") else {
        let msg = CreateInteractionResponseMessage::new().content("Please provide a location.").ephemeral(true);
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await?;
        return Ok(());
    };
    cmd.defer(&ctx.http).await?;

    let coords = match weather::get_coordinates(location).await {
        Ok(c) => c,
        Err(e) if e.is::<weather::LocationNotFound>() => {
            cmd.edit_response(&ctx.http, not_found_reply(location)).await?;
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let current = weather::get_current(&coords).await?;

    // The precipitation map is optional: without it the embed is sent as before.
    // Its grid of forecasts (~108 Open-Meteo calls) is only fetched when rain
    // is likely here; otherwise the map is drawn without precipitation.
    let precipitation_expected = radar::rain_expected(&current);
    let map = async {
        let data = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            radar::fetch(&coords, precipitation_expected),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow!("timed out")))?;
        let offset = current.utc_offset_seconds;
        let gif = tokio::task::spawn_blocking(move || radar::render_gif(&data, offset)).await??;
        Ok::<_, anyhow::Error>(radar::Map { gif, precipitation_expected })
    }
    .await
    .inspect_err(|e| warn!(error = format!("{e:#}"), location, "Precipitation map unavailable"))
    .ok();
    let units = weather::Units::from_option(string_option(cmd, "units"));
    let (embed, attachments) = weather::weather_embed(&coords, &current, units, map);

    let mut reply = EditInteractionResponse::new().embed(embed);
    for a in attachments {
        reply = reply.new_attachment(a);
    }
    cmd.edit_response(&ctx.http, reply).await?;
    info!(location, resolved = %coords.formatted, "Weather delivered");
    Ok(())
}

async fn forecast_command(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let Some(location) = string_option(cmd, "location") else {
        let msg = CreateInteractionResponseMessage::new().content("Please provide a location.").ephemeral(true);
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await?;
        return Ok(());
    };
    cmd.defer(&ctx.http).await?;

    let coords = match weather::get_coordinates(location).await {
        Ok(c) => c,
        Err(e) if e.is::<weather::LocationNotFound>() => {
            cmd.edit_response(&ctx.http, not_found_reply(location)).await?;
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let daily = weather::get_forecast(&coords).await?;
    let units = weather::Units::from_option(string_option(cmd, "units"));
    let embed = weather::forecast_embed(&coords, &daily, units);

    cmd.edit_response(&ctx.http, EditInteractionResponse::new().embed(embed)).await?;
    info!(location, resolved = %coords.formatted, "Forecast delivered");
    Ok(())
}
