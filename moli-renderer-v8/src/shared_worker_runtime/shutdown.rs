use super::{host_removal::SharedWorkerRemovedHost, service::SharedWorkerRuntimeService};

impl SharedWorkerRuntimeService {
    pub(crate) fn terminate_all_for_context_shutdown(&self) {
        for removed in self.take_context_shutdown_hosts() {
            removed.terminate();
        }
    }

    fn take_context_shutdown_hosts(&self) -> Vec<SharedWorkerRemovedHost> {
        self.take_context_shutdown_hosts_from_stores()
    }
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::{
        SharedWorkerConnectAction, SharedWorkerDescriptor, SharedWorkerLoadReady,
    };

    use crate::{
        message_port_runtime::new_message_port_registry,
        shared_worker_runtime::{
            SharedWorkerClientEvent, service::SharedWorkerRuntimeService, test_support,
        },
    };

    fn runtime_service() -> SharedWorkerRuntimeService {
        test_support::runtime_service()
    }

    #[test]
    fn context_shutdown_cancels_loading_hosts_and_clears_registry() {
        let service = runtime_service();
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
        let host = test_support::loading_host(instance_id, &key);
        host.add_client(
            client_id,
            message_port_owner.shared_worker_client(
                client_id,
                client_port_id,
                worker_port_id,
                message_port_registry,
            ),
        );
        let cancel_handle = host.begin_loading_task();
        test_support::store_loading_host(&service, instance_id, host.clone());

        service.terminate_all_for_context_shutdown();

        assert!(cancel_handle.is_cancelled());
        assert!(host.is_closed());
        assert!(test_support::matching_is_empty(&service));
        assert!(test_support::loading_hosts_empty(&service));
        assert!(test_support::owner_lifecycle_is_empty(&service));

        let task = message_port_owner
            .pop_shared_worker_client_event()
            .expect("context shutdown must send a terminal close event to loading clients");
        assert_eq!(task.owner().client_id(), client_id);
        assert!(matches!(task.into_event(), SharedWorkerClientEvent::Closed));
        assert!(
            message_port_owner
                .pop_shared_worker_client_event()
                .is_none()
        );
    }

    #[test]
    fn context_shutdown_terminates_running_hosts_and_records_destroyed_lifecycle() {
        let service = runtime_service();
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

        service.terminate_all_for_context_shutdown();

        assert!(host.is_closed());
        assert!(test_support::matching_is_empty(&service));
        assert!(test_support::owner_lifecycle_is_empty(&service));
        assert!(
            host.target_output_retired()
                .load(std::sync::atomic::Ordering::Acquire),
            "context shutdown must retire the running worker's concrete output stream"
        );
    }
}
