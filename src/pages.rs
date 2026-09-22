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

//! The page table, page rendering, and the game sub-pages.

use crate::state::AppState;
use crate::weather;
use axum::{
    extract::{Path, State},
    response::Html,
};
use tera::Context;

/// All navigable pages, in display order.
///
/// Each tuple is `(page_number, human-readable title, blink)`.  The page
/// number is both the URL path segment (`/100`) and the Tera template name
/// (`100.html`).  `blink` causes the number to flash in the nav bar — used
/// to highlight special or time-limited pages.
pub(crate) const PAGES: &[(&str, &str, bool)] = &[
    ("100", "Startseite", false),
    ("101", "Radio hören", false),
    ("170", "Wettermagazin", false),
    ("300", "20 Jahre Brutto", true),
    ("404", "Fanseite", false),
    ("666", "Kontakt", false),
    ("777", "Spiele", false),
    ("999", "Impressum", false),
];

/// Builds the nav-bar page list handed to every template.
///
/// Every handler needs the same list, so it is built here rather than
/// repeated at each call site.
pub(crate) fn nav_pages() -> Vec<serde_json::Value> {
    PAGES
        .iter()
        .map(|(num, title, blink)| serde_json::json!({"num": num, "title": title, "blink": blink}))
        .collect()
}

/// Renders `template` with `ctx`, falling back to an inline error string.
///
/// Every handler needs the same fallback: a template that fails to render
/// must not take the process down, and must not surface a Tera error to the
/// visitor.
pub(crate) fn render(state: &AppState, template: &str, ctx: &Context) -> Html<String> {
    Html(
        state
            .tera
            .render(template, ctx)
            .unwrap_or_else(|_| "<h1 style='color:#FC0204'>PAGE NOT FOUND</h1>".into()),
    )
}

/// Renders a page by number.
///
/// Validates `page` against [`PAGES`] before constructing the template name,
/// so only known page numbers are ever passed to the renderer.  Unknown pages
/// render `not_found.html` directly.  Falls back to an inline error string if
/// even that template fails.
///
/// Note that `404` is a *content* page in [`PAGES`] ("Fanseite"), not the
/// error page; the error template is `not_found.html` (`SPEC.md` D3).
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
pub(crate) async fn page_handler(
    Path(page): Path<String>,
    State(state): State<AppState>,
) -> Html<String> {
    let mut ctx = Context::new();
    ctx.insert("current_page", &page);
    ctx.insert("page_title", page_title_for(&page));
    ctx.insert("pages", &nav_pages());

    // Only render a page template for known page numbers.  This prevents
    // user-supplied path segments from being passed to Tera::render.
    let template = if PAGES.iter().any(|(num, _, _)| *num == page) {
        if page == "170" {
            let now = weather::now_secs();
            ctx.insert("weather", &weather::current_weather(&state, now).await);
            ctx.insert("forecast", &weather::build_forecast(now));
        }
        format!("{}.html", page)
    } else {
        "not_found.html".to_string()
    };

    render(&state, &template, &ctx)
}

/// Returns the human-readable title for a page number, or `"???"` if unknown.
///
/// Performs a linear scan of [`PAGES`]; acceptable given the tiny page count.
pub(crate) fn page_title_for(page: &str) -> &'static str {
    PAGES
        .iter()
        .find(|(num, _, _)| *num == page)
        .map(|(_, title, _)| *title)
        .unwrap_or("???")
}

/// Renders a game sub-page (`GET /777/{game}`).
///
/// Valid game names are `tetris`, `invaders`, and `snake`; anything else
/// falls through to `not_found.html`.  `current_page` is always `"777"` so
/// the *Spiele* nav entry stays highlighted.
pub(crate) async fn game_subpage_handler(
    Path(game): Path<String>,
    State(state): State<AppState>,
) -> Html<String> {
    let valid = ["tetris", "invaders", "snake"];
    let template = if valid.contains(&game.as_str()) {
        format!("777_{}.html", game)
    } else {
        "not_found.html".to_string()
    };

    let mut ctx = Context::new();
    ctx.insert("current_page", "777");
    ctx.insert("page_title", "Spiele");
    ctx.insert("pages", &nav_pages());

    render(&state, &template, &ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{body_string, test_app, test_app_with_urls};
    use axum::{body::Body, http::Request, http::StatusCode};
    use tower::ServiceExt;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

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
        assert!(
            body.contains("°C") || body.contains("°F"),
            "fallback should contain a temperature"
        );
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
        assert!(
            body.contains("°C") || body.contains("°F"),
            "should fall back to generated weather"
        );
        assert!(
            !body.contains("quota"),
            "quota error message should not appear in output"
        );
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
    /// through to `not_found.html`.
    #[tokio::test]
    async fn path_traversal_returns_404_template() {
        for path in ["/../../etc/passwd", "/../secret", "/100%2F..%2Fetc"] {
            let resp = test_app()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            // Either the router rejects it (non-200) or our handler serves
            // not_found.html (`404` is a content page; see SPEC.md D3).
            let status = resp.status();
            if status == StatusCode::OK {
                let body = body_string(resp.into_body()).await;
                assert!(
                    body.contains("nicht gefunden") || body.contains("PAGE NOT FOUND"),
                    "traversal path {path} should render not_found.html, got: {body:.80}"
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

    #[tokio::test]
    async fn game_pages_return_200_with_canvas() {
        for game in ["tetris", "invaders", "snake"] {
            let resp = test_app()
                .oneshot(
                    Request::get(format!("/777/{}", game))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "game {game} should be 200");
            let body = body_string(resp.into_body()).await;
            assert!(
                body.contains("game-canvas"),
                "game {game} should contain canvas element"
            );
        }
    }

    #[tokio::test]
    async fn unknown_game_returns_not_found_content() {
        let resp = test_app()
            .oneshot(Request::get("/777/pong").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("nicht gefunden") || body.contains("PAGE NOT FOUND"));
    }
}
