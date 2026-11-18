//! Animated precipitation map for `/weather`: OpenStreetMap tiles, darkened,
//! with Open-Meteo precipitation drawn over them, one frame for now and one for each
//! of the next three hours.
//!
//! No free provider serves forecast radar tiles, so the overlay is built from
//! model data: precipitation and weather code are sampled on a grid of points
//! across the map (one multi-location Open-Meteo request) and interpolated.
//! Thunderstorm cells (WMO 95–99) get a lightning bolt, snow its own colour.
//!
//! The grid costs ~108 calls against Open-Meteo's free limit, so it is only
//! fetched when rain is likely at the location itself (`rain_expected`);
//! otherwise the map is drawn with the time labels and no precipitation.

use crate::metrics;
use anyhow::{Context as _, Result, anyhow, bail};
use chrono::{Duration as ChronoDuration, NaiveDateTime, Timelike, Utc};
use futures_util::future::try_join_all;
use serde::Deserialize;
use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tracing::debug;

use super::weather::{CurrentWeather, Location};

/// Output size in pixels.
const WIDTH: usize = 480;
const HEIGHT: usize = 360;

/// OpenStreetMap tiles at zoom 8: about 600 m per pixel at mid latitudes, so
/// the map spans roughly 290 × 220 km.
const ZOOM: u32 = 8;
const TILE: usize = 256;
const WORLD: f64 = (TILE << ZOOM) as f64;
const OSM_TILES: &str = "https://tile.openstreetmap.org";

/// The OSM tile usage policy asks clients to cache tiles: keep the PNGs (about
/// 20 KB each) for a week.
const TILE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const TILE_CACHE_SIZE: usize = 512;

/// Grid of sample points over the map. Each point counts as one call against
/// Open-Meteo's free limit (10,000 a day), hence the cache below.
const COLS: usize = 12;
const ROWS: usize = 9;

/// Hours of forecast after the "now" frame.
const FORECAST_FRAMES: i64 = 3;

/// Default for `RADAR_RAIN_CHANCE`: the grid is fetched when the chance of
/// precipitation at the location reaches this (in %) in any of the map's hours.
const DEFAULT_RAIN_CHANCE: f64 = 20.0;

/// Fetched data stays valid this long (Open-Meteo updates every 15 minutes).
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_SIZE: usize = 16;

pub const ATTRIBUTION: &str = "© OpenStreetMap contributors";

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!(
            "ClimaBot/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/viniciussaporto/ClimaBot)"
        ))
        .build()
        .expect("failed to build HTTP client")
});

type Rgba = [u8; 4];

/// Everything needed to draw the GIF, fetched once per location.
pub struct RadarData {
    /// Darkened basemap, WIDTH × HEIGHT.
    base: Vec<Rgba>,
    frames: Vec<Frame>,
}

/// The finished map for the embed.
pub struct Map {
    pub gif: Vec<u8>,
    /// False when the grid was skipped and the map shows no precipitation.
    pub precipitation_expected: bool,
}

struct Frame {
    /// UTC time the frame shows (for forecasts: the start of the hour).
    time: NaiveDateTime,
    forecast: bool,
    /// Precipitation rate in mm/h, `COLS * ROWS`, row-major.
    rate: Vec<f32>,
    code: Vec<i32>,
}

/// Recently fetched locations, keyed by their exact coordinates.
struct Cached {
    key: (u64, u64, bool),
    at: Instant,
    data: Arc<RadarData>,
}
static CACHE: LazyLock<Mutex<Vec<Cached>>> = LazyLock::new(Default::default);

type TileCache = HashMap<(i64, i64), (Instant, Arc<Vec<u8>>)>;
static TILES: LazyLock<Mutex<TileCache>> = LazyLock::new(Default::default);

// ─────────────────────────────────────────────
//  Web Mercator
// ─────────────────────────────────────────────

fn to_world(lat: f64, lng: f64) -> (f64, f64) {
    let lat = lat.clamp(-85.0, 85.0).to_radians();
    let x = (lng + 180.0) / 360.0 * WORLD;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * WORLD;
    (x, y)
}

fn from_world(x: f64, y: f64) -> (f64, f64) {
    let lng = x / WORLD * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y / WORLD))
        .sinh()
        .atan()
        .to_degrees();
    (lat, (lng + 180.0).rem_euclid(360.0) - 180.0)
}

/// World pixel of the map's top-left corner.
fn origin(loc: &Location) -> (f64, f64) {
    let (cx, cy) = to_world(loc.lat, loc.lng);
    (cx - WIDTH as f64 / 2.0, cy - HEIGHT as f64 / 2.0)
}

/// Map pixel of grid point (col, row): the grid spans the whole image.
fn grid_pixel(col: usize, row: usize) -> (f64, f64) {
    (
        col as f64 * (WIDTH - 1) as f64 / (COLS - 1) as f64,
        row as f64 * (HEIGHT - 1) as f64 / (ROWS - 1) as f64,
    )
}

// ─────────────────────────────────────────────
//  Fetching
// ─────────────────────────────────────────────

/// Whether rain is likely enough at the location, from now to +3 h, to fetch
/// the precipitation grid: it's precipitating now, some is forecast, or the
/// chance reaches `RADAR_RAIN_CHANCE` (default 20%).
pub fn rain_expected(c: &CurrentWeather) -> bool {
    let threshold = std::env::var("RADAR_RAIN_CHANCE")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_RAIN_CHANCE);
    rain_expected_with(c.weathercode, c.rain_outlook, threshold)
}

fn rain_expected_with(code: i32, outlook: super::weather::RainOutlook, threshold: f64) -> bool {
    // WMO 51 and up are all drizzle, rain, snow, showers or thunderstorms.
    code >= 51 || outlook.total_mm >= 0.1 || outlook.max_chance.is_some_and(|p| p >= threshold)
}

/// Fetch the basemap and, if `precipitation_expected`, the precipitation grid
/// for `loc` (cached for a few minutes). Without the grid the frames are dry.
pub async fn fetch(loc: &Location, precipitation_expected: bool) -> Result<Arc<RadarData>> {
    let key = (loc.lat.to_bits(), loc.lng.to_bits(), precipitation_expected);
    {
        let mut cache = CACHE.lock().unwrap();
        cache.retain(|e| e.at.elapsed() < CACHE_TTL);
        if let Some(e) = cache.iter().find(|e| e.key == key) {
            debug!(location = %loc.formatted, "Radar data from cache");
            return Ok(e.data.clone());
        }
    }

    let (ox, oy) = origin(loc);
    let (base, frames) = if precipitation_expected {
        tokio::try_join!(basemap(ox, oy), precipitation_grid(ox, oy))?
    } else {
        metrics::WEATHER_API_COUNTER
            .with_label_values(&["precip_grid", "skipped"])
            .inc();
        debug!(location = %loc.formatted, "No rain expected: precipitation grid skipped");
        (basemap(ox, oy).await?, dry_frames(Utc::now().naive_utc()))
    };
    let data = Arc::new(RadarData { base, frames });

    let mut cache = CACHE.lock().unwrap();
    if cache.len() >= CACHE_SIZE {
        cache.remove(0);
    }
    cache.push(Cached {
        key,
        at: Instant::now(),
        data: data.clone(),
    });
    Ok(data)
}

/// Stitch the OSM tiles covering the map into a darkened WIDTH × HEIGHT image.
async fn basemap(ox: f64, oy: f64) -> Result<Vec<Rgba>> {
    let tiles = 1i64 << ZOOM;
    let tx0 = (ox / TILE as f64).floor() as i64;
    let ty0 = (oy / TILE as f64).floor() as i64;
    let tx1 = ((ox + WIDTH as f64 - 1.0) / TILE as f64).floor() as i64;
    let ty1 = ((oy + HEIGHT as f64 - 1.0) / TILE as f64).floor() as i64;

    let mut wanted = Vec::new();
    for ty in ty0.max(0)..=ty1.min(tiles - 1) {
        for tx in tx0..=tx1 {
            wanted.push((tx, ty));
        }
    }
    let fetched = try_join_all(wanted.iter().map(|&(tx, ty)| async move {
        let bytes = tile(tx.rem_euclid(tiles), ty).await?;
        let img = decode_png(&bytes).with_context(|| format!("bad tile {ZOOM}/{tx}/{ty}"))?;
        Ok::<_, anyhow::Error>((tx, ty, img))
    }))
    .await;

    // Outside the map (beyond ±85°): the darkened colour of OSM's sea.
    let mut out = vec![darken([0xaa, 0xd3, 0xdf, 255]); WIDTH * HEIGHT];
    for (tx, ty, (w, h, px)) in fetched? {
        if w != TILE || h != TILE {
            bail!("unexpected tile size {w}x{h}");
        }
        let left = tx * TILE as i64 - ox.floor() as i64;
        let top = ty * TILE as i64 - oy.floor() as i64;
        for y in 0..TILE as i64 {
            let my = top + y;
            if !(0..HEIGHT as i64).contains(&my) {
                continue;
            }
            for x in 0..TILE as i64 {
                let mx = left + x;
                if (0..WIDTH as i64).contains(&mx) {
                    out[my as usize * WIDTH + mx as usize] =
                        darken(px[y as usize * TILE + x as usize]);
                }
            }
        }
    }
    Ok(out)
}

/// One OSM tile's PNG, from the cache when possible.
async fn tile(x: i64, y: i64) -> Result<Arc<Vec<u8>>> {
    if let Some((at, png)) = TILES.lock().unwrap().get(&(x, y))
        && at.elapsed() < TILE_TTL
    {
        return Ok(png.clone());
    }
    let url = format!("{OSM_TILES}/{ZOOM}/{x}/{y}.png");
    let result = async {
        HTTP.get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
    }
    .await;
    let status = if result.is_ok() { "success" } else { "error" };
    metrics::WEATHER_API_COUNTER
        .with_label_values(&["map_tile", status])
        .inc();
    let png = Arc::new(result.with_context(|| format!("map tile {url}"))?.to_vec());

    let mut cache = TILES.lock().unwrap();
    if cache.len() >= TILE_CACHE_SIZE {
        cache.retain(|_, (at, _)| at.elapsed() < TILE_TTL);
        if cache.len() >= TILE_CACHE_SIZE {
            cache.clear();
        }
    }
    cache.insert((x, y), (Instant::now(), png.clone()));
    Ok(png)
}

/// Turn the light OSM style into a dark, nearly grey one so the precipitation
/// colours stand out: invert the luminance, compress it, tint it slightly blue.
fn darken(p: Rgba) -> Rgba {
    let lum = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
    let v = 22.0 + (255.0 - lum) * 0.6;
    [
        (v * 0.92) as u8,
        (v * 0.97) as u8,
        (v * 1.08).min(255.0) as u8,
        255,
    ]
}

/// Decode a PNG into RGBA pixels.
fn decode_png(bytes: &[u8]) -> Result<(usize, usize, Vec<Rgba>)> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| anyhow!("PNG too large"))?
    ];
    let info = reader.next_frame(&mut buf)?;
    let buf = &buf[..info.buffer_size()];
    let px: Vec<Rgba> = match info.color_type {
        png::ColorType::Rgba => buf
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect(),
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => bail!("PNG palette was not expanded"),
    };
    Ok((info.width as usize, info.height as usize, px))
}

#[derive(Deserialize)]
struct GridPoint {
    current: GridCurrent,
    hourly: GridHourly,
}

#[derive(Deserialize)]
struct GridCurrent {
    time: String,
    /// Length of the period `precipitation` covers, in seconds.
    interval: f64,
    precipitation: Option<f64>,
    weather_code: Option<i32>,
}

#[derive(Deserialize)]
struct GridHourly {
    time: Vec<String>,
    /// Total of the preceding hour, in mm.
    precipitation: Vec<Option<f64>>,
    weather_code: Vec<Option<i32>>,
}

/// Sample precipitation on the COLS × ROWS grid: now, and the next hours.
async fn precipitation_grid(ox: f64, oy: f64) -> Result<Vec<Frame>> {
    let (mut lats, mut lngs) = (Vec::new(), Vec::new());
    for row in 0..ROWS {
        for col in 0..COLS {
            let (px, py) = grid_pixel(col, row);
            let (lat, lng) = from_world(ox + px, oy + py);
            lats.push(format!("{lat:.3}"));
            lngs.push(format!("{lng:.3}"));
        }
    }
    let form = [
        ("latitude", lats.join(",")),
        ("longitude", lngs.join(",")),
        ("current", "precipitation,weather_code".into()),
        ("hourly", "precipitation,weather_code".into()),
        ("forecast_hours", (FORECAST_FRAMES + 2).to_string()),
        // Same clock for every point, even when the map crosses a time zone.
        ("timezone", "GMT".into()),
    ];
    // POST: 108 coordinate pairs make a long query string.
    let result = async {
        HTTP.post("https://api.open-meteo.com/v1/forecast")
            .form(&form)
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<GridPoint>>()
            .await
    }
    .await;
    let status = if result.is_ok() { "success" } else { "error" };
    metrics::WEATHER_API_COUNTER
        .with_label_values(&["precip_grid", status])
        .inc();
    frames_from(result.context("Open-Meteo precipitation grid request failed")?)
}

/// Frames with no precipitation, for `now` (UTC) and the next hours.
fn dry_frames(now: NaiveDateTime) -> Vec<Frame> {
    let now = now
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now);
    let hour = now.with_minute(0).unwrap_or(now);
    let frame = |time, forecast| Frame {
        time,
        forecast,
        rate: vec![0.0; COLS * ROWS],
        code: vec![0; COLS * ROWS],
    };
    let mut frames = vec![frame(now, false)];
    frames.extend((1..=FORECAST_FRAMES).map(|k| frame(hour + ChronoDuration::hours(k), true)));
    frames
}

fn frames_from(points: Vec<GridPoint>) -> Result<Vec<Frame>> {
    if points.len() != COLS * ROWS {
        bail!("expected {} grid points, got {}", COLS * ROWS, points.len());
    }
    let parse = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M");
    let now = parse(&points[0].current.time).context("bad current time")?;

    let mut frames = vec![Frame {
        time: now,
        forecast: false,
        rate: points
            .iter()
            .map(|p| {
                (p.current.precipitation.unwrap_or(0.0) * 3600.0 / p.current.interval.max(60.0))
                    as f32
            })
            .collect(),
        code: points
            .iter()
            .map(|p| p.current.weather_code.unwrap_or(0))
            .collect(),
    }];

    // Hourly values are totals for the hour *ending* at their timestamp, so
    // the forecast for 21:00–22:00 is the entry stamped 22:00.
    let hour = now.with_minute(0).unwrap_or(now);
    for k in 1..=FORECAST_FRAMES {
        let start = hour + ChronoDuration::hours(k);
        let stamp = (start + ChronoDuration::hours(1))
            .format("%Y-%m-%dT%H:%M")
            .to_string();
        let Some(i) = points[0].hourly.time.iter().position(|t| *t == stamp) else {
            bail!("no hourly forecast for {stamp}");
        };
        frames.push(Frame {
            time: start,
            forecast: true,
            rate: points
                .iter()
                .map(|p| {
                    p.hourly
                        .precipitation
                        .get(i)
                        .copied()
                        .flatten()
                        .unwrap_or(0.0) as f32
                })
                .collect(),
            code: points
                .iter()
                .map(|p| p.hourly.weather_code.get(i).copied().flatten().unwrap_or(0))
                .collect(),
        });
    }
    Ok(frames)
}

// ─────────────────────────────────────────────
//  Drawing
// ─────────────────────────────────────────────

/// Windy-like colour stops: (mm/h, colour, opacity). Rates between two stops
/// get a blend of both, so the overlay is a continuous gradient.
const RAIN_BANDS: [(f32, [u8; 3], f32); 7] = [
    (0.1, [0x3a, 0x8f, 0xd6], 0.45),
    (0.5, [0x1e, 0xb4, 0xe6], 0.55),
    (1.0, [0x2e, 0xc7, 0x5a], 0.60),
    (2.0, [0xe6, 0xd8, 0x2e], 0.65),
    (4.0, [0xf0, 0x8c, 0x1e], 0.70),
    (8.0, [0xe0, 0x32, 0x2e], 0.75),
    (16.0, [0xc8, 0x3c, 0xc8], 0.80),
];
const SNOW: [u8; 3] = [0xe4, 0xec, 0xff];

fn is_snow(code: i32) -> bool {
    matches!(code, 71..=77 | 85 | 86)
}

fn is_thunderstorm(code: i32) -> bool {
    matches!(code, 95..=99)
}

/// Below this rate it's dry; up to twice the first stop the overlay fades in,
/// so rain areas have soft edges.
const DRY: f32 = 0.03;

/// Colour and opacity for a precipitation rate, or `None` when it's dry.
/// `snow` is the share of snow (0–1) at that spot.
fn precip_colour(rate: f32, snow: f32) -> Option<([u8; 3], f32)> {
    if rate < DRY {
        return None;
    }
    // Position along the stops, in log space: the stops roughly double, so
    // each step gets the same share of the gradient.
    let band = RAIN_BANDS.iter().rposition(|b| rate >= b.0).unwrap_or(0);
    let (colour, alpha, pos) = match RAIN_BANDS.get(band + 1) {
        Some(next) if rate >= RAIN_BANDS[0].0 => {
            let (lo, hi) = (RAIN_BANDS[band], next);
            let t = (rate / lo.0).ln() / (hi.0 / lo.0).ln();
            let mix = |a: u8, b: u8| a as f32 + (b as f32 - a as f32) * t;
            let c = [
                mix(lo.1[0], hi.1[0]),
                mix(lo.1[1], hi.1[1]),
                mix(lo.1[2], hi.1[2]),
            ];
            (c, lo.2 + (hi.2 - lo.2) * t, band as f32 + t)
        }
        _ => {
            let (_, c, a) = RAIN_BANDS[band];
            ([c[0] as f32, c[1] as f32, c[2] as f32], a, band as f32)
        }
    };
    let x = ((rate - DRY) / (RAIN_BANDS[0].0 * 2.0 - DRY)).clamp(0.0, 1.0);
    let fade = x * x * (3.0 - 2.0 * x);
    let mix = |a: f32, b: u8| (a + (b as f32 - a) * snow).round() as u8;
    let colour = [
        mix(colour[0], SNOW[0]),
        mix(colour[1], SNOW[1]),
        mix(colour[2], SNOW[2]),
    ];
    let alpha = alpha + (0.35 + pos * 0.08 - alpha) * snow;
    Some((colour, alpha * fade))
}

/// Catmull-Rom weights for sampling a grid axis of `n` points at position `g`:
/// four (index, weight) pairs, indices clamped at the edges. Smoother than
/// bilinear, which leaves diamond-shaped creases on a coarse grid.
fn cubic_weights(g: f32, n: usize) -> [(usize, f32); 4] {
    let i = (g.floor() as isize).min(n as isize - 2);
    let t = g - i as f32;
    let (t2, t3) = (t * t, t * t * t);
    let w = [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ];
    let idx = |k: isize| (i + k).clamp(0, n as isize - 1) as usize;
    [
        (idx(-1), w[0]),
        (idx(0), w[1]),
        (idx(1), w[2]),
        (idx(2), w[3]),
    ]
}

fn blend(dst: &mut Rgba, src: [u8; 3], alpha: f32) {
    for i in 0..3 {
        dst[i] = (dst[i] as f32 * (1.0 - alpha) + src[i] as f32 * alpha).round() as u8;
    }
}

/// Render the animated GIF. `utc_offset_seconds` sets the clock shown on each frame.
pub fn render_gif(data: &RadarData, utc_offset_seconds: i32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, WIDTH as u16, HEIGHT as u16, &[])?;
        encoder.set_repeat(gif::Repeat::Infinite)?;
        let n = data.frames.len();
        for (i, frame) in data.frames.iter().enumerate() {
            let mut px = draw_frame(data, frame, i, n, utc_offset_seconds);
            let mut gif_frame =
                gif::Frame::from_rgba_speed(WIDTH as u16, HEIGHT as u16, px.as_flattened_mut(), 10);
            // In hundredths of a second; hold the last frame longer before looping.
            gif_frame.delay = if i + 1 == n { 250 } else { 120 };
            encoder.write_frame(&gif_frame)?;
        }
    }
    Ok(out)
}

fn draw_frame(
    data: &RadarData,
    frame: &Frame,
    index: usize,
    count: usize,
    utc_offset_seconds: i32,
) -> Vec<Rgba> {
    let mut px = data.base.clone();

    // Precipitation: bicubic interpolation between grid points, of the rate
    // and of the share of snow.
    let snow: Vec<f32> = frame
        .code
        .iter()
        .map(|&c| if is_snow(c) { 1.0 } else { 0.0 })
        .collect();
    let sx = (COLS - 1) as f32 / (WIDTH - 1) as f32;
    let sy = (ROWS - 1) as f32 / (HEIGHT - 1) as f32;
    for y in 0..HEIGHT {
        let wy = cubic_weights(y as f32 * sy, ROWS);
        for x in 0..WIDTH {
            let wx = cubic_weights(x as f32 * sx, COLS);
            let sample = |v: &[f32]| {
                let mut sum = 0.0;
                for &(r, a) in &wy {
                    for &(c, b) in &wx {
                        sum += v[r * COLS + c] * a * b;
                    }
                }
                sum.max(0.0)
            };
            if let Some((colour, alpha)) =
                precip_colour(sample(&frame.rate), sample(&snow).min(1.0))
            {
                blend(&mut px[y * WIDTH + x], colour, alpha);
            }
        }
    }

    for row in 0..ROWS {
        for col in 0..COLS {
            if is_thunderstorm(frame.code[row * COLS + col]) {
                let (x, y) = grid_pixel(col, row);
                draw_bolt(&mut px, x as i32, y as i32);
            }
        }
    }

    draw_marker(&mut px, WIDTH as i32 / 2, HEIGHT as i32 / 2);

    let local = frame.time + ChronoDuration::seconds(utc_offset_seconds.into());
    let label = if frame.forecast {
        format!("+{}H {}", index, local.format("%H:%M"))
    } else {
        format!("NOW {}", local.format("%H:%M"))
    };
    draw_label(&mut px, &label);
    draw_progress(&mut px, index, count);
    px
}

fn put(px: &mut [Rgba], x: i32, y: i32, colour: [u8; 3], alpha: f32) {
    if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
        blend(&mut px[y as usize * WIDTH + x as usize], colour, alpha);
    }
}

fn fill_rect(px: &mut [Rgba], x: i32, y: i32, w: i32, h: i32, colour: [u8; 3], alpha: f32) {
    for yy in y..y + h {
        for xx in x..x + w {
            put(px, xx, yy, colour, alpha);
        }
    }
}

/// The requested location: a white dot with a dark ring.
fn draw_marker(px: &mut [Rgba], cx: i32, cy: i32) {
    for dy in -6..=6i32 {
        for dx in -6..=6i32 {
            let d2 = dx * dx + dy * dy;
            if d2 <= 16 {
                put(px, cx + dx, cy + dy, [255, 255, 255], 1.0);
            } else if d2 <= 36 {
                put(px, cx + dx, cy + dy, [0x10, 0x10, 0x14], 0.85);
            }
        }
    }
}

/// Lightning bolt centred on (cx, cy), yellow with a dark outline.
fn draw_bolt(px: &mut [Rgba], cx: i32, cy: i32) {
    const BOLT: [&str; 11] = [
        "....###", "...###.", "..###..", ".###...", "#######", "...###.", "..###..", ".###...",
        ".##....", "##.....", "#......",
    ];
    let (x0, y0) = (cx - 3, cy - 5);
    let on = |x: i32, y: i32| {
        BOLT.get(y as usize)
            .and_then(|r| r.as_bytes().get(x as usize))
            .is_some_and(|&b| b == b'#' && x >= 0 && y >= 0)
    };
    for y in -1..=11 {
        for x in -1..=7 {
            if on(x, y) {
                put(px, x0 + x, y0 + y, [0xff, 0xe0, 0x2a], 1.0);
            } else if (-1..=1).any(|dy| (-1..=1).any(|dx| on(x + dx, y + dy))) {
                put(px, x0 + x, y0 + y, [0x10, 0x10, 0x10], 0.9);
            }
        }
    }
}

/// 5×7 glyphs for the frame label; bit 4 is the leftmost column.
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        _ => [0; 7],
    }
}

/// Frame label in the top-left corner, on a dark box.
fn draw_label(px: &mut [Rgba], text: &str) {
    const SCALE: i32 = 2;
    const ADVANCE: i32 = 6 * SCALE;
    let n = text.chars().count() as i32;
    fill_rect(
        px,
        8,
        8,
        n * ADVANCE - SCALE + 16,
        7 * SCALE + 12,
        [0x10, 0x10, 0x14],
        0.75,
    );
    for (i, c) in text.chars().enumerate() {
        let rows = glyph(c);
        for (r, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    let x = 16 + i as i32 * ADVANCE + col * SCALE;
                    let y = 14 + r as i32 * SCALE;
                    fill_rect(px, x, y, SCALE, SCALE, [255, 255, 255], 1.0);
                }
            }
        }
    }
}

/// One segment per frame along the bottom edge; the current one is bright.
fn draw_progress(px: &mut [Rgba], index: usize, count: usize) {
    let (margin, gap, h) = (8i32, 4i32, 4i32);
    let w = (WIDTH as i32 - 2 * margin - gap * (count as i32 - 1)) / count as i32;
    for i in 0..count as i32 {
        let x = margin + i * (w + gap);
        let (colour, alpha) = if i as usize == index {
            ([255, 255, 255], 0.95)
        } else {
            ([0x90, 0x90, 0x98], 0.5)
        };
        fill_rect(px, x, HEIGHT as i32 - margin - h, w, h, colour, alpha);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_round_trip() {
        for (lat, lng) in [(41.15, -8.61), (-22.9, -43.2), (60.0, 179.9), (0.0, 0.0)] {
            let (x, y) = to_world(lat, lng);
            let (la, ln) = from_world(x, y);
            assert!(
                (la - lat).abs() < 1e-9 && (ln - lng).abs() < 1e-9,
                "{lat},{lng} -> {la},{ln}"
            );
        }
        // Zoom 8: Greenwich/equator is the centre of a 65536 px world.
        assert_eq!(to_world(0.0, 0.0), (32768.0, 32768.0));
    }

    #[test]
    fn cubic_weights_interpolate() {
        // On a grid point the sample is exactly that point; weights sum to 1.
        let w = cubic_weights(3.0, COLS);
        assert_eq!((w[1].0, w[1].1), (3, 1.0));
        let w = cubic_weights(0.4, COLS);
        assert!((w.iter().map(|p| p.1).sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(w[0].0, 0, "clamped at the edge");
        assert_eq!(cubic_weights((COLS - 1) as f32, COLS)[2].0, COLS - 1);
    }

    #[test]
    fn colours() {
        assert!(precip_colour(0.02, 0.0).is_none());
        assert!(
            precip_colour(0.05, 0.0).unwrap().1 < RAIN_BANDS[0].2,
            "fades in"
        );
        assert_eq!(precip_colour(0.5, 0.0).unwrap().0, RAIN_BANDS[1].1);
        // Halfway (in log space) between two stops: a blend of both colours.
        let mid = precip_colour((2.0f32 * 4.0).sqrt(), 0.0).unwrap().0;
        assert_eq!(mid, [0xeb, 0xb2, 0x26]);
        assert_eq!(precip_colour(50.0, 0.0).unwrap().0, RAIN_BANDS[6].1);
        assert_eq!(precip_colour(3.0, 1.0).unwrap().0, SNOW);
        assert!(is_thunderstorm(95) && is_thunderstorm(99) && !is_thunderstorm(82));
    }

    fn point(now: &str, rate: f64, code: i32) -> GridPoint {
        GridPoint {
            current: GridCurrent {
                time: now.into(),
                interval: 900.0,
                precipitation: Some(rate / 4.0),
                weather_code: Some(code),
            },
            hourly: GridHourly {
                time: ["20:00", "21:00", "22:00", "23:00"]
                    .iter()
                    .map(|h| format!("2026-11-18T{h}"))
                    .chain(["2026-11-19T00:00".to_string()])
                    .collect(),
                precipitation: vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0), Some(4.0)],
                weather_code: vec![Some(0), Some(61), Some(63), Some(95), Some(65)],
            },
        }
    }

    fn sample() -> Vec<Frame> {
        let points = (0..COLS * ROWS)
            .map(|_| point("2026-11-18T20:15", 2.0, 80))
            .collect();
        frames_from(points).unwrap()
    }

    #[test]
    fn frames_pick_the_next_hours() {
        let frames = sample();
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0].rate[0], 2.0, "15-minute total scaled to mm/h");
        // 21:00–22:00 is the entry stamped 22:00, and so on.
        let starts: Vec<_> = frames[1..]
            .iter()
            .map(|f| f.time.format("%H:%M").to_string())
            .collect();
        assert_eq!(starts, ["21:00", "22:00", "23:00"]);
        assert_eq!(frames[1].rate[0], 2.0);
        assert_eq!(frames[2].code[0], 95);
        assert_eq!(frames[3].rate[0], 4.0);
    }

    #[test]
    fn gate() {
        use crate::commands::weather::RainOutlook;
        let dry = RainOutlook {
            max_chance: Some(10.0),
            total_mm: 0.0,
        };
        assert!(!rain_expected_with(3, dry, 20.0));
        assert!(rain_expected_with(
            3,
            RainOutlook {
                max_chance: Some(20.0),
                ..dry
            },
            20.0
        ));
        assert!(rain_expected_with(
            3,
            RainOutlook {
                total_mm: 0.2,
                ..dry
            },
            20.0
        ));
        assert!(rain_expected_with(61, dry, 20.0), "raining now");
        assert!(
            !rain_expected_with(45, dry, 20.0),
            "fog isn't precipitation"
        );
        assert!(
            !rain_expected_with(0, RainOutlook::default(), 20.0),
            "no data, no rain"
        );
    }

    #[test]
    fn dry_frames_follow_the_clock() {
        let now = NaiveDateTime::parse_from_str("2026-11-18T20:47", "%Y-%m-%dT%H:%M").unwrap();
        let frames = dry_frames(now);
        let times: Vec<_> = frames
            .iter()
            .map(|f| f.time.format("%H:%M").to_string())
            .collect();
        assert_eq!(times, ["20:47", "21:00", "22:00", "23:00"]);
        assert!(frames.iter().all(|f| f.rate.iter().all(|&r| r == 0.0)));
        let data = RadarData {
            base: vec![[40, 40, 40, 255]; WIDTH * HEIGHT],
            frames,
        };
        assert_eq!(&render_gif(&data, 0).unwrap()[..6], b"GIF89a");
    }

    #[test]
    fn grid_size_is_checked() {
        assert!(frames_from(vec![point("2026-11-18T20:15", 0.0, 0)]).is_err());
    }

    #[test]
    fn renders_a_gif() {
        let data = RadarData {
            base: vec![[40, 40, 40, 255]; WIDTH * HEIGHT],
            frames: sample(),
        };
        let gif = render_gif(&data, 3600).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        let mut decoder = gif::DecodeOptions::new()
            .read_info(Cursor::new(&gif))
            .unwrap();
        let mut n = 0;
        while decoder.read_next_frame().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 4);
    }
}

/// Live render for eyeballing the result:
/// `RADAR_OUT=map.gif cargo test live_render -- --ignored --nocapture`
#[cfg(test)]
#[tokio::test]
#[ignore = "needs network"]
async fn live_render() {
    let lat = std::env::var("RADAR_LAT").map_or(41.15, |v| v.parse().unwrap());
    let lng = std::env::var("RADAR_LNG").map_or(-8.61, |v| v.parse().unwrap());
    let loc = Location {
        lat,
        lng,
        formatted: "test".into(),
    };
    let started = Instant::now();
    let current = super::weather::get_current(&loc).await.unwrap();
    let wet = rain_expected(&current);
    println!("outlook {:?}, code {}, grid fetched: {wet}", current.rain_outlook, current.weathercode);
    let data = fetch(&loc, wet).await.unwrap();
    let fetched = started.elapsed();
    let gif = render_gif(&data, 0).unwrap();
    println!(
        "fetch {fetched:?}, total {:?}, {} KiB",
        started.elapsed(),
        gif.len() / 1024
    );
    let out = std::env::var("RADAR_OUT").unwrap_or("radar.gif".into());
    // Also each frame as a PNG, for viewers that don't animate.
    for (i, frame) in data.frames.iter().enumerate() {
        let px = draw_frame(&data, frame, i, data.frames.len(), 0);
        let file = std::fs::File::create(format!("{out}.{i}.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), WIDTH as u32, HEIGHT as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header()
            .unwrap()
            .write_image_data(px.as_flattened())
            .unwrap();
    }
    std::fs::write(out, gif).unwrap();
}
