use std::sync::Arc;
use std::sync::atomic::Ordering;

use tracing::trace;

use crate::{
    runtime::{
        PendingRendererOutputRecord, RendererProtocolObservation, RendererRuntimeInspectorMessage,
        RendererSharedWorkerConsoleMessage, RendererSharedWorkerObservation,
        RendererSharedWorkerTargetInfo, RendererWorkerLifecycle,
    },
    worker::{WorkerConsoleMessage, WorkerRuntimeInspectorMessageBatch},
};

use super::host::RendererSharedWorkerHost;
use super::host::SharedWorkerRuntimeResponsePublicationState;

impl RendererSharedWorkerHost {
    pub(super) fn publish_created_target_event(&self) {
        self.publish_lifecycle(RendererWorkerLifecycle::SharedCreated(self.target_info()));
    }

    pub(super) fn publish_destroyed_target_event(self: &Arc<Self>) {
        self.begin_runtime_response_retirement();
        if !self.claim_target_output_retirement() {
            return;
        }
        self.publish_lifecycle(RendererWorkerLifecycle::SharedDestroyed(self.instance_id()));
        self.finish_runtime_response_retirement();
        self.finish_target_output_retirement();
    }

    pub(super) fn retire_unstarted_output(&self) {
        self.begin_runtime_response_retirement();
        if self.claim_target_output_retirement() {
            self.finish_runtime_response_retirement();
            self.finish_target_output_retirement();
        }
    }

    /// Stops direct response publication before target retirement begins.
    ///
    /// SharedWorker Inspector responses and lifecycle records originate on
    /// different threads but enter the host through one worker-parent FIFO.
    /// Once that FIFO observes `SharedWorkerClosed`, responses must wait for
    /// the terminal worker-stream cursor. Otherwise a V8
    /// "execution context destroyed" reply can overtake target detachment,
    /// whereas Chromium closes the target/session and rejects the outstanding
    /// command as `Target closed`.
    pub(super) fn begin_runtime_response_retirement(&self) {
        let mut state = self.runtime_response_publications.lock();
        if matches!(*state, SharedWorkerRuntimeResponsePublicationState::Active) {
            *state = SharedWorkerRuntimeResponsePublicationState::Closing(Vec::new());
        }
    }

    pub(super) fn publish_runtime_inspector_response(
        &self,
        publication: crate::runtime::RendererRuntimeInspectorResponsePublication,
    ) {
        let mut state = self.runtime_response_publications.lock();
        match &mut *state {
            SharedWorkerRuntimeResponsePublicationState::Active => {
                let target_output = self.target_output();
                let predecessor = target_output
                    .last_published_cursor()
                    .map(|cursor| target_output.declare_fence(cursor));
                let _ = publication.commit(predecessor);
            }
            SharedWorkerRuntimeResponsePublicationState::Closing(pending) => {
                pending.push(publication);
            }
            SharedWorkerRuntimeResponsePublicationState::Retired {
                terminal_predecessor,
            } => {
                let _ = publication.commit(terminal_predecessor.clone());
            }
        }
    }

    fn finish_runtime_response_retirement(&self) {
        let target_output = self.target_output();
        let predecessor = target_output
            .last_published_cursor()
            .map(|cursor| target_output.declare_fence(cursor));
        let pending = {
            let mut state = self.runtime_response_publications.lock();
            match std::mem::replace(
                &mut *state,
                SharedWorkerRuntimeResponsePublicationState::Retired {
                    terminal_predecessor: predecessor.clone(),
                },
            ) {
                SharedWorkerRuntimeResponsePublicationState::Closing(pending) => pending,
                SharedWorkerRuntimeResponsePublicationState::Active => {
                    panic!("SharedWorker response retirement must begin before it finishes")
                }
                SharedWorkerRuntimeResponsePublicationState::Retired { .. } => {
                    panic!("SharedWorker response retirement finished twice")
                }
            }
        };
        for publication in pending {
            let _ = publication.commit(predecessor.clone());
        }
    }

    pub(super) fn record_runtime_inspector_messages_if_running(
        self: &Arc<Self>,
        batches: Vec<WorkerRuntimeInspectorMessageBatch>,
        script_url: &str,
    ) -> bool {
        let mut recorded = false;
        for batch in batches {
            let (responses, notifications): (Vec<_>, Vec<_>) = batch
                .messages
                .into_iter()
                .partition(|message| match message {
                    RendererRuntimeInspectorMessage::Protocol(message) => {
                        message.get("id").is_some()
                    }
                    RendererRuntimeInspectorMessage::RuntimeContext(_) => false,
                });
            for message in responses {
                trace!(
                    url = %script_url,
                    instance_id = self.instance_id().as_u64(),
                    inspector_session_id = ?batch.inspector_session_id,
                    message = ?message,
                    "dropping stale shared worker runtime inspector response without a deferred callback"
                );
            }
            if notifications.is_empty() {
                continue;
            }
            recorded |= self.publish_target_event_if_running(
                RendererSharedWorkerObservation::RuntimeInspectorMessages {
                    instance_id: self.instance_id(),
                    inspector_session_id: batch.inspector_session_id,
                    messages: notifications,
                },
                || {
                    trace!(
                        url = %script_url,
                        instance_id = self.instance_id().as_u64(),
                        "dropping late shared worker runtime inspector messages for non-running host"
                    );
                },
            );
        }
        recorded
    }

    pub(super) fn record_console_message_if_running(
        self: &Arc<Self>,
        console: &WorkerConsoleMessage,
    ) -> bool {
        self.publish_target_event_if_running(
            RendererSharedWorkerObservation::Console {
                instance_id: self.instance_id(),
                message: RendererSharedWorkerConsoleMessage {
                    message: console.message.clone(),
                    args: console.args.clone(),
                    stack: console.stack.clone(),
                },
            },
            || {},
        )
    }

    fn publish_target_event_if_running(
        self: &Arc<Self>,
        event: RendererSharedWorkerObservation,
        on_not_running: impl FnOnce(),
    ) -> bool {
        if !self.is_running_in_runtime_service() {
            on_not_running();
            return false;
        }
        self.publish_target_event(event);
        true
    }

    fn is_running_in_runtime_service(self: &Arc<Self>) -> bool {
        self.runtime_service().is_running_host(self)
    }

    fn target_info(&self) -> RendererSharedWorkerTargetInfo {
        RendererSharedWorkerTargetInfo {
            owner_local_host_id: self.owner_local_host_id(),
            instance_id: self.instance_id(),
            url: self.current_script_url(),
            name: self.worker_name(),
        }
    }

    fn publish_lifecycle(&self, lifecycle: RendererWorkerLifecycle) {
        self.publish_observation(RendererProtocolObservation::WorkerLifecycle(
            self.worker_lifecycle.report(lifecycle),
        ));
    }

    fn publish_target_event(&self, event: RendererSharedWorkerObservation) {
        self.publish_observation(RendererProtocolObservation::SharedWorker(event));
    }

    fn publish_observation(&self, observation: RendererProtocolObservation) {
        self.target_output().publish_record(
            PendingRendererOutputRecord::observation(None, observation)
                .resolve()
                .unwrap_or_else(|_| {
                    panic!("SharedWorker target output must have resolved source identity")
                }),
        );
    }

    /// Claims the one terminal transition before appending a terminal record.
    ///
    /// A worker-close acknowledgement can race a browser-side terminate. The
    /// claim must happen before `Destroyed` is appended; checking only while
    /// closing the stream would let the loser append to an already-closed
    /// journal.
    fn claim_target_output_retirement(&self) -> bool {
        !self.target_output_retired().swap(true, Ordering::AcqRel)
    }

    fn finish_target_output_retirement(&self) {
        if let Some(service) = self.runtime_service().upgrade() {
            service.retire_target_output_stream(self.instance_id());
        } else {
            self.target_output()
                .retire(crate::runtime::RendererOutputStreamCloseReason::ResidenceRetired);
        }
    }
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::SharedWorkerInstanceId;

    use crate::{
        runtime::{
            RendererOutputItem, RendererProtocolObservation, RendererRuntimeInspectorMessage,
            RendererWorkerLifecycle,
        },
        shared_worker_runtime::test_support,
        worker::{WorkerConsoleMessage, WorkerRuntimeInspectorMessageBatch},
    };

    #[test]
    fn non_running_host_drops_tail_target_output() {
        let key = test_support::shared_worker_key();
        let host = test_support::loading_host(SharedWorkerInstanceId::from_u64(41), &key);
        let console = WorkerConsoleMessage {
            message: "late-console".to_owned(),
            args: Vec::new(),
            stack: None,
        };

        assert!(!host.record_console_message_if_running(&console));
        assert!(!host.record_runtime_inspector_messages_if_running(
            vec![WorkerRuntimeInspectorMessageBatch {
                inspector_session_id: Some("SID-worker".to_owned()),
                messages: vec![RendererRuntimeInspectorMessage::protocol(
                    serde_json::json!({
                        "method": "Runtime.consoleAPICalled"
                    })
                )],
            }],
            key.script_url()
        ));
        assert_eq!(host.target_output().pending_len(), 0);
    }

    #[test]
    fn created_target_fact_is_frozen_until_transport_binding() {
        let runtime_service = test_support::runtime_service();
        let key = test_support::shared_worker_key();
        let instance_id = SharedWorkerInstanceId::from_u64(42);
        let host =
            test_support::loading_host_with_runtime_service(instance_id, &key, &runtime_service);

        host.publish_created_target_event();
        let (tx, mut rx) = crate::runtime::renderer_output_transport_channel();
        runtime_service.bind_target_output_transport(tx);

        assert!(matches!(
            rx.try_recv().expect("stream open should precede output"),
            crate::runtime::RendererOutputTransportMessage::StreamControl(
                crate::runtime::RendererOutputStreamControl::Opened { .. }
            )
        ));
        let crate::runtime::RendererOutputTransportMessage::Publication(publication) = rx
            .try_recv()
            .expect("pre-transport target fact should publish after open")
        else {
            panic!("target fact must use concrete output")
        };
        let [record] = publication.records() else {
            panic!("one created fact should produce one record")
        };
        assert!(matches!(
            record.item(),
            RendererOutputItem::Observation(RendererProtocolObservation::WorkerLifecycle(observation))
                if matches!(observation.lifecycle(), RendererWorkerLifecycle::SharedCreated(info) if info.instance_id == instance_id)
        ));
    }

    #[test]
    fn retirement_declares_terminal_response_fence_before_stream_close() {
        let runtime_service = test_support::runtime_service();
        let key = test_support::shared_worker_key();
        let instance_id = SharedWorkerInstanceId::from_u64(43);
        let host =
            test_support::loading_host_with_runtime_service(instance_id, &key, &runtime_service);
        let stream = host.target_output().stream();
        let (tx, mut rx) = crate::runtime::renderer_output_transport_channel();
        runtime_service.bind_target_output_transport(tx);

        host.publish_created_target_event();
        host.publish_destroyed_target_event();

        assert!(matches!(
            rx.try_recv().expect("stream open"),
            crate::runtime::RendererOutputTransportMessage::StreamControl(
                crate::runtime::RendererOutputStreamControl::Opened { stream: actual }
            ) if actual == stream
        ));
        for expected_sequence in [1, 2] {
            let crate::runtime::RendererOutputTransportMessage::Publication(publication) =
                rx.try_recv().expect("worker lifecycle publication")
            else {
                panic!("worker lifecycle fact must use concrete output")
            };
            assert_eq!(publication.cursor().sequence(), expected_sequence);
        }
        let lease_id = match rx.try_recv().expect("terminal cursor lease") {
            crate::runtime::RendererOutputTransportMessage::CursorLeaseDeclared {
                cursor,
                lease_id,
            } => {
                assert_eq!(cursor.stream(), stream);
                assert_eq!(cursor.sequence(), 2);
                lease_id
            }
            other => panic!("expected terminal cursor lease before close, got {other:?}"),
        };
        assert!(matches!(
            rx.try_recv().expect("stream close"),
            crate::runtime::RendererOutputTransportMessage::StreamControl(
                crate::runtime::RendererOutputStreamControl::Closed {
                    stream: actual,
                    last_published_sequence: Some(last),
                    ..
                }
            ) if actual == stream && last.get() == 2
        ));

        drop(host);
        assert_eq!(
            rx.try_recv().expect("terminal cursor lease release"),
            crate::runtime::RendererOutputTransportMessage::CursorLeaseReleased {
                stream,
                lease_id,
            }
        );
    }
}
