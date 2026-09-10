use std::collections::HashMap;

use parking_lot::Mutex;

use crate::runtime::{
    PendingRendererOutputRecord, RendererOutputRecord, RendererOutputStreamCloseReason,
    RendererOutputStreamIdentity, RendererOutputTransportSenderSlot, RendererProtocolObservation,
    RendererServiceWorkerLifecycle, RendererServiceWorkerObservation, RendererTurnOutputJournal,
    RendererWorkerLifecycle, RendererWorkerLifecycleReporter,
};

use super::ids::ServiceWorkerVersionId;

/// Concrete protocol streams owned by stable ServiceWorker version targets.
///
/// A ServiceWorker may stop and restart many V8 runs while its version target
/// remains alive. Consequently the stream lifetime follows the version, not a
/// worker thread or run run: `Created` opens it, run/status events append
/// to it, and `Destroyed` is its final record before closure.
pub(super) struct ServiceWorkerTargetOutputStreams {
    worker_lifecycle: RendererWorkerLifecycleReporter,
    transport: RendererOutputTransportSenderSlot,
    state: Mutex<ServiceWorkerTargetOutputStreamsState>,
}

#[derive(Default)]
struct ServiceWorkerTargetOutputStreamsState {
    live: HashMap<ServiceWorkerVersionId, RendererTurnOutputJournal>,
    /// ServiceWorker versions may become redundant during BrowserContext
    /// setup. Retain their already-frozen terminal stream until the one-shot
    /// protocol transport binding can deliver it in FIFO order.
    retired_before_transport: Vec<RendererTurnOutputJournal>,
}

impl ServiceWorkerTargetOutputStreams {
    pub(super) fn new(
        worker_lifecycle: RendererWorkerLifecycleReporter,
        transport: RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            worker_lifecycle,
            transport,
            state: Mutex::new(ServiceWorkerTargetOutputStreamsState::default()),
        }
    }

    pub(super) fn bind_transport(&self, transport: crate::runtime::RendererOutputTransportSender) {
        let mut state = self.state.lock();
        self.transport.set(transport.clone());
        for journal in state.live.values() {
            journal.bind_transport(transport.clone());
        }
        for journal in state.retired_before_transport.drain(..) {
            journal.bind_transport(transport.clone());
        }
    }

    pub(super) fn publish_created(
        &self,
        version_id: ServiceWorkerVersionId,
        event: RendererServiceWorkerLifecycle,
    ) {
        let mut state = self.state.lock();
        let stream = RendererOutputStreamIdentity::new_service_worker(
            self.worker_lifecycle.runtime(),
            version_id.as_u64(),
        );
        let journal = match self.transport.sender() {
            Some(transport) => RendererTurnOutputJournal::new_with_transport(stream, transport),
            None => RendererTurnOutputJournal::new(stream),
        };
        assert!(
            state.live.insert(version_id, journal.clone()).is_none(),
            "ServiceWorker output stream opened twice for one version"
        );
        drop(state);
        journal.publish_record(self.lifecycle_record(event));
    }

    pub(super) fn publish(
        &self,
        version_id: ServiceWorkerVersionId,
        event: RendererServiceWorkerLifecycle,
    ) {
        let journal = self
            .state
            .lock()
            .live
            .get(&version_id)
            .cloned()
            .expect("ServiceWorker target output requires a live version stream");
        journal.publish_record(self.lifecycle_record(event));
    }

    pub(super) fn journal(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> Option<RendererTurnOutputJournal> {
        self.state.lock().live.get(&version_id).cloned()
    }

    pub(super) fn publish_destroyed(
        &self,
        version_id: ServiceWorkerVersionId,
        event: RendererServiceWorkerLifecycle,
    ) {
        let mut state = self.state.lock();
        let journal = state
            .live
            .remove(&version_id)
            .expect("ServiceWorker target stream must exist until target destruction");
        journal.publish_record(self.lifecycle_record(event));
        journal.retire(RendererOutputStreamCloseReason::ResidenceRetired);
        if !journal.transport_is_bound() {
            state.retired_before_transport.push(journal);
        }
    }

    pub(super) fn publish_observation(
        &self,
        version_id: ServiceWorkerVersionId,
        event: RendererServiceWorkerObservation,
    ) {
        let Some(journal) = self.journal(version_id) else {
            return;
        };
        journal.publish_record(
            PendingRendererOutputRecord::observation(
                None,
                RendererProtocolObservation::ServiceWorker(event),
            )
            .resolve()
            .expect("ServiceWorker observation must have resolved source identity"),
        );
    }

    fn lifecycle_record(&self, event: RendererServiceWorkerLifecycle) -> RendererOutputRecord {
        PendingRendererOutputRecord::observation(
            None,
            RendererProtocolObservation::WorkerLifecycle(
                self.worker_lifecycle
                    .report(RendererWorkerLifecycle::Service(event)),
            ),
        )
        .resolve()
        .expect("ServiceWorker lifecycle has a concrete version stream")
    }
}

/// Producer tests inspect both record kinds without forging a Browser acknowledgement.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ServiceWorkerOutputForTest {
    Lifecycle(RendererServiceWorkerLifecycle),
    Observation(RendererServiceWorkerObservation),
}

#[cfg(test)]
pub(super) fn drain_service_worker_target_events_for_test(
    receiver: &mut crate::runtime::RendererOutputTransportReceiver,
) -> Vec<ServiceWorkerOutputForTest> {
    let mut events = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        let crate::runtime::RendererOutputTransportMessage::Publication(output) = message else {
            continue;
        };
        events.extend(
            output
                .records()
                .iter()
                .filter_map(|record| match record.item() {
                    crate::runtime::RendererOutputItem::Observation(
                        RendererProtocolObservation::WorkerLifecycle(observation),
                    ) => match observation.lifecycle() {
                        RendererWorkerLifecycle::Service(event) => {
                            Some(ServiceWorkerOutputForTest::Lifecycle(event.clone()))
                        }
                        _ => None,
                    },
                    crate::runtime::RendererOutputItem::Observation(
                        RendererProtocolObservation::ServiceWorker(event),
                    ) => Some(ServiceWorkerOutputForTest::Observation(event.clone())),
                    _ => None,
                }),
        );
    }
    events
}
