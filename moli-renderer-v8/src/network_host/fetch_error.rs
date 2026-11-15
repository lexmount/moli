/// Preserve transport classification when a browser request crosses an async
/// boundary whose diagnostic error message is owned by the renderer.
#[derive(Debug)]
pub(crate) struct SubresourceFetchError {
    pub(crate) message: String,
    pub(crate) network_error_text: Option<&'static str>,
}

impl From<anyhow::Error> for SubresourceFetchError {
    fn from(error: anyhow::Error) -> Self {
        let network_error_text = error
            .downcast_ref::<moli_fetch::NetworkFetchFailureContext>()
            .map(moli_fetch::NetworkFetchFailureContext::network_error_text)
            .or_else(|| moli_fetch::is_fetch_cancelled(&error).then_some("net::ERR_ABORTED"));
        Self {
            message: format!("{error:#}"),
            network_error_text,
        }
    }
}

impl From<String> for SubresourceFetchError {
    fn from(message: String) -> Self {
        Self {
            message,
            network_error_text: None,
        }
    }
}

impl From<SubresourceFetchError> for String {
    fn from(error: SubresourceFetchError) -> Self {
        error.message
    }
}

impl std::fmt::Display for SubresourceFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SubresourceFetchError {}

pub(crate) fn fetch_response_error_reason(error_text: Option<&str>) -> &'static str {
    match error_text {
        Some("net::ERR_ABORTED") => "Aborted",
        Some("net::ERR_TIMED_OUT") => "TimedOut",
        Some("net::ERR_ACCESS_DENIED") => "AccessDenied",
        Some("net::ERR_CONNECTION_CLOSED") => "ConnectionClosed",
        Some("net::ERR_CONNECTION_RESET") => "ConnectionReset",
        Some("net::ERR_CONNECTION_REFUSED") => "ConnectionRefused",
        Some("net::ERR_CONNECTION_ABORTED") => "ConnectionAborted",
        Some("net::ERR_CONNECTION_FAILED") => "ConnectionFailed",
        Some("net::ERR_NAME_NOT_RESOLVED") => "NameNotResolved",
        Some("net::ERR_INTERNET_DISCONNECTED") => "InternetDisconnected",
        Some("net::ERR_ADDRESS_UNREACHABLE") => "AddressUnreachable",
        Some("net::ERR_BLOCKED_BY_CLIENT") => "BlockedByClient",
        Some("net::ERR_BLOCKED_BY_RESPONSE") => "BlockedByResponse",
        _ => "Failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_text_cannot_impersonate_a_transport_classification() {
        for message in [
            "net::ERR_CONNECTION_REFUSED",
            "connection refused",
            "net::ERR_ABORTED",
        ] {
            let error = SubresourceFetchError::from(message.to_owned());
            assert_eq!(error.network_error_text, None);
            assert_eq!(
                fetch_response_error_reason(error.network_error_text),
                "Failed"
            );
            let error = SubresourceFetchError::from(anyhow::anyhow!(message));
            assert_eq!(error.network_error_text, None);
        }
    }

    #[test]
    fn cancellation_classification_survives_unrelated_context() {
        let error = SubresourceFetchError::from(
            anyhow::Error::new(moli_fetch::FetchCancelled).context("unrelated diagnostic"),
        );
        assert_eq!(
            fetch_response_error_reason(error.network_error_text),
            "Aborted"
        );
        assert!(error.message.contains("unrelated diagnostic"));
    }
}
