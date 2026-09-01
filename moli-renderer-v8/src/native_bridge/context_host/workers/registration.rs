use super::super::JsContextHost;
use super::{WorkerConnectionState, WorkerExecutionState, WorkerRelayTerminalState};
use crate::RendererSyntheticResponseBody;
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease};
use crate::page_task_queue::{
    RendererDedicatedWorkerClientEvent, RendererDedicatedWorkerMessageEvent,
    RendererPageDedicatedWorkerClientEventProducer, RendererWorkerHostBridgeEventSender,
};
use crate::runtime::RendererBrowserContextRuntime;
use crate::service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerRequestDestination};
use crate::structured_clone::V8StructuredClonePayload;
use crate::types::{DedicatedWorkerId, SubresourcePolicyContext};
use crate::worker::{
    WorkerGlobalKind, WorkerHandle, WorkerNetworkPolicy, WorkerPendingFetchContinue,
    WorkerPendingXhrContinue, WorkerRuntimeEvent, WorkerScriptKind, WorkerScriptSource,
    WorkerSpawnOptions, spawn_worker_with_options,
};
use moli_storage_key::MoliStorageKey;
use url::Url;

struct LoadedWorkerScript {
    final_url: Url,
    source: WorkerScriptSource,
    network_response: crate::protocol_types::NavigationResponse,
    response_referrer_policy: Option<String>,
    network_partition_key: Option<String>,
    policy_context: SubresourcePolicyContext,
    content_security_policies: Vec<String>,
    content_security_report_only_policies: Vec<String>,
    content_security_reporting_endpoints:
        crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
}

struct WorkerScriptLoadFailure {
    error_message: String,
    network_response: Option<Box<crate::protocol_types::NavigationResponse>>,
}

impl WorkerScriptLoadFailure {
    fn without_response(error: impl std::fmt::Display) -> Self {
        Self {
            error_message: error.to_string(),
            network_response: None,
        }
    }

    fn with_response(
        error: impl std::fmt::Display,
        response: crate::protocol_types::NavigationResponse,
    ) -> Self {
        Self {
            error_message: error.to_string(),
            network_response: Some(Box::new(response)),
        }
    }
}

impl JsContextHost {
    pub(crate) fn register_dedicated_worker_outside_settings_load(
        &self,
        dispatch_scope: super::super::OwnerDispatchScope,
    ) -> Option<ResourceLoadLease> {
        self.document_resource_loader_for_dispatch_scope(dispatch_scope)?
            .register_load(
                ResourceLoadKind::Script,
                ResourceLoadDisposition::Ordinary,
                None,
            )
    }

    fn dedicated_worker_client_event_producer(
        &self,
        worker_id: DedicatedWorkerId,
        owner: &super::super::WindowExecutionContextBinding,
    ) -> RendererPageDedicatedWorkerClientEventProducer {
        let identity = owner
            .resolve_identity(self)
            .or_else(|| {
                self.current_registered_window_execution_context_identity(owner.dispatch_scope())
                    .filter(|identity| identity.owner() == owner.owner())
            })
            .expect("a registered DedicatedWorker must retain its Window owner identity");
        self.page_dedicated_worker_client_event_sender()
            .bind_worker(identity, worker_id)
    }

    fn dedicated_worker_message_event(
        message: crate::worker::WorkerToParentMessage,
    ) -> Result<RendererDedicatedWorkerMessageEvent, Box<crate::worker::WorkerToParentMessage>>
    {
        match message {
            crate::worker::WorkerToParentMessage::Post(payload) => {
                Ok(RendererDedicatedWorkerMessageEvent::Message(payload))
            }
            crate::worker::WorkerToParentMessage::Error {
                message,
                filename,
                lineno,
                colno,
                event_kind,
                phase,
                source,
            } => Ok(RendererDedicatedWorkerMessageEvent::Error {
                message,
                filename,
                lineno,
                colno,
                event_kind,
                phase,
                source,
            }),
            message => Err(Box::new(message)),
        }
    }

    fn start_worker_message_relay(
        worker_id: DedicatedWorkerId,
        client_event_producer: RendererPageDedicatedWorkerClientEventProducer,
        worker_host_bridge_tx: RendererWorkerHostBridgeEventSender,
        worker_handle: &mut WorkerHandle,
    ) {
        let Some(mut rx) = worker_handle.take_receiver() else {
            return;
        };
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                match Self::dedicated_worker_message_event(message) {
                    Ok(event) => {
                        let _ = client_event_producer
                            .send(RendererDedicatedWorkerClientEvent::Message(event));
                    }
                    Err(message) => {
                        let _ = worker_host_bridge_tx
                            .send(WorkerRuntimeEvent::Message { worker_id, message });
                    }
                }
            }
            // Each marker is ordered behind the relay records routed through
            // its own FIFO. The wrapper can be retired only after both sides
            // observe their marker; Page-source fairness may otherwise let a
            // terminal overtake records in the other source.
            let _ =
                client_event_producer.send(RendererDedicatedWorkerClientEvent::ClientSourceDrained);
            let _ = worker_host_bridge_tx.send(WorkerRuntimeEvent::HostBridgeDrained { worker_id });
        });
    }

    pub(crate) fn register_worker(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        wrapper: v8::Local<'_, v8::Object>,
        mut worker_handle: WorkerHandle,
        owner: super::super::WindowExecutionContextBinding,
    ) -> DedicatedWorkerId {
        let worker_id = DedicatedWorkerId::new(self.next_worker_id);
        self.next_worker_id += 1;
        let renderer_instance_id = self
            .browser_context_runtime()
            .allocate_dedicated_worker_instance_id();
        self.browser_context_runtime()
            .attach_dedicated_worker_devtools_handle(
                renderer_instance_id,
                &worker_handle,
                self.renderer_output_journal(),
            );
        let client_event_producer = self.dedicated_worker_client_event_producer(worker_id, &owner);
        Self::start_worker_message_relay(
            worker_id,
            client_event_producer.clone(),
            self.page_worker_host_bridge_event_sender().clone(),
            &mut worker_handle,
        );
        self.workers.insert(
            worker_id,
            WorkerConnectionState {
                renderer_instance_id,
                target_created: false,
                wrapper: v8::Global::new(scope, wrapper),
                owner,
                client_event_producer,
                relay_terminal: WorkerRelayTerminalState::default(),
                execution: WorkerExecutionState::Running {
                    handle: worker_handle,
                },
            },
        );
        worker_id
    }

    pub(crate) fn register_loading_worker(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        wrapper: v8::Local<'_, v8::Object>,
        storage_key_top_level_site: String,
        creator_storage_key: MoliStorageKey,
        name: String,
        module_credentials_mode: moli_fetch::RequestCredentialsMode,
        reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
        outside_settings_load: ResourceLoadLease,
        owner: super::super::WindowExecutionContextBinding,
    ) -> DedicatedWorkerId {
        let worker_id = DedicatedWorkerId::new(self.next_worker_id);
        self.next_worker_id += 1;
        let renderer_instance_id = self
            .browser_context_runtime()
            .allocate_dedicated_worker_instance_id();
        let client_event_producer = self.dedicated_worker_client_event_producer(worker_id, &owner);
        self.workers.insert(
            worker_id,
            WorkerConnectionState {
                renderer_instance_id,
                target_created: false,
                wrapper: v8::Global::new(scope, wrapper),
                owner,
                client_event_producer,
                relay_terminal: WorkerRelayTerminalState::default(),
                execution: WorkerExecutionState::Loading {
                    pending_messages: Vec::new(),
                    load_task: None,
                    terminated: false,
                    outside_settings_load,
                    name,
                    module_credentials_mode,
                    storage_key_top_level_site,
                    creator_storage_key,
                    reserved_service_worker_client_id,
                },
            },
        );
        worker_id
    }

    pub(crate) fn start_worker_script_load(
        &mut self,
        worker_id: DedicatedWorkerId,
        script_url: Url,
        initiator_url: Url,
        network_partition_key: Option<String>,
        creator_policy_context: SubresourcePolicyContext,
        script_kind: WorkerScriptKind,
        module_credentials_mode: moli_fetch::RequestCredentialsMode,
        document_referrer_policy: Option<String>,
        name: String,
        reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
    ) -> bool {
        let browser_context_runtime = self.browser_context_runtime();
        let Some((outside_settings_load, client_event_producer)) =
            self.workers.get(&worker_id).map(|state| {
                let outside_settings_load = match &state.execution {
                    WorkerExecutionState::Loading {
                        outside_settings_load,
                        ..
                    } => Some(outside_settings_load.clone()),
                    WorkerExecutionState::Running { .. } => None,
                };
                (outside_settings_load, state.client_event_producer.clone())
            })
        else {
            return false;
        };
        let Some(outside_settings_load) = outside_settings_load else {
            return false;
        };
        let request_client = outside_settings_load.request_client();
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        outside_settings_load.attach_cancel_handle(cancel_handle.clone());
        let creator_secure_context = moli_url::is_potentially_trustworthy_url(&initiator_url);
        let task_runner = outside_settings_load.task_runner();
        let fetch_task_runner = task_runner.clone();
        let load_task = task_runner.spawn_abortable(async move {
            let result = fetch_worker_script_source(
                &request_client,
                fetch_task_runner,
                cancel_handle,
                &script_url,
                &initiator_url,
                network_partition_key.clone(),
                creator_policy_context,
                script_kind,
                module_credentials_mode,
                document_referrer_policy,
                browser_context_runtime,
                reserved_service_worker_client_id,
            )
            .await;
            outside_settings_load.finish();
            match result {
                Ok(mut loaded) => {
                    let final_url = &mut loaded.final_url;
                    if final_url.fragment().is_none() {
                        final_url.set_fragment(script_url.fragment());
                    }
                    let secure_context = crate::worker::worker_secure_context_for_script_url(
                        final_url,
                        creator_secure_context,
                    );
                    let _ = client_event_producer.send(
                        RendererDedicatedWorkerClientEvent::ScriptLoaded {
                            script_url: final_url.to_string(),
                            script_source: loaded.source,
                            network_response: Box::new(loaded.network_response),
                            script_kind,
                            secure_context,
                            response_referrer_policy: loaded.response_referrer_policy,
                            network_partition_key: loaded.network_partition_key,
                            policy_context: loaded.policy_context,
                            content_security_policies: loaded.content_security_policies,
                            content_security_report_only_policies: loaded
                                .content_security_report_only_policies,
                            content_security_reporting_endpoints: loaded
                                .content_security_reporting_endpoints,
                        },
                    );
                }
                Err(error) => {
                    let _ = client_event_producer.send(
                        RendererDedicatedWorkerClientEvent::ScriptLoadFailed {
                            script_url: script_url.to_string(),
                            error_message: error.error_message,
                            network_response: error.network_response,
                        },
                    );
                }
            }
        });
        match self.workers.get_mut(&worker_id) {
            Some(state) => match &mut state.execution {
                WorkerExecutionState::Loading {
                    load_task: slot,
                    terminated,
                    name: loading_name,
                    ..
                } => {
                    if *terminated {
                        load_task.abort();
                        return false;
                    }
                    *loading_name = name;
                    *slot = Some(load_task);
                    true
                }
                WorkerExecutionState::Running { .. } => {
                    load_task.abort();
                    false
                }
            },
            None => {
                load_task.abort();
                false
            }
        }
    }

    pub(crate) fn start_failed_worker_script_load(
        &mut self,
        worker_id: DedicatedWorkerId,
        script_url: Url,
        error_message: &'static str,
    ) -> bool {
        let Some((client_event_producer, outside_settings_load)) = self
            .workers
            .get(&worker_id)
            .and_then(|state| match &state.execution {
                WorkerExecutionState::Loading {
                    outside_settings_load,
                    ..
                } => Some((
                    state.client_event_producer.clone(),
                    outside_settings_load.clone(),
                )),
                WorkerExecutionState::Running { .. } => None,
            })
        else {
            return false;
        };
        let task_runner = outside_settings_load.task_runner();
        let load_task = task_runner.spawn_abortable(async move {
            outside_settings_load.finish();
            let _ =
                client_event_producer.send(RendererDedicatedWorkerClientEvent::ScriptLoadFailed {
                    script_url: script_url.to_string(),
                    error_message: error_message.to_owned(),
                    network_response: None,
                });
        });
        match self.workers.get_mut(&worker_id) {
            Some(state) => match &mut state.execution {
                WorkerExecutionState::Loading {
                    load_task: slot,
                    terminated,
                    ..
                } => {
                    if *terminated {
                        load_task.abort();
                        return false;
                    }
                    *slot = Some(load_task);
                    true
                }
                WorkerExecutionState::Running { .. } => {
                    load_task.abort();
                    false
                }
            },
            None => {
                load_task.abort();
                false
            }
        }
    }

    pub(crate) fn finish_loading_worker(
        &mut self,
        worker_id: DedicatedWorkerId,
        script_url: String,
        script_source: WorkerScriptSource,
        script_kind: WorkerScriptKind,
        secure_context: bool,
        response_referrer_policy: Option<String>,
        network_partition_key: Option<String>,
        policy_context: SubresourcePolicyContext,
        content_security_policies: Vec<String>,
        content_security_report_only_policies: Vec<String>,
        content_security_reporting_endpoints:
            crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    ) -> bool {
        enum FinishLoadingAction {
            MissingOrRunning,
            DiscardTerminated,
            Spawn {
                pending_messages: Vec<V8StructuredClonePayload>,
                name: String,
                storage_key_top_level_site: String,
                creator_storage_key: MoliStorageKey,
                reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
                module_credentials_mode: moli_fetch::RequestCredentialsMode,
                request_client: crate::network::ResourceRequestClient,
            },
        }

        let action = match self.workers.get_mut(&worker_id) {
            Some(state) => match &mut state.execution {
                WorkerExecutionState::Loading {
                    pending_messages,
                    load_task,
                    terminated,
                    name,
                    module_credentials_mode,
                    storage_key_top_level_site,
                    creator_storage_key,
                    reserved_service_worker_client_id,
                    outside_settings_load,
                } => {
                    let _ = load_task.take();
                    if *terminated {
                        FinishLoadingAction::DiscardTerminated
                    } else {
                        FinishLoadingAction::Spawn {
                            pending_messages: std::mem::take(pending_messages),
                            name: name.clone(),
                            storage_key_top_level_site: storage_key_top_level_site.clone(),
                            creator_storage_key: creator_storage_key.clone(),
                            reserved_service_worker_client_id: reserved_service_worker_client_id
                                .take(),
                            module_credentials_mode: *module_credentials_mode,
                            request_client: outside_settings_load.request_client(),
                        }
                    }
                }
                WorkerExecutionState::Running { .. } => FinishLoadingAction::MissingOrRunning,
            },
            None => FinishLoadingAction::MissingOrRunning,
        };
        let (
            pending_messages,
            name,
            storage_key_top_level_site,
            creator_storage_key,
            reserved_service_worker_client_id,
            module_credentials_mode,
            request_client,
        ) = match action {
            FinishLoadingAction::MissingOrRunning => return false,
            FinishLoadingAction::DiscardTerminated => {
                self.forget_worker(worker_id);
                return false;
            }
            FinishLoadingAction::Spawn {
                pending_messages,
                name,
                storage_key_top_level_site,
                creator_storage_key,
                reserved_service_worker_client_id,
                module_credentials_mode,
                request_client,
            } => (
                pending_messages,
                name,
                storage_key_top_level_site,
                creator_storage_key,
                reserved_service_worker_client_id,
                module_credentials_mode,
                request_client,
            ),
        };
        let network_policy = WorkerNetworkPolicy {
            secure_context,
            permission_overrides: self.permission_overrides().to_vec(),
            extra_http_headers: self.extra_http_headers().to_vec(),
            network_offline: self.network_offline(),
            blocked_url_patterns: self.blocked_url_patterns().to_vec(),
            network_partition_key,
            fetch_subresource_interception_enabled: self.fetch_subresource_interception_enabled(),
            fetch_subresource_interception_resource_type: self
                .fetch_subresource_interception_resource_type(),
        };
        let mut spawn_options = WorkerSpawnOptions::with_source_and_request_client(
            script_source,
            script_url,
            request_client,
        )
        .with_script_kind(script_kind)
        .with_module_credentials_mode(module_credentials_mode)
        .with_referrer_policy(response_referrer_policy)
        .with_content_security_policies(content_security_policies)
        .with_content_security_report_only_policies(content_security_report_only_policies)
        .with_content_security_reporting_endpoints(content_security_reporting_endpoints)
        .with_network_policy(network_policy)
        .with_policy_context(policy_context)
        .with_worker_context_runtime(self.browser_context_runtime().worker_context_runtime())
        .with_service_worker_runtime(self.browser_context_runtime().service_worker_runtime())
        .with_global_kind(WorkerGlobalKind::Dedicated { name })
        .with_storage_key_top_level_site(Some(storage_key_top_level_site))
        .with_creator_storage_key(creator_storage_key)
        .with_storage_bucket_store(Some(self.storage_bucket_store()))
        .with_indexed_db_manager(self.indexed_db_manager())
        .with_pause_evaluation_until_debugger(
            self.browser_context_runtime()
                .dedicated_worker_pause_on_start_for_devtools(),
        );
        if let Some(client_id) = reserved_service_worker_client_id {
            spawn_options = spawn_options.with_reserved_service_worker_client_id(client_id);
        }
        let mut worker_handle = spawn_worker_with_options(spawn_options);
        let renderer_instance_id = self
            .workers
            .get(&worker_id)
            .map(|state| state.renderer_instance_id)
            .expect("loading DedicatedWorker must retain its renderer instance identity");
        self.browser_context_runtime()
            .attach_dedicated_worker_devtools_handle(
                renderer_instance_id,
                &worker_handle,
                self.renderer_output_journal(),
            );
        for message in pending_messages {
            worker_handle.post_message(message);
        }
        let client_event_producer = self
            .workers
            .get(&worker_id)
            .map(|state| state.client_event_producer.clone())
            .expect("loading DedicatedWorker must retain its bound client-event producer");
        let worker_host_bridge_tx = self.page_worker_host_bridge_event_sender().clone();
        Self::start_worker_message_relay(
            worker_id,
            client_event_producer,
            worker_host_bridge_tx,
            &mut worker_handle,
        );
        let Some(state) = self.workers.get_mut(&worker_id) else {
            worker_handle.terminate();
            return false;
        };
        state.execution = WorkerExecutionState::Running {
            handle: worker_handle,
        };
        true
    }

    pub(crate) fn record_dedicated_worker_target_created(
        &mut self,
        worker_id: DedicatedWorkerId,
        document_url: Url,
        request_url: Url,
        name: String,
    ) -> bool {
        let renderer_instance_id = {
            let Some(state) = self.workers.get_mut(&worker_id) else {
                return false;
            };
            if state.target_created {
                return false;
            }
            state.target_created = true;
            state.renderer_instance_id
        };
        let page_token = self
            .page_dedicated_worker_client_event_sender()
            .page_token();
        self.append_dedicated_worker_target_lifecycle(
            crate::runtime::RendererDedicatedWorkerTargetEvent::Created(
                crate::runtime::RendererDedicatedWorkerTargetInfo {
                    owner_local_host_id: page_token.local_host_id(),
                    page_id: page_token.page_id(),
                    instance_id: renderer_instance_id,
                    request_url: request_url.to_string(),
                    document_url: document_url.to_string(),
                    name,
                },
            ),
        );
        true
    }

    /// Publishes one Worker lifecycle fact without conflating a test-only
    /// missing output sink with a disappeared Worker.
    ///
    /// Production Pages structurally require an owner reservation and bind a
    /// concrete output journal. Low-level standalone PageVm fixtures
    /// deliberately omit that journal while still exercising Worker state.
    fn append_dedicated_worker_target_lifecycle(
        &self,
        event: crate::runtime::RendererDedicatedWorkerTargetEvent,
    ) {
        let appended = self.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::DedicatedWorkerTargetLifecycle(event),
        );
        debug_assert!(
            appended || cfg!(test),
            "production DedicatedWorker lifecycle requires a renderer output sink"
        );
    }

    pub(crate) fn record_dedicated_worker_target_script_loaded(
        &mut self,
        worker_id: DedicatedWorkerId,
        script_url: String,
        response: Box<crate::protocol_types::NavigationResponse>,
    ) -> bool {
        let Some(state) = self.workers.get(&worker_id) else {
            return false;
        };
        if !state.target_created {
            return true;
        }
        let instance_id = state.renderer_instance_id;
        self.append_dedicated_worker_target_lifecycle(
            crate::runtime::RendererDedicatedWorkerTargetEvent::ScriptLoaded {
                instance_id,
                script_url,
                response,
            },
        );
        true
    }

    pub(crate) fn record_dedicated_worker_target_script_load_failed(
        &mut self,
        worker_id: DedicatedWorkerId,
        script_url: String,
        error_message: String,
        response: Option<Box<crate::protocol_types::NavigationResponse>>,
    ) -> bool {
        let Some(state) = self.workers.get(&worker_id) else {
            return false;
        };
        if !state.target_created {
            return true;
        }
        let instance_id = state.renderer_instance_id;
        self.append_dedicated_worker_target_lifecycle(
            crate::runtime::RendererDedicatedWorkerTargetEvent::ScriptLoadFailed {
                instance_id,
                script_url,
                error_message,
                response,
            },
        );
        true
    }

    pub(crate) fn record_dedicated_worker_runtime_inspector_messages(
        &mut self,
        worker_id: DedicatedWorkerId,
        batches: Vec<crate::worker::WorkerRuntimeInspectorMessageBatch>,
    ) -> bool {
        let Some(state) = self.workers.get(&worker_id) else {
            return false;
        };
        if !state.target_created {
            return true;
        }
        let instance_id = state.renderer_instance_id;
        for batch in batches {
            self.append_dedicated_worker_target_lifecycle(
                crate::runtime::RendererDedicatedWorkerTargetEvent::RuntimeInspectorMessages {
                    instance_id,
                    inspector_session_id: batch.inspector_session_id,
                    messages: batch.messages,
                },
            );
        }
        true
    }

    pub(crate) fn record_dedicated_worker_target_console_message(
        &mut self,
        worker_id: DedicatedWorkerId,
        message: crate::worker::WorkerConsoleMessage,
    ) -> bool {
        let Some(state) = self.workers.get(&worker_id) else {
            return false;
        };
        if !state.target_created {
            return true;
        }
        self.append_dedicated_worker_target_lifecycle(
            crate::runtime::RendererDedicatedWorkerTargetEvent::Console {
                instance_id: state.renderer_instance_id,
                message: crate::runtime::RendererSharedWorkerConsoleMessage {
                    message: message.message,
                    args: message.args,
                    stack: message.stack,
                },
            },
        );
        true
    }

    pub(crate) fn post_worker_message(
        &mut self,
        worker_id: DedicatedWorkerId,
        payload: V8StructuredClonePayload,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| {
                match &mut state.execution {
                    WorkerExecutionState::Loading {
                        pending_messages,
                        terminated,
                        ..
                    } => {
                        if *terminated {
                            return true;
                        }
                        pending_messages.push(payload);
                    }
                    WorkerExecutionState::Running { handle } => {
                        handle.post_message(payload);
                    }
                }
                true
            })
            .unwrap_or(false)
    }

    pub(crate) fn continue_worker_fetch(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.continue_pending_fetch(request);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn continue_worker_xhr(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingXhrContinue,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.continue_pending_xhr(request);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn continue_worker_csp_report(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.continue_pending_csp_report(request);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn continue_worker_fetch_response(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.continue_pending_fetch_response(
                        request,
                        response_code,
                        response_headers,
                    );
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn continue_worker_xhr_response(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.continue_pending_xhr_response(request, response_code, response_headers);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fail_worker_fetch(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fail_pending_fetch(request, error_text);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fail_worker_xhr(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fail_pending_xhr(request, error_text);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fail_worker_csp_report(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fail_pending_csp_report(request, error_text);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fail_worker_fetch_response(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fail_pending_fetch_response(request, error_text);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fail_worker_xhr_response(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fail_pending_xhr_response(request, error_text);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fail_worker_fetch_auth(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fail_pending_fetch_auth(request, error_text);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fail_worker_xhr_auth(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fail_pending_xhr_auth(request, error_text);
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fulfill_worker_fetch(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fulfill_pending_fetch(
                        request,
                        response_code,
                        response_headers,
                        response_body,
                    );
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fulfill_worker_xhr(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fulfill_pending_xhr(
                        request,
                        response_code,
                        response_headers,
                        response_body,
                    );
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fulfill_worker_csp_report(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fulfill_pending_csp_report(
                        request,
                        response_code,
                        response_headers,
                        response_body,
                    );
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fulfill_worker_fetch_response(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fulfill_pending_fetch_response(
                        request,
                        response_code,
                        response_headers,
                        response_body,
                    );
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn fulfill_worker_xhr_response(
        &mut self,
        worker_id: DedicatedWorkerId,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.workers
            .get_mut(&worker_id)
            .map(|state| match &mut state.execution {
                WorkerExecutionState::Running { handle } => {
                    handle.fulfill_pending_xhr_response(
                        request,
                        response_code,
                        response_headers,
                        response_body,
                    );
                    true
                }
                WorkerExecutionState::Loading { .. } => false,
            })
            .unwrap_or(false)
    }

    pub(crate) fn terminate_worker(&mut self, worker_id: DedicatedWorkerId) -> bool {
        let browser_context_runtime = self.browser_context_runtime();
        self.workers
            .get_mut(&worker_id)
            .map(|state| {
                match &mut state.execution {
                    WorkerExecutionState::Loading {
                        pending_messages,
                        load_task,
                        terminated,
                        reserved_service_worker_client_id,
                        ..
                    } => {
                        *terminated = true;
                        pending_messages.clear();
                        if let Some(load_task) = load_task.take() {
                            load_task.abort();
                        }
                        if let Some(client_id) = reserved_service_worker_client_id.take() {
                            browser_context_runtime.unregister_service_worker_client(client_id);
                        }
                    }
                    WorkerExecutionState::Running { handle } => {
                        handle.terminate();
                    }
                }
                true
            })
            .unwrap_or(false)
    }

    pub(crate) fn release_loading_worker_service_worker_client(
        &mut self,
        worker_id: DedicatedWorkerId,
    ) {
        let browser_context_runtime = self.browser_context_runtime();
        if let Some(state) = self.workers.get_mut(&worker_id)
            && let WorkerExecutionState::Loading {
                reserved_service_worker_client_id,
                ..
            } = &mut state.execution
            && let Some(client_id) = reserved_service_worker_client_id.take()
        {
            browser_context_runtime.unregister_service_worker_client(client_id);
        }
    }

    pub(crate) fn worker_execution_context_is_current(
        &mut self,
        worker_id: DedicatedWorkerId,
    ) -> bool {
        let Some((owner, dispatch_scope)) = self
            .workers
            .get(&worker_id)
            .map(|state| (state.owner.owner(), state.owner.dispatch_scope()))
        else {
            return false;
        };
        if self.window_execution_context_owner_is_current(owner, dispatch_scope) {
            return true;
        }
        self.forget_worker(worker_id);
        tracing::debug!(
            worker_id = worker_id.as_u64(),
            ?owner,
            ?dispatch_scope,
            "retired DedicatedWorker after its LocalWindow stopped being current"
        );
        false
    }

    /// Pure exact-target lookup used by the Page arbiter before it authorizes
    /// one DedicatedWorker client event. Unlike the legacy bridge lookup this
    /// never retires state or otherwise advances the target.
    pub(crate) fn current_dedicated_worker_client_event_identity(
        &self,
        worker_id: DedicatedWorkerId,
    ) -> Option<super::super::WindowExecutionContextIdentity> {
        let state = self.workers.get(&worker_id)?;
        let identity = state.client_event_producer.owner().execution_context();
        self.window_execution_context_identity_is_current(identity)
            .then_some(identity)
    }

    pub(crate) fn mark_dedicated_worker_client_source_drained(
        &mut self,
        worker_id: DedicatedWorkerId,
    ) -> bool {
        let should_retire = match self.workers.get_mut(&worker_id) {
            Some(state) => {
                state.relay_terminal.mark_client_source_drained();
                state.relay_terminal.is_fully_drained()
            }
            None => return false,
        };
        if should_retire {
            self.forget_worker(worker_id);
        }
        true
    }

    pub(crate) fn mark_dedicated_worker_host_bridge_drained(
        &mut self,
        worker_id: DedicatedWorkerId,
    ) -> bool {
        let should_retire = match self.workers.get_mut(&worker_id) {
            Some(state) => {
                state.relay_terminal.mark_host_bridge_drained();
                state.relay_terminal.is_fully_drained()
            }
            None => return false,
        };
        if should_retire {
            self.forget_worker(worker_id);
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn dedicated_worker_client_event_producer_for_test(
        &self,
        worker_id: DedicatedWorkerId,
    ) -> Option<RendererPageDedicatedWorkerClientEventProducer> {
        self.workers
            .get(&worker_id)
            .map(|state| state.client_event_producer.clone())
    }

    /// Resolve the already-authorized V8 target without performing another
    /// current/stale arbitration in the executor.
    pub(crate) fn authorized_dedicated_worker_dispatch_target<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        worker_id: DedicatedWorkerId,
        execution_context: super::super::WindowExecutionContextIdentity,
    ) -> Option<(
        super::super::OwnerDispatchScope,
        v8::Local<'s, v8::Context>,
        v8::Local<'s, v8::Object>,
    )> {
        let state = self.workers.get(&worker_id)?;
        assert_eq!(
            state.client_event_producer.owner().execution_context(),
            execution_context,
            "authorized DedicatedWorker event target changed inside one owner turn"
        );
        Some((
            state.owner.dispatch_scope(),
            state.owner.context(scope),
            v8::Local::new(scope, &state.wrapper),
        ))
    }

    pub(crate) fn worker_dispatch_target<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        worker_id: DedicatedWorkerId,
    ) -> Option<(
        super::super::OwnerDispatchScope,
        super::super::RuntimeObservableContextToken,
        v8::Local<'s, v8::Context>,
        v8::Local<'s, v8::Object>,
    )> {
        if !self.worker_execution_context_is_current(worker_id) {
            return None;
        }
        let state = self.workers.get(&worker_id)?;
        Some((
            state.owner.dispatch_scope(),
            state.owner.realm_token(),
            state.owner.context(scope),
            v8::Local::new(scope, &state.wrapper),
        ))
    }

    pub(crate) fn retire_workers_for_execution_context_owner(
        &mut self,
        owner: super::super::WindowExecutionContextOwner,
    ) -> usize {
        let worker_ids = self
            .workers
            .iter()
            .filter_map(|(worker_id, state)| (state.owner.owner() == owner).then_some(*worker_id))
            .collect::<Vec<_>>();
        let retired_count = worker_ids.len();
        for worker_id in worker_ids {
            self.forget_worker(worker_id);
        }
        if retired_count > 0 {
            tracing::debug!(
                ?owner,
                retired_count,
                "retired DedicatedWorkers with LocalWindow execution context"
            );
        }
        retired_count
    }

    pub(crate) fn retire_workers_for_context_token(
        &mut self,
        context_token: super::super::RuntimeObservableContextToken,
    ) -> usize {
        let worker_ids = self
            .workers
            .iter()
            .filter_map(|(worker_id, state)| {
                (state.owner.realm_token() == context_token).then_some(*worker_id)
            })
            .collect::<Vec<_>>();
        let retired_count = worker_ids.len();
        for worker_id in worker_ids {
            self.forget_worker(worker_id);
        }
        if retired_count > 0 {
            tracing::debug!(
                ?context_token,
                retired_count,
                "retired DedicatedWorkers with destroyed V8 execution context"
            );
        }
        retired_count
    }

    #[cfg(test)]
    pub(crate) fn worker_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        DedicatedWorkerId,
        super::super::WindowExecutionContextOwner,
        super::super::RuntimeObservableContextToken,
    )> {
        let mut workers = self
            .workers
            .iter()
            .map(|(worker_id, state)| (*worker_id, state.owner.owner(), state.owner.realm_token()))
            .collect::<Vec<_>>();
        workers.sort_by_key(|(worker_id, _, _)| worker_id.as_u64());
        workers
    }

    pub(crate) fn forget_worker(&mut self, worker_id: DedicatedWorkerId) {
        let retired_subresource_count = self.cancel_subresource_fetches_for_worker(worker_id);
        let browser_context_runtime = self.browser_context_runtime();
        if let Some(state) = self.workers.remove(&worker_id) {
            self.browser_context_runtime()
                .unregister_dedicated_worker_devtools_handle(state.renderer_instance_id);
            if state.target_created {
                self.append_dedicated_worker_target_lifecycle(
                    crate::runtime::RendererDedicatedWorkerTargetEvent::Destroyed {
                        instance_id: state.renderer_instance_id,
                    },
                );
            }
            let owner = state.owner.owner();
            let realm_token = state.owner.realm_token();
            let execution_state = match &state.execution {
                WorkerExecutionState::Loading { .. } => "loading",
                WorkerExecutionState::Running { .. } => "running",
            };
            match state.execution {
                WorkerExecutionState::Loading {
                    mut load_task,
                    reserved_service_worker_client_id,
                    ..
                } => {
                    if let Some(load_task) = load_task.take() {
                        load_task.abort();
                    }
                    if let Some(client_id) = reserved_service_worker_client_id {
                        browser_context_runtime.unregister_service_worker_client(client_id);
                    }
                }
                WorkerExecutionState::Running { .. } => {}
            }
            tracing::debug!(
                worker_id = worker_id.as_u64(),
                ?owner,
                ?realm_token,
                execution_state,
                retired_subresource_count,
                "forgot DedicatedWorker execution-context state"
            );
        }
    }

    pub(crate) fn shutdown_workers(&mut self) {
        let browser_context_runtime = self.browser_context_runtime();
        let workers = std::mem::take(&mut self.workers);
        for (_, state) in workers {
            browser_context_runtime
                .unregister_dedicated_worker_devtools_handle(state.renderer_instance_id);
            match state.execution {
                WorkerExecutionState::Loading {
                    load_task,
                    reserved_service_worker_client_id,
                    ..
                } => {
                    if let Some(load_task) = load_task {
                        load_task.abort();
                    }
                    if let Some(client_id) = reserved_service_worker_client_id {
                        browser_context_runtime.unregister_service_worker_client(client_id);
                    }
                }
                WorkerExecutionState::Running { handle } => {
                    handle.terminate_and_join();
                }
            }
        }
    }
}

async fn fetch_worker_script_source(
    request_client: &crate::network::ResourceRequestClient,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    cancel_handle: moli_fetch::FetchCancelHandle,
    script_url: &Url,
    initiator_url: &Url,
    network_partition_key: Option<String>,
    creator_policy_context: SubresourcePolicyContext,
    script_kind: WorkerScriptKind,
    module_credentials_mode: moli_fetch::RequestCredentialsMode,
    document_referrer_policy: Option<String>,
    browser_context_runtime: RendererBrowserContextRuntime,
    reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
) -> Result<LoadedWorkerScript, WorkerScriptLoadFailure> {
    let mut request_url = script_url.clone();
    request_url.set_fragment(None);
    let mut request = moli_fetch::Request::new("GET", request_url.as_str(), None, vec![])
        .map_err(WorkerScriptLoadFailure::without_response)?
        .with_page_network_policy()
        .with_network_partition_key(network_partition_key.clone())
        .with_initiator_url(initiator_url)
        .with_credentials_mode(match script_kind {
            WorkerScriptKind::Classic => moli_fetch::RequestCredentialsMode::SameOrigin,
            WorkerScriptKind::Module => module_credentials_mode,
        });
    if let Some(document_referrer_policy) = document_referrer_policy {
        request = request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
            document_referrer_policy: Some(document_referrer_policy),
            ..moli_fetch::ScriptFetchRequestMetadata::default()
        });
    }
    if let Some(client_id) = reserved_service_worker_client_id
        && let Some(response) = browser_context_runtime
            .fetch_service_worker_main_resource_for_worker(
                client_id,
                &request,
                request_client,
                resource_task_runner,
                ServiceWorkerRequestDestination::Worker,
            )
            .await
            .map_err(|error| {
                WorkerScriptLoadFailure::without_response(format!(
                    "failed to fetch worker script `{script_url}` through service worker: {error}"
                ))
            })?
    {
        return loaded_worker_script_from_navigation_response(
            response,
            initiator_url,
            network_partition_key,
            creator_policy_context,
            script_kind,
        );
    }
    let observed = request_client
        .fetch_text_stream_with_cancel_and_network_metadata(request, cancel_handle)
        .await
        .map_err(|error| {
            WorkerScriptLoadFailure::without_response(format!(
                "failed to fetch worker script `{script_url}`: {error}"
            ))
        })?;
    let (response, request_observation) = observed.into_parts();
    let network_request_headers =
        request_observation.map(moli_fetch::NetworkRequestObservation::into_headers);

    loaded_worker_script_from_navigation_response(
        crate::protocol_types::NavigationResponse::from(response)
            .with_network_request_headers(network_request_headers),
        initiator_url,
        network_partition_key,
        creator_policy_context,
        script_kind,
    )
}

fn loaded_worker_script_from_navigation_response(
    response: crate::protocol_types::NavigationResponse,
    initiator_url: &Url,
    network_partition_key: Option<String>,
    creator_policy_context: SubresourcePolicyContext,
    script_kind: WorkerScriptKind,
) -> Result<LoadedWorkerScript, WorkerScriptLoadFailure> {
    let response_head = response.head();
    if let Err(error) = crate::worker::ensure_worker_script_redirect_chain_same_origin(
        initiator_url,
        &response_head.redirect_chain,
        &response.final_url,
    ) {
        return Err(WorkerScriptLoadFailure::with_response(error, response));
    }
    if let Err(error) =
        moli_fetch::ensure_http_status_success(response.final_url.as_str(), response.status, false)
    {
        return Err(WorkerScriptLoadFailure::with_response(error, response));
    }
    let content_security_policies =
        crate::content_security_policy::content_security_policy_headers(&response.headers);
    let content_security_report_only_policies =
        crate::content_security_policy::content_security_policy_report_only_headers(
            &response.headers,
        );
    let content_security_reporting_endpoints =
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            &response.headers,
            &response.final_url,
        );
    let response_referrer_policy =
        crate::referrer_policy::response_referrer_policy_from_headers(&response.headers);
    let policy_context =
        dedicated_worker_policy_context_from_headers(&response.headers, creator_policy_context);
    if script_kind == WorkerScriptKind::Module
        && crate::worker::worker_response_has_webassembly_mime(&response.headers)
    {
        return Ok(LoadedWorkerScript {
            final_url: response.final_url.clone(),
            source: WorkerScriptSource::binary(response.clone_body_bytes()),
            response_referrer_policy,
            network_partition_key,
            policy_context,
            content_security_policies,
            content_security_report_only_policies,
            content_security_reporting_endpoints,
            network_response: response,
        });
    }
    if let Err(error) = crate::worker::ensure_worker_script_mime_acceptable(
        &response.final_url,
        &response.headers,
        response.body_bytes(),
    ) {
        return Err(WorkerScriptLoadFailure::with_response(error, response));
    }

    Ok(LoadedWorkerScript {
        final_url: response.final_url.clone(),
        source: WorkerScriptSource::text(response.body_text().to_owned()),
        response_referrer_policy,
        network_partition_key,
        policy_context,
        content_security_policies,
        content_security_report_only_policies,
        content_security_reporting_endpoints,
        network_response: response,
    })
}

fn dedicated_worker_policy_context_from_headers(
    headers: &[(String, String)],
    creator_policy_context: SubresourcePolicyContext,
) -> SubresourcePolicyContext {
    SubresourcePolicyContext {
        cross_origin_embedder_policy:
            crate::cross_origin_isolation::cross_origin_embedder_policy_from_headers(headers),
        // Chromium's DedicatedWorkerHost currently keeps Document-Isolation-Policy
        // aligned with the creator while deriving the worker COEP from the script response.
        document_isolation_policy: creator_policy_context.document_isolation_policy,
        cross_origin_isolated: creator_policy_context.cross_origin_isolated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedicated_worker_policy_context_uses_response_coep_and_creator_dip() {
        let creator_policy_context = SubresourcePolicyContext {
            document_isolation_policy:
                crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp,
            cross_origin_isolated: true,
            ..Default::default()
        };
        let headers = vec![
            (
                "Cross-Origin-Embedder-Policy".to_owned(),
                "credentialless".to_owned(),
            ),
            (
                "Document-Isolation-Policy".to_owned(),
                "isolate-and-credentialless".to_owned(),
            ),
        ];

        let policy_context =
            dedicated_worker_policy_context_from_headers(&headers, creator_policy_context);

        assert_eq!(
            policy_context.cross_origin_embedder_policy,
            crate::cross_origin_isolation::CrossOriginEmbedderPolicy::Credentialless
        );
        assert_eq!(
            policy_context.document_isolation_policy,
            crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndRequireCorp
        );
        assert!(policy_context.cross_origin_isolated);
    }
}
