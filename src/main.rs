// Copyright (c) 2006-2026 afri & veit
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! FUNKFABRIK\*B — web server.
//!
//! A single-binary Axum application that serves the FUNKFABRIK\*B teletext-
//! aesthetic website.  All pages are rendered server-side via Tera templates.
//! Static assets are served from the `static/` directory.

mod guestbook;

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{Html, Redirect},
    routing::{get, post},
};
use guestbook::GuestEntry;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tera::{Context, Tera};
use tower_http::services::ServeDir;

/// All navigable pages, in display order.
///
/// Each tuple is `(page_number, human-readable title, blink)`.  The page
/// number is both the URL path segment (`/100`) and the Tera template name
/// (`100.html`).  `blink` causes the number to flash in the nav bar — used
/// to highlight special or time-limited pages.
const PAGES: &[(&str, &str, bool)] = &[
    ("100", "Startseite",      false),
    ("101", "Radio hören",     false),
    ("170", "Wettermagazin",   false),
    ("300", "20 Jahre Brutto", true),
    ("404", "Fanseite",        false),
    ("666", "Kontakt",         false),
    ("777", "Spiele",          false),
    ("999", "Impressum",       false),
];

/// Cached weather data fetched from wttr.in.
///
/// The cache is shared across all requests via [`AppState`].  A [`Mutex`] is
/// used rather than a `tokio::sync::Mutex` because the critical section is
/// purely in-memory (no `.await` points while the lock is held).
struct WeatherCache {
    /// Last successfully validated weather string (wttr.in `format=2` output).
    value: String,
    /// Unix timestamp (seconds) when `value` was populated.  Starts at `0` so
    /// the first request always triggers a fetch.
    fetched_at: u64,
}

/// Shared application state passed to every handler via Axum's [`State`] extractor.
#[derive(Clone)]
struct AppState {
    /// Compiled Tera template engine, wrapped in an [`Arc`] so it can be shared
    /// cheaply across async tasks without cloning the underlying template data.
    tera: Arc<Tera>,
    /// Reusable HTTP client for outbound requests (weather API, RSS proxy).
    /// [`reqwest::Client`] is cheaply cloneable and manages a connection pool
    /// internally, so a single instance is shared for the lifetime of the server.
    http: reqwest::Client,
    /// URL for the wttr.in current-conditions endpoint.  Stored here so tests
    /// can substitute a local mock server without patching the binary.
    weather_url: String,
    /// URL for the podcast archive RSS feed.  Stored here so tests can
    /// substitute a local mock server without patching the binary.
    rss_url: String,
    /// Server-side cache for the wttr.in response.  Shared across all handlers
    /// via `Arc`; refreshed at most once per [`WEATHER_CACHE_TTL_SECS`].
    weather_cache: Arc<Mutex<WeatherCache>>,
    /// In-memory guestbook entries, mirroring the JSON file on disk.
    /// Protected by a [`Mutex`] so concurrent requests can safely append.
    guestbook: Arc<Mutex<Vec<GuestEntry>>>,
    /// Path to the guestbook JSON file.  Stored here so tests can use a
    /// temporary path without touching the production data directory.
    guestbook_path: std::path::PathBuf,
}

/// How long (in seconds) a cached wttr.in response is considered fresh.
const WEATHER_CACHE_TTL_SECS: u64 = 3600;

/// Entry point.  Compiles templates, builds the router, and starts the server.
#[tokio::main]
async fn main() {
    let tera = Tera::new("templates/**/*.html").expect("failed to parse templates");
    let guestbook_path = std::path::PathBuf::from("data/guestbook.json");
    let guestbook_entries = guestbook::load(&guestbook_path);
    let state = AppState {
        tera: Arc::new(tera),
        http: reqwest::Client::new(),
        weather_url: "https://wttr.in/Berlin?format=2".into(),
        rss_url: "https://archiv.funkfabrik-b.de/rss".into(),
        weather_cache: Arc::new(Mutex::new(WeatherCache {
            value: String::new(),
            fetched_at: 0,
        })),
        guestbook: Arc::new(Mutex::new(guestbook_entries)),
        guestbook_path,
    };

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("FUNKFABRIK*B listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}

/// Constructs the application [`Router`].
///
/// Extracted from `main` so that tests can build the router without binding a
/// real TCP socket.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::permanent("/100") }))
        .route("/api/rss", get(rss_proxy))
        // Dedicated handlers for sub-pages — must be registered before the
        // generic `/{page}` wildcard so Axum's static-path priority rule
        // resolves them first.
        .route("/666", get(guestbook_page))
        .route("/666/send", post(guestbook_post))
        .route("/777/{game}", get(game_subpage_handler))
        .route("/{page}", get(page_handler))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state)
}

/// Renders a page by number.
///
/// Validates `page` against [`PAGES`] before constructing the template name,
/// so only known page numbers are ever passed to the renderer.  Unknown pages
/// render `404.html` directly.  Falls back to an inline error string if even
/// the 404 template fails.
///
/// # Template context variables
///
/// | Variable       | Type                     | Description                    |
/// |----------------|--------------------------|--------------------------------|
/// | `current_page` | `String`                 | The requested page number      |
/// | `page_title`   | `&str`                   | Human-readable page title      |
/// | `pages`        | `Vec<{num, title}>`      | All pages, used by the nav bar |
/// | `weather`      | `String` *(page 170)*    | Current conditions from wttr.in|
/// | `forecast`     | `Vec<{…}>` *(page 170)*  | 3-day generated forecast       |
async fn page_handler(Path(page): Path<String>, State(state): State<AppState>) -> Html<String> {
    let pages: Vec<serde_json::Value> = PAGES
        .iter()
        .map(|(num, title, blink)| serde_json::json!({"num": num, "title": title, "blink": blink}))
        .collect();

    let mut ctx = Context::new();
    ctx.insert("current_page", &page);
    ctx.insert("page_title", page_title_for(&page));
    ctx.insert("pages", &pages);

    // Only render a page template for known page numbers.  This prevents
    // user-supplied path segments from being passed to Tera::render.
    let template = if PAGES.iter().any(|(num, _, _)| *num == page) {
        if page == "170" {
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Check whether the cached value is still fresh.  The lock is
            // dropped immediately after the read so no mutex is held across
            // the subsequent `.await`.
            let cached: Option<String> = {
                let cache = state.weather_cache.lock().unwrap();
                if !cache.value.is_empty()
                    && now_secs.saturating_sub(cache.fetched_at) < WEATHER_CACHE_TTL_SECS
                {
                    Some(cache.value.clone())
                } else {
                    None
                }
            };

            let weather = if let Some(w) = cached {
                w
            } else {
                let raw: String = match state
                    .http
                    .get(&state.weather_url)
                    .header("User-Agent", "curl/8.0")
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                {
                    Ok(r) => r.text().await.unwrap_or_default(),
                    Err(_) => String::new(),
                };

                let fetched = if looks_like_weather(raw.trim()) {
                    raw.trim().to_string()
                } else {
                    generate_current_weather(now_secs)
                };

                // Store result (real or generated fallback) so the next
                // request within the TTL window skips the outbound call.
                {
                    let mut cache = state.weather_cache.lock().unwrap();
                    cache.value = fetched.clone();
                    cache.fetched_at = now_secs;
                }
                fetched
            };

            ctx.insert("weather", &weather);
            ctx.insert("forecast", &build_forecast(now_secs));
        }
        format!("{}.html", page)
    } else {
        "not_found.html".to_string()
    };

    let html = state
        .tera
        .render(&template, &ctx)
        .unwrap_or_else(|_| "<h1 style='color:#FC0204'>PAGE NOT FOUND</h1>".into());

    Html(html)
}

/// Returns the human-readable title for a page number, or `"???"` if unknown.
///
/// Performs a linear scan of [`PAGES`]; acceptable given the tiny page count.
fn page_title_for(page: &str) -> &'static str {
    PAGES
        .iter()
        .find(|(num, _, _)| *num == page)
        .map(|(_, title, _)| *title)
        .unwrap_or("???")
}

/// Computes the ISO weekday index (0 = Monday … 6 = Sunday) from a Unix
/// timestamp in seconds.
///
/// The Unix epoch (1970-01-01) was a Thursday, which is index 3 when Monday
/// is 0, so the formula is `(days_since_epoch + 3) % 7`.
fn weekday_from_secs(secs: u64) -> u64 {
    (secs / 86400 + 3) % 7
}

/// Advances a PCG-style LCG seed by one step and returns the upper 31 bits.
///
/// Uses the Knuth multiplicative hash constants to give good distribution.
/// Not cryptographically secure; suitable only for decorative randomness.
fn lcg_next(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

/// Generates a 3-day weather forecast as a list of Tera-serialisable objects.
///
/// Each entry contains:
/// - `day`   — German weekday abbreviation (Mo, Di, …)
/// - `icon`  — a weather emoji drawn from a fixed palette
/// - `temp`  — temperature in °C (8–22)
/// - `wind`  — wind speed in km/h (5–40)
/// - `color` — a CSS utility class from the teletext palette
///
/// The forecast is seeded from `now_secs` so it is stable within a given
/// second and varies naturally across page loads.
fn build_forecast(now_secs: u64) -> Vec<serde_json::Value> {
    const DAYS: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
    const ICONS: [&str; 7] = ["☀", "🌤", "⛅", "🌦", "☁", "🌧", "⛈"];
    const COLORS: [&str; 4] = ["color-green", "color-yellow", "color-cyan", "color-red"];

    let today = weekday_from_secs(now_secs);
    let mut seed = now_secs;

    (1u64..=3)
        .map(|i| {
            let day   = DAYS[((today + i) % 7) as usize];
            let icon  = ICONS[lcg_next(&mut seed) as usize % ICONS.len()];
            let temp  = 8 + lcg_next(&mut seed) as usize % 15;  // 8–22 °C
            let wind  = 5 + lcg_next(&mut seed) as usize % 36;  // 5–40 km/h
            let color = COLORS[lcg_next(&mut seed) as usize % COLORS.len()];
            serde_json::json!({ "day": day, "icon": icon, "temp": temp, "wind": wind, "color": color })
        })
        .collect()
}

/// Returns `true` if `s` looks like a wttr.in weather string.
///
/// wttr.in `format=2` output always contains a degree symbol followed by `C`
/// or `F` (e.g. `⛅️  +12°C`).  Any response that lacks this is treated as
/// an error page (quota exceeded, HTML fallback, etc.).
fn looks_like_weather(s: &str) -> bool {
    s.contains("°C") || s.contains("°F")
}

/// Generates a plausible current-conditions string using the LCG, as a
/// fallback when wttr.in is unreachable or returns a non-weather response.
///
/// Output format mirrors wttr.in `format=2`, e.g. `⛅  +14°C`.
fn generate_current_weather(now_secs: u64) -> String {
    const ICONS: [&str; 7] = ["☀", "🌤", "⛅", "🌦", "☁", "🌧", "⛈"];
    let mut seed = now_secs.wrapping_mul(2654435761); // different seed offset from forecast
    let icon = ICONS[lcg_next(&mut seed) as usize % ICONS.len()];
    let temp = lcg_next(&mut seed) as i64 % 15 + 8; // 8–22 °C
    format!("{}  +{}°C", icon, temp)
}

/// Renders a game sub-page (`GET /777/{game}`).
///
/// Valid game names are `tetris`, `invaders`, and `snake`; anything else
/// falls through to `not_found.html`.  `current_page` is always `"777"` so
/// the *Spiele* nav entry stays highlighted.
async fn game_subpage_handler(
    Path(game): Path<String>,
    State(state): State<AppState>,
) -> Html<String> {
    let pages: Vec<serde_json::Value> = PAGES
        .iter()
        .map(|(num, title, blink)| serde_json::json!({"num": num, "title": title, "blink": blink}))
        .collect();

    let valid = ["tetris", "invaders", "snake"];
    let template = if valid.contains(&game.as_str()) {
        format!("777_{}.html", game)
    } else {
        "not_found.html".to_string()
    };

    let mut ctx = Context::new();
    ctx.insert("current_page", "777");
    ctx.insert("page_title", "Spiele");
    ctx.insert("pages", &pages);

    let html = state
        .tera
        .render(&template, &ctx)
        .unwrap_or_else(|_| "<h1 style='color:#FC0204'>PAGE NOT FOUND</h1>".into());

    Html(html)
}

/// Form data submitted to `POST /666/send`.
#[derive(Deserialize)]
struct GuestbookForm {
    name: String,
    message: String,
    captcha: String,
}

/// Renders the guestbook page (`GET /666`).
///
/// Passes flash state via query parameters (`?success=1` or `?error=captcha` /
/// `?error=empty`) set by [`guestbook_post`] after a redirect.  Entries are
/// presented newest-first.
async fn guestbook_page(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let pages: Vec<serde_json::Value> = PAGES
        .iter()
        .map(|(num, title, blink)| serde_json::json!({"num": num, "title": title, "blink": blink}))
        .collect();

    let entries: Vec<serde_json::Value> = {
        let gb = state.guestbook.lock().unwrap();
        gb.iter()
            .rev()
            .map(|e| {
                let name = if e.name.is_empty() {
                    "Anonym".to_string()
                } else {
                    e.name.clone()
                };
                serde_json::json!({
                    "name": name,
                    "message": e.message,
                    "date": guestbook::format_timestamp(e.timestamp_secs),
                })
            })
            .collect()
    };

    let flash_success = params.get("success").map(|v| v == "1").unwrap_or(false);
    let flash_error = params.get("error").cloned();

    let mut ctx = Context::new();
    ctx.insert("current_page", "666");
    ctx.insert("page_title", page_title_for("666"));
    ctx.insert("pages", &pages);
    ctx.insert("entries", &entries);
    ctx.insert("flash_success", &flash_success);
    ctx.insert("flash_error", &flash_error);

    let html = state
        .tera
        .render("666.html", &ctx)
        .unwrap_or_else(|_| "<h1 style='color:#FC0204'>PAGE NOT FOUND</h1>".into());

    Html(html)
}

/// Handles guestbook form submission (`POST /666/send`).
///
/// Validates the captcha (answer must be `"B"`, case-insensitive) and that
/// the message is non-empty, then appends the entry and persists it to disk.
/// Always responds with a redirect so a browser refresh does not re-submit.
async fn guestbook_post(
    State(state): State<AppState>,
    Form(form): Form<GuestbookForm>,
) -> Redirect {
    let name = form.name.trim().to_string();
    let message = form.message.trim().to_string();
    let captcha = form.captcha.trim().to_string();

    if !captcha.eq_ignore_ascii_case("b") {
        return Redirect::to("/666?error=captcha");
    }
    if message.is_empty() {
        return Redirect::to("/666?error=empty");
    }
    if message.contains("://") {
        return Redirect::to("/666?error=url");
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entry = GuestEntry { name, message, timestamp_secs: now_secs };

    {
        let mut gb = state.guestbook.lock().unwrap();
        gb.push(entry);
        // Best-effort persist; a write failure is logged to stderr but does
        // not crash the handler — the entry remains in memory for this run.
        if let Err(e) = guestbook::save(&state.guestbook_path, &gb) {
            eprintln!("guestbook save error: {e}");
        }
    }

    Redirect::to("/666?success=1")
}

/// Proxies the podcast archive RSS feed.
///
/// Fetching the feed server-side avoids browser CORS restrictions that would
/// otherwise block the client-side JavaScript from reading the response.
/// Returns `502 Bad Gateway` if the upstream is unreachable or returns an
/// error.
async fn rss_proxy(State(state): State<AppState>) -> Result<(HeaderMap, String), StatusCode> {
    let body = state
        .http
        .get(&state.rss_url)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .text()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "Content-Type",
        HeaderValue::from_static("application/rss+xml; charset=utf-8"),
    );
    Ok((headers, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    /// Build an app backed by the real templates for integration tests.
    fn test_app() -> Router {
        test_app_with_urls("http://127.0.0.1:1/weather", "http://127.0.0.1:1/rss")
    }

    /// Build an app with explicit external URL overrides and a unique temp
    /// guestbook path per call (so tests don't share state on disk).
    fn test_app_with_urls(weather_url: &str, rss_url: &str) -> Router {
        test_app_full(weather_url, rss_url, temp_guestbook_path())
    }

    /// Build an app with full control over all external dependencies.
    fn test_app_full(
        weather_url: &str,
        rss_url: &str,
        gb_path: std::path::PathBuf,
    ) -> Router {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let pattern = format!("{}/templates/**/*.html", manifest);
        let tera = Tera::new(&pattern).expect("failed to parse templates");
        let entries = guestbook::load(&gb_path);
        build_router(AppState {
            tera: Arc::new(tera),
            http: reqwest::Client::new(),
            weather_url: weather_url.into(),
            rss_url: rss_url.into(),
            weather_cache: Arc::new(Mutex::new(WeatherCache {
                value: String::new(),
                fetched_at: 0,
            })),
            guestbook: Arc::new(Mutex::new(entries)),
            guestbook_path: gb_path,
        })
    }

    /// Generate a unique temporary path for the guestbook JSON file so that
    /// parallel tests never share a file on disk.
    fn temp_guestbook_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "fb_gb_test_{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    async fn body_string(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    // ── Pure unit tests ────────────────────────────────────────────────────

    #[test]
    fn page_title_known_pages() {
        assert_eq!(page_title_for("100"), "Startseite");
        assert_eq!(page_title_for("101"), "Radio hören");
        assert_eq!(page_title_for("170"), "Wettermagazin");
        assert_eq!(page_title_for("300"), "20 Jahre Brutto");
        assert_eq!(page_title_for("404"), "Fanseite");
        assert_eq!(page_title_for("666"), "Kontakt");
        assert_eq!(page_title_for("777"), "Spiele");
        assert_eq!(page_title_for("999"), "Impressum");
    }

    #[test]
    fn page_title_unknown_returns_fallback() {
        assert_eq!(page_title_for("000"), "???");
        assert_eq!(page_title_for(""), "???");
        assert_eq!(page_title_for("abc"), "???");
    }

    #[test]
    fn weekday_epoch_is_thursday() {
        // 1970-01-01 00:00:00 UTC was a Thursday (index 3, Mon = 0)
        assert_eq!(weekday_from_secs(0), 3);
    }

    #[test]
    fn weekday_wraps_correctly() {
        // 7 days after epoch is also Thursday
        assert_eq!(weekday_from_secs(7 * 86400), 3);
        // 1 day after epoch is Friday (index 4)
        assert_eq!(weekday_from_secs(86400), 4);
        // 6 days after epoch is Wednesday (index 2)
        assert_eq!(weekday_from_secs(6 * 86400), 2);
    }

    #[test]
    fn lcg_next_is_deterministic() {
        let mut s1 = 42u64;
        let mut s2 = 42u64;
        assert_eq!(lcg_next(&mut s1), lcg_next(&mut s2));
        assert_eq!(lcg_next(&mut s1), lcg_next(&mut s2));
    }

    #[test]
    fn lcg_next_advances_seed() {
        let mut seed = 1u64;
        let a = lcg_next(&mut seed);
        let b = lcg_next(&mut seed);
        assert_ne!(a, b);
    }

    #[test]
    fn build_forecast_returns_three_days() {
        let f = build_forecast(0);
        assert_eq!(f.len(), 3);
    }

    #[test]
    fn build_forecast_days_follow_today() {
        // Seed 0 → epoch → Thursday (idx 3) → next days are Fr, Sa, So
        let f = build_forecast(0);
        assert_eq!(f[0]["day"], "Fr");
        assert_eq!(f[1]["day"], "Sa");
        assert_eq!(f[2]["day"], "So");
    }

    #[test]
    fn build_forecast_temp_in_range() {
        for entry in build_forecast(12345678) {
            let t = entry["temp"].as_u64().unwrap();
            assert!((8..=22).contains(&t), "temp {t} out of range");
        }
    }

    #[test]
    fn build_forecast_wind_in_range() {
        for entry in build_forecast(12345678) {
            let w = entry["wind"].as_u64().unwrap();
            assert!((5..=40).contains(&w), "wind {w} out of range");
        }
    }

    #[test]
    fn build_forecast_is_deterministic() {
        assert_eq!(build_forecast(99999), build_forecast(99999));
    }

    #[test]
    fn build_forecast_varies_by_seed() {
        assert_ne!(build_forecast(1), build_forecast(2));
    }

    #[test]
    fn looks_like_weather_accepts_celsius() {
        assert!(looks_like_weather("⛅  +12°C"));
        assert!(looks_like_weather("☀  +22°C"));
    }

    #[test]
    fn looks_like_weather_accepts_fahrenheit() {
        assert!(looks_like_weather("☀  +72°F"));
    }

    #[test]
    fn looks_like_weather_rejects_quota_message() {
        assert!(!looks_like_weather("Sorry, we are out of quota for your IP."));
        assert!(!looks_like_weather(""));
        assert!(!looks_like_weather("<html><body>Error</body></html>"));
    }

    #[test]
    fn generate_current_weather_contains_degree() {
        let w = generate_current_weather(12345678);
        assert!(looks_like_weather(&w), "generated weather should pass looks_like_weather: {w}");
    }

    #[test]
    fn generate_current_weather_is_deterministic() {
        assert_eq!(generate_current_weather(42), generate_current_weather(42));
    }

    #[test]
    fn generate_current_weather_varies_by_seed() {
        assert_ne!(generate_current_weather(1), generate_current_weather(2));
    }

    // ── HTTP integration tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn root_redirects_to_100() {
        let resp = test_app()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(resp.headers()["location"], "/100");
    }

    #[tokio::test]
    async fn known_page_returns_200() {
        for (num, _, _) in PAGES {
            let resp = test_app()
                .oneshot(
                    Request::get(format!("/{}", num))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "page {num} should be 200");
        }
    }

    /// Page 170 should render successfully even when the weather API is
    /// unreachable, generating random weather rather than showing an error.
    #[tokio::test]
    async fn page_170_renders_with_generated_weather_on_failure() {
        // 127.0.0.1:1 is guaranteed to refuse connections immediately.
        let resp = test_app_with_urls("http://127.0.0.1:1/weather", "http://127.0.0.1:1/rss")
            .oneshot(Request::get("/170").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("°C") || body.contains("°F"), "fallback should contain a temperature");
        assert!(body.contains("Vorhersage"));
    }

    /// Page 170 should use the wttr.in response when it looks like real weather.
    #[tokio::test]
    async fn page_170_uses_real_weather_when_valid() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("⛅  +14°C"))
            .mount(&mock_server)
            .await;

        let resp = test_app_with_urls(&mock_server.uri(), "http://127.0.0.1:1/rss")
            .oneshot(Request::get("/170").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("+14°C"));
    }

    /// Page 170 should generate random weather when wttr.in returns a quota error.
    #[tokio::test]
    async fn page_170_generates_weather_on_quota_exceeded() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("Sorry, we are out of quota for your IP."),
            )
            .mount(&mock_server)
            .await;

        let resp = test_app_with_urls(&mock_server.uri(), "http://127.0.0.1:1/rss")
            .oneshot(Request::get("/170").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("°C") || body.contains("°F"), "should fall back to generated weather");
        assert!(!body.contains("quota"), "quota error message should not appear in output");
    }

    #[tokio::test]
    async fn unknown_page_returns_200_with_404_template() {
        let resp = test_app()
            .oneshot(Request::get("/000").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("nicht gefunden") || body.contains("PAGE NOT FOUND"));
    }

    /// Path traversal attempts must not reach Tera::render; they should fall
    /// through to the 404 template.
    #[tokio::test]
    async fn path_traversal_returns_404_template() {
        for path in ["/../../etc/passwd", "/../secret", "/100%2F..%2Fetc"] {
            let resp = test_app()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            // Either the router rejects it (non-200) or our handler serves 404.html.
            let status = resp.status();
            if status == StatusCode::OK {
                let body = body_string(resp.into_body()).await;
                assert!(
                    body.contains("nicht gefunden") || body.contains("PAGE NOT FOUND"),
                    "traversal path {path} should render 404 template, got: {body:.80}"
                );
            }
        }
    }

    #[tokio::test]
    async fn page_100_contains_station_name() {
        let resp = test_app()
            .oneshot(Request::get("/100").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("FUNKFABRIK"));
    }

    #[tokio::test]
    async fn page_101_contains_player() {
        let resp = test_app()
            .oneshot(Request::get("/101").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("podcast-player"));
    }

    #[tokio::test]
    async fn static_files_are_served() {
        let resp = test_app()
            .oneshot(
                Request::get("/static/teletext.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Page 170 must call the weather API only once when two requests arrive
    /// within the TTL window.  wiremock's `expect(1)` assertion fires on drop
    /// and will panic the test if the mock is hit more or fewer than once.
    #[tokio::test]
    async fn page_170_caches_weather_within_ttl() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("☀  +20°C"))
            .expect(1) // must be called exactly once across both requests
            .mount(&mock_server)
            .await;

        let app = test_app_with_urls(&mock_server.uri(), "http://127.0.0.1:1/rss");

        // First request — cold cache, should hit the mock.
        let resp1 = app
            .clone()
            .oneshot(Request::get("/170").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        assert!(body_string(resp1.into_body()).await.contains("+20°C"));

        // Second request — warm cache, must NOT hit the mock again.
        let resp2 = app
            .oneshot(Request::get("/170").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        assert!(body_string(resp2.into_body()).await.contains("+20°C"));
        // wiremock verifies the expect(1) constraint when mock_server is dropped here.
    }

    /// rss_proxy returns the feed body and the correct Content-Type on success.
    #[tokio::test]
    async fn rss_proxy_returns_content_type_on_success() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<rss><channel><title>Test</title></channel></rss>"),
            )
            .mount(&mock_server)
            .await;

        let resp = test_app_with_urls("http://127.0.0.1:1/weather", &mock_server.uri())
            .oneshot(Request::get("/api/rss").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()["content-type"],
            "application/rss+xml; charset=utf-8"
        );
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("<rss>"));
    }

    /// rss_proxy returns 502 when the upstream feed is unreachable.
    #[tokio::test]
    async fn rss_proxy_returns_502_when_upstream_unreachable() {
        let resp = test_app_with_urls("http://127.0.0.1:1/weather", "http://127.0.0.1:1/rss")
            .oneshot(Request::get("/api/rss").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }

    // ── Guestbook integration tests ───────────────────────────────────────

    fn post_form(path: &str, body: &str) -> Request<Body> {
        Request::post(path)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn guestbook_post_correct_captcha_redirects_to_success() {
        let resp = test_app()
            .oneshot(post_form("/666/send", "name=Punk&message=Oi%21&captcha=B"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers()["location"], "/666?success=1");
    }

    #[tokio::test]
    async fn guestbook_post_lowercase_captcha_is_accepted() {
        let resp = test_app()
            .oneshot(post_form("/666/send", "name=Punk&message=Oi%21&captcha=b"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers()["location"], "/666?success=1");
    }

    #[tokio::test]
    async fn guestbook_post_wrong_captcha_redirects_to_error() {
        let resp = test_app()
            .oneshot(post_form("/666/send", "name=Punk&message=Oi%21&captcha=X"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers()["location"], "/666?error=captcha");
    }

    #[tokio::test]
    async fn guestbook_post_url_in_message_redirects_to_error() {
        for body in [
            "name=Spam&message=visit+https%3A%2F%2Fexample.com&captcha=B",
            "name=Spam&message=visit+http%3A%2F%2Fexample.com&captcha=B",
            "name=Spam&message=ftp%3A%2F%2Fexample.com&captcha=B",
        ] {
            let resp = test_app()
                .oneshot(post_form("/666/send", body))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::SEE_OTHER, "body: {body}");
            assert_eq!(resp.headers()["location"], "/666?error=url", "body: {body}");
        }
    }

    #[tokio::test]
    async fn guestbook_post_empty_message_redirects_to_error() {
        let resp = test_app()
            .oneshot(post_form("/666/send", "name=Punk&message=&captcha=B"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers()["location"], "/666?error=empty");
    }

    #[tokio::test]
    async fn guestbook_get_success_flash_shown() {
        let resp = test_app()
            .oneshot(Request::get("/666?success=1").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("gespeichert"), "success flash missing");
    }

    #[tokio::test]
    async fn guestbook_get_captcha_error_flash_shown() {
        let resp = test_app()
            .oneshot(
                Request::get("/666?error=captcha")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("Lösungswort"), "captcha error flash missing");
    }

    #[tokio::test]
    async fn guestbook_entry_appears_after_post() {
        let gb_path = temp_guestbook_path();
        let app = test_app_full("http://127.0.0.1:1/weather", "http://127.0.0.1:1/rss", gb_path.clone());

        // Submit an entry.
        let post_resp = app
            .clone()
            .oneshot(post_form(
                "/666/send",
                "name=TestUser&message=Hallo+Welt&captcha=B",
            ))
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::SEE_OTHER);

        // The entry should be visible on the page.
        let get_resp = app
            .oneshot(Request::get("/666").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = body_string(get_resp.into_body()).await;
        assert!(body.contains("TestUser"), "name not in body");
        assert!(body.contains("Hallo Welt"), "message not in body");

        let _ = std::fs::remove_file(&gb_path);
    }
}
