use std::collections::HashMap;

use parking_lot::Mutex;

use crate::runtime::{
    PendingRendererOutputRecord, RendererBrowserContextRuntimeId, RendererOutputRecord,
    RendererOutputStreamCloseReason, RendererOutputStreamIdentity,
    RendererOutputTransportSenderSlot, RendererOwnerAction, RendererServiceWorkerTargetEvent,
    RendererTurnOutputJournal,
};

use super::ids::ServiceWorkerVersionId;

/// Concrete protocol streams owned by stable ServiceWorker version targets.
///
/// A ServiceWorker may stop and restart many V8 runs while its version target
/// remains alive. Consequently the stream lifetime follows the version, not a
/// worker thread or run run: `Created` opens it, run/status events append
/// to it, and `Destroyed` is its final record before closure.
pub(super) struct ServiceWorkerTargetOutputStreams {
    browser_context_runtime_id: RendererBrowserContextRuntimeId,
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
        browser_context_runtime_id: RendererBrowserContextRuntimeId,
        transport: RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            browser_context_runtime_id,
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
        event: RendererServiceWorkerTargetEvent,
    ) {
        let mut state = self.state.lock();
        let stream = RendererOutputStreamIdentity::new_service_worker(
            self.browser_context_runtime_id,
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
        journal.publish_record(Self::record(event));
    }

    pub(super) fn publish(
        &self,
        version_id: ServiceWorkerVersionId,
        event: RendererServiceWorkerTargetEvent,
    ) {
        let journal = self
            .state
            .lock()
            .live
            .get(&version_id)
            .cloned()
            .expect("ServiceWorker target output requires a live version stream");
        journal.publish_record(Self::record(event));
    }

    pub(super) fn publish_destroyed(
        &self,
        version_id: ServiceWorkerVersionId,
        event: RendererServiceWorkerTargetEvent,
    ) {
        let mut state = self.state.lock();
        let journal = state
            .live
            .remove(&version_id)
            .expect("ServiceWorker target stream must exist until target destruction");
        journal.publish_record(Self::record(event));
        journal.retire(RendererOutputStreamCloseReason::ResidenceRetired);
        if !journal.transport_is_bound() {
            state.retired_before_transport.push(journal);
        }
    }

    fn record(event: RendererServiceWorkerTargetEvent) -> RendererOutputRecord {
        PendingRendererOutputRecord::owner_action(
            None,
            RendererOwnerAction::ServiceWorkerTargetLifecycle(event),
        )
        .resolve()
        .unwrap_or_else(|_| {
            panic!("ServiceWorker target output must have resolved source identity")
        })
    }
}

#[cfg(test)]
pub(super) fn drain_service_worker_target_events_for_test(
    receiver: &mut crate::runtime::RendererOutputTransportReceiver,
) -> Vec<RendererServiceWorkerTargetEvent> {
    let mut events = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        let crate::runtime::RendererOutputTransportMessage::Publication(output) = message else {
            continue;
        };
        events.extend(output.records().iter().filter_map(|record| {
            let crate::runtime::RendererOutputItem::OwnerAction(
                RendererOwnerAction::ServiceWorkerTargetLifecycle(event),
            ) = record.item()
            else {
                return None;
            };
            Some(event.clone())
        }));
    }
    events
}
