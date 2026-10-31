use crate::cancellation::{FetchResponseTiming, ResponseProgress};

/// Whether Main Fetch received an HTTP response or a respondWith() result.
/// HTTP cache hits retain Network provenance and the cached body's size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResponseSource {
    #[default]
    Network,
    ServiceWorker,
}

/// Read-only observations of the selected response's transport. Clones share
/// native measurements without retaining cancellation authority or the body.
/// Service Worker observations describe client-to-worker handover, not an
/// internal network fetch performed by the worker.
#[derive(Debug, Clone, Default)]
pub struct ResponseTimingInfo {
    source: ResponseSource,
    progress: Option<ResponseProgress>,
}

impl ResponseTimingInfo {
    pub(crate) fn from_progress(progress: ResponseProgress) -> Self {
        Self {
            source: ResponseSource::Network,
            progress: Some(progress),
        }
    }

    pub fn from_service_worker() -> Self {
        let progress = ResponseProgress::default();
        progress.mark_response_started();
        Self {
            source: ResponseSource::ServiceWorker,
            progress: Some(progress),
        }
    }

    pub fn source(&self) -> ResponseSource {
        self.source
    }

    pub fn transport(&self) -> FetchResponseTiming {
        self.progress
            .as_ref()
            .map(ResponseProgress::timing)
            .unwrap_or_default()
    }

    /// Actual encoded payload bytes, excluding HTTP headers and framing.
    /// Unknown measurements stay distinct from an observed zero-byte body.
    pub fn encoded_body_size(&self) -> Option<u64> {
        self.progress
            .as_ref()
            .and_then(ResponseProgress::encoded_body_size)
    }

    /// A memory cache hit is a new fetch. Keep representation sizes, but never
    /// reuse the preceding network request's timing boundaries.
    pub fn for_memory_cache(&self) -> Self {
        let progress = ResponseProgress::default();
        progress.set_encoded_body_size(self.encoded_body_size());
        progress.mark_cached_response_available().finish();
        Self::from_progress(progress)
    }
}

// Request state compares transfer identity, not measurements that can still be
// changing on the transport thread. This also makes equality reflexive while
// a streamed response is being received.
impl PartialEq for ResponseTimingInfo {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && match (&self.progress, &other.progress) {
                (None, None) => true,
                (Some(left), Some(right)) => left.same_transfer(right),
                _ => false,
            }
    }
}

impl Eq for ResponseTimingInfo {}
