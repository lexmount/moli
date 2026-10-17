//! Request-owned Resource Timing, independent of CDP capture and JS receivers.

use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;

use moli_fetch::{
    FetchCancelHandle, FetchResponseFilter, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, ResponseHead,
};
use moli_url::WebOrigin;
use url::Url;

use crate::context_bootstrap::{FetchResourceTiming, ResourcePerformanceEntry};
use crate::network_host::{
    FetchResponseRequest, validate_cors_response_chain, validate_fetch_response_headers,
};
use crate::types::SubresourcePolicyContext;

pub(super) enum WorkerTimingRequest {
    Fetch {
        method: String,
        mode: RequestMode,
        credentials: RequestCredentialsMode,
        redirect: RequestRedirectMode,
        policy: SubresourcePolicyContext,
        font: bool,
    },
    Xhr {
        credentials: RequestCredentialsMode,
    },
}

impl WorkerTimingRequest {
    fn filter(&self, origin: &WebOrigin, head: &ResponseHead) -> Option<FetchResponseFilter> {
        let (method, mode, credentials, redirect) = match self {
            Self::Fetch {
                method,
                mode,
                credentials,
                redirect,
                policy,
                ..
            } => {
                validate_fetch_response_headers(origin, head, *mode, *credentials, *policy).ok()?;
                (method.as_str(), *mode, *credentials, *redirect)
            }
            Self::Xhr { credentials } => {
                validate_cors_response_chain(origin, head, *credentials).ok()?;
                (
                    "GET",
                    RequestMode::Cors,
                    *credentials,
                    RequestRedirectMode::Follow,
                )
            }
        };
        Some(
            FetchResponseRequest {
                method,
                mode,
                redirect_mode: redirect,
            }
            .network_response_filter(origin, head, credentials),
        )
    }

    fn initiator_type(&self) -> &'static str {
        match self {
            Self::Fetch { font: true, .. } => "css",
            Self::Fetch { font: false, .. } => "fetch",
            Self::Xhr { .. } => "xmlhttprequest",
        }
    }
}

/// IO owns observations; the Worker owner selects the response and publishes.
/// A terminal IO event also retains this owner after abort/open retires the
/// pending JS request, so an already completed response cannot lose its entry.
#[derive(Clone)]
pub(in crate::worker) struct WorkerResourceTiming(Arc<WorkerResourceTimingInner>);

struct WorkerResourceTimingInner {
    name: Url,
    origin: WebOrigin,
    request: WorkerTimingRequest,
    state: Mutex<WorkerResourceTimingState>,
}

struct WorkerResourceTimingState {
    clock: FetchResourceTiming,
    transport: Option<FetchCancelHandle>,
    response: Option<(ResponseHead, FetchResponseFilter)>,
    body_size: usize,
    completed_at: Option<Instant>,
    synthetic_response_start: Option<Instant>,
    awaiting_selection: bool,
    aborted: bool,
    reported: bool,
}

impl WorkerResourceTiming {
    pub(super) fn new(name: Url, origin: WebOrigin, request: WorkerTimingRequest) -> Self {
        let clock = FetchResourceTiming::for_initiator_type(request.initiator_type());
        Self(Arc::new(WorkerResourceTimingInner {
            name,
            origin,
            request,
            state: Mutex::new(WorkerResourceTimingState {
                clock,
                transport: None,
                response: None,
                body_size: 0,
                completed_at: None,
                synthetic_response_start: None,
                awaiting_selection: false,
                aborted: false,
                reported: false,
            }),
        }))
    }

    pub(super) fn origin(&self) -> &WebOrigin {
        &self.0.origin
    }

    pub(super) fn begin_transport(&self, cancel: FetchCancelHandle) {
        let mut state = self.0.state.lock();
        state.transport = Some(cancel);
        state.response = None;
        state.body_size = 0;
        state.completed_at = None;
        state.synthetic_response_start = None;
        state.awaiting_selection = false;
        state.aborted = false;
    }

    pub(super) fn observe_head(&self, head: &ResponseHead) {
        let filter = self.0.request.filter(&self.0.origin, head);
        let mut state = self.0.state.lock();
        state.response = filter.map(|filter| (head.clone(), filter));
    }

    pub(super) fn observe_chunk(&self, size: usize) {
        let mut state = self.0.state.lock();
        state.body_size = state.body_size.saturating_add(size);
    }

    pub(super) fn complete(&self) {
        let mut state = self.0.state.lock();
        state.completed_at.get_or_insert_with(Instant::now);
    }

    pub(super) fn pause(&self) {
        self.0.state.lock().awaiting_selection = true;
    }

    pub(super) fn discard_response(&self) {
        let mut state = self.0.state.lock();
        state.transport = None;
        state.response = None;
        state.body_size = 0;
        state.completed_at = Some(Instant::now());
        state.synthetic_response_start = None;
    }

    pub(super) fn begin_synthetic_response(&self) {
        self.discard_response();
        let mut state = self.0.state.lock();
        state.synthetic_response_start = state.completed_at;
    }

    pub(super) fn select_response(
        &self,
        head: &ResponseHead,
        filter: FetchResponseFilter,
        body_size: usize,
        synthetic: bool,
    ) {
        let mut state = self.0.state.lock();
        if synthetic && state.synthetic_response_start.is_none() {
            // A fulfilled/intercepted response is a new client handover. The
            // transport that previously supplied a challenge is not its clock.
            state.transport = None;
            let now = Instant::now();
            state.synthetic_response_start = Some(now);
            state.completed_at = Some(now);
        }
        state.response = Some((head.clone(), filter));
        state.body_size = body_size;
        state.awaiting_selection = false;
    }

    pub(super) fn publish(&self, scope: &mut v8::PinScope<'_, '_>) {
        self.publish_entry(scope, false, false);
    }

    pub(super) fn reject(&self, scope: &mut v8::PinScope<'_, '_>) {
        self.publish_entry(scope, true, false);
    }

    pub(super) fn abort(&self, scope: &mut v8::PinScope<'_, '_>) {
        {
            let mut state = self.0.state.lock();
            state.aborted = true;
            if !state.awaiting_selection
                && state.transport.as_ref().is_some_and(|transport| {
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
        let completed = !state.awaiting_selection
            && native.is_some_and(|timing| timing.response_end_at().is_some());
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
        let body_size = if aborted && !completed {
            0
        } else {
            state.body_size
        };
        let entry = if !rejected && let Some((head, filter)) = &state.response {
            if aborted {
                state
                    .clock
                    .aborted_response(&self.0.name, &self.0.origin, head, filter, body_size)
            } else {
                state
                    .clock
                    .response(&self.0.name, &self.0.origin, head, filter, state.body_size)
            }
        } else {
            state.clock.failure(&self.0.name)
        };
        Some(state.clock.completed_at(entry, end))
    }
}
