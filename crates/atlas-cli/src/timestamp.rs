//! Minimal UTC timestamp formatting, so `components.yaml::generated_at`
//! gets an ISO-8601 value without pulling `chrono` / `time` into the
//! dependency tree.
//!
//! The one exported helper ([`format_utc_rfc3339`]) returns a string
//! shaped like `YYYY-MM-DDTHH:MM:SSZ`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Format `time` as `YYYY-MM-DDTHH:MM:SSZ` (UTC, seconds precision).
/// Pre-epoch inputs are clamped to the epoch — Atlas does not run on
/// machines with clocks before 1970.
pub fn format_utc_rfc3339(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, minute, second) = civil_from_unix_seconds(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert unix seconds to a civil `(year, month, day, hour, minute,
/// second)` tuple in UTC. Algorithm from Howard Hinnant's public-
/// domain `days_from_civil` / `civil_from_days` — reproduced here so
/// we don't pull a date crate into the dependency tree.
fn civil_from_unix_seconds(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let day_seconds = 86_400u64;
    let days = (secs / day_seconds) as i64;
    let time_of_day = secs % day_seconds;
    let hour = (time_of_day / 3600) as u32;
    let minute = ((time_of_day % 3600) / 60) as u32;
    let second = (time_of_day % 60) as u32;

    // Hinnant's `civil_from_days`: treats days as offset from
    // 1970-01-01, but internally shifts to an era anchored at
    // 0000-03-01 (which gives a clean 400-year cycle).
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };

    (year, m, d, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn format_epoch_is_1970() {
        assert_eq!(format_utc_rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn format_one_hour_after_epoch() {
        let t = UNIX_EPOCH + Duration::from_secs(3600);
        assert_eq!(format_utc_rfc3339(t), "1970-01-01T01:00:00Z");
    }

    #[test]
    fn format_mid_2026() {
        // 2026-04-24T00:00:00Z = 56 years + leap days
        // Use a known timestamp: 1_745_452_800 is 2025-04-24T00:00:00Z
        let t = UNIX_EPOCH + Duration::from_secs(1_745_452_800);
        assert_eq!(format_utc_rfc3339(t), "2025-04-24T00:00:00Z");
    }

    #[test]
    fn format_handles_leap_year_day() {
        // 2024-02-29T12:34:56Z = 1_709_210_096
        let t = UNIX_EPOCH + Duration::from_secs(1_709_210_096);
        assert_eq!(format_utc_rfc3339(t), "2024-02-29T12:34:56Z");
    }

    #[test]
    fn format_handles_year_boundary() {
        // 1999-12-31T23:59:59Z = 946_684_799
        let t = UNIX_EPOCH + Duration::from_secs(946_684_799);
        assert_eq!(format_utc_rfc3339(t), "1999-12-31T23:59:59Z");
        // 2000-01-01T00:00:00Z = 946_684_800
        let t = UNIX_EPOCH + Duration::from_secs(946_684_800);
        assert_eq!(format_utc_rfc3339(t), "2000-01-01T00:00:00Z");
    }
}
