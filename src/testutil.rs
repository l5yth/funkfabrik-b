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

//! Test-only helpers shared by every module's `tests` block.
//!
//! Compiled only under `cfg(test)`: building an app against the real
//! templates, a unique guestbook path per test, and request/response
//! plumbing.

use crate::guestbook;
use crate::routes::build_router;
use crate::state::build_tera;
use crate::state::{AppState, WeatherCache};
use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Build an app backed by the real templates for integration tests.
pub(crate) fn test_app() -> Router {
    test_app_with_urls("http://127.0.0.1:1/weather", "http://127.0.0.1:1/rss")
}

/// Build an app with explicit external URL overrides and a unique temp
/// guestbook path per call (so tests don't share state on disk).
pub(crate) fn test_app_with_urls(weather_url: &str, rss_url: &str) -> Router {
    test_app_full(weather_url, rss_url, temp_guestbook_path())
}

/// Build an app with full control over all external dependencies.
pub(crate) fn test_app_full(
    weather_url: &str,
    rss_url: &str,
    gb_path: std::path::PathBuf,
) -> Router {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let pattern = format!("{}/templates/**/*.html", manifest);
    let tera = build_tera(&pattern);
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
pub(crate) fn temp_guestbook_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "fb_gb_test_{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

pub(crate) async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ── Pure unit tests ────────────────────────────────────────────────────

pub(crate) fn post_form(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap()
}
