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

//! HTTP surface for the guestbook: the form, the entry list, and the
//! POST-redirect-GET submission flow.  Persistence lives in
//! [`crate::guestbook`].

use crate::guestbook::{self, GuestEntry};
use crate::pages::{nav_pages, page_title_for, render};
use crate::state::AppState;
use crate::weather::now_secs;
use axum::{
    Form,
    extract::{Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;
use std::collections::HashMap;
use tera::Context;

/// Form data submitted to `POST /666/send`.
#[derive(Deserialize)]
pub(crate) struct GuestbookForm {
    /// Visitor-supplied display name.  Optional; an empty value renders as
    /// "Anonym" (`SPEC.md` D9).
    pub(crate) name: String,
    /// Body of the entry.  Rejected when empty or when it contains `://`
    /// (`SPEC.md` D10).
    pub(crate) message: String,
    /// Captcha answer.  Must be `b`, case-insensitive (`SPEC.md` D10).
    pub(crate) captcha: String,
}

/// Renders the guestbook page (`GET /666`).
///
/// Passes flash state via query parameters (`?success=1` or `?error=captcha` /
/// `?error=empty`) set by [`guestbook_post`] after a redirect.  Entries are
/// presented newest-first.
pub(crate) async fn guestbook_page(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
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
    ctx.insert("pages", &nav_pages());
    ctx.insert("entries", &entries);
    ctx.insert("flash_success", &flash_success);
    ctx.insert("flash_error", &flash_error);

    render(&state, "666.html", &ctx)
}

/// Handles guestbook form submission (`POST /666/send`).
///
/// Validates the captcha (answer must be `"B"`, case-insensitive) and that
/// the message is non-empty, then appends the entry and persists it to disk.
/// Always responds with a redirect so a browser refresh does not re-submit.
pub(crate) async fn guestbook_post(
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

    let entry = GuestEntry {
        name,
        message,
        timestamp_secs: now_secs(),
    };

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{body_string, post_form, temp_guestbook_path, test_app, test_app_full};
    use axum::{body::Body, http::Request, http::StatusCode};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

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
        let app = test_app_full(
            "http://127.0.0.1:1/weather",
            "http://127.0.0.1:1/rss",
            gb_path.clone(),
        );

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

    // ── SPEC D9: guestbook persistence behaviours ─────────────────────────

    /// SPEC D9 — an entry submitted with an empty name renders as `Anonym`.
    ///
    /// The form marks the name field optional, so this is the common path for
    /// a drive-by post, not an edge case.
    #[tokio::test]
    async fn guestbook_empty_name_renders_as_anonym() {
        let gb_path = temp_guestbook_path();
        let app = test_app_full(
            "http://127.0.0.1:1/weather",
            "http://127.0.0.1:1/rss",
            gb_path.clone(),
        );

        let post_resp = app
            .clone()
            .oneshot(post_form("/666/send", "name=&message=Ohne+Namen&captcha=B"))
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::SEE_OTHER);

        let body = body_string(
            app.oneshot(Request::get("/666").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .into_body(),
        )
        .await;

        assert!(
            body.contains("Anonym"),
            "empty name did not render as Anonym"
        );
        assert!(body.contains("Ohne Namen"), "message not in body");

        let _ = std::fs::remove_file(&gb_path);
    }

    /// SPEC D9 — a failed persist is logged, not fatal, and the entry stays
    /// visible for the rest of the process lifetime.
    ///
    /// The failure is provoked by pointing `guestbook_path` at a directory:
    /// `create_dir_all` on its parent succeeds, then `fs::write` fails with
    /// `EISDIR`.  The handler must still redirect to the success page and the
    /// in-memory vector must still hold the entry.
    #[tokio::test]
    async fn guestbook_survives_a_failing_save() {
        let dir_as_path = std::env::temp_dir().join(format!(
            "fb_gb_dir_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir_as_path).unwrap();

        // Writing to a directory path must fail, or the test proves nothing.
        assert!(
            guestbook::save(&dir_as_path, &[]).is_err(),
            "expected saving to a directory path to fail"
        );

        let app = test_app_full(
            "http://127.0.0.1:1/weather",
            "http://127.0.0.1:1/rss",
            dir_as_path.clone(),
        );

        let post_resp = app
            .clone()
            .oneshot(post_form(
                "/666/send",
                "name=Trotzdem&message=Bleibt+da&captcha=B",
            ))
            .await
            .unwrap();
        assert_eq!(
            post_resp.status(),
            StatusCode::SEE_OTHER,
            "a save failure must not fail the request"
        );
        assert_eq!(
            post_resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok()),
            Some("/666?success=1"),
        );

        let body = body_string(
            app.oneshot(Request::get("/666").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .into_body(),
        )
        .await;
        assert!(
            body.contains("Bleibt da"),
            "entry lost after a failed persist"
        );

        let _ = std::fs::remove_dir_all(&dir_as_path);
    }

    // ── Regression guards (ACCEPTANCE.md Layer D) ──────────────────────────

    /// ACCEPTANCE R3 — guestbook input stays autoescaped across a Tera major.
    ///
    /// Escaping is the only thing between a guestbook post and stored XSS,
    /// and Tera 2 changed its escape set, so the behaviour is pinned here
    /// rather than inferred from release notes.
    #[tokio::test]
    async fn guestbook_escapes_html_in_entries() {
        let gb_path = temp_guestbook_path();
        let app = test_app_full(
            "http://127.0.0.1:1/weather",
            "http://127.0.0.1:1/rss",
            gb_path.clone(),
        );

        // `<script>alert("x&y's")</script>` — every metacharacter, no `://`
        // so the URL filter in `guestbook_post` does not reject it.
        let form = concat!(
            "name=%3Cb%3EEvil%3C%2Fb%3E",
            "&message=%3Cscript%3Ealert%28%22x%26y%27s%22%29%3C%2Fscript%3E",
            "&captcha=B",
        );
        let post_resp = app
            .clone()
            .oneshot(post_form("/666/send", form))
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::SEE_OTHER);

        let get_resp = app
            .oneshot(Request::get("/666").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = body_string(get_resp.into_body()).await;

        // Payload-specific: `base.html` legitimately carries a
        // `<script src=...>` tag, so match the injected call, not the tag.
        assert!(
            !body.contains("<script>alert("),
            "raw <script> from user input reached the page"
        );
        assert!(
            !body.contains("<b>Evil</b>"),
            "raw markup from the name field reached the page"
        );
        assert!(body.contains("&lt;script&gt;"), "`<` / `>` not escaped");
        assert!(body.contains("&amp;"), "`&` not escaped");
        assert!(
            body.contains("&quot;") || body.contains("&#34;"),
            "`\"` not escaped"
        );
        assert!(
            body.contains("&#39;") || body.contains("&#x27;"),
            "`'` not escaped"
        );

        let _ = std::fs::remove_file(&gb_path);
    }
}
