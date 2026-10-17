use std::time::Instant;

use moli_fetch::{FetchResponseFilter, FetchUrlList, ResponseCacheState, ResponseHead};
use moli_url::WebOrigin;
use url::Url;

use super::ResourcePerformanceEntry;

/// State owned by a fetch, from registration through its body terminal. Reading
/// or cloning a JavaScript Response does not create another timing record.
pub(crate) struct FetchResourceTiming {
    initiator_type: &'static str,
    started: Instant,
    start_unix_millis: f64,
    response_timing: Option<Box<FetchResourceResponseTiming>>,
}

#[derive(Default)]
struct FetchResourceResponseTiming {
    request_start: Option<f64>,
    body_bytes_received: bool,
    final_headers_start: Option<f64>,
    first_interim_start: Option<f64>,
    end: Option<f64>,
}

impl FetchResourceTiming {
    pub(crate) fn new() -> Self {
        Self::for_initiator_type("fetch")
    }

    pub(crate) fn for_initiator_type(initiator_type: &'static str) -> Self {
        Self {
            initiator_type,
            started: Instant::now(),
            start_unix_millis: moli_time::unix_epoch_millis(),
            response_timing: None,
        }
    }

    fn now(&self) -> f64 {
        self.start_unix_millis + self.started.elapsed().as_secs_f64() * 1000.0
    }

    pub(crate) fn response_started(&mut self) {
        self.response_timing = Some(Box::new(FetchResourceResponseTiming {
            final_headers_start: Some(self.now()),
            ..Default::default()
        }));
    }

    /// Use the native header observation even when JS delayed its delivery.
    pub(crate) fn response_started_at(&mut self, start: Option<Instant>) {
        self.response_timing = Some(Box::new(FetchResourceResponseTiming {
            final_headers_start: start.map(|instant| self.unix_millis(instant)),
            ..Default::default()
        }));
    }

    pub(crate) fn observe_native_timing(&mut self, native: moli_fetch::FetchResponseTiming) {
        let request_start = native
            .request_started_at()
            .map(|value| self.unix_millis(value));
        let final_start = native
            .response_started_at()
            .map(|value| self.unix_millis(value));
        let first_interim = native
            .first_interim_response_started_at()
            .map(|value| self.unix_millis(value));
        let end = native
            .response_end_at()
            .map(|value| self.unix_millis(value));
        if request_start.is_none()
            && final_start.is_none()
            && first_interim.is_none()
            && end.is_none()
        {
            return;
        }
        let timing = self.response_timing.get_or_insert_with(Default::default);
        timing.request_start = request_start;
        timing.body_bytes_received = native.body_bytes_received();
        if final_start.is_some() {
            timing.final_headers_start = final_start;
            timing.first_interim_start = first_interim;
        }
        if end.is_some() {
            timing.end = end;
        }
    }

    fn end(&self) -> f64 {
        self.response_timing
            .as_ref()
            .and_then(|timing| timing.end)
            .unwrap_or_else(|| self.now())
    }

    pub(crate) fn completed_at(
        &self,
        mut entry: ResourcePerformanceEntry,
        end: Instant,
    ) -> ResourcePerformanceEntry {
        entry.end_unix_millis = Some(self.unix_millis(end));
        entry
    }

    fn unix_millis(&self, instant: Instant) -> f64 {
        self.start_unix_millis
            + instant
                .saturating_duration_since(self.started)
                .as_secs_f64()
                * 1000.0
    }

    pub(crate) fn failure(&self, name: &Url) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_network_failure(
            name.as_str(),
            self.initiator_type,
            Some(self.start_unix_millis),
        );
        entry.end_unix_millis = Some(self.end());
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
            self.initiator_type,
            Some(self.start_unix_millis),
            head,
            if readable { body_size } else { 0 },
        );
        if readable
            && self
                .response_timing
                .as_ref()
                .is_some_and(|timing| timing.body_bytes_received)
        {
            entry.content_encoding =
                moli_fetch::resource_timing_content_encoding(&head.headers).to_owned();
        }
        if !readable {
            entry.response_status = 0.0;
            entry.content_type.clear();
            entry.content_encoding.clear();
        }
        self.finish_response(&mut entry, origin, head);
        entry
    }

    pub(crate) fn classic_script_response(
        &self,
        name: &Url,
        origin: &WebOrigin,
        head: &ResponseHead,
        body_size: usize,
    ) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_streaming_network_response(
            name.as_str(),
            self.initiator_type,
            Some(self.start_unix_millis),
            head,
            body_size,
        );
        // Imported scripts execute opaque no-cors responses. TAO exposes their
        // transfer/body sizes, but never changes the filtered response status.
        if !self.finish_response(&mut entry, origin, head) {
            entry.encoded_body_size = 0.0;
            entry.decoded_body_size = 0.0;
        }
        if head.url_list().has_cross_origin_url(origin) {
            entry.response_status = 0.0;
            entry.content_type.clear();
            entry.content_encoding.clear();
        }
        entry
    }

    pub(crate) fn aborted_response(
        &self,
        name: &Url,
        origin: &WebOrigin,
        head: &ResponseHead,
        filter: &FetchResponseFilter,
        body_size: usize,
    ) -> ResourcePerformanceEntry {
        let mut entry = self.response(name, origin, head, filter, body_size);
        entry.response_status = 0.0;
        entry.content_type.clear();
        entry
    }

    fn finish_response(
        &self,
        entry: &mut ResourcePerformanceEntry,
        origin: &WebOrigin,
        head: &ResponseHead,
    ) -> bool {
        entry.end_unix_millis = Some(self.end());
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
        if !allowed {
            entry.delivery_type.clear();
        }
        if allowed {
            if let Some(timing) = self.response_timing.as_ref() {
                entry.request_start_unix_millis = timing.request_start;
                entry.final_response_headers_start_unix_millis = timing.final_headers_start;
                entry.first_interim_response_start_unix_millis = timing.first_interim_start;
                entry.response_start_unix_millis =
                    timing.first_interim_start.or(timing.final_headers_start);
            }
            entry.next_hop_protocol = head
                .negotiated_http_version
                .map(|version| version.protocol_name().to_owned())
                .unwrap_or_default();
        }
        allowed
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
