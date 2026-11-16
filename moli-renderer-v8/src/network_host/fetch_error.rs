use std::fmt;

/// Retain the transport's machine-readable error independently of JS-facing
/// diagnostics. Policy failures have no transport error and remain `Failed`.
#[derive(Debug, Clone)]
pub(crate) struct FetchError {
    pub(crate) message: String,
    pub(crate) network_error_text: Option<&'static str>,
}

impl FetchError {
    pub(crate) fn from_network(error: anyhow::Error) -> Self {
        Self {
            network_error_text: error
                .downcast_ref::<moli_fetch::NetworkFetchFailureContext>()
                .map(moli_fetch::NetworkFetchFailureContext::network_error_text)
                // A generic classification carries no additional transport
                // information. Keep policy diagnostics (e.g. CORP) intact.
                .filter(|reason| *reason != "net::ERR_FAILED"),
            message: format!("{error:#}"),
        }
    }
}

impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            message,
            network_error_text: None,
        }
    }
}

impl From<&str> for FetchError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl From<FetchError> for String {
    fn from(error: FetchError) -> Self {
        error.message
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FetchError {}

pub(crate) fn response_error_reason(network_error_text: Option<&str>) -> &'static str {
    match network_error_text {
        Some("net::ERR_ABORTED") => "Aborted",
        Some("net::ERR_NAME_NOT_RESOLVED") => "NameNotResolved",
        Some("net::ERR_CONNECTION_REFUSED") => "ConnectionRefused",
        Some("net::ERR_CONNECTION_RESET") => "ConnectionReset",
        Some("net::ERR_TIMED_OUT") => "TimedOut",
        Some("net::ERR_CONNECTION_CLOSED") => "ConnectionClosed",
        Some("net::ERR_CONNECTION_ABORTED") => "ConnectionAborted",
        Some("net::ERR_CONNECTION_FAILED") => "ConnectionFailed",
        Some("net::ERR_ADDRESS_UNREACHABLE") => "AddressUnreachable",
        Some("net::ERR_INTERNET_DISCONNECTED") => "InternetDisconnected",
        _ => "Failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_words_cannot_fabricate_a_transport_error_reason() {
        let error = FetchError::from_network(anyhow::anyhow!("net::ERR_CONNECTION_REFUSED"));
        assert_eq!(response_error_reason(error.network_error_text), "Failed");
        assert_eq!(
            response_error_reason(Some("net::ERR_CONNECTION_REFUSED")),
            "ConnectionRefused"
        );
        assert_eq!(response_error_reason(Some("net::ERR_FAILED")), "Failed");
    }
}
