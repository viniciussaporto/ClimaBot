//! Extraction rules for large shops whose pages defeat the generic heuristics
//! (many prices per page: list price, installments, related products, …).

use super::{currency, scrape::Product};
use scraper::{ElementRef, Html, Selector};

/// Result of a site rule: `Known(None)` means the page belongs to a known
/// shop but shows no current price (e.g. out of stock), in which case the
/// generic extractor must not guess from other prices on the page.
pub enum SiteResult {
    Known(Option<Product>),
    Unknown,
}

pub fn extract(doc: &Html, host: Option<&str>) -> SiteResult {
    let Some(host) = host else {
        return SiteResult::Unknown;
    };
    if host.split('.').any(|l| l == "amazon") {
        SiteResult::Known(amazon(doc, host))
    } else if host.split('.').any(|l| l == "aliexpress") {
        match aliexpress(doc, host) {
            Some(p) => SiteResult::Known(Some(p)),
            None => SiteResult::Unknown,
        }
    } else {
        SiteResult::Unknown
    }
}

fn select<'a>(doc: &'a Html, selector: &str) -> impl Iterator<Item = ElementRef<'a>> {
    let sel = Selector::parse(selector).expect("valid selector");
    doc.select(&sel).collect::<Vec<_>>().into_iter()
}

fn text(el: ElementRef<'_>) -> String {
    el.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_text(doc: &Html, selector: &str) -> Option<String> {
    select(doc, selector).map(text).find(|t| !t.is_empty())
}

// ─────────────────────────────────────────────
//  Amazon (all country sites)
// ─────────────────────────────────────────────

/// The price shown in an Amazon `.a-price` element: its screen-reader text,
/// or the visible whole + fraction parts when that is blank.
fn amazon_price_text(el: ElementRef<'_>) -> Option<String> {
    let offscreen = Selector::parse(".a-offscreen").ok()?;
    if let Some(t) = el.select(&offscreen).map(text).find(|t| t.chars().any(|c| c.is_ascii_digit())) {
        return Some(t);
    }
    let part = |s: &str| {
        let sel = Selector::parse(s).ok()?;
        el.select(&sel).next().map(|e| text(e).trim_end_matches(['.', ',']).to_string())
    };
    let symbol = part(".a-price-symbol").unwrap_or_default();
    let whole = part(".a-price-whole")?;
    let fraction = part(".a-price-fraction");
    Some(match fraction {
        // Rebuild with an unambiguous separator: grouping dots/commas removed.
        Some(f) => format!("{symbol}{}.{f}", whole.replace(['.', ','], "")),
        None => format!("{symbol}{}", whole.replace(['.', ','], "")),
    })
}

fn amazon(doc: &Html, host: &str) -> Option<Product> {
    // The main price box, most specific first. `.a-text-price` is the
    // struck-through list price, never the price to pay.
    const PRICE_BOXES: &[&str] = &[
        "#corePrice_feature_div .apex-pricetopay-value",
        "#corePriceDisplay_desktop_feature_div .priceToPay",
        "#corePriceDisplay_desktop_feature_div .apex-pricetopay-value",
        "#corePrice_desktop .apex-pricetopay-value",
        "#corePrice_desktop .a-price:not(.a-text-price)",
        "#corePrice_feature_div .a-price:not(.a-text-price)",
        "#apex_desktop .apexPriceToPay",
        "#apex_offerDisplay_desktop .apex-pricetopay-value",
        "#tp_price_block_total_price_ww",
    ];
    const LEGACY_PRICE: &[&str] = &[
        "#priceblock_dealprice",
        "#priceblock_ourprice",
        "#priceblock_saleprice",
        "#price_inside_buybox",
        "#newBuyBoxPrice",
        "#kindle-price",
    ];

    let shown = PRICE_BOXES
        .iter()
        .flat_map(|s| select(doc, s))
        .find_map(amazon_price_text)
        .or_else(|| LEGACY_PRICE.iter().find_map(|s| first_text(doc, s)));

    let currency = shown
        .as_deref()
        .and_then(|t| currency::detect(t, Some(host)))
        .or_else(|| currency::from_host(Some(host)))
        .or_else(|| (host.ends_with("amazon.com")).then(|| "USD".to_string()));
    let digits = currency::minor_digits(currency.as_deref());

    let price = match shown {
        Some(t) => super::scrape::parse_price(&t, digits),
        // Hidden form values used by the buy box, in machine format.
        None => ["#twister-plus-price-data-price", "#attach-base-product-price"]
            .iter()
            .flat_map(|s| select(doc, s))
            .filter_map(|e| e.value().attr("value")?.trim().parse::<f64>().ok())
            .find(|p| p.is_finite() && *p > 0.0),
    }?;

    let name = first_text(doc, "#productTitle")
        .or_else(|| first_text(doc, "title"))
        .unwrap_or_else(|| "Amazon product".into());
    Some(Product { name, price, currency })
}

// ─────────────────────────────────────────────
//  AliExpress
// ─────────────────────────────────────────────

fn aliexpress(doc: &Html, host: &str) -> Option<Product> {
    const PRICE: &[&str] = &[
        "[class*='price-default--current']",
        "[class*='price--currentPriceText']",
        "[class*='product-price-current']",
        ".product-price-value",
        "[class*='price-kr--current']",
    ];
    let shown = PRICE.iter().find_map(|s| first_text(doc, s))?;
    let currency = currency::detect(&shown, Some(host));
    let price = super::scrape::parse_price(&shown, currency::minor_digits(currency.as_deref()))?;
    let name = first_text(doc, "h1[data-pl='product-title']")
        .or_else(|| first_text(doc, "h1"))
        .or_else(|| first_text(doc, "title"))
        .unwrap_or_else(|| "AliExpress product".into());
    Some(Product { name, price, currency })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(html: &str, host: &str) -> Option<Product> {
        match extract(&Html::parse_document(html), Some(host)) {
            SiteResult::Known(p) => p,
            SiteResult::Unknown => panic!("{host} should be a known site"),
        }
    }

    #[test]
    fn amazon_price_to_pay_not_list_price() {
        let html = r#"
            <span id="productTitle"> Apple iPhone 15 </span>
            <div id="corePriceDisplay_desktop_feature_div">
              <span class="a-price a-text-price"><span class="a-offscreen">R$5.999,00</span></span>
            </div>
            <div id="corePrice_feature_div">
              <span class="a-price apex-pricetopay-value"><span class="a-offscreen">R$3.999,00</span></span>
              em até 12x de R$ 398,12
            </div>
            <input id="twister-plus-price-data-price" value="4776.67">"#;
        let p = run(html, "www.amazon.com.br").unwrap();
        assert_eq!((p.name.as_str(), p.price, p.currency.as_deref()), ("Apple iPhone 15", 3999.0, Some("BRL")));
    }

    #[test]
    fn amazon_whole_and_fraction() {
        let html = r#"<div id="corePrice_desktop"><span class="a-price">
            <span class="a-offscreen"> </span><span class="a-price-symbol">$</span>
            <span class="a-price-whole">1,299<span class="a-price-decimal">.</span></span>
            <span class="a-price-fraction">99</span></span></div>"#;
        let p = run(html, "www.amazon.com").unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (1299.99, Some("USD")));

        let html = r#"<div id="corePrice_desktop"><span class="a-price">
            <span class="a-price-symbol">¥</span><span class="a-price-whole">12,800</span></span></div>"#;
        let p = run(html, "www.amazon.co.jp").unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (12800.0, Some("JPY")));
    }

    #[test]
    fn amazon_unavailable_is_not_guessed() {
        // Empty buy box, but other prices elsewhere on the page.
        let html = r#"<div id="corePrice_desktop"><div class="a-section"></div></div>
            <div class="a-carousel"><span class="a-price"><span class="a-offscreen">$4,117.00</span></span></div>"#;
        assert!(run(html, "www.amazon.com").is_none());
    }

    #[test]
    fn amazon_hidden_value_fallback() {
        let html = r#"<span id="productTitle">Book</span><input id="attach-base-product-price" value="42.5">"#;
        let p = run(html, "www.amazon.de").unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (42.5, Some("EUR")));
    }

    #[test]
    fn aliexpress_current_price() {
        let html = r#"<h1 data-pl="product-title">Pliers</h1>
            <div class="price-default--wrap--x"><span class="price-default--current--F8OlYIo">$5.85</span>
            <span class="price-default--original--y">$11.70</span></div>"#;
        let p = run(html, "www.aliexpress.us").unwrap();
        assert_eq!((p.name.as_str(), p.price, p.currency.as_deref()), ("Pliers", 5.85, Some("USD")));

        let html = r#"<h1>Alicate</h1><span class="price-default--current--a">R$ 32,90</span>"#;
        let p = run(html, "pt.aliexpress.com").unwrap();
        assert_eq!((p.price, p.currency.as_deref()), (32.9, Some("BRL")));
    }
}
