use moli_cookie_jar::{
    CookiePriority, StoredCookie, StoredCookieEffectiveSameSite, StoredCookiePartitionKey,
    StoredCookieSameSite, StoredCookieSetRejectionReason, StoredCookieSetReport,
    StoredCookieSetStatus, StoredCookieSourceScheme,
};
use url::Url;

use super::params::CdpCookieParam;

pub(super) enum NormalizedCdpCookieParam {
    Ready(Box<StoredCookie>, Option<Url>),
    Rejected(StoredCookieSetReport),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString)]
enum CdpCookieSameSite {
    None,
    Strict,
    Lax,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString)]
#[strum(ascii_case_insensitive)]
enum CdpCookieSourceScheme {
    Secure,
    NonSecure,
}

pub(crate) fn stored_cookie_same_site_from_cdp(value: Option<&str>) -> StoredCookieSameSite {
    match value.and_then(|value| value.parse::<CdpCookieSameSite>().ok()) {
        Some(CdpCookieSameSite::None) => StoredCookieSameSite::None,
        Some(CdpCookieSameSite::Strict) => StoredCookieSameSite::Strict,
        Some(CdpCookieSameSite::Lax) => StoredCookieSameSite::Lax,
        None => StoredCookieSameSite::Unspecified,
    }
}

pub(crate) fn stored_cookie_source_scheme_from_cdp(
    value: Option<&str>,
) -> StoredCookieSourceScheme {
    match value.and_then(|value| value.parse::<CdpCookieSourceScheme>().ok()) {
        Some(CdpCookieSourceScheme::Secure) => StoredCookieSourceScheme::Secure,
        Some(CdpCookieSourceScheme::NonSecure) => StoredCookieSourceScheme::NonSecure,
        None => StoredCookieSourceScheme::Unset,
    }
}

pub(super) fn normalize_cookie_param(
    param: CdpCookieParam,
    default_request_url: Option<&Url>,
) -> NormalizedCdpCookieParam {
    // Blink's Cookie Store API does not treat browser-facing structured input
    // validation as a protocol-layer parse failure. Keep that split here too:
    // malformed command JSON still returns `InvalidParams`, but per-cookie
    // semantic issues become structured cookie reports instead of aborting the
    // whole batch.
    let same_site = stored_cookie_same_site_from_cdp(param.same_site.as_deref());
    let now = time::OffsetDateTime::now_utc();
    let effective_same_site = Some(StoredCookieEffectiveSameSite::for_cookie(
        same_site,
        Some(now),
        now,
    ));
    let mut rejection_reasons = Vec::new();

    // Blink's Cookie Store API rejects several structured-name/value edge
    // cases up front instead of delegating them to backend parsing. Keep the
    // same ownership split for CDP-style structured writes.
    if param.name.is_empty() && param.value.is_empty() {
        rejection_reasons.push(StoredCookieSetRejectionReason::EmptyNameAndValue);
    }
    if param.name.is_empty() && param.value.contains('=') {
        rejection_reasons.push(StoredCookieSetRejectionReason::EmptyNameValueContainsEquals);
    }
    if param.name.contains('=') {
        rejection_reasons.push(StoredCookieSetRejectionReason::NameContainsEquals);
    }

    let partition_key = match normalize_partition_key(
        param.partition_key.as_ref(),
        param.partition_key_opaque.unwrap_or(false),
    ) {
        Ok(partition_key) => partition_key,
        Err(reason) => {
            rejection_reasons.push(reason);
            None
        }
    };

    let parsed_url = match param.url.as_deref() {
        Some(url) => match Url::parse(url) {
            Ok(url) => Some(url),
            Err(_) => {
                rejection_reasons.push(StoredCookieSetRejectionReason::InvalidUrl);
                None
            }
        },
        None => default_request_url.cloned(),
    };

    let request_host = parsed_url.as_ref().and_then(|url| {
        if matches!(url.scheme(), "http" | "https") {
            url.host_str().map(|host| host.to_ascii_lowercase())
        } else if param.url.is_some() {
            rejection_reasons.push(StoredCookieSetRejectionReason::NonHttpScheme);
            None
        } else {
            None
        }
    });
    if parsed_url.is_some()
        && request_host.is_none()
        && param.url.is_some()
        && !rejection_reasons.contains(&StoredCookieSetRejectionReason::NonHttpScheme)
    {
        rejection_reasons.push(StoredCookieSetRejectionReason::NonHttpScheme);
    }

    let (domain, host_only) = match (param.domain.as_deref(), request_host.as_deref()) {
        (Some(domain), maybe_host) => {
            let trimmed = domain.trim();
            // Chromium accepts both `example.com` and `.example.com` on the
            // structured write surface, then canonicalizes by dropping the
            // legacy leading dot before backend validation.
            let normalized_domain = trimmed.trim_start_matches('.').to_ascii_lowercase();
            if normalized_domain.is_empty() {
                rejection_reasons.push(StoredCookieSetRejectionReason::UnspecifiedDomain);
            } else if let Some(host) = maybe_host
                && host != normalized_domain
                && !host.ends_with(&format!(".{normalized_domain}"))
            {
                rejection_reasons.push(StoredCookieSetRejectionReason::DomainMismatch);
            }
            (normalized_domain, false)
        }
        (None, Some(host)) => (host.to_owned(), true),
        (None, None) => {
            rejection_reasons.push(if param.url.is_none() {
                StoredCookieSetRejectionReason::MissingCookieUrl
            } else {
                StoredCookieSetRejectionReason::UnspecifiedDomain
            });
            (String::new(), true)
        }
    };

    let path = match param.path {
        Some(path) if path.starts_with('/') => path,
        Some(_) => {
            rejection_reasons.push(StoredCookieSetRejectionReason::PathMustStartWithSlash);
            "/".to_owned()
        }
        None => "/".to_owned(),
    };
    let secure = param.secure.unwrap_or_else(|| {
        parsed_url
            .as_ref()
            .is_some_and(|url| url.scheme() == "https")
    });
    let priority = param.priority.as_deref().and_then(CookiePriority::parse);
    // Treat omitted sourceScheme as "unspecified" metadata instead of deriving
    // it from the URL and turning later request matching into an accidental
    // SchemeMismatch.
    let source_scheme = stored_cookie_source_scheme_from_cdp(param.source_scheme.as_deref());
    let source_port = param.source_port.unwrap_or_else(|| {
        parsed_url
            .as_ref()
            .and_then(|url| url.port_or_known_default())
            .map(i32::from)
            .unwrap_or(-1)
    });

    if !rejection_reasons.is_empty() {
        return NormalizedCdpCookieParam::Rejected(rejected_cookie_param_report(
            rejection_reasons,
            effective_same_site,
        ));
    }

    NormalizedCdpCookieParam::Ready(
        Box::new(StoredCookie {
            name: param.name,
            value: param.value,
            domain,
            host_only,
            path,
            secure,
            http_only: param.http_only,
            expires: param.expires.and_then(offset_datetime_from_cdp_timestamp),
            same_site,
            priority,
            partition_key,
            source_scheme,
            source_port,
            creation_index: 0,
            creation_time: Some(now),
            last_access_index: 0,
        }),
        parsed_url,
    )
}

pub(crate) fn normalize_partition_key(
    value: Option<&serde_json::Value>,
    partition_key_opaque: bool,
) -> Result<Option<StoredCookiePartitionKey>, StoredCookieSetRejectionReason> {
    if partition_key_opaque {
        return Err(StoredCookieSetRejectionReason::InvalidPartitionKey);
    }
    let Some(value) = value else {
        return Ok(None);
    };
    let top_level_site = value
        .get("topLevelSite")
        .and_then(serde_json::Value::as_str)
        .ok_or(StoredCookieSetRejectionReason::InvalidPartitionKey)?;
    let has_cross_site_ancestor = value
        .get("hasCrossSiteAncestor")
        .and_then(serde_json::Value::as_bool)
        .ok_or(StoredCookieSetRejectionReason::InvalidPartitionKey)?;
    let url = Url::parse(top_level_site)
        .map_err(|_| StoredCookieSetRejectionReason::InvalidPartitionKey)?;
    if !(url.scheme() == "file"
        || matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
    {
        return Err(StoredCookieSetRejectionReason::InvalidPartitionKey);
    }
    Ok(Some(StoredCookiePartitionKey::site(
        moli_storage_key::site_for_url(&url),
        has_cross_site_ancestor,
    )))
}

fn rejected_cookie_param_report(
    rejection_reasons: Vec<StoredCookieSetRejectionReason>,
    effective_same_site: Option<StoredCookieEffectiveSameSite>,
) -> StoredCookieSetReport {
    let primary_reason = rejection_reasons
        .first()
        .copied()
        .unwrap_or(StoredCookieSetRejectionReason::Parse);
    StoredCookieSetReport {
        status: StoredCookieSetStatus::Rejected(primary_reason),
        rejection_reasons,
        warning_reasons: Vec::new(),
        effective_same_site,
    }
}

fn offset_datetime_from_cdp_timestamp(value: f64) -> Option<time::OffsetDateTime> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }

    let seconds = value.trunc() as i64;
    // Chromium caps persistent cookies at 400 days. Clamp before converting
    // the timestamp so large, valid protocol values cannot silently become
    // session cookies when they exceed OffsetDateTime's supported range.
    let latest_expiry = time::OffsetDateTime::now_utc() + time::Duration::days(400);
    if seconds > latest_expiry.unix_timestamp() {
        return Some(latest_expiry);
    }
    let nanos = ((value.fract()) * 1_000_000_000.0).round() as i64;
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|dt| dt.checked_add(time::Duration::nanoseconds(nanos)))
        .map(|dt| dt.min(latest_expiry))
}

#[cfg(test)]
mod tests {
    use moli_cookie_jar::{StoredCookieSameSite, StoredCookieSourceScheme};

    use super::{
        CdpCookieSameSite, CdpCookieSourceScheme, stored_cookie_same_site_from_cdp,
        stored_cookie_source_scheme_from_cdp,
    };

    #[test]
    fn cdp_cookie_expiry_preserves_persistence_and_caps_the_lifetime() {
        use super::offset_datetime_from_cdp_timestamp;
        use time::{Duration, OffsetDateTime};

        let start = OffsetDateTime::now_utc();
        let near_expiry = start.unix_timestamp() + Duration::days(1).whole_seconds();
        assert_eq!(
            offset_datetime_from_cdp_timestamp(near_expiry as f64)
                .expect("near expiry")
                .unix_timestamp(),
            near_expiry
        );
        for timestamp in [9_007_199_254_740_991.0, f64::MAX] {
            let expiry = offset_datetime_from_cdp_timestamp(timestamp).expect("persistent expiry");
            assert!(expiry >= start + Duration::days(400));
            assert!(expiry <= OffsetDateTime::now_utc() + Duration::days(400));
        }
        assert_eq!(
            offset_datetime_from_cdp_timestamp(0.0)
                .expect("expired persistent cookie")
                .unix_timestamp(),
            0
        );
        for timestamp in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(offset_datetime_from_cdp_timestamp(timestamp), None);
        }
    }

    #[test]
    fn cdp_cookie_same_site_tokens_are_derived_and_case_sensitive() {
        assert_eq!(
            "None".parse::<CdpCookieSameSite>(),
            Ok(CdpCookieSameSite::None)
        );
        assert_eq!(
            "Strict".parse::<CdpCookieSameSite>(),
            Ok(CdpCookieSameSite::Strict)
        );
        assert_eq!(
            "Lax".parse::<CdpCookieSameSite>(),
            Ok(CdpCookieSameSite::Lax)
        );
        assert!("none".parse::<CdpCookieSameSite>().is_err());
        assert!("Unspecified".parse::<CdpCookieSameSite>().is_err());
    }

    #[test]
    fn cdp_cookie_same_site_maps_unknown_values_to_unspecified() {
        assert_eq!(
            stored_cookie_same_site_from_cdp(Some("None")),
            StoredCookieSameSite::None
        );
        assert_eq!(
            stored_cookie_same_site_from_cdp(Some("Strict")),
            StoredCookieSameSite::Strict
        );
        assert_eq!(
            stored_cookie_same_site_from_cdp(Some("Lax")),
            StoredCookieSameSite::Lax
        );
        assert_eq!(
            stored_cookie_same_site_from_cdp(Some("none")),
            StoredCookieSameSite::Unspecified
        );
        assert_eq!(
            stored_cookie_same_site_from_cdp(None),
            StoredCookieSameSite::Unspecified
        );
    }

    #[test]
    fn cdp_cookie_source_scheme_tokens_are_derived_and_case_insensitive() {
        assert_eq!(
            "Secure".parse::<CdpCookieSourceScheme>(),
            Ok(CdpCookieSourceScheme::Secure)
        );
        assert_eq!(
            "nonsecure".parse::<CdpCookieSourceScheme>(),
            Ok(CdpCookieSourceScheme::NonSecure)
        );
        assert!("Unset".parse::<CdpCookieSourceScheme>().is_err());
    }

    #[test]
    fn cdp_cookie_source_scheme_maps_unknown_values_to_unset() {
        assert_eq!(
            stored_cookie_source_scheme_from_cdp(Some("Secure")),
            StoredCookieSourceScheme::Secure
        );
        assert_eq!(
            stored_cookie_source_scheme_from_cdp(Some("NonSecure")),
            StoredCookieSourceScheme::NonSecure
        );
        assert_eq!(
            stored_cookie_source_scheme_from_cdp(Some("unknown")),
            StoredCookieSourceScheme::Unset
        );
        assert_eq!(
            stored_cookie_source_scheme_from_cdp(None),
            StoredCookieSourceScheme::Unset
        );
    }
}
