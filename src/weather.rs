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

//! Weather for page 170: wttr.in fetch with cache, and the decorative
//! generator that stands in when the upstream is unusable.

use crate::state::{AppState, WEATHER_CACHE_TTL_SECS};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Returns the current-conditions string for page 170.
///
/// Serves the cached value while it is fresher than
/// [`WEATHER_CACHE_TTL_SECS`], otherwise fetches wttr.in and validates the
/// response with [`looks_like_weather`], falling back to
/// [`generate_current_weather`].  The result is cached either way, so a
/// failing upstream is retried at most once per TTL window rather than on
/// every request.
///
/// The cache mutex is released before every `.await`, so no lock is held
/// across a suspension point.
pub(crate) async fn current_weather(state: &AppState, now_secs: u64) -> String {
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

    if let Some(w) = cached {
        return w;
    }

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
}

/// Seconds since the Unix epoch, saturating to 0 before 1970.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Computes the ISO weekday index (0 = Monday … 6 = Sunday) from a Unix
/// timestamp in seconds.
///
/// The Unix epoch (1970-01-01) was a Thursday, which is index 3 when Monday
/// is 0, so the formula is `(days_since_epoch + 3) % 7`.
pub(crate) fn weekday_from_secs(secs: u64) -> u64 {
    (secs / 86400 + 3) % 7
}

/// Advances a PCG-style LCG seed by one step and returns the upper 31 bits.
///
/// Uses the Knuth multiplicative hash constants to give good distribution.
/// Not cryptographically secure; suitable only for decorative randomness.
pub(crate) fn lcg_next(seed: &mut u64) -> u64 {
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
pub(crate) fn build_forecast(now_secs: u64) -> Vec<serde_json::Value> {
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
pub(crate) fn looks_like_weather(s: &str) -> bool {
    s.contains("°C") || s.contains("°F")
}

/// Generates a plausible current-conditions string using the LCG, as a
/// fallback when wttr.in is unreachable or returns a non-weather response.
///
/// Output format mirrors wttr.in `format=2`, e.g. `⛅  +14°C`.
pub(crate) fn generate_current_weather(now_secs: u64) -> String {
    const ICONS: [&str; 7] = ["☀", "🌤", "⛅", "🌦", "☁", "🌧", "⛈"];
    let mut seed = now_secs.wrapping_mul(2654435761); // different seed offset from forecast
    let icon = ICONS[lcg_next(&mut seed) as usize % ICONS.len()];
    let temp = lcg_next(&mut seed) as i64 % 15 + 8; // 8–22 °C
    format!("{}  +{}°C", icon, temp)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!looks_like_weather(
            "Sorry, we are out of quota for your IP."
        ));
        assert!(!looks_like_weather(""));
        assert!(!looks_like_weather("<html><body>Error</body></html>"));
    }

    #[test]
    fn generate_current_weather_contains_degree() {
        let w = generate_current_weather(12345678);
        assert!(
            looks_like_weather(&w),
            "generated weather should pass looks_like_weather: {w}"
        );
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
}
