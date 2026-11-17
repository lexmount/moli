//! Request-owned Resource Timing, independent of CDP capture and JS receivers.

use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;

use moli_fetch::{
    FetchCancelHandle, RequestCredentialsMode, RequestMode, RequestRedirectMode, ResponseHead,
};
use moli_url::WebOrigin;
use url::Url;

use crate::context_bootstrap::{FetchResourceTiming, ResourcePerformanceEntry};
use crate::network_host::{validate_cors_response_chain, validate_fetch_response_headers};
use crate::types::{AsyncSubresourceFetchResponseFilter, SubresourcePolicyContext};

pub(super) enum WorkerTimingRequest {
    Fetch {
        mode: RequestMode,
        credentials: RequestCredentialsMode,
        redirect: RequestRedirectMode,
        policy: SubresourcePolicyContext,
        font: bool,
    },
    Xhr {
        credentials: RequestCredentialsMode,
    },
    EventSource {
        credentials: RequestCredentialsMode,
        policy: SubresourcePolicyContext,
    },
}

impl WorkerTimingRequest {
    fn response_snapshot(
        &self,
        origin: &WebOrigin,
        head: &ResponseHead,
        web_security_disabled: bool,
    ) -> Option<(
        moli_fetch::FetchResponseMetadata,
        Option<AsyncSubresourceFetchResponseFilter>,
    )> {
        let (mode, redirect) = match self {
            Self::Fetch {
                mode,
                credentials,
                redirect,
                policy,
                ..
            } => {
                validate_fetch_response_headers(
                    origin,
                    head,
                    *mode,
                    *credentials,
                    *policy,
                    web_security_disabled,
                )
                .ok()?;
                (*mode, *redirect)
            }
            Self::Xhr { credentials } => {
                validate_cors_response_chain(origin, head, *credentials, web_security_disabled)
                    .ok()?;
                (RequestMode::Cors, RequestRedirectMode::Follow)
            }
            Self::EventSource {
                credentials,
                policy,
            } => {
                validate_fetch_response_headers(
                    origin,
                    head,
                    RequestMode::Cors,
                    *credentials,
                    *policy,
                    web_security_disabled,
                )
                .ok()?;
                (RequestMode::Cors, RequestRedirectMode::Follow)
            }
        };
        let delivery = crate::network_host::network_response_filter(
            origin,
            head,
            mode,
            redirect,
            web_security_disabled,
        );
        let metadata = crate::network_host::fetch_response_metadata(
            origin,
            head,
            mode,
            delivery.as_ref(),
            web_security_disabled,
        );
        Some((metadata, delivery))
    }

    fn initiator_type(&self) -> &'static str {
        match self {
            Self::Fetch { font: true, .. } => "css",
            Self::Fetch { font: false, .. } => "fetch",
            Self::Xhr { .. } => "xmlhttprequest",
            Self::EventSource { .. } => "other",
        }
    }
}

/// IO owns observations; the Worker owner selects the response and publishes.
/// A terminal IO event also retains this owner after abort/open retires the
/// pending JS request, so an already completed response cannot lose its entry.
#[derive(Clone)]
pub(crate) struct WorkerResourceTiming(Arc<WorkerResourceTimingInner>);

struct WorkerResourceTimingInner {
    name: Url,
    origin: WebOrigin,
    request: WorkerTimingRequest,
    web_security_disabled: bool,
    state: Mutex<WorkerResourceTimingState>,
}

struct WorkerResourceTimingState {
    clock: FetchResourceTiming,
    transport: Option<FetchCancelHandle>,
    response: Option<WorkerTimingResponse>,
    body_size: usize,
    completed_at: Option<Instant>,
    synthetic_response_start: Option<Instant>,
    response_is_synthetic: bool,
    aborted: bool,
    reported: bool,
}

struct WorkerTimingResponse {
    head: ResponseHead,
    metadata: Arc<moli_fetch::FetchResponseMetadata>,
    delivery: Option<AsyncSubresourceFetchResponseFilter>,
}

impl WorkerResourceTiming {
    pub(super) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        name: Url,
        origin: WebOrigin,
        request: WorkerTimingRequest,
        web_security_disabled: bool,
    ) -> Self {
        let clock = FetchResourceTiming::capture_for_initiator_type(
            scope,
            &origin,
            request.initiator_type(),
        );
        Self(Arc::new(WorkerResourceTimingInner {
            name,
            origin,
            request,
            web_security_disabled,
            state: Mutex::new(WorkerResourceTimingState {
                clock,
                transport: None,
                response: None,
                body_size: 0,
                completed_at: None,
                synthetic_response_start: None,
                response_is_synthetic: false,
                aborted: false,
                reported: false,
            }),
        }))
    }

    pub(crate) fn origin(&self) -> &WebOrigin {
        &self.0.origin
    }

    pub(crate) fn begin_transport(&self, cancel: FetchCancelHandle) {
        let mut state = self.0.state.lock();
        state.transport = Some(cancel);
        state.response = None;
        state.body_size = 0;
        state.completed_at = None;
        state.synthetic_response_start = None;
        state.response_is_synthetic = false;
        state.aborted = false;
    }

    pub(crate) fn observe_head(&self, head: &ResponseHead) {
        let snapshot =
            self.0
                .request
                .response_snapshot(&self.0.origin, head, self.0.web_security_disabled);
        let mut state = self.0.state.lock();
        state.response = snapshot.map(|(metadata, delivery)| WorkerTimingResponse {
            head: head.clone(),
            metadata: Arc::new(metadata),
            delivery,
        });
        state.response_is_synthetic = false;
    }

    pub(crate) fn observe_chunk(&self, size: usize) {
        let mut state = self.0.state.lock();
        state.body_size = state.body_size.saturating_add(size);
    }

    pub(crate) fn complete(&self) {
        let mut state = self.0.state.lock();
        state.completed_at.get_or_insert_with(Instant::now);
    }

    pub(crate) fn discard_response(&self) {
        let mut state = self.0.state.lock();
        if let Some(transport) = state.transport.take() {
            state
                .clock
                .observe_service_worker_timing(transport.response_timing());
        }
        state.response = None;
        state.body_size = 0;
        state.completed_at = Some(Instant::now());
        state.synthetic_response_start = None;
        state.response_is_synthetic = false;
    }

    pub(crate) fn begin_synthetic_response(&self) {
        self.discard_response();
        let mut state = self.0.state.lock();
        state.synthetic_response_start = state.completed_at.take();
        state.response_is_synthetic = true;
    }

    /// Use the same immutable decision that constructs the native Fetch response.
    pub(crate) fn select_native_response(
        &self,
        head: &ResponseHead,
        metadata: Arc<moli_fetch::FetchResponseMetadata>,
        delivery: Option<AsyncSubresourceFetchResponseFilter>,
        body_size: Option<usize>,
        synthetic: bool,
    ) {
        let mut state = self.0.state.lock();
        if synthetic && state.synthetic_response_start.is_none() {
            if let Some(transport) = state.transport.take() {
                state
                    .clock
                    .observe_service_worker_timing(transport.response_timing());
            }
            state.synthetic_response_start = Some(Instant::now());
        }
        state.response = Some(WorkerTimingResponse {
            head: head.clone(),
            metadata,
            delivery,
        });
        state.response_is_synthetic = synthetic;
        if let Some(body_size) = body_size {
            state.body_size = body_size;
        }
    }

    pub(crate) fn select_response(
        &self,
        head: &ResponseHead,
        filter: AsyncSubresourceFetchResponseFilter,
        body_size: usize,
        synthetic: bool,
    ) {
        let mode = match &self.0.request {
            WorkerTimingRequest::Fetch { mode, .. } => *mode,
            WorkerTimingRequest::Xhr { .. } | WorkerTimingRequest::EventSource { .. } => {
                RequestMode::Cors
            }
        };
        let metadata = crate::network_host::fetch_response_metadata(
            &self.0.origin,
            head,
            mode,
            Some(&filter),
            self.0.web_security_disabled,
        );
        self.select_native_response(
            head,
            Arc::new(metadata),
            Some(filter),
            Some(body_size),
            synthetic,
        );
    }

    pub(crate) fn publish(&self, scope: &mut v8::PinScope<'_, '_>) {
        self.publish_entry(scope, false, false);
    }

    pub(crate) fn reject(&self, scope: &mut v8::PinScope<'_, '_>) {
        self.publish_entry(scope, true, false);
    }

    pub(crate) fn abort(&self, scope: &mut v8::PinScope<'_, '_>) {
        {
            let mut state = self.0.state.lock();
            state.aborted = true;
            if state.completed_at.is_some()
                || state.transport.as_ref().is_some_and(|transport| {
                    transport.response_timing().response_end_at().is_some()
                })
            {
                // IO has committed its terminal but its Worker delivery may
                // still be queued behind running JS. Let that event report.
                return;
            }
        }
        self.publish_entry(scope, false, true);
    }

    fn publish_entry(&self, scope: &mut v8::PinScope<'_, '_>, rejected: bool, aborted: bool) {
        if let Some(entry) = self.take_entry(rejected, aborted) {
            crate::context_bootstrap::record_resource_performance_entry(scope, entry);
        }
    }

    fn take_entry(&self, rejected: bool, aborted: bool) -> Option<ResourcePerformanceEntry> {
        let mut state = self.0.state.lock();
        if state.reported || !matches!(self.0.name.scheme(), "http" | "https") {
            return None;
        }
        state.reported = true;
        let aborted = aborted || state.aborted;
        let native = state
            .transport
            .as_ref()
            .map(FetchCancelHandle::response_timing);
        let completed = state.completed_at.is_some()
            || native.is_some_and(|timing| timing.response_end_at().is_some());
        let end = if aborted && !completed {
            Instant::now()
        } else {
            native
                .and_then(|timing| timing.response_end_at())
                .or(state.completed_at)
                .unwrap_or_else(Instant::now)
        };
        let start = native
            .and_then(|timing| timing.response_started_at())
            .or(state.synthetic_response_start);
        state.clock.response_started_at(start);
        if let Some(native) = native {
            state.clock.observe_native_timing(native);
        }
        let body_size = if aborted && !completed {
            0
        } else {
            state.body_size
        };
        let mut entry = if !rejected && let Some(response) = &state.response {
            let mut entry = state.clock.completed(
                &self.0.name,
                &response.head,
                body_size,
                &response.metadata,
                response.delivery.as_ref(),
            );
            if aborted {
                entry.mark_aborted(completed);
            }
            entry
        } else {
            state.clock.failed(&self.0.name)
        };
        if state.response_is_synthetic {
            entry.clear_initiator();
        }
        Some(state.clock.completed_at(entry, end))
    }
}
