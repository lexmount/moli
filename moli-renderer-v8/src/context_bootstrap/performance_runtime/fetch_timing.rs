use super::ResourcePerformanceEntry;
use crate::types::AsyncSubresourceFetchResponseFilter;
use moli_fetch::{RequestMode, ResponseCacheState, ResponseHead};
use moli_url::WebOrigin;
use url::Url;

/// Request-time metadata, independent of the callback or Realm that happens to
/// be active when the body finishes. This must not retain any V8 objects.
#[derive(Clone)]
pub(crate) struct FetchResourceTiming {
    start_unix_millis: f64,
    initiator_url: String,
}

impl FetchResourceTiming {
    pub(crate) fn capture(scope: &mut v8::PinScope<'_, '_>, origin: &WebOrigin) -> Self {
        Self {
            start_unix_millis: crate::context_bootstrap::unix_epoch_millis(),
            initiator_url: crate::resource_initiator::current_url(scope, origin),
        }
    }

    pub(crate) fn completed(
        &self,
        url: &Url,
        origin: &WebOrigin,
        mode: RequestMode,
        head: &ResponseHead,
        body_size: usize,
        filter: Option<&AsyncSubresourceFetchResponseFilter>,
    ) -> ResourcePerformanceEntry {
        let network_filter = crate::network_host::network_response_filter(origin, head, mode);
        let filter = filter.or(network_filter.as_ref());
        let readable = filter.is_none_or(|filter| filter.is_readable());
        let mut entry = ResourcePerformanceEntry::from_streaming_network_response(
            url.as_str(),
            "fetch",
            Some(self.start_unix_millis),
            head,
            body_size,
        );
        // CORS controls response metadata and script attribution. TAO controls
        // timing/body sizes separately, including for opaque responses.
        if readable && head.cache_state == ResponseCacheState::None {
            entry.initiator_url.clone_from(&self.initiator_url);
        }
        if !readable {
            entry.response_status = 0.0;
            entry.content_type.clear();
        }
        if filter.is_some_and(|filter| filter.service_worker_metadata().is_some()) {
            // Bytes delivered by a Service Worker are not a network transfer
            // to this global, even when respondWith forwards a fetched body.
            entry.transfer_size = 0.0;
        }
        if !Self::allows_timing(origin, head, filter) {
            entry.transfer_size = 0.0;
            entry.encoded_body_size = 0.0;
            entry.decoded_body_size = 0.0;
        }
        entry
    }

    fn allows_timing(
        origin: &WebOrigin,
        head: &ResponseHead,
        filter: Option<&AsyncSubresourceFetchResponseFilter>,
    ) -> bool {
        filter
            .and_then(|filter| filter.service_worker_metadata())
            .map_or_else(
                || head.timing_allow_passed(origin),
                |metadata| metadata.timing_allow_passed,
            )
    }

    pub(crate) fn failed(&self, url: &Url) -> ResourcePerformanceEntry {
        ResourcePerformanceEntry::from_network_failure(
            url.as_str(),
            "fetch",
            Some(self.start_unix_millis),
        )
    }
}
