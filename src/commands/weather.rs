use crate::metrics;
use anyhow::{Context as _, Result, anyhow};
use chrono::{Datelike, NaiveDate, Weekday};
use serde::Deserialize;
use serenity::all::{CreateAttachment, CreateEmbed, CreateEmbedFooter};
use std::{env, sync::LazyLock, time::Duration};
use tracing::{debug, info, warn};

const OPEN_METEO_BASE: &str = "https://api.open-meteo.com/v1/forecast";
const OPENCAGE_BASE: &str = "https://api.opencagedata.com/geocode/v1/json";
const EMBED_COLOR: u32 = 0x0099ff;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("ClimaBot/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build HTTP client")
});

/// Error returned when geocoding finds nothing for the query, so the command
/// can tell the user instead of reporting a generic failure.
#[derive(Debug)]
pub struct LocationNotFound(pub String);

impl std::fmt::Display for LocationNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "location not found: {}", self.0)
    }
}

impl std::error::Error for LocationNotFound {}

// ─────────────────────────────────────────────
//  OpenCage geocoding response
// ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GeocodingResponse {
    #[serde(default)]
    results: Vec<GeocodingResult>,
}

#[derive(Debug, Deserialize)]
struct GeocodingResult {
    formatted: String,
    geometry: Geometry,
}

#[derive(Debug, Deserialize)]
struct Geometry {
    lat: f64,
    lng: f64,
}

// ─────────────────────────────────────────────
//  Open-Meteo current weather response
// ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WeatherResponse {
    current: CurrentWeather,
}

#[derive(Debug, Deserialize)]
pub struct CurrentWeather {
    pub temperature_2m: f64,
    pub apparent_temperature: f64,
    pub relativehumidity_2m: f64,
    pub weathercode: i32,
    pub pressure_msl: f64,
    pub cloudcover: f64,
    pub windspeed_10m: f64,
    pub winddirection_10m: f64,
}

// ─────────────────────────────────────────────
//  Open-Meteo 5-day forecast response
// ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    daily: ForecastDaily,
}

#[derive(Debug, Deserialize)]
pub struct ForecastDaily {
    pub time: Vec<String>,
    pub temperature_2m_max: Vec<Option<f64>>,
    pub temperature_2m_min: Vec<Option<f64>>,
    pub precipitation_probability_max: Vec<Option<f64>>,
}

// ─────────────────────────────────────────────
//  API calls
// ─────────────────────────────────────────────

pub struct Location {
    pub lat: f64,
    pub lng: f64,
    pub formatted: String,
}

/// Resolve a free-text location string into lat/lng via OpenCage.
pub async fn get_coordinates(location: &str) -> Result<Location> {
    let api_key = env::var("OPENCAGEAPIKEY").map_err(|_| anyhow!("OPENCAGEAPIKEY env var not set"))?;
    let api_key = api_key.trim();

    debug!(location, "Geocoding location");

    let result = HTTP
        .get(OPENCAGE_BASE)
        .query(&[("key", api_key), ("q", location), ("no_annotations", "1"), ("limit", "1")])
        .send()
        .await
        .and_then(|r| r.error_for_status());
    let response: GeocodingResponse = match result {
        Ok(r) => r.json().await.context("invalid OpenCage response")?,
        Err(e) => {
            metrics::WEATHER_API_COUNTER.with_label_values(&["geocoding", "error"]).inc();
            return Err(anyhow!(e).context("OpenCage request failed"));
        }
    };
    metrics::WEATHER_API_COUNTER.with_label_values(&["geocoding", "success"]).inc();

    let result = response
        .results
        .into_iter()
        .next()
        .ok_or_else(|| LocationNotFound(location.to_string()))?;

    info!(location, resolved = %result.formatted, "Resolved coordinates");

    Ok(Location {
        lat: result.geometry.lat,
        lng: result.geometry.lng,
        formatted: result.formatted,
    })
}

async fn open_meteo<T: serde::de::DeserializeOwned>(kind: &str, params: &[(&str, String)]) -> Result<T> {
    let result = async {
        HTTP.get(OPEN_METEO_BASE)
            .query(params)
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await
    }
    .await;

    let status = if result.is_ok() { "success" } else { "error" };
    metrics::WEATHER_API_COUNTER.with_label_values(&[kind, status]).inc();
    result.with_context(|| format!("Open-Meteo {kind} request failed"))
}

/// Fetch current conditions for a coordinate.
pub async fn get_current(loc: &Location) -> Result<CurrentWeather> {
    let params = [
        ("latitude", loc.lat.to_string()),
        ("longitude", loc.lng.to_string()),
        (
            "current",
            "temperature_2m,apparent_temperature,relativehumidity_2m,weathercode,\
             pressure_msl,cloudcover,windspeed_10m,winddirection_10m"
                .to_string(),
        ),
        ("forecast_days", "1".to_string()),
        ("timezone", "auto".to_string()),
    ];
    let response: WeatherResponse = open_meteo("current", &params).await?;
    Ok(response.current)
}

/// Fetch a 5-day daily forecast for a coordinate.
pub async fn get_forecast(loc: &Location) -> Result<ForecastDaily> {
    let params = [
        ("latitude", loc.lat.to_string()),
        ("longitude", loc.lng.to_string()),
        (
            "daily",
            "temperature_2m_max,temperature_2m_min,precipitation_probability_max".to_string(),
        ),
        ("forecast_days", "5".to_string()),
        ("timezone", "auto".to_string()),
    ];
    let response: ForecastResponse = open_meteo("forecast", &params).await?;
    Ok(response.daily)
}

// ─────────────────────────────────────────────
//  Discord message builders
// ─────────────────────────────────────────────

/// Build the `/weather` embed and its weather-icon attachment.
pub fn weather_embed(loc: &Location, c: &CurrentWeather) -> (CreateEmbed, CreateAttachment) {
    if c.temperature_2m == 0.0 && c.weathercode == 0 {
        warn!(location = %loc.formatted, "Suspicious weather data");
    }

    let attachment = CreateAttachment::bytes(weather_image(c.weathercode), "weather.png");

    let embed = CreateEmbed::new()
        .title(format!("{} Weather in {}", weather_emoji(c.weathercode), loc.formatted))
        .description(format!("**{}**", weather_description(c.weathercode)))
        .field(
            "\u{200b}",
            format!(
                "🌡 **Temperature:** {:.1}°C\n\
                 🌡️ **Feels Like:** {:.1}°C\n\
                 💧 **Humidity:** {:.0}%\n\
                 ☁ **Clouds:** {:.0}%",
                c.temperature_2m, c.apparent_temperature, c.relativehumidity_2m, c.cloudcover,
            ),
            true,
        )
        .field(
            "\u{200b}",
            format!(
                "🌬 **Wind:** {:.1} km/h\n\
                 🧭 **Direction:** {:.0}° {}\n\
                 📊 **Pressure:** {:.0} hPa",
                c.windspeed_10m,
                c.winddirection_10m,
                compass_point(c.winddirection_10m),
                c.pressure_msl,
            ),
            true,
        )
        .color(EMBED_COLOR)
        .thumbnail("attachment://weather.png")
        .footer(CreateEmbedFooter::new("Open-Meteo · OpenCage"));

    (embed, attachment)
}

/// Build the `/forecast` embed.
pub fn forecast_embed(loc: &Location, d: &ForecastDaily) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .title(format!("🌦 Previsão para 5 dias - {}", loc.formatted))
        .color(EMBED_COLOR);

    let fmt_temp = |v: Option<&Option<f64>>| match v.copied().flatten() {
        Some(t) => format!("{t:.1}°C"),
        None => "–".into(),
    };

    for (i, day) in d.time.iter().enumerate() {
        let rain = match d.precipitation_probability_max.get(i).copied().flatten() {
            Some(p) => format!("{p:.0}%"),
            None => "–".into(),
        };
        embed = embed.field(
            format!("📅 {}", format_date_pt_br(day)),
            format!(
                "⬆ {} ⬇ {}\n💧 Prob. Chuva: {rain}",
                fmt_temp(d.temperature_2m_max.get(i)),
                fmt_temp(d.temperature_2m_min.get(i)),
            ),
            true,
        );
    }

    embed
}

/// Format an ISO date like `2026-09-29` as `ter., 29/09`, matching the
/// `pt-BR` `toLocaleDateString` output of the previous bot.
fn format_date_pt_br(iso: &str) -> String {
    let Ok(date) = NaiveDate::parse_from_str(iso, "%Y-%m-%d") else {
        return iso.to_string();
    };
    let weekday = match date.weekday() {
        Weekday::Mon => "seg.",
        Weekday::Tue => "ter.",
        Weekday::Wed => "qua.",
        Weekday::Thu => "qui.",
        Weekday::Fri => "sex.",
        Weekday::Sat => "sáb.",
        Weekday::Sun => "dom.",
    };
    format!("{weekday}, {}", date.format("%d/%m"))
}

fn compass_point(degrees: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let idx = ((degrees.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8;
    POINTS[idx]
}

// ─────────────────────────────────────────────
//  WMO weather code helpers
//  https://open-meteo.com/en/docs#weathervariables
// ─────────────────────────────────────────────

pub fn weather_description(code: i32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Depositing rime fog",
        51 => "Light drizzle",
        53 => "Moderate drizzle",
        55 => "Dense drizzle",
        56 => "Light freezing drizzle",
        57 => "Dense freezing drizzle",
        61 => "Slight rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        66 => "Light freezing rain",
        67 => "Heavy freezing rain",
        71 => "Slight snowfall",
        73 => "Moderate snowfall",
        75 => "Heavy snowfall",
        77 => "Snow grains",
        80 => "Slight rain showers",
        81 => "Moderate rain showers",
        82 => "Violent rain showers",
        85 => "Slight snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 => "Thunderstorm with slight hail",
        99 => "Thunderstorm with heavy hail",
        _ => "Unknown",
    }
}

fn weather_emoji(code: i32) -> &'static str {
    match code {
        0 => "☀️",
        1 | 2 => "⛅",
        3 => "☁️",
        45 | 48 => "🌫️",
        51..=57 => "🌦️",
        61..=67 | 80..=82 => "🌧️",
        71..=77 | 85 | 86 => "🌨️",
        95..=99 => "⛈️",
        _ => "🌡️",
    }
}

/// Weather icon PNG, embedded in the binary so the image needs no asset files.
fn weather_image(code: i32) -> &'static [u8] {
    macro_rules! icon {
        ($name:literal) => {
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/images/icons8-", $name, "-48.png"))
        };
    }
    match code {
        0 => icon!("sun"),
        1 | 2 => icon!("partly-cloudy-day"),
        3 => icon!("cloud"),
        45 => icon!("fog"),
        48 => icon!("haze"),
        51 | 61 | 80 => icon!("light-rain"),
        53 | 63 | 81 => icon!("moderate-rain"),
        55 => icon!("rain"),
        65 => icon!("heavy-rain"),
        82 => icon!("torrential-rain"),
        56 | 57 | 66 | 67 => icon!("sleet"),
        71 | 85 => icon!("light-snow"),
        73 | 77 => icon!("snow"),
        75 | 86 => icon!("snow-storm"),
        95 => icon!("storm"),
        96 | 99 => icon!("hail"),
        _ => icon!("puzzled"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pt_br_dates() {
        assert_eq!(format_date_pt_br("2026-09-29"), "ter., 29/09");
        assert_eq!(format_date_pt_br("2026-10-04"), "dom., 04/10");
        assert_eq!(format_date_pt_br("garbage"), "garbage");
    }

    #[test]
    fn compass() {
        assert_eq!(compass_point(0.0), "N");
        assert_eq!(compass_point(359.0), "N");
        assert_eq!(compass_point(90.0), "E");
        assert_eq!(compass_point(200.0), "S");
        assert_eq!(compass_point(300.0), "NW");
    }

    #[test]
    fn every_documented_code_has_description_and_icon() {
        let codes = [
            0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82,
            85, 86, 95, 96, 99,
        ];
        let unknown = weather_image(-1);
        for code in codes {
            assert_ne!(weather_description(code), "Unknown", "code {code}");
            assert_ne!(weather_image(code).as_ptr(), unknown.as_ptr(), "code {code}");
        }
    }

    #[test]
    fn forecast_deserializes_nulls() {
        let json = r#"{"daily":{"time":["2026-09-29"],"temperature_2m_max":[null],
            "temperature_2m_min":[12.5],"precipitation_probability_max":[null]}}"#;
        let r: ForecastResponse = serde_json::from_str(json).unwrap();
        let loc = Location { lat: 0.0, lng: 0.0, formatted: "X".into() };
        let _ = forecast_embed(&loc, &r.daily);
        assert_eq!(r.daily.temperature_2m_min[0], Some(12.5));
    }
}
