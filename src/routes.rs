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

//! Router construction and the RSS proxy.

use crate::guestbook_web::{guestbook_page, guestbook_post};
use crate::pages::{game_subpage_handler, page_handler};
use crate::state::AppState;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Redirect,
    routing::{get, post},
};
use tower_http::services::ServeDir;

/// Constructs the application [`Router`].
///
/// Extracted from `main` so that tests can build the router without binding a
/// real TCP socket.
pub(crate) fn build_router(state: AppState) -> Router {
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

/// Proxies the podcast archive RSS feed.
///
/// Fetching the feed server-side avoids browser CORS restrictions that would
/// otherwise block the client-side JavaScript from reading the response.
/// Returns `502 Bad Gateway` if the upstream is unreachable or returns an
/// error.
pub(crate) async fn rss_proxy(
    State(state): State<AppState>,
) -> Result<(HeaderMap, String), StatusCode> {
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
    use crate::testutil::{body_string, test_app, test_app_with_urls};
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

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

    // ── Guestbook integration tests ───────────────────────────────────────

    // ── Game sub-page tests ───────────────────────────────────────────────
}
