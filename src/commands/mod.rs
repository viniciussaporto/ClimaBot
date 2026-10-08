pub mod pricetracking;
pub mod roles;
pub mod weather;

use crate::{
    metrics,
    pricetracker::{STOP_BUTTON_PREFIX, store::Store},
};
use anyhow::{Result, anyhow};
use serenity::all::{
    CommandInteraction, CommandOptionType, ComponentInteraction, Context, CreateCommand,
    CreateCommandOption, CreateEmbed, CreateInteractionResponse, CreateInteractionResponseMessage,
    EditInteractionResponse, InteractionContext, UserId,
};
use std::{sync::OnceLock, time::Instant};
use tracing::{debug, error, info};

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

    vec![
        CreateCommand::new("weather")
            .description("Get the weather information for a location")
            .add_option(location("weather information"))
            .add_option(units()),
        CreateCommand::new("forecast")
            .description("Get a 5-day weather forecast for a location")
            .add_option(location("weather forecast"))
            .add_option(units()),
        CreateCommand::new("roles")
            .description("Manage self-assignable roles in this server")
            .contexts(vec![InteractionContext::Guild]),
        pricetracking::register(),
        CreateCommand::new("help").description("Show what ClimaBot can do"),
    ]
}

// ─────────────────────────────────────────────
//  Dispatch
// ─────────────────────────────────────────────

fn string_option<'a>(cmd: &'a CommandInteraction, name: &str) -> Option<&'a str> {
    cmd.data
        .options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| o.value.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub async fn handle_command(ctx: &Context, cmd: &CommandInteraction, store: &Store) {
    let name = cmd.data.name.as_str();
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
        "help" => help_command(ctx, cmd).await,
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

pub async fn handle_component(ctx: &Context, comp: &ComponentInteraction, store: &Store) {
    let id = comp.data.custom_id.as_str();

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
//  /weather, /forecast
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
    let units = weather::Units::from_option(string_option(cmd, "units"));
    let (embed, attachment) = weather::weather_embed(&coords, &current, units);

    cmd.edit_response(&ctx.http, EditInteractionResponse::new().embed(embed).new_attachment(attachment))
        .await?;
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

// ─────────────────────────────────────────────
//  /help
// ─────────────────────────────────────────────

async fn help_command(ctx: &Context, cmd: &CommandInteraction) -> Result<()> {
    let embed = CreateEmbed::new()
        .title("🤖 ClimaBot — Commands")
        .field(
            "🌤 Weather",
            "`/weather <location> [units]` — Current weather conditions\n\
             `/forecast <location> [units]` — 5-day forecast\n\
             Pick **Imperial** under `units` for °F, mph and inHg",
            false,
        )
        .field("🎭 Roles", "`/roles` — Pick self-assignable roles from a menu", false)
        .field(
            "📦 Price Tracking",
            "`/pt add <url>` — Track a product page\n\
             `/pt list` — Your tracked products\n\
             `/pt history <item>` — Price history\n\
             `/pt remove <item>` — Stop tracking\n\
             Each product is re-checked about every hour, at a slightly random time; \
             I'll DM you when its price changes. Any currency is supported.",
            false,
        )
        .color(0x0099ff);

    let msg = CreateInteractionResponseMessage::new().embed(embed).ephemeral(true);
    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await?;
    Ok(())
}
