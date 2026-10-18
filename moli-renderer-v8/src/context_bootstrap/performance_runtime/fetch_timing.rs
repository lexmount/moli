use super::ResourcePerformanceEntry;
use crate::types::AsyncSubresourceFetchResponseFilter;
use moli_fetch::{FetchResponseMetadata, ResponseCacheState, ResponseHead};
use moli_url::WebOrigin;
use url::Url;

/// Request-time metadata, independent of the callback or Realm that happens to
/// be active when the body finishes. This must not retain any V8 objects.
#[derive(Clone)]
pub(crate) struct FetchResourceTiming {
    initiator_type: &'static str,
    started: std::time::Instant,
    start_unix_millis: f64,
    response_timing: std::cell::RefCell<Option<Box<FetchResourceResponseTiming>>>,
    initiator_url: String,
}

#[derive(Clone, Default)]
struct FetchResourceResponseTiming {
    request_start: Option<f64>,
    body_bytes_received: bool,
    final_headers_start: Option<f64>,
    first_interim_start: Option<f64>,
    end: Option<f64>,
}

impl FetchResourceTiming {
    pub(crate) fn capture(scope: &mut v8::PinScope<'_, '_>, origin: &WebOrigin) -> Self {
        Self::capture_for_initiator_type(scope, origin, "fetch")
    }

    pub(crate) fn capture_for_initiator_type(
        scope: &mut v8::PinScope<'_, '_>,
        origin: &WebOrigin,
        initiator_type: &'static str,
    ) -> Self {
        Self::from_captured_initiator(
            initiator_type,
            crate::resource_initiator::current_url(scope, origin),
        )
    }

    pub(crate) fn from_captured_initiator(
        initiator_type: &'static str,
        initiator_url: String,
    ) -> Self {
        Self {
            initiator_type,
            started: std::time::Instant::now(),
            start_unix_millis: crate::context_bootstrap::unix_epoch_millis(),
            response_timing: Default::default(),
            initiator_url,
        }
    }

    fn now(&self) -> f64 {
        self.start_unix_millis + self.started.elapsed().as_secs_f64() * 1000.0
    }

    pub(crate) fn response_started(&self) {
        *self.response_timing.borrow_mut() = Some(Box::new(FetchResourceResponseTiming {
            final_headers_start: Some(self.now()),
            ..Default::default()
        }));
    }

    /// Use the transport's header boundary even when Worker JS delayed delivery.
    pub(crate) fn response_started_at(&self, start: Option<std::time::Instant>) {
        *self.response_timing.borrow_mut() = Some(Box::new(FetchResourceResponseTiming {
            final_headers_start: start.map(|instant| self.unix_millis(instant)),
            ..Default::default()
        }));
    }

    pub(crate) fn completed_at(
        &self,
        mut entry: ResourcePerformanceEntry,
        end: std::time::Instant,
    ) -> ResourcePerformanceEntry {
        entry.end_unix_millis = Some(self.unix_millis(end));
        entry
    }

    fn unix_millis(&self, instant: std::time::Instant) -> f64 {
        self.start_unix_millis
            + instant
                .saturating_duration_since(self.started)
                .as_secs_f64()
                * 1000.0
    }

    pub(crate) fn observe_native_timing(&self, native: moli_fetch::FetchResponseTiming) {
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
        let mut response_timing = self.response_timing.borrow_mut();
        let timing = response_timing.get_or_insert_with(Default::default);
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
            .borrow()
            .as_ref()
            .and_then(|timing| timing.end)
            .unwrap_or_else(|| self.now())
    }

    pub(crate) fn completed(
        &self,
        url: &Url,
        head: &ResponseHead,
        body_size: usize,
        metadata: &FetchResponseMetadata,
        delivery: Option<&AsyncSubresourceFetchResponseFilter>,
    ) -> ResourcePerformanceEntry {
        let readable = metadata.response_type.is_readable();
        let mut entry = ResourcePerformanceEntry::from_streaming_network_response(
            url.as_str(),
            self.initiator_type,
            Some(self.start_unix_millis),
            head,
            body_size,
        );
        // CORS controls response metadata and script attribution. TAO controls
        // timing/body sizes separately, including for opaque responses.
        if readable && head.cache_state == ResponseCacheState::None {
            entry.initiator_url.clone_from(&self.initiator_url);
        }
        if readable
            && self
                .response_timing
                .borrow()
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
        if delivery.is_some_and(|delivery| delivery.service_worker_metadata().is_some()) {
            // Bytes delivered by a Service Worker are not a network transfer
            // to this global, even when respondWith forwards a fetched body.
            entry.transfer_size = 0.0;
        }
        entry.end_unix_millis = Some(self.end());
        if metadata.timing_allow_passed {
            if let Some(timing) = self.response_timing.borrow().as_ref() {
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
        if !metadata.timing_allow_passed {
            entry.delivery_type.clear();
            entry.transfer_size = 0.0;
            entry.encoded_body_size = 0.0;
            entry.decoded_body_size = 0.0;
        }
        entry
    }

    pub(crate) fn failed(&self, url: &Url) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_network_failure(
            url.as_str(),
            self.initiator_type,
            Some(self.start_unix_millis),
        );
        entry.end_unix_millis = Some(self.end());
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_fetch::FetchResponseType::{Basic, Opaque};

    #[test]
    fn completed_uses_the_response_snapshot_and_keeps_delivery_separate() {
        let timing = FetchResourceTiming {
            initiator_type: "fetch",
            started: std::time::Instant::now(),
            response_timing: Default::default(),
            start_unix_millis: 42.0,
            initiator_url: "https://page.test/script.js".to_owned(),
        };
        let url = Url::parse("https://cross.test/body").unwrap();
        let head = ResponseHead {
            final_url: url.clone(),
            status: 200,
            status_text: None,
            headers: vec![
                ("Content-Type".into(), b"text/plain".to_vec()),
                ("Timing-Allow-Origin".into(), b"*".to_vec()),
            ],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            cache_state: Default::default(),
            preload_state: Default::default(),
            negotiated_http_version: None,
        };
        let delivered = AsyncSubresourceFetchResponseFilter::service_worker(Default::default());
        for (kind, tao, service_worker, transfer, size, status) in [
            (Basic, true, false, 309.0, 9.0, 200.0),
            (Opaque, true, false, 309.0, 9.0, 0.0),
            (Basic, false, false, 0.0, 0.0, 200.0),
            (Opaque, false, false, 0.0, 0.0, 0.0),
            (Basic, true, true, 0.0, 9.0, 200.0),
            (Opaque, true, true, 0.0, 9.0, 0.0),
            (Basic, false, true, 0.0, 0.0, 200.0),
            (Opaque, false, true, 0.0, 0.0, 0.0),
        ] {
            let metadata = FetchResponseMetadata {
                response_type: kind,
                timing_allow_passed: tao,
                ..Default::default()
            };
            let entry = timing.completed(
                &url,
                &head,
                9,
                &metadata,
                service_worker.then_some(&delivered),
            );
            assert_eq!(entry.transfer_size, transfer);
            assert_eq!(entry.encoded_body_size, size);
            assert_eq!(entry.decoded_body_size, size);
            assert_eq!(entry.response_status, status);
        }
    }
}
