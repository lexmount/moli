use std::time::Instant;

use moli_fetch::{FetchResponseFilter, FetchUrlList, ResponseCacheState, ResponseHead};
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

    fn now(&self) -> f64 {
        self.start_unix_millis + self.started.elapsed().as_secs_f64() * 1000.0
    }

    pub(crate) fn response_started(&mut self) {
        self.response_start_unix_millis = Some(self.now());
    }

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
        filter: &FetchResponseFilter,
        body_size: usize,
    ) -> ResourcePerformanceEntry {
        let readable = filter.is_readable();
        let mut entry = ResourcePerformanceEntry::from_streaming_network_response(
            name.as_str(),
            "fetch",
            Some(self.start_unix_millis),
            head,
            if readable { body_size } else { 0 },
        );
        entry.end_unix_millis = Some(self.now());
        if !readable {
            entry.response_status = 0.0;
            entry.content_type.clear();
        }
        // CORS controls response body information; TAO separately controls
        // connection timing, transfer size, and the cache state.
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
            entry.response_start_unix_millis = self.response_start_unix_millis;
            entry.next_hop_protocol = head
                .negotiated_http_version
                .map(|version| version.protocol_name().to_owned())
                .unwrap_or_default();
        }
        entry
    }
}

fn timing_allow_passes(origin: &WebOrigin, head: &ResponseHead) -> bool {
    // Evaluate each response with its own URL-list prefix. Crossing origins
    // can taint both the response and the serialized request origin, even when
    // the final response returns to the initiating origin.
    for (index, redirect) in head.redirect_chain.iter().enumerate() {
        if !response_allows_timing(
            origin,
            FetchUrlList::new(&redirect.from_url, &head.redirect_chain[..index]),
            &redirect.headers,
        ) {
            return false;
        }
    }
    response_allows_timing(origin, head.url_list(), &head.headers)
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
