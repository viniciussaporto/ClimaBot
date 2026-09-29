//! ISO 4217 currencies: detection from page text and formatting.

/// Active ISO 4217 codes with their number of minor-unit digits.
const CURRENCIES: &[(&str, u8)] = &[
    ("AED", 2), ("AFN", 2), ("ALL", 2), ("AMD", 2), ("ANG", 2), ("AOA", 2), ("ARS", 2), ("AUD", 2),
    ("AWG", 2), ("AZN", 2), ("BAM", 2), ("BBD", 2), ("BDT", 2), ("BGN", 2), ("BHD", 3), ("BIF", 0),
    ("BMD", 2), ("BND", 2), ("BOB", 2), ("BRL", 2), ("BSD", 2), ("BTN", 2), ("BWP", 2), ("BYN", 2),
    ("BZD", 2), ("CAD", 2), ("CDF", 2), ("CHF", 2), ("CLF", 4), ("CLP", 0), ("CNY", 2), ("COP", 2),
    ("CRC", 2), ("CUP", 2), ("CVE", 2), ("CZK", 2), ("DJF", 0), ("DKK", 2), ("DOP", 2), ("DZD", 2),
    ("EGP", 2), ("ERN", 2), ("ETB", 2), ("EUR", 2), ("FJD", 2), ("FKP", 2), ("GBP", 2), ("GEL", 2),
    ("GHS", 2), ("GIP", 2), ("GMD", 2), ("GNF", 0), ("GTQ", 2), ("GYD", 2), ("HKD", 2), ("HNL", 2),
    ("HTG", 2), ("HUF", 2), ("IDR", 2), ("ILS", 2), ("INR", 2), ("IQD", 3), ("IRR", 2), ("ISK", 0),
    ("JMD", 2), ("JOD", 3), ("JPY", 0), ("KES", 2), ("KGS", 2), ("KHR", 2), ("KMF", 0), ("KPW", 2),
    ("KRW", 0), ("KWD", 3), ("KYD", 2), ("KZT", 2), ("LAK", 2), ("LBP", 2), ("LKR", 2), ("LRD", 2),
    ("LSL", 2), ("LYD", 3), ("MAD", 2), ("MDL", 2), ("MGA", 2), ("MKD", 2), ("MMK", 2), ("MNT", 2),
    ("MOP", 2), ("MRU", 2), ("MUR", 2), ("MVR", 2), ("MWK", 2), ("MXN", 2), ("MYR", 2), ("MZN", 2),
    ("NAD", 2), ("NGN", 2), ("NIO", 2), ("NOK", 2), ("NPR", 2), ("NZD", 2), ("OMR", 3), ("PAB", 2),
    ("PEN", 2), ("PGK", 2), ("PHP", 2), ("PKR", 2), ("PLN", 2), ("PYG", 0), ("QAR", 2), ("RON", 2),
    ("RSD", 2), ("RUB", 2), ("RWF", 0), ("SAR", 2), ("SBD", 2), ("SCR", 2), ("SDG", 2), ("SEK", 2),
    ("SGD", 2), ("SHP", 2), ("SLE", 2), ("SOS", 2), ("SRD", 2), ("SSP", 2), ("STN", 2), ("SVC", 2),
    ("SYP", 2), ("SZL", 2), ("THB", 2), ("TJS", 2), ("TMT", 2), ("TND", 3), ("TOP", 2), ("TRY", 2),
    ("TTD", 2), ("TWD", 2), ("TZS", 2), ("UAH", 2), ("UGX", 0), ("USD", 2), ("UYU", 2), ("UZS", 2),
    ("VES", 2), ("VND", 0), ("VUV", 0), ("WST", 2), ("XAF", 0), ("XCD", 2), ("XOF", 0), ("XPF", 0),
    ("YER", 2), ("ZAR", 2), ("ZMW", 2), ("ZWG", 2),
];

/// Codes that are also common English words; only trusted from structured data.
const WORDLIKE_CODES: &[&str] = &["ALL", "TOP", "CUP", "MOP", "BOB", "SOS", "PEN", "GEL", "MAD", "NAD", "LAK"];

/// Unambiguous symbols, longest first so `R$` wins over `$`.
const SYMBOLS: &[(&str, &str)] = &[
    ("COL$", "COP"), ("US$", "USD"), ("AU$", "AUD"), ("CA$", "CAD"), ("NZ$", "NZD"), ("HK$", "HKD"),
    ("SG$", "SGD"), ("MX$", "MXN"), ("NT$", "TWD"), ("RD$", "DOP"), ("CN¥", "CNY"), ("JP¥", "JPY"),
    ("R$", "BRL"), ("A$", "AUD"), ("C$", "CAD"), ("S$", "SGD"), ("S/", "PEN"), ("zł", "PLN"),
    ("Kč", "CZK"), ("Ft", "HUF"), ("лв", "BGN"), ("руб", "RUB"), ("€", "EUR"), ("£", "GBP"),
    ("₹", "INR"), ("₩", "KRW"), ("₽", "RUB"), ("₺", "TRY"), ("₴", "UAH"), ("₪", "ILS"), ("₫", "VND"),
    ("฿", "THB"), ("₱", "PHP"), ("₦", "NGN"), ("₲", "PYG"), ("₡", "CRC"), ("₸", "KZT"), ("₼", "AZN"),
    ("₾", "GEL"), ("৳", "BDT"), ("₮", "MNT"), ("₭", "LAK"), ("₵", "GHS"),
];

/// Currency used by a country-code top-level domain.
fn tld_currency(tld: &str) -> Option<&'static str> {
    Some(match tld {
        "br" => "BRL",
        "us" => "USD",
        "uk" | "gb" => "GBP",
        "ca" => "CAD",
        "au" => "AUD",
        "nz" => "NZD",
        "mx" => "MXN",
        "ar" => "ARS",
        "cl" => "CLP",
        "co" => "COP",
        "pe" => "PEN",
        "uy" => "UYU",
        "py" => "PYG",
        "bo" => "BOB",
        "jp" => "JPY",
        "cn" => "CNY",
        "hk" => "HKD",
        "tw" => "TWD",
        "kr" => "KRW",
        "in" => "INR",
        "sg" => "SGD",
        "my" => "MYR",
        "th" => "THB",
        "vn" => "VND",
        "ph" => "PHP",
        "id" => "IDR",
        "ch" | "li" => "CHF",
        "se" => "SEK",
        "no" => "NOK",
        "dk" => "DKK",
        "is" => "ISK",
        "pl" => "PLN",
        "cz" => "CZK",
        "hu" => "HUF",
        "ro" => "RON",
        "bg" => "BGN",
        "rs" => "RSD",
        "ru" => "RUB",
        "ua" => "UAH",
        "tr" => "TRY",
        "il" => "ILS",
        "ae" => "AED",
        "sa" => "SAR",
        "eg" => "EGP",
        "za" => "ZAR",
        "ng" => "NGN",
        "ke" => "KES",
        "de" | "fr" | "es" | "it" | "pt" | "nl" | "be" | "at" | "ie" | "fi" | "gr" | "sk" | "si"
        | "ee" | "lv" | "lt" | "lu" | "mt" | "cy" | "hr" | "eu" => "EUR",
        _ => return None,
    })
}

fn host_tld(host: Option<&str>) -> Option<&str> {
    host?.trim_end_matches('.').rsplit('.').next()
}

/// Normalise a currency code from structured data (`"brl"` → `"BRL"`).
pub fn normalise_code(raw: &str) -> Option<String> {
    let code = raw.trim().to_ascii_uppercase();
    (code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase())).then_some(code)
}

/// Number of decimal digits the currency uses (2 when unknown).
pub fn minor_digits(code: Option<&str>) -> u8 {
    code.and_then(|c| CURRENCIES.iter().find(|(k, _)| *k == c))
        .map_or(2, |(_, d)| *d)
}

/// Detect the currency of a price string such as `R$ 10,00`, `10 EUR` or
/// `$5`. Ambiguous symbols (`$`, `¥`, `kr`) are resolved using the site's
/// country domain.
pub fn detect(text: &str, host: Option<&str>) -> Option<String> {
    let tld = host_tld(host);

    // Explicit ISO code as a standalone uppercase word.
    let code = text
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|w| {
            w.len() == 3
                && !WORDLIKE_CODES.contains(w)
                && CURRENCIES.iter().any(|(k, _)| k == w)
        });
    if let Some(code) = code {
        return Some(code.to_string());
    }

    if let Some((_, code)) = SYMBOLS.iter().find(|(sym, _)| text.contains(sym)) {
        return Some((*code).to_string());
    }

    let from_tld = |allowed: &[&str], default: Option<&'static str>| {
        tld.and_then(tld_currency)
            .filter(|c| allowed.contains(c))
            .or(default)
            .map(str::to_string)
    };
    if text.contains('$') {
        return from_tld(
            &["USD", "CAD", "AUD", "NZD", "MXN", "ARS", "CLP", "COP", "UYU", "HKD", "SGD", "TWD"],
            Some("USD"),
        );
    }
    if text.contains('¥') || text.contains('￥') || text.contains('円') {
        return from_tld(&["CNY"], Some("JPY"));
    }
    if text.contains("元") {
        return Some("CNY".into());
    }
    if text.to_lowercase().split(|c: char| !c.is_alphabetic()).any(|w| w == "kr") {
        return from_tld(&["SEK", "NOK", "DKK", "ISK"], None);
    }
    None
}

/// Fallback when the page shows no currency at all: the site's country.
pub fn from_host(host: Option<&str>) -> Option<String> {
    host_tld(host).and_then(tld_currency).map(str::to_string)
}

fn display_prefix(code: &str) -> Option<&'static str> {
    Some(match code {
        "USD" => "US$",
        "EUR" => "€",
        "GBP" => "£",
        "BRL" => "R$",
        "JPY" => "¥",
        "CNY" => "CN¥",
        "INR" => "₹",
        "KRW" => "₩",
        "RUB" => "₽",
        "TRY" => "₺",
        "UAH" => "₴",
        "ILS" => "₪",
        "VND" => "₫",
        "THB" => "฿",
        "PHP" => "₱",
        "NGN" => "₦",
        "AUD" => "A$",
        "CAD" => "CA$",
        "NZD" => "NZ$",
        "HKD" => "HK$",
        "SGD" => "S$",
        "MXN" => "MX$",
        "TWD" => "NT$",
        _ => return None,
    })
}

/// Format a price with the currency's symbol (or code) and decimals, e.g.
/// `R$ 1,299.90`, `¥ 1,299`, `KWD 1.250`, `SEK 99.00`.
pub fn format(price: f64, code: Option<&str>) -> String {
    let digits = usize::from(minor_digits(code));
    let fixed = format!("{:.*}", digits, price.abs());
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));

    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if price < 0.0 { "-" } else { "" };
    let amount = if frac.is_empty() { format!("{sign}{grouped}") } else { format!("{sign}{grouped}.{frac}") };

    match code {
        Some(c) => format!("{} {amount}", display_prefix(c).unwrap_or(c)),
        None => amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_currencies() {
        let d = |t: &str, h: Option<&str>| detect(t, h);
        assert_eq!(d("R$ 1.299,90", None).as_deref(), Some("BRL"));
        assert_eq!(d("1 299,90 €", None).as_deref(), Some("EUR"));
        assert_eq!(d("£5", None).as_deref(), Some("GBP"));
        assert_eq!(d("99.00 CHF", None).as_deref(), Some("CHF"));
        assert_eq!(d("$10", Some("www.amazon.com")).as_deref(), Some("USD"));
        assert_eq!(d("$10", Some("www.amazon.com.mx")).as_deref(), Some("MXN"));
        assert_eq!(d("$ 10.990", Some("tienda.cl")).as_deref(), Some("CLP"));
        assert_eq!(d("¥1,299", Some("shop.jp")).as_deref(), Some("JPY"));
        assert_eq!(d("¥1,299", Some("shop.cn")).as_deref(), Some("CNY"));
        assert_eq!(d("499 kr", Some("shop.se")).as_deref(), Some("SEK"));
        assert_eq!(d("499 kr", Some("shop.no")).as_deref(), Some("NOK"));
        assert_eq!(d("₹ 1,29,999", None).as_deref(), Some("INR"));
        assert_eq!(d("12,50 zł", None).as_deref(), Some("PLN"));
        assert_eq!(d("ALL SALE 10", None), None);
        assert_eq!(d("10.00", None), None);
        assert_eq!(from_host(Some("www.kabum.com.br")).as_deref(), Some("BRL"));
        assert_eq!(from_host(Some("example.com")), None);
    }

    #[test]
    fn formats() {
        assert_eq!(format(1299.9, Some("BRL")), "R$ 1,299.90");
        assert_eq!(format(1299.0, Some("JPY")), "¥ 1,299");
        assert_eq!(format(1.25, Some("KWD")), "KWD 1.250");
        assert_eq!(format(99.0, Some("SEK")), "SEK 99.00");
        assert_eq!(format(1234567.891, None), "1,234,567.89");
        assert_eq!(format(5.0, Some("XYZ")), "XYZ 5.00");
    }

    #[test]
    fn minor_units() {
        assert_eq!(minor_digits(Some("JPY")), 0);
        assert_eq!(minor_digits(Some("KWD")), 3);
        assert_eq!(minor_digits(Some("BRL")), 2);
        assert_eq!(minor_digits(None), 2);
        assert_eq!(normalise_code(" brl ").as_deref(), Some("BRL"));
        assert_eq!(normalise_code("R$"), None);
    }
}
