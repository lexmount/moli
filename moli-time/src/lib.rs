//! Shared browser-facing time helpers.
//!
//! Servo's date/time surfaces mostly lean on the `time` crate with large-date
//! support. Keep the V8 bridge thin by centralizing browser timestamp and
//! lightweight Date locale formatting behavior here.

use std::{
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use temporal_rs::{
    PlainDateTime, TimeZone,
    options::Disambiguation,
    provider::{COMPILED_TZ_PROVIDER, TimeZoneProvider},
};

mod timers;

pub use timers::{ReadyTimer, TimerId, TimerReadyAllowance, TimerScheduler};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalDateFields {
    pub year: i32,
    pub month_zero_based: u8,
    pub day: u8,
    pub weekday_sunday_zero: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
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
        Some(timezone) => resolve_time_zone_offset(datetime, timezone),
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
    let whole_millis = timestamp_ms.trunc();
    if whole_millis < i128::MIN as f64 || whole_millis > i128::MAX as f64 {
        return None;
    }
    // Multiplying an epoch-sized millisecond value by 1e6 in f64 first loses
    // precision (current dates are already around 1.7e18 nanoseconds). Split
    // the value before scaling so integral ECMAScript Date milliseconds remain
    // exact and only the sub-millisecond remainder is rounded.
    let whole_nanos = (whole_millis as i128).checked_mul(1_000_000)?;
    let fractional_nanos = ((timestamp_ms - whole_millis) * 1_000_000.0).round() as i128;
    time::OffsetDateTime::from_unix_timestamp_nanos(whole_nanos.checked_add(fractional_nanos)?).ok()
}

fn unix_millis_from_offset_datetime(datetime: time::OffsetDateTime) -> f64 {
    const NANOS_PER_MILLISECOND: i128 = 1_000_000;
    let nanos = datetime.unix_timestamp_nanos();
    let whole_millis = nanos.div_euclid(NANOS_PER_MILLISECOND);
    let fractional_nanos = nanos.rem_euclid(NANOS_PER_MILLISECOND);
    whole_millis as f64 + fractional_nanos as f64 / NANOS_PER_MILLISECOND as f64
}

fn resolve_time_zone_offset(datetime: time::OffsetDateTime, timezone: &str) -> time::UtcOffset {
    let Some(offset_seconds) =
        time_zone_offset_seconds_at(datetime.unix_timestamp_nanos(), timezone)
    else {
        return time::UtcOffset::UTC;
    };
    time::UtcOffset::from_whole_seconds(offset_seconds).unwrap_or(time::UtcOffset::UTC)
}

/// Resolves the IANA time-zone offset used by local `Date` operations.
///
/// The lookup is timestamp-sensitive, so daylight-saving transitions follow
/// the same tzdb data used by Temporal instead of a fixed per-zone table.
pub fn time_zone_offset_seconds(timestamp_ms: f64, timezone: &str) -> Option<i32> {
    let datetime = offset_datetime_from_unix_millis(timestamp_ms)?;
    time_zone_offset_seconds_at(datetime.unix_timestamp_nanos(), timezone)
}

pub fn local_time_zone_offset_seconds(timestamp_ms: f64) -> Option<i32> {
    let datetime = offset_datetime_from_unix_millis(timestamp_ms)?;
    Some(
        time::UtcOffset::local_offset_at(datetime)
            .unwrap_or(time::UtcOffset::UTC)
            .whole_seconds(),
    )
}

/// Returns whether the compiled IANA database recognizes a time-zone name.
///
/// Protocol surfaces must reject an unknown override before publishing it to
/// a target. Date formatting deliberately has a UTC fallback for defensive
/// rendering, so using an offset lookup as validation after committing state
/// would turn an invalid CDP command into a misleading success.
pub fn is_valid_time_zone_identifier(timezone: &str) -> bool {
    !timezone.is_empty() && COMPILED_TZ_PROVIDER.get(timezone.as_bytes()).is_ok()
}

pub fn local_date_fields(timestamp_ms: f64, timezone: Option<&str>) -> Option<LocalDateFields> {
    let datetime = offset_datetime_from_unix_millis(timestamp_ms)?;
    let offset = timezone
        .map(|timezone| resolve_time_zone_offset(datetime, timezone))
        .unwrap_or_else(|| {
            time::UtcOffset::local_offset_at(datetime).unwrap_or(time::UtcOffset::UTC)
        });
    let datetime = datetime.to_offset(offset);
    Some(LocalDateFields {
        year: datetime.year(),
        month_zero_based: u8::from(datetime.month()).saturating_sub(1),
        day: datetime.day(),
        weekday_sunday_zero: datetime.weekday().number_days_from_sunday(),
        hour: datetime.hour(),
        minute: datetime.minute(),
        second: datetime.second(),
        millisecond: datetime.millisecond(),
    })
}

/// Projects an instant into a UTC-shaped millisecond value whose fields are
/// the instant's local wall-clock fields in `timezone`.
///
/// V8's UTC Date setters already implement ECMAScript argument conversion and
/// field balancing. The renderer uses this projection as their temporary input
/// and converts the resulting fields back with
/// [`epoch_millis_for_local_wall_clock`], avoiding a second implementation of
/// Date's normalization rules.
pub fn local_wall_clock_as_utc_millis(timestamp_ms: f64, timezone: &str) -> Option<f64> {
    let datetime = offset_datetime_from_unix_millis(timestamp_ms)?;
    let offset = resolve_time_zone_offset(datetime, timezone);
    let wall_clock = datetime
        .to_offset(offset)
        .replace_offset(time::UtcOffset::UTC);
    Some(unix_millis_from_offset_datetime(wall_clock))
}

/// Interprets the UTC fields of `wall_clock_utc_ms` as a local wall-clock time
/// in `timezone` and returns the corresponding ECMAScript epoch milliseconds.
///
/// Temporal's `Compatible` disambiguation matches legacy Date behavior: it
/// chooses the earlier instant for a repeated clock time and advances across a
/// daylight-saving gap.
pub fn epoch_millis_for_local_wall_clock(wall_clock_utc_ms: f64, timezone: &str) -> Option<f64> {
    let fields = offset_datetime_from_unix_millis(wall_clock_utc_ms)?;
    let date_time = PlainDateTime::try_new_iso(
        fields.year(),
        u8::from(fields.month()),
        fields.day(),
        fields.hour(),
        fields.minute(),
        fields.second(),
        fields.millisecond(),
        0,
        0,
    )
    .ok()?;
    let time_zone = TimeZone::try_from_identifier_str(timezone).ok()?;
    let zoned = date_time
        .to_zoned_date_time(time_zone, Disambiguation::Compatible)
        .ok()?;
    Some(zoned.epoch_milliseconds() as f64)
}

pub fn format_date_local_string(timestamp_ms: f64, timezone: Option<&str>) -> String {
    let Some(fields) = local_date_fields(timestamp_ms, timezone) else {
        return "Invalid Date".to_owned();
    };
    format!(
        "{} {} {:02} {:04} {}",
        weekday_name(fields.weekday_sunday_zero),
        month_name(fields.month_zero_based),
        fields.day,
        fields.year,
        format_date_local_time_string(timestamp_ms, timezone),
    )
}

pub fn format_date_local_date_string(timestamp_ms: f64, timezone: Option<&str>) -> String {
    let Some(fields) = local_date_fields(timestamp_ms, timezone) else {
        return "Invalid Date".to_owned();
    };
    format!(
        "{} {} {:02} {:04}",
        weekday_name(fields.weekday_sunday_zero),
        month_name(fields.month_zero_based),
        fields.day,
        fields.year,
    )
}

pub fn format_date_local_time_string(timestamp_ms: f64, timezone: Option<&str>) -> String {
    let Some(fields) = local_date_fields(timestamp_ms, timezone) else {
        return "Invalid Date".to_owned();
    };
    let offset_seconds = timezone
        .and_then(|timezone| time_zone_offset_seconds(timestamp_ms, timezone))
        .or_else(|| local_time_zone_offset_seconds(timestamp_ms))
        .unwrap_or_default();
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let offset_minutes = offset_seconds.unsigned_abs() / 60;
    let offset_hours = offset_minutes / 60;
    let offset_minutes = offset_minutes % 60;
    format!(
        "{:02}:{:02}:{:02} GMT{sign}{offset_hours:02}{offset_minutes:02}",
        fields.hour, fields.minute, fields.second,
    )
}

fn weekday_name(weekday_sunday_zero: u8) -> &'static str {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .get(usize::from(weekday_sunday_zero))
        .copied()
        .unwrap_or("Invalid")
}

fn month_name(month_zero_based: u8) -> &'static str {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .get(usize::from(month_zero_based))
    .copied()
    .unwrap_or("Invalid")
}

fn time_zone_offset_seconds_at(epoch_nanoseconds: i128, timezone: &str) -> Option<i32> {
    let id = COMPILED_TZ_PROVIDER.get(timezone.as_bytes()).ok()?;
    let seconds = COMPILED_TZ_PROVIDER
        .transition_nanoseconds_for_utc_epoch_nanoseconds(id, epoch_nanoseconds)
        .ok()?
        .0;
    i32::try_from(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iana_timezone_offsets_follow_daylight_saving_transitions() {
        let winter = 1_704_067_200_000.0; // 2024-01-01T00:00:00Z
        let summer = 1_719_792_000_000.0; // 2024-07-01T00:00:00Z

        assert_eq!(time_zone_offset_seconds(winter, "Europe/Paris"), Some(3600));
        assert_eq!(time_zone_offset_seconds(summer, "Europe/Paris"), Some(7200));
        assert_eq!(
            time_zone_offset_seconds(winter, "America/New_York"),
            Some(-18_000)
        );
        assert_eq!(time_zone_offset_seconds(winter, "Not/AZone"), None);
        assert_eq!(
            local_date_fields(winter, Some("Europe/Paris")),
            Some(LocalDateFields {
                year: 2024,
                month_zero_based: 0,
                day: 1,
                weekday_sunday_zero: 1,
                hour: 1,
                minute: 0,
                second: 0,
                millisecond: 0,
            })
        );
        assert_eq!(
            format_date_local_string(winter, Some("Europe/Paris")),
            "Mon Jan 01 2024 01:00:00 GMT+0100"
        );
    }

    #[test]
    fn time_zone_identifier_validation_uses_the_same_compiled_database() {
        assert!(is_valid_time_zone_identifier("UTC"));
        assert!(is_valid_time_zone_identifier("Europe/Paris"));
        assert!(!is_valid_time_zone_identifier(""));
        assert!(!is_valid_time_zone_identifier("Mars/Olympus"));
    }

    #[test]
    fn local_wall_clock_round_trips_through_the_selected_timezone() {
        let instant = 1_704_067_200_123.0; // 2024-01-01T00:00:00.123Z
        let wall_clock = local_wall_clock_as_utc_millis(instant, "Europe/Paris")
            .expect("Paris wall clock should project");
        assert_eq!(wall_clock, 1_704_070_800_123.0);
        assert_eq!(
            epoch_millis_for_local_wall_clock(wall_clock, "Europe/Paris"),
            Some(instant)
        );
    }

    #[test]
    fn epoch_millisecond_conversion_does_not_round_through_large_f64_nanoseconds() {
        for timestamp_ms in [-1.0, 0.0, 1_704_130_496_789.0] {
            let datetime = offset_datetime_from_unix_millis(timestamp_ms)
                .expect("ECMAScript timestamp should be representable");
            assert_eq!(unix_millis_from_offset_datetime(datetime), timestamp_ms);
        }
    }

    #[test]
    fn local_wall_clock_uses_legacy_date_dst_disambiguation() {
        // 2024-03-10T02:30 does not exist in New York. Compatible
        // disambiguation advances it to 03:30 EDT (07:30Z).
        assert_eq!(
            epoch_millis_for_local_wall_clock(1_710_037_800_000.0, "America/New_York"),
            Some(1_710_055_800_000.0)
        );
        // 2024-11-03T01:30 occurs twice. Legacy Date selects the earlier EDT
        // occurrence (05:30Z).
        assert_eq!(
            epoch_millis_for_local_wall_clock(1_730_597_400_000.0, "America/New_York"),
            Some(1_730_611_800_000.0)
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
