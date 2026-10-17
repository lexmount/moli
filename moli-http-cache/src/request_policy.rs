use moli_header_field::{split_outside_quoted_strings, unquote_parameter_value};

use crate::policy::{cached_response_is_fresh_immutable_at, response_cache_policy_at};

/// Constraints on reuse of an HTTP representation. `Some(u64::MAX)` permits
/// unbounded staleness; `None` never permits stale reuse.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HttpCacheRequestPolicy {
    pub no_store: bool,
    pub no_cache: bool,
    pub only_if_cached: bool,
    pub max_age_seconds: Option<u64>,
    pub max_stale_seconds: Option<u64>,
    pub min_fresh_seconds: Option<u64>,
}

impl HttpCacheRequestPolicy {
    pub fn from_headers(headers: &[(String, Vec<u8>)]) -> Self {
        let mut policy = Self::default();
        let mut has_cache_control = false;
        let mut pragma_no_cache = false;
        for (name, bytes) in headers {
            let value = moli_header_field::decode_header_value(bytes);
            if name.eq_ignore_ascii_case("pragma") {
                pragma_no_cache |= crate::request_pragma_requires_validation(&value);
            }
            if !name.eq_ignore_ascii_case("cache-control") {
                continue;
            }
            has_cache_control = true;
            for directive in split_outside_quoted_strings(&value, ',') {
                let (name, value) = directive
                    .trim()
                    .split_once('=')
                    .map(|(name, value)| (name.trim(), Some(unquote_parameter_value(value.trim()))))
                    .unwrap_or((directive.trim(), None));
                match name.to_ascii_lowercase().as_str() {
                    "no-store" => policy.no_store = true,
                    "no-cache" => policy.no_cache = true,
                    "only-if-cached" => policy.only_if_cached = true,
                    "max-age" | "min-fresh" | "max-stale" => {
                        let seconds = match value.as_deref() {
                            None if name.eq_ignore_ascii_case("max-stale") => Some(u64::MAX),
                            Some(value) => delta_seconds(value),
                            None => None,
                        };
                        let Some(seconds) = seconds else {
                            // A malformed known constraint must not widen reuse,
                            // even if a later field supplies a valid duplicate.
                            policy.no_cache = true;
                            continue;
                        };
                        let slot = if name.eq_ignore_ascii_case("max-age") {
                            &mut policy.max_age_seconds
                        } else if name.eq_ignore_ascii_case("max-stale") {
                            &mut policy.max_stale_seconds
                        } else {
                            &mut policy.min_fresh_seconds
                        };
                        *slot = Some(slot.map_or(seconds, |previous| {
                            if name.eq_ignore_ascii_case("min-fresh") {
                                previous.max(seconds)
                            } else {
                                previous.min(seconds)
                            }
                        }));
                    }
                    _ => {}
                }
            }
        }
        if !has_cache_control && pragma_no_cache {
            policy.no_cache = true;
        }
        policy
    }

    pub fn allows_reuse_at(
        &self,
        response_headers: &[(String, Vec<u8>)],
        stored_at_unix_ms: u64,
        expires_at_unix_ms: Option<u64>,
        force_validate: bool,
        now_unix_ms: u64,
    ) -> bool {
        if self.no_store || self.no_cache || force_validate {
            return false;
        }
        let response = response_cache_policy_at(response_headers, stored_at_unix_ms);
        if !response.store || response.requires_validation {
            return false;
        }
        // Recalculate explicit freshness from the stored header facts. This
        // also handles persisted entries written before stale age was retained.
        let expiry = response
            .expires_at_unix_ms
            .or(expires_at_unix_ms.filter(|value| *value != 0));
        let Some(expiry) = expiry else {
            return false;
        };
        let fresh = now_unix_ms < expiry;
        if let Some(max_age) = self.max_age_seconds {
            let immutable_reload = max_age == 0
                && fresh
                && cached_response_is_fresh_immutable_at(
                    now_unix_ms,
                    response_headers,
                    Some(expiry),
                );
            if !immutable_reload
                && (max_age == 0
                    || current_age_ms(response_headers, stored_at_unix_ms, now_unix_ms)
                        > max_age.saturating_mul(1000))
            {
                return false;
            }
        }
        if self.min_fresh_seconds.is_some_and(|seconds| {
            now_unix_ms > expiry
                || expiry.saturating_sub(now_unix_ms) < seconds.saturating_mul(1000)
        }) {
            return false;
        }
        fresh
            || (!response.must_revalidate
                && self.max_stale_seconds.is_some_and(|seconds| {
                    now_unix_ms.saturating_sub(expiry) <= seconds.saturating_mul(1000)
                }))
    }
}

fn delta_seconds(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().unwrap_or(u64::MAX))
}

fn current_age_ms(headers: &[(String, Vec<u8>)], stored_at: u64, now: u64) -> u64 {
    let apparent_age = crate::response_date(headers)
        .and_then(|date| date.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| {
            stored_at.saturating_sub(duration.as_millis().min(u128::from(u64::MAX)) as u64)
        })
        .unwrap_or_default();
    let age = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("age"))
        .filter_map(|(_, value)| {
            delta_seconds(moli_header_field::decode_header_value(value).trim())
        })
        .max()
        .unwrap_or_default()
        .saturating_mul(1000);
    apparent_age
        .max(age)
        .saturating_add(now.saturating_sub(stored_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(fields: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
        fields
            .iter()
            .map(|(name, value)| ((*name).into(), value.as_bytes().to_vec()))
            .collect()
    }

    fn request(value: &str) -> HttpCacheRequestPolicy {
        HttpCacheRequestPolicy::from_headers(&headers(&[("Cache-Control", value)]))
    }

    #[test]
    fn request_directives_merge_fields_without_reading_quoted_extensions() {
        let policy = HttpCacheRequestPolicy::from_headers(&headers(&[
            (
                "Cache-Control",
                "extension=\"only-if-cached, no-store\", MAX-AGE=90, max-stale",
            ),
            (
                "cache-control",
                "max-age=30, max-stale=80, min-fresh=10, ONLY-IF-CACHED",
            ),
            ("Cache-Control", "max-age=60, max-stale=90, min-fresh=20"),
        ]));
        assert!(!policy.no_store && !policy.no_cache);
        assert!(policy.only_if_cached);
        assert_eq!(policy.max_age_seconds, Some(30));
        assert_eq!(policy.max_stale_seconds, Some(80));
        assert_eq!(policy.min_fresh_seconds, Some(20));
        assert!(!request("extension=\"only-if-cached, no-store\"").only_if_cached);
    }

    #[test]
    fn malformed_request_bounds_cannot_be_overridden_by_valid_duplicates() {
        for name in ["max-age", "min-fresh", "max-stale"] {
            for invalid in ["", "+1", "-1", "1.5", "one"] {
                let value = format!("{name}={invalid}, {name}=999, max-stale");
                assert!(request(&value).no_cache, "{value}");
            }
        }
        assert!(request("max-age").no_cache);
        assert!(request("min-fresh").no_cache);
        assert_eq!(request("max-stale").max_stale_seconds, Some(u64::MAX));
        assert_eq!(
            request("max-age=99999999999999999999999").max_age_seconds,
            Some(u64::MAX)
        );
    }

    #[test]
    fn pragma_is_a_fallback_only_when_cache_control_is_absent() {
        assert!(HttpCacheRequestPolicy::from_headers(&headers(&[("Pragma", "no-cache")])).no_cache);
        assert!(
            !HttpCacheRequestPolicy::from_headers(&headers(&[
                ("Pragma", "no-cache"),
                ("Cache-Control", "max-age=60")
            ]))
            .no_cache
        );
    }

    #[test]
    fn max_stale_counts_age_before_storage_and_resident_time() {
        let response = headers(&[("Cache-Control", "max-age=1500"), ("Age", "2000")]);
        let stored = 3_000_000;
        // An older persisted expiry clamped to the storage time must not
        // hide the response's existing five hundred seconds of staleness.
        assert!(!request("max-stale=499").allows_reuse_at(
            &response,
            stored,
            Some(stored),
            false,
            stored
        ));
        assert!(request("max-stale=500").allows_reuse_at(
            &response,
            stored,
            Some(stored),
            false,
            stored
        ));
        assert!(!request("max-stale=500").allows_reuse_at(
            &response,
            stored,
            Some(stored),
            false,
            stored + 1
        ));
        assert!(request("max-stale").allows_reuse_at(
            &response,
            stored,
            Some(stored),
            false,
            stored + 999_000
        ));
        assert!(!request("").allows_reuse_at(&response, stored, Some(stored), false, stored));
    }

    #[test]
    fn stale_reuse_never_overrides_response_or_caller_validation_requirements() {
        for control in [
            "max-age=0, no-cache",
            "max-age=0, must-revalidate",
            "max-age=0, no-store",
            "max-age=0, private",
        ] {
            let response = headers(&[("Cache-Control", control)]);
            assert!(
                !request("max-stale").allows_reuse_at(
                    &response,
                    100_000,
                    Some(100_000),
                    false,
                    101_000
                ),
                "{control}"
            );
        }
        let response = headers(&[("Cache-Control", "max-age=0")]);
        for control in [
            "no-cache, max-stale",
            "no-store, max-stale",
            "min-fresh=0, max-stale",
        ] {
            assert!(
                !request(control).allows_reuse_at(
                    &response,
                    100_000,
                    Some(100_000),
                    false,
                    101_000
                ),
                "{control}"
            );
        }
        assert!(!request("max-stale").allows_reuse_at(
            &response,
            100_000,
            Some(100_000),
            true,
            101_000
        ));
    }

    #[test]
    fn max_age_and_min_fresh_use_current_age_not_time_since_storage() {
        let response = headers(&[("Cache-Control", "max-age=600"), ("Age", "100")]);
        for (control, expected) in [
            ("max-age=130", true),
            ("max-age=129", false),
            ("min-fresh=470", true),
            ("min-fresh=471", false),
        ] {
            assert_eq!(
                request(control).allows_reuse_at(&response, 100_000, None, false, 130_000),
                expected,
                "{control}"
            );
        }
    }

    #[test]
    fn immutable_only_relaxes_soft_reload_while_fresh() {
        let response = headers(&[("Cache-Control", "max-age=600, immutable")]);
        assert!(request("max-age=0").allows_reuse_at(&response, 100_000, None, false, 130_000));
        assert!(!request("no-cache").allows_reuse_at(&response, 100_000, None, false, 130_000));
        assert!(!request("max-age=0").allows_reuse_at(&response, 100_000, None, true, 130_000));
        assert!(!request("max-age=0").allows_reuse_at(&response, 100_000, None, false, 701_000));
        assert!(request("max-stale=2").allows_reuse_at(&response, 100_000, None, false, 701_000));
    }

    #[test]
    fn max_stale_does_not_invent_freshness_for_validator_only_entries() {
        let response = headers(&[("ETag", "\"validator\"")]);
        for expiry in [None, Some(0)] {
            assert!(
                !request("max-stale").allows_reuse_at(&response, 100_000, expiry, false, 130_000)
            );
        }
    }

    #[test]
    fn expires_lifetime_accounts_for_clock_skew_and_age() {
        let response = headers(&[
            ("Date", "Thu, 01 Jan 1970 00:01:30 GMT"),
            ("Expires", "Thu, 01 Jan 1970 00:02:30 GMT"),
            ("Age", "20"),
        ]);
        assert!(request("").allows_reuse_at(&response, 100_000, None, false, 139_999));
        assert!(!request("").allows_reuse_at(&response, 100_000, None, false, 140_000));
        assert!(request("max-stale=1").allows_reuse_at(&response, 100_000, None, false, 141_000));
        assert!(!request("max-stale=1").allows_reuse_at(&response, 100_000, None, false, 141_001));
    }
}
