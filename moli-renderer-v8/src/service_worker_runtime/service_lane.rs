use std::collections::VecDeque;

use parking_lot::Mutex;

use super::{
    service::ServiceWorkerRuntimeService, start_completion::ServiceWorkerRuntimeCompletion,
};

pub(super) struct ServiceWorkerServiceLane {
    events: Mutex<Option<VecDeque<Box<ServiceWorkerRuntimeCompletion>>>>,
}

impl Default for ServiceWorkerServiceLane {
    fn default() -> Self {
        Self {
            events: Mutex::new(Some(VecDeque::new())),
        }
    }
}

impl ServiceWorkerServiceLane {
    pub(super) fn enqueue_completion(&self, completion: ServiceWorkerRuntimeCompletion) {
        if let Some(events) = self.events.lock().as_mut() {
            events.push_back(Box::new(completion));
        }
    }

    pub(super) fn close(&self) {
        let discarded = self.events.lock().take();
        drop(discarded);
    }

    pub(super) fn drain(&self) -> usize {
        let events = self
            .events
            .lock()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let count = events.len();
        for event in events {
            event.complete();
        }
        count
    }

    pub(super) fn pending_count(&self) -> usize {
        self.events.lock().as_ref().map_or(0, VecDeque::len)
    }
}

impl ServiceWorkerRuntimeService {
    pub(crate) fn drain_service_lane(&self) -> usize {
        self.service_lane().drain()
    }

    pub(super) fn enqueue_service_lane_completion(
        &self,
        completion: ServiceWorkerRuntimeCompletion,
    ) {
        self.service_lane().enqueue_completion(completion);
    }

    pub(crate) fn pending_service_lane_event_count(&self) -> usize {
        self.service_lane().pending_count()
    }
}
