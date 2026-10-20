use std::time::Instant;

use moli_fetch::{FetchResponseFilter, FetchUrlList, ResponseCacheState, ResponseHead};
use moli_url::WebOrigin;
use url::Url;

use super::{ResourceConnectionTiming, ResourcePerformanceEntry};

/// State owned by a fetch, from registration through its body terminal. Reading
/// or cloning a JavaScript Response does not create another timing record.
#[derive(Debug)]
pub(crate) struct FetchResourceTiming {
    initiator_type: &'static str,
    started: Instant,
    start_unix_millis: f64,
    response_timing: Option<Box<FetchResourceResponseTiming>>,
}

#[derive(Debug, Default)]
struct FetchResourceResponseTiming {
    native: Option<moli_fetch::FetchResponseTiming>,
    request_start: Option<f64>,
    body_sizes: Option<moli_fetch::FetchResponseBodySizes>,
    final_headers_start: Option<f64>,
    first_interim_start: Option<f64>,
    end: Option<f64>,
}

impl FetchResourceTiming {
    pub(crate) fn new() -> Self {
        Self::for_initiator_type("fetch")
    }

    pub(crate) fn for_initiator_type(initiator_type: &'static str) -> Self {
        Self::from_start(
            initiator_type,
            Instant::now(),
            moli_time::unix_epoch_millis(),
        )
    }

    pub(crate) fn from_start(
        initiator_type: &'static str,
        started: Instant,
        start_unix_millis: f64,
    ) -> Self {
        Self {
            initiator_type,
            started,
            start_unix_millis,
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
            && native.body_sizes().is_none()
            && native.fetch_started_at().is_none()
        {
            return;
        }
        let timing = self.response_timing.get_or_insert_with(Default::default);
        timing.native = Some(native);
        timing.request_start = request_start;
        timing.body_sizes = native.body_sizes();
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
        if readable {
            self.apply_native_body_sizes(&mut entry, head);
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
        self.apply_native_body_sizes(&mut entry, head);
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
        body_complete: bool,
    ) -> ResourcePerformanceEntry {
        let mut entry = self.response(name, origin, head, filter, body_size);
        if !body_complete {
            // Worker abort retires the incomplete response's public body info.
            // A completed IO response keeps its original encoded/decoded sizes.
            entry.encoded_body_size = 0.0;
            entry.decoded_body_size = 0.0;
            self.finish_response(&mut entry, origin, head);
        }
        entry.response_status = 0.0;
        entry.content_type.clear();
        entry
    }

    fn apply_native_body_sizes(&self, entry: &mut ResourcePerformanceEntry, head: &ResponseHead) {
        if let Some(sizes) = self
            .response_timing
            .as_ref()
            .and_then(|timing| timing.body_sizes)
        {
            entry.encoded_body_size = sizes.encoded as f64;
            entry.decoded_body_size = sizes.decoded as f64;
            entry.content_encoding = if sizes.encoded > 0 {
                moli_fetch::resource_timing_content_encoding(&head.headers).to_owned()
            } else {
                String::new()
            };
        }
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
            self.apply_connection_timing(
                entry,
                head.final_url.scheme() == "https",
                head.redirect_chain
                    .iter()
                    .any(|redirect| redirect.source != moli_fetch::RedirectSource::Internal),
                head.cache_state,
            );
            if let Some(timing) = self.response_timing.as_ref() {
                entry.request_start_unix_millis = timing.request_start.or_else(|| {
                    (head.cache_state == ResponseCacheState::Local).then_some(
                        entry
                            .fetch_start_unix_millis
                            .unwrap_or(self.start_unix_millis),
                    )
                });
                entry.final_response_headers_start_unix_millis = timing.final_headers_start;
                entry.first_interim_response_start_unix_millis = timing.first_interim_start;
                entry.response_start_unix_millis =
                    timing.first_interim_start.or(timing.final_headers_start);
            }
            entry.next_hop_protocol = head
                .negotiated_http_version
                .filter(|_| head.cache_state != ResponseCacheState::Local)
                .map(|version| version.protocol_name().to_owned())
                .unwrap_or_default();
        }
        allowed
    }

    pub(crate) fn timing_allowed(origin: &WebOrigin, head: &ResponseHead) -> bool {
        timing_allow_passes(origin, head)
    }

    fn apply_connection_timing(
        &self,
        entry: &mut ResourcePerformanceEntry,
        secure: bool,
        redirected: bool,
        cache_state: ResponseCacheState,
    ) {
        let native = self
            .response_timing
            .as_ref()
            .and_then(|timing| timing.native);
        let fetch_start = if redirected {
            native
                .and_then(|timing| timing.fetch_started_at())
                .unwrap_or(self.started)
                .max(self.started)
        } else {
            self.started
        };
        let start = self.unix_millis(fetch_start);
        entry.fetch_start_unix_millis = Some(start);
        if redirected
            && native
                .and_then(|timing| timing.fetch_started_at())
                .is_some()
        {
            entry.redirect_start_unix_millis = Some(self.start_unix_millis);
            entry.redirect_end_unix_millis = Some(start);
        }
        if cache_state == ResponseCacheState::Local {
            entry.connection_timing = Some(ResourceConnectionTiming::at(start, secure));
            return;
        }
        let Some(connection) = native
            .and_then(|timing| timing.connection_timing())
            .and_then(|timing| timing.connection)
        else {
            return;
        };
        // Reuse is decided by the selected connection's actual creation time,
        // not by whether this individual transfer opened a socket. Concurrent
        // HTTP/2 streams can legitimately observe setup after their fetch began.
        entry.connection_timing = Some(if connection.connect_started_at < fetch_start {
            ResourceConnectionTiming::at(start, secure)
        } else {
            let (domain_lookup_start, domain_lookup_end) = connection
                .domain_lookup
                .map(|dns| {
                    (
                        self.unix_millis(dns.started_at),
                        self.unix_millis(dns.completed_at),
                    )
                })
                .unwrap_or((start, start));
            ResourceConnectionTiming {
                domain_lookup_start,
                domain_lookup_end,
                connect_start: self.unix_millis(connection.connect_started_at),
                connect_end: self.unix_millis(connection.connect_completed_at),
                secure_connection_start: connection
                    .secure_started_at
                    .filter(|_| secure)
                    .map(|time| self.unix_millis(time)),
            }
        });
    }

    pub(crate) fn apply_child_document_timing(
        &self,
        entry: &mut ResourcePerformanceEntry,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
        expose_document_response: bool,
    ) {
        let timing_allowed = expose_document_response && network.resource_timing_allowed;
        // Cross-origin timing failures use the enclosing frame's actual load
        // boundary, rather than leaking when its response body finished.
        entry.end_unix_millis = Some(if timing_allowed {
            self.end()
        } else {
            self.now()
        });
        if !expose_document_response || !network.resource_timing_body_visible {
            entry.content_encoding.clear();
            entry.response_status = 0.0;
            entry.content_type.clear();
        }
        if !timing_allowed {
            entry.encoded_body_size = 0.0;
            entry.decoded_body_size = 0.0;
            entry.transfer_size = 0.0;
            entry.delivery_type.clear();
            return;
        }
        if let Some(sizes) = self
            .response_timing
            .as_ref()
            .and_then(|timing| timing.body_sizes)
        {
            entry.encoded_body_size = sizes.encoded as f64;
            entry.decoded_body_size = sizes.decoded as f64;
        }
        entry.transfer_size = match network.cache_state {
            ResponseCacheState::None => entry.encoded_body_size + 300.0,
            ResponseCacheState::Local => 0.0,
            ResponseCacheState::Validated => 300.0,
        };
        self.apply_connection_timing(
            entry,
            network.final_url.starts_with("https:"),
            network.resource_timing_redirected,
            network.cache_state,
        );
        if let Some(timing) = self.response_timing.as_ref() {
            entry.request_start_unix_millis = timing.request_start.or_else(|| {
                (network.cache_state == ResponseCacheState::Local).then_some(
                    entry
                        .fetch_start_unix_millis
                        .unwrap_or(self.start_unix_millis),
                )
            });
            entry.final_response_headers_start_unix_millis = timing.final_headers_start;
            entry.first_interim_response_start_unix_millis = timing.first_interim_start;
            entry.response_start_unix_millis =
                timing.first_interim_start.or(timing.final_headers_start);
        }
        entry.next_hop_protocol = network
            .resource_timing_protocol
            .map(|version| version.protocol_name().to_owned())
            .unwrap_or_default();
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
