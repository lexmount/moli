use super::*;
use crate::module_runtime::{
    DynamicModuleEvaluationTarget, DynamicModuleExecutionContextRetirement,
    DynamicModuleFetchContinuation, DynamicModuleFetchFailure, DynamicModuleFetchOwnerAdvance,
    DynamicModuleInflightFetch, DynamicModuleJoinedFetch, ModuleEntryId, ModuleFetchMetadata,
    ModuleIdentityHash, ModuleLoadError, ModuleMapFetchDisposition, ModuleMapKey,
    ModuleRecordEntry, ModuleRequestRecord, ModuleResolvedDependency,
    ModuleScriptGraphFetchContinuation, ModuleSource, NativeModuleGraphFetchRequest,
    NativeModuleSingleFetchRequest, NativeModulepreloadFetchStart,
    PendingDynamicModuleEvaluationReaction, PendingDynamicModuleImport, WasmModuleRecord,
};
#[cfg(test)]
use crate::native_bridge::PendingRuntimeBindingCall;
use url::Url;

impl DocumentRuntime {
    pub(crate) fn with_native_module_owner<R>(
        &self,
        f: impl FnOnce(&crate::module_runtime::ModuleOwnerState) -> R,
    ) -> R {
        f(self.script_lifecycle.scripts().native_module_owner())
    }

    pub(crate) fn take_inflight_native_dynamic_module_import_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<DynamicModuleInflightFetch> {
        self.script_lifecycle
            .scripts_mut()
            .take_inflight_native_dynamic_module_import_fetch(load_id)
    }

    pub(crate) fn take_joined_native_dynamic_module_import_fetch(
        &mut self,
        client: moli_module_script_tree::SingleModuleClientToken,
    ) -> Option<DynamicModuleJoinedFetch> {
        self.script_lifecycle
            .scripts_mut()
            .take_joined_native_dynamic_module_import_fetch(client)
    }

    pub(crate) fn continue_native_dynamic_module_import_fetch(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        owner_module_fetch_starts: Vec<
            Option<crate::frame_owner_model::FrameDocumentModuleFetchClientStart>,
        >,
    ) -> DynamicModuleFetchOwnerAdvance {
        self.script_lifecycle
            .scripts_mut()
            .continue_native_dynamic_module_import_fetch(continuation, owner_module_fetch_starts)
    }

    pub(crate) fn clear_failed_native_dynamic_module_import_fetch(
        &mut self,
        failure: DynamicModuleFetchFailure,
    ) -> (PendingDynamicModuleImport, ModuleLoadError) {
        self.script_lifecycle
            .scripts_mut()
            .clear_failed_native_dynamic_module_import_fetch(failure)
    }

    pub(crate) fn register_import_map_source(
        &mut self,
        source: &str,
    ) -> std::result::Result<(), String> {
        let base_url = self.document.url().clone();
        self.register_import_map_source_with_base_url(source, &base_url)
    }

    pub(crate) fn register_import_map_source_with_base_url(
        &mut self,
        source: &str,
        base_url: &url::Url,
    ) -> std::result::Result<(), String> {
        self.script_lifecycle
            .scripts_mut()
            .register_import_map(source, base_url)
    }

    pub(crate) fn register_runtime_owned_import_map_source(
        &mut self,
        source: &str,
        base_url: &Url,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .register_dynamic_import_map(base_url, source);
    }

    pub(crate) fn resolve_module_specifier(
        &mut self,
        specifier: &str,
        base_url: &Url,
    ) -> std::result::Result<Url, String> {
        self.script_lifecycle
            .scripts_mut()
            .resolve_module_specifier(specifier, base_url)
    }

    pub(crate) fn resolve_module_integrity(&self, url: &Url) -> Option<String> {
        self.script_lifecycle
            .scripts()
            .resolve_module_integrity(url)
    }

    pub(crate) fn next_inline_module_eval_id(&mut self) -> u64 {
        self.script_lifecycle
            .scripts_mut()
            .next_inline_module_eval_id()
    }

    #[cfg(test)]
    pub(crate) fn insert_native_module_source(
        &mut self,
        key: ModuleMapKey,
        source: ModuleSource,
    ) -> ModuleEntryId {
        self.script_lifecycle
            .scripts_mut()
            .insert_native_module_source(key, source)
    }

    pub(crate) fn insert_native_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.script_lifecycle
            .scripts_mut()
            .insert_native_module_source_for_request(
                request_key,
                effective_key,
                source,
                effective_fetch_metadata,
            )
    }

    pub(crate) fn start_or_join_native_module_fetch(
        &mut self,
        key: ModuleMapKey,
    ) -> ModuleMapFetchDisposition {
        self.script_lifecycle
            .scripts_mut()
            .start_or_join_native_module_fetch(key)
    }

    pub(crate) fn insert_native_compiled_module_record_with_metadata(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.script_lifecycle
            .scripts_mut()
            .insert_native_compiled_module_record_with_metadata(
                request_key,
                record,
                identity,
                effective_fetch_metadata,
            )
    }

    pub(crate) fn native_module_entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId> {
        self.script_lifecycle.scripts().native_module_entry_id(key)
    }

    pub(crate) fn native_module_entry_state(
        &self,
        entry_id: ModuleEntryId,
    ) -> crate::module_runtime::ModuleMapEntryState {
        self.script_lifecycle
            .scripts()
            .native_module_entry_state(entry_id)
    }

    pub(crate) fn native_module_entry_key(&self, entry_id: ModuleEntryId) -> ModuleMapKey {
        self.script_lifecycle
            .scripts()
            .native_module_entry_key(entry_id)
    }

    pub(crate) fn native_module_effective_fetch_metadata(
        &self,
        entry_id: ModuleEntryId,
    ) -> ModuleFetchMetadata {
        self.script_lifecycle
            .scripts()
            .native_module_effective_fetch_metadata(entry_id)
    }

    pub(crate) fn native_module_source(&self, entry_id: ModuleEntryId) -> Option<ModuleSource> {
        self.script_lifecycle
            .scripts()
            .native_module_source(entry_id)
    }

    pub(crate) fn native_module_source_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<(ModuleMapKey, ModuleSource)> {
        self.script_lifecycle
            .scripts()
            .native_module_source_for(module)
    }

    pub(crate) fn native_module_wasm_record_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<WasmModuleRecord> {
        self.script_lifecycle
            .scripts()
            .native_module_wasm_record_for(module)
    }

    pub(crate) fn native_module_wasm_record(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<WasmModuleRecord> {
        self.script_lifecycle
            .scripts()
            .native_module_wasm_record(entry_id)
    }

    pub(crate) fn native_wasm_instance_for_namespace<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        namespace: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.script_lifecycle
            .scripts()
            .native_wasm_instance_for_namespace(scope, namespace)
    }

    pub(crate) fn native_resolved_dependency_module_for(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &crate::module_runtime::ModuleAttributesKey,
    ) -> Option<v8::Global<v8::Module>> {
        self.script_lifecycle
            .scripts()
            .native_resolved_dependency_module_for(referrer, specifier, attributes)
    }

    pub(crate) fn native_module_entry_url(&self, entry_id: ModuleEntryId) -> Url {
        self.script_lifecycle
            .scripts()
            .native_module_entry_url(entry_id)
    }

    pub(crate) fn native_module_failure(&self, entry_id: ModuleEntryId) -> Option<ModuleLoadError> {
        self.script_lifecycle
            .scripts()
            .native_module_failure(entry_id)
    }

    pub(crate) fn native_module_requests(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<Vec<ModuleRequestRecord>> {
        self.script_lifecycle
            .scripts()
            .native_module_requests(entry_id)
    }

    pub(crate) fn set_native_module_resolved_dependencies(
        &mut self,
        entry_id: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .set_native_module_resolved_dependencies(entry_id, dependencies);
    }

    #[cfg(test)]
    pub(crate) fn native_module_resolved_dependencies(
        &self,
        entry_id: ModuleEntryId,
    ) -> Vec<ModuleResolvedDependency> {
        self.script_lifecycle
            .scripts()
            .native_module_resolved_dependencies(entry_id)
    }

    pub(crate) fn native_document_modulator_ptr(
        &self,
    ) -> *const crate::module_runtime::NativeDocumentModulator {
        self.script_lifecycle
            .scripts()
            .native_document_modulator_ptr()
    }

    pub(crate) fn native_compiled_module(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<v8::Global<v8::Module>> {
        self.script_lifecycle
            .scripts()
            .native_compiled_module(entry_id)
    }

    pub(crate) fn mark_native_module_instantiated(&mut self, entry_id: ModuleEntryId) {
        self.script_lifecycle
            .scripts_mut()
            .mark_native_module_instantiated(entry_id);
    }

    pub(crate) fn mark_native_module_evaluating(&mut self, entry_id: ModuleEntryId) {
        self.script_lifecycle
            .scripts_mut()
            .mark_native_module_evaluating(entry_id);
    }

    pub(crate) fn mark_native_module_evaluated(&mut self, entry_id: ModuleEntryId) {
        self.script_lifecycle
            .scripts_mut()
            .mark_native_module_evaluated(entry_id);
    }

    pub(crate) fn mark_native_module_failed(
        &mut self,
        key: ModuleMapKey,
        error: crate::module_runtime::ModuleLoadError,
    ) -> ModuleEntryId {
        self.script_lifecycle
            .scripts_mut()
            .mark_native_module_failed(key, error)
    }

    pub(crate) fn queue_native_dynamic_module_import(
        &mut self,
        request: crate::module_runtime::PendingDynamicModuleImport,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .queue_native_dynamic_module_import(request);
    }

    pub(crate) fn take_next_native_dynamic_module_import(
        &mut self,
    ) -> Option<crate::module_runtime::NativeModuleGraphJob> {
        self.script_lifecycle
            .scripts_mut()
            .take_next_native_dynamic_module_import()
    }

    pub(crate) fn resume_native_dynamic_module_import_front(
        &mut self,
        job: crate::module_runtime::NativeModuleGraphJob,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .resume_native_dynamic_module_import_front(job);
    }

    pub(crate) fn reserve_native_dynamic_module_evaluation_reaction(
        &mut self,
        request: crate::module_runtime::PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
    ) -> u64 {
        self.script_lifecycle
            .scripts_mut()
            .reserve_native_dynamic_module_evaluation_reaction(request, target)
    }

    pub(crate) fn native_dynamic_module_evaluation_reaction_owner(
        &self,
        reaction_id: u64,
    ) -> Option<crate::module_runtime::DynamicModuleImportOwner> {
        self.script_lifecycle
            .scripts()
            .native_dynamic_module_evaluation_reaction_owner(reaction_id)
    }

    pub(crate) fn take_native_dynamic_module_evaluation_reaction(
        &mut self,
        reaction_id: u64,
        expected_owner: crate::module_runtime::DynamicModuleImportOwner,
    ) -> Option<PendingDynamicModuleEvaluationReaction> {
        self.script_lifecycle
            .scripts_mut()
            .take_native_dynamic_module_evaluation_reaction(reaction_id, expected_owner)
    }

    pub(crate) fn retire_native_dynamic_module_import_execution_context(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) -> DynamicModuleExecutionContextRetirement {
        self.script_lifecycle
            .scripts_mut()
            .retire_native_dynamic_module_import_execution_context(owner)
    }

    pub(crate) fn suspend_native_module_script_fetch(
        &mut self,
        continuation: ModuleScriptGraphFetchContinuation,
    ) -> u64 {
        self.script_lifecycle
            .scripts_mut()
            .suspend_native_module_script_fetch(continuation)
    }

    pub(crate) fn take_inflight_native_module_script_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<ModuleScriptGraphFetchContinuation> {
        self.script_lifecycle
            .scripts_mut()
            .take_inflight_native_module_script_fetch(load_id)
    }

    pub(crate) fn has_inflight_native_module_script_fetch(&self, load_id: u64) -> bool {
        self.script_lifecycle
            .scripts()
            .has_inflight_native_module_script_fetch(load_id)
    }

    pub(crate) fn suspend_native_dynamic_module_import_fetches(
        &mut self,
        requests: Vec<NativeModuleGraphFetchRequest>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        job: crate::module_runtime::NativeModuleGraphJob,
        owner_module_fetch_starts: Vec<
            Option<crate::frame_owner_model::FrameDocumentModuleFetchClientStart>,
        >,
    ) -> Vec<crate::module_runtime::DynamicModuleScheduledFetch> {
        self.script_lifecycle
            .scripts_mut()
            .suspend_native_dynamic_module_import_fetches(
                requests,
                joined_clients,
                job,
                owner_module_fetch_starts,
            )
    }

    pub(crate) fn has_pending_native_dynamic_module_import(&self) -> bool {
        self.script_lifecycle
            .scripts()
            .has_pending_native_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_ready_native_dynamic_module_import(&self) -> bool {
        self.script_lifecycle
            .scripts()
            .has_ready_native_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_native_dynamic_module_import_fetch(&self) -> bool {
        self.script_lifecycle
            .scripts()
            .has_inflight_native_dynamic_module_import_fetch()
    }

    pub(crate) fn fetch_single_native_module_for_modulepreload(
        &mut self,
        request: NativeModuleSingleFetchRequest,
    ) -> std::result::Result<NativeModulepreloadFetchStart, ModuleLoadError> {
        self.fetch_single_native_module_for_modulepreload_client(request, None)
            .map(|(fetch_start, _)| fetch_start)
    }

    pub(crate) fn fetch_single_native_module_for_modulepreload_link(
        &mut self,
        request: NativeModuleSingleFetchRequest,
        link_client: std::sync::Arc<crate::module_runtime::NativeModulepreloadLinkClient>,
    ) -> std::result::Result<NativeModulepreloadLinkFetchOutcome, ModuleLoadError> {
        self.fetch_single_native_module_for_modulepreload_client(request, Some(link_client))
            .map(|(fetch_start, pending_event)| {
                NativeModulepreloadLinkFetchOutcome::new(fetch_start, pending_event)
            })
    }

    fn fetch_single_native_module_for_modulepreload_client(
        &mut self,
        request: NativeModuleSingleFetchRequest,
        link_client: Option<std::sync::Arc<crate::module_runtime::NativeModulepreloadLinkClient>>,
    ) -> std::result::Result<
        (
            NativeModulepreloadFetchStart,
            Option<PendingNativeModulepreloadLinkEvent>,
        ),
        ModuleLoadError,
    > {
        let key = request.module_key().clone();
        match self.start_or_join_native_module_fetch(key.clone()) {
            ModuleMapFetchDisposition::StartedFetch(_) => {
                if let Some(client) = link_client {
                    self.suspend_native_modulepreload_link_clients(key, vec![client]);
                }
                Ok((
                    NativeModulepreloadFetchStart::Started(Box::new(request)),
                    None,
                ))
            }
            ModuleMapFetchDisposition::JoinedFetching(_) => {
                if let Some(client) = link_client {
                    self.suspend_native_modulepreload_link_clients(key, vec![client]);
                }
                Ok((NativeModulepreloadFetchStart::Joined, None))
            }
            ModuleMapFetchDisposition::AlreadyFetched(_)
            | ModuleMapFetchDisposition::AlreadyCompiled(_) => {
                let pending_event = link_client.and_then(|client| {
                    self.accept_native_modulepreload_link_client_terminals(&key, vec![client], true)
                        .into_iter()
                        .next()
                });
                Ok((
                    NativeModulepreloadFetchStart::AlreadyComplete,
                    pending_event,
                ))
            }
            ModuleMapFetchDisposition::AlreadyFailed(_) => {
                let pending_event = link_client.and_then(|client| {
                    self.accept_native_modulepreload_link_client_terminals(
                        &key,
                        vec![client],
                        false,
                    )
                    .into_iter()
                    .next()
                });
                Ok((
                    NativeModulepreloadFetchStart::AlreadyComplete,
                    pending_event,
                ))
            }
        }
    }

    pub(crate) fn suspend_native_modulepreload_fetch(
        &mut self,
        request: NativeModuleSingleFetchRequest,
    ) -> u64 {
        self.script_lifecycle
            .scripts_mut()
            .suspend_native_modulepreload_fetch(request)
    }

    pub(crate) fn take_inflight_native_modulepreload_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<NativeModuleSingleFetchRequest> {
        self.script_lifecycle
            .scripts_mut()
            .take_inflight_native_modulepreload_fetch(load_id)
    }

    pub(crate) fn suspend_native_modulepreload_link_clients(
        &mut self,
        key: ModuleMapKey,
        clients: Vec<std::sync::Arc<crate::module_runtime::NativeModulepreloadLinkClient>>,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .suspend_native_modulepreload_link_clients(key, clients);
    }

    pub(crate) fn post_modulepreload_link_error_event(&mut self, link: DomHandle) {
        self.script_lifecycle
            .scripts_mut()
            .post_modulepreload_link_error_event(link);
    }

    pub(crate) fn suspend_native_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: crate::module_runtime::NativeModuleMapSingleModuleClient,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .suspend_native_module_fetch_waiter(key, client);
    }

    pub(crate) fn detach_native_module_fetch_waiter(
        &mut self,
        client: moli_module_script_tree::SingleModuleClientToken,
    ) -> bool {
        self.script_lifecycle
            .scripts_mut()
            .detach_native_module_fetch_waiter(client)
    }

    pub(crate) fn take_next_native_module_owner_event(
        &mut self,
    ) -> Option<crate::module_runtime::NativeModuleOwnerEvent> {
        self.script_lifecycle
            .scripts_mut()
            .take_next_native_module_owner_event()
    }

    pub(crate) fn has_ready_native_module_owner_event(&mut self) -> bool {
        self.script_lifecycle
            .scripts_mut()
            .has_ready_native_module_owner_event()
    }

    #[cfg(test)]
    pub(crate) fn has_native_module_script_fetch_waiters(&self) -> bool {
        self.script_lifecycle
            .scripts()
            .has_native_module_script_fetch_waiters()
    }

    #[cfg(test)]
    pub(crate) fn native_module_script_client_count_for_testing(&self) -> usize {
        self.script_lifecycle
            .scripts()
            .module_script_client_count_for_testing()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_native_modulepreload_fetch(&self) -> bool {
        self.script_lifecycle
            .scripts()
            .has_inflight_native_modulepreload_fetch()
    }

    #[cfg(test)]
    pub(crate) fn absorb_runtime_binding_calls(
        &mut self,
        calls: Vec<PendingRuntimeBindingCall>,
    ) -> Vec<String> {
        for call in calls {
            self.pending_runtime_binding_calls.push(call);
        }

        Vec::new()
    }

    #[cfg(test)]
    pub(crate) fn absorb_runtime_binding_calls_from_host(
        &mut self,
        host: &mut JsContextHost,
    ) -> Vec<String> {
        self.absorb_runtime_binding_calls(host.take_runtime_binding_calls())
    }

    #[cfg(test)]
    pub(crate) fn take_runtime_binding_calls(&mut self) -> Vec<PendingRuntimeBindingCall> {
        std::mem::take(&mut self.pending_runtime_binding_calls)
    }

    pub(crate) fn pending_runtime_binding_call_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_runtime_binding_calls.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }
}
