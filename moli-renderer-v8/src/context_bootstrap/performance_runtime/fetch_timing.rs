use std::time::Instant;

use moli_fetch::{FetchUrlList, RedirectSource, ResponseCacheState, ResponseHead, ResponseSource};
use moli_url::WebOrigin;
use url::Url;

use super::ResourcePerformanceEntry;

/// State owned by a fetch, from registration through its body terminal. Reading
/// or cloning a JavaScript Response does not create another timing record.
pub(crate) struct FetchResourceTiming {
    started: Instant,
    start_unix_millis: f64,
    response_start_unix_millis: Option<f64>,
}

impl FetchResourceTiming {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            start_unix_millis: moli_time::unix_epoch_millis(),
            response_start_unix_millis: None,
        }
    }

    fn at(&self, instant: Instant) -> f64 {
        self.start_unix_millis
            + instant
                .saturating_duration_since(self.started)
                .as_secs_f64()
                * 1000.0
    }

    fn now(&self) -> f64 {
        self.at(Instant::now())
    }

    pub(crate) fn response_started(&mut self) {
        self.response_start_unix_millis = Some(self.now());
    }

    // Fetch response handover reports opaque timing for network errors,
    // including CORS and preflight failures. Requests rejected before a fetch
    // starts never register this state and do not reach this publication path.
    pub(crate) fn failure(&self, name: &Url) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_network_failure(
            name.as_str(),
            "fetch",
            Some(self.start_unix_millis),
        );
        entry.end_unix_millis = Some(self.now());
        entry
    }

    pub(crate) fn response(
        &self,
        name: &Url,
        origin: &WebOrigin,
        head: &ResponseHead,
        readable: bool,
        body_size: usize,
    ) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_streaming_network_response(
            name.as_str(),
            "fetch",
            Some(self.start_unix_millis),
            head,
            body_size,
        );
        let transport = head.timing_info.transport();
        entry.end_unix_millis = Some(
            transport
                .response_end_at()
                .map_or_else(|| self.now(), |instant| self.at(instant)),
        );
        if !readable {
            entry.response_status = 0.0;
            entry.content_type.clear();
            entry.encoded_body_size = 0.0;
            entry.decoded_body_size = 0.0;
        }
        // CORS controls response body information; TAO separately controls
        // connection timing, transfer size, and the cache state. respondWith()
        // results retain their client-to-worker measurements without applying
        // a network TAO check to the worker's final response.
        let allowed = timing_allow_passes(origin, head);
        entry.transfer_size = if allowed {
            match head.cache_state {
                ResponseCacheState::Local => 0.0,
                ResponseCacheState::Validated => 300.0,
                ResponseCacheState::None => entry.encoded_body_size + 300.0,
            }
        } else {
            0.0
        };
        if allowed {
            entry.response_start_unix_millis = transport
                .first_interim_response_started_at()
                .or(transport.response_started_at())
                .map(|instant| self.at(instant))
                .or(self.response_start_unix_millis);
            entry.next_hop_protocol = head
                .negotiated_http_version
                .map(|version| version.protocol_name().to_owned())
                .unwrap_or_default();
        }
        entry
    }
}

fn timing_allow_passes(origin: &WebOrigin, head: &ResponseHead) -> bool {
    // Only network responses perform TAO. Redirects from workers still
    // contribute to URL-list taint, but their synthetic headers are not an
    // additional network permission check.
    for (index, redirect) in head.redirect_chain.iter().enumerate() {
        if redirect.source == RedirectSource::Network
            && !response_allows_timing(
                origin,
                FetchUrlList::new(&redirect.from_url, &head.redirect_chain[..index]),
                &redirect.headers,
            )
        {
            return false;
        }
    }
    head.timing_info.source() == ResponseSource::ServiceWorker
        || response_allows_timing(origin, head.url_list(), &head.headers)
}

fn response_allows_timing(
    origin: &WebOrigin,
    urls: FetchUrlList<'_>,
    headers: &[(String, Vec<u8>)],
) -> bool {
    if !urls.has_cross_origin_url(origin) {
        return true;
    }
    let serialized_origin = urls.serialized_origin(origin);
    let value = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("timing-allow-origin"))
        .map(|(_, value)| moli_fetch::decode_header_value(value))
        .collect::<Vec<_>>()
        .join(", ");
    moli_fetch::split_http_header_list(&value)
        .any(|value| value == "*" || value == serialized_origin)
}
