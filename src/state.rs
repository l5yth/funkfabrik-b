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

//! Shared application state and template-engine construction.

use crate::guestbook::GuestEntry;
use std::sync::{Arc, Mutex};
use tera::Tera;

/// Cached weather data fetched from wttr.in.
///
/// The cache is shared across all requests via [`AppState`].  A [`Mutex`] is
/// used rather than a `tokio::sync::Mutex` because the critical section is
/// purely in-memory (no `.await` points while the lock is held).
pub(crate) struct WeatherCache {
    /// Last successfully validated weather string (wttr.in `format=2` output).
    pub(crate) value: String,
    /// Unix timestamp (seconds) when `value` was populated.  Starts at `0` so
    /// the first request always triggers a fetch.
    pub(crate) fetched_at: u64,
}

/// Shared application state passed to every handler via Axum's
/// [`axum::extract::State`] extractor.
#[derive(Clone)]
pub(crate) struct AppState {
    /// Compiled Tera template engine, wrapped in an [`Arc`] so it can be shared
    /// cheaply across async tasks without cloning the underlying template data.
    pub(crate) tera: Arc<Tera>,
    /// Reusable HTTP client for outbound requests (weather API, RSS proxy).
    /// [`reqwest::Client`] is cheaply cloneable and manages a connection pool
    /// internally, so a single instance is shared for the lifetime of the server.
    pub(crate) http: reqwest::Client,
    /// URL for the wttr.in current-conditions endpoint.  Stored here so tests
    /// can substitute a local mock server without patching the binary.
    pub(crate) weather_url: String,
    /// URL for the podcast archive RSS feed.  Stored here so tests can
    /// substitute a local mock server without patching the binary.
    pub(crate) rss_url: String,
    /// Server-side cache for the wttr.in response.  Shared across all handlers
    /// via `Arc`; refreshed at most once per [`WEATHER_CACHE_TTL_SECS`].
    pub(crate) weather_cache: Arc<Mutex<WeatherCache>>,
    /// In-memory guestbook entries, mirroring the JSON file on disk.
    /// Protected by a [`Mutex`] so concurrent requests can safely append.
    pub(crate) guestbook: Arc<Mutex<Vec<GuestEntry>>>,
    /// Path to the guestbook JSON file.  Stored here so tests can use a
    /// temporary path without touching the production data directory.
    pub(crate) guestbook_path: std::path::PathBuf,
}

/// How long (in seconds) a cached wttr.in response is considered fresh.
pub(crate) const WEATHER_CACHE_TTL_SECS: u64 = 3600;

/// Builds the Tera engine from a glob pattern.
///
/// Tera 2 split construction from loading: [`Tera::new`] takes no arguments
/// and cannot fail, and templates are pulled in afterwards by
/// [`Tera::load_from_glob`], which requires the `glob_fs` feature.  Both the
/// server and the test harness go through this function so the engine is
/// only ever built one way.
///
/// # Panics
///
/// Panics if a matched template fails to parse, and if the glob matched no
/// templates at all.
///
/// The empty case has to be checked explicitly: Tera treats a glob that
/// matches nothing as success and hands back an empty engine, so a mistyped
/// pattern or a wrong working directory would otherwise start cleanly and
/// serve the inline fallback string with HTTP 200 on every route.  Failing at
/// startup surfaces that immediately.
pub(crate) fn build_tera(pattern: &str) -> Tera {
    let mut tera = Tera::new();
    tera.load_from_glob(pattern)
        .expect("failed to parse templates");
    assert!(
        tera.get_template_names().next().is_some(),
        "no templates matched `{pattern}` (run the server from the project root)"
    );
    tera
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::PAGES;

    /// ACCEPTANCE R2 — a glob that matches nothing is fatal, not silent.
    ///
    /// Tera's `load_from_glob` returns `Ok(())` for a glob matching no files,
    /// so without the explicit check in [`build_tera`] a mistyped pattern
    /// would serve `PAGE NOT FOUND` with HTTP 200 on every route instead of
    /// failing at startup.
    #[test]
    #[should_panic(expected = "no templates matched")]
    fn build_tera_panics_when_glob_matches_nothing() {
        // Fixed name, not a unique one: the panic unwinds past any cleanup,
        // so a unique path per run would litter the temp directory.
        let empty = std::env::temp_dir().join("fb_empty_templates");
        std::fs::create_dir_all(&empty).unwrap();
        build_tera(&format!("{}/**/*.html", empty.display()));
    }

    /// ACCEPTANCE R2 — the pattern `main` itself passes must be the one that
    /// resolves the templates, relative to the project root.
    ///
    /// `tera_engine_loads_every_template` builds from an absolute
    /// `CARGO_MANIFEST_DIR` path, so on its own it would not catch a typo in
    /// the literal `main` uses.  This runs that exact literal.
    #[test]
    fn build_tera_accepts_the_literal_pattern_main_uses() {
        // Cargo runs test binaries with the working directory set to the
        // package root, which is the same place the server is run from, so
        // the literal resolves here exactly as it does in `main`.  Asserted
        // rather than forced: `set_current_dir` is process-global and would
        // race the other tests in this module.
        assert_eq!(
            std::env::current_dir().unwrap(),
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()),
            "test harness cwd is not the package root"
        );

        let tera = build_tera("templates/**/*.html");
        assert!(
            tera.get_template_names().any(|n| n == "base.html"),
            "main's own glob literal resolved nothing from the project root"
        );
    }

    /// ACCEPTANCE R2 — the engine must actually hold every template.
    ///
    /// Tera 2's `Tera::new()` cannot fail, so forgetting `load_from_glob`
    /// would no longer blow up at startup: every route would silently fall
    /// back to the inline `PAGE NOT FOUND` string.  This asserts the glob
    /// really loaded, by name, for every template the router can reach.
    #[test]
    fn tera_engine_loads_every_template() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let tera = build_tera(&format!("{}/templates/**/*.html", manifest));

        let mut expected: Vec<String> = vec!["base.html".to_string(), "not_found.html".to_string()];
        expected.extend(PAGES.iter().map(|(num, _, _)| format!("{}.html", num)));
        expected.extend(
            ["tetris", "invaders", "snake"]
                .iter()
                .map(|g| format!("777_{}.html", g)),
        );

        for name in &expected {
            assert!(
                tera.get_template_names().any(|n| n == name),
                "template `{}` missing from the Tera engine",
                name
            );
        }
    }
}
