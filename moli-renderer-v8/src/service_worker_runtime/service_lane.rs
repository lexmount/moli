use std::{collections::VecDeque, sync::Arc};

use parking_lot::Mutex;

use super::{
    ids::ServiceWorkerVersionId, owner_wake::ServiceWorkerOwnerWake,
    service::ServiceWorkerRuntimeService, start_completion::ServiceWorkerRuntimeCompletion,
};

/// Protocol-neutral result sent back by one exact auxiliary Page owner.
///
/// This message deliberately contains no ServiceWorker runtime handle. That
/// keeps renderer output `Send` without introducing a
/// runtime -> output -> continuation -> runtime ownership cycle.
pub(crate) struct ServiceWorkerOpenWindowPageCompletion {
    pub(super) expected_page_id: crate::runtime::PageId,
    pub(super) committed_page: Option<(crate::runtime::PageId, u64)>,
    pub(super) request_id: u64,
    pub(super) source_version_id: ServiceWorkerVersionId,
    pub(super) source_run: crate::runtime::RendererServiceWorkerRunIdentity,
}

#[derive(Clone)]
pub(crate) struct ServiceWorkerOpenWindowCompletionEndpoint {
    queue: Arc<ServiceWorkerOpenWindowCompletionQueue>,
}

pub(super) struct ServiceWorkerOpenWindowCompletionQueue {
    events: Mutex<VecDeque<ServiceWorkerOpenWindowPageCompletion>>,
    owner_wake: Arc<ServiceWorkerOwnerWake>,
}

impl ServiceWorkerOpenWindowCompletionQueue {
    pub(super) fn new(owner_wake: Arc<ServiceWorkerOwnerWake>) -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(VecDeque::new()),
            owner_wake,
        })
    }

    pub(super) fn endpoint(self: &Arc<Self>) -> ServiceWorkerOpenWindowCompletionEndpoint {
        ServiceWorkerOpenWindowCompletionEndpoint {
            queue: self.clone(),
        }
    }

    fn take(&self) -> VecDeque<ServiceWorkerOpenWindowPageCompletion> {
        std::mem::take(&mut *self.events.lock())
    }

    fn pending_count(&self) -> usize {
        self.events.lock().len()
    }
}

impl ServiceWorkerOpenWindowCompletionEndpoint {
    pub(crate) fn enqueue_committed_page(
        &self,
        expected_page_id: crate::runtime::PageId,
        page_id: crate::runtime::PageId,
        service_worker_client_id: u64,
        request_id: u64,
        source_version_id: ServiceWorkerVersionId,
        source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    ) {
        self.enqueue(ServiceWorkerOpenWindowPageCompletion {
            expected_page_id,
            committed_page: Some((page_id, service_worker_client_id)),
            request_id,
            source_version_id,
            source_run,
        });
    }

    pub(crate) fn enqueue_null(
        &self,
        expected_page_id: crate::runtime::PageId,
        request_id: u64,
        source_version_id: ServiceWorkerVersionId,
        source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    ) {
        self.enqueue(ServiceWorkerOpenWindowPageCompletion {
            expected_page_id,
            committed_page: None,
            request_id,
            source_version_id,
            source_run,
        });
    }

    fn enqueue(&self, completion: ServiceWorkerOpenWindowPageCompletion) {
        self.queue.events.lock().push_back(completion);
        self.queue.owner_wake.signal_service_lane_wake();
    }
}

#[derive(Default)]
pub(super) struct ServiceWorkerServiceLane {
    events: Mutex<VecDeque<ServiceWorkerServiceLaneEvent>>,
}

enum ServiceWorkerServiceLaneEvent {
    Completion(Box<ServiceWorkerRuntimeCompletion>),
}

impl ServiceWorkerServiceLane {
    pub(super) fn enqueue_completion(&self, completion: ServiceWorkerRuntimeCompletion) {
        self.events
            .lock()
            .push_back(ServiceWorkerServiceLaneEvent::Completion(Box::new(
                completion,
            )));
    }

    pub(super) fn drain(&self) -> usize {
        let events = std::mem::take(&mut *self.events.lock());
        let count = events.len();
        for event in events {
            match event {
                ServiceWorkerServiceLaneEvent::Completion(completion) => completion.complete(),
            }
        }
        count
    }

    pub(super) fn pending_count(&self) -> usize {
        self.events.lock().len()
    }
}

impl ServiceWorkerRuntimeService {
    pub(crate) fn drain_service_lane(&self) -> usize {
        let external = self.open_window_completion_queue().take();
        let external_count = external.len();
        for completion in external {
            self.finish_clients_open_window_page_completion(completion);
        }
        external_count + self.service_lane().drain()
    }

    pub(super) fn enqueue_service_lane_completion(
        &self,
        completion: ServiceWorkerRuntimeCompletion,
    ) {
        self.service_lane().enqueue_completion(completion);
    }

    pub(crate) fn pending_service_lane_event_count(&self) -> usize {
        self.open_window_completion_queue().pending_count() + self.service_lane().pending_count()
    }
}
