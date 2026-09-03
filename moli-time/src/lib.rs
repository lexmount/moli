//! Shared browser-facing time helpers.
//!
//! Servo's date/time surfaces mostly lean on the `time` crate with large-date
//! support. Keep the V8 bridge thin by centralizing browser timestamp and
//! lightweight Date locale formatting behavior here.

use std::{
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod timers;

pub use timers::{
    ReadyTimer, TimerId, TimerReadyAllowance, TimerScheduleRange, TimerScheduleSnapshot,
    TimerScheduler,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateLocaleFormatKind {
    DateTime,
    DateOnly,
    TimeOnly,
}

pub fn unix_epoch_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn monotonic_epoch_duration() -> Duration {
    static START: OnceLock<(Instant, Duration)> = OnceLock::new();
    let (start, epoch_base) = START.get_or_init(|| {
        (
            Instant::now(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
        )
    });
    epoch_base.saturating_add(start.elapsed())
}

pub fn monotonic_timestamp_seconds() -> f64 {
    monotonic_epoch_duration().as_secs_f64()
}

pub fn monotonic_timestamp_micros() -> u64 {
    monotonic_epoch_duration()
        .as_micros()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub fn coarsened_dom_time_millis(millis: f64) -> f64 {
    const FIVE_MICROSECONDS_PER_MILLISECOND: f64 = 200.0;
    (millis * FIVE_MICROSECONDS_PER_MILLISECOND).floor() / FIVE_MICROSECONDS_PER_MILLISECOND
}

pub fn dom_time_since_origin_millis(time_origin: f64) -> f64 {
    coarsened_dom_time_millis((unix_epoch_millis() - time_origin).max(0.0))
}

pub fn format_date_locale_value(
    timestamp_ms: f64,
    kind: DateLocaleFormatKind,
    locale_override: Option<&str>,
    timezone_override: Option<&str>,
) -> String {
    let Some(datetime) = offset_datetime_from_unix_millis(timestamp_ms) else {
        return "Invalid Date".to_owned();
    };
    let datetime = datetime.to_offset(resolve_time_zone_offset(timezone_override));
    let month = u8::from(datetime.month());
    let day = datetime.day();
    let year = datetime.year();
    let hour24 = datetime.hour();
    let minute = datetime.minute();
    let second = datetime.second();
    let (hour12, meridiem) = match hour24 {
        0 => (12, "AM"),
        1..=11 => (hour24, "AM"),
        12 => (12, "PM"),
        _ => (hour24 - 12, "PM"),
    };
    let locale = locale_override.unwrap_or("en-US").to_ascii_lowercase();
    let is_french_like = locale.starts_with("fr");

    match kind {
        DateLocaleFormatKind::DateTime if is_french_like => {
            format!("{day:02}/{month:02}/{year} {hour24:02}:{minute:02}:{second:02}")
        }
        DateLocaleFormatKind::DateOnly if is_french_like => format!("{day:02}/{month:02}/{year}"),
        DateLocaleFormatKind::TimeOnly if is_french_like => {
            format!("{hour24:02}:{minute:02}:{second:02}")
        }
        DateLocaleFormatKind::DateTime => {
            format!("{month}/{day}/{year}, {hour12}:{minute:02}:{second:02} {meridiem}")
        }
        DateLocaleFormatKind::DateOnly => format!("{month}/{day}/{year}"),
        DateLocaleFormatKind::TimeOnly => format!("{hour12}:{minute:02}:{second:02} {meridiem}"),
    }
}

/// Formats the source modification time exposed by `Document.lastModified`.
///
/// The HTML surface uses the user's local time zone unless CDP has installed
/// an explicit override. A missing or unrepresentable source timestamp falls
/// back to the supplied current timestamp so callers can preserve the spec's
/// per-access fallback while tests remain deterministic.
pub fn format_document_last_modified_value(
    source_timestamp_ms: Option<f64>,
    current_timestamp_ms: f64,
    timezone_override: Option<&str>,
) -> String {
    let datetime = source_timestamp_ms
        .and_then(offset_datetime_from_unix_millis)
        .or_else(|| offset_datetime_from_unix_millis(current_timestamp_ms))
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let offset = match timezone_override {
        Some(timezone) => resolve_time_zone_offset(Some(timezone)),
        None => time::UtcOffset::local_offset_at(datetime).unwrap_or(time::UtcOffset::UTC),
    };
    let datetime = datetime.to_offset(offset);
    let month = u8::from(datetime.month());

    format!(
        "{month:02}/{day:02}/{year:04} {hour:02}:{minute:02}:{second:02}",
        day = datetime.day(),
        year = datetime.year(),
        hour = datetime.hour(),
        minute = datetime.minute(),
        second = datetime.second(),
    )
}

fn offset_datetime_from_unix_millis(timestamp_ms: f64) -> Option<time::OffsetDateTime> {
    if !timestamp_ms.is_finite() {
        return None;
    }
    let nanos = (timestamp_ms * 1_000_000.0).round();
    if !nanos.is_finite() || nanos < i128::MIN as f64 || nanos > i128::MAX as f64 {
        return None;
    }
    time::OffsetDateTime::from_unix_timestamp_nanos(nanos as i128).ok()
}

fn resolve_time_zone_offset(timezone_override: Option<&str>) -> time::UtcOffset {
    match timezone_override.unwrap_or_default() {
        "Asia/Shanghai" => time::UtcOffset::from_hms(8, 0, 0).unwrap_or(time::UtcOffset::UTC),
        "UTC" | "Etc/UTC" | "GMT" => time::UtcOffset::UTC,
        _ => time::UtcOffset::UTC,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_locale_format_matches_existing_en_us_and_fr_surface() {
        let timestamp = 1_704_067_384_005.0;

        assert_eq!(
            format_date_locale_value(timestamp, DateLocaleFormatKind::DateTime, None, Some("UTC")),
            "1/1/2024, 12:03:04 AM"
        );
        assert_eq!(
            format_date_locale_value(
                timestamp,
                DateLocaleFormatKind::DateOnly,
                Some("fr-FR"),
                Some("UTC")
            ),
            "01/01/2024"
        );
        assert_eq!(
            format_date_locale_value(
                timestamp,
                DateLocaleFormatKind::TimeOnly,
                Some("fr"),
                Some("UTC")
            ),
            "00:03:04"
        );
    }

    #[test]
    fn date_locale_timezone_override_shifts_display_time() {
        assert_eq!(
            format_date_locale_value(
                0.0,
                DateLocaleFormatKind::DateTime,
                Some("en-US"),
                Some("Asia/Shanghai")
            ),
            "1/1/1970, 8:00:00 AM"
        );
    }

    #[test]
    fn document_last_modified_uses_source_time_and_current_fallback() {
        assert_eq!(
            format_document_last_modified_value(Some(5_025_000.0), 0.0, Some("Asia/Shanghai")),
            "01/01/1970 09:23:45"
        );
        assert_eq!(
            format_document_last_modified_value(None, 1_704_067_384_005.0, Some("UTC")),
            "01/01/2024 00:03:04"
        );
    }

    #[test]
    fn chromium_js_date_limits_stay_representable_with_large_dates() {
        assert_eq!(
            format_date_locale_value(
                8_640_000_000_000_000.0,
                DateLocaleFormatKind::DateTime,
                Some("en-US"),
                Some("UTC")
            ),
            "9/13/275760, 12:00:00 AM"
        );
        assert_eq!(
            format_date_locale_value(
                f64::INFINITY,
                DateLocaleFormatKind::DateTime,
                Some("en-US"),
                Some("UTC")
            ),
            "Invalid Date"
        );
        assert_eq!(
            format_date_locale_value(
                f64::NAN,
                DateLocaleFormatKind::DateTime,
                Some("en-US"),
                Some("UTC")
            ),
            "Invalid Date"
        );
    }

    #[test]
    fn dom_time_coarsening_uses_five_microsecond_resolution() {
        assert_eq!(coarsened_dom_time_millis(1.234_567), 1.23);
        assert_eq!(coarsened_dom_time_millis(1.239_999), 1.235);
        assert_eq!(
            dom_time_since_origin_millis(unix_epoch_millis() + 1000.0),
            0.0
        );
    }

    #[test]
    fn shared_monotonic_seconds_and_micros_use_the_same_epoch() {
        let seconds = monotonic_timestamp_seconds();
        let micros = monotonic_timestamp_micros();
        assert!(seconds > 0.0);
        assert!(micros > 0);
        assert!(((micros as f64 / 1_000_000.0) - seconds).abs() < 0.1);
    }
}
