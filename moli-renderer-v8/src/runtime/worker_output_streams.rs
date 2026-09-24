use std::collections::HashMap;

use parking_lot::Mutex;

use crate::runtime::{
    RendererOutputStreamCloseReason, RendererOutputStreamIdentity,
    RendererOutputTransportSenderSlot, RendererTurnOutputJournal, RendererWorkerIdentity,
};

/// Browser-context-owned registry of exact Worker output streams.
///
/// A host owns the producer handle while this registry owns transport binding
/// and retirement. Unobserved executions keep only their live journal; Browser
/// owns the current native state used when observation is first bound.
#[derive(Debug)]
pub(crate) struct RendererWorkerOutputStreams {
    pub(crate) worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
    transport: RendererOutputTransportSenderSlot,
    state: Mutex<RendererWorkerOutputStreamsState>,
}

#[derive(Debug, Default)]
struct RendererWorkerOutputStreamsState {
    live: HashMap<RendererWorkerIdentity, RendererTurnOutputJournal>,
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
        let state = self.state.lock();
        self.transport.set(transport.clone());
        for journal in state.live.values() {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        RendererBrowserContextRuntimeId, RendererOutputStreamControl,
        RendererOutputTransportMessage,
    };
    use moli_shared_worker::SharedWorkerInstanceId;

    #[test]
    fn replacement_observer_binds_only_surviving_dedicated_and_shared_workers() {
        for worker in [
            RendererWorkerIdentity::Dedicated(1),
            RendererWorkerIdentity::Shared(SharedWorkerInstanceId::from_u64(1)),
        ] {
            let streams = RendererWorkerOutputStreams::new(
                crate::runtime::RendererWorkerLifecycleReporter::new(
                    RendererBrowserContextRuntimeId::new_for_testing(31),
                ),
                RendererOutputTransportSenderSlot::default(),
            );
            let (sender, receiver) = crate::runtime::renderer_output_transport_channel();
            streams.bind_transport(sender);
            let live = streams.open(worker.clone());
            let record = crate::runtime::RendererOutputRecord::new_for_test(
                crate::runtime::RendererOutputItem::Observation(
                    crate::runtime::RendererProtocolObservation::RuntimeLifecycleError {
                        text: "live worker".into(),
                        execution_context_id: None,
                    },
                ),
            );
            live.publish_record(record.clone());
            let retired = RendererWorkerIdentity::Dedicated(2);
            streams.open(retired.clone());
            drop(receiver);
            streams.retire(&retired);
            live.publish_record(record.clone());
            let (sender, mut receiver) = crate::runtime::renderer_output_transport_channel();
            streams.bind_transport(sender.clone());
            assert!(
                matches!(receiver.try_recv().unwrap(), RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream, first_sequence })
                if stream == live.stream() && first_sequence.get() == 3)
            );
            streams.bind_transport(sender);
            assert!(
                receiver.try_recv().is_err(),
                "retired workers and old output cannot replay"
            );
            live.publish_record(record.clone());
            assert!(
                matches!(receiver.try_recv().unwrap(), RendererOutputTransportMessage::Publication(publication)
                if publication.cursor().stream() == live.stream() && publication.cursor().sequence() == 3
                && publication.records() == [record])
            );
            streams.retire(&worker);
            assert!(
                matches!(receiver.try_recv().unwrap(), RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Closed { stream, .. }) if stream == live.stream())
            );
        }
    }

    #[test]
    fn retired_unobserved_workers_release_their_streams_before_transport_binding() {
        let transport_slot = RendererOutputTransportSenderSlot::default();
        let streams = RendererWorkerOutputStreams::new(
            crate::runtime::RendererWorkerLifecycleReporter::new(
                RendererBrowserContextRuntimeId::new_for_testing(31),
            ),
            transport_slot.clone(),
        );
        for instance in 1..=4096 {
            let worker = RendererWorkerIdentity::Shared(SharedWorkerInstanceId::from_u64(instance));
            streams.open(worker.clone());
            streams.retire(&worker);
        }
        let worker = RendererWorkerIdentity::Shared(SharedWorkerInstanceId::from_u64(4097));
        let journal = streams.open(worker.clone());
        let (sender, mut receiver) = crate::runtime::renderer_output_transport_channel();
        // Installation and registry binding are separate steps. Retirement in
        // between must not retain an already unobserved execution.
        transport_slot.set(sender.clone());
        streams.retire(&worker);
        streams.bind_transport(sender.clone());
        assert!(
            receiver.try_recv().is_err(),
            "retired unobserved streams must not flood or close a late transport"
        );
        let live = streams.open(RendererWorkerIdentity::Shared(
            SharedWorkerInstanceId::from_u64(4098),
        ));
        assert_ne!(live.stream(), journal.stream());
        assert_eq!(
            receiver.try_recv().unwrap(),
            RendererOutputTransportMessage::StreamControl(RendererOutputStreamControl::Opened {
                stream: live.stream(),
                first_sequence: std::num::NonZeroU64::MIN,
            })
        );
        streams.bind_transport(sender);
        assert!(
            receiver.try_recv().is_err(),
            "repeated binding cannot replay streams"
        );
    }
}
