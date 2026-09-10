use std::collections::HashMap;

use parking_lot::Mutex;

use crate::runtime::{
    RendererOutputStreamCloseReason, RendererOutputStreamIdentity,
    RendererOutputTransportSenderSlot, RendererTurnOutputJournal, RendererWorkerIdentity,
};

/// Browser-context-owned registry of exact Worker output streams.
///
/// A host owns the producer handle while this registry owns transport binding
/// and retirement. Keeping those responsibilities here lets a worker produce
/// concrete facts before CDP installs its channel without falling back to a
/// service-wide lifecycle queue.
#[derive(Debug)]
pub(crate) struct RendererWorkerOutputStreams {
    pub(crate) worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
    transport: RendererOutputTransportSenderSlot,
    state: Mutex<RendererWorkerOutputStreamsState>,
}

#[derive(Debug, Default)]
struct RendererWorkerOutputStreamsState {
    live: HashMap<RendererWorkerIdentity, RendererTurnOutputJournal>,
    /// A short-lived worker can finish before CDP installs the BrowserContext
    /// transport. The registry, rather than an incidental host clone, owns its
    /// frozen Created/Destroyed prefix through the first transport binding.
    retired_before_transport: Vec<RendererTurnOutputJournal>,
}

impl RendererWorkerOutputStreams {
    pub(crate) fn new(
        worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
        transport: RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            worker_lifecycle,
            transport,
            state: Mutex::new(RendererWorkerOutputStreamsState::default()),
        }
    }

    pub(crate) fn open(&self, worker: RendererWorkerIdentity) -> RendererTurnOutputJournal {
        let mut state = self.state.lock();
        let runtime = self.worker_lifecycle.runtime();
        let stream = match &worker {
            RendererWorkerIdentity::Dedicated(instance) => {
                RendererOutputStreamIdentity::new_dedicated_worker(runtime, *instance)
            }
            RendererWorkerIdentity::Shared(instance) => {
                RendererOutputStreamIdentity::new_shared_worker(runtime, instance.as_u64())
            }
            RendererWorkerIdentity::Service { version, .. } => {
                RendererOutputStreamIdentity::new_service_worker(runtime, *version)
            }
        };
        let journal = match self.transport.sender() {
            Some(transport) => RendererTurnOutputJournal::new_with_transport(stream, transport),
            None => RendererTurnOutputJournal::new(stream),
        };
        assert!(
            state.live.insert(worker, journal.clone()).is_none(),
            "Worker output stream opened twice for one physical execution"
        );
        journal
    }

    pub(crate) fn bind_transport(&self, transport: crate::runtime::RendererOutputTransportSender) {
        // Serialize transport installation with retirement. This prevents the
        // race where bind observes the live map just before retire removes a
        // journal, while retire still observes an empty transport slot.
        let mut state = self.state.lock();
        self.transport.set(transport.clone());
        for journal in state.live.values() {
            journal.bind_transport(transport.clone());
        }
        for journal in state.retired_before_transport.drain(..) {
            journal.bind_transport(transport.clone());
        }
    }

    pub(crate) fn retire(&self, worker: &RendererWorkerIdentity) {
        let mut state = self.state.lock();
        let journal = state
            .live
            .remove(worker)
            .expect("Worker output stream must exist until host retirement");
        journal.retire(RendererOutputStreamCloseReason::ResidenceRetired);
        if !journal.transport_is_bound() {
            state.retired_before_transport.push(journal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RendererBrowserContextRuntimeId;
    use crate::runtime::{
        PendingRendererOutputRecord, RendererOutputItem, RendererOutputStreamControl,
        RendererOutputTransportMessage, RendererProtocolObservation, RendererWorkerLifecycle,
    };
    use moli_shared_worker::SharedWorkerInstanceId;

    #[test]
    fn retired_pre_transport_stream_is_delivered_once_when_transport_binds() {
        let transport_slot = RendererOutputTransportSenderSlot::default();
        let streams = RendererWorkerOutputStreams::new(
            crate::runtime::RendererWorkerLifecycleReporter::new(
                RendererBrowserContextRuntimeId::new_for_testing(31),
            ),
            transport_slot.clone(),
        );
        let instance_id = SharedWorkerInstanceId::from_u64(7);
        let worker = RendererWorkerIdentity::Shared(instance_id);
        let journal = streams.open(worker.clone());
        let stream = journal.stream();
        journal.publish_record(
            PendingRendererOutputRecord::observation(
                None,
                RendererProtocolObservation::WorkerLifecycle(
                    streams
                        .worker_lifecycle
                        .report(RendererWorkerLifecycle::SharedDestroyed(instance_id)),
                ),
            )
            .resolve()
            .expect("test worker record should resolve"),
        );
        drop(journal);
        let (sender, mut receiver) = crate::runtime::renderer_output_transport_channel();
        // BrowserContext installs the shared slot immediately before it asks
        // each Worker registry to bind. Exercise retirement in that narrow
        // interval: slot presence alone must not be mistaken for a journal
        // which already delivered its stream.
        transport_slot.set(sender.clone());
        streams.retire(&worker);

        streams.bind_transport(sender.clone());
        assert_eq!(
            receiver.try_recv().expect("stream open"),
            RendererOutputTransportMessage::StreamControl(RendererOutputStreamControl::Opened {
                stream,
            })
        );
        let RendererOutputTransportMessage::Publication(publication) =
            receiver.try_recv().expect("terminal publication")
        else {
            panic!("retained worker output must remain a concrete publication");
        };
        assert!(matches!(
            publication.records(),
            [record]
                if matches!(
                    record.item(),
                    RendererOutputItem::Observation(RendererProtocolObservation::WorkerLifecycle(observation))
                        if matches!(observation.lifecycle(), RendererWorkerLifecycle::SharedDestroyed(actual) if *actual == instance_id)
                )
        ));
        assert_eq!(
            receiver.try_recv().expect("stream close"),
            RendererOutputTransportMessage::StreamControl(RendererOutputStreamControl::Closed {
                stream,
                last_published_sequence: std::num::NonZeroU64::new(1),
                reason: RendererOutputStreamCloseReason::ResidenceRetired,
            })
        );

        streams.bind_transport(sender);
        assert!(
            receiver.try_recv().is_err(),
            "a terminal journal must not replay Opened on repeated binding"
        );
    }
}
