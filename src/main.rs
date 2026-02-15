mod commands;
mod logger;
mod metrics;
mod revolt;

use anyhow::Result;
use dotenv::dotenv;
use futures_util::{SinkExt, StreamExt};
use mongodb::Client as MongoClient;
use revolt::{RevoltClient, RevoltEvent};
use serde_json::json;
use std::{collections::HashMap, env, sync::Arc, time::Instant};
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

// Revolt WebSocket gateway URL
const REVOLT_WS: &str = "wss://ws.revolt.chat";

/// Map of channel_id → server_id, built from the `Ready` event.
type ChannelCache = Arc<RwLock<HashMap<String, String>>>;

// ─────────────────────────────────────────────────────────────
//  Entry point
// ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    logger::init();

    let token = env::var("REVOLT_TOKEN")
        .expect("REVOLT_TOKEN must be set in .env");

    let metrics_port: u16 = env::var("METRICS_PORT")
        .unwrap_or_else(|_| "9464".into())
        .parse()
        .unwrap_or(9464);

    // ── Metrics server ────────────────────────────────────────
    metrics::start_metrics_server(metrics_port);

    // ── MongoDB ───────────────────────────────────────────────
    let mongo_uri = env::var("MONGODB_URI")
        .unwrap_or_else(|_| "mongodb://localhost:27017".into());
    let mongo = MongoClient::with_uri_str(&mongo_uri).await?;
    let db = mongo.database("pricetracker");
    info!("Connected to MongoDB at {mongo_uri}");

    // ── Hourly price-tracking cron ────────────────────────────
    let sched = JobScheduler::new().await?;
    let db_for_cron = db.clone();
    sched
        .add(Job::new_async(
            // tokio-cron-scheduler uses 6-field cron: sec min hour day month weekday
            "0 0 * * * *",
            move |_uuid, _lock| {
                let db = db_for_cron.clone();
                Box::pin(async move {
                    info!("Starting hourly price-check cron job");
                    commands::pricetracking::run_price_check_cron(db).await;
                })
            },
        )?)
        .await?;
    sched.start().await?;

    // ── Bot / gateway loop ────────────────────────────────────
    let client = Arc::new(RevoltClient::new(token.clone()));
    let channel_cache: ChannelCache = Arc::new(RwLock::new(HashMap::new()));

    loop {
        match gateway_loop(&token, client.clone(), db.clone(), channel_cache.clone()).await {
            Ok(_) => warn!("Gateway connection closed — reconnecting in 5 s…"),
            Err(e) => error!(error = %e, "Gateway error — reconnecting in 5 s…"),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

// ─────────────────────────────────────────────────────────────
//  WebSocket gateway
// ─────────────────────────────────────────────────────────────

async fn gateway_loop(
    token: &str,
    client: Arc<RevoltClient>,
    db: mongodb::Database,
    channel_cache: ChannelCache,
) -> Result<()> {
    info!("Connecting to Revolt WebSocket gateway…");
    let (ws, _) = connect_async(REVOLT_WS).await?;
    let (mut write, mut read) = ws.split();

    // Authenticate with the Revolt gateway
    write
        .send(Message::Text(
            json!({ "type": "Authenticate", "token": token }).to_string(),
        ))
        .await?;

    while let Some(result) = read.next().await {
        let msg = result?;

        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(p) => {
                // Respond to WebSocket pings to keep the connection alive
                write.send(Message::Pong(p)).await.ok();
                continue;
            }
            _ => continue,
        };

        let event: RevoltEvent = match serde_json::from_str(&text) {
            Ok(e) => e,
            Err(e) => {
                debug!(error = %e, raw = %text, "Unrecognised gateway event");
                continue;
            }
        };

        match event {
            RevoltEvent::Authenticated => {
                info!("✅ Authenticated with Revolt gateway");
            }

            RevoltEvent::Ready { channels, .. } => {
                info!("📡 Ready — caching {} channels", channels.len());
                let mut cache = channel_cache.write().await;
                for ch in channels {
                    if let Some(srv) = ch.server {
                        cache.insert(ch.id, srv);
                    }
                }
            }

            RevoltEvent::Message(message) => {
                // Handle each message in its own task so the gateway loop
                // never blocks on slow network I/O or DB calls.
                let client = client.clone();
                let db = db.clone();
                let cache = channel_cache.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_message(message, client, db, cache).await {
                        error!(error = %e, "Error in message handler");
                    }
                });
            }

            RevoltEvent::Pong { .. } | RevoltEvent::Unknown => {}
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
//  Message / command handler
// ─────────────────────────────────────────────────────────────

async fn handle_message(
    message: revolt::Message,
    client: Arc<RevoltClient>,
    db: mongodb::Database,
    channel_cache: ChannelCache,
) -> Result<()> {
    let content = match &message.content {
        Some(c) if !c.trim().is_empty() => c.clone(),
        _ => return Ok(()),
    };

    // All commands are prefixed with `!`
    if !content.starts_with('!') {
        return Ok(());
    }

    // Parse `!command arg1 arg2 …`
    let without_prefix = content.trim_start_matches('!');
    let mut tokens = without_prefix.splitn(2, char::is_whitespace);
    let command = tokens.next().unwrap_or("").to_lowercase();
    let rest = tokens.next().unwrap_or("").trim();
    let args: Vec<&str> = rest.split_whitespace().collect();

    let channel_id = &message.channel;
    let user_id = &message.author;

    debug!(command, user = %user_id, "Incoming command");

    let start = Instant::now();
    let mut status = "success";

    match command.as_str() {
        // ── !weather ──────────────────────────────────────────
        "weather" => {
            metrics::COMMAND_COUNTER
                .with_label_values(&["weather", "received"])
                .inc();

            if rest.is_empty() {
                client
                    .send_message(channel_id, "Usage: `!weather <location>`")
                    .await?;
                return Ok(());
            }

            match commands::weather::get_weather(rest).await {
                Ok(reply) => {
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["weather", "success"])
                        .inc();
                    metrics::WEATHER_API_COUNTER
                        .with_label_values(&["current", "success"])
                        .inc();
                    client.send_message(channel_id, &reply).await?;
                    info!(location = rest, "Weather delivered");
                }
                Err(e) => {
                    error!(error = %e, location = rest, "Weather command failed");
                    status = "error";
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["weather", "error"])
                        .inc();
                    metrics::WEATHER_API_COUNTER
                        .with_label_values(&["current", "error"])
                        .inc();
                    client
                        .send_message(channel_id, "❌ Unable to retrieve weather information.")
                        .await?;
                }
            }
        }

        // ── !forecast ─────────────────────────────────────────
        "forecast" => {
            metrics::COMMAND_COUNTER
                .with_label_values(&["forecast", "received"])
                .inc();

            if rest.is_empty() {
                client
                    .send_message(channel_id, "Usage: `!forecast <location>`")
                    .await?;
                return Ok(());
            }

            match commands::weather::get_forecast(rest).await {
                Ok(reply) => {
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["forecast", "success"])
                        .inc();
                    metrics::WEATHER_API_COUNTER
                        .with_label_values(&["forecast", "success"])
                        .inc();
                    client.send_message(channel_id, &reply).await?;
                    info!(location = rest, "Forecast delivered");
                }
                Err(e) => {
                    error!(error = %e, location = rest, "Forecast command failed");
                    status = "error";
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["forecast", "error"])
                        .inc();
                    metrics::WEATHER_API_COUNTER
                        .with_label_values(&["forecast", "error"])
                        .inc();
                    client
                        .send_message(channel_id, "❌ Unable to retrieve forecast information.")
                        .await?;
                }
            }
        }

        // ── !roles ────────────────────────────────────────────
        "roles" => {
            metrics::COMMAND_COUNTER
                .with_label_values(&["roles", "received"])
                .inc();

            let cache = channel_cache.read().await;
            let server_id = cache.get(channel_id).cloned();
            drop(cache);

            let Some(server_id) = server_id else {
                metrics::COMMAND_COUNTER
                    .with_label_values(&["roles", "error"])
                    .inc();
                client
                    .send_message(channel_id, "❌ This command only works inside a server.")
                    .await?;
                return Ok(());
            };

            match commands::roles::handle_roles(
                &client, channel_id, &server_id, user_id, &args,
            )
            .await
            {
                Ok(_) => {
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["roles", "success"])
                        .inc();
                }
                Err(e) => {
                    error!(error = %e, user = %user_id, "Roles command failed");
                    status = "error";
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["roles", "error"])
                        .inc();
                    client
                        .send_message(
                            channel_id,
                            "❌ Failed to manage roles — check that the bot has the required permissions.",
                        )
                        .await?;
                }
            }
        }

        // ── !pt / !pricetracking / !price ─────────────────────
        "pt" | "pricetracking" | "price" => {
            metrics::COMMAND_COUNTER
                .with_label_values(&["pricetracking", "received"])
                .inc();

            match commands::pricetracking::handle_pricetracking(
                &db, channel_id, user_id, &args, &client,
            )
            .await
            {
                Ok(_) => {
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["pricetracking", "success"])
                        .inc();
                }
                Err(e) => {
                    error!(error = %e, user = %user_id, "Price tracking command failed");
                    status = "error";
                    metrics::COMMAND_COUNTER
                        .with_label_values(&["pricetracking", "error"])
                        .inc();
                    client
                        .send_message(channel_id, "⚠️ Something went wrong with price tracking.")
                        .await?;
                }
            }
        }

        // ── !help ─────────────────────────────────────────────
        "help" => {
            let help = "## 🤖 ClimaBot — Command Reference\n\n\
                ### 🌤 Weather\n\
                `!weather <location>` — Current weather conditions\n\
                `!forecast <location>` — 5-day daily forecast\n\n\
                ### 🎭 Roles\n\
                `!roles` — List self-assignable roles\n\
                `!roles <number>` — Toggle a role on/off\n\n\
                ### 📦 Price Tracking\n\
                `!pt add <url>` — Track a product URL\n\
                `!pt remove <number|url>` — Stop tracking a product\n\
                `!pt list` — View all your tracked products\n\
                `!pt history <number|url>` — See full price history\n\n\
                > Prices are refreshed automatically every hour.";

            client.send_message(channel_id, help).await?;
        }

        // Unknown commands are silently ignored to avoid noise
        _ => return Ok(()),
    }

    // Record latency
    let elapsed = start.elapsed().as_secs_f64();
    metrics::RESPONSE_TIME
        .with_label_values(&[&command, status])
        .observe(elapsed);

    Ok(())
}
