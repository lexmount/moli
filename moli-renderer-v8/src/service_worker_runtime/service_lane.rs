use std::collections::VecDeque;

use parking_lot::Mutex;

use super::{
    service::ServiceWorkerRuntimeService, start_completion::ServiceWorkerRuntimeCompletion,
};

#[derive(Default)]
pub(super) struct ServiceWorkerServiceLane {
    state: Mutex<ServiceWorkerServiceLaneState>,
}

#[derive(Default)]
struct ServiceWorkerServiceLaneState {
    events: VecDeque<ServiceWorkerServiceLaneEvent>,
    draining: bool,
}

struct ServiceWorkerServiceLaneDrain<'a>(&'a ServiceWorkerServiceLane);

impl Drop for ServiceWorkerServiceLaneDrain<'_> {
    fn drop(&mut self) {
        self.0.state.lock().draining = false;
    }
}

enum ServiceWorkerServiceLaneEvent {
    Completion(Box<ServiceWorkerRuntimeCompletion>),
    #[cfg(test)]
    Callback(Box<dyn FnOnce() + Send>),
}

impl ServiceWorkerServiceLane {
    pub(super) fn enqueue_completion(&self, completion: ServiceWorkerRuntimeCompletion) {
        self.state
            .lock()
            .events
            .push_back(ServiceWorkerServiceLaneEvent::Completion(Box::new(
                completion,
            )));
    }

    pub(super) fn drain(&self) -> Option<usize> {
        let events = {
            let mut state = self.state.lock();
            // Every Page owner can receive the same service wake. Claim the
            // whole batch so a peer cannot deliver chunks or completion ahead
            // of a response head that is still being processed.
            if state.draining {
                return None;
            }
            state.draining = true;
            std::mem::take(&mut state.events)
        };
        let _drain = ServiceWorkerServiceLaneDrain(self);
        let count = events.len();
        for event in events {
            match event {
                ServiceWorkerServiceLaneEvent::Completion(completion) => completion.complete(),
                #[cfg(test)]
                ServiceWorkerServiceLaneEvent::Callback(callback) => callback(),
            }
        }
        Some(count)
    }

    pub(super) fn pending_count(&self) -> usize {
        self.state.lock().events.len()
    }
}

impl ServiceWorkerRuntimeService {
    pub(crate) fn drain_service_lane(&self) -> usize {
        let Some(count) = self.service_lane().drain() else {
            return 0;
        };
        // A peer may consume its wake while another owner is draining. Rearm
        // arrivals after releasing the claim, keeping each batch bounded.
        if self.pending_service_lane_event_count() > 0 {
            self.signal_service_lane_wake();
        }
        count
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};

    use super::*;
    use crate::service_worker_runtime::{
        ServiceWorkerRuntimeOwnerWake, service_worker_owner_wake_channel,
    };

    impl ServiceWorkerServiceLane {
        fn enqueue_callback(&self, callback: impl FnOnce() + Send + 'static) {
            self.state
                .lock()
                .events
                .push_back(ServiceWorkerServiceLaneEvent::Callback(Box::new(callback)));
        }
    }

    #[test]
    fn concurrent_drains_preserve_service_worker_completion_order() {
        let lane = Arc::new(ServiceWorkerServiceLane::default());
        let order = Arc::new(Mutex::new(Vec::new()));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let first_order = Arc::clone(&order);
        lane.enqueue_callback(move || {
            first_order.lock().push("head-start");
            entered_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
            first_order.lock().push("head-end");
        });
        let first_lane = Arc::clone(&lane);
        let first = std::thread::spawn(move || first_lane.drain());
        entered_rx.recv().unwrap();

        let next_order = Arc::clone(&order);
        lane.enqueue_callback(move || next_order.lock().push("chunk"));
        let peer = lane.drain();
        let before_resume = order.lock().clone();
        resume_tx.send(()).unwrap();
        let first_count = first.join().unwrap();

        assert_eq!(peer, None, "a peer must leave the in-flight batch alone");
        assert_eq!(before_resume, ["head-start"]);
        assert_eq!(first_count, Some(1));
        assert_eq!(lane.pending_count(), 1);
        assert_eq!(lane.drain(), Some(1));
        assert_eq!(*order.lock(), ["head-start", "head-end", "chunk"]);
    }

    #[test]
    fn reentrant_drain_defers_arrivals_and_rearms_the_owner_wake() {
        let owner = super::super::service::new_service_worker_runtime_service();
        let service = ServiceWorkerRuntimeService::clone(&owner);
        let (wake_tx, mut wake_rx) = service_worker_owner_wake_channel();
        service.add_owner_wake_sender(wake_tx);
        let order = Arc::new(Mutex::new(Vec::new()));
        let nested_service = service.clone();
        let nested_order = Arc::clone(&order);
        service.service_lane().enqueue_callback(move || {
            nested_order.lock().push("head-start");
            let chunk_order = Arc::clone(&nested_order);
            nested_service
                .service_lane()
                .enqueue_callback(move || chunk_order.lock().push("chunk"));
            assert_eq!(nested_service.drain_service_lane(), 0);
            nested_order.lock().push("head-end");
        });

        assert_eq!(service.drain_service_lane(), 1);
        assert_eq!(*order.lock(), ["head-start", "head-end"]);
        assert!(matches!(
            wake_rx.try_recv(),
            Ok(ServiceWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(service.pending_service_lane_event_count(), 1);
        assert_eq!(service.drain_service_lane(), 1);
        assert_eq!(*order.lock(), ["head-start", "head-end", "chunk"]);
        assert!(wake_rx.try_recv().is_err(), "an empty lane needs no wake");
    }
}
