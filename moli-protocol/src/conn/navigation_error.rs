use std::{error::Error, fmt};

use url::Url;

/// A failed navigation request, captured where the network policy is enforced.
///
/// This is an error cause rather than display context, so callers can recover
/// its request identity through `anyhow::Error::downcast_ref` after adding
/// diagnostic context along the navigation pipeline.
#[derive(Debug)]
pub struct NavigationNetworkError {
    pub error_text: String,
    pub unreachable_url: Url,
    pub request_method: String,
    pub request_headers: Vec<(String, String)>,
}

impl fmt::Display for NavigationNetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error_text)
    }
}

impl Error for NavigationNetworkError {}
