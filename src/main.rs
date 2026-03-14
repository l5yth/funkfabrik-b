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

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{Html, Redirect},
    routing::get,
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tera::{Context, Tera};
use tower_http::services::ServeDir;

/// All navigable pages, in display order.
///
/// Each tuple is `(page_number, human-readable title)`.  The page number is
/// both the URL path segment (`/100`) and the Tera template name (`100.html`).
const PAGES: &[(&str, &str)] = &[
    ("100", "Startseite"),
    ("101", "Radio hören"),
    ("170", "Wettermagazin"),
    ("300", "Fanseite"),
    ("666", "Kontakt"),
    ("777", "Spiele"),
    ("999", "Impressum"),
];

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
}

/// Entry point.  Compiles templates, builds the router, and starts the server.
#[tokio::main]
async fn main() {
    let tera = Tera::new("templates/**/*.html").expect("failed to parse templates");
    let state = AppState {
        tera: Arc::new(tera),
        http: reqwest::Client::new(),
        weather_url: "https://wttr.in/Berlin?format=2".into(),
        rss_url: "https://archiv.funkfabrik-b.de/rss".into(),
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
        .map(|(num, title)| serde_json::json!({"num": num, "title": title}))
        .collect();

    let mut ctx = Context::new();
    ctx.insert("current_page", &page);
    ctx.insert("page_title", page_title_for(&page));
    ctx.insert("pages", &pages);

    // Only render a page template for known page numbers.  This prevents
    // user-supplied path segments from being passed to Tera::render.
    let template = if PAGES.iter().any(|(num, _)| *num == page) {
        if page == "170" {
            let weather: String = match state
                .http
                .get(&state.weather_url)
                .header("User-Agent", "curl/8.0")
                .send()
                .await
            {
                Ok(r) => r
                    .text()
                    .await
                    .unwrap_or_else(|_| "Wetterdaten nicht verfügbar".into()),
                Err(_) => "Wetterdaten nicht verfügbar".into(),
            };
            ctx.insert("weather", weather.trim());

            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            ctx.insert("forecast", &build_forecast(now_secs));
        }
        format!("{}.html", page)
    } else {
        "404.html".to_string()
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
        .find(|(num, _)| *num == page)
        .map(|(_, title)| *title)
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

    /// Build an app with explicit external URL overrides.
    fn test_app_with_urls(weather_url: &str, rss_url: &str) -> Router {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let pattern = format!("{}/templates/**/*.html", manifest);
        let tera = Tera::new(&pattern).expect("failed to parse templates");
        build_router(AppState {
            tera: Arc::new(tera),
            http: reqwest::Client::new(),
            weather_url: weather_url.into(),
            rss_url: rss_url.into(),
        })
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
        assert_eq!(page_title_for("300"), "Fanseite");
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
        for (num, _) in PAGES {
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
    /// unreachable, falling back to the "Wetterdaten nicht verfügbar" string.
    #[tokio::test]
    async fn page_170_renders_with_fallback_weather() {
        // 127.0.0.1:1 is guaranteed to refuse connections immediately.
        let resp = test_app_with_urls("http://127.0.0.1:1/weather", "http://127.0.0.1:1/rss")
            .oneshot(Request::get("/170").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("Wetterdaten nicht verfügbar"));
        assert!(body.contains("Vorhersage"));
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
}
