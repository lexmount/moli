use moli_shared_worker::SharedWorkerInstanceId;

use crate::worker::WorkerParentErrorEventKind;

use super::{host::RendererSharedWorkerHost, service::WeakSharedWorkerRuntimeService};

impl WeakSharedWorkerRuntimeService {
    pub(super) fn finish_worker_closed(&self, instance_id: SharedWorkerInstanceId) {
        // Remove the registry entry before publishing destroyed so a racing
        // constructor cannot observe a stale running instance. Loading-stage
        // closes had no created target yet, and service-side terminate may have
        // already removed running entries, so both paths are lifecycle no-ops.
        self.remove_host_for_instance(instance_id)
            .close_after_worker_closed();
    }

    pub(super) fn finish_worker_bootstrap_error(
        &self,
        instance_id: SharedWorkerInstanceId,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) {
        self.remove_host_for_instance(instance_id)
            .fail_after_worker_bootstrap_error(message, filename, lineno, colno, event_kind);
    }
}

impl RendererSharedWorkerHost {
    pub(super) fn fail_bootstrap(
        &self,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
    ) {
        let runtime_service = self.runtime_service();
        if !runtime_service.enqueue_service_lane_worker_bootstrap_error(
            self.instance_id(),
            message,
            filename,
            lineno,
            colno,
            event_kind,
        ) {
            return;
        }
        runtime_service.signal_service_lane_wake();
    }
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::{
        SharedWorkerConnectAction, SharedWorkerDescriptor, SharedWorkerInstanceId,
        SharedWorkerLoadReady,
    };

    use crate::{
        message_port_runtime::new_message_port_registry,
        runtime::{
            RendererOutputItem, RendererOutputStreamControl, RendererProtocolObservation,
            RendererWorkerLifecycle,
        },
        shared_worker_runtime::{
            SharedWorkerClientEndpointDisposition, SharedWorkerClientEvent,
            SharedWorkerRuntimeOwnerWake, test_support,
        },
        worker::WorkerParentErrorEventKind,
    };

    #[test]
    fn racing_terminal_paths_publish_one_destroyed_record_and_one_close() {
        let instance_id = SharedWorkerInstanceId::from_u64(10);
        let key = test_support::shared_worker_key();
        let host = test_support::loading_host(instance_id, &key);
        let (tx, mut rx) = crate::runtime::renderer_output_transport_channel();
        host.target_output().bind_transport(tx);

        host.publish_created_target_event();
        host.publish_destroyed_target_event();
        host.publish_destroyed_target_event();

        assert!(matches!(
            rx.try_recv()
                .expect("stream open must precede worker facts"),
            crate::runtime::RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { .. }
            )
        ));
        let crate::runtime::RendererOutputTransportMessage::Publication(created) =
            rx.try_recv().expect("created fact must be published")
        else {
            panic!("created fact must be concrete")
        };
        assert!(matches!(
            created.records()[0].item(),
            RendererOutputItem::Observation(RendererProtocolObservation::WorkerLifecycle(observation))
                if matches!(observation.lifecycle(), RendererWorkerLifecycle::SharedCreated(_))
        ));
        let crate::runtime::RendererOutputTransportMessage::Publication(destroyed) =
            rx.try_recv().expect("destroyed fact must be published")
        else {
            panic!("destroyed fact must be concrete")
        };
        assert!(matches!(
            destroyed.records()[0].item(),
            RendererOutputItem::Observation(RendererProtocolObservation::WorkerLifecycle(observation))
                if matches!(observation.lifecycle(), RendererWorkerLifecycle::SharedDestroyed(destroyed_instance_id) if *destroyed_instance_id == instance_id)
        ));
        assert!(matches!(
            rx.try_recv().expect(
                "terminal response fence must retain the destroyed cursor before stream close"
            ),
            crate::runtime::RendererOutputTransportMessage::CursorLeaseDeclared { .. }
        ));
        assert!(matches!(
            rx.try_recv()
                .expect("stream close must follow the terminal fence"),
            crate::runtime::RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Closed { .. }
            )
        ));
        assert!(
            rx.try_recv().is_err(),
            "the losing terminal path must publish neither a duplicate fact nor a duplicate close"
        );
        drop(host);
        assert!(matches!(
            rx.try_recv()
                .expect("dropping the terminal host must release its cursor lease"),
            crate::runtime::RendererOutputTransportMessage::CursorLeaseReleased { .. }
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn worker_closed_while_loading_does_not_emit_destroyed_without_created() {
        let service = test_support::runtime_service();
        let mut service_wake_rx = test_support::install_owner_wake_sender(&service);
        let message_port_registry = new_message_port_registry();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action = test_support::connect_matching(&service, key.clone(), descriptor.clone());
        let (instance_id, client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let host = test_support::loading_host_with_runtime_service(instance_id, &key, &service);
        host.add_client(
            client_id,
            message_port_owner.shared_worker_client(
                client_id,
                client_port_id,
                worker_port_id,
                message_port_registry,
            ),
        );
        test_support::store_loading_host(&service, instance_id, host.clone());

        host.notify_worker_closed();

        assert!(
            !test_support::matching_is_empty(&service),
            "worker close acknowledgement should not mutate the registry before service lane drain"
        );
        assert!(matches!(
            service_wake_rx.try_recv(),
            Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(service.drain_service_lane(), 1);

        assert!(
            host.target_output_retired()
                .load(std::sync::atomic::Ordering::Acquire),
            "startup close must retire the exact worker output stream"
        );
        assert!(test_support::matching_is_empty(&service));
        assert!(matches!(
            test_support::connect_matching(&service, key, descriptor),
            SharedWorkerConnectAction::StartLoading { .. }
        ));
        let task = message_port_owner
            .pop_shared_worker_client_event()
            .expect("startup close should still notify the pending client wrapper");
        assert_eq!(task.owner().client_id(), client_id);
        assert!(matches!(task.into_event(), SharedWorkerClientEvent::Closed));
        assert!(
            message_port_owner
                .pop_shared_worker_client_event()
                .is_none()
        );
    }

    #[test]
    fn bootstrap_error_is_queued_through_service_lane() {
        let service = test_support::runtime_service();
        let mut service_wake_rx = test_support::install_owner_wake_sender(&service);
        let message_port_registry = new_message_port_registry();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action = test_support::connect_matching(&service, key.clone(), descriptor);
        let (instance_id, client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let host = test_support::loading_host_with_runtime_service(instance_id, &key, &service);
        host.add_client(
            client_id,
            message_port_owner.shared_worker_client(
                client_id,
                client_port_id,
                worker_port_id,
                message_port_registry,
            ),
        );
        match test_support::finish_loading_matching(&service, &key, instance_id, host.clone()) {
            SharedWorkerLoadReady::Running { .. } => {}
            SharedWorkerLoadReady::Stale => panic!("expected loading slot to become running"),
        }

        host.fail_bootstrap(
            "bootstrap failed".to_owned(),
            "https://example.test/worker.js".to_owned(),
            3,
            4,
            WorkerParentErrorEventKind::Event,
        );

        assert!(
            !test_support::matching_is_empty(&service),
            "bootstrap failure should not mutate the registry before service lane drain"
        );
        assert!(matches!(
            service_wake_rx.try_recv(),
            Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(service.drain_service_lane(), 1);

        assert!(test_support::matching_is_empty(&service));
        let task = message_port_owner
            .pop_shared_worker_client_event()
            .expect("expected shared worker client error");
        assert_eq!(task.owner().client_id(), client_id);
        match task.into_event() {
            SharedWorkerClientEvent::Error(error) => {
                assert_eq!(error.message(), "bootstrap failed");
                assert_eq!(error.filename(), "https://example.test/worker.js");
                assert_eq!(error.lineno(), 3);
                assert_eq!(error.colno(), 4);
                assert_eq!(error.event_kind(), WorkerParentErrorEventKind::Event);
                assert_eq!(
                    error.endpoint_disposition(),
                    SharedWorkerClientEndpointDisposition::Retire
                );
            }
            other => panic!("expected shared worker client error, got {other:?}"),
        }
        assert!(
            message_port_owner
                .pop_shared_worker_client_event()
                .is_none()
        );
    }
}
