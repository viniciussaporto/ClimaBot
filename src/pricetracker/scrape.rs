//! Fetches a product page and extracts its name and price.
//!
//! Fetch strategy, cheapest first:
//! 1. Mercado Livre API, for Mercado Livre URLs when credentials are set.
//! 2. A direct HTTP request.
//! 3. Challenge-solving browsers (FlareSolverr, then Byparr) when configured;
//!    each retried once with extra wait time when it returns a challenge page
//!    that may still be solving itself.
//!
//! The method that worked last for a shop is tried first next time.
//!
//! Users supply arbitrary URLs, so every URL (including each redirect hop and
//! the browser's final URL) must resolve only to public IP addresses on the
//! standard web ports. Otherwise the bot could be used to probe services on
//! its private network (Prometheus, Grafana, …).

use reqwest::{StatusCode, header, redirect};
use scraper::{Html, Selector};
use serde_json::Value;
use super::{currency, mercadolivre, sites, solver};
use crate::metrics;
use std::{
    collections::HashMap,
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tracing::debug;
use url::{Host, Url};

/// Direct requests look like a desktop browser; many shops reject anything else.
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const MAX_REDIRECTS: usize = 8;
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
/// Extra time given to self-solving JavaScript challenges (e.g. Akamai).
const CHALLENGE_WAIT: Duration = Duration::from_secs(8);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_NAME_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct Product {
    pub name: String,
    pub price: f64,
    pub currency: Option<String>,
}

#[derive(Debug)]
pub enum ScrapeError {
    InvalidUrl(String),
    Blocked(String),
    Status(StatusCode),
    Network(String),
    /// The shop served a bot-protection or verification page instead.
    BotProtection,
    NoPrice,
}

impl fmt::Display for ScrapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(why) => write!(f, "invalid URL: {why}"),
            Self::Blocked(why) => write!(f, "URL not allowed: {why}"),
            Self::Status(s) => write!(f, "the page answered with HTTP {s}"),
            Self::Network(e) => write!(f, "could not load the page: {e}"),
            Self::BotProtection => write!(f, "the shop's bot protection blocked access"),
            Self::NoPrice => write!(f, "no price found on the page"),
        }
    }
}

impl std::error::Error for ScrapeError {}

/// Validate a user-supplied URL before storing it.
pub fn parse_url(raw: &str) -> Result<Url, ScrapeError> {
    let url = Url::parse(raw.trim()).map_err(|e| ScrapeError::InvalidUrl(e.to_string()))?;
    check_url_shape(&url)?;
    Ok(url)
}

/// Optional proxy for all scraping traffic (`SCRAPER_PROXY`, e.g.
/// `http://user:pass@host:port` or `socks5://host:port`). A residential proxy
/// gets past shops that block datacenter IPs outright.
pub fn proxy() -> Option<String> {
    std::env::var("SCRAPER_PROXY").ok().map(|p| p.trim().to_string()).filter(|p| !p.is_empty())
}

/// Heuristic: does this page look like a bot wall rather than a product?
fn looks_blocked(html: &str, url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    let head = &html.as_bytes()[..html.len().min(200_000)];
    let lower = String::from_utf8_lossy(head).to_ascii_lowercase();
    ["verification", "captcha", "challenge", "blocked", "robot"].iter().any(|k| path.contains(k))
        || lower.contains("captcha")
        || lower.contains("sec-if-cpt-container")
        || lower.contains("/cdn-cgi/challenge-platform")
        || html.len() < 3000
}

/// How a page was (or can be) fetched.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Method {
    Direct,
    Solver(usize),
}

impl Method {
    fn label(self) -> String {
        match self {
            Method::Direct => "direct".into(),
            Method::Solver(i) => solver::SOLVERS[i].name.to_ascii_lowercase(),
        }
    }
}

/// The method that last worked for each host, tried first next time so hourly
/// checks don't repeat attempts that are known to fail.
static PREFERRED: LazyLock<Mutex<HashMap<String, Method>>> = LazyLock::new(Default::default);

fn methods_for(host: &str) -> Vec<Method> {
    let mut methods: Vec<Method> = std::iter::once(Method::Direct)
        .chain((0..solver::SOLVERS.len()).map(Method::Solver))
        .collect();
    let preferred = PREFERRED.lock().unwrap_or_else(|p| p.into_inner()).get(host).copied();
    if let Some(p) = preferred
        && let Some(pos) = methods.iter().position(|m| *m == p)
    {
        methods[..=pos].rotate_right(1);
    }
    methods
}

/// Outcome of looking for a price on a page that did load.
fn page_result(html: &str, url: &Url) -> Result<Product, ScrapeError> {
    match extract_product(html, url.host_str()) {
        Some(p) => Ok(p),
        None if looks_blocked(html, url) => Err(ScrapeError::BotProtection),
        None => Err(ScrapeError::NoPrice),
    }
}

async fn attempt(method: Method, url: &Url) -> Result<Product, ScrapeError> {
    match method {
        Method::Direct => match fetch_html(url.clone()).await {
            Ok((html, final_url)) => page_result(&html, &final_url),
            // What bot protection answers to requests it rejects.
            Err(ScrapeError::Status(s)) if matches!(s.as_u16(), 403 | 429 | 503) => Err(ScrapeError::BotProtection),
            Err(e) => Err(e),
        },
        Method::Solver(i) => {
            let solver = &solver::SOLVERS[i];
            let mut result = Err(ScrapeError::BotProtection);
            // A second, slower try only when the first one came back with a
            // challenge page that may still be solving itself (e.g. Akamai).
            for wait in [None, Some(CHALLENGE_WAIT)] {
                let page = solver.get(url, wait).await?;
                // The browser followed redirects on its own: re-check where it ended up.
                check_url_shape(&page.url)?;
                resolve_public(&page.url).await?;
                result = page_result(&page.html, &page.url);
                debug!(solver = solver.name, url = %page.url, ?wait, ok = result.is_ok(), "Browser fetch");
                if !matches!(result, Err(ScrapeError::BotProtection)) {
                    break;
                }
            }
            result
        }
    }
}

/// Fetch `url` and extract the product name and price.
pub async fn fetch_product(raw_url: &str) -> Result<Product, ScrapeError> {
    let url = parse_url(raw_url)?;
    resolve_public(&url).await?;
    let host = url.host_str().unwrap_or_default().to_string();

    if let (Some(listing), Some(ml)) = (mercadolivre::listing(&url), mercadolivre::CLIENT.as_ref()) {
        let result = ml.fetch(&listing).await;
        let status = if result.is_ok() { "success" } else { "error" };
        metrics::SCRAPE_FETCH_COUNTER.with_label_values(&["mercadolivre_api", status]).inc();
        match result {
            Ok(p) => return Ok(p),
            Err(e) => debug!(error = %e, url = %url, "Mercado Livre API failed; trying the page"),
        }
    }

    // The most informative failure wins: a page that loaded without a price
    // beats a bot wall, which beats network errors.
    let rank = |e: &ScrapeError| match e {
        ScrapeError::NoPrice => 3,
        ScrapeError::BotProtection => 2,
        ScrapeError::Status(_) => 1,
        _ => 0,
    };
    let mut best_error: Option<ScrapeError> = None;

    for method in methods_for(&host) {
        let result = attempt(method, &url).await;
        let status = match &result {
            Ok(_) => "success",
            Err(ScrapeError::NoPrice) => "no_price",
            Err(ScrapeError::BotProtection) => "blocked",
            Err(_) => "error",
        };
        metrics::SCRAPE_FETCH_COUNTER.with_label_values(&[method.label().as_str(), status]).inc();

        match result {
            Ok(p) => {
                PREFERRED.lock().unwrap_or_else(|p| p.into_inner()).insert(host, method);
                return Ok(p);
            }
            // Refusing a URL is final, whichever method found out.
            Err(e @ (ScrapeError::InvalidUrl(_) | ScrapeError::Blocked(_))) => return Err(e),
            Err(e) => {
                debug!(method = %method.label(), error = %e, url = %url, "Fetch method failed");
                // A browser rendered the real page and it has no price (e.g.
                // out of stock): another browser won't find one either.
                let final_answer = matches!((method, &e), (Method::Solver(_), ScrapeError::NoPrice));
                if best_error.as_ref().is_none_or(|b| rank(&e) >= rank(b)) {
                    best_error = Some(e);
                }
                if final_answer {
                    break;
                }
            }
        }
    }
    Err(best_error.unwrap_or(ScrapeError::NoPrice))
}

// ─────────────────────────────────────────────
//  Safe fetching
// ─────────────────────────────────────────────

fn check_url_shape(url: &Url) -> Result<(), ScrapeError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ScrapeError::InvalidUrl("only http:// and https:// URLs are supported".into()));
    }
    if url.host().is_none() {
        return Err(ScrapeError::InvalidUrl("missing host".into()));
    }
    if !matches!(url.port(), None | Some(80) | Some(443)) {
        return Err(ScrapeError::Blocked("non-standard port".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ScrapeError::Blocked("credentials in URL".into()));
    }
    Ok(())
}

/// Resolve the URL's host and make sure every address is publicly routable.
async fn resolve_public(url: &Url) -> Result<Vec<SocketAddr>, ScrapeError> {
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(Host::Ipv4(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::new(ip.into(), port)],
        Some(Host::Domain(domain)) => tokio::net::lookup_host((domain, port))
            .await
            .map_err(|e| ScrapeError::Network(format!("DNS lookup failed: {e}")))?
            .collect(),
        None => return Err(ScrapeError::InvalidUrl("missing host".into())),
    };

    if addrs.is_empty() {
        return Err(ScrapeError::Network("host has no addresses".into()));
    }
    if let Some(bad) = addrs.iter().find(|a| !is_public_ip(a.ip())) {
        return Err(ScrapeError::Blocked(format!("{} is not a public address", bad.ip())));
    }
    Ok(addrs)
}

/// Returns the page body and the final URL after redirects.
async fn fetch_html(mut url: Url) -> Result<(String, Url), ScrapeError> {
    // Shared across redirect hops: many shops set a cookie and redirect back.
    let cookies = std::sync::Arc::new(reqwest::cookie::Jar::default());
    for _ in 0..=MAX_REDIRECTS {
        check_url_shape(&url)?;
        let addrs = resolve_public(&url).await?;

        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .cookie_provider(cookies.clone())
            .redirect(redirect::Policy::none());
        if let Some(proxy) = proxy() {
            let proxy = reqwest::Proxy::all(&proxy)
                .map_err(|e| ScrapeError::Network(format!("bad SCRAPER_PROXY: {e}")))?;
            builder = builder.proxy(proxy);
        } else if let Some(Host::Domain(domain)) = url.host() {
            // Pin the connection to the addresses we just vetted so a second
            // DNS lookup cannot return something different.
            builder = builder.resolve_to_addrs(domain, &addrs);
        }
        let client = builder.build().map_err(|e| ScrapeError::Network(e.to_string()))?;

        let mut response = client
            .get(url.clone())
            .header(header::ACCEPT, "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8")
            .header(header::ACCEPT_LANGUAGE, "pt-BR,pt;q=0.9,en;q=0.8")
            .send()
            .await
            .map_err(|e| ScrapeError::Network(e.without_url().to_string()))?;

        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(ScrapeError::Status(status))?;
            url = url
                .join(location)
                .map_err(|e| ScrapeError::InvalidUrl(format!("bad redirect: {e}")))?;
            continue;
        }
        if !status.is_success() {
            return Err(ScrapeError::Status(status));
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ScrapeError::Network(e.without_url().to_string()))?
        {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_BODY_BYTES {
                return Err(ScrapeError::Network("page is too large".into()));
            }
        }
        return Ok((String::from_utf8_lossy(&body).into_owned(), url));
    }

    Err(ScrapeError::Network("too many redirects".into()))
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let seg = v6.segments();
            // NAT64 (64:ff9b::/96) embeds an IPv4 address in the low 32 bits.
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [.., a, b, c, d] = v6.octets();
                return is_public_v4(Ipv4Addr::new(a, b, c, d));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg[0] & 0xffc0) == 0xfe80 // link local fe80::/10
                || (seg[0] == 0x2001 && seg[1] == 0x0db8) // documentation
                || seg[..6] == [0, 0, 0, 0, 0, 0]) // IPv4-compatible (deprecated)
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0 // "this network"
        || (a == 100 && (64..128).contains(&b)) // CGNAT / Tailscale 100.64.0.0/10
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || a >= 240) // reserved
}

// ─────────────────────────────────────────────
//  Extraction
// ─────────────────────────────────────────────

/// Extract a product from HTML, trying the most reliable sources first:
/// schema.org JSON-LD, then product meta tags, then microdata, then CSS class
/// heuristics. `host` is the page's host name, used to resolve ambiguous
/// currency symbols (`$`, `¥`, `kr`) and as a last-resort currency guess.
pub fn extract_product(html: &str, host: Option<&str>) -> Option<Product> {
    let doc = Html::parse_document(html);
    if let sites::SiteResult::Known(product) = sites::extract(&doc, host) {
        return product;
    }

    let ld = json_ld_product(&doc);
    let structured_currency = ld
        .as_ref()
        .and_then(|p| p.currency.clone())
        .or_else(|| meta_currency(&doc))
        .and_then(|c| currency::normalise_code(&c));

    let (price, text_currency) = match ld.as_ref().map(|p| p.price).or_else(|| meta_price(&doc)) {
        Some(p) => (p, None),
        None => markup_price(&doc, structured_currency.as_deref(), host)?,
    };
    let currency = structured_currency
        .or(text_currency)
        .or_else(|| currency::from_host(host));

    let name = ld
        .map(|p| p.name)
        .filter(|n| !n.is_empty())
        .or_else(|| attr_of(&doc, "meta[property='og:title']", "content"))
        .or_else(|| {
            let sel = Selector::parse("title").ok()?;
            doc.select(&sel).next().map(|el| el.text().collect::<String>())
        })
        .map(|n| clean_text(&n))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Unnamed product".into());

    Some(Product { name, price, currency })
}

fn clean_text(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_NAME_CHARS).collect()
}

fn attr_of(doc: &Html, selector: &str, attr: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    doc.select(&sel)
        .filter_map(|el| el.value().attr(attr))
        .map(str::trim)
        .find(|v| !v.is_empty())
        .map(str::to_string)
}

fn json_ld_product(doc: &Html) -> Option<Product> {
    let sel = Selector::parse("script[type='application/ld+json']").ok()?;
    doc.select(&sel).find_map(|el| {
        let text = el.text().collect::<String>();
        let value: Value = serde_json::from_str(text.trim()).ok()?;
        find_ld_product(&value, 0)
    })
}

fn find_ld_product(v: &Value, depth: usize) -> Option<Product> {
    if depth > 8 {
        return None;
    }
    match v {
        Value::Array(items) => items.iter().find_map(|i| find_ld_product(i, depth + 1)),
        Value::Object(map) => {
            let is_product = match map.get("@type") {
                Some(Value::String(t)) => t == "Product" || t == "ProductGroup",
                Some(Value::Array(ts)) => ts.iter().any(|t| t == "Product"),
                _ => false,
            };
            if let Some((price, currency)) = map.get("offers").and_then(ld_offer_price).filter(|_| is_product) {
                let name = map.get("name").and_then(Value::as_str).unwrap_or_default();
                return Some(Product { name: clean_text(name), price, currency });
            }
            map.values().find_map(|child| find_ld_product(child, depth + 1))
        }
        _ => None,
    }
}

fn ld_offer_price(offers: &Value) -> Option<(f64, Option<String>)> {
    match offers {
        Value::Array(items) => items.iter().find_map(ld_offer_price),
        Value::Object(offer) => {
            let spec = offer
                .get("priceSpecification")
                .map(|s| s.as_array().and_then(|a| a.first()).unwrap_or(s));
            let currency = offer
                .get("priceCurrency")
                .or_else(|| spec?.get("priceCurrency"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let digits = currency::minor_digits(currency.as_deref());
            let price = ["price", "lowPrice"]
                .iter()
                .find_map(|k| offer.get(*k))
                .or_else(|| spec?.get("price"))
                .and_then(|v| structured_price(v, digits))?;
            Some((price, currency))
        }
        _ => None,
    }
}

/// Price from a machine-readable field (JSON-LD / meta content), which should
/// use `.` as the decimal separator.
fn structured_price(v: &Value, minor_digits: u8) -> Option<f64> {
    let price = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .or_else(|| parse_price(s, minor_digits))?,
        _ => return None,
    };
    valid_price(price)
}

fn meta_price(doc: &Html) -> Option<f64> {
    let digits = currency::minor_digits(meta_currency(doc).as_deref());
    [
        "meta[property='product:price:amount']",
        "meta[property='og:price:amount']",
        "meta[itemprop='price']",
    ]
    .iter()
    .find_map(|sel| attr_of(doc, sel, "content"))
    .and_then(|s| structured_price(&Value::String(s), digits))
}

fn meta_currency(doc: &Html) -> Option<String> {
    [
        "meta[property='product:price:currency']",
        "meta[property='og:price:currency']",
        "[itemprop='priceCurrency']",
    ]
    .iter()
    .find_map(|sel| attr_of(doc, sel, "content"))
}

/// Price from page markup, with the currency shown next to it if any.
fn markup_price(doc: &Html, known_currency: Option<&str>, host: Option<&str>) -> Option<(f64, Option<String>)> {
    const SELECTORS: [&str; 5] = [
        "[itemprop='price']",
        "[data-price]",
        ".price",
        "#price",
        "[class*='price']",
    ];
    for sel_str in SELECTORS {
        let Ok(sel) = Selector::parse(sel_str) else {
            continue;
        };
        for el in doc.select(&sel) {
            let v = el.value();
            let text = el.text().collect::<String>();
            let shown = currency::detect(&text, host);
            let digits = currency::minor_digits(known_currency.or(shown.as_deref()));
            let found = v
                .attr("content")
                .or_else(|| v.attr("data-price"))
                .and_then(|c| structured_price(&Value::String(c.to_string()), digits))
                .or_else(|| parse_price(&text, digits));
            if let Some(price) = found {
                return Some((price, shown));
            }
        }
    }
    None
}

fn valid_price(p: f64) -> Option<f64> {
    (p.is_finite() && p > 0.0).then_some(p)
}

/// Characters used only to group thousands (`1 299,90`, `1'299.90`).
fn is_group_separator(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | '\u{202f}' | '\u{2009}' | '\'' | '\u{2019}')
}

/// Parse a human-formatted price in any locale, such as `R$ 1.299,90`,
/// `$1,299.90`, `1 299,90 €`, `CHF 1'299.90`, `₹1,29,999` or `¥1,299`.
/// Takes the first number in the text. `minor_digits` (the currency's number
/// of decimals) disambiguates `1.250`: thousands for most currencies, but a
/// decimal for three-decimal ones like KWD.
pub fn parse_price(raw: &str, minor_digits: u8) -> Option<f64> {
    let start = raw.find(|c: char| c.is_ascii_digit())?;
    let chars: Vec<char> = raw[start..].chars().collect();
    let mut token = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_digit() || c == '.' || c == ',' {
            token.push(c);
        } else if !(is_group_separator(c) && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            break;
        }
        // Grouping characters between digits are simply dropped.
    }
    let token = token.trim_end_matches(['.', ',']);

    let last_dot = token.rfind('.');
    let last_comma = token.rfind(',');
    let normalised = match (last_dot, last_comma) {
        (None, None) => token.to_string(),
        // Both present: whichever comes last is the decimal separator.
        (Some(d), Some(c)) => {
            if d > c {
                token.replace(',', "")
            } else {
                token.replace('.', "").replace(',', ".")
            }
        }
        (Some(_), None) => single_separator(token, '.', minor_digits),
        (None, Some(_)) => single_separator(token, ',', minor_digits),
    };

    valid_price(normalised.parse::<f64>().ok()?)
}

/// Only one kind of separator: it groups thousands if it appears more than
/// once, if the currency has no decimals, or if it is followed by exactly
/// three digits (unless the currency itself has three decimals); otherwise it
/// is the decimal point.
fn single_separator(token: &str, sep: char, minor_digits: u8) -> String {
    let parts: Vec<&str> = token.split(sep).collect();
    let is_thousands = parts.len() > 2
        || minor_digits == 0
        || (minor_digits != 3 && parts.last().is_some_and(|p| p.len() == 3));
    if is_thousands {
        parts.concat()
    } else {
        token.replace(sep, ".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_prices() {
        assert_eq!(parse_price("R$ 1.299,90", 2), Some(1299.90));
        assert_eq!(parse_price("$1,299.90", 2), Some(1299.90));
        assert_eq!(parse_price("€ 12,50", 2), Some(12.50));
        assert_eq!(parse_price("12.50 USD", 2), Some(12.50));
        assert_eq!(parse_price("1.299", 2), Some(1299.0));
        assert_eq!(parse_price("1,299", 2), Some(1299.0));
        assert_eq!(parse_price("1.234.567,89", 2), Some(1_234_567.89));
        assert_eq!(parse_price("1,234,567.89", 2), Some(1_234_567.89));
        assert_eq!(parse_price("Price: 42", 2), Some(42.0));
        assert_eq!(parse_price("99.", 2), Some(99.0));
        assert_eq!(parse_price("free", 2), None);
        assert_eq!(parse_price("0,00", 2), None);
    }

    #[test]
    fn parses_prices_in_any_locale() {
        assert_eq!(parse_price("1 299,90 €", 2), Some(1299.90));
        assert_eq!(parse_price("1\u{a0}299,90\u{a0}€", 2), Some(1299.90));
        assert_eq!(parse_price("1\u{202f}299,90 €", 2), Some(1299.90));
        assert_eq!(parse_price("CHF 1'299.90", 2), Some(1299.90));
        assert_eq!(parse_price("₹1,29,999.00", 2), Some(129_999.0));
        assert_eq!(parse_price("¥1,299", 0), Some(1299.0));
        assert_eq!(parse_price("¥ 12.800", 0), Some(12800.0));
        assert_eq!(parse_price("₩15,000", 0), Some(15000.0));
        assert_eq!(parse_price("1.250 KWD", 3), Some(1.25));
        assert_eq!(parse_price("12,5 zł", 2), Some(12.5));
        // A space followed by a non-digit ends the number.
        assert_eq!(parse_price("10 items for 5", 2), Some(10.0));
    }

    #[test]
    fn currency_from_markup_and_domain() {
        let html = r#"<title>X</title><span class="price">1 299,00 €</span>"#;
        let p = extract_product(html, Some("shop.fr")).unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (1299.0, Some("EUR")));

        let html = r#"<title>X</title><span class="price">$ 12.990</span>"#;
        let p = extract_product(html, Some("tienda.cl")).unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (12990.0, Some("CLP")));

        let html = r#"<title>X</title><span class="price">129,90</span>"#;
        let p = extract_product(html, Some("www.loja.com.br")).unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (129.9, Some("BRL")));

        let html = r#"<meta property="product:price:amount" content="1500"><meta property="product:price:currency" content="jpy">"#;
        let p = extract_product(html, None).unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (1500.0, Some("JPY")));
    }

    #[test]
    fn json_ld_product_wins() {
        let html = r#"<html><head><title>Shop | Thing</title>
            <script type="application/ld+json">
            {"@context":"https://schema.org","@graph":[
              {"@type":"WebSite","name":"Shop"},
              {"@type":"Product","name":"Super Thing","offers":{"@type":"Offer","price":"1299.90","priceCurrency":"BRL"}}
            ]}
            </script></head>
            <body><span class="price">R$ 5,00</span></body></html>"#;
        let p = extract_product(html, None).unwrap();
        assert_eq!(p.name, "Super Thing");
        assert_eq!(p.price, 1299.90);
        assert_eq!(p.currency.as_deref(), Some("BRL"));
    }

    #[test]
    fn json_ld_aggregate_offer_and_arrays() {
        let html = r#"<script type="application/ld+json">
            [{"@type":["Product"],"name":"Multi","offers":[{"@type":"AggregateOffer","lowPrice":19.5,"priceCurrency":"USD"}]}]
            </script>"#;
        let p = extract_product(html, None).unwrap();
        assert_eq!(p.price, 19.5);
        assert_eq!(p.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn meta_tags_then_title() {
        let html = r#"<html><head><title>  Fancy
            Lamp  </title>
            <meta property="product:price:amount" content="89.99">
            <meta property="product:price:currency" content="EUR"></head></html>"#;
        let p = extract_product(html, None).unwrap();
        assert_eq!(p.name, "Fancy Lamp");
        assert_eq!(p.price, 89.99);
        assert_eq!(p.currency.as_deref(), Some("EUR"));
    }

    #[test]
    fn microdata_and_class_fallbacks() {
        let html = r#"<title>A &amp; B</title><div itemprop="price" content="10.00">R$ 10,00</div>"#;
        let p = extract_product(html, None).unwrap();
        assert_eq!(p.name, "A & B");
        assert_eq!(p.price, 10.0);

        let html = r#"<title>X</title><p class="product-price">Por apenas R$ 2.499,00</p>"#;
        assert_eq!(extract_product(html, None).unwrap().price, 2499.0);

        let html = r#"<title>X</title><p class="price-label">Preço:</p><p class="price">R$ 7,49</p>"#;
        assert_eq!(extract_product(html, None).unwrap().price, 7.49);
    }

    #[test]
    fn no_price() {
        assert!(extract_product("<title>Nothing here</title><p>hello</p>", None).is_none());
    }

    #[test]
    fn blocks_private_addresses() {
        for ip in [
            "127.0.0.1", "10.1.2.3", "172.19.0.3", "192.168.1.1", "169.254.169.254", "100.124.239.27",
            "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} should be blocked");
        }
        for ip in ["1.1.1.1", "152.53.36.232", "2606:4700:4700::1111"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    #[test]
    fn url_shape() {
        assert!(parse_url("https://example.com/p/1").is_ok());
        assert!(parse_url("http://example.com:80/").is_ok());
        assert!(parse_url("ftp://example.com").is_err());
        assert!(parse_url("https://example.com:9090/").is_err());
        assert!(parse_url("https://user:pw@example.com/").is_err());
        assert!(parse_url("not a url").is_err());
    }

    #[tokio::test]
    async fn refuses_to_fetch_internal_hosts() {
        for url in ["http://127.0.0.1/", "http://localhost/", "http://[::1]/", "http://169.254.169.254/latest"] {
            match fetch_product(url).await {
                Err(ScrapeError::Blocked(_)) => {}
                other => panic!("{url}: expected Blocked, got {other:?}"),
            }
        }
    }
}
