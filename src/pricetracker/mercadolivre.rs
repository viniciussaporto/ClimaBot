//! Mercado Livre / Mercado Libre prices through the official API.
//!
//! Their website sends datacenter IPs (like our server's) to an account
//! verification page, so scraping it only works through a residential proxy.
//! The API works from anywhere with an application token: create an app at
//! https://developers.mercadolivre.com.br and set `ML_CLIENT_ID` and
//! `ML_CLIENT_SECRET`.

use super::scrape::{Product, ScrapeError};
use serde::Deserialize;
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use url::Url;

const API: &str = "https://api.mercadolibre.com";

#[derive(Debug, PartialEq)]
pub enum Listing {
    /// Catalog product page (`/p/MLB1027172677`): price of the buy box winner.
    Catalog(String),
    /// A seller's listing (`/MLB-3846867869-…`).
    Item(String),
}

pub fn is_mercadolivre(host: &str) -> bool {
    host.split('.')
        .any(|label| label == "mercadolivre" || label == "mercadolibre")
}

/// Find an ML id (`MLB123…`, `MLA-123…`) in `s`, normalised without the dash.
fn find_id(s: &str) -> Option<String> {
    let b = s.as_bytes();
    (0..b.len()).find_map(|i| {
        let site = b.get(i..i + 3)?;
        if !(site[0] == b'M' && site[1].is_ascii_uppercase() && site[2].is_ascii_uppercase()) {
            return None;
        }
        if i > 0 && b[i - 1].is_ascii_alphanumeric() {
            return None;
        }
        let mut j = i + 3;
        if b.get(j) == Some(&b'-') {
            j += 1;
        }
        let digits = b[j..].iter().take_while(|c| c.is_ascii_digit()).count();
        (digits >= 6).then(|| format!("{}{}", &s[i..i + 3], &s[j..j + digits]))
    })
}

pub fn listing(url: &Url) -> Option<Listing> {
    if !is_mercadolivre(url.host_str()?) {
        return None;
    }
    let path = url.path();
    if let Some(rest) = path.split("/p/").nth(1) {
        return find_id(rest).map(Listing::Catalog);
    }
    url.query_pairs()
        .find(|(k, _)| k == "item_id" || k == "wid")
        .and_then(|(_, v)| find_id(&v))
        .or_else(|| find_id(path))
        .map(Listing::Item)
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct Offer {
    price: Option<f64>,
    currency_id: Option<String>,
}

#[derive(Deserialize)]
struct CatalogProduct {
    name: String,
    buy_box_winner: Option<Offer>,
}

#[derive(Deserialize)]
struct CatalogItems {
    #[serde(default)]
    results: Vec<Offer>,
}

#[derive(Deserialize)]
struct Item {
    title: String,
    price: Option<f64>,
    currency_id: Option<String>,
}

pub struct Client {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    token: Mutex<Option<(String, Instant)>>,
}

fn api_error(e: reqwest::Error) -> ScrapeError {
    match e.status() {
        Some(s) => ScrapeError::Status(s),
        None => ScrapeError::Network(format!("Mercado Livre API: {}", e.without_url())),
    }
}

impl Client {
    async fn token(&self) -> Result<String, ScrapeError> {
        let mut cached = self.token.lock().await;
        if let Some((token, expires)) = cached.as_ref()
            && Instant::now() < *expires
        {
            return Ok(token.clone());
        }
        let token: Token = self
            .http
            .post(format!("{API}/oauth/token"))
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
            ])
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(api_error)?
            .json()
            .await
            .map_err(api_error)?;
        // Refresh a few minutes early.
        let ttl = Duration::from_secs(token.expires_in.saturating_sub(300).max(60));
        *cached = Some((token.access_token.clone(), Instant::now() + ttl));
        Ok(token.access_token)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ScrapeError> {
        let token = self.token().await?;
        self.http
            .get(format!("{API}{path}"))
            .bearer_auth(token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(api_error)?
            .json()
            .await
            .map_err(api_error)
    }

    /// Verify the credentials by obtaining an application token.
    pub async fn check_credentials(&self) -> Result<(), ScrapeError> {
        self.token().await.map(|_| ())
    }

    pub async fn fetch(&self, listing: &Listing) -> Result<Product, ScrapeError> {
        let (name, offer) = match listing {
            Listing::Item(id) => {
                let item: Item = self.get(&format!("/items/{id}")).await?;
                (item.title, Offer { price: item.price, currency_id: item.currency_id })
            }
            Listing::Catalog(id) => {
                let product: CatalogProduct = self.get(&format!("/products/{id}")).await?;
                let offer = match product.buy_box_winner {
                    Some(o) if o.price.is_some() => o,
                    // No buy box winner: cheapest listing of this product.
                    _ => {
                        let items: CatalogItems = self.get(&format!("/products/{id}/items")).await?;
                        items
                            .results
                            .into_iter()
                            .filter(|o| o.price.is_some())
                            .min_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal))
                            .ok_or(ScrapeError::NoPrice)?
                    }
                };
                (product.name, offer)
            }
        };
        let price = offer.price.filter(|p| p.is_finite() && *p > 0.0).ok_or(ScrapeError::NoPrice)?;
        Ok(Product { name, price, currency: offer.currency_id })
    }
}

pub static CLIENT: LazyLock<Option<Client>> = LazyLock::new(|| {
    let id = std::env::var("ML_CLIENT_ID").ok().filter(|s| !s.trim().is_empty())?;
    let secret = std::env::var("ML_CLIENT_SECRET").ok().filter(|s| !s.trim().is_empty())?;
    Some(Client {
        client_id: id.trim().to_string(),
        client_secret: secret.trim().to_string(),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build HTTP client"),
        token: Mutex::new(None),
    })
});

#[cfg(test)]
mod tests {
    use super::*;

    fn l(u: &str) -> Option<Listing> {
        listing(&Url::parse(u).unwrap())
    }

    #[test]
    fn recognises_listings() {
        assert_eq!(
            l("https://www.mercadolivre.com.br/apple-iphone-15-128-gb-preto/p/MLB1027172677"),
            Some(Listing::Catalog("MLB1027172677".into()))
        );
        assert_eq!(
            l("https://www.mercadolivre.com.br/x/p/MLB1027172677?pdp_filters=item_id:MLB123&wid=MLB9999999"),
            Some(Listing::Catalog("MLB1027172677".into()))
        );
        assert_eq!(
            l("https://produto.mercadolivre.com.br/MLB-3846867869-iphone-15-_JM"),
            Some(Listing::Item("MLB3846867869".into()))
        );
        assert_eq!(
            l("https://articulo.mercadolibre.com.ar/MLA-1411223344-zapatillas"),
            Some(Listing::Item("MLA1411223344".into()))
        );
        assert_eq!(
            l("https://www.mercadolibre.com.mx/ofertas?wid=MLM2233445566"),
            Some(Listing::Item("MLM2233445566".into()))
        );
        assert_eq!(l("https://www.mercadolivre.com.br/ofertas"), None);
        assert_eq!(l("https://www.amazon.com.br/dp/MLB1234567"), None);
        assert!(is_mercadolivre("produto.mercadolivre.com.br"));
        assert!(!is_mercadolivre("notmercadolivre.com"));
    }

    #[test]
    fn parses_api_payloads() {
        let p: CatalogProduct = serde_json::from_str(
            r#"{"id":"MLB1027172677","name":"Apple iPhone 15 (128 GB) - Preto",
                "buy_box_winner":{"item_id":"MLB1","price":4299.0,"currency_id":"BRL"}}"#,
        )
        .unwrap();
        assert_eq!(p.buy_box_winner.unwrap().price, Some(4299.0));

        let p: CatalogProduct = serde_json::from_str(r#"{"name":"X","buy_box_winner":null}"#).unwrap();
        assert!(p.buy_box_winner.is_none());

        let i: Item = serde_json::from_str(r#"{"title":"T","price":10.5,"currency_id":"ARS"}"#).unwrap();
        assert_eq!((i.price, i.currency_id.as_deref()), (Some(10.5), Some("ARS")));
    }
}
