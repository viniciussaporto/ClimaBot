//! `climabot selfcheck [url…]` — verifies a deployment without touching the
//! gateway: Discord credentials, registered commands, role menus per guild,
//! the weather APIs, the database, and optionally scraping the given URLs.

use crate::{
    commands::{roles, weather},
    pricetracker::{format_price, mercadolivre, scrape, solver, store::Store},
};
use anyhow::{Result, bail};
use serenity::all::{Http, Permissions};

struct Report {
    failures: usize,
}

impl Report {
    fn check<T>(&mut self, what: &str, result: Result<T>) -> Option<T> {
        match result {
            Ok(v) => {
                println!("[ OK ] {what}");
                Some(v)
            }
            Err(e) => {
                println!("[FAIL] {what}: {e:#}");
                self.failures += 1;
                None
            }
        }
    }
}

pub async fn run(token: Option<&str>, mongo_uri: &str, database: &str, urls: &[String]) -> Result<()> {
    let mut r = Report { failures: 0 };

    // ── Discord ───────────────────────────────
    let http = token.map(Http::new);
    let me = match &http {
        Some(http) => r.check("Discord token", http.get_current_user().await.map_err(Into::into)),
        None => {
            println!("[SKIP] Discord checks: DISCORD_TOKEN / TOKEN not set");
            None
        }
    };
    if let (Some(http), Some(me)) = (&http, me) {
        println!("       logged in as {} ({})", me.tag(), me.id);

        if let Some(app) = r.check(
            "Application info",
            http.get_current_application_info().await.map_err(Into::into),
        ) {
            http.set_application_id(app.id);
            if let Some(cmds) = r.check("Global commands", http.get_global_commands().await.map_err(Into::into)) {
                let names: Vec<_> = cmds.iter().map(|c| format!("/{}", c.name)).collect();
                println!("       registered: {}", names.join(" "));
            }
        }

        if let Some(guilds) = r.check("Guild list", http.get_guilds(None, None).await.map_err(Into::into)) {
            for g in guilds {
                let result = async {
                    let roles_list = http.get_guild_roles(g.id).await?;
                    let me_member = http.get_member(g.id, me.id).await?;
                    let perms = roles_list
                        .iter()
                        .filter(|role| role.id.get() == g.id.get() || me_member.roles.contains(&role.id))
                        .fold(Permissions::empty(), |acc, role| acc | role.permissions);
                    let assignable: Vec<String> = roles::assignable_roles(&roles_list, g.id, &me_member.roles)
                        .iter()
                        .map(|role| role.name.clone())
                        .collect();
                    anyhow::Ok((perms, assignable))
                }
                .await;
                if let Some((perms, assignable)) = r.check(&format!("Guild '{}' roles", g.name), result) {
                    let manage = perms.intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_ROLES);
                    println!(
                        "       manage roles: {manage}; {} assignable: {}",
                        assignable.len(),
                        assignable.join(", ")
                    );
                }
            }
        }
    }

    // ── Weather APIs ──────────────────────────
    if let Some(loc) = r.check("OpenCage geocoding", weather::get_coordinates("Rio de Janeiro").await) {
        println!("       resolved: {} ({:.3}, {:.3})", loc.formatted, loc.lat, loc.lng);
        if let Some(c) = r.check("Open-Meteo current", weather::get_current(&loc).await) {
            println!(
                "       {:.1}°C, {}",
                c.temperature_2m,
                weather::weather_description(c.weathercode)
            );
        }
        if let Some(d) = r.check("Open-Meteo forecast", weather::get_forecast(&loc).await) {
            println!("       {} days: {}", d.time.len(), d.time.join(", "));
        }
    }
    let result = weather::get_coordinates("zzqxv nowhere qqq").await;
    let not_found = match result {
        Err(e) if e.is::<weather::LocationNotFound>() => Ok(()),
        Err(e) => Err(e),
        Ok(l) => Err(anyhow::anyhow!("unexpectedly resolved to {}", l.formatted)),
    };
    r.check("OpenCage unknown location is reported as not found", not_found);

    // ── Database ──────────────────────────────
    let live = Store::connect(mongo_uri, database).await;
    if let Some(store) = r.check(&format!("MongoDB database '{database}'"), live)
        && let Some(n) = r.check("Count tracked products", store.count_products().await)
    {
        println!("       {n} tracked product(s)");
    }
    let scratch = format!("{database}_selfcheck");
    let roundtrip = db_roundtrip(mongo_uri, &scratch).await;
    r.check(&format!("MongoDB add/claim/check/remove round trip (scratch db '{scratch}')"), roundtrip);

    // ── Scraper ───────────────────────────────
    if solver::SOLVERS.is_empty() {
        println!("[SKIP] Browser solvers: FLARESOLVERR_URL / BYPARR_URL not set (direct fetches only)");
    }
    for s in solver::SOLVERS.iter() {
        if let Some(status) = r.check(s.name, s.health().await) {
            println!("       {status}");
        }
    }
    match mercadolivre::CLIENT.as_ref() {
        Some(ml) => {
            r.check("Mercado Livre API credentials", ml.check_credentials().await.map_err(Into::into));
        }
        None => println!("[SKIP] Mercado Livre API: ML_CLIENT_ID / ML_CLIENT_SECRET not set"),
    }
    if let Some(proxy) = scrape::proxy() {
        let host = url::Url::parse(&proxy).ok().and_then(|u| u.host_str().map(str::to_string));
        println!("[INFO] Scraping through proxy {}", host.unwrap_or_else(|| "(unparseable)".into()));
    }
    for url in urls {
        if let Some(p) = r.check(&format!("Scrape {url}"), scrape::fetch_product(url).await.map_err(Into::into)) {
            println!("       {} — {}", p.name, format_price(p.price, p.currency.as_deref()));
        }
    }

    if r.failures > 0 {
        bail!("{} check(s) failed", r.failures);
    }
    println!("All checks passed.");
    Ok(())
}

/// Exercise the store end to end on a throwaway database.
async fn db_roundtrip(uri: &str, database: &str) -> Result<()> {
    use crate::pricetracker::store::AddOutcome;
    use std::time::Duration;

    let store = Store::connect(uri, database).await?;
    let result = async {
        let user = 42;
        let url = "https://example.com/p".to_string();
        anyhow::ensure!(store.add_product(user, url.clone(), "P".into(), 10.0, Some("EUR".into())).await? == AddOutcome::Added);
        anyhow::ensure!(store.add_product(user, url.clone(), "P".into(), 10.0, None).await? == AddOutcome::AlreadyTracked);
        let products = store.list_products(user).await?;
        anyhow::ensure!(products.len() == 1, "expected 1 product");
        // Not due yet: nothing to claim.
        anyhow::ensure!(store.claim_due(Duration::from_secs(60)).await?.is_none(), "claimed a product that is not due");
        let p = &products[0];
        anyhow::ensure!(store.record_check(p, "P2", 8.5, None).await? == Some(10.0), "price change not reported");
        let hist = store.history(p.id).await?;
        anyhow::ensure!(hist.len() == 2 && hist[1].currency.as_deref() == Some("EUR"), "history mismatch");
        anyhow::ensure!(store.record_failure(p).await? == 1, "failure count mismatch");
        anyhow::ensure!(store.remove_product(user, p.id).await?, "remove failed");
        anyhow::ensure!(store.history(p.id).await?.is_empty(), "history not removed");
        Ok(())
    }
    .await;
    store.drop_database().await?;
    result
}
