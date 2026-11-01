//! Shared Unix-microsecond encoding for Cookie creation times.
//!
//! Creation order is tracked separately by the canonical Cookie's index.
//! Unknown age has no timestamp; zero is the Unix epoch, not a sentinel.

use time::OffsetDateTime;

/// Encode a creation time, rounding down to the containing microsecond.
pub fn to_unix_micros(created: OffsetDateTime) -> Option<i64> {
    created
        .unix_timestamp_nanos()
        .div_euclid(1_000)
        .try_into()
        .ok()
}

/// Decode a stored creation time without assigning an age to missing data.
pub fn from_unix_micros(micros: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).ok()
}

pub(crate) fn normalize(created: OffsetDateTime) -> Option<OffsetDateTime> {
    from_unix_micros(to_unix_micros(created)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_time_encoding_preserves_microseconds_on_both_sides_of_the_epoch() {
        for (nanos, micros) in [
            (0, 0),
            (999, 0),
            (1_000, 1),
            (1_999, 1),
            (-1, -1),
            (-999, -1),
            (-1_000, -1),
            (-1_001, -2),
            (1_700_000_000_123_456_789, 1_700_000_000_123_456),
        ] {
            let original = OffsetDateTime::from_unix_timestamp_nanos(nanos).unwrap();
            assert_eq!(to_unix_micros(original), Some(micros));
            let restored = from_unix_micros(micros).unwrap();
            assert_eq!(restored.unix_timestamp_nanos(), i128::from(micros) * 1_000);
        }
    }

    #[test]
    fn out_of_range_creation_times_are_rejected_without_overflow() {
        // The workspace also enables time's large-dates feature. Whether its
        // calendar accepts an i64 endpoint or not, decoding must never wrap.
        for micros in [i64::MIN, i64::MAX] {
            if let Some(created) = from_unix_micros(micros) {
                assert_eq!(to_unix_micros(created), Some(micros));
            }
        }
        for seconds in [i64::MIN / 1_000_000 - 1, i64::MAX / 1_000_000 + 1] {
            if let Ok(created) = OffsetDateTime::from_unix_timestamp(seconds) {
                assert!(to_unix_micros(created).is_none());
                assert!(normalize(created).is_none());
            }
        }
    }
}
