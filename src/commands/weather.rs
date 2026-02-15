use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use std::env;
use tracing::{debug, info, warn};

const OPEN_METEO_BASE: &str = "https://api.open-meteo.com/v1/forecast";

// ─────────────────────────────────────────────
//  OpenCage geocoding response
// ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GeocodingResponse {
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
struct CurrentWeather {
    temperature_2m: f64,
    apparent_temperature: f64,
    relativehumidity_2m: f64,
    weathercode: i32,
    pressure_msl: f64,
    cloudcover: f64,
    windspeed_10m: f64,
    winddirection_10m: f64,
}

// ─────────────────────────────────────────────
//  Open-Meteo 5-day forecast response
// ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    daily: ForecastDaily,
}

#[derive(Debug, Deserialize)]
struct ForecastDaily {
    time: Vec<String>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    precipitation_probability_max: Vec<u32>,
}

// ─────────────────────────────────────────────
//  Shared location value
// ─────────────────────────────────────────────

pub struct Location {
    pub lat: f64,
    pub lng: f64,
    pub formatted: String,
}

// ─────────────────────────────────────────────
//  Public command handlers
// ─────────────────────────────────────────────

/// Resolve a free-text location string into lat/lng via OpenCage.
pub async fn get_coordinates(location: &str) -> Result<Location> {
    let api_key = env::var("OPENCAGEAPIKEY")
        .map_err(|_| anyhow!("OPENCAGEAPIKEY env var not set"))?;

    debug!("Geocoding location: {location}");

    let url = format!(
        "https://api.opencagedata.com/geocode/v1/json\
         ?key={api_key}&q={location}&pretty=1&no_annotations=1"
    );

    let response: GeocodingResponse = Client::new().get(&url).send().await?.json().await?;

    let result = response
        .results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("Location not found: {location}"))?;

    info!("Resolved '{}' → {}", location, result.formatted);

    Ok(Location {
        lat: result.geometry.lat,
        lng: result.geometry.lng,
        formatted: result.formatted,
    })
}

/// Fetch current weather and return a formatted Markdown string.
pub async fn get_weather(location: &str) -> Result<String> {
    let coords = get_coordinates(location).await?;

    let url = format!(
        "{OPEN_METEO_BASE}?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,relativehumidity_2m,\
         weathercode,pressure_msl,cloudcover,windspeed_10m,winddirection_10m\
         &forecast_days=1&timezone=auto",
        coords.lat, coords.lng
    );

    let response: WeatherResponse = Client::new().get(&url).send().await?.json().await?;
    let c = &response.current;

    if c.temperature_2m == 0.0 && c.weathercode == 0 {
        warn!("Suspicious weather data for {}", coords.formatted);
    }

    let description = weather_description(c.weathercode);
    let emoji = weather_emoji(c.weathercode);

    let msg = format!(
        "## {emoji} Weather in {}\n\
        **{description}**\n\n\
        🌡 **Temperature:** {:.1}°C\n\
        🌡️ **Feels Like:** {:.1}°C\n\
        💧 **Humidity:** {}%\n\
        ☁ **Cloud Cover:** {}%\n\
        🌬 **Wind Speed:** {:.1} km/h\n\
        🧭 **Wind Direction:** {}°\n\
        📊 **Pressure:** {} hPa",
        coords.formatted,
        c.temperature_2m,
        c.apparent_temperature,
        c.relativehumidity_2m as u32,
        c.cloudcover as u32,
        c.windspeed_10m,
        c.winddirection_10m as u32,
        c.pressure_msl as u32,
    );

    info!("Weather delivered for {}", coords.formatted);
    Ok(msg)
}

/// Fetch a 5-day daily forecast and return a formatted Markdown string.
pub async fn get_forecast(location: &str) -> Result<String> {
    let coords = get_coordinates(location).await?;

    let url = format!(
        "{OPEN_METEO_BASE}?latitude={}&longitude={}\
         &daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max\
         &timezone=auto&forecast_days=5",
        coords.lat, coords.lng
    );

    let response: ForecastResponse = Client::new().get(&url).send().await?.json().await?;
    let d = &response.daily;

    let mut msg = format!("## 🌦 5-Day Forecast — {}\n\n", coords.formatted);

    for (i, day) in d.time.iter().enumerate() {
        let max = d.temperature_2m_max.get(i).copied().unwrap_or(0.0);
        let min = d.temperature_2m_min.get(i).copied().unwrap_or(0.0);
        let rain = d.precipitation_probability_max.get(i).copied().unwrap_or(0);

        msg.push_str(&format!(
            "📅 **{day}**  ⬆ {max:.1}°C  ⬇ {min:.1}°C  |  💧 Rain prob.: {rain}%\n"
        ));
    }

    info!("Forecast delivered for {}", coords.formatted);
    Ok(msg)
}

// ─────────────────────────────────────────────
//  WMO weather code helpers
// ─────────────────────────────────────────────

fn weather_description(code: i32) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        46 => "Depositing rime fog",
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
        45 | 46 => "🌫️",
        51..=57 => "🌦️",
        61..=67 => "🌧️",
        71..=77 => "❄️",
        80..=82 => "🌧️",
        85 | 86 => "🌨️",
        95..=99 => "⛈️",
        _ => "🌡️",
    }
}
