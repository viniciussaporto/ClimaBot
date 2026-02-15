use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use mongodb::{
    bson::doc,
    options::ReplaceOptions,
    Collection, Database,
};
use reqwest::Client as HttpClient;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

// ─────────────────────────────────────────────
//  Domain types  (equiv. to Mongoose schemas)
// ─────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PriceEntry {
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrackedProduct {
    pub url: String,
    pub name: String,
    pub price_history: Vec<PriceEntry>,
    pub last_checked: DateTime<Utc>,
    pub valid: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserTrack {
    pub user_id: String,
    pub product_links: Vec<TrackedProduct>,
}

// ─────────────────────────────────────────────
//  Scraper  (replaces cheerio)
// ─────────────────────────────────────────────

/// Attempt to scrape the page title and first recognisable price.
/// Returns `None` if the URL is unreachable or no price is found.
pub async fn scrape_product(url: &str) -> Option<(String, f64)> {
    let client = HttpClient::builder()
        .user_agent("Mozilla/5.0 (compatible; ClimaBot/1.0; +https://github.com/viniciussaporto/ClimaBot)")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    let html = client.get(url).send().await.ok()?.text().await.ok()?;
    let document = Html::parse_document(&html);

    // Page title as product name
    let name = Selector::parse("title")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|el| el.inner_html().trim().to_string())
        .unwrap_or_else(|| url.to_string());

    // Try price selectors from most to least specific
    let price_selectors = [
        "[itemprop='price']",
        "[data-price]",
        "[class*='price']",
        ".price",
        "#price",
    ];

    for sel_str in &price_selectors {
        let Ok(selector) = Selector::parse(sel_str) else {
            continue;
        };
        for element in document.select(&selector) {
            // Try content / data-price attribute first, then inner text
            let raw = element
                .value()
                .attr("content")
                .or_else(|| element.value().attr("data-price"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| element.text().collect::<String>());

            let digits: String = raw
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect();

            if digits.is_empty() {
                continue;
            }

            // Handle European decimal separator (comma)
            let normalised = if digits.matches(',').count() == 1 && !digits.contains('.') {
                digits.replace(',', ".")
            } else {
                digits.replace(',', "")
            };

            if let Ok(price) = normalised.parse::<f64>() {
                if price > 0.0 && price.is_finite() {
                    return Some((name, price));
                }
            }
        }
    }

    None
}

// ─────────────────────────────────────────────
//  Command dispatcher
// ─────────────────────────────────────────────

pub async fn handle_pricetracking(
    db: &Database,
    channel_id: &str,
    user_id: &str,
    args: &[&str],
    revolt_client: &crate::revolt::RevoltClient,
) -> Result<()> {
    let col: Collection<UserTrack> = db.collection("usertrack");

    match args.first().copied() {
        Some("add") => {
            let url = args.get(1).copied().unwrap_or("");
            cmd_add(&col, channel_id, user_id, url, revolt_client).await?;
        }
        Some("remove") | Some("rm") => {
            let target = args.get(1).copied().unwrap_or("");
            cmd_remove(&col, channel_id, user_id, target, revolt_client).await?;
        }
        Some("list") | Some("track") | Some("ls") => {
            cmd_list(&col, channel_id, user_id, revolt_client).await?;
        }
        Some("history") | Some("hist") => {
            let target = args.get(1).copied().unwrap_or("");
            cmd_history(&col, channel_id, user_id, target, revolt_client).await?;
        }
        _ => {
            revolt_client
                .send_message(
                    channel_id,
                    "## 📦 Price Tracking\n\n\
                    `!pt add <url>` — start tracking a product\n\
                    `!pt remove <number|url>` — stop tracking\n\
                    `!pt list` — view all tracked products\n\
                    `!pt history <number|url>` — view price history",
                )
                .await?;
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────
//  Sub-commands
// ─────────────────────────────────────────────

async fn cmd_add(
    col: &Collection<UserTrack>,
    channel_id: &str,
    user_id: &str,
    url: &str,
    client: &crate::revolt::RevoltClient,
) -> Result<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        client
            .send_message(channel_id, "❌ Please provide a valid URL starting with `http://` or `https://`.")
            .await?;
        return Ok(());
    }

    client.send_message(channel_id, "⏳ Fetching product info…").await?;

    match scrape_product(url).await {
        None => {
            client
                .send_message(
                    channel_id,
                    "❌ Failed to retrieve a price from that URL.\n\
                     The page may be unsupported, require login, or have no detectable price.",
                )
                .await?;
        }
        Some((name, price)) => {
            let mut doc = get_or_create(col, user_id).await?;

            if doc.product_links.iter().any(|p| p.url == url) {
                client.send_message(channel_id, "⚠️ Already tracking this URL.").await?;
                return Ok(());
            }

            doc.product_links.push(TrackedProduct {
                url: url.to_string(),
                name: name.clone(),
                price_history: vec![PriceEntry { price, timestamp: Utc::now() }],
                last_checked: Utc::now(),
                valid: true,
            });

            save(col, &doc).await?;
            info!(user_id, url, price, "Added tracked product");

            client
                .send_message(
                    channel_id,
                    &format!("✅ Now tracking **{name}** — current price: **{price:.2}**"),
                )
                .await?;
        }
    }

    Ok(())
}

async fn cmd_remove(
    col: &Collection<UserTrack>,
    channel_id: &str,
    user_id: &str,
    target: &str,
    client: &crate::revolt::RevoltClient,
) -> Result<()> {
    let mut doc = get_or_create(col, user_id).await?;

    if doc.product_links.is_empty() {
        client.send_message(channel_id, "❌ You have no tracked products.").await?;
        return Ok(());
    }

    let maybe_pos = if let Ok(n) = target.parse::<usize>() {
        if n >= 1 && n <= doc.product_links.len() {
            Some(n - 1)
        } else {
            None
        }
    } else {
        doc.product_links.iter().position(|p| p.url == target)
    };

    match maybe_pos {
        None => {
            client.send_message(channel_id, "❌ Product not found. Use `!pt list` to see positions.").await?;
        }
        Some(i) => {
            let removed = doc.product_links.remove(i);
            save(col, &doc).await?;
            client
                .send_message(channel_id, &format!("🗑️ Removed **{}**.", removed.name))
                .await?;
        }
    }

    Ok(())
}

async fn cmd_list(
    col: &Collection<UserTrack>,
    channel_id: &str,
    user_id: &str,
    client: &crate::revolt::RevoltClient,
) -> Result<()> {
    let doc = get_or_create(col, user_id).await?;

    if doc.product_links.is_empty() {
        client.send_message(channel_id, "📭 You have no tracked products yet. Use `!pt add <url>` to start.").await?;
        return Ok(());
    }

    let mut msg = format!("## 📦 Tracked Products ({} total)\n\n", doc.product_links.len());

    for (i, p) in doc.product_links.iter().enumerate() {
        let last = p.price_history.last();
        let price_str = last.map(|e| format!("{:.2}", e.price)).unwrap_or_else(|| "–".into());
        let date_str = last
            .map(|e| e.timestamp.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".into());

        msg.push_str(&format!(
            "**{}**. {}\n{}\nLast price: **{}** on {}\n\n",
            i + 1, p.name, p.url, price_str, date_str
        ));
    }

    client.send_message(channel_id, &msg).await?;
    Ok(())
}

async fn cmd_history(
    col: &Collection<UserTrack>,
    channel_id: &str,
    user_id: &str,
    target: &str,
    client: &crate::revolt::RevoltClient,
) -> Result<()> {
    let doc = get_or_create(col, user_id).await?;

    if doc.product_links.is_empty() {
        client.send_message(channel_id, "❌ You have no tracked products.").await?;
        return Ok(());
    }

    let product = if let Ok(n) = target.parse::<usize>() {
        doc.product_links.get(n.saturating_sub(1))
    } else {
        doc.product_links.iter().find(|p| p.url == target)
    };

    match product {
        None => {
            client.send_message(channel_id, "❌ Product not found.").await?;
        }
        Some(p) => {
            let mut msg = format!("## 📈 Price History — {}\n\n", p.name);

            for entry in &p.price_history {
                msg.push_str(&format!(
                    "• **{:.2}** — {}\n",
                    entry.price,
                    entry.timestamp.format("%Y-%m-%d %H:%M UTC")
                ));
            }

            client.send_message(channel_id, &msg).await?;
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────
//  DB helpers
// ─────────────────────────────────────────────

async fn get_or_create(col: &Collection<UserTrack>, user_id: &str) -> Result<UserTrack> {
    match col.find_one(doc! { "user_id": user_id }, None).await? {
        Some(d) => Ok(d),
        None => Ok(UserTrack {
            user_id: user_id.to_string(),
            product_links: vec![],
        }),
    }
}

async fn save(col: &Collection<UserTrack>, user: &UserTrack) -> Result<()> {
    let opts = ReplaceOptions::builder().upsert(true).build();
    col.replace_one(doc! { "user_id": &user.user_id }, user, opts).await?;
    Ok(())
}

// ─────────────────────────────────────────────
//  Hourly cron job  (equiv. node-cron)
// ─────────────────────────────────────────────

/// Called once per hour by the scheduler in main.
/// Iterates every tracked product and updates its price history.
pub async fn run_price_check_cron(db: Database) {
    let col: Collection<UserTrack> = db.collection("usertrack");

    let mut cursor = match col.find(None, None).await {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Cron: failed to query usertrack collection");
            return;
        }
    };

    while let Some(res) = cursor.next().await {
        let mut user = match res {
            Ok(u) => u,
            Err(e) => {
                error!(error = %e, "Cron: cursor error");
                continue;
            }
        };

        let mut changed = false;

        for product in &mut user.product_links {
            if !product.valid {
                continue;
            }

            match scrape_product(&product.url).await {
                None => {
                    error!(url = %product.url, "Cron: scrape failed — marking invalid");
                    product.valid = false;
                    changed = true;
                }
                Some((name, price)) => {
                    let last = product.price_history.last().map(|e| e.price);
                    if last != Some(price) {
                        info!(url = %product.url, old_price = ?last, new_price = price, "Price change detected");
                        product.price_history.push(PriceEntry { price, timestamp: Utc::now() });
                        changed = true;
                    }
                    product.name = name;
                    product.last_checked = Utc::now();
                    product.valid = true;
                }
            }
        }

        if changed {
            if let Err(e) = save(&col, &user).await {
                error!(error = %e, user_id = %user.user_id, "Cron: save failed");
            }
        }
    }
}
