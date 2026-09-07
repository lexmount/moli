use moli_shared_worker::{SharedWorkerClientId, SharedWorkerClientRemoval, SharedWorkerInstanceId};

use super::{host::SharedRendererSharedWorkerHost, service::SharedWorkerRuntimeService};

impl SharedWorkerRuntimeService {
    pub(crate) fn remove_client(&self, client_id: SharedWorkerClientId) {
        match self.remove_client_from_registry(client_id) {
            SharedWorkerClientRemoval::RemovedFromLoading { instance_id } => {
                if let Some(host) = self.loading_host_for_client_removal(instance_id) {
                    host.remove_client_endpoint(client_id);
                }
            }
            SharedWorkerClientRemoval::RemovedFromRunning { instance: host, .. } => {
                host.remove_client_endpoint(client_id);
            }
            SharedWorkerClientRemoval::Terminate { instance: host, .. } => {
                host.remove_client_endpoint(client_id);
                host.publish_destroyed_target_event();
                host.terminate();
            }
            SharedWorkerClientRemoval::CancelLoading { instance_id, .. } => {
                if let Some(host) = self.take_loading_host_for_client_removal(instance_id) {
                    host.remove_client_endpoint(client_id);
                    host.cancel_loading();
                }
            }
            SharedWorkerClientRemoval::Missing => {}
        }
    }

    fn remove_client_from_registry(
        &self,
        client_id: SharedWorkerClientId,
    ) -> SharedWorkerClientRemoval<SharedRendererSharedWorkerHost> {
        self.remove_matching_client(client_id)
    }

    fn loading_host_for_client_removal(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.loading_host(instance_id)
    }

    fn take_loading_host_for_client_removal(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.remove_loading_host(instance_id)
    }
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::{
        SharedWorkerClientId, SharedWorkerConnectAction, SharedWorkerDescriptor,
        SharedWorkerLoadReady,
    };

    use crate::{
        message_port_runtime::{SharedMessagePortRegistry, new_message_port_registry},
        shared_worker_runtime::{
            client::RendererSharedWorkerClient, service::SharedWorkerRuntimeService, test_support,
        },
    };

    fn add_test_client(
        host: &crate::shared_worker_runtime::host::RendererSharedWorkerHost,
        client_id: SharedWorkerClientId,
        message_port_registry: &SharedMessagePortRegistry,
        message_port_owner: &test_support::SharedWorkerPageClientHarness,
    ) {
        let (client_port_id, worker_port_id) =
            message_port_registry.create_entangled_message_port_pair(message_port_owner.owner());
        host.add_client(
            client_id,
            RendererSharedWorkerClient {
                client_port_id,
                worker_port_id,
                message_port_registry: message_port_registry.clone(),
                client_event_producer: message_port_owner
                    .shared_worker_client_event_producer(client_id),
                worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            },
        );
    }

    fn runtime_service() -> SharedWorkerRuntimeService {
        test_support::runtime_service()
    }

    #[test]
    fn removing_last_loading_client_cancels_pending_fetch() {
        let service = runtime_service();
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
        let cancel_handle = host.begin_loading_task();
        test_support::store_loading_host(&service, instance_id, host);

        service.remove_client(client_id);

        assert!(cancel_handle.is_cancelled());
        assert!(test_support::stored_loading_host(&service, instance_id).is_none());
    }

    #[test]
    fn removing_non_last_loading_client_only_removes_that_endpoint() {
        let service = runtime_service();
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action = test_support::connect_matching(&service, key.clone(), descriptor.clone());
        let (instance_id, first_client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let second_client_id =
            match test_support::connect_matching(&service, key.clone(), descriptor) {
                SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
                _ => panic!("expected QueueWhileLoading"),
            };
        let message_port_owner = test_support::SharedWorkerPageClientHarness::new();
        let message_port_registry = new_message_port_registry();
        let host = test_support::loading_host(instance_id, &key);
        add_test_client(
            &host,
            first_client_id,
            &message_port_registry,
            &message_port_owner,
        );
        add_test_client(
            &host,
            second_client_id,
            &message_port_registry,
            &message_port_owner,
        );
        let cancel_handle = host.begin_loading_task();
        test_support::store_loading_host(&service, instance_id, host.clone());

        service.remove_client(first_client_id);

        assert!(
            !cancel_handle.is_cancelled(),
            "non-last loading client removal must leave the loading task active"
        );
        assert!(test_support::stored_loading_host(&service, instance_id).is_some());
        assert_eq!(host.client_endpoint_count(), 1);
        assert_eq!(
            test_support::matching_clients_for_instance(&service, instance_id),
            vec![second_client_id]
        );
    }

    #[test]
    fn removing_non_last_running_client_uses_registry_returned_host() {
        let service = runtime_service();
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action = test_support::connect_matching(&service, key.clone(), descriptor.clone());
        let (instance_id, first_client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let second_client_id =
            match test_support::connect_matching(&service, key.clone(), descriptor) {
                SharedWorkerConnectAction::QueueWhileLoading { client_id, .. } => client_id,
                _ => panic!("expected QueueWhileLoading"),
            };
        let message_port_owner = test_support::SharedWorkerPageClientHarness::new();
        let message_port_registry = new_message_port_registry();
        let host = test_support::loading_host(instance_id, &key);
        add_test_client(
            &host,
            first_client_id,
            &message_port_registry,
            &message_port_owner,
        );
        add_test_client(
            &host,
            second_client_id,
            &message_port_registry,
            &message_port_owner,
        );
        assert!(matches!(
            test_support::finish_loading_matching(&service, &key, instance_id, host.clone()),
            SharedWorkerLoadReady::Running { .. }
        ));

        service.remove_client(first_client_id);

        assert_eq!(host.client_endpoint_count(), 1);
        assert_eq!(
            test_support::matching_clients_for_instance(&service, instance_id),
            vec![second_client_id]
        );
    }
}
