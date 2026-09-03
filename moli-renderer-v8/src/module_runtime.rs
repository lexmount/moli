mod driver;
mod dynamic_resolver;
mod frame_document_parser_tree;
mod graph;
mod graph_fetch_store;
mod host_callbacks;
mod import_map;
mod modulator;
mod owner_ids;
mod parser_tree_registry;
mod record;
mod record_resolver;
mod resolver;
mod single_module_fetch;
#[cfg(test)]
mod tests;
mod tree_adapter;
mod tree_job;
mod tree_owner;
mod wasm_synthetic;

use moli_module_script_tree as module_tree;
use url::Url;

use super::script_vm::ScriptVm;
use crate::document_task_lane::DocumentPostedTaskSource;
use crate::dom::native::NativeNodeId;
use crate::module_script_continuation::ModuleScriptCompletionOwner;
use crate::parser::{PreparedImportMap, PreparedImportMapSource};
use crate::planning::ScriptFetchMetadata;

use self::driver::{
    register_import_map_source as register_import_map_source_from_driver,
    resolve_module_integrity as resolve_module_integrity_from_driver,
    resolve_module_specifier as resolve_module_specifier_from_driver,
};
pub(crate) use self::dynamic_resolver::{
    DynamicModuleEvaluationTarget, DynamicModuleExecutionContextRetirement,
    DynamicModuleFetchContinuation, DynamicModuleFetchFailure, DynamicModuleFetchFinish,
    DynamicModuleFetchOwnerAdvance, DynamicModuleImportOwner, DynamicModuleInflightFetch,
    DynamicModuleJoinedFetch, DynamicModuleScheduledFetch, PendingDynamicModuleEvaluationReaction,
    PendingDynamicModuleImport,
};
pub(crate) use self::import_map::ImportMapRegistryState;
pub(crate) use self::modulator::{
    NativeDocumentModulator, NativeFrameDocumentDependencyFetchBuildFailure,
};
pub(crate) use self::parser_tree_registry::NativeParserModuleTreeJobResume;
pub(crate) use self::record::{ModuleRecordEntry, WasmImportRecord, WasmModuleRecord};
pub(crate) use self::resolver::{
    ResolverScopeGuard, resolve_static_module_callback, resolve_static_source_callback,
};
pub(crate) use self::single_module_fetch::NativeModuleSingleFetchRequest;
pub(crate) use self::tree_owner::{
    NativeModuleTreeDocumentOwnerAdapter, NativeModuleTreeFrameDocumentOwner,
};
pub(crate) use crate::document_module_graph::{
    ModuleAttributesKey, ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource,
    ModuleIdentityHash, ModuleImportPhase, ModuleKind, ModuleLoadError, ModuleLoadStage,
    ModuleMapEntryState, ModuleMapFetchDisposition, ModuleMapKey, ModuleMapTerminalNotification,
    ModuleRequestRecord, ModuleResolvedDependency, ModuleSource,
    NativeDynamicImportSingleModuleClient, NativeModuleMapSingleModuleClient,
    NativeModuleScriptSingleModuleClient, NativeModulepreloadLinkClient,
};
pub(crate) use graph::{
    ModuleGraphHandle, ModuleScriptGraphAdvance, ModuleScriptGraphFetchBatch,
    ModuleScriptGraphFetchContinuation, NativeDynamicModuleImportReady,
    NativeModuleGraphFetchRequest, NativeModuleGraphJob, NativeModuleGraphJobAdvance,
    advance_module_script_graph, module_script_graph_advance_from_native, next_inline_module_url,
    parser_owned_loaded_module_script_graph_job, runtime_owned_external_module_script_graph_job,
    runtime_owned_loaded_module_script_graph_job,
};
use graph_fetch_store::NativeModuleGraphFetchStore;
pub(crate) use host_callbacks::{
    dynamic_import_callback, dynamic_import_with_phase_callback,
    initialize_import_meta_object_callback,
};
pub(crate) use wasm_synthetic::{
    WasmDependencyModuleMessages, ensure_wasm_dependency_module_namespace_ready,
    evaluate_wasm_synthetic_module, preserve_current_v8_module_exception, throw_wasm_link_error,
    throw_wasm_synthetic_module_error, wasm_dependency_export_value,
};

pub(crate) fn module_identity_hash_from_v8_module(
    module: v8::Local<'_, v8::Module>,
) -> ModuleIdentityHash {
    ModuleIdentityHash::from_raw(module.get_identity_hash().get() as u32)
}

#[derive(Debug)]
pub(crate) enum NativeModuleOwnerEvent {
    ModuleMapTerminalNotification(ModuleMapTerminalNotification),
    ModulepreloadLinkError(NativeNodeId),
}

impl NativeModuleOwnerEvent {
    fn detach_single_module_client(
        &mut self,
        client: module_tree::SingleModuleClientToken,
    ) -> bool {
        match self {
            Self::ModuleMapTerminalNotification(notification) => {
                notification.detach_single_module_client(client)
            }
            Self::ModulepreloadLinkError(_) => false,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::ModuleMapTerminalNotification(notification) => notification.is_empty(),
            Self::ModulepreloadLinkError(_) => false,
        }
    }

    fn retain_for_document_replacement(&mut self) {
        match self {
            Self::ModuleMapTerminalNotification(notification) => {
                notification.retain_dynamic_import_clients();
            }
            Self::ModulepreloadLinkError(_) => {}
        }
    }

    fn survives_document_replacement(&self) -> bool {
        matches!(self, Self::ModuleMapTerminalNotification(notification) if !notification.is_empty())
    }
}

pub(crate) fn register_import_map_source(
    vm: &mut ScriptVm,
    source: &str,
) -> std::result::Result<(), String> {
    register_import_map_source_from_driver(vm, source)
}

pub(crate) fn register_parser_owned_import_map_source(
    vm: &mut ScriptVm,
    source: &str,
    base_url: &Url,
) -> std::result::Result<(), String> {
    vm.document_runtime
        .register_import_map_source_with_base_url(source, base_url)
}

pub(crate) fn accept_parser_owned_import_map_handoff(
    vm: &mut ScriptVm,
    node_id: NativeNodeId,
    start_line: u64,
    start_column: u64,
    import_map: PreparedImportMap,
) {
    vm.document_runtime
        .note_parser_script_start_position(node_id, start_line, start_column);
    let host_script_handle = vm
        .document_runtime
        .bind_parser_owned_script_handle_for_node(import_map.node_id);
    let _ = vm
        .document_runtime
        .dom_host_mut()
        .set_script_already_started(node_id, true);
    match import_map.source {
        PreparedImportMapSource::Inline(source) => {
            let request =
                crate::content_security_policy::ContentSecurityPolicyScriptElementRequest::
                    parser_inserted_with_nonce(import_map.nonce.as_deref());
            let Some(source) =
                vm.inline_script_element_source_for_execution(node_id, &source, request)
            else {
                return;
            };
            if let Err(error) =
                register_parser_owned_import_map_source(vm, &source, &import_map.base_url)
            {
                vm.report_parser_import_map_registration_failure_and_finish_algorithm_best_effort(
                    &error,
                    Some(import_map.initiator_url.as_str()),
                );
            }
        }
        PreparedImportMapSource::ExternalUnsupported => {
            let _ = vm.document_runtime.enqueue_script_event_lifecycle_work(
                crate::host::ScriptEventKind::Error,
                &host_script_handle,
            );
        }
    }
}

pub(crate) fn resolve_module_specifier(
    vm: &mut ScriptVm,
    specifier: &str,
    base_url: &Url,
) -> std::result::Result<Url, String> {
    resolve_module_specifier_from_driver(vm, specifier, base_url)
}

pub(crate) fn resolve_module_integrity(vm: &ScriptVm, url: &Url) -> Option<String> {
    resolve_module_integrity_from_driver(vm, url)
}

pub(crate) enum ModuleScriptExecutionOutcome {
    CompletedModuleGraph(ModuleGraphHandle),
    SuspendedModuleFetches(Box<ModuleScriptGraphFetchBatch>),
}

pub(crate) async fn execute_module_script_source(
    vm: &mut ScriptVm,
    source: ModuleSource,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    source_is_external: bool,
    completion_owner: ModuleScriptCompletionOwner,
) -> std::result::Result<ModuleScriptExecutionOutcome, ModuleLoadError> {
    graph::execute_native_module_script_source(
        vm,
        source,
        base_url,
        initiator_url,
        fetch_metadata,
        source_is_external,
        completion_owner,
    )
    .await
}

pub(crate) async fn execute_external_module_script_graph(
    vm: &mut ScriptVm,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    completion_owner: ModuleScriptCompletionOwner,
) -> std::result::Result<ModuleScriptExecutionOutcome, ModuleLoadError> {
    graph::execute_external_native_module_script_graph(
        vm,
        base_url,
        initiator_url,
        fetch_metadata,
        completion_owner,
    )
    .await
}

pub(crate) fn parser_owned_external_module_script_tree_job(
    vm: &mut ScriptVm,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
) -> NativeModuleGraphJob {
    graph::parser_owned_external_module_script_graph_job(
        vm,
        base_url,
        initiator_url,
        fetch_metadata,
    )
}

pub(crate) fn parser_owned_module_script_graph_job_for_prepared_script(
    vm: &mut ScriptVm,
    script: &crate::planning::PreparedScript,
) -> std::result::Result<Option<NativeModuleGraphJob>, ModuleLoadError> {
    if script.kind != crate::types::ScriptKind::Module {
        return Ok(None);
    }
    match &script.source {
        crate::planning::ScriptSource::External if script.url.scheme() == "data" => {
            let source = crate::planning::decode_data_url_script_source(&script.url)
                .map_err(|error| ModuleLoadError::new(ModuleLoadStage::Fetch, error.to_string()))?;
            parser_owned_loaded_module_script_graph_job(
                vm,
                ModuleSource::text(source),
                &script.base_url,
                &script.initiator_url,
                &script.fetch_metadata,
                true,
            )
            .map(Some)
        }
        crate::planning::ScriptSource::External => {
            Ok(Some(parser_owned_external_module_script_tree_job(
                vm,
                &script.url,
                &script.initiator_url,
                &script.fetch_metadata,
            )))
        }
        crate::planning::ScriptSource::Loaded(source) => {
            Ok(Some(parser_owned_loaded_module_script_graph_job(
                vm,
                ModuleSource::text(source.clone()),
                &script.base_url,
                &script.initiator_url,
                &script.fetch_metadata,
                true,
            )?))
        }
        crate::planning::ScriptSource::LoadedBinary { bytes, .. } => {
            Ok(Some(parser_owned_loaded_module_script_graph_job(
                vm,
                ModuleSource::binary(bytes.clone()),
                &script.base_url,
                &script.initiator_url,
                &script.fetch_metadata,
                true,
            )?))
        }
        crate::planning::ScriptSource::Inline(source) => {
            let source = vm.inline_module_script_source_for_graph_start(script, source);
            Ok(Some(parser_owned_loaded_module_script_graph_job(
                vm,
                source,
                &script.base_url,
                &script.initiator_url,
                &script.fetch_metadata,
                false,
            )?))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModulePreloadJobRun {
    Scheduled,
    CompletedSynchronously,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeModulepreloadFetchStart {
    Started(Box<NativeModuleSingleFetchRequest>),
    Joined,
    AlreadyComplete,
}

impl NativeModulepreloadFetchStart {
    pub(crate) fn started_request(self) -> Option<NativeModuleSingleFetchRequest> {
        match self {
            Self::Started(request) => Some(*request),
            Self::Joined | Self::AlreadyComplete => None,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ModuleOwnerState {
    import_map_registry: ImportMapRegistryState,
    document_modulator: NativeDocumentModulator,
    graph_fetches: NativeModuleGraphFetchStore,
    owner_event_tasks: DocumentPostedTaskSource<NativeModuleOwnerEvent>,
    next_inline_module_eval_id: u64,
}

impl ModuleOwnerState {
    pub(crate) fn clear_for_document_replacement(&mut self) {
        // Import maps, module-map entries, compiled records, ID allocation and
        // dynamic-import resolver state belong to the live ScriptState. Only
        // parser/script-element work belongs to the replaced Document.
        self.document_modulator.clear_for_document_replacement();
        self.graph_fetches.clear();
        self.owner_event_tasks.update_ready_tasks(|events| {
            for event in events.iter_mut() {
                event.retain_for_document_replacement();
            }
            events.retain(NativeModuleOwnerEvent::survives_document_replacement);
        });
    }

    fn post_ready_native_module_owner_event_tasks(&mut self) {
        let mut ready_events = Vec::new();
        self.document_modulator.drain_ready_owner_events(|event| {
            ready_events.push(event);
        });
        for event in ready_events {
            self.owner_event_tasks.post(event);
        }
    }

    pub(crate) fn post_modulepreload_link_error_event(&mut self, link: NativeNodeId) {
        self.owner_event_tasks
            .post(NativeModuleOwnerEvent::ModulepreloadLinkError(link));
    }

    pub(crate) fn register_import_map(
        &mut self,
        source: &str,
        base_url: &Url,
    ) -> std::result::Result<(), String> {
        self.import_map_registry
            .register_import_map(source, base_url)
    }

    pub(crate) fn resolve_module_specifier(
        &mut self,
        specifier: &str,
        base_url: &Url,
    ) -> std::result::Result<Url, String> {
        self.import_map_registry
            .resolve_module_specifier(specifier, base_url)
    }

    pub(crate) fn resolve_module_integrity(&self, url: &Url) -> Option<String> {
        self.import_map_registry.resolve_module_integrity(url)
    }

    pub(crate) fn next_inline_module_eval_id(&mut self) -> u64 {
        let id = self.next_inline_module_eval_id;
        self.next_inline_module_eval_id += 1;
        id
    }

    #[cfg(test)]
    pub(crate) fn insert_native_module_source(
        &mut self,
        key: ModuleMapKey,
        source: ModuleSource,
    ) -> ModuleEntryId {
        let entry_id = self.document_modulator.insert_fetched_source(key, source);
        self.post_ready_native_module_owner_event_tasks();
        entry_id
    }

    pub(crate) fn insert_native_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.apply_native_module_source_for_request(
            request_key,
            effective_key,
            source,
            effective_fetch_metadata,
        )
    }

    pub(crate) fn apply_native_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        let entry_id = self.document_modulator.insert_fetched_source_for_request(
            request_key,
            effective_key,
            source,
            effective_fetch_metadata,
        );
        self.post_ready_native_module_owner_event_tasks();
        entry_id
    }

    pub(crate) fn start_or_join_native_module_fetch(
        &mut self,
        key: ModuleMapKey,
    ) -> ModuleMapFetchDisposition {
        self.document_modulator.start_or_join_fetch(key)
    }

    pub(crate) fn insert_native_compiled_module_record_with_metadata(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.document_modulator
            .insert_compiled_record_with_metadata(
                request_key,
                record,
                identity,
                effective_fetch_metadata,
            )
    }

    pub(crate) fn native_module_entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId> {
        self.document_modulator.entry_id(key)
    }

    pub(crate) fn native_module_entry_state(&self, entry_id: ModuleEntryId) -> ModuleMapEntryState {
        self.document_modulator.entry(entry_id).state()
    }

    pub(crate) fn native_module_source(&self, entry_id: ModuleEntryId) -> Option<ModuleSource> {
        self.document_modulator.entry(entry_id).source().cloned()
    }

    pub(crate) fn native_module_source_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<(ModuleMapKey, ModuleSource)> {
        self.document_modulator.module_source_for(module)
    }

    pub(crate) fn native_module_wasm_record_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<WasmModuleRecord> {
        self.document_modulator.module_wasm_record_for(module)
    }

    pub(crate) fn native_module_wasm_record(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<WasmModuleRecord> {
        self.document_modulator.module_wasm_record(entry_id)
    }

    pub(crate) fn native_wasm_instance_for_namespace<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        namespace: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.document_modulator
            .wasm_instance_for_namespace(scope, namespace)
    }

    pub(crate) fn native_resolved_dependency_module_for(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<v8::Global<v8::Module>> {
        self.document_modulator
            .resolved_dependency_module_for(referrer, specifier, attributes)
    }

    pub(crate) fn native_module_entry_url(&self, entry_id: ModuleEntryId) -> Url {
        self.document_modulator.entry_url(entry_id)
    }

    pub(crate) fn native_module_entry_key(&self, entry_id: ModuleEntryId) -> ModuleMapKey {
        self.document_modulator
            .entry(entry_id)
            .effective_key()
            .clone()
    }

    pub(crate) fn native_module_effective_fetch_metadata(
        &self,
        entry_id: ModuleEntryId,
    ) -> ModuleFetchMetadata {
        self.document_modulator
            .entry(entry_id)
            .effective_fetch_metadata()
            .clone()
    }

    pub(crate) fn native_module_failure(&self, entry_id: ModuleEntryId) -> Option<ModuleLoadError> {
        self.document_modulator.entry(entry_id).failure().cloned()
    }

    pub(crate) fn native_module_requests(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<Vec<ModuleRequestRecord>> {
        self.document_modulator
            .compiled_record(entry_id)
            .map(|record| record.requests().to_vec())
    }

    pub(crate) fn set_native_module_resolved_dependencies(
        &mut self,
        entry_id: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        self.document_modulator
            .set_resolved_dependencies(entry_id, dependencies);
    }

    #[cfg(test)]
    pub(crate) fn native_module_resolved_dependencies(
        &self,
        entry_id: ModuleEntryId,
    ) -> Vec<ModuleResolvedDependency> {
        self.document_modulator
            .resolved_dependencies(entry_id)
            .to_vec()
    }

    pub(crate) fn native_document_modulator_ptr(&self) -> *const NativeDocumentModulator {
        &self.document_modulator
    }

    pub(crate) fn native_compiled_module(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<v8::Global<v8::Module>> {
        self.document_modulator
            .compiled_record(entry_id)
            .map(|record| record.compiled_module().clone())
    }

    pub(crate) fn mark_native_module_instantiated(&mut self, entry_id: ModuleEntryId) {
        self.document_modulator.mark_instantiated(entry_id);
    }

    pub(crate) fn mark_native_module_evaluating(&mut self, entry_id: ModuleEntryId) {
        self.document_modulator.mark_evaluating(entry_id);
    }

    pub(crate) fn mark_native_module_evaluated(&mut self, entry_id: ModuleEntryId) {
        self.document_modulator.mark_evaluated(entry_id);
    }

    pub(crate) fn mark_native_module_failed(
        &mut self,
        key: ModuleMapKey,
        error: ModuleLoadError,
    ) -> ModuleEntryId {
        self.apply_native_module_failure(key, error)
    }

    pub(crate) fn apply_native_module_failure(
        &mut self,
        key: ModuleMapKey,
        error: ModuleLoadError,
    ) -> ModuleEntryId {
        let entry_id = self.document_modulator.mark_failed(key, error);
        self.post_ready_native_module_owner_event_tasks();
        entry_id
    }

    pub(crate) fn queue_native_dynamic_module_import(
        &mut self,
        request: PendingDynamicModuleImport,
    ) {
        self.document_modulator.queue_dynamic_module_import(request);
    }

    pub(crate) fn take_next_native_dynamic_module_import(
        &mut self,
    ) -> Option<NativeModuleGraphJob> {
        self.document_modulator.take_next_dynamic_module_import()
    }

    pub(crate) fn resume_native_dynamic_module_import_front(&mut self, job: NativeModuleGraphJob) {
        self.document_modulator
            .resume_dynamic_module_import_front(job);
    }

    pub(crate) fn reserve_native_dynamic_module_evaluation_reaction(
        &mut self,
        request: PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
    ) -> u64 {
        self.document_modulator
            .reserve_dynamic_module_evaluation_reaction(request, target)
    }

    pub(crate) fn native_dynamic_module_evaluation_reaction_owner(
        &self,
        reaction_id: u64,
    ) -> Option<DynamicModuleImportOwner> {
        self.document_modulator
            .dynamic_module_evaluation_reaction_owner(reaction_id)
    }

    pub(crate) fn take_native_dynamic_module_evaluation_reaction(
        &mut self,
        reaction_id: u64,
        expected_owner: DynamicModuleImportOwner,
    ) -> Option<PendingDynamicModuleEvaluationReaction> {
        self.document_modulator
            .take_dynamic_module_evaluation_reaction(reaction_id, expected_owner)
    }

    pub(crate) fn retire_native_dynamic_module_import_execution_context(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) -> DynamicModuleExecutionContextRetirement {
        self.document_modulator
            .retire_dynamic_module_import_execution_context(owner)
    }

    pub(crate) fn suspend_native_module_script_fetch(
        &mut self,
        continuation: ModuleScriptGraphFetchContinuation,
    ) -> u64 {
        self.graph_fetches.suspend_fetch(continuation)
    }

    pub(crate) fn take_inflight_native_module_script_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<ModuleScriptGraphFetchContinuation> {
        self.graph_fetches.take_inflight_fetch(load_id)
    }

    pub(crate) fn has_inflight_native_module_script_fetch(&self, load_id: u64) -> bool {
        self.graph_fetches.has_inflight_fetch(load_id)
    }

    pub(crate) fn suspend_native_dynamic_module_import_fetches(
        &mut self,
        requests: Vec<NativeModuleGraphFetchRequest>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        job: NativeModuleGraphJob,
        owner_module_fetch_starts: Vec<
            Option<crate::frame_owner_model::FrameDocumentModuleFetchClientStart>,
        >,
    ) -> Vec<DynamicModuleScheduledFetch> {
        let fetches = self.reserve_native_module_graph_fetches(requests);
        self.document_modulator
            .suspend_dynamic_module_import_fetches(
                fetches,
                joined_clients,
                job,
                owner_module_fetch_starts,
            )
    }

    pub(crate) fn take_inflight_native_dynamic_module_import_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<DynamicModuleInflightFetch> {
        self.document_modulator
            .take_inflight_dynamic_module_import_fetch(load_id)
    }

    pub(crate) fn inflight_native_dynamic_module_import_fetch_owner(
        &self,
        load_id: u64,
    ) -> Option<DynamicModuleImportOwner> {
        self.document_modulator
            .inflight_dynamic_module_import_fetch_owner(load_id)
    }

    pub(crate) fn take_joined_native_dynamic_module_import_fetch(
        &mut self,
        client: moli_module_script_tree::SingleModuleClientToken,
    ) -> Option<DynamicModuleJoinedFetch> {
        self.document_modulator
            .take_joined_dynamic_module_import_fetch(client)
    }

    pub(crate) fn continue_native_dynamic_module_import_fetch(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        owner_module_fetch_starts: Vec<
            Option<crate::frame_owner_model::FrameDocumentModuleFetchClientStart>,
        >,
    ) -> DynamicModuleFetchOwnerAdvance {
        let fetches = continuation
            .pending_fetch_requests()
            .map(|requests| self.reserve_native_module_graph_fetches(requests.to_vec()))
            .unwrap_or_default();
        self.document_modulator
            .continue_dynamic_module_import_fetch(continuation, fetches, owner_module_fetch_starts)
    }

    pub(crate) fn clear_failed_native_dynamic_module_import_fetch(
        &mut self,
        failure: DynamicModuleFetchFailure,
    ) -> (PendingDynamicModuleImport, ModuleLoadError) {
        let (joined_clients, request, error) = self
            .document_modulator
            .clear_failed_dynamic_module_import_fetch(failure);
        for client in joined_clients {
            self.detach_native_module_fetch_waiter(client);
        }
        (request, error)
    }

    pub(crate) fn has_pending_native_dynamic_module_import(&self) -> bool {
        self.document_modulator.has_pending_dynamic_module_import()
    }

    pub(crate) fn has_ready_native_dynamic_module_import(&self) -> bool {
        self.document_modulator.has_ready_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_native_dynamic_module_import_fetch(&self) -> bool {
        self.document_modulator
            .has_inflight_dynamic_module_import_fetch()
    }

    fn reserve_native_module_graph_fetches(
        &mut self,
        requests: Vec<NativeModuleGraphFetchRequest>,
    ) -> Vec<(u64, NativeModuleGraphFetchRequest)> {
        let mut fetches = Vec::with_capacity(requests.len());
        for request in requests {
            let load_id = self.graph_fetches.reserve_load_id();
            fetches.push((load_id, request));
        }
        fetches
    }

    pub(crate) fn suspend_native_modulepreload_fetch(
        &mut self,
        request: NativeModuleSingleFetchRequest,
    ) -> u64 {
        let load_id = self.graph_fetches.reserve_load_id();
        self.document_modulator
            .suspend_modulepreload_fetch(load_id, request);
        load_id
    }

    pub(crate) fn take_inflight_native_modulepreload_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<NativeModuleSingleFetchRequest> {
        self.document_modulator
            .take_inflight_modulepreload_fetch(load_id)
    }

    pub(crate) fn has_inflight_native_modulepreload_fetch_for(&self, load_id: u64) -> bool {
        self.document_modulator
            .has_inflight_modulepreload_fetch_for(load_id)
    }

    pub(crate) fn suspend_native_modulepreload_link_clients(
        &mut self,
        key: ModuleMapKey,
        clients: Vec<std::sync::Arc<NativeModulepreloadLinkClient>>,
    ) {
        for client in clients {
            self.document_modulator
                .add_modulepreload_link_client(key.clone(), client);
        }
    }

    pub(crate) fn suspend_native_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: NativeModuleMapSingleModuleClient,
    ) {
        self.document_modulator
            .add_single_module_fetch_client(key, client);
    }

    pub(crate) fn detach_native_module_fetch_waiter(
        &mut self,
        client: moli_module_script_tree::SingleModuleClientToken,
    ) -> bool {
        let mut detached = self
            .document_modulator
            .detach_single_module_fetch_client(client);
        self.owner_event_tasks.update_ready_tasks(|events| {
            for event in events.iter_mut() {
                detached |= event.detach_single_module_client(client);
            }
            events.retain(|event| !event.is_empty());
        });
        detached
    }

    pub(crate) fn take_next_native_module_owner_event(&mut self) -> Option<NativeModuleOwnerEvent> {
        while let Some(event) = self.owner_event_tasks.pop_front() {
            if !event.is_empty() {
                return Some(event);
            }
        }
        None
    }

    pub(crate) fn has_ready_native_module_owner_event(&mut self) -> bool {
        self.owner_event_tasks.update_ready_tasks(|events| {
            events.retain(|event| !event.is_empty());
            !events.is_empty()
        })
    }

    #[cfg(test)]
    pub(crate) fn has_local_native_module_owner_event_for_testing(&self) -> bool {
        !self.owner_event_tasks.is_empty_local_only()
    }

    #[cfg(test)]
    pub(crate) fn drain_posted_native_module_owner_event_tasks_for_testing(&mut self) {
        self.owner_event_tasks.drain_posted_for_testing();
    }

    #[cfg(test)]
    pub(crate) fn has_native_module_script_fetch_waiters(&self) -> bool {
        self.document_modulator.module_script_client_count() > 0
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_native_modulepreload_fetch(&self) -> bool {
        self.document_modulator.has_inflight_modulepreload_fetch()
    }

    #[cfg(test)]
    pub(crate) fn module_script_client_count_for_testing(&self) -> usize {
        self.document_modulator.module_script_client_count()
    }

    #[cfg(test)]
    pub(crate) fn modulepreload_link_client_count_for_testing(&self) -> usize {
        self.document_modulator.modulepreload_link_client_count()
    }
}
