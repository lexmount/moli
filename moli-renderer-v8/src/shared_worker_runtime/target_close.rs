use moli_shared_worker::SharedWorkerInstanceId;

use super::service::SharedWorkerRuntimeService;

impl SharedWorkerRuntimeService {
    pub(crate) fn close_instance(&self, instance_id: SharedWorkerInstanceId) -> bool {
        self.remove_host_for_instance(instance_id).terminate()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc as std_mpsc},
        time::Duration,
    };

    use moli_shared_worker::{
        SharedWorkerConnectAction, SharedWorkerDescriptor, SharedWorkerInstanceId,
        SharedWorkerLoadReady,
    };

    use crate::{
        message_port_runtime::new_message_port_registry,
        shared_worker_runtime::{
            SharedWorkerClientEvent, host::RendererSharedWorkerHostState, test_support,
        },
        worker::WorkerHandle,
    };

    #[test]
    fn explicit_close_removes_running_host_and_publishes_one_native_destruction() {
        let service = test_support::runtime_service();
        let message_port_registry = new_message_port_registry();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let action = test_support::connect_matching(
            &service,
            key.clone(),
            SharedWorkerDescriptor::default(),
        );
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
        assert!(matches!(
            test_support::finish_loading_matching(&service, &key, instance_id, host.clone()),
            SharedWorkerLoadReady::Running { .. }
        ));
        assert!(!host.is_closed());
        let (sender, mut output) = crate::runtime::renderer_output_transport_channel();
        service.bind_target_output_transport(sender);
        host.publish_created_target_event();

        assert!(service.close_instance(instance_id));
        assert!(!service.close_instance(instance_id));
        let mut destructions = 0;
        while let Ok(message) = output.try_recv() {
            if let crate::runtime::RendererOutputTransportMessage::Publication(publication) =
                message
            {
                for record in publication.records() {
                    if let crate::runtime::RendererOutputItem::Observation(
                        crate::runtime::RendererProtocolObservation::WorkerLifecycle(observation),
                    ) = record.item()
                        && matches!(observation.lifecycle(), crate::runtime::RendererWorkerLifecycle::SharedDestroyed(actual) if *actual == instance_id)
                    {
                        destructions += 1;
                    }
                }
            }
        }
        assert_eq!(destructions, 1);

        assert!(host.is_closed());
        assert!(test_support::matching_is_empty(&service));
        assert!(
            host.target_output_retired()
                .load(std::sync::atomic::Ordering::Acquire),
            "explicit close must retire the worker stream after one native destruction"
        );
        let task = message_port_owner
            .pop_shared_worker_client_event()
            .expect("expected shared worker client close");
        assert_eq!(task.owner().client_id(), client_id);
        assert!(matches!(task.into_event(), SharedWorkerClientEvent::Closed));
        assert!(
            message_port_owner
                .pop_shared_worker_client_event()
                .is_none()
        );
    }

    #[test]
    fn devtools_target_close_does_not_join_running_worker_thread() {
        let service = test_support::runtime_service();
        let key = test_support::shared_worker_key();
        let action = test_support::connect_matching(
            &service,
            key.clone(),
            SharedWorkerDescriptor::default(),
        );
        let instance_id = match action {
            SharedWorkerConnectAction::StartLoading { instance_id, .. } => instance_id,
            _ => panic!("expected StartLoading"),
        };
        let host = test_support::loading_host_with_runtime_service(instance_id, &key, &service);
        let (worker_tx, _worker_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_parent_tx, parent_rx) = tokio::sync::mpsc::unbounded_channel();
        let (release_worker_tx, release_worker_rx) = std_mpsc::channel();
        let worker_join = std::thread::spawn(move || {
            let _ = release_worker_rx.recv();
        });
        let handle = WorkerHandle::new(
            worker_tx,
            parent_rx,
            worker_join,
            Arc::new(parking_lot::Mutex::new(None)),
        );
        {
            let mut state = host.state.lock();
            *state = RendererSharedWorkerHostState::Running {
                tx: handle.tx.clone(),
                handle: Some(handle),
                parent_rx: None,
            };
        }
        assert!(matches!(
            test_support::finish_loading_matching(&service, &key, instance_id, host),
            SharedWorkerLoadReady::Running { .. }
        ));

        let (closed_tx, closed_rx) = std_mpsc::channel();
        let close_service = service.clone();
        let close_thread = std::thread::spawn(move || {
            let closed = close_service.close_instance(instance_id);
            let _ = closed_tx.send(closed);
        });
        let close_result = closed_rx.recv_timeout(Duration::from_millis(250));
        release_worker_tx
            .send(())
            .expect("fake worker should still be releasable");
        close_thread
            .join()
            .expect("closeTarget runtime close thread should finish");

        assert_eq!(
            close_result,
            Ok(true),
            "Target.closeTarget runtime close must not synchronously join a running worker thread"
        );
        assert!(test_support::matching_is_empty(&service));
    }

    #[test]
    fn devtools_target_close_reports_missing_instance() {
        let service = test_support::runtime_service();

        assert!(!service.close_instance(SharedWorkerInstanceId::from_u64(404)));
    }
}
