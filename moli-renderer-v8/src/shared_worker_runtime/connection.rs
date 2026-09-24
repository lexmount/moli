use std::sync::Arc;

use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerCompatibilityError, SharedWorkerConnectAction,
    SharedWorkerDescriptor, SharedWorkerInstanceId,
};

use crate::worker::WorkerParentErrorEventKind;

use super::{
    client::RendererSharedWorkerClient,
    host::{RendererSharedWorkerHost, SharedRendererSharedWorkerHost},
    loading::{
        SharedWorkerLaunchParams, SharedWorkerScriptLoadKind, load_shared_worker_local_script,
    },
    service::SharedWorkerRuntimeService,
};

impl SharedWorkerRuntimeService {
    pub(crate) fn connect(
        &self,
        descriptor: SharedWorkerDescriptor,
        params: SharedWorkerLaunchParams,
    ) -> SharedWorkerClientId {
        connect_with_runtime_service(self, descriptor, params)
    }
}

fn connect_with_runtime_service(
    runtime_service: &SharedWorkerRuntimeService,
    descriptor: SharedWorkerDescriptor,
    params: SharedWorkerLaunchParams,
) -> SharedWorkerClientId {
    let _admission = runtime_service.connection_admission();
    let action = runtime_service.connect_registry_entry(descriptor, &params);
    let client_id = match &action {
        SharedWorkerConnectAction::StartLoading { client_id, .. }
        | SharedWorkerConnectAction::QueueWhileLoading { client_id, .. }
        | SharedWorkerConnectAction::ConnectToRunning { client_id, .. }
        | SharedWorkerConnectAction::RejectClient { client_id, .. } => *client_id,
    };
    let client = RendererSharedWorkerClient {
        client_port_id: params.client_port_id,
        worker_port_id: params.worker_port_id,
        message_port_registry: params
            .launch_context
            .execution_policy
            .worker_context_runtime
            .message_port_registry(),
        client_event_producer: params.client_event_realm.bind_client(client_id),
        worker_host_bridge_sender: params.worker_host_bridge_sender.clone(),
    };
    match action {
        SharedWorkerConnectAction::StartLoading {
            instance_id,
            client_id,
        } => {
            runtime_service.start_loading(instance_id, params, client_id, client);
            client_id
        }
        SharedWorkerConnectAction::QueueWhileLoading {
            instance_id,
            client_id,
        } => {
            if let Some(host) = runtime_service.loading_host_for_connect(instance_id) {
                host.add_client(client_id, client);
            } else {
                client.send_error(
                    "Failed to connect SharedWorker: loading host is unavailable.",
                    params.key.script_url(),
                    WorkerParentErrorEventKind::Event,
                );
                client.close_ports();
                runtime_service.remove_client(client_id);
            }
            client_id
        }
        SharedWorkerConnectAction::ConnectToRunning {
            client_id,
            instance: host,
            ..
        } => {
            if !host.add_and_connect_client(client_id, client, params.key.script_url()) {
                runtime_service.remove_client(client_id);
            }
            client_id
        }
        SharedWorkerConnectAction::RejectClient { client_id, error } => {
            client.send_error(
                shared_worker_compatibility_error_message(&error),
                params.key.script_url(),
                WorkerParentErrorEventKind::Event,
            );
            client.close_ports();
            client_id
        }
    }
}

impl SharedWorkerRuntimeService {
    fn connect_registry_entry(
        &self,
        descriptor: SharedWorkerDescriptor,
        params: &SharedWorkerLaunchParams,
    ) -> SharedWorkerConnectAction<SharedRendererSharedWorkerHost> {
        self.connect_matching(params.key.clone(), descriptor)
    }

    fn loading_host_for_connect(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.loading_host(instance_id)
    }

    fn store_loading_host_for_connect(
        &self,
        instance_id: SharedWorkerInstanceId,
        host: SharedRendererSharedWorkerHost,
    ) {
        self.insert_loading_host(instance_id, host);
    }

    fn start_loading(
        &self,
        instance_id: SharedWorkerInstanceId,
        mut params: SharedWorkerLaunchParams,
        client_id: SharedWorkerClientId,
        client: RendererSharedWorkerClient,
    ) {
        params.reserve_service_worker_worker_client_for_main_script();
        let target_output = self.open_target_output_stream(instance_id);
        let host = Arc::new(RendererSharedWorkerHost::new_loading(
            params
                .launch_context
                .execution_policy
                .worker_context_runtime
                .network_for_worker(crate::runtime::RendererWorkerIdentity::Shared(instance_id)),
            self.required_owner_local_host_id(),
            self.downgrade(),
            params.key.script_url().to_owned(),
            params.launch_context.name.clone(),
            target_output,
            self.worker_lifecycle(),
        ));
        host.add_client(client_id, client);
        self.store_loading_host_for_connect(instance_id, host.clone());
        host.publish_created_target_event();
        let script_url = url::Url::parse(params.key.script_url())
            .expect("SharedWorker key has a resolved script URL");
        let Some(network) = crate::network::ResourceTransfer::start_main_script(
            &host.network,
            host.network_observer(),
            &script_url,
            &params
                .launch_context
                .execution_policy
                .module_static_import_initiator_url,
        ) else {
            params.unregister_reserved_service_worker_client();
            return;
        };
        let result = match params.script_load.clone().into_kind() {
            SharedWorkerScriptLoadKind::Local { script_url } => {
                load_shared_worker_local_script(&script_url, &network)
            }
            SharedWorkerScriptLoadKind::Failure { message } => Err(message),
            SharedWorkerScriptLoadKind::Fetch(fetch) => {
                host.start_script_fetch(params, *fetch, network);
                return;
            }
        };
        if let Err(message) = &result {
            network.failed(&crate::network::ResourceResponseFailure::Request(
                message.clone(),
            ));
        }
        host.enqueue_loading_completion(params, result);
    }
}

fn shared_worker_compatibility_error_message(error: &SharedWorkerCompatibilityError) -> String {
    format!("Failed to connect to SharedWorker: {error}.")
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::{SharedWorkerDescriptor, SharedWorkerKey};
    use url::Url;

    use crate::{
        message_port_runtime::SharedMessagePortRegistry,
        runtime::{
            RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner,
            RendererWorkerContextRuntime,
        },
        shared_worker_runtime::{
            loading::{
                SharedWorkerExecutionPolicy, SharedWorkerLaunchContext, SharedWorkerScriptLoad,
            },
            test_support,
        },
        worker::{WorkerNetworkPolicy, WorkerScriptKind},
    };

    use super::*;

    fn test_launch_context(
        browser_context_runtime: &RendererBrowserContextRuntimeOwner,
        worker_context_runtime: RendererWorkerContextRuntime,
    ) -> SharedWorkerLaunchContext {
        SharedWorkerLaunchContext::new(
            "loader".to_owned(),
            crate::network::ResourceRequestClient::from_browser_resource_runtime(
                browser_context_runtime.browser_resource_runtime(),
            ),
            SharedWorkerExecutionPolicy::new(
                WorkerScriptKind::Classic,
                Url::parse("https://example.test/page.html").unwrap(),
                Vec::new(),
                WorkerNetworkPolicy::default(),
                Default::default(),
                worker_context_runtime,
                Some("https://example.test".to_owned()),
                moli_fetch::RequestCredentialsMode::SameOrigin,
            ),
        )
    }

    fn test_launch_params(
        browser_context_runtime: &RendererBrowserContextRuntimeOwner,
        key: SharedWorkerKey,
        message_port_registry: &SharedMessagePortRegistry,
        message_port_owner: &test_support::SharedWorkerPageClientHarness,
        worker_context_runtime: RendererWorkerContextRuntime,
    ) -> SharedWorkerLaunchParams {
        let (client_port_id, worker_port_id) =
            message_port_registry.create_entangled_message_port_pair(message_port_owner.owner());
        SharedWorkerLaunchParams {
            key,
            script_load: SharedWorkerScriptLoad::failure("not used".to_owned()),
            launch_context: test_launch_context(browser_context_runtime, worker_context_runtime),
            client_port_id,
            worker_port_id,
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        }
    }

    #[tokio::test]
    async fn runtime_connect_and_remove_update_registry_clients() {
        let service = test_support::runtime_service();
        let message_port_owner = test_support::SharedWorkerPageClientHarness::new();
        let message_port_registry = crate::message_port_runtime::new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            service.clone(),
            crate::network::RendererResourceTaskRunner::from_current_tokio().unwrap(),
        );
        let key = test_support::shared_worker_key();
        let params = test_launch_params(
            &browser_context_runtime,
            key,
            &message_port_registry,
            &message_port_owner,
            browser_context_runtime.worker_context_runtime(),
        );

        let client_id = service.connect(SharedWorkerDescriptor::default(), params);
        let instance_id = SharedWorkerInstanceId::from_u64(1);
        assert_eq!(
            test_support::matching_clients_for_instance(&service, instance_id),
            vec![client_id]
        );

        service.remove_client(client_id);

        assert!(test_support::matching_is_empty(&service));
        drop(browser_context_runtime);
    }

    #[tokio::test]
    async fn registry_keeps_peer_client_until_its_own_removal() {
        let service = test_support::runtime_service();
        let message_port_owner = test_support::SharedWorkerPageClientHarness::new();
        let message_port_registry = crate::message_port_runtime::new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            service.clone(),
            crate::network::RendererResourceTaskRunner::from_current_tokio().unwrap(),
        );
        let key = test_support::shared_worker_key();
        let first = test_launch_params(
            &browser_context_runtime,
            key.clone(),
            &message_port_registry,
            &message_port_owner,
            browser_context_runtime.worker_context_runtime(),
        );
        let second = test_launch_params(
            &browser_context_runtime,
            key,
            &message_port_registry,
            &message_port_owner,
            browser_context_runtime.worker_context_runtime(),
        );

        let first_client_id = service.connect(SharedWorkerDescriptor::default(), first);
        let second_client_id = service.connect(SharedWorkerDescriptor::default(), second);
        let instance_id = SharedWorkerInstanceId::from_u64(1);
        assert_eq!(
            test_support::matching_clients_for_instance(&service, instance_id),
            vec![first_client_id, second_client_id]
        );

        service.remove_client(first_client_id);
        assert_eq!(
            test_support::matching_clients_for_instance(&service, instance_id),
            vec![second_client_id],
            "removing one of two clients must preserve the other connection"
        );

        service.remove_client(second_client_id);
        assert!(test_support::matching_is_empty(&service));
        drop(browser_context_runtime);
    }
}
