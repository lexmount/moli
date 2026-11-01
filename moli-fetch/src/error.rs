use anyhow::{Result, bail};
use http::StatusCode;

pub const NET_ERR_ABORTED_ERROR_TEXT: &str = "net::ERR_ABORTED";

/// Explicit request cancellation, independent of diagnostic context or wording.
#[derive(Debug, Clone, Copy)]
pub struct FetchCancelled;

impl std::fmt::Display for FetchCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(NET_ERR_ABORTED_ERROR_TEXT)
    }
}

impl std::error::Error for FetchCancelled {}

/// A cache-only Fetch request cannot be satisfied without network access.
#[derive(Debug, Clone, Copy)]
pub struct FetchCacheMiss;

impl std::fmt::Display for FetchCacheMiss {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("net::ERR_CACHE_MISS")
    }
}

impl std::error::Error for FetchCacheMiss {}

/// Recognizes cancellation through typed causes, including shared body errors.
pub fn is_fetch_cancelled(error: &anyhow::Error) -> bool {
    error.is::<FetchCancelled>()
        || error.chain().any(|cause| {
            cause.is::<FetchCancelled>()
                || cause
                    .downcast_ref::<curl::Error>()
                    .is_some_and(curl::Error::is_aborted_by_callback)
        })
}

pub(crate) fn browser_network_error_text(error: &anyhow::Error) -> &'static str {
    if is_fetch_cancelled(error) {
        return NET_ERR_ABORTED_ERROR_TEXT;
    }

    if error.is::<FetchCacheMiss>() {
        return "net::ERR_CACHE_MISS";
    }

    let Some(error) = error.downcast_ref::<curl::Error>() else {
        return "net::ERR_FAILED";
    };
    if error.is_couldnt_resolve_host() {
        "net::ERR_NAME_NOT_RESOLVED"
    } else if error.is_couldnt_resolve_proxy() {
        "net::ERR_PROXY_CONNECTION_FAILED"
    } else if error.is_couldnt_connect() {
        "net::ERR_CONNECTION_REFUSED"
    } else if error.is_operation_timedout() {
        "net::ERR_TIMED_OUT"
    } else if error.is_recv_error() {
        "net::ERR_CONNECTION_RESET"
    } else if error.is_got_nothing() {
        "net::ERR_EMPTY_RESPONSE"
    } else if error.is_too_many_redirects() {
        "net::ERR_TOO_MANY_REDIRECTS"
    } else if error.is_peer_failed_verification()
        || error.is_ssl_cacert()
        || error.is_ssl_certproblem()
    {
        "net::ERR_CERT_AUTHORITY_INVALID"
    } else {
        "net::ERR_FAILED"
    }
}

pub fn ensure_http_status_success(
    request_url: &str,
    status: u16,
    allow_http_auth_challenge_status: bool,
) -> Result<()> {
    if (200..=299).contains(&status) {
        return Ok(());
    }

    if allow_http_auth_challenge_status && matches!(status, 401 | 407) {
        return Ok(());
    }

    let reason = StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("Unknown");
    bail!("HTTP request `{request_url}` returned {} {reason}", status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_survives_context_without_matching_diagnostic_text() {
        for error in [
            anyhow::Error::new(FetchCancelled),
            anyhow::Error::new(FetchCancelled)
                .context("transport stopped")
                .context("failed to prepare page"),
            anyhow::anyhow!("inner detail").context(FetchCancelled),
            anyhow::Error::new(curl::Error::new(curl_sys::CURLE_ABORTED_BY_CALLBACK))
                .context("arbitrary diagnostic"),
        ] {
            assert!(is_fetch_cancelled(&error), "{error:#}");
            assert_eq!(
                browser_network_error_text(&error),
                NET_ERR_ABORTED_ERROR_TEXT
            );
        }
    }

    #[test]
    fn cancellation_words_cannot_change_a_non_cancelled_failure() {
        for message in ["net::ERR_ABORTED", "request cancelled", "Callback aborted"] {
            let error = anyhow::anyhow!(message).context("failed to fetch");
            assert!(!is_fetch_cancelled(&error));
            assert_eq!(browser_network_error_text(&error), "net::ERR_FAILED");

            let error =
                anyhow::Error::new(curl::Error::new(curl_sys::CURLE_RECV_ERROR)).context(message);
            assert!(!is_fetch_cancelled(&error));
            assert_eq!(
                browser_network_error_text(&error),
                "net::ERR_CONNECTION_RESET"
            );
        }
    }

    #[test]
    fn curl_receive_failure_maps_to_browser_connection_reset() {
        let error = anyhow::Error::new(curl::Error::new(curl_sys::CURLE_RECV_ERROR))
            .context("curl request failed");

        assert_eq!(
            browser_network_error_text(&error),
            "net::ERR_CONNECTION_RESET"
        );
    }
}
