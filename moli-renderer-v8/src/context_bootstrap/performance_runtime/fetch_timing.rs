use super::ResourcePerformanceEntry;
use crate::types::AsyncSubresourceFetchResponseFilter;
use moli_fetch::{FetchUrlList, RequestMode, ResponseCacheState, ResponseHead};
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
        filter: Option<AsyncSubresourceFetchResponseFilter>,
    ) -> ResourcePerformanceEntry {
        let filter =
            filter.or_else(|| crate::network_host::network_response_filter(origin, head, mode));
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
        if filter.is_some_and(|filter| filter.service_worker_timing.is_some()) {
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

    pub(crate) fn allows_timing(
        origin: &WebOrigin,
        head: &ResponseHead,
        filter: Option<AsyncSubresourceFetchResponseFilter>,
    ) -> bool {
        filter
            .and_then(|filter| filter.service_worker_timing)
            .unwrap_or_else(|| timing_allowed(origin, head))
    }

    pub(crate) fn failed(&self, url: &Url) -> ResourcePerformanceEntry {
        ResourcePerformanceEntry::from_network_failure(
            url.as_str(),
            "fetch",
            Some(self.start_unix_millis),
        )
    }
}

fn timing_allowed(origin: &WebOrigin, head: &ResponseHead) -> bool {
    // Once a redirect has failed TAO, a later response cannot restore access.
    // Likewise, returning home does not reset cross-origin response tainting.
    let mut cross_origin = false;
    for (index, redirect) in head.redirect_chain.iter().enumerate() {
        cross_origin |= !origin.same_origin_url(&redirect.from_url);
        let urls = FetchUrlList::new(&redirect.from_url, &head.redirect_chain[..index]);
        if cross_origin && !allows_timing(&redirect.headers, &urls.serialized_origin(origin)) {
            return false;
        }
    }
    cross_origin |= !origin.same_origin_url(&head.final_url);
    !cross_origin || allows_timing(&head.headers, &head.url_list().serialized_origin(origin))
}

fn allows_timing(headers: &[(String, Vec<u8>)], origin: &str) -> bool {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("timing-allow-origin"))
        .filter_map(|(_, value)| std::str::from_utf8(value).ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim_matches(|ch: char| ch.is_ascii_whitespace()))
        .any(|value| value == "*" || value == origin)
}
