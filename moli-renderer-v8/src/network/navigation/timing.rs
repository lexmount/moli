use std::{sync::Arc, time::Instant};

use moli_fetch::{FetchCancelHandle, FetchResponseTiming, ResponseHead};
use parking_lot::Mutex;

/// Clock and response observations owned by one navigation, transferred to its
/// committed Document. This does not retain the navigation's load authority.
#[derive(Clone, Debug)]
pub(crate) struct NavigationTiming(Arc<NavigationTimingInner>);

#[derive(Debug)]
struct NavigationTimingInner {
    started: Instant,
    time_origin: f64,
    cancel: FetchCancelHandle,
    response: Mutex<Option<NavigationTimingResponse>>,
}

#[derive(Clone, Debug)]
struct NavigationTimingResponse {
    head: ResponseHead,
    headers_received: Instant,
    materialized_body_size: Option<usize>,
}

pub(crate) struct NavigationTimingSnapshot {
    pub started: Instant,
    pub time_origin: f64,
    pub native: FetchResponseTiming,
    pub head: ResponseHead,
    pub headers_received: Instant,
    pub materialized_body_size: Option<usize>,
}

impl NavigationTiming {
    pub(super) fn new(cancel: FetchCancelHandle) -> Self {
        Self(Arc::new(NavigationTimingInner {
            started: Instant::now(),
            time_origin: moli_time::unix_epoch_millis(),
            cancel,
            response: Mutex::new(None),
        }))
    }

    pub(crate) fn time_origin(&self) -> f64 {
        self.0.time_origin
    }

    pub(super) fn observe_response(&self, head: ResponseHead, body_size: Option<usize>) {
        *self.0.response.lock() = Some(NavigationTimingResponse {
            head,
            headers_received: Instant::now(),
            materialized_body_size: body_size,
        });
    }

    pub(crate) fn snapshot(&self) -> Option<NavigationTimingSnapshot> {
        let response = self.0.response.lock().clone()?;
        Some(NavigationTimingSnapshot {
            started: self.0.started,
            time_origin: self.0.time_origin,
            native: self.0.cancel.response_timing(),
            head: response.head,
            headers_received: response.headers_received,
            materialized_body_size: response.materialized_body_size,
        })
    }
}
