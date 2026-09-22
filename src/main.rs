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
//!
//! This file is startup wiring only.  Domain logic lives in the sibling
//! modules: [`pages`] renders page templates, [`weather`] backs page 170,
//! [`guestbook_web`] serves the guestbook, [`routes`] assembles the router,
//! and [`state`] holds the shared [`state::AppState`].

mod guestbook;
mod guestbook_web;
mod pages;
mod routes;
mod state;
#[cfg(test)]
mod testutil;
mod weather;

use routes::build_router;
use state::{AppState, WeatherCache, build_tera};
use std::sync::{Arc, Mutex};

/// Entry point.  Compiles templates, builds the router, and starts the server.
#[tokio::main]
async fn main() {
    let tera = build_tera("templates/**/*.html");
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

    let addr = std::env::var("FUNKFABRIK_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("FUNKFABRIK*B listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
