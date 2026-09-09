use std::fmt;

use moli_shared_worker::{
    SharedWorkerClientId, SharedWorkerInstanceId, SharedWorkerInstanceRemoval,
    SharedWorkerLoadFailure, SharedWorkerLoadReady,
};

use crate::worker::WorkerParentErrorEventKind;

use super::{
    host::{RendererSharedWorkerHost, SharedRendererSharedWorkerHost},
    loading::{SharedWorkerLaunchParams, SharedWorkerLoadedScript},
    service::{SharedWorkerRuntimeService, WeakSharedWorkerRuntimeService},
};

impl WeakSharedWorkerRuntimeService {
    pub(super) fn finish_loading(
        &self,
        instance_id: SharedWorkerInstanceId,
        mut params: SharedWorkerLaunchParams,
        result: Result<SharedWorkerLoadedScript, String>,
    ) {
        let Some(service) = self.upgrade() else {
            params.unregister_reserved_service_worker_client();
            return;
        };
        service.finish_loading(instance_id, params, result);
    }
}

impl SharedWorkerRuntimeService {
    pub(super) fn finish_loading(
        &self,
        instance_id: SharedWorkerInstanceId,
        params: SharedWorkerLaunchParams,
        result: Result<SharedWorkerLoadedScript, String>,
    ) {
        finish_loading_with_runtime_service(self, instance_id, params, result);
    }
}

fn finish_loading_with_runtime_service(
    runtime_service: &SharedWorkerRuntimeService,
    instance_id: SharedWorkerInstanceId,
    mut params: SharedWorkerLaunchParams,
    result: Result<SharedWorkerLoadedScript, String>,
) {
    match result {
        Ok(script) => {
            let Some(host) = runtime_service.take_loading_host_for_completion(instance_id) else {
                params.unregister_reserved_service_worker_client();
                return;
            };
            let live_clients =
                runtime_service.prune_disconnected_loading_clients(&host, instance_id);
            if live_clients.is_empty() {
                params.unregister_reserved_service_worker_client();
                runtime_service.close_unstarted_loading_entry(&host, &params, instance_id);
                return;
            }
            match runtime_service.finish_loading_registry_entry(&params, instance_id, host.clone())
            {
                SharedWorkerLoadReady::Running { clients, .. } => {
                    if !host.start_running(script, params) {
                        runtime_service.close_started_registry_entry_after_worker_start_failure(
                            &host,
                            instance_id,
                        );
                        return;
                    }
                    host.publish_created_target_event();
                    host.start_parent_message_pump();
                    for client_id in host.connect_pending_clients(clients) {
                        runtime_service.remove_client(client_id);
                    }
                }
                SharedWorkerLoadReady::Stale => {
                    params.unregister_reserved_service_worker_client();
                    host.close_completed_loading();
                    host.close_all_worker_ports_and_send_closed();
                }
            }
        }
        Err(message) => {
            params.unregister_reserved_service_worker_client();
            let host = runtime_service.take_loading_host_for_completion(instance_id);
            match runtime_service.fail_loading_registry_entry(&params, instance_id) {
                SharedWorkerLoadFailure::Failed { clients, .. } => {
                    if let Some(host) = host {
                        host.close_completed_loading();
                        host.fail_clients(
                            clients,
                            message,
                            params.key.script_url(),
                            WorkerParentErrorEventKind::ErrorEvent,
                        );
                    }
                }
                SharedWorkerLoadFailure::Stale => {
                    if let Some(host) = host {
                        host.close_completed_loading();
                        host.close_all_worker_ports_and_send_closed();
                    }
                }
            }
        }
    }
}

impl SharedWorkerRuntimeService {
    fn take_loading_host_for_completion(
        &self,
        instance_id: SharedWorkerInstanceId,
    ) -> Option<SharedRendererSharedWorkerHost> {
        self.remove_loading_host(instance_id)
    }

    fn prune_disconnected_loading_clients(
        &self,
        host: &RendererSharedWorkerHost,
        instance_id: SharedWorkerInstanceId,
    ) -> Vec<SharedWorkerClientId> {
        let mut live_clients = Vec::new();
        for client_id in self.loading_clients_for_instance(instance_id) {
            if host.has_client_endpoint(client_id) {
                live_clients.push(client_id);
            } else {
                self.remove_matching_client(client_id);
            }
        }
        live_clients
    }

    fn finish_loading_registry_entry(
        &self,
        params: &SharedWorkerLaunchParams,
        instance_id: SharedWorkerInstanceId,
        host: SharedRendererSharedWorkerHost,
    ) -> SharedWorkerLoadReady<SharedRendererSharedWorkerHost> {
        self.finish_loading_matching(&params.key, instance_id, host)
    }

    fn fail_loading_registry_entry(
        &self,
        params: &SharedWorkerLaunchParams,
        instance_id: SharedWorkerInstanceId,
    ) -> SharedWorkerLoadFailure {
        self.fail_loading_matching(&params.key, instance_id)
    }

    fn close_unstarted_loading_entry(
        &self,
        host: &RendererSharedWorkerHost,
        params: &SharedWorkerLaunchParams,
        instance_id: SharedWorkerInstanceId,
    ) {
        host.close_completed_loading();
        match self.fail_loading_registry_entry(params, instance_id) {
            SharedWorkerLoadFailure::Failed { clients, .. } => {
                host.close_worker_ports_and_send_closed(clients);
            }
            SharedWorkerLoadFailure::Stale => {
                host.close_all_worker_ports_and_send_closed();
            }
        }
    }

    fn close_started_registry_entry_after_worker_start_failure(
        &self,
        host: &RendererSharedWorkerHost,
        instance_id: SharedWorkerInstanceId,
    ) {
        match self.remove_matching_instance(instance_id) {
            SharedWorkerInstanceRemoval::Removed { clients, .. } => {
                host.close_worker_ports_and_send_closed(clients);
            }
            SharedWorkerInstanceRemoval::Missing => {
                host.close_all_worker_ports_and_send_closed();
            }
        }
    }
}

pub(super) struct SharedWorkerRuntimeCompletion {
    runtime_service: WeakSharedWorkerRuntimeService,
    kind: SharedWorkerRuntimeCompletionKind,
}

enum SharedWorkerRuntimeCompletionKind {
    ScriptLoadFinished {
        instance_id: SharedWorkerInstanceId,
        params: SharedWorkerLaunchParams,
        result: Result<SharedWorkerLoadedScript, String>,
    },
}

impl SharedWorkerRuntimeCompletion {
    pub(super) fn script_load_finished(
        runtime_service: WeakSharedWorkerRuntimeService,
        instance_id: SharedWorkerInstanceId,
        params: SharedWorkerLaunchParams,
        result: Result<SharedWorkerLoadedScript, String>,
    ) -> Self {
        Self {
            runtime_service,
            kind: SharedWorkerRuntimeCompletionKind::ScriptLoadFinished {
                instance_id,
                params,
                result,
            },
        }
    }

    pub(super) fn complete(self) {
        match self.kind {
            SharedWorkerRuntimeCompletionKind::ScriptLoadFinished {
                instance_id,
                params,
                result,
            } => self
                .runtime_service
                .finish_loading(instance_id, params, result),
        }
    }
}

impl fmt::Debug for SharedWorkerRuntimeCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            SharedWorkerRuntimeCompletionKind::ScriptLoadFinished { instance_id, .. } => f
                .debug_struct("SharedWorkerRuntimeCompletion::ScriptLoadFinished")
                .field("instance_id", instance_id)
                .finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use moli_shared_worker::{
        SharedWorkerConnectAction, SharedWorkerDescriptor, SharedWorkerInstanceId,
    };
    use url::Url;

    use crate::{
        message_port_runtime::new_message_port_registry,
        runtime::{RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner},
        shared_worker_runtime::{
            SharedWorkerClientEndpointDisposition, SharedWorkerClientEvent,
            SharedWorkerRuntimeOwnerWake,
            host::RendererSharedWorkerHost,
            loading::{
                SharedWorkerExecutionPolicy, SharedWorkerLaunchContext, SharedWorkerLoadedScript,
                SharedWorkerScriptLoad,
            },
            test_support,
        },
        worker::{WorkerNetworkPolicy, WorkerScriptKind},
    };

    use super::*;

    fn test_launch_context(
        browser_context_runtime: &RendererBrowserContextRuntimeOwner,
        worker_context_runtime: crate::runtime::RendererWorkerContextRuntime,
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

    #[test]
    fn immediate_script_load_failure_is_queued_through_service_lane() {
        let runtime_service = test_support::runtime_service();
        let mut service_wake_rx = test_support::install_owner_wake_sender(&runtime_service);
        let message_port_registry = new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            runtime_service.clone(),
        );
        let worker_context_runtime = browser_context_runtime.worker_context_runtime();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let params = SharedWorkerLaunchParams {
            key,
            script_load: SharedWorkerScriptLoad::failure("boom".to_owned()),
            launch_context: test_launch_context(&browser_context_runtime, worker_context_runtime),
            client_port_id,
            worker_port_id,
            client_owner_id: runtime_service.next_client_owner_id(),
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        };

        let client_id = runtime_service.connect(descriptor, params);
        let instance_id = SharedWorkerInstanceId::from_u64(1);
        assert!(
            test_support::stored_loading_host(&runtime_service, instance_id).is_some(),
            "immediate script load results should not mutate the registry before owner queue drain"
        );

        assert!(matches!(
            service_wake_rx.try_recv(),
            Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(runtime_service.drain_service_lane(), 1);

        assert!(test_support::stored_loading_host(&runtime_service, instance_id).is_none());
        let task = message_port_owner
            .pop_shared_worker_client_event()
            .expect("expected shared worker client error");
        assert_eq!(task.owner().client_id(), client_id);
        assert!(matches!(
            task.into_event(),
            SharedWorkerClientEvent::Error(error)
                if error.endpoint_disposition()
                    == SharedWorkerClientEndpointDisposition::Retire
        ));
        assert!(
            message_port_owner
                .pop_shared_worker_client_event()
                .is_none()
        );
    }

    #[test]
    fn load_completion_stays_in_service_lane_without_live_host_client() {
        let runtime_service = test_support::runtime_service();
        let mut service_wake_rx = test_support::install_owner_wake_sender(&runtime_service);
        let message_port_registry = new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            runtime_service.clone(),
        );
        let worker_context_runtime = browser_context_runtime.worker_context_runtime();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action =
            test_support::connect_matching(&runtime_service, key.clone(), descriptor.clone());
        let (instance_id, client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let host = Arc::new(RendererSharedWorkerHost::new_loading(
            instance_id,
            runtime_service.required_owner_local_host_id(),
            runtime_service.downgrade(),
            key.script_url().to_owned(),
            "loader".to_owned(),
            runtime_service.open_target_output_stream(instance_id),
            runtime_service.worker_lifecycle(),
        ));
        host.add_client(
            client_id,
            message_port_owner.shared_worker_client(
                client_id,
                client_port_id,
                worker_port_id,
                message_port_registry.clone(),
            ),
        );
        host.remove_client_endpoint(client_id);
        test_support::store_loading_host(&runtime_service, instance_id, host.clone());

        let params = SharedWorkerLaunchParams {
            key,
            script_load: SharedWorkerScriptLoad::failure(
                "not used by queued completion".to_owned(),
            ),
            launch_context: test_launch_context(&browser_context_runtime, worker_context_runtime),
            client_port_id,
            worker_port_id,
            client_owner_id: runtime_service.next_client_owner_id(),
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        };
        host.enqueue_loading_completion(params, Err("boom".to_owned()));

        assert!(
            test_support::stored_loading_host(&runtime_service, instance_id).is_some(),
            "service lane completion should not run synchronously"
        );
        assert_eq!(
            test_support::matching_clients_for_instance(&runtime_service, instance_id),
            vec![client_id]
        );
        assert!(matches!(
            service_wake_rx.try_recv(),
            Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(runtime_service.pending_service_lane_event_count(), 1);

        assert_eq!(runtime_service.drain_service_lane(), 1);

        assert!(test_support::stored_loading_host(&runtime_service, instance_id).is_none());
        assert!(test_support::matching_is_empty(&runtime_service));
        assert!(
            message_port_owner
                .pop_shared_worker_client_event()
                .is_none()
        );
    }

    #[test]
    fn successful_load_with_no_live_host_client_is_pruned_before_worker_start() {
        let runtime_service = test_support::runtime_service();
        let mut service_wake_rx = test_support::install_owner_wake_sender(&runtime_service);
        let message_port_registry = new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            runtime_service.clone(),
        );
        let worker_context_runtime = browser_context_runtime.worker_context_runtime();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action =
            test_support::connect_matching(&runtime_service, key.clone(), descriptor.clone());
        let (instance_id, client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let host =
            test_support::loading_host_with_runtime_service(instance_id, &key, &runtime_service);
        host.add_client(
            client_id,
            message_port_owner.shared_worker_client(
                client_id,
                client_port_id,
                worker_port_id,
                message_port_registry.clone(),
            ),
        );
        host.remove_client_endpoint(client_id);
        test_support::store_loading_host(&runtime_service, instance_id, host.clone());

        let params = SharedWorkerLaunchParams {
            key: key.clone(),
            script_load: SharedWorkerScriptLoad::ready(
                key.script_url().to_owned(),
                "self.close();".to_owned(),
            ),
            launch_context: test_launch_context(&browser_context_runtime, worker_context_runtime),
            client_port_id,
            worker_port_id,
            client_owner_id: runtime_service.next_client_owner_id(),
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        };
        host.enqueue_loading_completion(
            params,
            Ok(SharedWorkerLoadedScript::new(
                key.script_url().to_owned(),
                "self.close();".to_owned(),
            )),
        );

        assert!(matches!(
            service_wake_rx.try_recv(),
            Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(runtime_service.drain_service_lane(), 1);

        assert!(host.is_closed());
        assert!(test_support::stored_loading_host(&runtime_service, instance_id).is_none());
        assert!(test_support::matching_is_empty(&runtime_service));
        assert!(
            host.target_output_retired()
                .load(std::sync::atomic::Ordering::Acquire),
            "a load completion for a worker with no live client must close its uncreated target stream"
        );
    }

    #[test]
    fn worker_start_failure_after_registry_ready_removes_instance() {
        let runtime_service = test_support::runtime_service();
        let mut service_wake_rx = test_support::install_owner_wake_sender(&runtime_service);
        let message_port_registry = new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            runtime_service.clone(),
        );
        let worker_context_runtime = browser_context_runtime.worker_context_runtime();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action =
            test_support::connect_matching(&runtime_service, key.clone(), descriptor.clone());
        let (instance_id, client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let host =
            test_support::loading_host_with_runtime_service(instance_id, &key, &runtime_service);
        host.add_client(
            client_id,
            message_port_owner.shared_worker_client(
                client_id,
                client_port_id,
                worker_port_id,
                message_port_registry.clone(),
            ),
        );
        test_support::store_loading_host(&runtime_service, instance_id, host.clone());
        host.close_completed_loading();

        let params = SharedWorkerLaunchParams {
            key: key.clone(),
            script_load: SharedWorkerScriptLoad::ready(key.script_url().to_owned(), String::new()),
            launch_context: test_launch_context(&browser_context_runtime, worker_context_runtime),
            client_port_id,
            worker_port_id,
            client_owner_id: runtime_service.next_client_owner_id(),
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        };
        host.enqueue_loading_completion(
            params,
            Ok(SharedWorkerLoadedScript::new(
                key.script_url().to_owned(),
                String::new(),
            )),
        );

        assert!(matches!(
            service_wake_rx.try_recv(),
            Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(runtime_service.drain_service_lane(), 1);

        assert!(host.is_closed());
        assert!(test_support::stored_loading_host(&runtime_service, instance_id).is_none());
        assert!(test_support::matching_is_empty(&runtime_service));
        assert!(
            host.target_output_retired()
                .load(std::sync::atomic::Ordering::Acquire),
            "a stale startup completion must close its uncreated target stream"
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
    fn shared_worker_load_completion_runs_through_service_lane_wake() {
        let runtime_service = test_support::runtime_service();
        let mut service_wake_rx = test_support::install_owner_wake_sender(&runtime_service);
        let message_port_registry = new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            runtime_service.clone(),
        );
        let worker_context_runtime = browser_context_runtime.worker_context_runtime();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action =
            test_support::connect_matching(&runtime_service, key.clone(), descriptor.clone());
        let (instance_id, client_id) = match action {
            SharedWorkerConnectAction::StartLoading {
                instance_id,
                client_id,
            } => (instance_id, client_id),
            _ => panic!("expected StartLoading"),
        };
        let host =
            test_support::loading_host_with_runtime_service(instance_id, &key, &runtime_service);
        host.add_client(
            client_id,
            message_port_owner.shared_worker_client(
                client_id,
                client_port_id,
                worker_port_id,
                message_port_registry.clone(),
            ),
        );
        let _cancel_handle = host.begin_loading_task();
        test_support::store_loading_host(&runtime_service, instance_id, host);

        let params = SharedWorkerLaunchParams {
            key,
            script_load: SharedWorkerScriptLoad::failure(
                "not used by queued completion".to_owned(),
            ),
            launch_context: test_launch_context(&browser_context_runtime, worker_context_runtime),
            client_port_id,
            worker_port_id,
            client_owner_id: runtime_service.next_client_owner_id(),
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        };
        let host = test_support::stored_loading_host(&runtime_service, instance_id)
            .expect("loading host should be stored");
        host.enqueue_loading_completion(params, Err("boom".to_owned()));

        assert!(matches!(
            service_wake_rx.try_recv(),
            Ok(SharedWorkerRuntimeOwnerWake::ServiceLane)
        ));
        assert_eq!(runtime_service.drain_service_lane(), 1);

        assert!(test_support::stored_loading_host(&runtime_service, instance_id).is_none());
        assert!(
            test_support::matching_clients_for_instance(&runtime_service, instance_id).is_empty()
        );
        let task = message_port_owner
            .pop_shared_worker_client_event()
            .expect("expected shared worker client error");
        assert_eq!(task.owner().client_id(), client_id);
        assert!(matches!(
            task.into_event(),
            SharedWorkerClientEvent::Error(error)
                if error.endpoint_disposition()
                    == SharedWorkerClientEndpointDisposition::Retire
        ));
        assert!(
            message_port_owner
                .pop_shared_worker_client_event()
                .is_none()
        );
    }

    #[test]
    fn queued_load_completion_does_not_keep_runtime_service_alive_after_context_drop() {
        let runtime_service = test_support::runtime_service();
        let service_weak = runtime_service.downgrade();
        let message_port_registry = new_message_port_registry();
        let launch_context = {
            let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
                message_port_registry.clone(),
                crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
                runtime_service.clone(),
            );
            let worker_context_runtime = browser_context_runtime.worker_context_runtime();
            test_launch_context(&browser_context_runtime, worker_context_runtime)
        };
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let params = SharedWorkerLaunchParams {
            key,
            script_load: SharedWorkerScriptLoad::failure(
                "not used by late queued completion".to_owned(),
            ),
            launch_context,
            client_port_id,
            worker_port_id,
            client_owner_id: runtime_service.next_client_owner_id(),
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        };
        let event = SharedWorkerRuntimeCompletion::script_load_finished(
            runtime_service.downgrade(),
            SharedWorkerInstanceId::from_u64(1),
            params,
            Err("late completion".to_owned()),
        );

        drop(runtime_service);

        assert!(
            !service_weak.is_alive(),
            "queued owner-lane completions must not keep the browser-context SharedWorker runtime service alive"
        );
        event.complete();
    }

    #[test]
    fn stale_load_failure_after_registry_shutdown_closes_pending_clients() {
        let runtime_service = test_support::runtime_service();
        let message_port_registry = new_message_port_registry();
        let browser_context_runtime = RendererBrowserContextRuntime::new_with_parts_for_test(
            message_port_registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            runtime_service.clone(),
        );
        let worker_context_runtime = browser_context_runtime.worker_context_runtime();
        let (client_port_id, worker_port_id, message_port_owner) =
            test_support::page_message_port_pair(&message_port_registry);
        let key = test_support::shared_worker_key();
        let descriptor = SharedWorkerDescriptor::default();
        let action =
            test_support::connect_matching(&runtime_service, key.clone(), descriptor.clone());
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
                message_port_registry.clone(),
            ),
        );
        let _cancel_handle = host.begin_loading_task();
        test_support::store_loading_host(&runtime_service, instance_id, host.clone());

        let params = SharedWorkerLaunchParams {
            key,
            script_load: SharedWorkerScriptLoad::failure(
                "not used by queued completion".to_owned(),
            ),
            launch_context: test_launch_context(&browser_context_runtime, worker_context_runtime),
            client_port_id,
            worker_port_id,
            client_owner_id: runtime_service.next_client_owner_id(),
            client_event_realm: message_port_owner.shared_worker_client_event_realm(),
            worker_host_bridge_sender: message_port_owner.worker_host_bridge_sender(),
            parent_service_worker_client_id: None,
            reserved_service_worker_client_id: None,
        };

        let removals = test_support::remove_all_instances_matching(&runtime_service);
        assert_eq!(removals.len(), 1);

        runtime_service
            .downgrade()
            .finish_loading(instance_id, params, Err("boom".to_owned()));

        assert!(host.is_closed());
        assert!(test_support::stored_loading_host(&runtime_service, instance_id).is_none());
        assert!(test_support::matching_is_empty(&runtime_service));

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
}
