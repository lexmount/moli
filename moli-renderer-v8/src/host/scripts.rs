use crate::document_runtime::DeferredPageTaskLane;
use crate::frame_owner_model::MainDocumentScriptLoadDelayLease;
use crate::module_runtime::{
    DynamicModuleEvaluationTarget, DynamicModuleExecutionContextRetirement,
    DynamicModuleFetchContinuation, DynamicModuleFetchFailure, DynamicModuleFetchOwnerAdvance,
    DynamicModuleInflightFetch, DynamicModuleJoinedFetch, ModuleEntryId, ModuleFetchMetadata,
    ModuleIdentityHash, ModuleLoadError, ModuleLoadStage, ModuleMapEntryState, ModuleMapKey,
    ModuleRecordEntry, ModuleRequestRecord, ModuleResolvedDependency,
    ModuleScriptGraphFetchContinuation, ModuleSource, NativeModuleGraphFetchRequest,
    NativeModuleGraphJob, NativeModuleSingleFetchRequest, PendingDynamicModuleEvaluationReaction,
    PendingDynamicModuleImport, WasmModuleRecord,
};
#[cfg(test)]
use crate::page_task_queue::PageTask;
use crate::page_task_queue::{
    PageTaskSender, PostParseLifecycleWork, RendererPageMainDocumentRuntimeProducer,
    WindowScriptFailureReportTask,
};
use crate::{
    dom::{NodeId, native::NativeNodeId},
    {
        module_runtime::ModuleOwnerState,
        planning::{PreparedScript, ScriptSource},
        types::{ScriptErrorConstructorKind, ScriptKind, ScriptMode, ScriptSourceKind},
    },
};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::VecDeque;
use tracing::{debug, warn};
use url::Url;

mod loader;
mod main_document_admission;
mod policy;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use loader::*;
pub(crate) use runtime::*;

#[derive(Debug)]
pub(crate) struct HostScriptScheduler {
    #[cfg(test)]
    pending_dynamic_in_order_scripts: VecDeque<PreparedScript>,
    #[cfg(test)]
    pending_dynamic_importmap_in_order_scripts: VecDeque<PreparedScript>,
    #[cfg(test)]
    pending_dynamic_module_in_order_scripts: VecDeque<PreparedScript>,
    #[cfg(test)]
    pending_dynamic_async_scripts: VecDeque<PreparedScript>,
    #[cfg(test)]
    pending_failed_dynamic_scripts: VecDeque<FailedDynamicScript>,
    page_task_tx: Option<PageTaskSender>,
    main_document_runtime_producer: Option<RendererPageMainDocumentRuntimeProducer>,
    main_document_runtime_target: Option<crate::frame_owner_model::FrameDocumentTaskOwner>,
    main_document_completion_recheck_turn_queued: bool,
    dynamic_module_job_turn_queued: bool,
    native_module_owner_event_turn_queued: bool,
    module_owner: ModuleOwnerState,
    script_handles: HashMap<String, ScriptHandleState>,
    next_virtual_script_node_index: usize,
    next_dynamic_script_position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptEventKind {
    Load,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptEventTask {
    pub(crate) kind: ScriptEventKind,
    pub(crate) handle: String,
}

impl ScriptEventTask {
    pub(crate) fn new(kind: ScriptEventKind, handle: impl Into<String>) -> Self {
        Self {
            kind,
            handle: handle.into(),
        }
    }

    pub(crate) fn event_name(&self) -> &'static str {
        match self.kind {
            ScriptEventKind::Load => "load",
            ScriptEventKind::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptEventDispatchPolicy {
    Dispatch,
    Skip(ScriptEventSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptEventSkipReason {
    InlineClassicLoad,
    InlineModuleLoad,
    InlineImportMapError,
    ModuleGraphFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptHandleSource {
    Unknown,
    ParserOwned,
    DocumentWriteOwned,
    RuntimeOwned,
}

impl ScriptHandleSource {
    fn merge_registration(self, incoming: ScriptHandleSource) -> ScriptHandleSource {
        match (self, incoming) {
            (ScriptHandleSource::Unknown, source) => source,
            (source, ScriptHandleSource::Unknown) => source,
            (_, source) => source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptHandleExecutionSubject {
    PendingOrUnknown,
    NonExecutable,
    InlineClassicExecution,
    PreparedExecution,
    QueuedExecution,
    FailedQueuedExecution,
    SkippedExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScriptHostEventSubject {
    pub(crate) source: ScriptHandleSource,
    pub(crate) execution: ScriptHandleExecutionSubject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScriptEventPolicy {
    pub(crate) load: ScriptEventDispatchPolicy,
    pub(crate) error: ScriptEventDispatchPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScriptFailurePageTaskPolicy {
    pub(crate) load_event: ScriptEventDispatchPolicy,
    pub(crate) error_event: ScriptEventDispatchPolicy,
    pub(crate) report_window_failure: bool,
    pub(crate) load_event_after_window_failure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptPageTaskExecutionKind {
    Standard,
    RuntimeOwned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleFailurePolicy {
    TopLevelLoadFailure,
    ModuleTreeLoadFailure,
    GraphFailure,
    EvaluationFailure,
}

impl ModuleFailurePolicy {
    pub(crate) fn for_module_load_error(error: &ModuleLoadError) -> Self {
        if error.is_top_level_module_load_failure() {
            return Self::TopLevelLoadFailure;
        }
        match error.stage() {
            ModuleLoadStage::Fetch => Self::ModuleTreeLoadFailure,
            ModuleLoadStage::Evaluate if error.error_constructor().is_none() => {
                Self::EvaluationFailure
            }
            _ => Self::GraphFailure,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ScriptHandleState {
    node: NativeNodeId,
    source: ScriptHandleSource,
    start_state: ScriptHandleStartState,
    page_task_execution_kind: ScriptPageTaskExecutionKind,
    followup_lane: DeferredPageTaskLane,
    waits_for_blocking_stylesheets: bool,
    waits_until_dom_content_loaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptHandleStartState {
    Ready,
    Preparing,
    Committed(ScriptStartCommitKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptStartCommitKind {
    ExecuteInline,
    ExecutePrepared,
    RegisterImportMap,
    RejectImportMap,
    Queue,
    QueueFailed,
    Skip,
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct DynamicScriptBatch {
    pub(crate) in_order: VecDeque<PreparedScript>,
    pub(crate) importmap_in_order: VecDeque<PreparedScript>,
    pub(crate) module_in_order: VecDeque<PreparedScript>,
    pub(crate) async_scripts: VecDeque<PreparedScript>,
    pub(crate) failed_scripts: VecDeque<FailedDynamicScript>,
}

#[derive(Debug, Clone)]
pub(crate) struct FailedDynamicScript {
    pub(crate) script: PreparedScript,
    pub(crate) message: String,
    pub(crate) failure_kind: QueuedScriptFailureKind,
}

/// One concrete runtime-created script admitted to the stable Page source.
///
/// The payload is the executable residence. Its exact Document load-delay
/// binding is acquired synchronously before publication and moves with the
/// payload until the dynamic-script terminal consumes it.
#[derive(Debug)]
pub(crate) struct RuntimeScriptAdmission {
    payload: Box<RuntimeScriptAdmissionPayload>,
    load_delay_binding: MainDocumentScriptLoadDelayLease,
}

#[derive(Debug)]
pub(crate) enum RuntimeScriptAdmissionPayload {
    Script(PreparedScript),
    Failed(FailedDynamicScript),
}

impl RuntimeScriptAdmission {
    #[cfg(test)]
    pub(crate) fn new(
        payload: RuntimeScriptAdmissionPayload,
        load_delay_binding: MainDocumentScriptLoadDelayLease,
    ) -> Self {
        Self::from_boxed_payload(Box::new(payload), load_delay_binding)
    }

    pub(crate) fn from_boxed_payload(
        payload: Box<RuntimeScriptAdmissionPayload>,
        load_delay_binding: MainDocumentScriptLoadDelayLease,
    ) -> Self {
        Self {
            payload,
            load_delay_binding,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RuntimeScriptAdmissionPayload,
        MainDocumentScriptLoadDelayLease,
    ) {
        (*self.payload, self.load_delay_binding)
    }

    pub(crate) fn owner(&self) -> crate::frame_owner_model::FrameDocumentTaskOwner {
        self.load_delay_binding.owner()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueuedScriptFailureKind {
    Immediate,
    ModuleTopLevelLoad,
}

impl Default for HostScriptScheduler {
    fn default() -> Self {
        Self {
            #[cfg(test)]
            pending_dynamic_in_order_scripts: VecDeque::new(),
            #[cfg(test)]
            pending_dynamic_importmap_in_order_scripts: VecDeque::new(),
            #[cfg(test)]
            pending_dynamic_module_in_order_scripts: VecDeque::new(),
            #[cfg(test)]
            pending_dynamic_async_scripts: VecDeque::new(),
            #[cfg(test)]
            pending_failed_dynamic_scripts: VecDeque::new(),
            page_task_tx: None,
            main_document_runtime_producer: None,
            main_document_runtime_target: None,
            main_document_completion_recheck_turn_queued: false,
            dynamic_module_job_turn_queued: false,
            native_module_owner_event_turn_queued: false,
            module_owner: ModuleOwnerState::default(),
            script_handles: HashMap::new(),
            next_virtual_script_node_index: 1_000_000,
            next_dynamic_script_position: 0,
        }
    }
}

impl HostScriptScheduler {
    pub(crate) fn native_module_owner(&self) -> &ModuleOwnerState {
        &self.module_owner
    }

    fn default_page_task_execution_kind_for_source(
        source: ScriptHandleSource,
    ) -> ScriptPageTaskExecutionKind {
        match source {
            ScriptHandleSource::RuntimeOwned => ScriptPageTaskExecutionKind::RuntimeOwned,
            ScriptHandleSource::ParserOwned
            | ScriptHandleSource::Unknown
            | ScriptHandleSource::DocumentWriteOwned => ScriptPageTaskExecutionKind::Standard,
        }
    }

    fn default_followup_lane_for_source(source: ScriptHandleSource) -> DeferredPageTaskLane {
        match source {
            ScriptHandleSource::RuntimeOwned => DeferredPageTaskLane::PostDomContentLoaded,
            ScriptHandleSource::ParserOwned
            | ScriptHandleSource::Unknown
            | ScriptHandleSource::DocumentWriteOwned => DeferredPageTaskLane::PreDomContentLoaded,
        }
    }

    pub(crate) fn followup_lane_for_script(
        source: ScriptHandleSource,
        mode: ScriptMode,
    ) -> DeferredPageTaskLane {
        match source {
            ScriptHandleSource::RuntimeOwned => DeferredPageTaskLane::PostDomContentLoaded,
            ScriptHandleSource::ParserOwned => match mode {
                ScriptMode::Normal => DeferredPageTaskLane::ParserBoundary,
                ScriptMode::Defer | ScriptMode::ModuleDefer => {
                    DeferredPageTaskLane::PreDomContentLoaded
                }
                ScriptMode::Async
                | ScriptMode::InOrder
                | ScriptMode::ImportMapInOrder
                | ScriptMode::ModuleInOrder => DeferredPageTaskLane::PostDomContentLoaded,
            },
            ScriptHandleSource::Unknown => match mode {
                ScriptMode::Normal | ScriptMode::Defer | ScriptMode::ModuleDefer => {
                    DeferredPageTaskLane::PreDomContentLoaded
                }
                ScriptMode::Async
                | ScriptMode::InOrder
                | ScriptMode::ImportMapInOrder
                | ScriptMode::ModuleInOrder => DeferredPageTaskLane::PostDomContentLoaded,
            },
            ScriptHandleSource::DocumentWriteOwned => match mode {
                ScriptMode::Normal => DeferredPageTaskLane::PreDomContentLoaded,
                ScriptMode::Defer
                | ScriptMode::ModuleDefer
                | ScriptMode::InOrder
                | ScriptMode::ImportMapInOrder
                | ScriptMode::ModuleInOrder => DeferredPageTaskLane::PreDomContentLoaded,
                ScriptMode::Async => DeferredPageTaskLane::PostDomContentLoaded,
            },
        }
    }

    fn script_handle_source_waits_for_blocking_stylesheets(source: ScriptHandleSource) -> bool {
        !matches!(source, ScriptHandleSource::RuntimeOwned)
    }

    fn queued_script_waits_until_dom_content_loaded(
        source: ScriptHandleSource,
        kind: ScriptKind,
        mode: ScriptMode,
    ) -> bool {
        match source {
            // A script inserted by page JS is not parser ordered. Its module
            // graph may become runnable while a parser defer script is still
            // blocked, so DOMContentLoaded is never an execution prerequisite.
            ScriptHandleSource::RuntimeOwned => false,
            ScriptHandleSource::ParserOwned
            | ScriptHandleSource::Unknown
            | ScriptHandleSource::DocumentWriteOwned => {
                matches!(
                    (kind, mode),
                    (ScriptKind::Module, _)
                        | (_, ScriptMode::InOrder)
                        | (_, ScriptMode::ImportMapInOrder)
                        | (_, ScriptMode::ModuleInOrder)
                ) || matches!(
                    (source, mode),
                    (ScriptHandleSource::DocumentWriteOwned, ScriptMode::Async)
                )
            }
        }
    }

    pub(crate) fn with_page_task_injection(page_task_tx: PageTaskSender) -> Self {
        Self {
            page_task_tx: Some(page_task_tx),
            main_document_runtime_producer: None,
            ..Default::default()
        }
    }

    pub(crate) fn clear_for_document_replacement(&mut self) {
        #[cfg(test)]
        {
            self.pending_dynamic_in_order_scripts.clear();
            self.pending_dynamic_importmap_in_order_scripts.clear();
            self.pending_dynamic_module_in_order_scripts.clear();
            self.pending_dynamic_async_scripts.clear();
            self.pending_failed_dynamic_scripts.clear();
        }
        self.main_document_runtime_producer = None;
        self.main_document_runtime_target = None;
        self.main_document_completion_recheck_turn_queued = false;
        self.dynamic_module_job_turn_queued = false;
        self.native_module_owner_event_turn_queued = false;
        self.module_owner.clear_for_document_replacement();
        self.script_handles.clear();
    }

    pub(crate) fn register_script_handle_with_source(
        &mut self,
        handle: &str,
        node: NativeNodeId,
        source: ScriptHandleSource,
    ) {
        self.script_handles
            .entry(handle.to_owned())
            .and_modify(|state| {
                state.node = node;
                state.source = state.source.merge_registration(source);
                state.page_task_execution_kind =
                    Self::default_page_task_execution_kind_for_source(state.source);
                state.followup_lane = Self::default_followup_lane_for_source(state.source);
                state.waits_for_blocking_stylesheets =
                    Self::script_handle_source_waits_for_blocking_stylesheets(state.source);
            })
            .or_insert(ScriptHandleState {
                node,
                source,
                start_state: ScriptHandleStartState::Ready,
                page_task_execution_kind: Self::default_page_task_execution_kind_for_source(source),
                followup_lane: Self::default_followup_lane_for_source(source),
                waits_for_blocking_stylesheets:
                    Self::script_handle_source_waits_for_blocking_stylesheets(source),
                waits_until_dom_content_loaded: false,
            });
    }

    pub(crate) fn script_handle_page_task_execution_kind(
        &self,
        handle: &str,
    ) -> Option<ScriptPageTaskExecutionKind> {
        self.script_handles
            .get(handle)
            .map(|state| state.page_task_execution_kind)
    }

    pub(crate) fn script_handle_followup_lane(&self, handle: &str) -> Option<DeferredPageTaskLane> {
        self.script_handles
            .get(handle)
            .map(|state| state.followup_lane)
    }

    pub(crate) fn set_script_handle_followup_lane(
        &mut self,
        handle: &str,
        lane: DeferredPageTaskLane,
    ) {
        if let Some(state) = self.script_handles.get_mut(handle) {
            state.followup_lane = lane;
        }
    }

    pub(crate) fn script_handle_waits_for_blocking_stylesheets(&self, handle: &str) -> bool {
        self.script_handles
            .get(handle)
            .is_some_and(|state| state.waits_for_blocking_stylesheets)
    }

    pub(crate) fn script_handle_waits_until_dom_content_loaded(&self, handle: &str) -> bool {
        self.script_handles
            .get(handle)
            .is_some_and(|state| state.waits_until_dom_content_loaded)
    }

    pub(crate) fn set_script_handle_waits_until_dom_content_loaded(&mut self, handle: &str) {
        if let Some(state) = self.script_handles.get_mut(handle) {
            state.waits_until_dom_content_loaded = true;
        }
    }

    pub(crate) fn register_import_map(
        &mut self,
        source: &str,
        base_url: &Url,
    ) -> std::result::Result<(), String> {
        self.module_owner.register_import_map(source, base_url)
    }

    pub(crate) fn resolve_module_specifier(
        &mut self,
        specifier: &str,
        base_url: &Url,
    ) -> std::result::Result<Url, String> {
        self.module_owner
            .resolve_module_specifier(specifier, base_url)
    }

    pub(crate) fn resolve_module_integrity(&self, url: &Url) -> Option<String> {
        self.module_owner.resolve_module_integrity(url)
    }

    pub(crate) fn next_inline_module_eval_id(&mut self) -> u64 {
        self.module_owner.next_inline_module_eval_id()
    }

    #[cfg(test)]
    pub(crate) fn insert_native_module_source(
        &mut self,
        key: ModuleMapKey,
        source: ModuleSource,
    ) -> ModuleEntryId {
        let entry = self.module_owner.insert_native_module_source(key, source);
        self.admit_ready_native_module_owner_event();
        entry
    }

    pub(crate) fn insert_native_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        let entry = self.module_owner.insert_native_module_source_for_request(
            request_key,
            effective_key,
            source,
            effective_fetch_metadata,
        );
        self.admit_ready_native_module_owner_event();
        entry
    }

    pub(crate) fn start_or_join_native_module_fetch(
        &mut self,
        key: ModuleMapKey,
    ) -> crate::module_runtime::ModuleMapFetchDisposition {
        self.module_owner.start_or_join_native_module_fetch(key)
    }

    pub(crate) fn insert_native_compiled_module_record_with_metadata(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.module_owner
            .insert_native_compiled_module_record_with_metadata(
                request_key,
                record,
                identity,
                effective_fetch_metadata,
            )
    }

    pub(crate) fn native_module_entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId> {
        self.module_owner.native_module_entry_id(key)
    }

    pub(crate) fn native_module_entry_state(&self, entry_id: ModuleEntryId) -> ModuleMapEntryState {
        self.module_owner.native_module_entry_state(entry_id)
    }

    pub(crate) fn native_module_entry_key(&self, entry_id: ModuleEntryId) -> ModuleMapKey {
        self.module_owner.native_module_entry_key(entry_id)
    }

    pub(crate) fn native_module_effective_fetch_metadata(
        &self,
        entry_id: ModuleEntryId,
    ) -> ModuleFetchMetadata {
        self.module_owner
            .native_module_effective_fetch_metadata(entry_id)
    }

    pub(crate) fn native_module_source(&self, entry_id: ModuleEntryId) -> Option<ModuleSource> {
        self.module_owner.native_module_source(entry_id)
    }

    pub(crate) fn native_module_source_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<(ModuleMapKey, ModuleSource)> {
        self.module_owner.native_module_source_for(module)
    }

    pub(crate) fn native_module_wasm_record_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<WasmModuleRecord> {
        self.module_owner.native_module_wasm_record_for(module)
    }

    pub(crate) fn native_module_wasm_record(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<WasmModuleRecord> {
        self.module_owner.native_module_wasm_record(entry_id)
    }

    pub(crate) fn native_wasm_instance_for_namespace<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        namespace: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.module_owner
            .native_wasm_instance_for_namespace(scope, namespace)
    }

    pub(crate) fn native_resolved_dependency_module_for(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &crate::module_runtime::ModuleAttributesKey,
    ) -> Option<v8::Global<v8::Module>> {
        self.module_owner
            .native_resolved_dependency_module_for(referrer, specifier, attributes)
    }

    pub(crate) fn native_module_entry_url(&self, entry_id: ModuleEntryId) -> Url {
        self.module_owner.native_module_entry_url(entry_id)
    }

    pub(crate) fn native_module_failure(&self, entry_id: ModuleEntryId) -> Option<ModuleLoadError> {
        self.module_owner.native_module_failure(entry_id)
    }

    pub(crate) fn native_module_requests(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<Vec<ModuleRequestRecord>> {
        self.module_owner.native_module_requests(entry_id)
    }

    pub(crate) fn set_native_module_resolved_dependencies(
        &mut self,
        entry_id: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        self.module_owner
            .set_native_module_resolved_dependencies(entry_id, dependencies);
    }

    #[cfg(test)]
    pub(crate) fn native_module_resolved_dependencies(
        &self,
        entry_id: ModuleEntryId,
    ) -> Vec<ModuleResolvedDependency> {
        self.module_owner
            .native_module_resolved_dependencies(entry_id)
    }

    pub(crate) fn native_document_modulator_ptr(
        &self,
    ) -> *const crate::module_runtime::NativeDocumentModulator {
        self.module_owner.native_document_modulator_ptr()
    }

    pub(crate) fn native_compiled_module(
        &self,
        entry_id: ModuleEntryId,
    ) -> Option<v8::Global<v8::Module>> {
        self.module_owner.native_compiled_module(entry_id)
    }

    pub(crate) fn native_module_url_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<url::Url> {
        self.module_owner.native_module_url_for(module)
    }

    pub(crate) fn mark_native_module_instantiated(&mut self, entry_id: ModuleEntryId) {
        self.module_owner.mark_native_module_instantiated(entry_id);
    }

    pub(crate) fn mark_native_module_evaluating(&mut self, entry_id: ModuleEntryId) {
        self.module_owner.mark_native_module_evaluating(entry_id);
    }

    pub(crate) fn mark_native_module_evaluated(&mut self, entry_id: ModuleEntryId) {
        self.module_owner.mark_native_module_evaluated(entry_id);
    }

    pub(crate) fn mark_native_module_failed(
        &mut self,
        key: ModuleMapKey,
        error: crate::module_runtime::ModuleLoadError,
    ) -> ModuleEntryId {
        let entry = self.module_owner.mark_native_module_failed(key, error);
        self.admit_ready_native_module_owner_event();
        entry
    }

    pub(crate) fn queue_native_dynamic_module_import(
        &mut self,
        request: PendingDynamicModuleImport,
    ) {
        self.module_owner
            .queue_native_dynamic_module_import(request);
        self.admit_ready_dynamic_module_job();
    }

    pub(crate) fn take_next_native_dynamic_module_import(
        &mut self,
    ) -> Option<NativeModuleGraphJob> {
        self.dynamic_module_job_turn_queued = false;
        let job = self.module_owner.take_next_native_dynamic_module_import();
        self.admit_ready_dynamic_module_job();
        job
    }

    pub(crate) fn resume_native_dynamic_module_import_front(&mut self, job: NativeModuleGraphJob) {
        self.module_owner
            .resume_native_dynamic_module_import_front(job);
        self.admit_ready_dynamic_module_job();
    }

    pub(crate) fn reserve_native_dynamic_module_evaluation_reaction(
        &mut self,
        request: PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
    ) -> u64 {
        self.module_owner
            .reserve_native_dynamic_module_evaluation_reaction(request, target)
    }

    pub(crate) fn native_dynamic_module_evaluation_reaction_owner(
        &self,
        reaction_id: u64,
    ) -> Option<crate::module_runtime::DynamicModuleImportOwner> {
        self.module_owner
            .native_dynamic_module_evaluation_reaction_owner(reaction_id)
    }

    pub(crate) fn take_native_dynamic_module_evaluation_reaction(
        &mut self,
        reaction_id: u64,
        expected_owner: crate::module_runtime::DynamicModuleImportOwner,
    ) -> Option<PendingDynamicModuleEvaluationReaction> {
        self.module_owner
            .take_native_dynamic_module_evaluation_reaction(reaction_id, expected_owner)
    }

    pub(crate) fn retire_native_dynamic_module_import_execution_context(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) -> DynamicModuleExecutionContextRetirement {
        self.module_owner
            .retire_native_dynamic_module_import_execution_context(owner)
    }

    pub(crate) fn suspend_native_module_script_fetch(
        &mut self,
        continuation: ModuleScriptGraphFetchContinuation,
    ) -> u64 {
        self.module_owner
            .suspend_native_module_script_fetch(continuation)
    }

    pub(crate) fn take_inflight_native_module_script_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<ModuleScriptGraphFetchContinuation> {
        self.module_owner
            .take_inflight_native_module_script_fetch(load_id)
    }

    pub(crate) fn has_inflight_native_module_script_fetch(&self, load_id: u64) -> bool {
        self.module_owner
            .has_inflight_native_module_script_fetch(load_id)
    }

    pub(crate) fn suspend_native_dynamic_module_import_fetches(
        &mut self,
        requests: Vec<NativeModuleGraphFetchRequest>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        job: NativeModuleGraphJob,
        owner_module_fetch_starts: Vec<
            Option<crate::frame_owner_model::FrameDocumentModuleFetchClientStart>,
        >,
    ) -> Vec<crate::module_runtime::DynamicModuleScheduledFetch> {
        self.module_owner
            .suspend_native_dynamic_module_import_fetches(
                requests,
                joined_clients,
                job,
                owner_module_fetch_starts,
            )
    }

    pub(crate) fn take_inflight_native_dynamic_module_import_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<DynamicModuleInflightFetch> {
        self.module_owner
            .take_inflight_native_dynamic_module_import_fetch(load_id)
    }

    pub(crate) fn take_joined_native_dynamic_module_import_fetch(
        &mut self,
        client: moli_module_script_tree::SingleModuleClientToken,
    ) -> Option<DynamicModuleJoinedFetch> {
        self.module_owner
            .take_joined_native_dynamic_module_import_fetch(client)
    }

    pub(crate) fn continue_native_dynamic_module_import_fetch(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        owner_module_fetch_starts: Vec<
            Option<crate::frame_owner_model::FrameDocumentModuleFetchClientStart>,
        >,
    ) -> DynamicModuleFetchOwnerAdvance {
        self.module_owner
            .continue_native_dynamic_module_import_fetch(continuation, owner_module_fetch_starts)
    }

    pub(crate) fn clear_failed_native_dynamic_module_import_fetch(
        &mut self,
        failure: DynamicModuleFetchFailure,
    ) -> (PendingDynamicModuleImport, ModuleLoadError) {
        self.module_owner
            .clear_failed_native_dynamic_module_import_fetch(failure)
    }

    pub(crate) fn has_pending_native_dynamic_module_import(&self) -> bool {
        self.module_owner.has_pending_native_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_ready_native_dynamic_module_import(&self) -> bool {
        self.module_owner.has_ready_native_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_native_dynamic_module_import_fetch(&self) -> bool {
        self.module_owner
            .has_inflight_native_dynamic_module_import_fetch()
    }

    pub(crate) fn suspend_native_modulepreload_fetch(
        &mut self,
        request: NativeModuleSingleFetchRequest,
    ) -> u64 {
        self.module_owner
            .suspend_native_modulepreload_fetch(request)
    }

    pub(crate) fn take_inflight_native_modulepreload_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<NativeModuleSingleFetchRequest> {
        self.module_owner
            .take_inflight_native_modulepreload_fetch(load_id)
    }

    pub(crate) fn suspend_native_modulepreload_link_clients(
        &mut self,
        key: ModuleMapKey,
        clients: Vec<std::sync::Arc<crate::module_runtime::NativeModulepreloadLinkClient>>,
    ) {
        self.module_owner
            .suspend_native_modulepreload_link_clients(key, clients);
    }

    pub(crate) fn post_modulepreload_link_error_event(&mut self, link: NativeNodeId) {
        self.module_owner.post_modulepreload_link_error_event(link);
        self.admit_ready_native_module_owner_event();
    }

    #[cfg(test)]
    pub(crate) fn module_script_client_count_for_testing(&self) -> usize {
        self.module_owner.module_script_client_count_for_testing()
    }

    #[cfg(test)]
    pub(crate) fn modulepreload_link_client_count_for_testing(&self) -> usize {
        self.module_owner
            .modulepreload_link_client_count_for_testing()
    }

    pub(crate) fn suspend_native_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: crate::module_runtime::NativeModuleMapSingleModuleClient,
    ) {
        self.module_owner
            .suspend_native_module_fetch_waiter(key, client);
    }

    pub(crate) fn detach_native_module_fetch_waiter(
        &mut self,
        client: moli_module_script_tree::SingleModuleClientToken,
    ) -> bool {
        self.module_owner.detach_native_module_fetch_waiter(client)
    }

    pub(crate) fn take_next_native_module_owner_event(
        &mut self,
    ) -> Option<crate::module_runtime::NativeModuleOwnerEvent> {
        self.native_module_owner_event_turn_queued = false;
        let event = self.module_owner.take_next_native_module_owner_event();
        self.admit_ready_native_module_owner_event();
        event
    }

    pub(crate) fn has_ready_native_module_owner_event(&mut self) -> bool {
        self.module_owner.has_ready_native_module_owner_event()
    }

    #[cfg(test)]
    pub(crate) fn has_native_module_script_fetch_waiters(&self) -> bool {
        self.module_owner.has_native_module_script_fetch_waiters()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_native_modulepreload_fetch(&self) -> bool {
        self.module_owner.has_inflight_native_modulepreload_fetch()
    }

    pub(crate) fn script_handle_target(&self, handle: &str) -> Option<NativeNodeId> {
        self.script_handles.get(handle).map(|state| state.node)
    }

    pub(crate) fn script_handle_for_node_with_source(
        &self,
        node: NativeNodeId,
        source: ScriptHandleSource,
    ) -> Option<&str> {
        self.script_handles.iter().find_map(|(handle, state)| {
            (state.node == node && state.source == source).then_some(handle.as_str())
        })
    }

    pub(crate) fn script_host_event_subject(&self, handle: &str) -> ScriptHostEventSubject {
        let state = self.require_registered_script_handle_for_planning(handle);
        ScriptHostEventSubject::for_handle_state(state)
    }

    pub(crate) fn script_handle_source(&self, handle: &str) -> ScriptHandleSource {
        self.require_registered_script_handle_for_planning(handle)
            .map(|state| state.source)
            .unwrap_or(ScriptHandleSource::Unknown)
    }

    pub(crate) fn script_event_policy(&self, handle: &str) -> ScriptEventPolicy {
        ScriptEventPolicy::for_subject(self.script_host_event_subject(handle))
    }

    pub(crate) fn script_event_policy_for_script(
        &self,
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        handle: Option<&str>,
    ) -> ScriptEventPolicy {
        let subject = handle
            .map(|handle| self.script_host_event_subject(handle))
            .unwrap_or_else(ScriptHostEventSubject::unknown);
        ScriptEventPolicy::for_script(kind, source_kind, subject)
    }

    pub(crate) fn script_failure_page_task_policy(
        &self,
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        handle: Option<&str>,
        message: &str,
        module_failure_policy: Option<ModuleFailurePolicy>,
    ) -> ScriptFailurePageTaskPolicy {
        let subject = handle
            .map(|handle| self.script_host_event_subject(handle))
            .unwrap_or_else(ScriptHostEventSubject::unknown);
        ScriptFailurePageTaskPolicy::for_script(
            kind,
            source_kind,
            subject,
            message,
            module_failure_policy,
        )
    }

    pub(crate) fn plan_script_event_task(
        &self,
        kind: ScriptEventKind,
        handle: &str,
    ) -> Option<ScriptEventTask> {
        match self.script_event_policy(handle).task_dispatch_policy(kind) {
            ScriptEventDispatchPolicy::Dispatch => Some(ScriptEventTask::new(kind, handle)),
            ScriptEventDispatchPolicy::Skip(reason) => {
                debug!(
                    host_script_handle = handle,
                    event_kind = ?kind,
                    skip_reason = ?reason,
                    "skipping script event task at source planning boundary"
                );
                None
            }
        }
    }

    pub(crate) fn plan_script_event_lifecycle_work(
        &self,
        kind: ScriptEventKind,
        handle: &str,
    ) -> Option<PostParseLifecycleWork> {
        self.plan_script_event_task(kind, handle)
            .map(PostParseLifecycleWork::DispatchScriptEvent)
    }

    #[cfg(test)]
    pub(crate) fn plan_script_event_page_task(
        &self,
        kind: ScriptEventKind,
        handle: &str,
    ) -> Option<PageTask> {
        self.plan_script_event_lifecycle_work(kind, handle)
            .map(PostParseLifecycleWork::into_page_task)
    }

    pub(crate) fn plan_script_event_task_for_script(
        &self,
        event_kind: ScriptEventKind,
        script_kind: ScriptKind,
        source_kind: ScriptSourceKind,
        handle: &str,
    ) -> Option<ScriptEventTask> {
        match self
            .script_event_policy_for_script(script_kind, source_kind, Some(handle))
            .task_dispatch_policy(event_kind)
        {
            ScriptEventDispatchPolicy::Dispatch => Some(ScriptEventTask::new(event_kind, handle)),
            ScriptEventDispatchPolicy::Skip(reason) => {
                debug!(
                    host_script_handle = handle,
                    event_kind = ?event_kind,
                    skip_reason = ?reason,
                    "skipping prepared script event task at source planning boundary"
                );
                None
            }
        }
    }

    pub(crate) fn script_event_requires_dispatch_for_script(
        &self,
        event_kind: ScriptEventKind,
        script_kind: ScriptKind,
        source_kind: ScriptSourceKind,
        handle: Option<&str>,
    ) -> bool {
        matches!(
            self.script_event_policy_for_script(script_kind, source_kind, handle)
                .task_dispatch_policy(event_kind),
            ScriptEventDispatchPolicy::Dispatch
        )
    }

    pub(crate) fn plan_window_script_failure_report_lifecycle_work(
        &self,
        message: &str,
        filename: Option<&str>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> PostParseLifecycleWork {
        PostParseLifecycleWork::ReportWindowScriptFailure(
            WindowScriptFailureReportTask::new_with_error_constructor(
                message,
                filename.map(std::borrow::ToOwned::to_owned),
                error_constructor,
            ),
        )
    }

    pub(crate) fn plan_script_failure_lifecycle_work(
        &self,
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        handle: Option<&str>,
        message: &str,
        filename: Option<&str>,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Vec<PostParseLifecycleWork> {
        let mut tasks = Vec::new();
        let message = normalize_module_link_failure_message(message, filename);
        let policy = self.script_failure_page_task_policy(
            kind,
            source_kind,
            handle,
            &message,
            module_failure_policy,
        );

        if let Some(handle) = handle {
            match policy.error_event {
                ScriptEventDispatchPolicy::Dispatch => {
                    tasks.push(PostParseLifecycleWork::DispatchScriptEvent(
                        ScriptEventTask::new(ScriptEventKind::Error, handle),
                    ));
                }
                ScriptEventDispatchPolicy::Skip(reason) => {
                    debug!(
                        host_script_handle = handle,
                        event_kind = ?ScriptEventKind::Error,
                        skip_reason = ?reason,
                        "skipping script failure event task at source planning boundary"
                    );
                }
            }
        }

        if policy.report_window_failure {
            tasks.push(self.plan_window_script_failure_report_lifecycle_work(
                &message,
                filename,
                error_constructor,
            ));
        }

        if policy.load_event_after_window_failure
            && let Some(handle) = handle
        {
            match policy.load_event {
                ScriptEventDispatchPolicy::Dispatch => {
                    tasks.push(PostParseLifecycleWork::DispatchScriptEvent(
                        ScriptEventTask::new(ScriptEventKind::Load, handle),
                    ));
                }
                ScriptEventDispatchPolicy::Skip(reason) => {
                    debug!(
                        host_script_handle = handle,
                        event_kind = ?ScriptEventKind::Load,
                        skip_reason = ?reason,
                        "skipping script load task after failure at source planning boundary"
                    );
                }
            }
        }

        tasks
    }

    #[cfg(test)]
    pub(crate) fn plan_script_failure_page_tasks(
        &self,
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        handle: Option<&str>,
        message: &str,
        filename: Option<&str>,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Vec<PageTask> {
        self.plan_script_failure_lifecycle_work(
            kind,
            source_kind,
            handle,
            message,
            filename,
            module_failure_policy,
            error_constructor,
        )
        .into_iter()
        .map(PostParseLifecycleWork::into_page_task)
        .collect()
    }

    fn require_registered_script_handle_for_planning(
        &self,
        handle: &str,
    ) -> Option<ScriptHandleState> {
        let state = self.script_handles.get(handle).copied();
        if state.is_none() {
            warn!(
                host_script_handle = handle,
                "script host-event planning received an unregistered handle"
            );
            assert!(
                state.is_some(),
                "script host-event planning requires a registered handle"
            );
        }
        state
    }

    #[cfg(test)]
    pub(crate) fn enqueue_post_parse_lifecycle_page_task(&mut self, task: PageTask) {
        let task_label = task.phase_label();
        let Some(work) = PostParseLifecycleWork::from_page_task(task) else {
            debug_assert!(
                false,
                "script-bearing {task_label} must not enter post-parse lifecycle enqueue"
            );
            return;
        };
        self.enqueue_post_parse_lifecycle_work(work);
    }

    pub(crate) fn bind_main_document_runtime_producer(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        let next_target = owner;
        if self.main_document_runtime_target != Some(next_target) {
            self.main_document_completion_recheck_turn_queued = false;
        }
        self.main_document_runtime_producer = self
            .page_task_tx
            .as_ref()
            .map(|tx| tx.bind_main_document_runtime_producer(owner));
        self.main_document_runtime_target = self
            .main_document_runtime_producer
            .as_ref()
            .map(|_| next_target);
        self.dynamic_module_job_turn_queued = false;
        self.native_module_owner_event_turn_queued = false;
        self.admit_ready_dynamic_module_job();
        self.admit_ready_native_module_owner_event();
        self.main_document_runtime_producer.is_some()
    }

    fn admit_ready_dynamic_module_job(&mut self) -> bool {
        if self.dynamic_module_job_turn_queued {
            return true;
        }
        if !self.module_owner.has_ready_native_dynamic_module_import() {
            return false;
        }
        let Some(producer) = self.main_document_runtime_producer.as_ref() else {
            return false;
        };
        if producer.send_dynamic_module_job().is_err() {
            return false;
        }
        self.dynamic_module_job_turn_queued = true;
        true
    }

    fn admit_ready_native_module_owner_event(&mut self) -> bool {
        if self.native_module_owner_event_turn_queued {
            return true;
        }
        if !self.module_owner.has_ready_native_module_owner_event() {
            return false;
        }
        let Some(producer) = self.main_document_runtime_producer.as_ref() else {
            return false;
        };
        if producer.send_native_module_owner_event().is_err() {
            return false;
        }
        self.native_module_owner_event_turn_queued = true;
        true
    }

    pub(crate) fn has_main_document_runtime_route(&self) -> bool {
        self.page_task_tx
            .as_ref()
            .is_some_and(PageTaskSender::has_main_document_runtime_route)
    }

    pub(crate) fn publish_runtime_script_admission(
        &self,
        admission: RuntimeScriptAdmission,
    ) -> Result<(), RuntimeScriptAdmission> {
        let Some(producer) = self.main_document_runtime_producer.as_ref() else {
            return Err(admission);
        };
        producer.send_runtime_script_admission(admission)
    }

    pub(crate) fn enqueue_post_parse_lifecycle_work(&mut self, work: PostParseLifecycleWork) {
        if let Some(producer) = self.main_document_runtime_producer.as_ref() {
            let _ = producer.send_lifecycle_work(work);
            return;
        }
        debug_assert!(
            false,
            "exact main-Document runtime producer must be bound before post-DCL enqueue"
        );
    }

    pub(crate) fn enqueue_main_document_completion_recheck(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        if self
            .main_document_runtime_target
            .is_none_or(|target_owner| target_owner != owner)
        {
            return false;
        }
        if self.main_document_completion_recheck_turn_queued {
            return true;
        }
        let Some(producer) = self.main_document_runtime_producer.as_ref() else {
            debug_assert!(
                false,
                "main-Document runtime producer must be bound before completion recheck enqueue"
            );
            return false;
        };
        if producer
            .send_lifecycle_work(PostParseLifecycleWork::CheckMainDocumentCompletion { owner })
            .is_err()
        {
            return false;
        }
        self.main_document_completion_recheck_turn_queued = true;
        true
    }

    pub(crate) fn begin_main_document_completion_recheck_turn(&mut self) {
        self.main_document_completion_recheck_turn_queued = false;
    }

    fn enqueue_script_event_lifecycle_work(&mut self, kind: ScriptEventKind, handle: &str) {
        if let Some(work) = self.plan_script_event_lifecycle_work(kind, handle) {
            self.enqueue_post_parse_lifecycle_work(work);
        }
    }

    fn reserve_script_start(&mut self, handle: &str, node: NativeNodeId) -> bool {
        let entry = self
            .script_handles
            .entry(handle.to_owned())
            .or_insert(ScriptHandleState {
                node,
                source: ScriptHandleSource::Unknown,
                start_state: ScriptHandleStartState::Ready,
                page_task_execution_kind: ScriptPageTaskExecutionKind::Standard,
                followup_lane: DeferredPageTaskLane::PreDomContentLoaded,
                waits_for_blocking_stylesheets: true,
                waits_until_dom_content_loaded: false,
            });
        entry.node = node;
        if entry.start_state != ScriptHandleStartState::Ready {
            return false;
        }
        entry.start_state = ScriptHandleStartState::Preparing;
        true
    }

    fn finish_script_start(
        &mut self,
        handle: &str,
        node: NativeNodeId,
        kind: ScriptStartCommitKind,
    ) -> bool {
        let entry = self
            .script_handles
            .entry(handle.to_owned())
            .or_insert(ScriptHandleState {
                node,
                source: ScriptHandleSource::Unknown,
                start_state: ScriptHandleStartState::Ready,
                page_task_execution_kind: ScriptPageTaskExecutionKind::Standard,
                followup_lane: DeferredPageTaskLane::PreDomContentLoaded,
                waits_for_blocking_stylesheets: true,
                waits_until_dom_content_loaded: false,
            });
        entry.node = node;
        if entry.start_state != ScriptHandleStartState::Preparing {
            return false;
        }
        entry.start_state = ScriptHandleStartState::Committed(kind);
        true
    }

    fn cancel_script_start(&mut self, handle: &str, node: NativeNodeId) {
        let Some(entry) = self.script_handles.get_mut(handle) else {
            return;
        };
        entry.node = node;
        if entry.start_state == ScriptHandleStartState::Preparing {
            entry.start_state = ScriptHandleStartState::Ready;
        }
    }

    #[cfg(test)]
    fn script_start_state(&self, handle: &str) -> Option<ScriptHandleStartState> {
        self.script_handles
            .get(handle)
            .map(|state| state.start_state)
    }

    #[cfg(test)]
    fn queue_dynamic_script(
        &mut self,
        preparation: &RuntimeScriptPreparationContext,
        host_script_handle: &str,
        source: &str,
        source_kind: ScriptSourceKind,
        kind: ScriptKind,
        mode: ScriptMode,
    ) -> std::result::Result<(), String> {
        debug_assert!(
            matches!(kind, ScriptKind::Classic | ScriptKind::Module),
            "non-executable script kinds must not enter the dynamic script queue"
        );
        let node_id = self.next_virtual_node_id();
        self.queue_script_with_identity(
            preparation,
            node_id,
            host_script_handle,
            source,
            source_kind,
            kind,
            mode,
        )
    }

    #[cfg(test)]
    fn queue_dynamic_script_for_node(
        &mut self,
        preparation: &RuntimeScriptPreparationContext,
        node_id: NodeId,
        host_script_handle: &str,
        source: &str,
        source_kind: ScriptSourceKind,
        kind: ScriptKind,
        mode: ScriptMode,
    ) -> std::result::Result<(), String> {
        self.queue_script_with_identity(
            preparation,
            node_id,
            host_script_handle,
            source,
            source_kind,
            kind,
            mode,
        )
    }

    fn register_dynamic_import_map(
        &mut self,
        preparation: &RuntimeScriptPreparationContext,
        source: &str,
    ) {
        if let Err(message) = self.register_import_map(source, &preparation.base_url) {
            let work = self.plan_window_script_failure_report_lifecycle_work(
                &message,
                Some(preparation.base_url.as_str()),
                None,
            );
            self.enqueue_post_parse_lifecycle_work(work);
        }
    }

    #[cfg(test)]
    fn queue_script_with_identity(
        &mut self,
        preparation: &RuntimeScriptPreparationContext,
        node_id: NodeId,
        host_script_handle: &str,
        source: &str,
        source_kind: ScriptSourceKind,
        kind: ScriptKind,
        mode: ScriptMode,
    ) -> std::result::Result<(), String> {
        let script = self.prepare_dynamic_script(
            preparation,
            node_id,
            host_script_handle,
            source,
            source_kind,
            kind,
            mode,
        )?;
        match script.mode {
            ScriptMode::Async => self.pending_dynamic_async_scripts.push_back(script),
            ScriptMode::InOrder => self.pending_dynamic_in_order_scripts.push_back(script),
            ScriptMode::ImportMapInOrder => self
                .pending_dynamic_importmap_in_order_scripts
                .push_back(script),
            ScriptMode::ModuleInOrder => self
                .pending_dynamic_module_in_order_scripts
                .push_back(script),
            ScriptMode::Normal | ScriptMode::Defer | ScriptMode::ModuleDefer => {
                unreachable!("dynamic scripts must classify as async or in-order")
            }
        }
        Ok(())
    }

    fn prepare_dynamic_script(
        &mut self,
        preparation: &RuntimeScriptPreparationContext,
        node_id: NodeId,
        host_script_handle: &str,
        source: &str,
        source_kind: ScriptSourceKind,
        kind: ScriptKind,
        mode: ScriptMode,
    ) -> std::result::Result<PreparedScript, String> {
        let position = self.next_dynamic_position();
        let script = build_runtime_prepared_script(
            preparation,
            NativeNodeId::new(node_id.index()),
            position,
            Some(host_script_handle.to_owned()),
            source,
            source_kind,
            kind,
            mode,
        )?;
        if let Some(state) = self.script_handles.get_mut(host_script_handle) {
            state.followup_lane = Self::followup_lane_for_script(state.source, script.mode);
            if Self::queued_script_waits_until_dom_content_loaded(
                state.source,
                script.kind,
                script.mode,
            ) {
                state.waits_until_dom_content_loaded = true;
            }
        }
        Ok(script)
    }

    #[cfg(test)]
    fn queue_failed_dynamic_script(
        &mut self,
        preparation: &RuntimeScriptPreparationContext,
        host_script_handle: &str,
        source: &str,
        source_kind: ScriptSourceKind,
        kind: ScriptKind,
        mode: ScriptMode,
        message: &str,
    ) -> std::result::Result<(), String> {
        let node_id = self.next_virtual_node_id();
        let failed = self.prepare_failed_dynamic_script(
            preparation,
            node_id,
            host_script_handle,
            source,
            source_kind,
            kind,
            mode,
            message,
        )?;
        self.pending_failed_dynamic_scripts.push_back(failed);
        Ok(())
    }

    fn prepare_failed_dynamic_script(
        &mut self,
        preparation: &RuntimeScriptPreparationContext,
        node_id: NodeId,
        host_script_handle: &str,
        source: &str,
        source_kind: ScriptSourceKind,
        kind: ScriptKind,
        mode: ScriptMode,
        message: &str,
    ) -> std::result::Result<FailedDynamicScript, String> {
        let (url, source) = match source_kind {
            ScriptSourceKind::External => {
                let url = preparation
                    .base_url
                    .join(source)
                    .or_else(|_| Url::parse(source))
                    // URL parse failures are represented by this failed-script entry itself.
                    // Keep the owning document URL as the internal task identity; the original
                    // `src` remains on the element for observable error-event state.
                    .unwrap_or_else(|_| preparation.base_url.clone());
                (url, ScriptSource::External)
            }
            ScriptSourceKind::Inline => (
                preparation.base_url.clone(),
                ScriptSource::Inline(source.to_owned()),
            ),
        };

        if let Some(state) = self.script_handles.get_mut(host_script_handle)
            && Self::queued_script_waits_until_dom_content_loaded(state.source, kind, mode)
        {
            state.waits_until_dom_content_loaded = true;
        }

        let failure_kind = Self::queued_script_failure_kind(kind, source_kind);
        let position = self.next_dynamic_position();
        Ok(FailedDynamicScript {
            script: PreparedScript {
                position,
                node_id,
                kind,
                mode,
                source_kind,
                fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
                source,
                initiator_url: preparation.document_url.clone(),
                base_url: url.clone(),
                url,
                host_script_handle: Some(host_script_handle.to_owned()),
            },
            message: message.to_owned(),
            failure_kind,
        })
    }

    fn queued_script_failure_kind(
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
    ) -> QueuedScriptFailureKind {
        if kind == ScriptKind::Module && source_kind == ScriptSourceKind::External {
            QueuedScriptFailureKind::ModuleTopLevelLoad
        } else {
            QueuedScriptFailureKind::Immediate
        }
    }

    #[cfg(test)]
    pub(crate) fn drain_dynamic_scripts(&mut self) -> DynamicScriptBatch {
        DynamicScriptBatch {
            in_order: self.pending_dynamic_in_order_scripts.drain(..).collect(),
            importmap_in_order: self
                .pending_dynamic_importmap_in_order_scripts
                .drain(..)
                .collect(),
            module_in_order: self
                .pending_dynamic_module_in_order_scripts
                .drain(..)
                .collect(),
            async_scripts: self.pending_dynamic_async_scripts.drain(..).collect(),
            failed_scripts: self.pending_failed_dynamic_scripts.drain(..).collect(),
        }
    }

    fn next_virtual_node_id(&mut self) -> NodeId {
        let node_id = NodeId::new(self.next_virtual_script_node_index);
        self.next_virtual_script_node_index += 1;
        node_id
    }

    fn next_dynamic_position(&mut self) -> usize {
        let position = self.next_dynamic_script_position;
        self.next_dynamic_script_position += 1;
        position
    }
}

fn normalize_module_link_failure_message(message: &str, filename: Option<&str>) -> String {
    let Some(link_failure) = message.find("ModuleLinkFailed:") else {
        return message.to_owned();
    };
    let message = &message[link_failure..];
    rewrite_module_link_failure_relative_url(message, filename)
        .unwrap_or_else(|| message.to_owned())
}

fn rewrite_module_link_failure_relative_url(
    message: &str,
    filename: Option<&str>,
) -> Option<String> {
    let module_marker = "ModuleLinkFailed: module `";
    let export_marker = "` does not export `";
    let module_start = message.find(module_marker)? + module_marker.len();
    let module_rest = message.get(module_start..)?;
    let module_end = module_rest.find('`')?;
    let module = module_rest.get(..module_end)?;
    let export_start = module_start + module_end + export_marker.len();
    let export_rest = message.get(export_start..)?;
    let export_end = export_rest.find('`')?;
    let export = export_rest.get(..export_end)?;
    let resolved_module = resolve_module_link_failure_url(module, filename)?;
    Some(format!(
        "ModuleLinkFailed: module `{resolved_module}` does not export `{export}`"
    ))
}

fn resolve_module_link_failure_url(module: &str, filename: Option<&str>) -> Option<String> {
    if Url::parse(module).is_ok() {
        return None;
    }
    let base = Url::parse(filename?).ok()?;
    Some(base.join(module).ok()?.to_string())
}
