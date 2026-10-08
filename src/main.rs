mod commands;
mod logger;
mod metrics;
mod pricetracker;
mod selfcheck;
mod status;

use anyhow::{Context as _, Result, anyhow};
use pricetracker::store::Store;
use serenity::{
    all::{
        Command, Context, EventHandler, GatewayIntents, Guild, GuildId, Interaction, Ready, ShardManager,
        UnavailableGuild,
    },
    async_trait, Client,
};
use std::{
    collections::HashSet,
    env,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tracing::{error, info, warn};

struct Handler {
    store: Store,
    price_job_started: AtomicBool,
    /// Servers the bot is in, for the `discord_guilds` metric.
    guilds: Mutex<HashSet<GuildId>>,
}

impl Handler {
    fn update_guilds(&self, change: impl FnOnce(&mut HashSet<GuildId>)) {
        let mut guilds = self.guilds.lock().unwrap_or_else(|p| p.into_inner());
        change(&mut guilds);
        metrics::GUILDS.set(guilds.len() as i64);
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            user = %ready.user.tag(),
            guilds = ready.guilds.len(),
            "Logged in to Discord"
        );
        commands::set_bot_user_id(ready.user.id);
        self.update_guilds(|g| {
            g.clear();
            g.extend(ready.guilds.iter().map(|u| u.id));
        });

        // Ready fires again after every gateway reconnect; overwriting the
        // global commands is idempotent, so this is safe.
        match Command::set_global_commands(&ctx.http, commands::definitions()).await {
            Ok(cmds) => info!(count = cmds.len(), "Registered global application (/) commands"),
            Err(e) => error!(error = %e, "Error registering global application (/) commands"),
        }

        if !self.price_job_started.swap(true, Ordering::SeqCst) {
            tokio::spawn(pricetracker::run_scheduler(self.store.clone(), ctx.http.clone()));
            info!("Price-check scheduler started");
        }
    }

    async fn guild_create(&self, _ctx: Context, guild: Guild, _is_new: Option<bool>) {
        self.update_guilds(|g| {
            g.insert(guild.id);
        });
    }

    async fn guild_delete(&self, _ctx: Context, incomplete: UnavailableGuild, _full: Option<Guild>) {
        // `unavailable` means a Discord outage, not that the bot was removed.
        if !incomplete.unavailable {
            self.update_guilds(|g| {
                g.remove(&incomplete.id);
            });
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => commands::handle_command(&ctx, &cmd, &self.store).await,
            Interaction::Component(comp) => commands::handle_component(&ctx, &comp, &self.store).await,
            _ => {}
        }
    }
}

fn discord_token() -> Result<String> {
    // `TOKEN` is what the previous TypeScript deployment used.
    env::var("DISCORD_TOKEN")
        .or_else(|_| env::var("TOKEN"))
        .map(|t| t.trim().to_string())
        .map_err(|_| anyhow!("DISCORD_TOKEN (or TOKEN) must be set"))
}

struct MongoConfig {
    uri: String,
    database: String,
}

fn mongo_config() -> MongoConfig {
    MongoConfig {
        uri: env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".into()),
        database: env::var("MONGODB_DB").unwrap_or_else(|_| "climabot".into()),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    logger::init();

    // `climabot extract <saved.html> <page-url>`: run the price extractor on a
    // saved page, to debug why a shop isn't recognised.
    if env::args().nth(1).as_deref() == Some("extract") {
        let (Some(file), Some(url)) = (env::args().nth(2), env::args().nth(3)) else {
            anyhow::bail!("usage: climabot extract <saved.html> <page-url>");
        };
        let html = std::fs::read_to_string(&file).with_context(|| format!("reading {file}"))?;
        let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string));
        match pricetracker::scrape::extract_product(&html, host.as_deref()) {
            Some(p) => println!("{} — {}", p.name, pricetracker::format_price(p.price, p.currency.as_deref())),
            None => println!("no price found"),
        }
        return Ok(());
    }

    if env::args().nth(1).as_deref() == Some("selfcheck") {
        let urls: Vec<String> = env::args().skip(2).collect();
        let mongo = mongo_config();
        return selfcheck::run(discord_token().ok().as_deref(), &mongo.uri, &mongo.database, &urls).await;
    }

    let token = discord_token()?;
    if env::var("OPENCAGEAPIKEY").is_err() {
        warn!("OPENCAGEAPIKEY is not set: /weather and /forecast will fail");
    }

    let metrics_port: u16 = env::var("METRICS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9464);
    metrics::start_metrics_server(metrics_port);

    let mongo = mongo_config();
    let store = connect_with_retry(&mongo).await;
    info!(database = %mongo.database, "Connected to MongoDB");
    pricetracker::remember_fetch_methods(store.clone()).await;

    // Slash commands and components need no privileged intents.
    let mut client = Client::builder(&token, GatewayIntents::GUILDS)
        .event_handler(Handler {
            store: store.clone(),
            price_job_started: AtomicBool::new(false),
            guilds: Mutex::new(HashSet::new()),
        })
        .await
        .context("creating Discord client")?;

    tokio::spawn(sample_gateway_latency(client.shard_manager.clone()));
    tokio::spawn(status::run_probes(store, client.shard_manager.clone()));

    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        info!("Shutdown signal received, closing gateway connection");
        shard_manager.shutdown_all().await;
    });

    client.start().await.context("Discord client error")?;
    info!("Bot stopped");
    Ok(())
}

/// Export the gateway heartbeat latency every 30 seconds.
async fn sample_gateway_latency(shard_manager: Arc<ShardManager>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        let runners = shard_manager.runners.lock().await;
        if let Some(latency) = runners.values().filter_map(|r| r.latency).max() {
            metrics::GATEWAY_LATENCY.set(latency.as_secs_f64());
        }
    }
}

/// MongoDB may still be starting when the bot boots; keep retrying.
async fn connect_with_retry(mongo: &MongoConfig) -> Store {
    let mut delay = Duration::from_secs(2);
    loop {
        match Store::connect(&mongo.uri, &mongo.database).await {
            Ok(store) => return store,
            Err(e) => {
                warn!(error = format!("{e:#}"), retry_in = ?delay, "MongoDB not available yet");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
