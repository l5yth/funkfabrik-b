// Copyright (c) 2006-2026 afri & veit
// SPDX-License-Identifier: Apache-2.0

//! Guestbook persistence: entry type, JSON load/save, and timestamp formatting.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single guestbook entry as stored in the JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestEntry {
    /// Visitor-supplied display name; may be empty (render as "Anonym").
    pub name: String,
    /// The body of the guestbook entry.
    pub message: String,
    /// Unix timestamp (seconds since the Unix epoch) when the entry was submitted.
    pub timestamp_secs: u64,
}

/// Load all entries from the JSON file at `path`.
///
/// Returns an empty `Vec` if the file does not exist yet or cannot be parsed,
/// so a missing or corrupt file never crashes the server.
pub fn load(path: &Path) -> Vec<GuestEntry> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persist `entries` to the JSON file at `path`.
///
/// Creates parent directories if they do not exist.  The write is not atomic
/// (no temp-file swap) which is acceptable for a low-traffic guestbook.
pub fn save(path: &Path, entries: &[GuestEntry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(entries)
        .map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

/// Format a Unix timestamp (seconds since epoch) as `"DD.MM.YYYY HH:MM"` (UTC).
///
/// Uses pure integer arithmetic via Howard Hinnant's civil-calendar algorithm
/// so that no external date/time crate is required.
pub fn format_timestamp(secs: u64) -> String {
    let secs = secs as i64;
    let (y, m, d) = days_to_ymd(secs / 86400);
    let hh = (secs % 86400) / 3600;
    let mm = (secs % 3600) / 60;
    format!("{:02}.{:02}.{:04} {:02}:{:02}", d, m, y, hh, mm)
}

/// Convert a count of days since the Unix epoch to `(year, month, day)`.
///
/// Implements Howard Hinnant's civil calendar algorithm, which correctly
/// handles the full proleptic Gregorian calendar including leap years and
/// the 400-year cycle.
fn days_to_ymd(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── days_to_ymd ──────────────────────────────────────────────────────────

    #[test]
    fn epoch_is_1970_01_01() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn known_date_2024_03_15() {
        // 2024-03-15 is day 19797 since epoch (verified manually)
        assert_eq!(days_to_ymd(19_797), (2024, 3, 15));
    }

    #[test]
    fn leap_year_feb_29_2000() {
        // 2000-02-29 (leap day) — day 11_016 since epoch
        assert_eq!(days_to_ymd(11_016), (2000, 2, 29));
    }

    // ── format_timestamp ─────────────────────────────────────────────────────

    #[test]
    fn format_epoch() {
        assert_eq!(format_timestamp(0), "01.01.1970 00:00");
    }

    #[test]
    fn format_known_datetime() {
        // 2024-03-15 14:30:00 UTC = 19797 * 86400 + 52200 = 1_710_513_000
        assert_eq!(format_timestamp(1_710_513_000), "15.03.2024 14:30");
    }

    #[test]
    fn format_output_is_always_16_chars() {
        for secs in [0u64, 86400, 1_000_000_000, 1_700_000_000] {
            assert_eq!(
                format_timestamp(secs).len(),
                16,
                "bad length for secs={secs}"
            );
        }
    }

    // ── load / save ──────────────────────────────────────────────────────────

    #[test]
    fn load_returns_empty_for_missing_file() {
        assert!(load(Path::new("/nonexistent/path/guestbook_missing.json")).is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "fb_gb_roundtrip_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let entries = vec![
            GuestEntry {
                name: "Punk".into(),
                message: "Oi!".into(),
                timestamp_secs: 1_700_000_000,
            },
            GuestEntry {
                name: String::new(),
                message: "Anon war hier".into(),
                timestamp_secs: 1_700_000_001,
            },
        ];
        save(&path, &entries).expect("save failed");
        let loaded = load(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "Punk");
        assert_eq!(loaded[0].message, "Oi!");
        assert_eq!(loaded[0].timestamp_secs, 1_700_000_000);
        assert!(loaded[1].name.is_empty());
        assert_eq!(loaded[1].message, "Anon war hier");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_returns_empty_on_corrupt_json() {
        let path = std::env::temp_dir().join(format!(
            "fb_gb_corrupt_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"not valid json").unwrap();
        assert!(load(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "fb_gb_newdir_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("guestbook.json");
        save(&path, &[]).expect("save with new parent dir failed");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
