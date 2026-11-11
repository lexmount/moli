use std::collections::VecDeque;

use moli_shared_worker::{SharedWorkerInstanceId, SharedWorkerKey};
use parking_lot::Mutex;

use crate::worker::WorkerParentErrorEventKind;

use super::{
    load_completion::SharedWorkerRuntimeCompletion,
    service::{SharedWorkerRuntimeService, WeakSharedWorkerRuntimeService},
};

#[derive(Default)]
pub(super) struct SharedWorkerServiceLane {
    events: Mutex<VecDeque<SharedWorkerServiceLaneEvent>>,
}

enum SharedWorkerServiceLaneEvent {
    Completion(Box<SharedWorkerRuntimeCompletion>),
    IdleTimeout {
        runtime_service: WeakSharedWorkerRuntimeService,
        key: SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
    },
    WorkerClosed {
        runtime_service: WeakSharedWorkerRuntimeService,
        instance_id: SharedWorkerInstanceId,
    },
    WorkerBootstrapError {
        runtime_service: WeakSharedWorkerRuntimeService,
        instance_id: SharedWorkerInstanceId,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    },
}

impl SharedWorkerServiceLane {
    pub(super) fn enqueue_completion(&self, completion: SharedWorkerRuntimeCompletion) {
        self.events
            .lock()
            .push_back(SharedWorkerServiceLaneEvent::Completion(Box::new(
                completion,
            )));
    }

    pub(super) fn enqueue_idle_timeout(
        &self,
        runtime_service: WeakSharedWorkerRuntimeService,
        key: SharedWorkerKey,
        instance_id: SharedWorkerInstanceId,
    ) {
        self.events
            .lock()
            .push_back(SharedWorkerServiceLaneEvent::IdleTimeout {
                runtime_service,
                key,
                instance_id,
            });
    }

    pub(super) fn enqueue_worker_closed(
        &self,
        runtime_service: WeakSharedWorkerRuntimeService,
        instance_id: SharedWorkerInstanceId,
    ) {
        self.events
            .lock()
            .push_back(SharedWorkerServiceLaneEvent::WorkerClosed {
                runtime_service,
                instance_id,
            });
    }

    pub(super) fn enqueue_worker_bootstrap_error(
        &self,
        runtime_service: WeakSharedWorkerRuntimeService,
        instance_id: SharedWorkerInstanceId,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) {
        self.events
            .lock()
            .push_back(SharedWorkerServiceLaneEvent::WorkerBootstrapError {
                runtime_service,
                instance_id,
                message,
                filename,
                lineno,
                colno,
                event_kind,
            });
    }

    pub(super) fn drain(&self) -> usize {
        let events = std::mem::take(&mut *self.events.lock());
        let count = events.len();
        for event in events {
            match event {
                SharedWorkerServiceLaneEvent::Completion(completion) => completion.complete(),
                SharedWorkerServiceLaneEvent::IdleTimeout {
                    runtime_service,
                    key,
                    instance_id,
                } => {
                    if let Some(service) = runtime_service.upgrade() {
                        service.finish_idle_timeout(key, instance_id);
                    }
                }
                SharedWorkerServiceLaneEvent::WorkerClosed {
                    runtime_service,
                    instance_id,
                } => runtime_service.finish_worker_closed(instance_id),
                SharedWorkerServiceLaneEvent::WorkerBootstrapError {
                    runtime_service,
                    instance_id,
                    message,
                    filename,
                    lineno,
                    colno,
                    event_kind,
                } => runtime_service.finish_worker_bootstrap_error(
                    instance_id,
                    message,
                    filename,
                    lineno,
                    colno,
                    event_kind,
                ),
            }
        }
        count
    }

    pub(super) fn pending_count(&self) -> usize {
        self.events.lock().len()
    }
}

impl SharedWorkerRuntimeService {
    pub(crate) fn drain_service_lane(&self) -> usize {
        self.service_lane().drain()
    }

    pub(super) fn enqueue_service_lane_completion(
        &self,
        completion: SharedWorkerRuntimeCompletion,
    ) {
        self.service_lane().enqueue_completion(completion);
    }

    pub(super) fn enqueue_service_lane_worker_closed(
        &self,
        runtime_service: WeakSharedWorkerRuntimeService,
        instance_id: SharedWorkerInstanceId,
    ) {
        self.service_lane()
            .enqueue_worker_closed(runtime_service, instance_id);
    }

    pub(super) fn enqueue_service_lane_worker_bootstrap_error(
        &self,
        runtime_service: WeakSharedWorkerRuntimeService,
        instance_id: SharedWorkerInstanceId,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) {
        self.service_lane().enqueue_worker_bootstrap_error(
            runtime_service,
            instance_id,
            message,
            filename,
            lineno,
            colno,
            event_kind,
        );
    }

    pub(crate) fn pending_service_lane_event_count(&self) -> usize {
        self.service_lane().pending_count()
    }
}

impl WeakSharedWorkerRuntimeService {
    pub(super) fn enqueue_service_lane_completion(
        &self,
        completion: SharedWorkerRuntimeCompletion,
    ) -> bool {
        let Some(service) = self.upgrade() else {
            return false;
        };
        service.enqueue_service_lane_completion(completion);
        true
    }

    pub(super) fn enqueue_service_lane_worker_closed(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> bool {
        let Some(service) = self.upgrade() else {
            return false;
        };
        service.enqueue_service_lane_worker_closed(self.clone(), instance_id);
        true
    }

    pub(super) fn enqueue_service_lane_worker_bootstrap_error(
        &self,
        instance_id: SharedWorkerInstanceId,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) -> bool {
        let Some(service) = self.upgrade() else {
            return false;
        };
        service.enqueue_service_lane_worker_bootstrap_error(
            self.clone(),
            instance_id,
            message,
            filename,
            lineno,
            colno,
            event_kind,
        );
        true
    }
}
