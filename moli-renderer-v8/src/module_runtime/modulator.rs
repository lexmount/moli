use std::sync::Arc;

use moli_module_script_tree as module_tree;
use url::Url;

use crate::document_module_graph::{
    DocumentModuleMapCore, ModuleAttributesKey, ModuleEntryId, ModuleFetchMetadata,
    ModuleIdentityHash, ModuleImportPhase, ModuleLoadError, ModuleMapEntry,
    ModuleMapFetchDisposition, ModuleMapKey, ModuleMapTerminalNotification,
    ModuleResolvedDependency, ModuleSource, NativeModuleMapSingleModuleClient,
    NativeModulepreloadLinkClient,
};
use crate::frame_owner_model::{
    FrameDocumentModuleClientId, FrameDocumentModuleFetchClientStart,
    FrameDocumentParserRootTerminalClient,
};

use super::NativeModuleOwnerEvent;
use super::dynamic_resolver::DynamicModuleResolver;
use super::dynamic_resolver::{
    DynamicModuleEvaluationTarget, DynamicModuleExecutionContextRetirement,
    DynamicModuleFetchContinuation, DynamicModuleFetchFailure, DynamicModuleFetchOwnerAdvance,
    DynamicModuleFetchResume, DynamicModuleImportOwner, DynamicModuleInflightFetch,
    DynamicModuleJoinedFetch, DynamicModuleScheduledFetch, PendingDynamicModuleEvaluationReaction,
    PendingDynamicModuleImport,
};
use super::graph::{
    NativeDynamicModuleImportReady, NativeModuleGraphFetchRequest, NativeModuleGraphJob,
    NativeModuleGraphJobAdvance,
};
use super::module_identity_hash_from_v8_module;
use super::owner_ids::NativeDocumentModuleOwnerIds;
use super::parser_tree_registry::{
    NativeParserModuleTreeJobRegistry, NativeParserModuleTreeJobResume, NativeParserModuleTreeRoot,
};
use super::record::{ModuleRecordEntry, ModuleRecordState, WasmModuleRecord};
use super::record_resolver::NativeModuleRecordResolver;
use super::single_module_fetch::{NativeModuleSingleFetchQueue, NativeModuleSingleFetchRequest};

#[derive(Debug, Default)]
pub(crate) struct NativeDocumentModulator {
    core: DocumentModuleMapCore,
    records: NativeModuleRecordResolver,
    dynamic_resolver: DynamicModuleResolver,
    parser_tree_jobs: NativeParserModuleTreeJobRegistry,
    modulepreload_single_fetch_queue: NativeModuleSingleFetchQueue,
    owner_ids: NativeDocumentModuleOwnerIds,
}

pub(crate) struct NativeFrameDocumentDependencyFetchBuildFailure {
    error: ModuleLoadError,
    dependency_key: ModuleMapKey,
    parent_entry_id: Option<ModuleEntryId>,
}

impl NativeFrameDocumentDependencyFetchBuildFailure {
    pub(super) fn new(
        error: ModuleLoadError,
        dependency_key: ModuleMapKey,
        parent_entry_id: Option<ModuleEntryId>,
    ) -> Self {
        Self {
            error,
            dependency_key,
            parent_entry_id,
        }
    }

    pub(crate) fn error(&self) -> &ModuleLoadError {
        &self.error
    }

    pub(crate) fn dependency_key(&self) -> &ModuleMapKey {
        &self.dependency_key
    }

    pub(crate) fn parent_entry_id(&self) -> Option<ModuleEntryId> {
        self.parent_entry_id
    }

    pub(crate) fn into_error(self) -> ModuleLoadError {
        self.error
    }
}

impl NativeDocumentModulator {
    /// Clears work owned by the replaced Document without destroying the
    /// current realm's module environment.
    ///
    /// Chromium's `Modulator::From(ScriptState*)` stores the module map and
    /// dynamic resolver in per-context data. `Document::open()` creates a new
    /// parser but returns the same Document and keeps that ScriptState alive.
    /// Moli installs a new exact Document owner, so this explicit
    /// split prevents that implementation detail from canceling `import()`.
    pub(crate) fn clear_for_document_replacement(&mut self) {
        self.core
            .retain_script_state_clients_for_document_replacement();
        self.parser_tree_jobs.clear();
        // A modulepreload network request populates the Modulator's module map,
        // which is owned by the live ScriptState rather than by the connected
        // <link>. document.open() replaces parser/Document-owned clients but
        // keeps that ScriptState. Retain the exact in-flight request so its
        // terminal can still settle the realm cache without dispatching an
        // event or Resource Timing entry for the replacement Document.
    }

    pub(crate) fn start_or_join_fetch(&mut self, key: ModuleMapKey) -> ModuleMapFetchDisposition {
        self.core.start_or_join_fetch(key)
    }

    pub(crate) fn reserve_module_graph_fetch_load_id(&mut self) -> u64 {
        self.owner_ids.reserve_module_graph_fetch_load_id()
    }

    pub(crate) fn reserve_module_graph_fetches(
        &mut self,
        requests: Vec<NativeModuleGraphFetchRequest>,
    ) -> Vec<(u64, NativeModuleGraphFetchRequest)> {
        let mut fetches = Vec::with_capacity(requests.len());
        for request in requests {
            let load_id = self.reserve_module_graph_fetch_load_id();
            fetches.push((load_id, request));
        }
        fetches
    }

    pub(super) fn reserve_parser_root_module_client_id(&mut self) -> FrameDocumentModuleClientId {
        self.owner_ids.reserve_parser_root_module_client_id()
    }

    #[cfg(test)]
    pub(crate) fn start_or_join_module_fetch(
        &mut self,
        key: ModuleMapKey,
    ) -> ModuleMapFetchDisposition {
        self.start_or_join_fetch(key)
    }

    pub(crate) fn add_single_module_fetch_client(
        &mut self,
        key: ModuleMapKey,
        client: NativeModuleMapSingleModuleClient,
    ) {
        self.core.add_single_module_fetch_client(key, client);
    }

    #[cfg(test)]
    pub(crate) fn suspend_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: NativeModuleMapSingleModuleClient,
    ) {
        self.add_single_module_fetch_client(key, client);
    }

    pub(crate) fn detach_single_module_fetch_client(
        &mut self,
        client: module_tree::SingleModuleClientToken,
    ) -> bool {
        self.core.detach_single_module_fetch_client(client)
    }

    pub(crate) fn queue_dynamic_module_import(&mut self, request: PendingDynamicModuleImport) {
        self.dynamic_resolver.queue_import(request);
    }

    pub(crate) fn take_next_dynamic_module_import(&mut self) -> Option<NativeModuleGraphJob> {
        self.dynamic_resolver.take_next_import()
    }

    pub(crate) fn resume_dynamic_module_import_front(&mut self, job: NativeModuleGraphJob) {
        self.dynamic_resolver.resume_import_front(job);
    }

    pub(crate) fn reserve_dynamic_module_evaluation_reaction(
        &mut self,
        request: PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
    ) -> u64 {
        self.dynamic_resolver
            .reserve_evaluation_reaction(request, target)
    }

    pub(crate) fn dynamic_module_evaluation_reaction_owner(
        &self,
        reaction_id: u64,
    ) -> Option<DynamicModuleImportOwner> {
        self.dynamic_resolver.evaluation_reaction_owner(reaction_id)
    }

    pub(crate) fn take_dynamic_module_evaluation_reaction(
        &mut self,
        reaction_id: u64,
        expected_owner: DynamicModuleImportOwner,
    ) -> Option<PendingDynamicModuleEvaluationReaction> {
        self.dynamic_resolver
            .take_evaluation_reaction(reaction_id, expected_owner)
    }

    pub(crate) fn retire_dynamic_module_import_execution_context(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) -> DynamicModuleExecutionContextRetirement {
        self.dynamic_resolver.retire_execution_context_owner(owner)
    }

    pub(crate) fn suspend_dynamic_module_import_fetches(
        &mut self,
        fetches: Vec<(u64, NativeModuleGraphFetchRequest)>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
        job: NativeModuleGraphJob,
        owner_module_fetch_starts: Vec<Option<FrameDocumentModuleFetchClientStart>>,
    ) -> Vec<DynamicModuleScheduledFetch> {
        let scheduled_fetches = scheduled_dynamic_module_fetches_from_reserved_fetches(
            &fetches,
            &owner_module_fetch_starts,
        );
        let owner_module_fetch_starts = fetches
            .iter()
            .map(|(load_id, _)| *load_id)
            .zip(owner_module_fetch_starts)
            .filter_map(|(load_id, owner_start)| owner_start.map(|start| (load_id, start)))
            .collect();
        self.dynamic_resolver.suspend_fetches(
            fetches,
            joined_clients,
            job,
            owner_module_fetch_starts,
        );
        scheduled_fetches
    }

    pub(crate) fn take_inflight_dynamic_module_import_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<DynamicModuleInflightFetch> {
        self.dynamic_resolver.take_inflight_fetch(load_id)
    }

    pub(crate) fn inflight_dynamic_module_import_fetch_owner(
        &self,
        load_id: u64,
    ) -> Option<DynamicModuleImportOwner> {
        self.dynamic_resolver.inflight_fetch_owner(load_id)
    }

    pub(crate) fn take_joined_dynamic_module_import_fetch(
        &mut self,
        client: module_tree::SingleModuleClientToken,
    ) -> Option<DynamicModuleJoinedFetch> {
        self.dynamic_resolver.take_joined_fetch(client)
    }

    pub(crate) fn restore_dynamic_module_import_fetch_as_joined_owner_client(
        &mut self,
        inflight: DynamicModuleInflightFetch,
    ) -> Option<module_tree::SingleModuleClientToken> {
        self.dynamic_resolver
            .restore_inflight_fetch_as_joined_owner_module_client(inflight)
    }

    pub(crate) fn continue_dynamic_module_import_fetch(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        fetches: Vec<(u64, NativeModuleGraphFetchRequest)>,
        owner_module_fetch_starts: Vec<Option<FrameDocumentModuleFetchClientStart>>,
    ) -> DynamicModuleFetchOwnerAdvance {
        let (mut resume, advance) = continuation.into_parts();
        match advance {
            NativeModuleGraphJobAdvance::NeedFetches(requests) => {
                debug_assert_eq!(
                    requests.len(),
                    fetches.len(),
                    "dynamic module continuation fetch load-id allocation must match requested fetches"
                );
                let joined_clients = resume.take_pending_joined_clients();
                let scheduled_fetches = self.extend_dynamic_module_pending_tree(
                    resume,
                    fetches,
                    joined_clients,
                    owner_module_fetch_starts,
                );
                DynamicModuleFetchOwnerAdvance::Waiting { scheduled_fetches }
            }
            NativeModuleGraphJobAdvance::WaitingForFetches => {
                let joined_clients = resume.take_pending_joined_clients();
                self.extend_dynamic_module_pending_tree(
                    resume,
                    Vec::new(),
                    joined_clients,
                    Vec::new(),
                );
                DynamicModuleFetchOwnerAdvance::Waiting {
                    scheduled_fetches: Vec::new(),
                }
            }
            NativeModuleGraphJobAdvance::Complete(graph) => {
                if self.dynamic_module_fetch_resume_has_pending_waits(&resume) {
                    self.restore_dynamic_module_fetch_resume(resume);
                    return DynamicModuleFetchOwnerAdvance::RestoredAfterUnexpectedComplete;
                }
                DynamicModuleFetchOwnerAdvance::Ready(Box::new(NativeDynamicModuleImportReady {
                    job: resume.into_job(),
                    graph,
                }))
            }
        }
    }

    pub(in crate::module_runtime) fn extend_dynamic_module_pending_tree(
        &mut self,
        resume: DynamicModuleFetchResume,
        fetches: Vec<(u64, NativeModuleGraphFetchRequest)>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
        owner_module_fetch_starts: Vec<Option<FrameDocumentModuleFetchClientStart>>,
    ) -> Vec<DynamicModuleScheduledFetch> {
        let scheduled_fetches = scheduled_dynamic_module_fetches_from_reserved_fetches(
            &fetches,
            &owner_module_fetch_starts,
        );
        let owner_module_fetch_starts = fetches
            .iter()
            .map(|(load_id, _)| *load_id)
            .zip(owner_module_fetch_starts)
            .filter_map(|(load_id, owner_start)| owner_start.map(|start| (load_id, start)))
            .collect();
        self.dynamic_resolver.extend_pending_tree(
            resume,
            fetches,
            joined_clients,
            owner_module_fetch_starts,
        );
        scheduled_fetches
    }

    pub(in crate::module_runtime) fn restore_dynamic_module_fetch_resume(
        &mut self,
        resume: DynamicModuleFetchResume,
    ) {
        self.dynamic_resolver.restore_fetch_resume(resume);
    }

    pub(in crate::module_runtime) fn dynamic_module_fetch_resume_has_pending_waits(
        &self,
        resume: &DynamicModuleFetchResume,
    ) -> bool {
        self.dynamic_resolver.fetch_resume_has_pending_waits(resume)
    }

    pub(crate) fn clear_failed_dynamic_module_import_fetch(
        &mut self,
        failure: DynamicModuleFetchFailure,
    ) -> (
        Vec<module_tree::SingleModuleClientToken>,
        PendingDynamicModuleImport,
        ModuleLoadError,
    ) {
        let (resume, error) = failure.into_parts();
        let (joined_clients, job) = self.dynamic_resolver.clear_fetch_resume(resume);
        (joined_clients, job.into_dynamic_import_request(), error)
    }

    pub(crate) fn has_pending_dynamic_module_import(&self) -> bool {
        self.dynamic_resolver.has_pending_import()
    }

    pub(crate) fn has_ready_dynamic_module_import(&self) -> bool {
        self.dynamic_resolver.has_ready_import()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_dynamic_module_import_fetch(&self) -> bool {
        self.dynamic_resolver.has_inflight_fetch()
    }

    pub(crate) fn take_next_terminal_notification(
        &mut self,
    ) -> Option<ModuleMapTerminalNotification> {
        self.core.take_next_terminal_notification()
    }

    pub(crate) fn drain_ready_owner_events(
        &mut self,
        mut on_event: impl FnMut(NativeModuleOwnerEvent),
    ) {
        while let Some(notification) = self.take_next_terminal_notification() {
            on_event(NativeModuleOwnerEvent::ModuleMapTerminalNotification(
                notification,
            ));
        }
    }

    pub(crate) fn insert_parser_module_tree_job(
        &mut self,
        root: NativeParserModuleTreeRoot,
        job: NativeModuleGraphJob,
    ) {
        self.parser_tree_jobs.insert(root, job);
    }

    pub(crate) fn take_parser_module_tree_job(
        &mut self,
        tree_id: module_tree::ModuleTreeId,
    ) -> Option<NativeParserModuleTreeJobResume> {
        self.parser_tree_jobs.take(tree_id)
    }

    pub(crate) fn restore_parser_module_tree_job(
        &mut self,
        resume: NativeParserModuleTreeJobResume,
    ) {
        self.parser_tree_jobs.restore(resume);
    }

    #[cfg(test)]
    pub(crate) fn has_parser_module_tree_job_for_test(
        &self,
        tree_id: module_tree::ModuleTreeId,
    ) -> bool {
        self.parser_tree_jobs.contains(tree_id)
    }

    pub(crate) fn add_modulepreload_link_client(
        &mut self,
        key: ModuleMapKey,
        client: Arc<NativeModulepreloadLinkClient>,
    ) {
        self.core.add_modulepreload_link_client(key, client);
    }

    pub(crate) fn add_terminal_modulepreload_link_client(
        &mut self,
        key: ModuleMapKey,
        client: Arc<NativeModulepreloadLinkClient>,
    ) {
        self.core
            .add_terminal_modulepreload_link_client(key, client);
    }

    pub(crate) fn suspend_modulepreload_fetch(
        &mut self,
        load_id: u64,
        request: NativeModuleSingleFetchRequest,
    ) {
        self.modulepreload_single_fetch_queue
            .suspend_fetch(load_id, request);
    }

    pub(crate) fn reserve_modulepreload_fetch(
        &mut self,
        request: NativeModuleSingleFetchRequest,
    ) -> (u64, NativeModuleGraphFetchRequest) {
        let fetch_request = request.fetch_request();
        let load_id = self.reserve_module_graph_fetch_load_id();
        self.suspend_modulepreload_fetch(load_id, request);
        (load_id, fetch_request)
    }

    pub(crate) fn take_inflight_modulepreload_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<NativeModuleSingleFetchRequest> {
        self.modulepreload_single_fetch_queue
            .take_inflight_fetch(load_id)
    }

    pub(crate) fn has_inflight_modulepreload_fetch_for(&self, load_id: u64) -> bool {
        self.modulepreload_single_fetch_queue
            .has_inflight_fetch_for(load_id)
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_modulepreload_fetch(&self) -> bool {
        self.modulepreload_single_fetch_queue.has_inflight_fetch()
    }

    pub(crate) fn add_parser_root_module_client(
        &mut self,
        key: ModuleMapKey,
        client: FrameDocumentParserRootTerminalClient,
    ) {
        self.core.add_parser_root_module_client(key, client);
    }

    #[cfg(test)]
    pub(crate) fn suspend_modulepreload_link_client(
        &mut self,
        key: ModuleMapKey,
        client: Arc<NativeModulepreloadLinkClient>,
    ) {
        self.add_modulepreload_link_client(key, client);
    }

    #[cfg(test)]
    pub(crate) fn fetch_client_count(&self) -> usize {
        self.core.fetch_client_count()
    }

    #[cfg(test)]
    pub(crate) fn fetch_client_count_for_testing(&self) -> usize {
        self.fetch_client_count()
    }

    #[cfg(test)]
    pub(crate) fn single_module_fetch_client_count(&self) -> usize {
        self.core.single_module_fetch_client_count()
    }

    #[cfg(test)]
    pub(crate) fn single_module_fetch_client_count_for_testing(&self) -> usize {
        self.single_module_fetch_client_count()
    }

    #[cfg(test)]
    pub(crate) fn module_script_client_count(&self) -> usize {
        self.core.module_script_client_count()
    }

    #[cfg(test)]
    pub(crate) fn module_script_client_count_for_testing(&self) -> usize {
        self.module_script_client_count()
    }

    #[cfg(test)]
    pub(crate) fn modulepreload_link_client_count(&self) -> usize {
        self.core.modulepreload_link_client_count()
    }

    #[cfg(test)]
    pub(crate) fn modulepreload_link_client_count_for_testing(&self) -> usize {
        self.modulepreload_link_client_count()
    }

    #[cfg(test)]
    pub(crate) fn parser_root_module_client_count_for_testing(&self) -> usize {
        self.core.parser_root_module_client_count()
    }

    #[cfg(test)]
    pub(crate) fn insert_fetched_source(
        &mut self,
        key: ModuleMapKey,
        source: ModuleSource,
    ) -> ModuleEntryId {
        let (entry_id, cleared) = self.core.insert_fetched_source(key, source);
        self.records.detach_cleared_record(entry_id, cleared);
        entry_id
    }

    #[cfg(test)]
    pub(crate) fn insert_module_source(
        &mut self,
        key: ModuleMapKey,
        source: ModuleSource,
    ) -> ModuleEntryId {
        self.insert_fetched_source(key, source)
    }

    pub(crate) fn insert_fetched_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        let (entry_id, cleared) = self.core.insert_fetched_source_for_request(
            request_key,
            effective_key,
            source,
            effective_fetch_metadata,
        );
        self.records.detach_cleared_record(entry_id, cleared);
        entry_id
    }

    pub(crate) fn get_compiled(&self, key: &ModuleMapKey) -> Option<&ModuleRecordEntry> {
        self.entry_id(key)
            .and_then(|entry_id| self.usable_record(entry_id))
    }

    pub(crate) fn entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId> {
        self.core.entry_id(key)
    }

    #[cfg(test)]
    pub(crate) fn module_entry_state(
        &self,
        entry_id: ModuleEntryId,
    ) -> crate::document_module_graph::ModuleMapEntryState {
        self.entry(entry_id).state()
    }

    pub(crate) fn mark_failed(
        &mut self,
        key: ModuleMapKey,
        error: ModuleLoadError,
    ) -> ModuleEntryId {
        let (entry_id, cleared) = self.core.mark_failed(key, error);
        self.records.detach_cleared_record(entry_id, cleared);
        entry_id
    }

    pub(crate) fn insert_compiled_record_with_metadata(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        let effective_key = record.key().clone();
        let record_id = self.records.insert_record(record);
        let (entry_id, cleared) = self.core.insert_compiled_record_with_metadata(
            request_key,
            effective_key,
            record_id,
            identity,
            effective_fetch_metadata,
        );
        self.records.detach_cleared_record(entry_id, cleared);
        self.records.register_entry_identity(entry_id, identity);
        entry_id
    }

    pub(crate) fn set_resolved_dependencies(
        &mut self,
        entry_id: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        self.core.set_resolved_dependencies(entry_id, dependencies);
    }

    #[cfg(test)]
    pub(crate) fn resolved_dependencies(
        &self,
        entry_id: ModuleEntryId,
    ) -> &[ModuleResolvedDependency] {
        self.core.resolved_dependencies(entry_id)
    }

    pub(crate) fn mark_instantiated(&mut self, entry_id: ModuleEntryId) {
        self.core.mark_instantiated(entry_id);
        self.records
            .set_record_state_for_entry(self.core.entry(entry_id), ModuleRecordState::Instantiated);
    }

    pub(crate) fn mark_evaluating(&mut self, entry_id: ModuleEntryId) {
        self.core.mark_evaluating(entry_id);
        self.records
            .set_record_state_for_entry(self.core.entry(entry_id), ModuleRecordState::Evaluating);
    }

    pub(crate) fn mark_evaluated(&mut self, entry_id: ModuleEntryId) {
        self.core.mark_evaluated(entry_id);
        self.records
            .set_record_state_for_entry(self.core.entry(entry_id), ModuleRecordState::Evaluated);
    }

    pub(crate) fn module_key_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<&ModuleMapKey> {
        self.module_entry_id_for(module)
            .map(|entry_id| self.entry(entry_id).effective_key())
    }

    pub(crate) fn module_source_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<(ModuleMapKey, ModuleSource)> {
        let entry = self.entry(self.module_entry_id_for(module)?);
        Some((entry.effective_key().clone(), entry.source()?.clone()))
    }

    pub(crate) fn module_wasm_record_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<WasmModuleRecord> {
        let entry = self.entry(self.module_entry_id_for(module)?);
        self.records.record_for_entry(entry)?.wasm_module().cloned()
    }

    pub(crate) fn module_wasm_record(&self, entry_id: ModuleEntryId) -> Option<WasmModuleRecord> {
        self.records
            .record_for_entry(self.entry(entry_id))?
            .wasm_module()
            .cloned()
    }

    pub(crate) fn wasm_instance_for_namespace<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        namespace: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        for entry in self.core.entries() {
            let Some(record) = self.records.usable_record_for_entry(entry) else {
                continue;
            };
            let Some(wasm_record) = record.wasm_module() else {
                continue;
            };
            let module = v8::Local::new(scope, record.compiled_module());
            if !matches!(
                module.get_status(),
                v8::ModuleStatus::Instantiated
                    | v8::ModuleStatus::Evaluating
                    | v8::ModuleStatus::Evaluated
            ) {
                continue;
            }
            let candidate = module.get_module_namespace();
            if namespace.strict_equals(candidate) {
                return wasm_record.instance(scope);
            }
        }
        None
    }

    pub(crate) fn resolved_dependency_module_for(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<v8::Global<v8::Module>> {
        self.resolve_static_dependency(referrer, specifier, attributes)
            .map(|record| record.compiled_module().clone())
    }

    pub(crate) fn evaluation_dependency_modules_for(
        &self,
        referrer: v8::Local<'_, v8::Module>,
    ) -> Option<Vec<v8::Global<v8::Module>>> {
        let referrer_entry_id = self.module_entry_id_for(referrer)?;
        let referrer_entry = self.entry(referrer_entry_id);
        let record = self.records.record_for_entry(referrer_entry)?;
        let mut dependencies = Vec::new();
        for request in record
            .requests()
            .iter()
            .filter(|request| request.phase() == ModuleImportPhase::Evaluation)
        {
            dependencies.push(self.resolved_dependency_module_for(
                referrer,
                request.specifier(),
                request.attributes(),
            )?);
        }
        Some(dependencies)
    }

    pub(crate) fn entry_url(&self, entry_id: ModuleEntryId) -> Url {
        self.entry(entry_id).effective_key().url().clone()
    }

    pub(crate) fn module_entry_id_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<ModuleEntryId> {
        let identity = module_identity_hash_from_v8_module(module);
        let candidates = self.records.entry_ids_for_identity(identity)?;
        candidates.iter().find_map(|entry_id| {
            let entry = self.entry(*entry_id);
            let record = self.records.usable_record_for_entry(entry)?;
            (module == record.compiled_module()).then_some(*entry_id)
        })
    }

    pub(crate) fn resolve_static_dependency(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<&ModuleRecordEntry> {
        let referrer_entry_id = self.module_entry_id_for(referrer)?;
        let referrer_entry = self.entry(referrer_entry_id);
        let referrer_url = referrer_entry.effective_key().url();
        let dependency = referrer_entry
            .resolved_dependencies()
            .iter()
            .find(|dependency| dependency.matches_request(specifier, attributes, referrer_url))?;
        self.get_compiled(dependency.resolved_key())
    }

    pub(crate) fn entry(&self, entry_id: ModuleEntryId) -> &ModuleMapEntry {
        self.core.entry(entry_id)
    }

    pub(crate) fn compiled_record(&self, entry_id: ModuleEntryId) -> Option<&ModuleRecordEntry> {
        self.records.record_for_entry(self.entry(entry_id))
    }

    fn usable_record(&self, entry_id: ModuleEntryId) -> Option<&ModuleRecordEntry> {
        self.records.usable_record_for_entry(self.entry(entry_id))
    }
}

fn scheduled_dynamic_module_fetches_from_reserved_fetches(
    fetches: &[(u64, NativeModuleGraphFetchRequest)],
    owner_module_fetch_starts: &[Option<FrameDocumentModuleFetchClientStart>],
) -> Vec<DynamicModuleScheduledFetch> {
    fetches
        .iter()
        .enumerate()
        .map(|(index, (load_id, request))| {
            DynamicModuleScheduledFetch::new(
                *load_id,
                request.clone(),
                owner_module_fetch_starts.get(index).and_then(Clone::clone),
            )
        })
        .collect()
}
