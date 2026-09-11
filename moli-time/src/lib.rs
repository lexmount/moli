//! Shared browser clocks, timer scheduling, and timestamp formatting.
//! Date parsing, Intl and timezone rules belong to native V8/ICU.
use std::{
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod timers;
pub use timers::{ReadyTimer, TimerId, TimerReadyAllowance, TimerScheduler};

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
    // High Resolution Time requires the default, non-isolated clock to expose
    // no finer than 100 microseconds. Keep the shared clock on that safe
    // default; cross-origin-isolated realms may opt into a finer clock later.
    const TEN_TICKS_PER_MILLISECOND: f64 = 10.0;
    (millis * TEN_TICKS_PER_MILLISECOND).floor() / TEN_TICKS_PER_MILLISECOND
}

pub fn dom_time_since_origin_millis(time_origin: f64) -> f64 {
    coarsened_dom_time_millis((unix_epoch_millis() - time_origin).max(0.0))
}

/// Formats HTML's fixed, locale-independent Document.lastModified surface.
/// The caller supplies the timestamp-sensitive offset from the native
/// environment. This crate does not carry a second timezone database.
pub fn format_document_last_modified_value(
    timestamp_ms: f64,
    offset_seconds: i32,
) -> Option<String> {
    let datetime = offset_datetime_from_unix_millis(timestamp_ms)?
        .to_offset(time::UtcOffset::from_whole_seconds(offset_seconds).ok()?);
    let month = u8::from(datetime.month());
    Some(format!(
        "{month:02}/{day:02}/{year:04} {hour:02}:{minute:02}:{second:02}",
        day = datetime.day(),
        year = datetime.year(),
        hour = datetime.hour(),
        minute = datetime.minute(),
        second = datetime.second(),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_last_modified_formats_native_offsets_and_rejects_invalid_timestamps() {
        assert_eq!(
            format_document_last_modified_value(5_025_000.0, 28_800).as_deref(),
            Some("01/01/1970 09:23:45")
        );
        assert_eq!(
            format_document_last_modified_value(1_704_067_384_005.0, 0).as_deref(),
            Some("01/01/2024 00:03:04")
        );
        assert_eq!(format_document_last_modified_value(f64::NAN, 0), None);
        assert_eq!(format_document_last_modified_value(f64::INFINITY, 0), None);
    }

    #[test]
    fn dom_time_coarsening_uses_default_hundred_microsecond_resolution() {
        assert_eq!(coarsened_dom_time_millis(1.234_567), 1.2);
        assert_eq!(coarsened_dom_time_millis(1.299_999), 1.2);
        assert_eq!(coarsened_dom_time_millis(1.3), 1.3);
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
