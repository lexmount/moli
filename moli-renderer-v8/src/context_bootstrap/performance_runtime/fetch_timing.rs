use super::ResourcePerformanceEntry;
use crate::types::AsyncSubresourceFetchResponseFilter;
use moli_fetch::{FetchResponseMetadata, ResponseCacheState, ResponseHead};
use moli_url::WebOrigin;
use url::Url;

/// Request-time metadata, independent of the callback or Realm that happens to
/// be active when the body finishes. This must not retain any V8 objects.
#[derive(Clone)]
pub(crate) struct FetchResourceTiming {
    started: std::time::Instant,
    start_unix_millis: f64,
    response_start_unix_millis: std::cell::Cell<Option<f64>>,
    initiator_url: String,
}

impl FetchResourceTiming {
    pub(crate) fn capture(scope: &mut v8::PinScope<'_, '_>, origin: &WebOrigin) -> Self {
        Self {
            started: std::time::Instant::now(),
            start_unix_millis: crate::context_bootstrap::unix_epoch_millis(),
            response_start_unix_millis: std::cell::Cell::new(None),
            initiator_url: crate::resource_initiator::current_url(scope, origin),
        }
    }

    fn now(&self) -> f64 {
        self.start_unix_millis + self.started.elapsed().as_secs_f64() * 1000.0
    }

    pub(crate) fn response_started(&self) {
        self.response_start_unix_millis.set(Some(self.now()));
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
        if delivery.is_some_and(|delivery| delivery.service_worker_metadata().is_some()) {
            // Bytes delivered by a Service Worker are not a network transfer
            // to this global, even when respondWith forwards a fetched body.
            entry.transfer_size = 0.0;
        }
        entry.end_unix_millis = Some(self.now());
        if metadata.timing_allow_passed {
            entry.response_start_unix_millis = self.response_start_unix_millis.get();
            entry.next_hop_protocol = head
                .negotiated_http_version
                .map(|version| version.protocol_name().to_owned())
                .unwrap_or_default();
        }
        if !metadata.timing_allow_passed {
            entry.transfer_size = 0.0;
            entry.encoded_body_size = 0.0;
            entry.decoded_body_size = 0.0;
        }
        entry
    }

    pub(crate) fn failed(&self, url: &Url) -> ResourcePerformanceEntry {
        let mut entry = ResourcePerformanceEntry::from_network_failure(
            url.as_str(),
            "fetch",
            Some(self.start_unix_millis),
        );
        entry.end_unix_millis = Some(self.now());
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
            started: std::time::Instant::now(),
            response_start_unix_millis: std::cell::Cell::new(None),
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
