use crate::pricetracker::{
    format_price,
    scrape::{self, ScrapeError},
    store::{AddOutcome, MAX_PRODUCTS_PER_USER, Store, TrackedProduct},
};
use anyhow::Result;
use serenity::all::{
    CommandInteraction, CommandOptionType, Context, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateEmbedFooter, EditInteractionResponse, ResolvedOption, ResolvedValue,
};
use tracing::info;

const EMBED_COLOR: u32 = 0x0099ff;
const MAX_DESCRIPTION: usize = 4000;
const HISTORY_ENTRIES_SHOWN: usize = 25;

// TEMPORARY: shown under every /pt reply until more shops have been tested.
// Remove this constant and its use in `handle` to drop the notice.
const TESTED_SITES_NOTICE: &str = "\
Price tracking has been tested and works on: **KaBuM**, **Amazon Brasil**, **Magazine Luiza**, \
**Pichau**, **Terabyte** and **AliExpress**.\n\
**Mercado Livre** isn't supported yet. Other shops may work, but that isn't guaranteed.";

fn tested_sites_notice() -> CreateEmbed {
    CreateEmbed::new()
        .title("ℹ️ Supported shops")
        .description(TESTED_SITES_NOTICE)
        .color(0x95a5a6)
}

pub fn register() -> CreateCommand {
    let item = || {
        CreateCommandOption::new(
            CommandOptionType::String,
            "item",
            "Position shown in /pt list, or the product URL",
        )
        .required(true)
    };
    CreateCommand::new("pt")
        .description("Track product prices (checked every hour)")
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "add", "Start tracking a product")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "url", "Product page URL")
                        .required(true),
                ),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "remove", "Stop tracking a product")
                .add_sub_option(item()),
        )
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list",
            "View your tracked products",
        ))
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "history", "View a product's price history")
                .add_sub_option(item()),
        )
}

pub async fn handle(ctx: &Context, cmd: &CommandInteraction, store: &Store) -> Result<()> {
    // Scraping can take a while; Discord needs an answer within 3 seconds.
    cmd.defer_ephemeral(&ctx.http).await?;

    let user_id = cmd.user.id.get();
    let options = cmd.data.options();
    let (sub, args) = match options.first() {
        Some(ResolvedOption { name, value: ResolvedValue::SubCommand(args), .. }) => (*name, args.as_slice()),
        _ => ("", &[][..]),
    };
    let arg = |key: &str| {
        args.iter().find_map(|o| match (o.name == key, &o.value) {
            (true, ResolvedValue::String(s)) => Some(s.trim()),
            _ => None,
        })
    };

    let reply = match sub {
        "add" => add(store, user_id, arg("url").unwrap_or_default()).await?,
        "remove" => remove(store, user_id, arg("item").unwrap_or_default()).await?,
        "list" => list(store, user_id).await?,
        "history" => history(store, user_id, arg("item").unwrap_or_default()).await?,
        _ => EditInteractionResponse::new().content("Unknown subcommand."),
    };

    cmd.edit_response(&ctx.http, reply.add_embed(tested_sites_notice())).await?;
    Ok(())
}

fn text(s: impl Into<String>) -> EditInteractionResponse {
    EditInteractionResponse::new().content(s)
}

async fn add(store: &Store, user_id: u64, raw_url: &str) -> Result<EditInteractionResponse> {
    let url = match scrape::parse_url(raw_url) {
        Ok(u) => u.to_string(),
        Err(e) => return Ok(text(format!("❌ {e}"))),
    };

    let product = match scrape::fetch_product(&url).await {
        Ok(p) => p,
        Err(e @ ScrapeError::NoPrice) => {
            return Ok(text(format!(
                "❌ Failed to retrieve a price from that URL ({e}).\n\
                 The page may be unsupported, require login, or load its price with JavaScript."
            )));
        }
        Err(e) => return Ok(text(format!("❌ Failed to retrieve a price from that URL: {e}."))),
    };

    let outcome = store
        .add_product(user_id, url.clone(), product.name.clone(), product.price, product.currency.clone())
        .await?;

    Ok(match outcome {
        AddOutcome::Added => {
            info!(user_id, url, price = product.price, "Added tracked product");
            EditInteractionResponse::new().embed(
                CreateEmbed::new()
                    .title(format!("✅ Now tracking: {}", product.name))
                    .url(&url)
                    .description(format!(
                        "Current price: **{}**\nI'll check it every hour and DM you when it changes.",
                        format_price(product.price, product.currency.as_deref())
                    ))
                    .color(EMBED_COLOR),
            )
        }
        AddOutcome::AlreadyTracked => text("⚠️ You're already tracking this URL."),
        AddOutcome::LimitReached => text(format!(
            "❌ You can track at most {MAX_PRODUCTS_PER_USER} products. Remove one with `/pt remove` first."
        )),
    })
}

/// Find a product by 1-based position or exact URL.
fn find<'a>(products: &'a [TrackedProduct], item: &str) -> Option<(usize, &'a TrackedProduct)> {
    if let Ok(n) = item.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| products.get(i)).map(|p| (n, p));
    }
    let normalised = scrape::parse_url(item).map(|u| u.to_string()).ok();
    products
        .iter()
        .enumerate()
        .find(|(_, p)| p.url == item || Some(&p.url) == normalised.as_ref())
        .map(|(i, p)| (i + 1, p))
}

async fn remove(store: &Store, user_id: u64, item: &str) -> Result<EditInteractionResponse> {
    let products = store.list_products(user_id).await?;
    if products.is_empty() {
        return Ok(text("❌ You have no tracked products."));
    }
    let Some((_, product)) = find(&products, item) else {
        return Ok(text("❌ Product not found. Use `/pt list` to see positions."));
    };
    store.remove_product(user_id, product.id).await?;
    info!(user_id, url = %product.url, "Removed tracked product");
    Ok(text(format!("🗑️ Removed **{}**.", product.name)))
}

async fn list(store: &Store, user_id: u64) -> Result<EditInteractionResponse> {
    let products = store.list_products(user_id).await?;
    if products.is_empty() {
        return Ok(text("📭 You have no tracked products yet. Use `/pt add <url>` to start."));
    }

    let entries = products.iter().enumerate().map(|(i, p)| {
        let price = p
            .last_price
            .map(|v| format_price(v, p.currency.as_deref()))
            .unwrap_or_else(|| "–".into());
        let mut entry = format!(
            "**{}.** [{}]({})\n{} · next check <t:{}:R>",
            i + 1,
            escape_link_text(&truncate(&p.name, 80)),
            p.url,
            price,
            unix(p.next_check_at)
        );
        if p.failures > 0 {
            entry.push_str(&format!(" · ⚠️ last {} check(s) failed", p.failures));
        }
        entry
    });

    Ok(EditInteractionResponse::new().embed(
        CreateEmbed::new()
            .title(format!("📦 Tracked Products ({}/{MAX_PRODUCTS_PER_USER})", products.len()))
            .description(join_limited(entries, "\n\n", MAX_DESCRIPTION))
            .footer(CreateEmbedFooter::new("Each product is re-checked every hour from when it was added"))
            .color(EMBED_COLOR),
    ))
}

async fn history(store: &Store, user_id: u64, item: &str) -> Result<EditInteractionResponse> {
    let products = store.list_products(user_id).await?;
    if products.is_empty() {
        return Ok(text("❌ You have no tracked products."));
    }
    let Some((_, product)) = find(&products, item) else {
        return Ok(text("❌ Product not found. Use `/pt list` to see positions."));
    };

    let entries = store.history(product.id).await?;
    let currency = product.currency.as_deref();
    let skipped = entries.len().saturating_sub(HISTORY_ENTRIES_SHOWN);

    let mut prev: Option<f64> = skipped.checked_sub(1).map(|i| entries[i].price);
    let mut lines = Vec::new();
    if skipped > 0 {
        lines.push(format!("*… {skipped} older entries*"));
    }
    for e in &entries[skipped..] {
        let arrow = match prev {
            Some(p) if e.price < p => "📉",
            Some(p) if e.price > p => "📈",
            _ => "•",
        };
        lines.push(format!("{arrow} **{}** — <t:{}:f>", format_price(e.price, currency), unix(e.observed_at)));
        prev = Some(e.price);
    }

    let lowest = entries.iter().map(|e| e.price).fold(f64::INFINITY, f64::min);
    let highest = entries.iter().map(|e| e.price).fold(f64::NEG_INFINITY, f64::max);
    let footer = if entries.is_empty() {
        "No history yet".to_string()
    } else {
        format!(
            "Lowest {} · Highest {} · {} change(s)",
            format_price(lowest, currency),
            format_price(highest, currency),
            entries.len() - 1
        )
    };

    Ok(EditInteractionResponse::new().embed(
        CreateEmbed::new()
            .title(format!("📈 Price History — {}", truncate(&product.name, 200)))
            .url(&product.url)
            .description(join_limited(lines.into_iter(), "\n", MAX_DESCRIPTION))
            .footer(CreateEmbedFooter::new(footer))
            .color(EMBED_COLOR),
    ))
}

/// Seconds since the epoch, for Discord's `<t:…>` timestamp markup.
fn unix(t: mongodb::bson::DateTime) -> i64 {
    t.timestamp_millis() / 1000
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

fn escape_link_text(s: &str) -> String {
    s.replace('[', "(").replace(']', ")")
}

/// Join entries until the next one would exceed `limit` characters.
fn join_limited(entries: impl Iterator<Item = String>, sep: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut total = 0;
    let entries: Vec<String> = entries.collect();
    for (shown, entry) in entries.iter().enumerate() {
        let add = entry.chars().count() + if out.is_empty() { 0 } else { sep.len() };
        if total + add > limit - 40 {
            out.push_str(&format!("{sep}*… and {} more*", entries.len() - shown));
            break;
        }
        if !out.is_empty() {
            out.push_str(sep);
        }
        out.push_str(entry);
        total += add;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{DateTime, oid::ObjectId};

    fn product(id: u8, url: &str) -> TrackedProduct {
        TrackedProduct {
            id: ObjectId::from_bytes([id; 12]),
            user_id: 1,
            url: url.into(),
            name: format!("P{id}"),
            currency: None,
            last_price: Some(1.0),
            created_at: DateTime::now(),
            last_checked: None,
            next_check_at: DateTime::now(),
            failures: 0,
            lease_until: None,
        }
    }

    #[test]
    fn find_by_position_or_url() {
        let ps = vec![product(10, "https://a.com/x"), product(11, "https://b.com/")];
        assert_eq!(find(&ps, "1").map(|(_, p)| p.url.as_str()), Some("https://a.com/x"));
        assert_eq!(find(&ps, "2").map(|(_, p)| p.url.as_str()), Some("https://b.com/"));
        assert!(find(&ps, "0").is_none());
        assert!(find(&ps, "3").is_none());
        assert_eq!(find(&ps, "https://b.com/").map(|(n, _)| n), Some(2));
        // Normalised the same way as on insert.
        assert_eq!(find(&ps, "https://b.com").map(|(n, _)| n), Some(2));
        assert!(find(&ps, "https://c.com/").is_none());
    }

    #[test]
    fn limited_join() {
        let entries = (0..100).map(|i| format!("entry number {i}"));
        let s = join_limited(entries, "\n", 200);
        assert!(s.chars().count() <= 200);
        assert!(s.ends_with("more*"));
        assert_eq!(join_limited(["a".to_string()].into_iter(), "\n", 200), "a");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
    }
}
