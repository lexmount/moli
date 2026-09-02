use super::{JsContextHost, OwnerDispatchScope};
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease};
use crate::types::DedicatedWorkerId;
use crate::{
    module_runtime::{
        ModuleAttributesKey, ModuleMapKey, ModuleSource, PendingDynamicModuleImport,
        WasmModuleRecord,
    },
    page_task_queue::RendererResourceCompletionSender,
    renderer_resource_scheduler::RendererResourceScheduler,
    types::{
        InFlightWorkerSubresourceFetchState, NetworkBodySourceId, PendingSubresourceAuthInfo,
        PendingSubresourceAuthState, PendingSubresourceContinuation,
        PendingSubresourceContinueEvent, PendingSubresourceExecutionContext,
        PendingSubresourceFetchInfo, PendingSubresourceFetchState, PendingSubresourceResponseInfo,
        PendingSubresourceResponseState, PendingWebSocketConnection, PendingWebSocketResponseState,
        RunningSubresourceFetchState, ScriptNetworkOutput, ScriptNetworkOutputItem,
        StreamingSubresourceFetchState, SubresourceBodyFinished, SubresourceNetworkRecord,
        SubresourceNetworkRequestHandle, SubresourceRequestInitiatorType,
        SubresourceRequestStarted, SubresourceResourceType, SubresourceResponseBody,
        SubresourceResponseStarted,
    },
};
use moli_shared_worker::SharedWorkerInstanceId;

enum ImageSubresourceFetchRegistration {
    Intercepted,
    Dispatched(moli_fetch::FetchCancelHandle),
}

fn navigation_response_from_subresource_body(
    body: &SubresourceResponseBody,
    head: moli_fetch::ResponseHead,
) -> crate::types::NavigationResponse {
    let bytes = body.materialize_bytes().unwrap_or_default();
    crate::types::NavigationResponse::from_head_and_materialized_body(
        head,
        moli_fetch::ResponseBody::materialized_bytes(bytes),
    )
}

impl JsContextHost {
    #[cfg(test)]
    pub(crate) fn has_pending_load_event_delaying_subresource_requests(&self) -> bool {
        self.pending_subresource_fetches
            .values()
            .any(|pending| pending.continuation.delays_document_load_event())
    }

    fn push_pending_subresource_fetch_info(&mut self, info: PendingSubresourceFetchInfo) {
        if let Some(source_document) = self.root_document_lifecycle_identity()
            && self.append_live_turn_owner_action(
                crate::runtime::RendererOwnerAction::SubresourceFetchPause {
                    source_document,
                    info: Box::new(info.clone()),
                },
            )
        {
            return;
        }
        #[cfg(test)]
        self.pending_subresource_fetch_infos.push(info);
        #[cfg(not(test))]
        {
            let _ = info;
            panic!("a production Fetch pause must have a concrete renderer output sink");
        }
    }

    pub(crate) fn document_url(&self) -> &url::Url {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_url()
    }

    pub(crate) fn document_character_set(&self) -> &str {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_character_set()
    }

    pub(crate) fn resolve_module_specifier_with_base(
        &mut self,
        specifier: &str,
        base_url: &url::Url,
    ) -> std::result::Result<url::Url, String> {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &mut *self.runtime }.resolve_module_specifier(specifier, base_url)
    }

    pub(crate) fn native_module_source_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<(ModuleMapKey, ModuleSource)> {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.native_module_source_for(module)
    }

    pub(crate) fn native_module_wasm_record_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<WasmModuleRecord> {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.native_module_wasm_record_for(module)
    }

    pub(crate) fn native_wasm_instance_for_namespace<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        namespace: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.native_wasm_instance_for_namespace(scope, namespace)
    }

    pub(crate) fn native_resolved_dependency_module_for(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<v8::Global<v8::Module>> {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }
            .native_resolved_dependency_module_for(referrer, specifier, attributes)
    }

    pub(crate) fn native_document_modulator_ptr(
        &self,
    ) -> *const crate::module_runtime::NativeDocumentModulator {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.native_document_modulator_ptr()
    }

    pub(crate) fn queue_native_dynamic_module_import(
        &mut self,
        request: PendingDynamicModuleImport,
    ) {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &mut *self.runtime }.queue_native_dynamic_module_import(request);
    }

    pub(crate) fn current_dynamic_module_import_owner(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: Option<crate::document_runtime::DomHandle>,
    ) -> Option<crate::module_runtime::DynamicModuleImportOwner> {
        let dispatch_scope = child_handle.map_or(
            super::OwnerDispatchScope::Top,
            super::OwnerDispatchScope::Child,
        );
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        if execution_context.dispatch_scope() != dispatch_scope {
            return None;
        }
        let Some(child_handle) = child_handle else {
            let task_owner = self.current_main_document_task_owner()?;
            return Some(crate::module_runtime::DynamicModuleImportOwner::main(
                task_owner,
                execution_context,
            ));
        };
        let task_owner = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)?;
        let realm_id = self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(task_owner)?;
        Some(crate::module_runtime::DynamicModuleImportOwner::child(
            child_handle,
            task_owner,
            realm_id,
            execution_context,
        ))
    }

    pub(crate) fn dynamic_module_import_owner_is_current(
        &self,
        owner: crate::module_runtime::DynamicModuleImportOwner,
    ) -> bool {
        self.window_execution_context_identity_is_current(owner.execution_context())
    }

    pub(crate) fn record_subresource_network(&mut self, record: SubresourceNetworkRecord) {
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            record,
        )));
        self.note_subresource_activity();
    }

    pub(crate) fn next_subresource_network_request_handle(
        &mut self,
    ) -> SubresourceNetworkRequestHandle {
        let handle =
            SubresourceNetworkRequestHandle::new(self.next_subresource_network_request_handle);
        self.next_subresource_network_request_handle = self
            .next_subresource_network_request_handle
            .wrapping_add(1)
            .max(1);
        handle
    }

    pub(crate) fn record_subresource_request_started(
        &mut self,
        request: SubresourceRequestStarted,
    ) {
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceRequestStarted(
            Box::new(request),
        ));
        self.note_subresource_activity();
    }

    pub(crate) fn record_subresource_response_started(
        &mut self,
        response: SubresourceResponseStarted,
    ) {
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceResponseStarted(
            Box::new(response),
        ));
        self.note_subresource_activity();
    }

    pub(crate) fn record_subresource_data_received(
        &mut self,
        data: crate::types::SubresourceDataReceived,
    ) {
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceDataReceived(data));
        self.note_subresource_activity();
    }

    pub(crate) fn record_subresource_event_source_message_received(
        &mut self,
        message: crate::types::SubresourceEventSourceMessageReceived,
    ) {
        self.push_network_output_item(
            ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(Box::new(message)),
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_subresource_body_finished(&mut self, body: SubresourceBodyFinished) {
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(
            body,
        )));
        self.note_subresource_activity();
    }

    pub(crate) fn push_network_output_item(&mut self, item: ScriptNetworkOutputItem) {
        if let Some(source_document) = self.root_document_lifecycle_identity() {
            self.append_live_turn_observation(
                crate::runtime::RendererProtocolObservation::Network {
                    source_document,
                    item: item.clone(),
                },
            );
        }
        // The page report remains authoritative diagnostic state used by CLI
        // and benchmark consumers. Protocol live delivery is owned solely by
        // the concrete output record above and no longer rediscovers this
        // item from the accumulated report.
        self.pending_network_output.push(item);
    }

    pub(crate) fn record_get_subresource_network_result(
        &mut self,
        frame_id: Option<String>,
        document_url: url::Url,
        request_url: url::Url,
        resource_type: SubresourceResourceType,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        self.record_get_subresource_network_result_with_initiator(
            frame_id,
            document_url,
            request_url,
            resource_type,
            SubresourceRequestInitiatorType::Script,
            result,
        );
    }

    pub(crate) fn record_get_subresource_network_result_with_initiator(
        &mut self,
        frame_id: Option<String>,
        document_url: url::Url,
        request_url: url::Url,
        resource_type: SubresourceResourceType,
        request_initiator_type: SubresourceRequestInitiatorType,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        let record = Self::get_subresource_network_record_with_initiator(
            frame_id,
            document_url,
            request_url,
            resource_type,
            request_initiator_type,
            result,
        );
        self.record_subresource_network(record);
    }

    /// Publish a completed request from a retired Document without treating it
    /// as activity of the currently installed Document.
    pub(crate) fn record_historical_get_subresource_network_result_with_initiator(
        &mut self,
        frame_id: Option<String>,
        document_url: url::Url,
        request_url: url::Url,
        resource_type: SubresourceResourceType,
        request_initiator_type: SubresourceRequestInitiatorType,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        let record = Self::get_subresource_network_record_with_initiator(
            frame_id,
            document_url,
            request_url,
            resource_type,
            request_initiator_type,
            result,
        );
        self.push_network_output_item(ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            record,
        )));
    }

    fn get_subresource_network_record_with_initiator(
        frame_id: Option<String>,
        document_url: url::Url,
        request_url: url::Url,
        resource_type: SubresourceResourceType,
        request_initiator_type: SubresourceRequestInitiatorType,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> SubresourceNetworkRecord {
        match result {
            Ok(response) => SubresourceNetworkRecord::success_with_body(
                frame_id,
                document_url,
                request_url,
                "GET".to_owned(),
                Vec::new(),
                None,
                resource_type,
                response.request_cookie_report.clone(),
                response.redirect_chain.clone().into_iter().collect(),
                response.final_url.clone(),
                response.status,
                response.headers.clone(),
                SubresourceResponseBody::from_navigation_response(response),
                response.cookie_set_reports.clone(),
            )
            .with_request_initiator_type(request_initiator_type)
            .with_from_cache(response.from_cache)
            .with_negotiated_http_version(response.negotiated_http_version)
            .with_network_request_headers(
                response
                    .network_request_headers()
                    .map(|headers| headers.to_vec()),
            ),
            Err(error_text) => SubresourceNetworkRecord::failure(
                frame_id,
                document_url,
                request_url,
                "GET".to_owned(),
                Vec::new(),
                None,
                resource_type,
                error_text.clone(),
            )
            .with_request_initiator_type(request_initiator_type),
        }
    }

    #[cfg(test)]
    pub(crate) fn record_staged_get_subresource_network_result_with_initiator(
        &mut self,
        frame_id: Option<String>,
        document_url: url::Url,
        request_url: url::Url,
        resource_type: SubresourceResourceType,
        request_initiator_type: SubresourceRequestInitiatorType,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        let handle = self.next_subresource_network_request_handle();
        self.record_subresource_request_started(SubresourceRequestStarted::new(
            handle,
            frame_id,
            document_url,
            request_url,
            "GET".to_owned(),
            Vec::new(),
            None,
            resource_type,
            request_initiator_type,
            None,
        ));
        match result {
            Ok(response) => {
                self.record_subresource_response_started(
                    SubresourceResponseStarted::new(
                        handle,
                        response.redirect_chain.clone().into_iter().collect(),
                        response.final_url.clone(),
                        response.status,
                        response.headers.clone(),
                        response.cookie_set_reports.clone(),
                    )
                    .with_from_cache(response.from_cache)
                    .with_negotiated_http_version(response.negotiated_http_version)
                    .with_network_request_headers(
                        response
                            .network_request_headers()
                            .map(|headers| headers.to_vec()),
                    ),
                );
                self.record_subresource_body_finished(SubresourceBodyFinished::ready(
                    handle,
                    SubresourceResponseBody::from_navigation_response(response),
                ));
            }
            Err(error_text) => {
                self.record_subresource_body_finished(SubresourceBodyFinished::failed(
                    handle,
                    error_text.clone(),
                ));
            }
        }
    }

    pub(crate) fn record_css_module_text_for_url(&mut self, url: &url::Url, css_text: String) {
        self.css_module_texts_by_url
            .insert(url.as_str().to_owned(), css_text);
    }

    pub(crate) fn record_css_module_failure_for_url(&mut self, url: &url::Url) {
        if !self.css_module_texts_by_url.contains_key(url.as_str()) {
            self.css_module_failed_urls.insert(url.as_str().to_owned());
        }
    }

    pub(crate) fn css_module_text_for_url(&self, url: &url::Url) -> Option<String> {
        self.css_module_texts_by_url.get(url.as_str()).cloned()
    }

    pub(crate) fn css_module_failed_for_url(&self, url: &url::Url) -> bool {
        self.css_module_failed_urls.contains(url.as_str())
    }

    pub(crate) fn take_network_output(&mut self) -> ScriptNetworkOutput {
        ScriptNetworkOutput::from_items(std::mem::take(&mut self.pending_network_output))
    }

    pub(crate) fn subresource_activity_epoch(&self) -> u64 {
        self.subresource_activity_epoch
    }

    fn assign_pending_subresource_fetch_identity(
        &mut self,
        info: &mut PendingSubresourceFetchInfo,
    ) {
        self.next_pending_subresource_fetch_id += 1;
        info.internal_id = self.next_pending_subresource_fetch_id;
        if info.network_request_handle.is_none() {
            info.network_request_handle = Some(self.next_subresource_network_request_handle());
        }
    }

    fn register_document_resource_load(
        &self,
        owner: crate::native_bridge::WindowDocumentOwner,
        resource_type: SubresourceResourceType,
        disposition: ResourceLoadDisposition,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) -> Option<ResourceLoadLease> {
        self.document_resource_loader_for_window_owner(owner)?
            .register_load(resource_type.into(), disposition, cancel_handle)
    }

    fn register_document_resource_load_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
        resource_type: SubresourceResourceType,
        disposition: ResourceLoadDisposition,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) -> Option<ResourceLoadLease> {
        let owner = self
            .current_window_document_task_target_for_dispatch_scope(dispatch_scope)?
            .owner();
        self.register_document_resource_load(owner, resource_type, disposition, cancel_handle)
    }

    fn require_document_resource_load_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
        resource_type: SubresourceResourceType,
        disposition: ResourceLoadDisposition,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) -> ResourceLoadLease {
        self.register_document_resource_load_for_dispatch_scope(
            dispatch_scope,
            resource_type,
            disposition,
            cancel_handle,
        )
        .unwrap_or_else(|| {
            panic!(
                "active {dispatch_scope:?} Document authority required for {resource_type:?} load"
            )
        })
    }

    fn record_pending_subresource_request_started(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        disposition: ResourceLoadDisposition,
    ) {
        self.record_pending_subresource_request_started_with_initiator(
            info,
            SubresourceRequestInitiatorType::Script,
            disposition,
        );
    }

    fn record_pending_subresource_request_started_with_initiator(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        initiator_type: SubresourceRequestInitiatorType,
        disposition: ResourceLoadDisposition,
    ) {
        let Some(handle) = info.network_request_handle else {
            return;
        };
        self.record_subresource_request_started(
            SubresourceRequestStarted::new(
                handle,
                info.frame_id.clone(),
                info.document_url.clone(),
                info.url.clone(),
                info.method.clone(),
                info.request_headers.clone(),
                info.request_body.clone(),
                info.resource_type,
                initiator_type,
                info.request_cookie_report.clone(),
            )
            .with_request_body_bytes(info.request_body_bytes.clone())
            .with_keepalive(disposition == ResourceLoadDisposition::Keepalive),
        );
    }

    pub(crate) fn record_deferred_pending_subresource_request_started(
        &mut self,
        pending: &mut PendingSubresourceFetchState,
    ) {
        if !pending.deferred_request_started {
            return;
        }
        self.record_pending_subresource_request_started(&pending.info, pending.load.disposition());
        pending.deferred_request_started = false;
    }

    pub(crate) fn record_pending_subresource_fetch(
        &mut self,
        fetch_context: super::WindowFetchContext,
        resolver: v8::Global<v8::PromiseResolver>,
        keepalive: bool,
        connect_policy: crate::document_runtime::DocumentConnectPolicySnapshot,
        csp_report_context: crate::network_host::WindowCspReportRequestContext,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let disposition = if keepalive {
            ResourceLoadDisposition::Keepalive
        } else {
            ResourceLoadDisposition::Ordinary
        };
        let load = self.require_document_resource_load_for_dispatch_scope(
            fetch_context.request_target().dispatch_scope(),
            info.resource_type,
            disposition,
            None,
        );
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window_fetch(fetch_context),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::Fetch(
                    crate::types::PendingWindowFetchContinuation::new(
                        resolver,
                        keepalive,
                        connect_policy,
                        csp_report_context,
                    ),
                ),
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_pending_worker_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        worker_id: DedicatedWorkerId,
        fetch_id: u32,
        load: ResourceLoadLease,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(
                    OwnerDispatchScope::Top,
                    context,
                ),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::WorkerFetch {
                    worker_id,
                    fetch_id,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_pending_worker_subresource_xhr(
        &mut self,
        context: v8::Global<v8::Context>,
        worker_id: DedicatedWorkerId,
        xhr_id: u32,
        load: ResourceLoadLease,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(
                    OwnerDispatchScope::Top,
                    context,
                ),
                credentials_mode,
                request_mode: moli_fetch::RequestMode::Cors,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::WorkerXhr { worker_id, xhr_id },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_pending_worker_subresource_csp_report(
        &mut self,
        context: v8::Global<v8::Context>,
        worker_id: DedicatedWorkerId,
        report_id: u32,
        load: ResourceLoadLease,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(
                    OwnerDispatchScope::Top,
                    context,
                ),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::WorkerCspReport {
                    worker_id,
                    report_id,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_pending_shared_worker_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        instance_id: SharedWorkerInstanceId,
        fetch_id: u32,
        load: ResourceLoadLease,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(
                    OwnerDispatchScope::Top,
                    context,
                ),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::SharedWorkerFetch {
                    instance_id,
                    fetch_id,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_pending_shared_worker_subresource_xhr(
        &mut self,
        context: v8::Global<v8::Context>,
        instance_id: SharedWorkerInstanceId,
        xhr_id: u32,
        load: ResourceLoadLease,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(
                    OwnerDispatchScope::Top,
                    context,
                ),
                credentials_mode,
                request_mode: moli_fetch::RequestMode::Cors,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::SharedWorkerXhr {
                    instance_id,
                    xhr_id,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_pending_shared_worker_subresource_csp_report(
        &mut self,
        context: v8::Global<v8::Context>,
        instance_id: SharedWorkerInstanceId,
        report_id: u32,
        load: ResourceLoadLease,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(
                    OwnerDispatchScope::Top,
                    context,
                ),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::SharedWorkerCspReport {
                    instance_id,
                    report_id,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
    }

    pub(crate) fn record_async_subresource_fetch(
        &mut self,
        fetch_context: super::WindowFetchContext,
        resolver: v8::Global<v8::PromiseResolver>,
        keepalive: bool,
        connect_policy: crate::document_runtime::DocumentConnectPolicySnapshot,
        csp_report_context: crate::network_host::WindowCspReportRequestContext,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
        defer_request_started: bool,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let disposition = if keepalive {
            ResourceLoadDisposition::Keepalive
        } else {
            ResourceLoadDisposition::Ordinary
        };
        let load = self.require_document_resource_load_for_dispatch_scope(
            fetch_context.request_target().dispatch_scope(),
            info.resource_type,
            disposition,
            cancel_handle.clone(),
        );
        if !defer_request_started {
            self.record_pending_subresource_request_started(&info, load.disposition());
        }
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window_fetch(fetch_context),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::Fetch(
                    crate::types::PendingWindowFetchContinuation::new(
                        resolver,
                        keepalive,
                        connect_policy,
                        csp_report_context,
                    ),
                ),
                deferred_request_started: defer_request_started,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_async_subresource_event_source(
        &mut self,
        execution_context: super::WindowExecutionContextBinding,
        resource_loader: &crate::network::context::DocumentResourceLoader,
        event_source: v8::Global<v8::Object>,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
        request_started: bool,
    ) -> (u64, ResourceLoadLease) {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let load = resource_loader
            .register_load(
                ResourceLoadKind::EventSource,
                ResourceLoadDisposition::Ordinary,
                cancel_handle,
            )
            .expect("active Document authority required for EventSource load");
        if request_started {
            self.record_pending_subresource_request_started(&info, load.disposition());
        } else {
            self.push_pending_subresource_fetch_info(info.clone());
        }
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load: load.clone(),
                execution_context: PendingSubresourceExecutionContext::window(execution_context),
                credentials_mode,
                request_mode: moli_fetch::RequestMode::Cors,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::EventSource(event_source),
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        (internal_id, load)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_async_media_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        media_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::MediaLoadSequenceId,
        owner: OwnerDispatchScope,
        cancel_handle: moli_fetch::FetchCancelHandle,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) -> Option<u64> {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        if !self.bind_pending_media_load_network_request_if_matches(
            media_handle,
            sequence,
            internal_id,
        ) {
            return None;
        }
        let load = self.require_document_resource_load_for_dispatch_scope(
            owner,
            info.resource_type,
            ResourceLoadDisposition::Ordinary,
            Some(cancel_handle.clone()),
        );
        self.record_pending_subresource_request_started_with_initiator(
            &info,
            SubresourceRequestInitiatorType::Other,
            load.disposition(),
        );
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(owner, context),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::Media {
                    media_handle,
                    sequence,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        Some(internal_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_image_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        image_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::ImageLoadEventId,
        owner: OwnerDispatchScope,
        request_initiator_type: SubresourceRequestInitiatorType,
        registration: ImageSubresourceFetchRegistration,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) -> Option<u64> {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        if !self.bind_pending_image_load_network_request_if_matches(
            image_handle,
            sequence,
            internal_id,
        ) {
            return None;
        }
        let registry_cancel_handle = match &registration {
            ImageSubresourceFetchRegistration::Intercepted => None,
            ImageSubresourceFetchRegistration::Dispatched(cancel_handle) => {
                Some(cancel_handle.clone())
            }
        };
        let load = self.require_document_resource_load_for_dispatch_scope(
            owner,
            info.resource_type,
            ResourceLoadDisposition::Ordinary,
            registry_cancel_handle,
        );
        match registration {
            ImageSubresourceFetchRegistration::Intercepted => {
                self.push_pending_subresource_fetch_info(info.clone());
            }
            ImageSubresourceFetchRegistration::Dispatched(_) => {
                self.record_pending_subresource_request_started_with_initiator(
                    &info,
                    request_initiator_type,
                    load.disposition(),
                );
            }
        }
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(owner, context),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::Image {
                    image_handle,
                    sequence,
                    request_initiator_type,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        Some(internal_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_intercepted_image_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        image_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::ImageLoadEventId,
        owner: OwnerDispatchScope,
        request_initiator_type: SubresourceRequestInitiatorType,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        info: PendingSubresourceFetchInfo,
    ) -> Option<u64> {
        self.record_image_subresource_fetch(
            context,
            image_handle,
            sequence,
            owner,
            request_initiator_type,
            ImageSubresourceFetchRegistration::Intercepted,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            info,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_async_image_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        image_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::ImageLoadEventId,
        owner: OwnerDispatchScope,
        request_initiator_type: SubresourceRequestInitiatorType,
        cancel_handle: moli_fetch::FetchCancelHandle,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        info: PendingSubresourceFetchInfo,
    ) -> Option<u64> {
        self.record_image_subresource_fetch(
            context,
            image_handle,
            sequence,
            owner,
            request_initiator_type,
            ImageSubresourceFetchRegistration::Dispatched(cancel_handle),
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            info,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_async_text_track_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        track_handle: crate::document_runtime::DomHandle,
        sequence: crate::native_bridge::TextTrackLoadSequenceId,
        owner: OwnerDispatchScope,
        cancel_handle: moli_fetch::FetchCancelHandle,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) -> Option<u64> {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        if !self.bind_pending_text_track_network_request_if_matches(
            track_handle,
            sequence,
            internal_id,
        ) {
            return None;
        }
        let load = self.require_document_resource_load_for_dispatch_scope(
            owner,
            info.resource_type,
            ResourceLoadDisposition::Ordinary,
            Some(cancel_handle.clone()),
        );
        self.record_pending_subresource_request_started_with_initiator(
            &info,
            SubresourceRequestInitiatorType::Other,
            load.disposition(),
        );
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(owner, context),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::TextTrack {
                    track_handle,
                    sequence,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        Some(internal_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_async_stylesheet_subresource_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
        owner: OwnerDispatchScope,
        cancel_handle: moli_fetch::FetchCancelHandle,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        request_mode: moli_fetch::RequestMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        web_font: Option<crate::css_resource_urls::StylesheetWebFont>,
        css_image: Option<crate::native_bridge::CssImageResourceRequestIdentity>,
        mut info: PendingSubresourceFetchInfo,
    ) -> Option<u64> {
        if !self.stylesheet_subresource_load_delay_is_current(binding) {
            return None;
        }
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let load = self
            .register_document_resource_load(
                crate::native_bridge::WindowDocumentOwner::Frame(binding.owner()),
                info.resource_type,
                ResourceLoadDisposition::Ordinary,
                Some(cancel_handle.clone()),
            )
            .expect("current stylesheet owner must have a Document resource authority");
        self.record_pending_subresource_request_started_with_initiator(
            &info,
            SubresourceRequestInitiatorType::Css,
            load.disposition(),
        );
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(owner, context),
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::StylesheetSubresource {
                    binding,
                    web_font,
                    css_image,
                },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        Some(internal_id)
    }

    pub(crate) fn record_async_subresource_beacon(
        &mut self,
        execution_context: super::WindowExecutionContextIdentity,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let load = self.require_document_resource_load_for_dispatch_scope(
            execution_context.dispatch_scope(),
            info.resource_type,
            ResourceLoadDisposition::Keepalive,
            cancel_handle.clone(),
        );
        self.record_pending_subresource_request_started(&info, load.disposition());
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window_network_only(
                    execution_context,
                ),
                credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                request_mode: moli_fetch::RequestMode::NoCors,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::Beacon,
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    pub(crate) fn record_async_subresource_csp_report(
        &mut self,
        identity: super::WindowDocumentNetworkRequestIdentity,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
        load: ResourceLoadLease,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        debug_assert_eq!(
            load.kind(),
            crate::network::loads::ResourceLoadKind::CspReport
        );
        debug_assert_eq!(load.disposition(), ResourceLoadDisposition::Keepalive);
        self.record_pending_subresource_request_started(&info, load.disposition());
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window_document_network_only(
                    identity,
                ),
                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                request_mode: moli_fetch::RequestMode::NoCors,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::CspReport { client_id },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    pub(crate) fn record_pending_subresource_beacon(
        &mut self,
        execution_context: super::WindowExecutionContextIdentity,
        network_partition_key: Option<String>,
        mut info: PendingSubresourceFetchInfo,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let load = self.require_document_resource_load_for_dispatch_scope(
            execution_context.dispatch_scope(),
            info.resource_type,
            ResourceLoadDisposition::Keepalive,
            None,
        );
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window_network_only(
                    execution_context,
                ),
                credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                request_mode: moli_fetch::RequestMode::NoCors,
                network_partition_key,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::Beacon,
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    pub(crate) fn record_pending_subresource_csp_report(
        &mut self,
        identity: super::WindowDocumentNetworkRequestIdentity,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
        load: ResourceLoadLease,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        debug_assert_eq!(
            load.kind(),
            crate::network::loads::ResourceLoadKind::CspReport
        );
        debug_assert_eq!(load.disposition(), ResourceLoadDisposition::Keepalive);
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window_document_network_only(
                    identity,
                ),
                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                request_mode: moli_fetch::RequestMode::NoCors,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::CspReport { client_id },
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    pub(crate) fn record_pending_subresource_xhr(
        &mut self,
        execution_context: super::WindowExecutionContextBinding,
        xhr: v8::Global<v8::Object>,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let load = self.require_document_resource_load_for_dispatch_scope(
            execution_context.dispatch_scope(),
            info.resource_type,
            ResourceLoadDisposition::Ordinary,
            None,
        );
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window(execution_context),
                credentials_mode,
                request_mode: moli_fetch::RequestMode::Cors,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::Xhr(xhr),
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    pub(crate) fn record_pending_websocket_fetch(
        &mut self,
        context: v8::Global<v8::Context>,
        owner: OwnerDispatchScope,
        connection: PendingWebSocketConnection,
        mut info: PendingSubresourceFetchInfo,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let load = self.require_document_resource_load_for_dispatch_scope(
            owner,
            info.resource_type,
            ResourceLoadDisposition::Ordinary,
            None,
        );
        self.push_pending_subresource_fetch_info(info.clone());
        self.pending_subresource_fetches.insert(
            info.internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::adapter(owner, context),
                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                request_mode: moli_fetch::RequestMode::Cors,
                network_partition_key: None,
                policy_context: Default::default(),
                continuation: PendingSubresourceContinuation::WebSocket(connection),
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    pub(crate) fn record_async_subresource_xhr(
        &mut self,
        execution_context: super::WindowExecutionContextBinding,
        xhr: v8::Global<v8::Object>,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        network_partition_key: Option<String>,
        policy_context: crate::types::SubresourcePolicyContext,
        mut info: PendingSubresourceFetchInfo,
    ) -> u64 {
        self.assign_pending_subresource_fetch_identity(&mut info);
        let internal_id = info.internal_id;
        let load = self.require_document_resource_load_for_dispatch_scope(
            execution_context.dispatch_scope(),
            info.resource_type,
            ResourceLoadDisposition::Ordinary,
            cancel_handle.clone(),
        );
        self.record_pending_subresource_request_started(&info, load.disposition());
        self.pending_subresource_fetches.insert(
            internal_id,
            PendingSubresourceFetchState {
                info,
                load,
                execution_context: PendingSubresourceExecutionContext::window(execution_context),
                credentials_mode,
                request_mode: moli_fetch::RequestMode::Cors,
                network_partition_key,
                policy_context,
                continuation: PendingSubresourceContinuation::Xhr(xhr),
                deferred_request_started: false,
            },
        );
        self.note_subresource_activity();
        internal_id
    }

    pub(crate) fn resource_completion_sender(&self) -> RendererResourceCompletionSender {
        self.resource_completion_tx.clone()
    }

    pub(crate) fn resource_scheduler(&self) -> RendererResourceScheduler {
        self.resource_scheduler.clone()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_subresource_fetch_infos(
        &mut self,
    ) -> Vec<PendingSubresourceFetchInfo> {
        std::mem::take(&mut self.pending_subresource_fetch_infos)
    }

    pub(crate) fn pending_subresource_fetch_info_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_subresource_fetch_infos.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }

    /// Observes whether the exact phase-local target of a queued networking
    /// event is still resident. This method never claims or advances work.
    pub(crate) fn async_subresource_fetch_event_target_is_current(
        &self,
        target: crate::types::AsyncSubresourceFetchEventTarget,
    ) -> bool {
        use crate::types::AsyncSubresourceFetchEventTarget;

        match target {
            AsyncSubresourceFetchEventTarget::Completion { internal_id } => {
                self.pending_subresource_fetches.contains_key(&internal_id)
                    || self.running_subresource_fetches.contains_key(&internal_id)
            }
            AsyncSubresourceFetchEventTarget::StreamingStart {
                internal_id,
                body_source_id: _,
            } => self.pending_subresource_fetches.contains_key(&internal_id),
            AsyncSubresourceFetchEventTarget::StreamingChunk { body_source_id } => self
                .streaming_subresource_fetches
                .values()
                .any(|state| state.body_source_id == body_source_id),
            AsyncSubresourceFetchEventTarget::StreamingFinish {
                internal_id,
                body_source_id,
            } => self
                .streaming_subresource_fetches
                .get(&internal_id)
                .is_some_and(|state| state.body_source_id == body_source_id),
            AsyncSubresourceFetchEventTarget::ObservedNetworkRecord => true,
        }
    }

    pub(crate) fn take_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
    ) -> Option<PendingSubresourceFetchState> {
        self.pending_subresource_fetches.remove(&internal_id)
    }

    pub(crate) fn restore_pending_subresource_fetch(
        &mut self,
        state: PendingSubresourceFetchState,
    ) {
        self.pending_subresource_fetches
            .insert(state.info.internal_id, state);
        self.note_subresource_activity();
    }

    pub(crate) fn cancel_pending_worker_subresource_fetch(
        &mut self,
        worker_id: DedicatedWorkerId,
        fetch_id: u32,
        error_text: String,
    ) -> bool {
        let Some(internal_id) = self.pending_subresource_fetches.iter().find_map(
            |(internal_id, pending)| match &pending.continuation {
                PendingSubresourceContinuation::WorkerFetch {
                    worker_id: pending_worker_id,
                    fetch_id: pending_fetch_id,
                } if *pending_worker_id == worker_id && *pending_fetch_id == fetch_id => {
                    Some(*internal_id)
                }
                PendingSubresourceContinuation::WorkerXhr {
                    worker_id: pending_worker_id,
                    xhr_id: pending_xhr_id,
                } if *pending_worker_id == worker_id && *pending_xhr_id == fetch_id => {
                    Some(*internal_id)
                }
                PendingSubresourceContinuation::WorkerCspReport {
                    worker_id: pending_worker_id,
                    report_id: pending_report_id,
                } if *pending_worker_id == worker_id && *pending_report_id == fetch_id => {
                    Some(*internal_id)
                }
                _ => None,
            },
        ) else {
            return false;
        };
        let Some(pending) = self.pending_subresource_fetches.remove(&internal_id) else {
            return false;
        };
        #[cfg(test)]
        self.pending_subresource_fetch_infos
            .retain(|info| info.internal_id != internal_id);
        self.record_pending_subresource_failure(&pending.info, error_text);
        self.record_pending_subresource_continue_event(
            PendingSubresourceContinueEvent::Completed { internal_id },
        );
        true
    }

    pub(crate) fn cancel_pending_shared_worker_subresource_fetch(
        &mut self,
        instance_id: SharedWorkerInstanceId,
        fetch_id: u32,
        error_text: String,
    ) -> bool {
        let Some(internal_id) = self.pending_subresource_fetches.iter().find_map(
            |(internal_id, pending)| match &pending.continuation {
                PendingSubresourceContinuation::SharedWorkerFetch {
                    instance_id: pending_instance_id,
                    fetch_id: pending_fetch_id,
                } if *pending_instance_id == instance_id && *pending_fetch_id == fetch_id => {
                    Some(*internal_id)
                }
                PendingSubresourceContinuation::SharedWorkerXhr {
                    instance_id: pending_instance_id,
                    xhr_id: pending_xhr_id,
                } if *pending_instance_id == instance_id && *pending_xhr_id == fetch_id => {
                    Some(*internal_id)
                }
                PendingSubresourceContinuation::SharedWorkerCspReport {
                    instance_id: pending_instance_id,
                    report_id: pending_report_id,
                } if *pending_instance_id == instance_id && *pending_report_id == fetch_id => {
                    Some(*internal_id)
                }
                _ => None,
            },
        ) else {
            return false;
        };
        let Some(pending) = self.pending_subresource_fetches.remove(&internal_id) else {
            return false;
        };
        #[cfg(test)]
        self.pending_subresource_fetch_infos
            .retain(|info| info.internal_id != internal_id);
        self.record_pending_subresource_failure(&pending.info, error_text);
        self.record_pending_subresource_continue_event(
            PendingSubresourceContinueEvent::Completed { internal_id },
        );
        true
    }

    fn record_pending_subresource_failure(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        error_text: String,
    ) {
        let mut record = SubresourceNetworkRecord::failure(
            info.frame_id.clone(),
            info.document_url.clone(),
            info.url.clone(),
            info.method.clone(),
            info.request_headers.clone(),
            info.request_body.clone(),
            info.resource_type,
            error_text,
        )
        .with_request_body_bytes(info.request_body_bytes.clone());
        if let Some(handle) = info.network_request_handle {
            record = record.with_request_handle(handle);
        }
        self.record_subresource_network(record);
    }

    pub(crate) fn record_running_subresource_fetch(&mut self, state: RunningSubresourceFetchState) {
        self.running_subresource_fetches
            .insert(state.pending.info.internal_id, state);
        self.note_subresource_activity();
    }

    pub(crate) fn record_streaming_subresource_fetch(
        &mut self,
        state: StreamingSubresourceFetchState,
    ) {
        self.streaming_subresource_fetches
            .insert(state.pending.info.internal_id, state);
        self.note_subresource_activity();
    }

    pub(crate) fn append_streaming_subresource_body(
        &mut self,
        body_source_id: NetworkBodySourceId,
        bytes: &[u8],
    ) {
        let appended = if let Some(state) = self
            .streaming_subresource_fetches
            .values_mut()
            .find(|state| state.body_source_id == body_source_id)
        {
            state.body_writer.append(bytes);
            true
        } else {
            false
        };
        if appended {
            self.note_subresource_activity();
        }
    }

    pub(crate) fn streaming_subresource_is_event_source(
        &self,
        body_source_id: NetworkBodySourceId,
    ) -> bool {
        self.streaming_subresource_fetches
            .values()
            .find(|state| state.body_source_id == body_source_id)
            .is_some_and(|state| state.event_source_parser.is_some())
    }

    pub(crate) fn streaming_subresource_is_xhr(&self, body_source_id: NetworkBodySourceId) -> bool {
        self.streaming_subresource_fetches
            .values()
            .find(|state| state.body_source_id == body_source_id)
            .is_some_and(|state| state.xhr_response.is_some())
    }

    pub(crate) fn append_streaming_event_source_chunk<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        body_source_id: NetworkBodySourceId,
        bytes: &[u8],
    ) -> Option<crate::types::EventSourceStreamingChunkDelivery<'s>> {
        let state = self
            .streaming_subresource_fetches
            .values_mut()
            .find(|state| state.body_source_id == body_source_id)?;
        let PendingSubresourceContinuation::EventSource(event_source) = &state.pending.continuation
        else {
            return None;
        };
        let context = v8::Local::new(scope, state.pending.execution_context.context_global()?);
        let event_source = v8::Local::new(scope, event_source);
        let parser = state.event_source_parser.as_mut()?;
        state.body_writer.append(bytes);
        let messages = parser.push(bytes);
        let delivery = crate::types::EventSourceStreamingChunkDelivery {
            context,
            event_source,
            request_handle: state.pending.info.network_request_handle,
            messages,
        };
        self.note_subresource_activity();
        Some(delivery)
    }

    pub(crate) fn append_streaming_xhr_chunk<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        body_source_id: NetworkBodySourceId,
        bytes: &[u8],
    ) -> Option<crate::types::XhrStreamingChunkDelivery<'s>> {
        let delivery = {
            let state = self
                .streaming_subresource_fetches
                .values_mut()
                .find(|state| state.body_source_id == body_source_id)?;
            let PendingSubresourceContinuation::Xhr(xhr) = &state.pending.continuation else {
                return None;
            };
            let context = v8::Local::new(scope, state.pending.execution_context.context_global()?);
            let xhr = v8::Local::new(scope, xhr);
            state.body_writer.append(bytes);
            let (decoded_text, loaded, total) = state.xhr_response.as_mut()?.append(bytes);
            crate::types::XhrStreamingChunkDelivery {
                context,
                xhr,
                dispatch_scope: state.pending.execution_context.dispatch_scope(),
                realm_token: state.pending.execution_context.realm_token(),
                internal_id: state.pending.info.internal_id,
                request_handle: state.pending.info.network_request_handle,
                decoded_text,
                loaded,
                total,
            }
        };
        self.note_subresource_activity();
        Some(delivery)
    }

    pub(crate) fn streaming_subresource_body_binding_by_body_source_id(
        &self,
        body_source_id: NetworkBodySourceId,
    ) -> Option<(
        *const v8::Global<v8::Context>,
        crate::native_bridge::OwnerDispatchScope,
        Option<crate::native_bridge::RuntimeObservableContextToken>,
    )> {
        let state = self
            .streaming_subresource_fetches
            .values()
            .find(|state| state.body_source_id == body_source_id)?;
        if let Some(target) = state.pending.execution_context.window_request_target()
            && !self
                .window_execution_context_owner_is_current(target.owner(), target.dispatch_scope())
        {
            return None;
        }
        let context = state.pending.execution_context.context_global()?;
        Some((
            context as *const _,
            state.pending.execution_context.dispatch_scope(),
            state.pending.execution_context.realm_token(),
        ))
    }

    pub(crate) fn cancel_streaming_subresource_body_source(
        &mut self,
        body_source_id: NetworkBodySourceId,
    ) -> bool {
        let Some(internal_id) =
            self.streaming_subresource_fetches
                .iter()
                .find_map(|(internal_id, state)| {
                    (state.body_source_id == body_source_id).then_some(*internal_id)
                })
        else {
            return false;
        };
        let streaming = self
            .streaming_subresource_fetches
            .remove(&internal_id)
            .expect("streaming subresource id was selected from the same map");
        streaming.pending.load.cancel();
        self.browser_context_runtime
            .abort_service_worker_fetch(internal_id);
        self.record_streaming_subresource_fetch_failure(
            streaming,
            crate::network_host::ABORTED_ERROR_TEXT.to_owned(),
        );
        self.record_pending_subresource_continue_event(
            PendingSubresourceContinueEvent::Completed { internal_id },
        );
        true
    }

    fn record_streaming_subresource_fetch_failure(
        &mut self,
        streaming: StreamingSubresourceFetchState,
        error_text: String,
    ) {
        if let Some(handle) = streaming.pending.info.network_request_handle {
            let partial_body = streaming.body_writer.finish();
            self.record_subresource_response_started(
                SubresourceResponseStarted::new(
                    handle,
                    streaming
                        .head
                        .redirect_chain
                        .clone()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    streaming.head.final_url.clone(),
                    streaming.head.status,
                    streaming.head.headers.clone(),
                    streaming.head.cookie_set_reports.clone(),
                )
                .with_from_cache(streaming.head.from_cache)
                .with_negotiated_http_version(streaming.head.negotiated_http_version)
                .with_network_request_headers(streaming.network_request_headers.clone()),
            );
            self.record_subresource_body_finished(
                SubresourceBodyFinished::failed_with_partial_body(handle, error_text, partial_body),
            );
        } else {
            self.record_subresource_network(
                SubresourceNetworkRecord::failure(
                    streaming.pending.info.frame_id.clone(),
                    streaming.pending.info.document_url.clone(),
                    streaming.request_url,
                    streaming.request_method,
                    streaming.request_headers,
                    streaming.request_body,
                    streaming.pending.info.resource_type,
                    error_text,
                )
                .with_request_body_bytes(streaming.pending.info.request_body_bytes.clone()),
            );
        }
    }

    pub(crate) fn take_streaming_subresource_fetch(
        &mut self,
        internal_id: u64,
    ) -> Option<StreamingSubresourceFetchState> {
        self.streaming_subresource_fetches.remove(&internal_id)
    }

    pub(crate) fn record_in_flight_worker_subresource_fetch(
        &mut self,
        state: InFlightWorkerSubresourceFetchState,
    ) {
        self.in_flight_worker_subresource_fetches
            .insert(state.pending.info.internal_id, state);
        self.note_subresource_activity();
    }

    pub(crate) fn take_running_subresource_fetch(
        &mut self,
        internal_id: u64,
    ) -> Option<RunningSubresourceFetchState> {
        self.running_subresource_fetches.remove(&internal_id)
    }

    fn record_observable_abort_before_response(&mut self, pending: &PendingSubresourceFetchState) {
        if !matches!(
            &pending.continuation,
            PendingSubresourceContinuation::EventSource(_) | PendingSubresourceContinuation::Xhr(_)
        ) {
            return;
        }
        let error_text = crate::network_host::ABORTED_ERROR_TEXT.to_owned();
        if let Some(handle) = pending.info.network_request_handle {
            self.record_subresource_body_finished(SubresourceBodyFinished::failed(
                handle, error_text,
            ));
        } else {
            self.record_subresource_network(
                SubresourceNetworkRecord::failure(
                    pending.info.frame_id.clone(),
                    pending.info.document_url.clone(),
                    pending.info.url.clone(),
                    pending.info.method.clone(),
                    pending.info.request_headers.clone(),
                    pending.info.request_body.clone(),
                    pending.info.resource_type,
                    error_text,
                )
                .with_request_body_bytes(pending.info.request_body_bytes.clone()),
            );
        }
    }

    fn record_observable_abort_after_response(
        &mut self,
        pending: &PendingSubresourceResponseState,
    ) {
        if !matches!(
            &pending.pending.continuation,
            PendingSubresourceContinuation::EventSource(_) | PendingSubresourceContinuation::Xhr(_)
        ) {
            return;
        }
        let error_text = crate::network_host::ABORTED_ERROR_TEXT.to_owned();
        if let Some(handle) = pending.pending.info.network_request_handle {
            self.record_subresource_response_started(
                SubresourceResponseStarted::new(
                    handle,
                    pending.response.redirect_chain.clone(),
                    pending.response.final_url.clone(),
                    pending.response.status,
                    pending.response.headers.clone(),
                    pending.response.cookie_set_reports.clone(),
                )
                .with_from_cache(pending.response.from_cache)
                .with_negotiated_http_version(pending.response.negotiated_http_version)
                .with_network_request_headers(
                    pending
                        .response
                        .network_request_headers()
                        .map(|headers| headers.to_vec()),
                ),
            );
            self.record_subresource_body_finished(
                SubresourceBodyFinished::failed_with_partial_body(
                    handle,
                    error_text,
                    SubresourceResponseBody::from_navigation_response(&pending.response),
                ),
            );
        } else {
            self.record_subresource_network(
                SubresourceNetworkRecord::failure(
                    pending.pending.info.frame_id.clone(),
                    pending.pending.info.document_url.clone(),
                    pending.request_url.clone(),
                    pending.request_method.clone(),
                    pending.request_headers.clone(),
                    pending.request_body.clone(),
                    pending.pending.info.resource_type,
                    error_text,
                )
                .with_request_body_bytes(pending.pending.info.request_body_bytes.clone()),
            );
        }
    }

    fn record_observable_stream_abort(&mut self, streaming: StreamingSubresourceFetchState) {
        if !matches!(
            &streaming.pending.continuation,
            PendingSubresourceContinuation::EventSource(_) | PendingSubresourceContinuation::Xhr(_)
        ) {
            return;
        }
        let error_text = crate::network_host::ABORTED_ERROR_TEXT.to_owned();
        if let Some(handle) = streaming.pending.info.network_request_handle {
            self.record_subresource_body_finished(
                SubresourceBodyFinished::failed_with_partial_body(
                    handle,
                    error_text,
                    streaming.body_writer.finish(),
                ),
            );
        } else {
            self.record_subresource_network(
                SubresourceNetworkRecord::failure(
                    streaming.pending.info.frame_id.clone(),
                    streaming.pending.info.document_url.clone(),
                    streaming.request_url,
                    streaming.request_method,
                    streaming.request_headers,
                    streaming.request_body,
                    streaming.pending.info.resource_type,
                    error_text,
                )
                .with_request_body_bytes(streaming.pending.info.request_body_bytes.clone()),
            );
        }
    }

    pub(crate) fn abort_event_source_fetch(&mut self, internal_id: u64) -> bool {
        if self
            .streaming_subresource_fetches
            .get(&internal_id)
            .is_some_and(|streaming| streaming.pending.load.response_completion_is_committed())
        {
            // The network owner has already accepted a complete declared body
            // or a real terminal result. Keep the stream registered so that its
            // queued completion, rather than close(), determines CDP's terminal.
            return true;
        }
        let _ = self
            .browser_context_runtime
            .abort_service_worker_fetch(internal_id);
        self.abort_subresource_fetch(internal_id)
    }

    pub(crate) fn abort_subresource_fetch(&mut self, internal_id: u64) -> bool {
        if let Some(pending) = self.pending_subresource_fetches.remove(&internal_id) {
            #[cfg(test)]
            self.pending_subresource_fetch_infos
                .retain(|info| info.internal_id != internal_id);
            self.record_observable_abort_before_response(&pending);
            pending.load.cancel();
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(in_flight) = self
            .in_flight_worker_subresource_fetches
            .remove(&internal_id)
        {
            self.record_observable_abort_before_response(&in_flight.pending);
            in_flight.pending.load.cancel();
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(running) = self.running_subresource_fetches.remove(&internal_id) {
            self.record_observable_abort_before_response(&running.pending);
            running.pending.load.cancel();
            self.finish_active_subresource_request();
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(streaming) = self.streaming_subresource_fetches.remove(&internal_id) {
            streaming.pending.load.cancel();
            self.record_observable_stream_abort(streaming);
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(pending) = self.pending_subresource_auths.remove(&internal_id) {
            self.record_observable_abort_before_response(&pending.pending);
            pending.pending.load.cancel();
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(pending) = self.pending_subresource_responses.remove(&internal_id) {
            self.record_observable_abort_after_response(&pending);
            pending.pending.load.cancel();
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        false
    }

    fn subresource_fetch_states(&self) -> impl Iterator<Item = &PendingSubresourceFetchState> {
        self.pending_subresource_fetches
            .values()
            .chain(
                self.in_flight_worker_subresource_fetches
                    .values()
                    .map(|state| &state.pending),
            )
            .chain(
                self.running_subresource_fetches
                    .values()
                    .map(|state| &state.pending),
            )
            .chain(
                self.streaming_subresource_fetches
                    .values()
                    .map(|state| &state.pending),
            )
            .chain(
                self.pending_subresource_auths
                    .values()
                    .map(|state| &state.pending),
            )
            .chain(
                self.pending_subresource_responses
                    .values()
                    .map(|state| &state.pending),
            )
    }

    /// Retires JS/DOM consumers selected by one exact resource authority.
    ///
    /// The authority registry owns transport lifetime, while these maps own
    /// V8 and DOM continuation state. A Document transition must update both
    /// halves at the same boundary: ordinary work is removed after transport
    /// cancellation, Window Fetch keepalives drop their JS delivery endpoint,
    /// and already-network-only keepalives remain to publish historical
    /// network observations.
    pub(crate) fn retire_document_resource_load_consumers(
        &mut self,
        registry_id: u64,
    ) -> (usize, usize) {
        let mut ordinary_ids = self
            .subresource_fetch_states()
            .filter(|pending| {
                pending.load.registry_id() == registry_id
                    && pending.load.disposition() == ResourceLoadDisposition::Ordinary
            })
            .map(|pending| pending.info.internal_id)
            .collect::<Vec<_>>();
        ordinary_ids.sort_unstable();
        ordinary_ids.dedup();

        let mut detached_window_fetches = 0;
        self.for_each_subresource_fetch_state_mut(|pending| {
            if pending.load.registry_id() == registry_id
                && pending.load.disposition() == ResourceLoadDisposition::Keepalive
            {
                detached_window_fetches += usize::from(pending.detach_keepalive_window_fetch());
            }
        });

        let mut aborted = 0;
        for internal_id in ordinary_ids.iter().copied() {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            aborted += usize::from(self.abort_subresource_fetch(internal_id));
        }
        #[cfg(test)]
        self.pending_subresource_fetch_infos
            .retain(|info| !ordinary_ids.contains(&info.internal_id));
        (aborted, detached_window_fetches)
    }

    fn retire_window_subresources_matching(
        &mut self,
        matches_continuation: impl Fn(&PendingSubresourceContinuation) -> bool,
        matches_execution_context: impl Fn(&PendingSubresourceExecutionContext) -> bool,
    ) -> usize {
        let mut internal_ids = self
            .subresource_fetch_states()
            .filter(|pending| {
                matches_continuation(&pending.continuation)
                    && matches_execution_context(&pending.execution_context)
            })
            .map(|pending| pending.info.internal_id)
            .collect::<Vec<_>>();
        internal_ids.sort_unstable();
        internal_ids.dedup();

        let mut retired = 0;
        for internal_id in internal_ids.iter().copied() {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            retired += usize::from(self.abort_subresource_fetch(internal_id));
        }
        #[cfg(test)]
        self.pending_subresource_fetch_infos
            .retain(|info| !internal_ids.contains(&info.internal_id));
        retired
    }

    fn for_each_subresource_fetch_state_mut(
        &mut self,
        mut apply: impl FnMut(&mut PendingSubresourceFetchState),
    ) {
        for pending in self.pending_subresource_fetches.values_mut() {
            apply(pending);
        }
        for state in self.in_flight_worker_subresource_fetches.values_mut() {
            apply(&mut state.pending);
        }
        for state in self.running_subresource_fetches.values_mut() {
            apply(&mut state.pending);
        }
        for state in self.streaming_subresource_fetches.values_mut() {
            apply(&mut state.pending);
        }
        for state in self.pending_subresource_auths.values_mut() {
            apply(&mut state.pending);
        }
        for state in self.pending_subresource_responses.values_mut() {
            apply(&mut state.pending);
        }
    }

    fn retire_window_fetches_matching(
        &mut self,
        matches_execution_context: impl Fn(&PendingSubresourceExecutionContext) -> bool,
    ) -> (usize, usize) {
        let mut abort_ids = self
            .subresource_fetch_states()
            .filter(|pending| {
                pending.continuation.is_window_fetch()
                    && !pending.continuation.window_fetch_keepalive()
                    && matches_execution_context(&pending.execution_context)
            })
            .map(|pending| pending.info.internal_id)
            .collect::<Vec<_>>();
        abort_ids.sort_unstable();
        abort_ids.dedup();

        let mut detached = 0;
        self.for_each_subresource_fetch_state_mut(|pending| {
            if pending.continuation.is_window_fetch()
                && pending.continuation.window_fetch_keepalive()
                && matches_execution_context(&pending.execution_context)
            {
                detached += usize::from(pending.detach_keepalive_window_fetch());
            }
        });

        let mut aborted = 0;
        for internal_id in abort_ids.iter().copied() {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            aborted += usize::from(self.abort_subresource_fetch(internal_id));
        }
        #[cfg(test)]
        self.pending_subresource_fetch_infos
            .retain(|info| !abort_ids.contains(&info.internal_id));
        (aborted, detached)
    }

    pub(crate) fn retire_window_fetches_for_execution_context_owner(
        &mut self,
        owner: super::WindowExecutionContextOwner,
    ) -> (usize, usize) {
        // Script realm and request target are derived from the same receiver,
        // but they have different retirement coordinates. LocalWindow
        // retirement follows the target even after a keepalive request drops
        // its V8 Global.
        let retirement = self.retire_window_fetches_matching(|execution_context| {
            execution_context
                .window_request_target()
                .is_some_and(|target| target.owner() == owner)
        });
        if retirement != (0, 0) {
            tracing::debug!(
                ?owner,
                aborted = retirement.0,
                detached_keepalive = retirement.1,
                "retired Window Fetch work with execution context"
            );
        }
        retirement
    }

    pub(crate) fn retire_window_fetches_for_context_token(
        &mut self,
        context_token: super::RuntimeObservableContextToken,
    ) -> (usize, usize) {
        // Realm teardown is the complementary half of request-owner
        // retirement: it invalidates JS delivery by exact token, independently
        // of whether keepalive network work may continue for the LocalWindow.
        let retirement = self.retire_window_fetches_matching(|execution_context| {
            execution_context.realm_token() == Some(context_token)
        });
        if retirement != (0, 0) {
            tracing::debug!(
                ?context_token,
                aborted = retirement.0,
                detached_keepalive = retirement.1,
                "retired Window Fetch work with destroyed V8 execution context"
            );
        }
        retirement
    }

    pub(crate) fn retire_window_xhrs_for_execution_context_owner(
        &mut self,
        owner: super::WindowExecutionContextOwner,
    ) -> usize {
        let retired = self.retire_window_subresources_matching(
            PendingSubresourceContinuation::is_window_xhr,
            |execution_context| execution_context.window_realm_owner() == Some(owner),
        );
        if retired > 0 {
            tracing::debug!(
                ?owner,
                retired,
                "aborted XMLHttpRequests for retired Window execution context"
            );
        }
        retired
    }

    pub(crate) fn retire_window_xhrs_for_context_token(
        &mut self,
        context_token: super::RuntimeObservableContextToken,
    ) -> usize {
        let retired = self.retire_window_subresources_matching(
            PendingSubresourceContinuation::is_window_xhr,
            |execution_context| execution_context.realm_token() == Some(context_token),
        );
        if retired > 0 {
            tracing::debug!(
                ?context_token,
                retired,
                "aborted XMLHttpRequests for destroyed V8 execution context"
            );
        }
        retired
    }

    pub(crate) fn retire_window_event_sources_for_execution_context_owner(
        &mut self,
        owner: super::WindowExecutionContextOwner,
    ) -> usize {
        let retired = self.retire_window_subresources_matching(
            PendingSubresourceContinuation::is_window_event_source,
            |execution_context| execution_context.window_realm_owner() == Some(owner),
        );
        if retired > 0 {
            tracing::debug!(
                ?owner,
                retired,
                "retired Window EventSource work with execution context"
            );
        }
        retired
    }

    pub(crate) fn retire_window_event_sources_for_context_token(
        &mut self,
        context_token: super::RuntimeObservableContextToken,
    ) -> usize {
        let retired = self.retire_window_subresources_matching(
            PendingSubresourceContinuation::is_window_event_source,
            |execution_context| execution_context.realm_token() == Some(context_token),
        );
        if retired > 0 {
            tracing::debug!(
                ?context_token,
                retired,
                "retired Window EventSource work with destroyed V8 execution context"
            );
        }
        retired
    }

    #[cfg(test)]
    pub(crate) fn pending_window_xhr_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        u64,
        super::WindowExecutionContextOwner,
        super::RuntimeObservableContextToken,
    )> {
        let mut pending = self
            .subresource_fetch_states()
            .filter(|pending| pending.continuation.is_window_xhr())
            .filter_map(|pending| {
                Some((
                    pending.info.internal_id,
                    pending.execution_context.window_realm_owner()?,
                    pending.execution_context.realm_token()?,
                ))
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|(internal_id, _, _)| *internal_id);
        pending
    }

    #[cfg(test)]
    pub(crate) fn pending_window_fetch_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        u64,
        bool,
        Option<super::WindowExecutionContextOwner>,
        Option<super::RuntimeObservableContextToken>,
    )> {
        let mut pending = self
            .subresource_fetch_states()
            .filter(|pending| pending.continuation.is_window_fetch())
            .map(|pending| {
                let detached_identity = pending.execution_context.detached_window_fetch_identity();
                (
                    pending.info.internal_id,
                    pending.continuation.is_detached_window_fetch(),
                    pending
                        .execution_context
                        .window_request_target()
                        .map(super::WindowTaskTarget::owner)
                        .or_else(|| detached_identity.map(|identity| identity.0)),
                    pending
                        .execution_context
                        .realm_token()
                        .or_else(|| detached_identity.map(|identity| identity.1)),
                )
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|(internal_id, _, _, _)| *internal_id);
        pending
    }

    #[cfg(test)]
    pub(crate) fn active_window_fetch_contexts_for_test(
        &self,
    ) -> Vec<(
        u64,
        super::WindowExecutionContextOwner,
        super::OwnerDispatchScope,
        super::RuntimeObservableContextToken,
        super::WindowTaskTarget,
    )> {
        let mut pending = self
            .subresource_fetch_states()
            .filter(|pending| pending.continuation.is_window_fetch())
            .filter_map(|pending| {
                let context = pending.execution_context.active_window_fetch_context()?;
                Some((
                    pending.info.internal_id,
                    context.script_realm().owner(),
                    context.script_realm().dispatch_scope(),
                    context.script_realm().realm_token(),
                    context.request_target(),
                ))
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|(internal_id, _, _, _, _)| *internal_id);
        pending
    }

    #[cfg(test)]
    pub(crate) fn pending_window_beacon_execution_contexts_for_test(
        &self,
    ) -> Vec<(u64, super::WindowExecutionContextIdentity, bool)> {
        let mut pending = self
            .subresource_fetch_states()
            .filter(|pending| {
                matches!(pending.continuation, PendingSubresourceContinuation::Beacon)
            })
            .filter_map(|pending| {
                Some((
                    pending.info.internal_id,
                    pending.execution_context.window_network_only_identity()?,
                    pending.execution_context.context_global().is_some(),
                ))
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|(internal_id, _, _)| *internal_id);
        pending
    }

    #[cfg(test)]
    pub(crate) fn pending_window_csp_report_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        u64,
        super::WindowDocumentNetworkRequestIdentity,
        bool,
        moli_fetch::RequestCredentialsMode,
    )> {
        let mut pending = self
            .subresource_fetch_states()
            .filter(|pending| {
                matches!(
                    pending.continuation,
                    PendingSubresourceContinuation::CspReport { .. }
                )
            })
            .filter_map(|pending| {
                Some((
                    pending.info.internal_id,
                    pending
                        .execution_context
                        .window_document_network_only_identity()?,
                    pending.execution_context.context_global().is_some(),
                    pending.credentials_mode,
                ))
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|(internal_id, _, _, _)| *internal_id);
        pending
    }

    pub(crate) fn retire_websocket_subresource_fetches(
        &mut self,
        socket_id: u64,
        mut internal_ids: Vec<u64>,
    ) -> usize {
        internal_ids.extend(
            self.pending_websocket_responses
                .values()
                .filter_map(|pending| {
                    (pending.socket_id == socket_id).then_some(pending.internal_id)
                }),
        );
        self.pending_websocket_responses
            .retain(|_, pending| pending.socket_id != socket_id);
        internal_ids.sort_unstable();
        internal_ids.dedup();

        let mut retired = 0;
        for internal_id in internal_ids.iter().copied() {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            retired += usize::from(self.abort_subresource_fetch(internal_id));
        }
        #[cfg(test)]
        self.pending_subresource_fetch_infos
            .retain(|info| !internal_ids.contains(&info.internal_id));
        retired
    }

    pub(crate) fn cancel_subresource_fetches_for_worker(
        &mut self,
        worker_id: DedicatedWorkerId,
    ) -> usize {
        let mut internal_ids =
            self.pending_subresource_fetches
                .iter()
                .filter_map(|(internal_id, pending)| {
                    (pending.continuation.dedicated_worker_id() == Some(worker_id))
                        .then_some(*internal_id)
                })
                .chain(self.in_flight_worker_subresource_fetches.iter().filter_map(
                    |(internal_id, running)| {
                        (running.pending.continuation.dedicated_worker_id() == Some(worker_id))
                            .then_some(*internal_id)
                    },
                ))
                .chain(self.running_subresource_fetches.iter().filter_map(
                    |(internal_id, running)| {
                        (running.pending.continuation.dedicated_worker_id() == Some(worker_id))
                            .then_some(*internal_id)
                    },
                ))
                .chain(self.streaming_subresource_fetches.iter().filter_map(
                    |(internal_id, streaming)| {
                        (streaming.pending.continuation.dedicated_worker_id() == Some(worker_id))
                            .then_some(*internal_id)
                    },
                ))
                .chain(self.pending_subresource_auths.iter().filter_map(
                    |(internal_id, pending)| {
                        (pending.pending.continuation.dedicated_worker_id() == Some(worker_id))
                            .then_some(*internal_id)
                    },
                ))
                .chain(self.pending_subresource_responses.iter().filter_map(
                    |(internal_id, pending)| {
                        (pending.pending.continuation.dedicated_worker_id() == Some(worker_id))
                            .then_some(*internal_id)
                    },
                ))
                .collect::<Vec<_>>();
        internal_ids.sort_unstable();
        internal_ids.dedup();
        let mut cancelled = 0;
        for internal_id in internal_ids.iter().copied() {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            cancelled += usize::from(self.abort_subresource_fetch(internal_id));
        }
        #[cfg(test)]
        self.pending_subresource_fetch_infos
            .retain(|info| !internal_ids.contains(&info.internal_id));
        if cancelled > 0 {
            tracing::debug!(
                worker_id = worker_id.as_u64(),
                cancelled,
                "cancelled subresource requests for retired DedicatedWorker"
            );
        }
        cancelled
    }

    pub(crate) fn cancel_stylesheet_subresource_fetches_for_document_owner(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> usize {
        let mut internal_ids =
            self.pending_subresource_fetches
                .iter()
                .filter_map(|(internal_id, pending)| {
                    (pending.continuation.stylesheet_subresource_owner() == Some(owner))
                        .then_some(*internal_id)
                })
                .chain(self.pending_subresource_auths.iter().filter_map(
                    |(internal_id, pending)| {
                        (pending.pending.continuation.stylesheet_subresource_owner() == Some(owner))
                            .then_some(*internal_id)
                    },
                ))
                .chain(self.pending_subresource_responses.iter().filter_map(
                    |(internal_id, pending)| {
                        (pending.pending.continuation.stylesheet_subresource_owner() == Some(owner))
                            .then_some(*internal_id)
                    },
                ))
                .chain(self.running_subresource_fetches.iter().filter_map(
                    |(internal_id, running)| {
                        (running.pending.continuation.stylesheet_subresource_owner() == Some(owner))
                            .then_some(*internal_id)
                    },
                ))
                .chain(self.streaming_subresource_fetches.iter().filter_map(
                    |(internal_id, streaming)| {
                        (streaming
                            .pending
                            .continuation
                            .stylesheet_subresource_owner()
                            == Some(owner))
                        .then_some(*internal_id)
                    },
                ))
                .collect::<Vec<_>>();
        internal_ids.sort_unstable();
        internal_ids.dedup();
        let mut cancelled = 0;
        for internal_id in internal_ids {
            let _ = self
                .browser_context_runtime
                .abort_service_worker_fetch(internal_id);
            cancelled += usize::from(self.abort_subresource_fetch(internal_id));
        }
        if cancelled != 0 {
            tracing::debug!(
                ?owner,
                cancelled,
                "cancelled stylesheet subresource requests for retired document owner"
            );
        }
        cancelled
    }

    pub(crate) fn abort_fetch_promise<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        internal_id: u64,
        reason: v8::Local<'s, v8::Value>,
        reason_payload: Option<crate::structured_clone::V8StructuredClonePayload>,
    ) -> bool {
        if let Some(pending) = self.pending_subresource_fetches.remove(&internal_id) {
            pending.load.cancel();
            self.browser_context_runtime
                .abort_service_worker_fetch_with_reason(internal_id, reason_payload.clone());
            reject_fetch_continuation(scope, pending.continuation, reason);
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(running) = self.running_subresource_fetches.remove(&internal_id) {
            running.pending.load.cancel();
            self.browser_context_runtime
                .abort_service_worker_fetch_with_reason(internal_id, reason_payload.clone());
            reject_fetch_continuation(scope, running.pending.continuation, reason);
            self.finish_active_subresource_request();
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(streaming) = self.streaming_subresource_fetches.remove(&internal_id) {
            streaming.pending.load.cancel();
            self.browser_context_runtime
                .abort_service_worker_fetch_with_reason(internal_id, reason_payload.clone());
            // Headers-first fetch may already have resolved the Response. In
            // that state aborting must error the body stream/materialization
            // promise, not only the original fetch continuation.
            crate::network_host::error_pending_network_body_stream_with_reason(
                scope,
                streaming.body_source_id,
                "The operation was aborted.".to_owned(),
                reason,
            );
            self.record_streaming_subresource_fetch_failure(
                streaming,
                crate::network_host::ABORTED_ERROR_TEXT.to_owned(),
            );
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(auth) = self.pending_subresource_auths.remove(&internal_id) {
            let pending = auth.pending;
            pending.load.cancel();
            self.browser_context_runtime
                .abort_service_worker_fetch_with_reason(internal_id, reason_payload.clone());
            reject_fetch_continuation(scope, pending.continuation, reason);
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        if let Some(response) = self.pending_subresource_responses.remove(&internal_id) {
            let pending = response.pending;
            pending.load.cancel();
            self.browser_context_runtime
                .abort_service_worker_fetch_with_reason(internal_id, reason_payload);
            reject_fetch_continuation(scope, pending.continuation, reason);
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
            return true;
        }

        false
    }

    pub(crate) fn record_pending_subresource_continue_event(
        &mut self,
        event: PendingSubresourceContinueEvent,
    ) {
        if let Some(source_document) = self.root_document_lifecycle_identity()
            && self.append_live_turn_owner_action(
                crate::runtime::RendererOwnerAction::SubresourceContinue {
                    source_document,
                    event: Box::new(event.clone()),
                },
            )
        {
            return;
        }
        #[cfg(test)]
        {
            self.pending_subresource_continue_events.push(event);
            self.note_subresource_activity();
        }
        #[cfg(not(test))]
        {
            let _ = event;
            panic!("a production Fetch continuation must have a concrete renderer output sink");
        }
    }

    #[cfg(test)]
    pub(crate) fn take_pending_subresource_continue_events(
        &mut self,
    ) -> Vec<PendingSubresourceContinueEvent> {
        std::mem::take(&mut self.pending_subresource_continue_events)
    }

    pub(crate) fn pending_subresource_continue_event_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_subresource_continue_events.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }

    pub(crate) fn record_pending_subresource_response(
        &mut self,
        state: PendingSubresourceResponseState,
    ) {
        self.pending_subresource_responses
            .insert(state.pending.info.internal_id, state);
        self.note_subresource_activity();
    }

    pub(crate) fn record_worker_subresource_response_pause(
        &mut self,
        info: PendingSubresourceResponseInfo,
    ) {
        if let Some(in_flight) = self
            .in_flight_worker_subresource_fetches
            .remove(&info.internal_id)
        {
            self.record_pending_subresource_response(PendingSubresourceResponseState {
                pending: in_flight.pending,
                request_url: in_flight.request_url,
                request_method: in_flight.request_method,
                request_headers: in_flight.request_headers,
                request_body: in_flight.request_body,
                response: navigation_response_from_subresource_body(
                    &info.response_body,
                    moli_fetch::ResponseHead {
                        final_url: info.url.clone(),
                        status: info.response_status,
                        headers: info.response_headers.clone(),
                        request_cookie_report: info.request_cookie_report.clone(),
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: info.from_cache,
                        negotiated_http_version: None,
                    },
                )
                .with_network_request_headers(info.network_request_headers.clone()),
            });
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::ResponsePaused(info),
            );
        } else {
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed {
                    internal_id: info.internal_id,
                },
            );
        }
    }

    pub(crate) fn record_worker_subresource_auth_pause(
        &mut self,
        info: PendingSubresourceAuthInfo,
    ) {
        if let Some(in_flight) = self
            .in_flight_worker_subresource_fetches
            .remove(&info.internal_id)
        {
            self.record_pending_subresource_auth(PendingSubresourceAuthState {
                pending: in_flight.pending,
                request_url: in_flight.request_url,
                request_method: in_flight.request_method,
                request_headers: in_flight.request_headers,
                request_body: in_flight.request_body,
                intercept_response: info.intercept_response,
                initial_network_request_headers: info.network_request_headers.clone(),
                response: navigation_response_from_subresource_body(
                    &info.response_body,
                    moli_fetch::ResponseHead {
                        final_url: info.response_final_url.clone(),
                        status: info.response_status,
                        headers: info.response_headers.clone(),
                        request_cookie_report: info.request_cookie_report.clone(),
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: info.response_from_cache,
                        negotiated_http_version: None,
                    },
                )
                .with_network_request_headers(info.network_request_headers.clone()),
            });
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::AuthRequired(info),
            );
        } else {
            self.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed {
                    internal_id: info.internal_id,
                },
            );
        }
    }

    pub(crate) fn record_worker_subresource_completed(&mut self, internal_id: u64) {
        self.in_flight_worker_subresource_fetches
            .remove(&internal_id);
        self.record_pending_subresource_continue_event(
            PendingSubresourceContinueEvent::Completed { internal_id },
        );
    }

    pub(crate) fn record_pending_websocket_response(
        &mut self,
        state: PendingWebSocketResponseState,
    ) {
        self.pending_websocket_responses
            .insert(state.internal_id, state);
        self.note_subresource_activity();
    }

    pub(crate) fn record_pending_subresource_auth(&mut self, state: PendingSubresourceAuthState) {
        self.pending_subresource_auths
            .insert(state.pending.info.internal_id, state);
        self.note_subresource_activity();
    }

    pub(crate) fn take_pending_subresource_response(
        &mut self,
        internal_id: u64,
    ) -> Option<PendingSubresourceResponseState> {
        self.pending_subresource_responses.remove(&internal_id)
    }

    pub(crate) fn take_pending_websocket_response(
        &mut self,
        internal_id: u64,
    ) -> Option<PendingWebSocketResponseState> {
        self.pending_websocket_responses.remove(&internal_id)
    }

    pub(crate) fn take_pending_subresource_auth(
        &mut self,
        internal_id: u64,
    ) -> Option<PendingSubresourceAuthState> {
        self.pending_subresource_auths.remove(&internal_id)
    }

    pub(crate) fn begin_active_subresource_request(&mut self) {
        self.active_subresource_requests += 1;
        self.note_subresource_activity();
    }

    pub(crate) fn finish_active_subresource_request(&mut self) {
        self.active_subresource_requests = self.active_subresource_requests.saturating_sub(1);
        self.note_subresource_activity();
    }

    pub(crate) fn pending_subresource_request_count(&self) -> usize {
        self.active_subresource_requests
            + self.pending_subresource_fetches.len()
            + self.pending_subresource_auths.len()
            + self.pending_subresource_responses.len()
            + self.pending_websocket_responses.len()
            + self.streaming_subresource_fetches.len()
            + self.in_flight_worker_subresource_fetches.len()
    }

    pub(crate) fn note_subresource_activity(&mut self) {
        self.subresource_activity_epoch = self.subresource_activity_epoch.wrapping_add(1);
        self.subresource_last_activity_at = std::time::Instant::now();
    }
}

fn reject_fetch_continuation(
    scope: &mut v8::PinScope<'_, '_>,
    continuation: crate::types::PendingSubresourceContinuation,
    reason: v8::Local<'_, v8::Value>,
) {
    if let crate::types::PendingSubresourceContinuation::Fetch(fetch) = continuation
        && let Some(resolver) = fetch.into_resolver()
    {
        let resolver = v8::Local::new(scope, &resolver);
        let _ = resolver.reject(scope, reason);
    }
}
