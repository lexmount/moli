use std::pin::pin;

use anyhow::Result;
use moli_webapi_declare::WebApiObject;
use url::Url;

use super::ScriptVm;
use crate::context_bootstrap::{
    ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
    ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
};
use crate::document_script_scheduler::{
    DocumentOwnedScriptReadyAction, ParserDeferredClassicSourceLoadApplyResult,
    ParserDeferredClassicSourceLoadCompletion, ParserDeferredScriptStartAction,
    ParserModuleEvaluationReactionUpdate, ParserPendingScriptId, ParserPendingScriptKey,
};
use crate::dom::NodeId;
use crate::frame_owner_model::{
    ChildDynamicModuleCompletedFetchRestoreAction,
    ChildDynamicModuleOwnerFetchCompletionSettlementAction,
    ChildDynamicModuleOwnerFetchWithoutNetworkSettlementAction, DocumentId,
    FrameDocumentDynamicImportEvaluationReadyAction,
    FrameDocumentDynamicImportEvaluationReadyResult,
    FrameDocumentDynamicImportJoinedFetchRestoreResult,
    FrameDocumentDynamicImportMissingJoinedTerminalClient,
    FrameDocumentDynamicImportMissingJoinedTerminalFetch, FrameDocumentDynamicImportOwnerAction,
    FrameDocumentDynamicImportOwnerActionDiagnostic, FrameDocumentDynamicImportOwnerActionHooks,
    FrameDocumentDynamicImportOwnerActionRunner,
    FrameDocumentDynamicImportOwnerFetchSettlementResult,
    FrameDocumentDynamicImportOwnerTerminalRestoreAction, FrameDocumentDynamicImportRejectAction,
    FrameDocumentDynamicImportRejectResult, FrameDocumentDynamicImportSourceReadyAction,
    FrameDocumentDynamicImportSourceReadyResult, FrameDocumentDynamicImportTerminalClientAction,
    FrameDocumentDynamicImportTerminalClientFinishResult,
    FrameDocumentDynamicImportTerminalOutcome, FrameDocumentDynamicImportTerminalPreparedAction,
    FrameDocumentDynamicImportWaitingFetchScheduleAction,
    FrameDocumentDynamicImportWaitingFetchScheduleResult, FrameDocumentModuleFetchClientStart,
    FrameDocumentModuleTerminalQueueFollowup, FrameDocumentTaskOwner, FrameRealmId,
    FrameSchedulerLaneId, LocalWindowId, MainDocumentScriptLoadDelayKind,
    MainDocumentScriptLoadDelayLease,
};
use crate::module_runtime::{
    DynamicModuleEvaluationTarget, DynamicModuleFetchContinuation, DynamicModuleFetchFailure,
    DynamicModuleFetchFinish, DynamicModuleJoinedFetch, DynamicModuleScheduledFetch,
    ModuleAttributesKey, ModuleEntryId, ModuleGraphFetchedSource, ModuleGraphHandle,
    ModuleIdentityHash, ModuleImportPhase, ModuleKind, ModuleLoadError, ModuleLoadStage,
    ModuleMapEntryState, ModuleMapKey, ModuleMapTerminalNotification, ModuleRecordEntry,
    ModuleRequestRecord, ModuleScriptGraphFetchContinuation, ModuleSource, NativeDocumentModulator,
    NativeDynamicImportSingleModuleClient, NativeDynamicModuleImportReady, NativeModuleGraphJob,
    NativeModuleGraphJobAdvance, NativeModuleMapSingleModuleClient, NativeModuleOwnerEvent,
    NativeModuleScriptSingleModuleClient, NativeModuleSingleFetchRequest,
    NativeModulepreloadFetchStart, PendingDynamicModuleImport, ResolverScopeGuard,
    WasmDependencyModuleMessages, WasmImportRecord, WasmModuleRecord,
    ensure_wasm_dependency_module_namespace_ready, evaluate_wasm_synthetic_module,
    module_identity_hash_from_v8_module, preserve_current_v8_module_exception,
    resolve_static_module_callback, resolve_static_source_callback, throw_wasm_link_error,
    wasm_dependency_export_value,
};
use crate::module_script_continuation::{
    MainParserDeferredClassicSourceLoadCompletion, MainParserDocumentOwner,
    ModuleMapTerminalFanout, ModuleScriptCompletionOwner, ModuleScriptContinuation,
    ModuleScriptContinuationGraphAdvance, ModuleScriptEvaluationContinuation,
    ModuleScriptEvaluationUpdate, ModuleScriptGraphFetchResume, ModuleScriptGraphResumeResult,
    NativeDynamicModuleTerminalFanout, NativeModuleOwnerActions, ParserModuleScriptFailure,
    parser_module_evaluation_continuation_into_ready_action,
};
use crate::page_task_queue::{
    PageModuleReactionApplication, PageModuleReactionFollowup, RendererPageModuleReactionEvent,
};
use crate::planning::PreparedScript;
use crate::types::{
    ChildDynamicImportFetchCompletion, ScriptErrorConstructorKind, SubresourceRequestInitiatorType,
    SubresourceResourceType,
};
#[cfg(test)]
use crate::types::{
    ModuleGraphFetchCompletion, ModuleGraphFetchOrdering, ModuleGraphFetchRequester,
};
use crate::util::{context_host_ptr_from_global_bridge, get_private_value, v8_string, v8str};
use crate::wasm_module_support::{
    prepare_wasm_module_record, v8_exception_message_or, wasm_evaluation_import_modules,
};

mod child_dynamic_import;
mod child_parser_module;
mod child_ready_document_script;
mod dynamic_import_selected_task_body;
mod main_selected_task;
pub(crate) use main_selected_task::{
    MainDynamicImportGraphFetchBodySettlement, MainNativeModuleSelectedTaskApplication,
    MainNativeModuleSelectedTaskBodyActivity,
};

pub(crate) enum NativeDynamicModuleSourceImportResolution {
    Resolved,
    Rejected,
}

enum DocumentModuleReactionUpdate {
    ParserOwned(ParserModuleEvaluationReactionUpdate),
    RuntimeOwned(ModuleScriptEvaluationUpdate),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NativeDynamicModuleTerminalFanoutOutcome {
    scheduled_dynamic_import_fetches: usize,
    ready_imports_handled: usize,
    failed_fetches_rejected: usize,
    graph_advance_failures_handled: usize,
    source_imports_resolved: usize,
    source_imports_rejected: usize,
    evaluation_imports_resolved: usize,
    evaluation_imports_pending: usize,
    evaluation_imports_rejected: usize,
    dynamic_import_jobs_resumed: usize,
    dynamic_import_waits_retained: usize,
    child_followup: FrameDocumentModuleTerminalQueueFollowup,
}

impl NativeDynamicModuleTerminalFanoutOutcome {
    fn child_followup(self) -> FrameDocumentModuleTerminalQueueFollowup {
        self.child_followup
    }

    fn record_scheduled_dynamic_import_fetches(&mut self, count: usize) {
        self.scheduled_dynamic_import_fetches += count;
    }

    fn record_ready_import_outcome(&mut self, outcome: FrameDocumentDynamicImportTerminalOutcome) {
        self.ready_imports_handled += 1;
        self.record_dynamic_import_terminal_outcome(outcome);
    }

    fn record_failed_fetch_rejection_outcome(
        &mut self,
        outcome: FrameDocumentDynamicImportTerminalOutcome,
    ) {
        self.failed_fetches_rejected += outcome.dynamic_import_was_rejected() as usize;
        self.record_dynamic_import_terminal_outcome(outcome);
    }

    fn record_graph_advance_failure_outcome(
        &mut self,
        outcome: FrameDocumentDynamicImportTerminalOutcome,
    ) {
        self.graph_advance_failures_handled += outcome.dynamic_import_was_rejected() as usize;
        self.record_dynamic_import_terminal_outcome(outcome);
    }

    fn record_dynamic_import_terminal_outcome(
        &mut self,
        outcome: FrameDocumentDynamicImportTerminalOutcome,
    ) {
        self.child_followup
            .merge(outcome.owner_action_queue_followup());
        self.source_imports_resolved += outcome.source_import_was_resolved() as usize;
        self.source_imports_rejected += outcome.source_import_was_rejected() as usize;
        self.evaluation_imports_resolved += outcome.evaluation_import_was_resolved() as usize;
        self.evaluation_imports_pending += outcome.evaluation_import_was_pending() as usize;
        self.evaluation_imports_rejected += outcome.evaluation_import_was_rejected() as usize;
    }

    fn record_graph_advance_failure_followup(
        &mut self,
        followup: FrameDocumentModuleTerminalQueueFollowup,
    ) {
        self.graph_advance_failures_handled += 1;
        self.child_followup.merge(followup);
    }

    fn record_dynamic_import_job_resumed(&mut self) {
        self.dynamic_import_jobs_resumed += 1;
    }

    fn record_dynamic_import_wait_retained(&mut self) {
        self.dynamic_import_waits_retained += 1;
    }

    fn record_restored_after_unexpected_complete(&mut self) {
        self.child_followup
            .merge(FrameDocumentModuleTerminalQueueFollowup::terminal_warning_recorded());
    }

    fn merge(&mut self, other: Self) {
        self.scheduled_dynamic_import_fetches += other.scheduled_dynamic_import_fetches;
        self.ready_imports_handled += other.ready_imports_handled;
        self.failed_fetches_rejected += other.failed_fetches_rejected;
        self.graph_advance_failures_handled += other.graph_advance_failures_handled;
        self.source_imports_resolved += other.source_imports_resolved;
        self.source_imports_rejected += other.source_imports_rejected;
        self.evaluation_imports_resolved += other.evaluation_imports_resolved;
        self.evaluation_imports_pending += other.evaluation_imports_pending;
        self.evaluation_imports_rejected += other.evaluation_imports_rejected;
        self.dynamic_import_jobs_resumed += other.dynamic_import_jobs_resumed;
        self.dynamic_import_waits_retained += other.dynamic_import_waits_retained;
        self.child_followup.merge(other.child_followup);
    }

    #[cfg(test)]
    fn scheduled_dynamic_import_fetch_count(self) -> usize {
        self.scheduled_dynamic_import_fetches
    }

    #[cfg(test)]
    fn ready_import_count(self) -> usize {
        self.ready_imports_handled
    }

    #[cfg(test)]
    fn failed_fetch_rejected_count(self) -> usize {
        self.failed_fetches_rejected
    }

    #[cfg(test)]
    fn graph_advance_failure_handled_count(self) -> usize {
        self.graph_advance_failures_handled
    }

    #[cfg(test)]
    fn source_import_rejected_count(self) -> usize {
        self.source_imports_rejected
    }

    #[cfg(test)]
    fn evaluation_import_rejected_count(self) -> usize {
        self.evaluation_imports_rejected
    }

    #[cfg(test)]
    fn dynamic_import_job_resumed_count(self) -> usize {
        self.dynamic_import_jobs_resumed
    }
}

/// Realm-settlement strategy used while consuming main native-module work.
///
/// Main-document runtime tasks and selected Networking graph terminals use the
/// body-only strategy in `main_selected_task`, leaving the ordinary task end
/// to the central dispatcher. Compatibility callers that still own a complete
/// local task use the checkpointing strategy below. This strategy is local to
/// an executing carrier; it is never stored in a queued task or consulted by
/// the scheduler.
trait ScriptVmMainNativeModuleTaskBody {
    fn note_page_realm_body_attempted(&mut self) {}

    fn resolve_ready_source_import(
        &mut self,
        vm: &mut ScriptVm,
        request: PendingDynamicModuleImport,
        root_entry: ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleSourceImportResolution, ModuleLoadError>;

    fn resolve_completed_evaluation_import(
        &mut self,
        vm: &mut ScriptVm,
        request: PendingDynamicModuleImport,
        target: &DynamicModuleEvaluationTarget,
    ) -> std::result::Result<(), ModuleLoadError>;

    fn reject_dynamic_import(
        &mut self,
        vm: &mut ScriptVm,
        request: PendingDynamicModuleImport,
        error: &ModuleLoadError,
    ) -> std::result::Result<(), ModuleLoadError>;
}

#[cfg(test)]
struct ScriptVmCheckpointingMainNativeModuleTaskBody;

#[cfg(test)]
impl ScriptVmMainNativeModuleTaskBody for ScriptVmCheckpointingMainNativeModuleTaskBody {
    fn resolve_ready_source_import(
        &mut self,
        vm: &mut ScriptVm,
        request: PendingDynamicModuleImport,
        root_entry: ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleSourceImportResolution, ModuleLoadError> {
        vm.resolve_native_dynamic_module_source_import(request, root_entry)
    }

    fn resolve_completed_evaluation_import(
        &mut self,
        vm: &mut ScriptVm,
        request: PendingDynamicModuleImport,
        target: &DynamicModuleEvaluationTarget,
    ) -> std::result::Result<(), ModuleLoadError> {
        vm.resolve_native_dynamic_module_import(request, target)
    }

    fn reject_dynamic_import(
        &mut self,
        vm: &mut ScriptVm,
        request: PendingDynamicModuleImport,
        error: &ModuleLoadError,
    ) -> std::result::Result<(), ModuleLoadError> {
        vm.reject_native_dynamic_module_import_with_error(request, error)
    }
}

struct ScriptVmMainDynamicImportOwnerActionHooks<'vm, 'body, Body> {
    vm: &'vm mut ScriptVm,
    body: &'body mut Body,
}

impl<'vm, 'body, Body> ScriptVmMainDynamicImportOwnerActionHooks<'vm, 'body, Body> {
    fn new(vm: &'vm mut ScriptVm, body: &'body mut Body) -> Self {
        Self { vm, body }
    }

    fn unsupported<T>(&self, action: &'static str) -> std::result::Result<T, String> {
        Err(format!(
            "main dynamic import owner-action hook cannot run child-only {action}"
        ))
    }
}

impl<Body> FrameDocumentDynamicImportOwnerActionHooks
    for ScriptVmMainDynamicImportOwnerActionHooks<'_, '_, Body>
where
    Body: ScriptVmMainNativeModuleTaskBody,
{
    fn finish_terminal_client(
        &mut self,
        _action: FrameDocumentDynamicImportTerminalClientAction,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalClientFinishResult, String> {
        self.unsupported("terminal client action")
    }

    fn queue_owner_action_followups(
        &mut self,
        _actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> std::result::Result<FrameDocumentModuleTerminalQueueFollowup, String> {
        self.unsupported("owner action follow-ups")
    }

    fn record_missing_joined_terminal_client(
        &mut self,
        _missing: FrameDocumentDynamicImportMissingJoinedTerminalClient,
    ) -> std::result::Result<(), String> {
        self.unsupported("missing joined terminal client")
    }

    fn settle_owner_module_fetch_completion(
        &mut self,
        _action: ChildDynamicModuleOwnerFetchCompletionSettlementAction,
    ) -> std::result::Result<FrameDocumentDynamicImportOwnerFetchSettlementResult, String> {
        self.unsupported("owner module fetch completion")
    }

    fn restore_completed_owner_module_fetch_as_joined_terminal_client(
        &mut self,
        _restore: ChildDynamicModuleCompletedFetchRestoreAction,
    ) -> std::result::Result<FrameDocumentDynamicImportJoinedFetchRestoreResult, String> {
        self.unsupported("completed owner module fetch restore")
    }

    fn finish_owner_module_fetch_without_network(
        &mut self,
        _action: ChildDynamicModuleOwnerFetchWithoutNetworkSettlementAction,
    ) -> std::result::Result<FrameDocumentDynamicImportOwnerFetchSettlementResult, String> {
        self.unsupported("owner module fetch without network")
    }

    fn restore_scheduled_fetch_as_joined_terminal_client(
        &mut self,
        _action: FrameDocumentDynamicImportOwnerTerminalRestoreAction,
    ) -> std::result::Result<FrameDocumentDynamicImportJoinedFetchRestoreResult, String> {
        self.unsupported("scheduled fetch restore")
    }

    fn schedule_waiting_fetch(
        &mut self,
        _action: FrameDocumentDynamicImportWaitingFetchScheduleAction,
    ) -> std::result::Result<FrameDocumentDynamicImportWaitingFetchScheduleResult, String> {
        self.unsupported("waiting fetch schedule")
    }

    fn record_missing_joined_terminal_fetch(
        &mut self,
        _missing: FrameDocumentDynamicImportMissingJoinedTerminalFetch,
    ) -> std::result::Result<(), String> {
        self.unsupported("missing joined terminal fetch")
    }

    fn resolve_ready_source_import(
        &mut self,
        action: FrameDocumentDynamicImportSourceReadyAction,
    ) -> std::result::Result<FrameDocumentDynamicImportSourceReadyResult, String> {
        let (request, root_entry) = action.into_parts();
        let document_owner = request.owner();
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped stale dynamic import source resolution: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportSourceReadyResult::DroppedStaleOwner);
        }
        self.body.note_page_realm_body_attempted();
        match self
            .body
            .resolve_ready_source_import(self.vm, request, root_entry)
            .map_err(|error| error.message().to_owned())?
        {
            NativeDynamicModuleSourceImportResolution::Resolved => {
                Ok(FrameDocumentDynamicImportSourceReadyResult::Resolved)
            }
            NativeDynamicModuleSourceImportResolution::Rejected => {
                Ok(FrameDocumentDynamicImportSourceReadyResult::Rejected)
            }
        }
    }

    fn continue_ready_evaluation_import(
        &mut self,
        action: FrameDocumentDynamicImportEvaluationReadyAction,
    ) -> std::result::Result<FrameDocumentDynamicImportEvaluationReadyResult, String> {
        let (request, graph) = action.into_parts();
        let document_owner = request.owner();
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped stale dynamic import before evaluation: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportEvaluationReadyResult::DroppedStaleOwner);
        }
        self.body.note_page_realm_body_attempted();
        let evaluation = self.vm.start_native_dynamic_module_import_evaluation(graph);
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped dynamic import settlement after evaluation replaced its document: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportEvaluationReadyResult::DroppedStaleOwner);
        }
        match evaluation {
            Ok(DynamicModuleImportEvaluationStart::Completed(target)) => {
                self.body
                    .resolve_completed_evaluation_import(self.vm, request, &target)
                    .map_err(|error| error.message().to_owned())?;
                Ok(FrameDocumentDynamicImportEvaluationReadyResult::Resolved)
            }
            Ok(DynamicModuleImportEvaluationStart::Pending { target, promise }) => {
                self.vm
                    .attach_native_dynamic_module_import_reactions(request, target, promise)
                    .map_err(|error| error.message().to_owned())?;
                Ok(FrameDocumentDynamicImportEvaluationReadyResult::Pending)
            }
            Err(error) => {
                self.body
                    .reject_dynamic_import(self.vm, request, &error)
                    .map_err(|error| error.message().to_owned())?;
                Ok(FrameDocumentDynamicImportEvaluationReadyResult::Rejected)
            }
        }
    }

    fn record_restored_after_unexpected_complete(
        &mut self,
        _diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
    ) -> std::result::Result<(), String> {
        self.unsupported("unexpected complete warning")
    }

    fn reject_dynamic_import(
        &mut self,
        action: FrameDocumentDynamicImportRejectAction,
    ) -> std::result::Result<FrameDocumentDynamicImportRejectResult, String> {
        let (request, error) = action.into_parts();
        let document_owner = request.owner();
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped stale dynamic import rejection: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportRejectResult::DroppedStaleOwner);
        }
        self.body.note_page_realm_body_attempted();
        self.body
            .reject_dynamic_import(self.vm, request, &error)
            .map_err(|error| error.message().to_owned())?;
        Ok(FrameDocumentDynamicImportRejectResult::Rejected)
    }

    fn record_action_resumed(
        &mut self,
        _diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
    ) {
    }

    fn record_action_failed(
        &mut self,
        _diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
        _error: &str,
    ) {
    }
}

fn chromium_module_key(key: &ModuleMapKey) -> moli_module_script_tree::ModuleMapKey {
    moli_module_script_tree::ModuleMapKey::new(
        key.url().clone(),
        match key.kind() {
            ModuleKind::JavaScript => moli_module_script_tree::ModuleKind::JavaScript,
            ModuleKind::Json => moli_module_script_tree::ModuleKind::Json,
            ModuleKind::Css => moli_module_script_tree::ModuleKind::Css,
            ModuleKind::ModulePreloadText => moli_module_script_tree::ModuleKind::JavaScript,
            ModuleKind::WebAssembly => moli_module_script_tree::ModuleKind::WebAssembly,
        },
        moli_module_script_tree::ModuleAttributesKey::from_pairs(key.attributes().pairs().to_vec()),
    )
}

fn native_dynamic_import_owner_fetch_starts_for_continuation(
    continuation: &DynamicModuleFetchContinuation,
) -> Vec<Option<FrameDocumentModuleFetchClientStart>> {
    continuation
        .pending_fetch_requests()
        .map(|requests| vec![None; requests.len()])
        .unwrap_or_default()
}

fn module_script_source_for_runtime_graph_start(
    vm: &mut ScriptVm,
    script: &PreparedScript,
) -> std::result::Result<(ModuleSource, bool), ModuleLoadError> {
    match &script.source {
        crate::planning::ScriptSource::External => Ok((ModuleSource::text(String::new()), true)),
        crate::planning::ScriptSource::Loaded(source) => {
            Ok((ModuleSource::text(source.clone()), true))
        }
        crate::planning::ScriptSource::LoadedBinary { bytes, .. } => {
            Ok((ModuleSource::binary(bytes.clone()), true))
        }
        crate::planning::ScriptSource::Inline(source) => Ok((
            vm.inline_module_script_source_for_graph_start(script, source),
            false,
        )),
    }
}

fn runtime_owned_module_script_graph_job_for_prepared_script(
    vm: &mut ScriptVm,
    script: &PreparedScript,
) -> std::result::Result<NativeModuleGraphJob, ModuleLoadError> {
    match &script.source {
        crate::planning::ScriptSource::External => Ok(
            crate::module_runtime::runtime_owned_external_module_script_graph_job(
                vm,
                &script.url,
                &script.initiator_url,
                &script.fetch_metadata,
            ),
        ),
        crate::planning::ScriptSource::Loaded(_)
        | crate::planning::ScriptSource::LoadedBinary { .. }
        | crate::planning::ScriptSource::Inline(_) => {
            let (source, source_is_external) =
                module_script_source_for_runtime_graph_start(vm, script)?;
            crate::module_runtime::runtime_owned_loaded_module_script_graph_job(
                vm,
                source,
                &script.url,
                &script.initiator_url,
                &script.fetch_metadata,
                source_is_external,
            )
        }
    }
}

const DYNAMIC_MODULE_REACTION_ID_SLOT: &str = "reactionId";
const MODULE_SCRIPT_REACTION_ID_SLOT: &str = "moduleScriptReactionId";
const MODULE_REACTION_SCHEDULER_LANE_ID_SLOT: &str = "schedulerLaneId";
const MODULE_REACTION_LOCAL_WINDOW_ID_SLOT: &str = "localWindowId";
const MODULE_REACTION_DOCUMENT_ID_SLOT: &str = "documentId";
const MODULE_REACTION_REALM_ID_SLOT: &str = "realmId";

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct NativeDynamicModuleReactionDataDeclaration<'scope> {
    reaction_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct NativeModuleScriptReactionDataDeclaration<'scope> {
    module_script_reaction_id: v8::Local<'scope, v8::BigInt>,
    scheduler_lane_id: v8::Local<'scope, v8::BigInt>,
    local_window_id: v8::Local<'scope, v8::BigInt>,
    document_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct NativeChildModuleScriptReactionDataDeclaration<'scope> {
    module_script_reaction_id: v8::Local<'scope, v8::BigInt>,
    scheduler_lane_id: v8::Local<'scope, v8::BigInt>,
    local_window_id: v8::Local<'scope, v8::BigInt>,
    document_id: v8::Local<'scope, v8::BigInt>,
    realm_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeModuleEvaluationOwner {
    Script,
    DynamicImport,
}

pub(crate) struct NativeDynamicModuleEvaluation {
    target: DynamicModuleEvaluationTarget,
    promise: Option<v8::Global<v8::Promise>>,
}

struct NativeModuleEvaluationResult {
    module: v8::Global<v8::Module>,
    promise: Option<v8::Global<v8::Promise>>,
}

enum DynamicModuleImportEvaluationStart {
    Completed(DynamicModuleEvaluationTarget),
    Pending {
        target: DynamicModuleEvaluationTarget,
        promise: v8::Global<v8::Promise>,
    },
}

pub(crate) enum RuntimeModuleScriptGraphStart {
    NotModuleScript,
    Started(NativeModuleOwnerActions),
}

impl NativeDynamicModuleEvaluation {
    pub(crate) fn into_parts(
        self,
    ) -> (
        DynamicModuleEvaluationTarget,
        Option<v8::Global<v8::Promise>>,
    ) {
        (self.target, self.promise)
    }
}

impl ScriptVm {
    pub(crate) fn current_main_document_task_owner(&self) -> Option<FrameDocumentTaskOwner> {
        self._context_host
            .borrow()
            .current_main_document_task_owner()
    }

    pub(crate) fn current_main_parser_module_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainParserModuleGraphFetchTarget> {
        let pending_script_id = self
            .document_runtime
            .parser_module_scripts()
            .pending_script_id_for_fetch(load_id)?;
        (self.current_main_document_task_owner() == Some(pending_script_id.owner().task_owner()))
            .then(|| {
                crate::page_resource_completion::MainParserModuleGraphFetchTarget::new(
                    pending_script_id,
                    load_id,
                )
            })
    }

    pub(crate) fn main_parser_module_graph_fetch_target_is_current(
        &self,
        target: crate::page_resource_completion::MainParserModuleGraphFetchTarget,
    ) -> bool {
        self.current_main_document_task_owner() == Some(target.document_owner())
            && (self.current_main_parser_module_graph_fetch_target(target.load_id())
                == Some(target)
                || self
                    .document_runtime
                    .has_inflight_native_module_script_fetch(target.load_id()))
    }

    pub(crate) fn current_main_runtime_module_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget> {
        let document_owner = self.current_main_document_task_owner()?;
        let dynamic_script_owner_id = self
            .document_runtime
            .runtime_script_work()
            .dynamic_scripts
            .module_script_owner_id_for_pending_fetch(load_id)?;
        Some(
            crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget::new(
                document_owner,
                dynamic_script_owner_id,
                load_id,
            ),
        )
    }

    pub(crate) fn main_runtime_module_graph_fetch_target_is_current(
        &self,
        target: crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget,
    ) -> bool {
        self.current_main_document_task_owner() == Some(target.document_owner())
            && (self.current_main_runtime_module_graph_fetch_target(target.load_id())
                == Some(target)
                || self
                    .document_runtime
                    .has_inflight_native_module_script_fetch(target.load_id()))
    }

    fn main_dynamic_import_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainDynamicImportGraphFetchTarget> {
        let import_owner = self.document_runtime.with_native_module_owner(|owner| {
            owner.inflight_native_dynamic_module_import_fetch_owner(load_id)
        })?;
        import_owner.child_handle().is_none().then(|| {
            crate::page_resource_completion::MainDynamicImportGraphFetchTarget::new(
                import_owner,
                load_id,
            )
        })
    }

    pub(crate) fn current_main_dynamic_import_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainDynamicImportGraphFetchTarget> {
        let target = self.main_dynamic_import_graph_fetch_target(load_id)?;
        self.dynamic_module_import_owner_is_current(target.import_owner())
            .then_some(target)
    }

    /// Retires an old exact dynamic-import fetch after its network terminal is
    /// dequeued for a no-longer-current Document snapshot.
    ///
    /// A replacement PageVm may reuse `load_id`, so both the resolver-owned
    /// import identity and the load id must match before the old wait is
    /// removed. This preserves the pre-migration behavior without allowing a
    /// stale terminal to advance or delete replacement work.
    pub(crate) fn retire_stale_main_dynamic_import_graph_fetch(
        &mut self,
        target: crate::page_resource_completion::MainDynamicImportGraphFetchTarget,
    ) -> bool {
        if self.main_dynamic_import_graph_fetch_target(target.load_id()) != Some(target) {
            return false;
        }
        self.document_runtime
            .take_inflight_native_dynamic_module_import_fetch(target.load_id())
            .is_some()
    }

    pub(crate) fn current_main_modulepreload_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainModulepreloadFetchTarget> {
        let document_owner = self.current_main_document_task_owner()?;
        let is_inflight = self.document_runtime.with_native_module_owner(|owner| {
            owner.has_inflight_native_modulepreload_fetch_for(load_id)
        });
        is_inflight.then(|| {
            crate::page_resource_completion::MainModulepreloadFetchTarget::new(
                document_owner,
                load_id,
            )
        })
    }

    pub(crate) fn claim_main_parser_deferred_script(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: PreparedScript,
        shared_load: Option<crate::planning::SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: std::collections::HashSet<
            crate::DocumentBlockingStylesheetSignature,
        >,
    ) -> Result<bool> {
        let Some(start) = self.accept_main_parser_deferred_script(
            task_owner,
            script,
            shared_load,
            document_character_set,
            blocking_signatures_before,
        ) else {
            return Ok(false);
        };
        self.start_main_parser_deferred_script(start)?;
        Ok(true)
    }

    pub(super) fn accept_main_parser_deferred_script(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: PreparedScript,
        shared_load: Option<crate::planning::SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: std::collections::HashSet<
            crate::DocumentBlockingStylesheetSignature,
        >,
    ) -> Option<crate::document_runtime::PendingMainParserDeferredScriptStart> {
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping stale main parser-deferred preparation"
            );
            return None;
        }
        let Some(load_delay_token) = self
            ._context_host
            .borrow_mut()
            .acquire_current_main_parser_deferred_script_load_delay(task_owner)
        else {
            tracing::debug!(
                ?task_owner,
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping main parser-deferred preparation without a current lifecycle owner"
            );
            return None;
        };
        let script_node_id = script.node_id;
        let script_url = script.url.clone();
        let start_action = self.document_runtime.accept_main_parser_deferred_script(
            task_owner,
            script,
            shared_load,
            document_character_set,
            blocking_signatures_before,
            load_delay_token,
        );
        let Some(start_action) = start_action else {
            let released = self
                ._context_host
                .borrow_mut()
                .release_main_parser_deferred_script_load_delay(task_owner, load_delay_token);
            debug_assert!(
                released,
                "rejected parser-deferred acceptance must release its lifecycle token"
            );
            return None;
        };
        tracing::debug!(
            ?task_owner,
            ?load_delay_token,
            ?script_node_id,
            script_url = %script_url,
            "accepted main parser-deferred PendingScript with lifecycle ownership"
        );
        Some(start_action)
    }

    pub(super) fn start_main_parser_deferred_script(
        &mut self,
        start: crate::document_runtime::PendingMainParserDeferredScriptStart,
    ) -> Result<()> {
        let (task_owner, load_delay_token, start_action) = start.into_parts();
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                "dropping stale accepted main parser-deferred start action"
            );
            return Ok(());
        }
        let document_loader = self.document_runtime.current_document_resource_loader();
        match start_action {
            ParserDeferredScriptStartAction::NoFetch => {}
            ParserDeferredScriptStartAction::ClassicSource(source_load_request) => {
                if let Some(document_loader) = document_loader.as_ref() {
                    let (document_url, request_url) =
                        source_load_request.network_attribution_urls();
                    let network_attribution = crate::page_resource_completion::
                        MainParserDeferredClassicSourceNetworkAttribution::new(
                            document_url,
                            request_url,
                        );
                    let source_load = source_load_request.start(
                        document_loader.request_client(),
                        document_loader.task_runner(),
                    );
                    let completion_tx = self._context_host.borrow().resource_completion_sender();
                    let (pending_script_id, source_load) = source_load.into_parts();
                    let completed_source_load = source_load.clone();
                    source_load.register_completion_wake(move || {
                        let outcome = completed_source_load.try_outcome().expect(
                            "script source completion callback requires a terminal outcome",
                        );
                        let _ = completion_tx.send_main_parser_deferred_classic_source_load(
                            ParserDeferredClassicSourceLoadCompletion::new(
                                pending_script_id,
                                outcome,
                            ),
                            network_attribution,
                        );
                    });
                } else {
                    self.complete_main_parser_deferred_classic_source_load(
                        source_load_request.into_failure_completion(
                            "parser-deferred classic script accepted without an installed loader",
                        ),
                    );
                }
            }
            ParserDeferredScriptStartAction::ModuleGraph(start) => {
                let (pending_script_id, script) = start.into_parts();
                let start_result = self
                    .accept_registered_parser_pending_module_script_graph_for_document_owner(
                        pending_script_id,
                        &script,
                        None,
                    );
                let start_error = match start_result {
                    Ok(true) => None,
                    Ok(false) => Some(anyhow::anyhow!(
                        "registered parser module PendingScript {:?} did not start its graph",
                        pending_script_id
                    )),
                    Err(error) => Some(error),
                };
                if let Some(error) = start_error {
                    let canceled_token = self
                        .document_runtime
                        .parser_module_document_scripts_mut()
                        .cancel_parser_deferred_script(pending_script_id);
                    debug_assert_eq!(
                        canceled_token,
                        Some(load_delay_token),
                        "failed graph start must cancel the accepted PendingScript"
                    );
                    let released = self
                        ._context_host
                        .borrow_mut()
                        .release_main_parser_deferred_script_load_delay(
                            task_owner,
                            load_delay_token,
                        );
                    debug_assert!(
                        released,
                        "failed graph start must release its lifecycle token"
                    );
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn start_pending_main_parser_deferred_scripts(&mut self) -> Result<()> {
        let mut starts = self
            .document_runtime
            .take_main_parser_deferred_script_starts();
        while let Some(start) = starts.pop_front() {
            self.start_main_parser_deferred_script(start)?;
        }
        Ok(())
    }

    pub(crate) fn inline_module_script_source_for_graph_start(
        &mut self,
        script: &PreparedScript,
        source: &str,
    ) -> ModuleSource {
        let request = self.content_security_policy_script_element_request(script);
        // Module graphs compile their root before the later page-task
        // evaluation boundary. Feed that graph the Trusted Types/CSP-compliant
        // source now. A rejected inline source becomes an inert root so the
        // module owner can still retire its scheduling state without compiling
        // the rejected text.
        let source = self
            .inline_script_element_source_for_execution(script.node_id, source, request)
            .unwrap_or_default();
        ModuleSource::text(source)
    }

    pub(crate) fn seal_main_parser_deferred_scripts(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
    ) -> Option<crate::page_task_queue::PostParsePageOwnedWork> {
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                "dropping stale main parser-deferred EOF finalization"
            );
            return None;
        }
        let owner = MainParserDocumentOwner::new(task_owner);
        let initial_count = match self
            .document_runtime
            .parser_module_document_scripts_mut()
            .seal_parser_deferred_scripts(owner)
        {
            Ok(initial_count) => initial_count,
            Err(missing) => {
                self.record_runtime_warning(format_args!(
                    "dropping parser-deferred script queue because module PendingScript is missing for parser position {} node {:?}",
                    missing.parser_position(),
                    missing.script_node_id()
                ));
                return None;
            }
        };
        self.arm_main_parser_deferred_scripts(owner, initial_count)
    }

    pub(crate) fn complete_main_parser_deferred_classic_source_load(
        &mut self,
        completion: MainParserDeferredClassicSourceLoadCompletion,
    ) {
        let pending_script_id = completion.pending_script_id();
        let result = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .complete_parser_deferred_classic_source_load(completion);
        match result {
            ParserDeferredClassicSourceLoadApplyResult::Applied => {}
            ParserDeferredClassicSourceLoadApplyResult::MissingDocument => {
                tracing::debug!(
                    owner = ?pending_script_id.owner(),
                    parser_position = pending_script_id.parser_position(),
                    script_node_id = ?pending_script_id.script_node_id(),
                    "dropping parser-deferred classic source terminal for retired document"
                );
            }
            ParserDeferredClassicSourceLoadApplyResult::MissingPendingScript => {
                tracing::warn!(
                    owner = ?pending_script_id.owner(),
                    parser_position = pending_script_id.parser_position(),
                    script_node_id = ?pending_script_id.script_node_id(),
                    "dropping parser-deferred classic source terminal without PendingScript"
                );
            }
        }
    }

    fn arm_main_parser_deferred_scripts(
        &mut self,
        owner: MainParserDocumentOwner,
        initial_count: usize,
    ) -> Option<crate::page_task_queue::PostParsePageOwnedWork> {
        if initial_count == 0 {
            self.document_runtime
                .disarm_main_parser_deferred_scripts(owner.task_owner());
            return None;
        }
        self.document_runtime
            .arm_main_parser_deferred_scripts(owner.task_owner());
        Some(
            crate::page_task_queue::PostParsePageOwnedWork::main_parser_deferred_scripts(
                owner.task_owner(),
                initial_count,
            ),
        )
    }

    fn queue_main_module_script_graph_ready_work(
        &mut self,
        continuation: ModuleScriptContinuation,
    ) -> bool {
        assert_eq!(
            continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned ready graph continuations should be owned by DynamicScriptOwner"
        );
        let work = continuation.into_main_document_graph_ready_work();
        self.document_runtime
            .parser_module_document_scripts_mut()
            .notify_module_script_graph_ready_work(work)
    }

    fn queue_main_parser_module_graph_failure_work(
        &mut self,
        failure: ParserModuleScriptFailure,
    ) -> bool {
        assert_eq!(
            failure.continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned graph failures should be owned by DynamicScriptOwner"
        );
        self.document_runtime
            .parser_module_document_scripts_mut()
            .notify_module_script_graph_failed_action(DocumentOwnedScriptReadyAction::new(
                failure
                    .continuation
                    .parser_document_owner()
                    .expect("parser graph failure requires its original document owner"),
                failure,
            ))
    }

    fn queue_main_parser_module_evaluation_work(
        &mut self,
        evaluation: ModuleScriptEvaluationContinuation,
    ) {
        assert_eq!(
            evaluation.script_continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned module evaluation continuations should be owned by DynamicScriptOwner"
        );
        let owner = evaluation
            .script_continuation
            .parser_document_owner()
            .expect("parser module evaluation requires its original document owner");
        if evaluation.reaction_state.is_pending() {
            self.document_runtime
                .parser_module_document_scripts_mut()
                .push_pending_parser_module_evaluation_with_reaction_id(
                    DocumentOwnedScriptReadyAction::new(owner, evaluation.script_continuation),
                    evaluation.root_entry,
                    evaluation.reaction_id,
                );
        } else {
            self.document_runtime
                .parser_module_document_scripts_mut()
                .notify_module_script_evaluation_completed(DocumentOwnedScriptReadyAction::new(
                    owner, evaluation,
                ));
        }
    }

    pub(crate) fn clear_pending_module_script_fetches_for_script(
        &mut self,
        node_id: NodeId,
        owner_error: &ModuleLoadError,
    ) {
        let (stale_load_ids, stale_joined_clients) = self
            .document_runtime
            .parser_module_scripts_mut()
            .clear_pending_fetches_for_script(node_id);
        for load_id in stale_load_ids {
            tracing::debug!(
                load_id,
                owner_error = owner_error.message(),
                "detached pending module script fetch from failed owner; network completion will settle the module map entry"
            );
        }
        for client in stale_joined_clients {
            let detached = self
                .document_runtime
                .detach_native_module_fetch_waiter(client);
            tracing::debug!(
                ?client,
                detached,
                owner_error = owner_error.message(),
                "detached joined module script fetch client from failed owner"
            );
        }
    }

    fn clear_runtime_owned_module_script_graph_waits_for_owner(
        &mut self,
        continuation: &ModuleScriptContinuation,
        owner_error: &ModuleLoadError,
    ) {
        let Some(owner_id) = continuation.dynamic_script_owner_id() else {
            return;
        };
        let (stale_load_ids, stale_joined_clients) = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .clear_module_script_graph_pending_waits(owner_id);
        for load_id in stale_load_ids {
            tracing::debug!(
                load_id,
                dynamic_script_owner_id = ?owner_id,
                owner_error = owner_error.message(),
                "detached runtime-owned pending module script fetch from failed owner; network completion will settle the module map entry"
            );
        }
        for client in stale_joined_clients {
            let detached = self
                .document_runtime
                .detach_native_module_fetch_waiter(client);
            tracing::debug!(
                ?client,
                detached,
                dynamic_script_owner_id = ?owner_id,
                owner_error = owner_error.message(),
                "detached runtime-owned joined module script fetch client from failed owner"
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn complete_native_module_graph_fetch(
        &mut self,
        completion: ModuleGraphFetchCompletion,
    ) -> Result<()> {
        let load_id = completion.load_id;
        let has_modulepreload = self.document_runtime.with_native_module_owner(|owner| {
            owner.has_inflight_native_modulepreload_fetch_for(completion.load_id)
        });
        if has_modulepreload {
            if let Some(network_result) = completion.network_result.as_deref() {
                self.record_module_graph_subresource_network_result(&completion, network_result);
                self.record_modulepreload_resource_performance_entry(
                    &completion.request_url,
                    network_result,
                );
            }
            self.warn_on_module_graph_fetch_metadata_mismatch(
                &completion,
                ModuleGraphFetchRequester::ModulePreload,
                ModuleGraphFetchOrdering::BackgroundPreload,
            );
            if self
                .apply_main_modulepreload_fetch_result(load_id, completion.result)?
                .is_none()
            {
                return Err(anyhow::anyhow!(
                    "known legacy modulepreload fetch lost its in-flight request before application"
                ));
            }
            return Ok(());
        }
        if matches!(
            completion.requester,
            ModuleGraphFetchRequester::ParserOwnedModuleScript
                | ModuleGraphFetchRequester::RuntimeOwnedModuleScript
        ) {
            self.complete_abandoned_module_script_graph_fetch(completion)?;
            return Ok(());
        }
        let result_summary = match &completion.result {
            Ok(source) => format!("ok({} bytes)", source.len()),
            Err(error) => format!("error({error})"),
        };
        self.record_runtime_warning(format_args!(
            "native module graph fetch completion {load_id} arrived without an in-flight module graph job: requester={:?} ordering={:?} {result_summary}",
            completion.requester,
            completion.ordering
        ));
        Ok(())
    }

    fn apply_main_modulepreload_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<Option<ModuleEntryId>> {
        let Some(preload) = self
            .document_runtime
            .take_inflight_native_modulepreload_fetch(load_id)
        else {
            return Ok(None);
        };
        let fetch_key = preload.module_key().clone();
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                preload.fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(ModuleLoadStage::Fetch, error)),
        };
        let application = match source {
            Ok(fetched_source) => {
                let effective_key = preload.effective_key_for_fetched_source(&fetched_source);
                let effective_fetch_metadata =
                    preload.effective_fetch_metadata_for_fetched_source(&fetched_source);
                self.record_css_modulepreload_source_for_owner(&effective_key, &fetched_source);
                self.document_runtime
                    .insert_native_module_source_for_request(
                        fetch_key,
                        effective_key,
                        fetched_source.into_source(),
                        effective_fetch_metadata,
                    )
            }
            Err(error) => {
                self.record_css_modulepreload_failure_for_owner(&fetch_key);
                self.document_runtime
                    .mark_native_module_failed(fetch_key, error)
            }
        };
        Ok(Some(application))
    }

    pub(crate) fn apply_live_main_modulepreload_fetch_completion(
        &mut self,
        authorization: crate::runtime::AuthorizedLiveMainModulepreloadFetchCompletion,
    ) -> Result<()> {
        let completion = authorization.into_completion();
        let target = completion.target();
        self.apply_main_modulepreload_fetch_result(target.load_id(), completion.into_result())?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "authorized main modulepreload terminal lost its exact in-flight request"
                )
            })?;
        Ok(())
    }

    pub(crate) fn accept_main_parser_async_module_script(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: &PreparedScript,
    ) -> Result<bool> {
        if script.kind != crate::types::ScriptKind::Module
            || script.mode != crate::types::ScriptMode::Async
        {
            return Ok(false);
        }
        let Some(binding) = self.accept_main_document_script_load_delay_binding(
            task_owner,
            MainDocumentScriptLoadDelayKind::Module,
        ) else {
            return Ok(false);
        };
        self.accept_main_parser_async_module_script_with_binding(task_owner, script, binding)
    }

    pub(crate) fn accept_main_parser_async_module_admission(
        &mut self,
        admission: crate::document_script_scheduler::MainParserAsyncModuleAdmission,
    ) -> Result<bool> {
        let (script, binding) = admission.into_parts();
        let task_owner = binding.owner();
        self.accept_main_parser_async_module_script_with_binding(task_owner, &script, binding)
    }

    fn accept_main_parser_async_module_script_with_binding(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: &PreparedScript,
        binding: MainDocumentScriptLoadDelayLease,
    ) -> Result<bool> {
        assert_eq!(
            binding.owner(),
            task_owner,
            "parser async-module admission lease must target its exact PendingScript owner"
        );
        assert_eq!(
            binding.kind(),
            MainDocumentScriptLoadDelayKind::Module,
            "parser async-module admission requires a module load-delay lease"
        );
        assert_eq!(
            (script.kind, script.mode),
            (
                crate::types::ScriptKind::Module,
                crate::types::ScriptMode::Async
            ),
            "parser async-module admission requires an async module"
        );
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping stale main parser async module acceptance"
            );
            return Ok(false);
        }
        let owner = MainParserDocumentOwner::new(task_owner);
        let pending_script_id =
            ParserPendingScriptId::from_key(owner, ParserPendingScriptKey::from_script(script));
        if self
            .document_runtime
            .parser_module_document_scripts()
            .has_module_script(pending_script_id)
        {
            self.record_runtime_warning(format_args!(
                "rejecting duplicate main parser async module PendingScript {:?}",
                pending_script_id
            ));
            let _ = self
                ._context_host
                .borrow_mut()
                .release_main_document_script_load_delay(binding);
            return Ok(false);
        }
        let load_delay_token = binding.load_delay_token();
        let watch = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .register_and_watch_module_script(owner, script);
        debug_assert_eq!(watch.pending_script_id(), pending_script_id);
        if !watch.watched() {
            let _ = self
                .document_runtime
                .parser_module_document_scripts_mut()
                .discard_module_script(pending_script_id);
            let settled = self
                ._context_host
                .borrow_mut()
                .release_main_document_script_load_delay(binding);
            tracing::debug!(
                ?task_owner,
                ?pending_script_id,
                ?load_delay_token,
                settled = ?settled,
                "cancelled main parser async module lifecycle binding after watch rejection"
            );
            return Ok(false);
        }
        let started = match self
            .accept_registered_parser_pending_module_script_graph_for_document_owner(
                pending_script_id,
                script,
                Some(binding),
            ) {
            Ok(started) => started,
            Err(error) => {
                let _ = self
                    .document_runtime
                    .parser_module_document_scripts_mut()
                    .discard_module_script(pending_script_id);
                return Err(error);
            }
        };
        if !started {
            let _ = self
                .document_runtime
                .parser_module_document_scripts_mut()
                .discard_module_script(pending_script_id);
            tracing::debug!(
                ?task_owner,
                ?pending_script_id,
                ?load_delay_token,
                "cancelled main parser async module after graph start rejected and released its lease"
            );
            return Ok(false);
        }
        tracing::debug!(
            ?task_owner,
            ?pending_script_id,
            ?load_delay_token,
            ready_before_graph_start = watch.queued_ready_work(),
            script_url = %script.url,
            "accepted and watched main parser async module before graph work"
        );
        if self
            .document_runtime
            .parser_module_document_scripts()
            .has_ready_work()
        {
            let _ = self.enqueue_parser_owned_module_continuation();
        }
        Ok(true)
    }

    fn accept_registered_parser_pending_module_script_graph_for_document_owner(
        &mut self,
        pending_script_id: ParserPendingScriptId<MainParserDocumentOwner>,
        script: &PreparedScript,
        mut load_delay_binding: Option<crate::frame_owner_model::MainDocumentScriptLoadDelayLease>,
    ) -> Result<bool> {
        let owner = pending_script_id.owner();
        if script.kind != crate::types::ScriptKind::Module
            || ParserPendingScriptId::from_key(owner, ParserPendingScriptKey::from_script(script))
                != pending_script_id
        {
            self.record_runtime_warning(format_args!(
                "dropping parser module graph start whose script does not match PendingScript {:?}",
                pending_script_id
            ));
            if let Some(binding) = load_delay_binding.take() {
                let _ = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
            }
            return Ok(false);
        }
        if self.current_main_document_task_owner() != Some(owner.task_owner()) {
            tracing::debug!(
                ?owner,
                current_owner = ?self.current_main_document_task_owner(),
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping stale registered parser module graph start"
            );
            if let Some(binding) = load_delay_binding.take() {
                let _ = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
            }
            return Ok(false);
        }
        if !self
            .document_runtime
            .parser_module_document_scripts()
            .has_module_script(pending_script_id)
        {
            self.record_runtime_warning(format_args!(
                "dropping parser module graph start without registered PendingScript {:?}",
                pending_script_id
            ));
            if let Some(binding) = load_delay_binding.take() {
                let _ = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
            }
            return Ok(false);
        }
        let mut continuation =
            ModuleScriptContinuation::new_parser(script.clone(), pending_script_id);
        if let Some(binding) = load_delay_binding.take() {
            continuation = continuation.with_main_document_load_delay_binding(binding);
        }
        if self
            .document_runtime
            .current_document_resource_loader()
            .is_none()
        {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "parser module graph accepted without a Document resource authority",
            );
            let pending_failure =
                self.notify_module_script_graph_failure_for_owner(continuation, error);
            debug_assert!(
                pending_failure.is_none(),
                "parser module failure should stay on its watched PendingScript owner queue"
            );
            tracing::debug!(
                url = %script.url,
                position = script.position,
                node_id = ?script.node_id,
                "recorded missing-authority parser module graph failure on preparation-time PendingScript"
            );
            return Ok(true);
        }
        let job =
            match crate::module_runtime::parser_owned_module_script_graph_job_for_prepared_script(
                self, script,
            ) {
                Ok(Some(job)) => job,
                Ok(None) => {
                    let error = ModuleLoadError::new(
                        ModuleLoadStage::Fetch,
                        "registered parser module PendingScript produced no module graph job",
                    );
                    let pending_failure =
                        self.notify_module_script_graph_failure_for_owner(continuation, error);
                    debug_assert!(
                        pending_failure.is_none(),
                        "parser module no-job failure should stay on its watched PendingScript owner queue"
                    );
                    return Ok(true);
                }
                Err(error) => {
                    let pending_failure =
                        self.notify_module_script_graph_failure_for_owner(continuation, error);
                    debug_assert!(
                        pending_failure.is_none(),
                        "parser module graph failure should stay on its watched PendingScript owner queue"
                    );
                    return Ok(true);
                }
            };
        let continuation = continuation.with_resumed_graph_job(job);
        let advance = continuation.advance_graph(self);
        let actions = self.handle_module_script_graph_advance_for_owner(advance);
        let (ready_scripts, ready_evaluations, runtime_failures) = actions.into_parts();
        debug_assert!(
            ready_scripts.is_empty() && ready_evaluations.is_empty() && runtime_failures.is_empty(),
            "parser-owned graph start should not produce immediate ready owner actions"
        );
        Ok(true)
    }

    pub(crate) fn start_runtime_module_script_graph_for_owner(
        &mut self,
        script: &PreparedScript,
        dynamic_script_owner_id: crate::dynamic_script_owner::DynamicScriptOwnerId,
    ) -> RuntimeModuleScriptGraphStart {
        if script.kind != crate::types::ScriptKind::Module || script.url.scheme() == "data" {
            return RuntimeModuleScriptGraphStart::NotModuleScript;
        }
        let document_owner = self
            .current_main_document_task_owner()
            .expect("runtime module graph start requires a current main Document owner");
        let continuation = ModuleScriptContinuation::new_runtime(
            script.clone(),
            dynamic_script_owner_id,
            document_owner,
        );
        let job = match runtime_owned_module_script_graph_job_for_prepared_script(self, script) {
            Ok(job) => job,
            Err(error) => {
                let actions = self
                    .notify_module_script_graph_failure_for_owner(continuation, error)
                    .map(|(continuation, error)| {
                        NativeModuleOwnerActions::from_runtime_module_failure(continuation, error)
                    })
                    .unwrap_or_else(NativeModuleOwnerActions::empty);
                return RuntimeModuleScriptGraphStart::Started(actions);
            }
        };
        let continuation = continuation.with_resumed_graph_job(job);
        let advance = continuation.advance_graph(self);
        let actions = self.handle_runtime_module_script_graph_advance_for_dynamic_owner(advance);
        RuntimeModuleScriptGraphStart::Started(actions)
    }

    pub(crate) fn dynamic_module_import_owner_is_current(
        &self,
        owner: crate::module_runtime::DynamicModuleImportOwner,
    ) -> bool {
        self._context_host
            .borrow()
            .dynamic_module_import_owner_is_current(owner)
    }

    fn advance_native_dynamic_module_import_job_with_body<Body>(
        &mut self,
        mut job: NativeModuleGraphJob,
        body: &mut Body,
    ) -> std::result::Result<(), String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        if job
            .dynamic_import_request()
            .and_then(PendingDynamicModuleImport::child_browsing_context_handle)
            .is_some()
        {
            return self.advance_child_native_dynamic_module_import_job(job);
        }
        let advance = match job.advance_dynamic_import_owner_lane(self) {
            Ok(advance) => advance,
            Err(error) => {
                let mut fanout = NativeDynamicModuleTerminalFanout::default();
                fanout.push_graph_advance_failure(job, error);
                return self
                    .handle_dynamic_module_terminal_fanout_for_owner_with_body(fanout, body)
                    .map(|_| ())
                    .map_err(|error| error.to_string());
            }
        };
        self.continue_native_dynamic_module_import_after_tree_advance_with_body(job, advance, body)
    }

    fn continue_native_dynamic_module_import_after_tree_advance_with_body<Body>(
        &mut self,
        job: NativeModuleGraphJob,
        advance: NativeModuleGraphJobAdvance,
        body: &mut Body,
    ) -> std::result::Result<(), String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        if job
            .dynamic_import_request()
            .and_then(PendingDynamicModuleImport::child_browsing_context_handle)
            .is_some()
        {
            return self
                .continue_child_native_dynamic_module_import_after_tree_advance(job, advance);
        }
        self.continue_main_native_dynamic_module_import_after_tree_advance_with_body(
            job, advance, body,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    fn continue_main_native_dynamic_module_import_after_tree_advance_with_body<Body>(
        &mut self,
        mut job: NativeModuleGraphJob,
        advance: NativeModuleGraphJobAdvance,
        body: &mut Body,
    ) -> Result<NativeDynamicModuleTerminalFanoutOutcome>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let mut outcome = NativeDynamicModuleTerminalFanoutOutcome::default();
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        match advance {
            NativeModuleGraphJobAdvance::NeedFetches(requests) => {
                let joined_clients = job.take_pending_joined_clients();
                let owner_module_fetch_starts = vec![None; requests.len()];
                let scheduled = self
                    .document_runtime
                    .suspend_native_dynamic_module_import_fetches(
                        requests,
                        joined_clients,
                        job,
                        owner_module_fetch_starts,
                    );
                fanout.extend_scheduled_dynamic_import_fetches(scheduled);
            }
            NativeModuleGraphJobAdvance::WaitingForFetches => {
                let joined_clients = job.take_pending_joined_clients();
                if joined_clients.is_empty() {
                    self.document_runtime
                        .resume_native_dynamic_module_import_front(job);
                    outcome.record_dynamic_import_job_resumed();
                } else {
                    self.document_runtime
                        .suspend_native_dynamic_module_import_fetches(
                            Vec::new(),
                            joined_clients,
                            job,
                            Vec::new(),
                        );
                    outcome.record_dynamic_import_wait_retained();
                }
            }
            NativeModuleGraphJobAdvance::Complete(graph) => {
                fanout.push_ready_import(NativeDynamicModuleImportReady { job, graph });
            }
        }
        outcome
            .merge(self.handle_dynamic_module_terminal_fanout_for_owner_with_body(fanout, body)?);
        Ok(outcome)
    }

    fn run_completed_native_dynamic_module_import_action_with_body<Body>(
        &mut self,
        job: NativeModuleGraphJob,
        graph: ModuleGraphHandle,
        body: &mut Body,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalOutcome, String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let document_owner = job
            .dynamic_import_request()
            .expect("completed dynamic import graph must retain its request")
            .owner();
        if let Some((_child_handle, task_owner, realm_id)) = document_owner.child_parts() {
            let owner = task_owner.document_owner();
            let followup = self
                .child_document_modulator_store
                .dynamic_import_ready_followup(
                    owner,
                    realm_id,
                    NativeDynamicModuleImportReady { job, graph },
                );
            return Ok(
                FrameDocumentDynamicImportTerminalOutcome::from_owner_action_queue_followup(
                    self.apply_child_dynamic_import_followup(followup),
                ),
            );
        }
        self.run_main_dynamic_import_owner_action_with_body(
            FrameDocumentDynamicImportOwnerAction::ready(NativeDynamicModuleImportReady {
                job,
                graph,
            }),
            body,
        )
    }

    fn run_main_dynamic_import_owner_action_with_body<Body>(
        &mut self,
        action: FrameDocumentDynamicImportOwnerAction,
        body: &mut Body,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalOutcome, String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        FrameDocumentDynamicImportOwnerActionRunner::new(
            ScriptVmMainDynamicImportOwnerActionHooks::new(self, body),
        )
        .run_owner_action(action)
    }

    pub(crate) fn has_ready_native_module_owner_actions(&mut self) -> bool {
        self.document_runtime.has_ready_native_module_owner_event()
    }

    #[cfg(test)]
    pub(crate) fn drain_ready_native_module_owner_actions(
        &mut self,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )> {
        self.drain_ready_native_module_owner_actions_with_body(
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
    }

    fn drain_ready_native_module_owner_actions_with_body<Body>(
        &mut self,
        body: &mut Body,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        if !self.has_ready_native_module_owner_actions() {
            return Ok((
                NativeModuleOwnerActions::empty(),
                FrameDocumentModuleTerminalQueueFollowup::none(),
            ));
        }
        let Some(event) = self.document_runtime.take_next_native_module_owner_event() else {
            return Ok((
                NativeModuleOwnerActions::empty(),
                FrameDocumentModuleTerminalQueueFollowup::none(),
            ));
        };
        self.dispatch_native_module_owner_event_with_body(event, body)
    }

    fn dispatch_native_module_owner_event_with_body<Body>(
        &mut self,
        event: NativeModuleOwnerEvent,
        body: &mut Body,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        match event {
            NativeModuleOwnerEvent::ModuleMapTerminalNotification(notification) => {
                let fanout =
                    self.dispatch_native_document_modulator_terminal_notification(notification)?;
                self.handle_document_modulator_terminal_fanout_for_owner_with_body(fanout, body)
            }
            NativeModuleOwnerEvent::ModulepreloadLinkError(handle) => {
                body.note_page_realm_body_attempted();
                self.dispatch_preload_like_link_error_event(handle);
                Ok((
                    NativeModuleOwnerActions::empty(),
                    FrameDocumentModuleTerminalQueueFollowup::none(),
                ))
            }
        }
    }

    pub(crate) fn has_ready_runtime_owned_module_owner_actions(&mut self) -> bool {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .has_ready_module_script_continuation()
    }

    pub(crate) fn register_native_modulepreload_for_owner(
        &mut self,
        request: NativeModuleSingleFetchRequest,
    ) -> std::result::Result<Option<crate::module_runtime::ModulePreloadJobRun>, String> {
        let document_owner = self.current_main_document_task_owner().ok_or_else(|| {
            "main modulepreload fetch started without a current Document owner".to_owned()
        })?;
        let resource_scheduler = self.resource_scheduler();
        let outcome = self
            .document_runtime
            .start_main_document_modulepreload_fetch(document_owner, &resource_scheduler, request)
            .map_err(|error| error.message().to_owned())?;
        Ok(self.consume_main_document_modulepreload_fetch_outcome(outcome))
    }

    pub(crate) fn register_native_modulepreload_link_for_owner(
        &mut self,
        request: NativeModuleSingleFetchRequest,
        link_client: std::sync::Arc<crate::module_runtime::NativeModulepreloadLinkClient>,
    ) -> std::result::Result<Option<crate::module_runtime::ModulePreloadJobRun>, String> {
        let start = self
            .document_runtime
            .fetch_single_native_module_for_modulepreload_link(request, link_client)
            .map_err(|error| error.message().to_owned())?;
        self.run_native_modulepreload_fetch_start_for_owner(start)
    }

    fn run_native_modulepreload_fetch_start_for_owner(
        &mut self,
        start: NativeModulepreloadFetchStart,
    ) -> std::result::Result<Option<crate::module_runtime::ModulePreloadJobRun>, String> {
        let Some(request) = start.started_request() else {
            return Ok(None);
        };
        let document_owner = self.current_main_document_task_owner().ok_or_else(|| {
            "main modulepreload fetch started without a current Document owner".to_owned()
        })?;
        let resource_scheduler = self.resource_scheduler();
        let outcome = self
            .document_runtime
            .schedule_reserved_main_document_modulepreload_fetch(
                document_owner,
                &resource_scheduler,
                request,
            )
            .map_err(|error| error.message().to_owned())?;
        Ok(self.consume_main_document_modulepreload_fetch_outcome(outcome))
    }

    pub(crate) fn drain_ready_runtime_owned_module_owner_actions(
        &mut self,
    ) -> Result<NativeModuleOwnerActions> {
        let Some(work) = self.take_ready_runtime_owned_module_script_continuation_work() else {
            return Ok(NativeModuleOwnerActions::empty());
        };
        Ok(Self::runtime_owned_module_owner_actions_from_work(work))
    }

    fn runtime_owned_module_owner_actions_from_work(
        work: crate::dynamic_script_owner::DynamicModuleScriptContinuationWork,
    ) -> NativeModuleOwnerActions {
        let mut actions = NativeModuleOwnerActions::empty();
        match work {
            crate::dynamic_script_owner::DynamicModuleScriptContinuationWork::Graph {
                continuation,
            } => actions.push_ready_module_script(*continuation),
            crate::dynamic_script_owner::DynamicModuleScriptContinuationWork::Evaluation {
                evaluation,
            } => actions.push_ready_module_evaluation(*evaluation),
        }
        actions
    }

    fn consume_main_document_modulepreload_fetch_outcome(
        &mut self,
        outcome: crate::document_runtime::MainDocumentModulepreloadFetchOutcome,
    ) -> Option<crate::module_runtime::ModulePreloadJobRun> {
        let (job_run, csp_violations, runtime_warning) = outcome.into_parts();
        for violation in csp_violations {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
        if let Some(runtime_warning) = runtime_warning {
            self.record_runtime_warning(format_args!("{runtime_warning}"));
        }
        job_run
    }

    fn handle_document_modulator_terminal_fanout_for_owner_with_body<Body>(
        &mut self,
        fanout: ModuleMapTerminalFanout,
        body: &mut Body,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let (module_script_results, dynamic_import_fanout) = fanout.into_parts();
        let mut actions = NativeModuleOwnerActions::empty();
        for result in module_script_results {
            actions.merge(self.handle_module_script_graph_advance_for_owner(result));
        }
        let dynamic_import_outcome = self
            .handle_dynamic_module_terminal_fanout_for_owner_with_body(
                dynamic_import_fanout,
                body,
            )?;
        Ok((actions, dynamic_import_outcome.child_followup()))
    }

    #[cfg(test)]
    fn handle_dynamic_module_terminal_fanout_for_owner(
        &mut self,
        fanout: NativeDynamicModuleTerminalFanout,
    ) -> Result<NativeDynamicModuleTerminalFanoutOutcome> {
        self.handle_dynamic_module_terminal_fanout_for_owner_with_body(
            fanout,
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
    }

    fn handle_dynamic_module_terminal_fanout_for_owner_with_body<Body>(
        &mut self,
        fanout: NativeDynamicModuleTerminalFanout,
        body: &mut Body,
    ) -> Result<NativeDynamicModuleTerminalFanoutOutcome>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let (
            dynamic_imports,
            scheduled_dynamic_import_fetches,
            failed_fetches,
            graph_advance_failures,
            restored_after_unexpected_complete,
        ) = fanout.into_parts();
        let mut outcome = NativeDynamicModuleTerminalFanoutOutcome::default();
        let scheduled_fetch_count = scheduled_dynamic_import_fetches.len();
        self.schedule_native_dynamic_module_import_fetches(scheduled_dynamic_import_fetches);
        outcome.record_scheduled_dynamic_import_fetches(scheduled_fetch_count);
        for dynamic_import in dynamic_imports {
            let ready_outcome = self
                .run_completed_native_dynamic_module_import_action_with_body(
                    dynamic_import.job,
                    dynamic_import.graph,
                    body,
                )
                .map_err(|error| anyhow::anyhow!(error))?;
            outcome.record_ready_import_outcome(ready_outcome);
        }
        for failure in failed_fetches {
            let failure_outcome = self
                .run_failed_native_dynamic_module_fetch_action_with_body(failure, body)
                .map_err(|error| anyhow::anyhow!(error))?;
            outcome.record_failed_fetch_rejection_outcome(failure_outcome);
        }
        for (job, error) in graph_advance_failures {
            if job
                .dynamic_import_request()
                .and_then(PendingDynamicModuleImport::child_browsing_context_handle)
                .is_some()
            {
                let followup = self
                    .enqueue_child_dynamic_import_graph_advance_failed_owner_action(job, error)
                    .map_err(|error| anyhow::anyhow!(error))?;
                outcome.record_graph_advance_failure_followup(followup);
            } else {
                let failure_outcome = self
                    .run_main_dynamic_import_owner_action_with_body(
                        FrameDocumentDynamicImportOwnerAction::graph_advance_failed(job, error),
                        body,
                    )
                    .map_err(|error| anyhow::anyhow!(error))?;
                outcome.record_graph_advance_failure_outcome(failure_outcome);
            }
        }
        if restored_after_unexpected_complete {
            self.record_runtime_warning(format_args!(
                "native dynamic import tree completed while its pending tree still had clients"
            ));
            outcome.record_restored_after_unexpected_complete();
        }
        Ok(outcome)
    }

    fn handle_module_script_graph_fetch_resume_for_owner(
        &mut self,
        resume: ModuleScriptGraphFetchResume,
    ) -> Result<NativeModuleOwnerActions> {
        match resume {
            ModuleScriptGraphFetchResume::Finished { result } => {
                Ok(self.handle_module_script_graph_advance_for_owner(*result))
            }
            ModuleScriptGraphFetchResume::RestoredMissingGraphContinuation => {
                Ok(NativeModuleOwnerActions::empty())
            }
        }
    }

    fn schedule_native_dynamic_module_import_fetches(
        &mut self,
        scheduled: Vec<DynamicModuleScheduledFetch>,
    ) {
        let document_loader = self
            .document_runtime
            .current_document_resource_loader()
            .expect("main dynamic import requires the committed Document resource authority");
        for scheduled_fetch in scheduled {
            let (load_id, request, _) = scheduled_fetch.into_parts();
            let import_owner = self
                .document_runtime
                .with_native_module_owner(|owner| {
                    owner.inflight_native_dynamic_module_import_fetch_owner(load_id)
                })
                .expect("scheduled dynamic-import fetch must retain its resolver owner");
            assert!(
                import_owner.child_handle().is_none(),
                "main dynamic-import scheduler cannot publish a child fetch"
            );
            let target = crate::page_resource_completion::MainDynamicImportGraphFetchTarget::new(
                import_owner,
                load_id,
            );
            let document_url = self.document_runtime.document_url().clone();
            self.resource_scheduler()
                .schedule_main_dynamic_import_graph_fetch(
                    document_loader.clone(),
                    target,
                    request,
                    document_url,
                );
        }
    }

    fn complete_shared_module_map_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<()> {
        let graph_continuation = self
            .document_runtime
            .take_inflight_native_module_script_fetch(load_id)
            .expect("authorized shared module-map terminal must retain its in-flight fetch");
        let fetch_key = graph_continuation
            .request()
            .pending_fetch_key()
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "authorized shared module-map terminal {load_id} has no module map key"
                )
            })?;
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                graph_continuation.request().fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("native module script fetch completion {load_id} failed: {error}"),
            )),
        };
        match source {
            Ok(fetched_source) => {
                let effective_key = graph_continuation
                    .request()
                    .effective_key_for_fetched_source(&fetched_source)
                    .unwrap_or_else(|| fetch_key.clone());
                let effective_fetch_metadata = graph_continuation
                    .request()
                    .effective_fetch_metadata_for_fetched_source(&fetched_source);
                self.document_runtime
                    .insert_native_module_source_for_request(
                        fetch_key,
                        effective_key,
                        fetched_source.into_source(),
                        effective_fetch_metadata,
                    );
            }
            Err(error) => {
                self.document_runtime
                    .mark_native_module_failed(fetch_key, error);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn complete_abandoned_module_script_graph_fetch(
        &mut self,
        completion: ModuleGraphFetchCompletion,
    ) -> Result<()> {
        let load_id = completion.load_id;
        if !self
            .document_runtime
            .has_inflight_native_module_script_fetch(load_id)
        {
            let result_summary = match &completion.result {
                Ok(source) => format!("ok({} bytes)", source.len()),
                Err(error) => format!("error({error})"),
            };
            self.record_runtime_warning(format_args!(
                "native module script fetch completion {load_id} arrived without an owner or in-flight module map continuation: requester={:?} ordering={:?} {result_summary}",
                completion.requester,
                completion.ordering
            ));
            return Ok(());
        }
        if let Some(network_result) = completion.network_result.as_deref() {
            self.record_module_graph_subresource_network_result(&completion, network_result);
        }
        self.complete_shared_module_map_fetch_result(load_id, completion.result)
    }

    fn dynamic_module_fetch_resume_advance_for_owner_with_body<Body>(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        body: &mut Body,
    ) -> Result<FrameDocumentModuleTerminalQueueFollowup>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let owner_module_fetch_starts =
            native_dynamic_import_owner_fetch_starts_for_continuation(&continuation);
        let advance = self
            .document_runtime
            .continue_native_dynamic_module_import_fetch(continuation, owner_module_fetch_starts);
        Ok(self
            .handle_dynamic_module_terminal_fanout_for_owner_with_body(
                NativeDynamicModuleTerminalFanout::from_owner_advance(advance),
                body,
            )?
            .child_followup())
    }

    fn dynamic_module_fetch_finish_to_owner_actions_with_body<Body>(
        &mut self,
        finish: DynamicModuleFetchFinish,
        body: &mut Body,
    ) -> Result<FrameDocumentModuleTerminalQueueFollowup>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        match finish {
            DynamicModuleFetchFinish::Advanced(continuation) => {
                self.dynamic_module_fetch_resume_advance_for_owner_with_body(continuation, body)
            }
            DynamicModuleFetchFinish::Failed(failure) => {
                let mut fanout = NativeDynamicModuleTerminalFanout::default();
                fanout.push_failed_fetch(failure);
                Ok(self
                    .handle_dynamic_module_terminal_fanout_for_owner_with_body(fanout, body)?
                    .child_followup())
            }
        }
    }

    fn dispatch_native_document_modulator_terminal_notification(
        &mut self,
        notification: ModuleMapTerminalNotification,
    ) -> Result<ModuleMapTerminalFanout> {
        let (key, clients, successful) = notification.into_parts();
        let (fetch_clients, _, modulepreload_link_clients) = clients.into_parts();
        debug_assert!(
            modulepreload_link_clients
                .iter()
                .all(|client| client.frame_document_client().is_none())
        );
        self.document_runtime
            .complete_native_modulepreload_link_clients(
                &key,
                modulepreload_link_clients,
                successful,
            );
        self.resume_native_document_modulator_fetch_clients(&key, fetch_clients)
    }

    fn resume_native_document_modulator_fetch_clients(
        &mut self,
        key: &ModuleMapKey,
        clients: Vec<NativeModuleMapSingleModuleClient>,
    ) -> Result<ModuleMapTerminalFanout> {
        let mut fanout = ModuleMapTerminalFanout::empty();
        if clients.is_empty() {
            return Ok(fanout);
        }
        tracing::debug!(
            url = %key.url(),
            client_count = clients.len(),
            "resuming native module map fetch clients"
        );
        for client in clients {
            tracing::trace!(
                url = %key.url(),
                client_name = client.client_name(),
                import_phase = ?client.import_phase(),
                token = ?client.token(),
                "dispatching native module map single-module client"
            );
            match client {
                NativeModuleMapSingleModuleClient::ModuleScript(client) => {
                    if let Some(result) = self.resume_module_script_fetch_join_waiter(key, client) {
                        fanout.push_module_script_result(result);
                    }
                }
                NativeModuleMapSingleModuleClient::DynamicImport(client) => {
                    self.resume_native_dynamic_module_fetch_waiter(key, client, &mut fanout)?;
                }
            }
        }
        Ok(fanout)
    }

    fn resume_native_dynamic_module_fetch_waiter(
        &mut self,
        key: &ModuleMapKey,
        client: NativeDynamicImportSingleModuleClient,
        fanout: &mut ModuleMapTerminalFanout,
    ) -> Result<()> {
        let client_token = client.token();
        let Some(joined) = self
            .document_runtime
            .take_joined_native_dynamic_module_import_fetch(client_token)
        else {
            self.record_runtime_warning(format_args!(
                "native module joined fetch client {:?} had no dynamic import continuation",
                client_token
            ));
            return Ok(());
        };
        debug_assert_eq!(
            joined.client(),
            client_token,
            "dynamic pending tree should return the requested joined client"
        );
        let document_owner = joined.owner();
        if !self.dynamic_module_import_owner_is_current(document_owner) {
            self.record_runtime_warning(format_args!(
                "dropped stale joined dynamic import terminal: client={client_token:?} owner={document_owner:?}"
            ));
            return Ok(());
        }
        match self.finish_native_dynamic_module_joined_fetch(joined, key) {
            DynamicModuleFetchFinish::Advanced(continuation) => {
                self.push_dynamic_module_fetch_resume_advance_into_fanout(continuation, fanout);
            }
            DynamicModuleFetchFinish::Failed(failure) => {
                fanout.push_dynamic_import_fetch_failure(failure);
            }
        }
        Ok(())
    }

    fn push_dynamic_module_fetch_resume_advance_into_fanout(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        fanout: &mut ModuleMapTerminalFanout,
    ) {
        let owner_module_fetch_starts =
            native_dynamic_import_owner_fetch_starts_for_continuation(&continuation);
        let advance = self
            .document_runtime
            .continue_native_dynamic_module_import_fetch(continuation, owner_module_fetch_starts);
        fanout.absorb_dynamic_import_owner_advance(advance);
    }

    fn run_failed_native_dynamic_module_fetch_action_with_body<Body>(
        &mut self,
        failure: DynamicModuleFetchFailure,
        body: &mut Body,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalOutcome, String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let (request, error) = self
            .document_runtime
            .clear_failed_native_dynamic_module_import_fetch(failure);
        self.run_main_dynamic_import_owner_action_with_body(
            FrameDocumentDynamicImportOwnerAction::fetch_failed(request, error),
            body,
        )
    }

    #[cfg(test)]
    pub(crate) fn record_module_graph_subresource_network_result(
        &mut self,
        completion: &ModuleGraphFetchCompletion,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        let document_url = self.document_runtime.document_url().clone();
        self._context_host
            .borrow_mut()
            .record_staged_get_subresource_network_result_with_initiator(
                None,
                document_url,
                completion.request_url.clone(),
                SubresourceResourceType::Script,
                match completion.requester {
                    ModuleGraphFetchRequester::DynamicImport => {
                        SubresourceRequestInitiatorType::Script
                    }
                    ModuleGraphFetchRequester::ParserOwnedModuleScript
                    | ModuleGraphFetchRequester::RuntimeOwnedModuleScript
                    | ModuleGraphFetchRequester::ModulePreload => {
                        SubresourceRequestInitiatorType::Parser
                    }
                },
                network_result,
            );
    }

    pub(crate) fn record_main_modulepreload_network_result(
        &mut self,
        document_url: Url,
        request_url: Url,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        self._context_host
            .borrow_mut()
            .record_get_subresource_network_result_with_initiator(
                None,
                document_url,
                request_url.clone(),
                SubresourceResourceType::Script,
                SubresourceRequestInitiatorType::Parser,
                network_result,
            );
        self.record_modulepreload_resource_performance_entry(&request_url, network_result);
    }

    pub(crate) fn record_historical_main_modulepreload_network_result(
        &mut self,
        document_url: Url,
        request_url: Url,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        self._context_host
            .borrow_mut()
            .record_historical_get_subresource_network_result_with_initiator(
                None,
                document_url,
                request_url,
                SubresourceResourceType::Script,
                SubresourceRequestInitiatorType::Parser,
                network_result,
            );
    }

    fn record_modulepreload_resource_performance_entry(
        &mut self,
        request_url: &Url,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        let performance_entry =
            crate::context_bootstrap::ResourcePerformanceEntry::from_network_result(
                request_url.as_str(),
                "link",
                None,
                network_result,
            );
        if let Err(error) = self.with_default_context_scope(|scope, _host_ptr| {
            crate::context_bootstrap::record_resource_performance_entry(scope, performance_entry);
            Ok(())
        }) {
            self.record_runtime_warning(format_args!(
                "failed to record native modulepreload performance entry for `{}`: {}",
                request_url, error
            ));
        }
    }

    fn record_css_modulepreload_source_for_owner(
        &mut self,
        key: &ModuleMapKey,
        fetched_source: &ModuleGraphFetchedSource,
    ) {
        if key.kind() != ModuleKind::Css {
            return;
        }
        let Some(source) = fetched_source.source().text_source() else {
            self.record_css_modulepreload_failure_for_owner(key);
            return;
        };
        self._context_host
            .borrow_mut()
            .record_css_module_text_for_url(key.url(), source.to_owned());
    }

    fn record_css_modulepreload_failure_for_owner(&mut self, key: &ModuleMapKey) {
        if key.kind() != ModuleKind::Css {
            return;
        }
        self._context_host
            .borrow_mut()
            .record_css_module_failure_for_url(key.url());
    }

    pub(crate) fn module_graph_fetched_source_or_csp_error(
        &mut self,
        load_id: u64,
        fetched_source: ModuleGraphFetchedSource,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<ModuleGraphFetchedSource, ModuleLoadError> {
        if fetched_source.redirected() {
            let redirect_status =
                crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect;
            let request = module_fetch_csp_request(fetch_metadata);
            if let Some(violation) = self
                .document_runtime
                .script_element_request_csp_report_only_violation_with_redirect_status(
                    fetched_source.final_url(),
                    redirect_status,
                    request,
                )
            {
                self.queue_content_security_policy_violation_event_best_effort(&violation);
            }
            if let Some(violation) = self
                .document_runtime
                .script_element_request_csp_violation_with_redirect_status(
                    fetched_source.final_url(),
                    redirect_status,
                    request,
                )
            {
                self.queue_content_security_policy_violation_event_best_effort(&violation);
                return Err(ModuleLoadError::new(
                    ModuleLoadStage::Fetch,
                    format!(
                        "native module graph fetch completion {load_id} blocked by document Content Security Policy for `{}`",
                        violation.blocked_uri
                    ),
                ));
            }
        }
        Ok(fetched_source)
    }

    pub(crate) fn csp_blocked_module_fetch_error_for_owner(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> Option<ModuleLoadError> {
        if !matches!(key.kind(), ModuleKind::JavaScript | ModuleKind::WebAssembly) {
            return None;
        }
        let violation = self
            .document_runtime
            .script_element_request_csp_violation_with_request(
                key.url(),
                module_fetch_csp_request(fetch_metadata),
            )?;
        self.queue_content_security_policy_violation_event_best_effort(&violation);
        Some(ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            format!(
                "Refused to load module `{}` because it violates the document Content Security Policy directive `{}`",
                key.url(),
                violation.effective_directive
            ),
        ))
    }

    pub(crate) fn dispatch_module_fetch_csp_report_only_violation_for_owner(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) {
        if !matches!(key.kind(), ModuleKind::JavaScript | ModuleKind::WebAssembly) {
            return;
        }
        if let Some(violation) = self
            .document_runtime
            .script_element_request_csp_report_only_violation_with_request(
                key.url(),
                module_fetch_csp_request(fetch_metadata),
            )
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
    }

    #[cfg(test)]
    pub(super) fn warn_on_module_graph_fetch_metadata_mismatch(
        &mut self,
        completion: &ModuleGraphFetchCompletion,
        expected_requester: ModuleGraphFetchRequester,
        expected_ordering: ModuleGraphFetchOrdering,
    ) {
        if completion.requester == expected_requester && completion.ordering == expected_ordering {
            return;
        }
        self.record_runtime_warning(format_args!(
            "module graph fetch completion {} metadata mismatch: got requester={:?} ordering={:?}, expected requester={:?} ordering={:?}",
            completion.load_id,
            completion.requester,
            completion.ordering,
            expected_requester,
            expected_ordering
        ));
    }

    pub(crate) fn note_module_script_evaluation_suspended_for_owner(
        &mut self,
        evaluation: ModuleScriptEvaluationContinuation,
    ) {
        match evaluation.script_continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => {
                self.queue_main_parser_module_evaluation_work(evaluation)
            }
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = evaluation
                    .script_continuation
                    .dynamic_script_owner_id()
                    .expect("runtime-owned module evaluation should carry dynamic owner id");
                self.note_runtime_owned_module_script_evaluation_suspended(
                    owner_id,
                    Box::new(evaluation),
                );
            }
        }
    }

    fn mark_module_evaluation_reaction_fulfilled_for_owner(
        &mut self,
        reaction_id: u64,
    ) -> Option<DocumentModuleReactionUpdate> {
        let parser_update = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .mark_parser_module_evaluation_fulfilled(
                reaction_id,
                parser_module_evaluation_continuation_into_ready_action,
            )
            .map(DocumentModuleReactionUpdate::ParserOwned);
        parser_update.or_else(|| {
            self.mark_runtime_owned_module_script_evaluation_fulfilled(reaction_id)
                .map(DocumentModuleReactionUpdate::RuntimeOwned)
        })
    }

    fn mark_module_evaluation_reaction_rejected_for_owner(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Option<DocumentModuleReactionUpdate> {
        let parser_update = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .mark_parser_module_evaluation_rejected(
                reaction_id,
                reason.clone(),
                error_constructor,
                parser_module_evaluation_continuation_into_ready_action,
            )
            .map(DocumentModuleReactionUpdate::ParserOwned);
        parser_update.or_else(|| {
            self.mark_runtime_owned_module_script_evaluation_rejected(
                reaction_id,
                reason,
                error_constructor,
            )
            .map(DocumentModuleReactionUpdate::RuntimeOwned)
        })
    }

    #[cfg(test)]
    pub(crate) fn has_pending_parser_owned_module_script(&self) -> bool {
        let Some(owner) = self.current_main_document_task_owner() else {
            return false;
        };
        self._context_host
            .borrow()
            .current_main_document_has_parser_deferred_script_load_delay(owner)
            .unwrap_or(false)
    }

    pub(crate) fn release_main_parser_deferred_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: crate::frame_owner_model::DocumentLoadDelayTokenId,
    ) -> bool {
        let released = self
            ._context_host
            .borrow_mut()
            .release_main_parser_deferred_script_load_delay(owner, token);
        tracing::debug!(
            ?owner,
            ?token,
            released,
            "settled main parser-deferred lifecycle ownership"
        );
        released
    }

    #[cfg(test)]
    pub(crate) fn has_pending_parser_owned_module_fetch(&self) -> bool {
        self.document_runtime
            .parser_module_scripts()
            .has_pending_fetch(self.document_runtime.parser_module_document_scripts())
            || self
                .document_runtime
                .has_native_module_script_fetch_waiters()
    }

    pub(crate) fn note_module_script_graph_waits_suspended_for_owner(
        &mut self,
        load_ids: Vec<u64>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        continuation: ModuleScriptContinuation,
    ) {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => self
                .document_runtime
                .parser_module_scripts_mut()
                .insert_parser_pending_waits(load_ids, joined_clients, continuation),
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = continuation
                    .dynamic_script_owner_id()
                    .expect("runtime-owned module graph fetch should carry dynamic owner id");
                self.note_runtime_owned_module_script_graph_fetch_suspended(
                    owner_id,
                    load_ids,
                    joined_clients,
                    Box::new(continuation),
                );
            }
        }
    }

    pub(crate) fn note_or_restore_module_script_graph_waits_for_owner(
        &mut self,
        load_ids: Vec<u64>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        continuation: ModuleScriptContinuation,
    ) {
        if load_ids.is_empty() && joined_clients.is_empty() {
            self.restore_module_script_graph_pending_continuation_for_owner(continuation);
            return;
        }
        self.note_module_script_graph_waits_suspended_for_owner(
            load_ids,
            joined_clients,
            continuation,
        );
    }

    pub(crate) fn suspend_and_schedule_module_script_graph_fetches_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
        job: NativeModuleGraphJob,
        fetches: Vec<ModuleScriptGraphFetchContinuation>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        trace_message: &'static str,
    ) {
        #[derive(Clone, Copy)]
        enum FetchScheduleOwner {
            Parser(
                crate::document_script_scheduler::ParserPendingScriptId<
                    crate::module_script_continuation::MainParserDocumentOwner,
                >,
            ),
            Runtime {
                document_owner: FrameDocumentTaskOwner,
                dynamic_script_owner_id: crate::dynamic_script_owner::DynamicScriptOwnerId,
            },
        }

        let completion_owner = continuation.completion_owner();
        let fetch_schedule_owner = match completion_owner {
            ModuleScriptCompletionOwner::Parser => FetchScheduleOwner::Parser(
                continuation
                    .parser_pending_script_id()
                    .expect("parser module continuation must retain its PendingScript owner"),
            ),
            ModuleScriptCompletionOwner::Runtime => FetchScheduleOwner::Runtime {
                document_owner: self.current_main_document_task_owner().expect(
                    "runtime-created module graph fetch must start with a current main Document owner",
                ),
                dynamic_script_owner_id: continuation.dynamic_script_owner_id().expect(
                    "runtime-created module continuation must retain its dynamic script owner",
                ),
            },
        };
        let mut load_ids = Vec::with_capacity(fetches.len());
        let mut scheduled = Vec::with_capacity(fetches.len());
        for fetch in fetches {
            let request = fetch.request().clone();
            let load_id = self
                .document_runtime
                .suspend_native_module_script_fetch(fetch);
            load_ids.push(load_id);
            scheduled.push((load_id, request));
        }
        let continuation = continuation.with_pending_graph_fetches(job, load_ids.first().copied());
        tracing::debug!(
            url = %continuation.script.url,
            completion_owner = ?continuation.completion_owner(),
            dynamic_script_owner_id = ?continuation.dynamic_script_owner_id(),
            fetch_count = load_ids.len(),
            joined_fetch_count = joined_clients.len(),
            trace_message,
            "module script graph fetch waits installed"
        );
        self.note_or_restore_module_script_graph_waits_for_owner(
            load_ids,
            joined_clients,
            continuation,
        );
        let document_loader = self
            .document_runtime
            .current_document_resource_loader()
            .expect("main module graph requires the committed Document resource authority");
        for (load_id, request) in scheduled {
            match fetch_schedule_owner {
                FetchScheduleOwner::Parser(pending_script_id) => {
                    self.resource_scheduler()
                        .schedule_main_parser_module_graph_fetch(
                            document_loader.clone(),
                            crate::page_resource_completion::MainParserModuleGraphFetchTarget::new(
                                pending_script_id,
                                load_id,
                            ),
                            request,
                            self.document_runtime.document_url().clone(),
                        );
                }
                FetchScheduleOwner::Runtime {
                    document_owner,
                    dynamic_script_owner_id,
                } => {
                    self.resource_scheduler()
                        .schedule_main_runtime_module_graph_fetch(
                            document_loader.clone(),
                            crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget::new(
                                document_owner,
                                dynamic_script_owner_id,
                                load_id,
                            ),
                            request,
                            self.document_runtime.document_url().clone(),
                        );
                }
            }
        }
    }

    pub(crate) fn handle_module_script_graph_advance_for_owner(
        &mut self,
        advance: ModuleScriptContinuationGraphAdvance,
    ) -> NativeModuleOwnerActions {
        match advance {
            ModuleScriptContinuationGraphAdvance::Ready(script_continuation) => {
                if script_continuation.completed_graph.is_some() {
                    tracing::debug!(
                        url = %script_continuation.script.url,
                        completion_owner = ?script_continuation.completion_owner(),
                        active_fetch_load_id = ?script_continuation.active_fetch_load_id(),
                        "module script graph completed after fetch"
                    );
                }
                self.note_module_script_graph_ready_for_owner(*script_continuation);
                NativeModuleOwnerActions::empty()
            }
            ModuleScriptContinuationGraphAdvance::NeedFetches {
                continuation,
                mut job,
                fetches,
            } => {
                let joined_clients = job.take_pending_joined_clients();
                self.suspend_and_schedule_module_script_graph_fetches_for_owner(
                    *continuation,
                    *job,
                    fetches,
                    joined_clients,
                    "module script graph requested parallel fetches",
                );
                NativeModuleOwnerActions::empty()
            }
            ModuleScriptContinuationGraphAdvance::Failed {
                continuation,
                error,
            } => {
                self.clear_pending_module_script_fetches_for_script(
                    continuation.script.node_id,
                    &error,
                );
                self.notify_module_script_graph_failure_for_owner(*continuation, error)
                    .map(|(continuation, error)| {
                        NativeModuleOwnerActions::from_runtime_module_failure(continuation, error)
                    })
                    .unwrap_or_else(NativeModuleOwnerActions::empty)
            }
        }
    }

    fn handle_runtime_module_script_graph_advance_for_dynamic_owner(
        &mut self,
        advance: ModuleScriptContinuationGraphAdvance,
    ) -> NativeModuleOwnerActions {
        match advance {
            ModuleScriptContinuationGraphAdvance::Ready(script_continuation) => {
                NativeModuleOwnerActions::from_ready_module_script(*script_continuation)
            }
            ModuleScriptContinuationGraphAdvance::NeedFetches {
                continuation,
                mut job,
                fetches,
            } => {
                let joined_clients = job.take_pending_joined_clients();
                self.suspend_and_schedule_module_script_graph_fetches_for_owner(
                    *continuation,
                    *job,
                    fetches,
                    joined_clients,
                    "runtime module script graph requested parallel fetches",
                );
                NativeModuleOwnerActions::empty()
            }
            ModuleScriptContinuationGraphAdvance::Failed {
                continuation,
                error,
            } => {
                self.clear_runtime_owned_module_script_graph_waits_for_owner(&continuation, &error);
                NativeModuleOwnerActions::from_runtime_module_failure(*continuation, error)
            }
        }
    }

    pub(crate) fn restore_module_script_graph_pending_continuation_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
    ) {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => self
                .document_runtime
                .parser_module_scripts_mut()
                .restore_pending_continuation(continuation),
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = continuation.dynamic_script_owner_id().expect(
                    "runtime-owned module graph continuation should carry dynamic owner id",
                );
                let restored = self.restore_runtime_owned_module_script_graph_pending_continuation(
                    owner_id,
                    Box::new(continuation),
                );
                debug_assert!(
                    restored,
                    "runtime-owned module graph pending continuation should restore into owner state"
                );
            }
        }
    }

    pub(crate) fn note_module_script_graph_ready_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
    ) {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => {
                self.queue_main_module_script_graph_ready_work(continuation);
            }
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = continuation
                    .dynamic_script_owner_id()
                    .expect("runtime-owned module continuation should carry dynamic owner id");
                let ready = self
                    .note_runtime_owned_module_script_graph_ready(owner_id, Box::new(continuation));
                debug_assert!(
                    ready,
                    "dynamic owner should accept runtime-owned ready graph continuation"
                );
            }
        }
    }

    pub(crate) fn notify_module_script_graph_failure_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
        error: ModuleLoadError,
    ) -> Option<(ModuleScriptContinuation, ModuleLoadError)> {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => {
                self.queue_main_parser_module_graph_failure_work(ParserModuleScriptFailure {
                    continuation,
                    error,
                });
                None
            }
            ModuleScriptCompletionOwner::Runtime => Some((continuation, error)),
        }
    }

    fn complete_parser_owned_module_script_graph_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<Option<ModuleScriptGraphFetchResume>> {
        let Some(active_tree) = self
            .document_runtime
            .parser_module_scripts_mut()
            .take_active_tree_for_fetch_completion(load_id)
        else {
            return Ok(None);
        };
        let Some(graph_continuation) = self
            .document_runtime
            .take_inflight_native_module_script_fetch(load_id)
        else {
            self.record_runtime_warning(format_args!(
                "module script continuation {load_id} had no graph fetch continuation"
            ));
            self.document_runtime
                .parser_module_scripts_mut()
                .restore_active_tree_for_fetch(load_id, active_tree);
            return Ok(Some(
                ModuleScriptGraphFetchResume::RestoredMissingGraphContinuation,
            ));
        };
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                graph_continuation.request().fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("native module script fetch completion {load_id} failed: {error}"),
            )),
        };
        let result = active_tree.finish_fetch_into_graph(self, graph_continuation, source);
        Ok(Some(ModuleScriptGraphFetchResume::finished(result)))
    }

    pub(crate) fn apply_current_main_parser_module_graph_fetch_completion(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentMainParserModuleGraphFetchCompletion,
    ) -> Result<()> {
        let completion = authorization.into_completion();
        let target = completion.target();
        assert!(
            self.main_parser_module_graph_fetch_target_is_current(target),
            "authorized main parser module terminal must retain its exact Document fetch"
        );
        let result = completion.into_result();
        if self.current_main_parser_module_graph_fetch_target(target.load_id()) != Some(target) {
            return self.complete_shared_module_map_fetch_result(target.load_id(), result);
        }
        let resume = self
            .complete_parser_owned_module_script_graph_fetch_result(target.load_id(), result)?
            .expect("active main parser module terminal must retain its PendingScript fetch");
        let actions = self.handle_module_script_graph_fetch_resume_for_owner(resume)?;
        let (ready_scripts, ready_evaluations, runtime_failures) = actions.into_parts();
        assert!(
            ready_scripts.is_empty() && ready_evaluations.is_empty() && runtime_failures.is_empty(),
            "parser-owned module fetch application must publish follow-up work through the parser owner, not inline runtime actions"
        );
        Ok(())
    }

    fn complete_runtime_owned_module_script_graph_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<Option<ModuleScriptGraphFetchResume>> {
        let Some(script_continuation) =
            self.take_runtime_owned_module_script_graph_pending_fetch(load_id)
        else {
            return Ok(None);
        };
        let Some(graph_continuation) = self
            .document_runtime
            .take_inflight_native_module_script_fetch(load_id)
        else {
            self.record_runtime_warning(format_args!(
                "module script continuation {load_id} had no graph fetch continuation"
            ));
            self.note_module_script_graph_waits_suspended_for_owner(
                vec![load_id],
                Vec::new(),
                script_continuation,
            );
            return Ok(Some(
                ModuleScriptGraphFetchResume::RestoredMissingGraphContinuation,
            ));
        };

        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                graph_continuation.request().fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("native module script fetch completion {load_id} failed: {error}"),
            )),
        };
        let result =
            script_continuation.finish_fetch_into_resumed_graph(self, graph_continuation, source);
        Ok(Some(ModuleScriptGraphFetchResume::finished(result)))
    }

    pub(crate) fn apply_current_main_runtime_module_graph_fetch_completion(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion,
    ) -> Result<NativeModuleOwnerActions> {
        let completion = authorization.into_completion();
        let target = completion.target();
        assert!(
            self.main_runtime_module_graph_fetch_target_is_current(target),
            "authorized main runtime module terminal must retain its exact Document fetch"
        );
        let result = completion.into_result();
        if self.current_main_runtime_module_graph_fetch_target(target.load_id()) != Some(target) {
            self.complete_shared_module_map_fetch_result(target.load_id(), result)?;
            return Ok(NativeModuleOwnerActions::empty());
        }
        let resume = self
            .complete_runtime_owned_module_script_graph_fetch_result(target.load_id(), result)?
            .expect("active main runtime module terminal must retain its dynamic-script fetch");
        self.handle_module_script_graph_fetch_resume_for_owner(resume)
    }

    #[cfg(test)]
    fn complete_current_main_dynamic_import_graph_fetch_result(
        &mut self,
        target: crate::page_resource_completion::MainDynamicImportGraphFetchTarget,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<NativeModuleOwnerActions> {
        self.complete_current_main_dynamic_import_graph_fetch_result_with_body(
            target,
            result,
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
    }

    fn complete_current_main_dynamic_import_graph_fetch_result_with_body<Body>(
        &mut self,
        target: crate::page_resource_completion::MainDynamicImportGraphFetchTarget,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
        body: &mut Body,
    ) -> Result<NativeModuleOwnerActions>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let inflight = self
            .document_runtime
            .take_inflight_native_dynamic_module_import_fetch(target.load_id())
            .expect("authorized main dynamic-import terminal must retain its resolver fetch claim");
        assert_eq!(
            inflight.owner(),
            target.import_owner(),
            "dynamic-import resolver claim must match the authorized exact import owner"
        );
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                target.load_id(),
                fetched_source,
                inflight.fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!(
                    "native dynamic import fetch completion {} failed: {error}",
                    target.load_id()
                ),
            )),
        };
        let finish = self.finish_native_dynamic_module_inflight_fetch(inflight, source);
        let mut terminal_followup =
            self.dynamic_module_fetch_finish_to_owner_actions_with_body(finish, body)?;
        let mut owner_actions = NativeModuleOwnerActions::empty();
        while self.has_ready_native_module_owner_actions() {
            let (actions, followup) =
                self.drain_ready_native_module_owner_actions_with_body(body)?;
            owner_actions.merge(actions);
            terminal_followup.merge(followup);
        }
        // `terminal_followup` describes already-published child/module-owner
        // work. The stable sources own that work; this exact Page terminal only
        // returns main runtime actions to its caller.
        let _ = terminal_followup;
        Ok(owner_actions)
    }

    pub(crate) fn resume_parser_owned_module_script_joined_fetch(
        &mut self,
        key: &ModuleMapKey,
        client: NativeModuleScriptSingleModuleClient,
    ) -> Option<ModuleScriptGraphResumeResult> {
        let client_token = client.token();
        let active_tree = self
            .document_runtime
            .parser_module_scripts_mut()
            .take_active_tree_for_joined_client(client_token)?;
        Some(active_tree.finish_joined_fetch_into_graph(
            self,
            chromium_module_key(key),
            client_token,
        ))
    }

    pub(crate) fn resume_runtime_owned_module_script_joined_fetch(
        &mut self,
        key: &ModuleMapKey,
        client: NativeModuleScriptSingleModuleClient,
    ) -> Option<ModuleScriptGraphResumeResult> {
        let client_token = client.token();
        let script_continuation =
            self.take_runtime_owned_module_script_graph_pending_joined_client(client_token)?;
        let result = script_continuation.finish_joined_fetch_into_resumed_graph(
            self,
            chromium_module_key(key),
            client_token,
        );
        Some(result)
    }

    fn resume_module_script_fetch_join_waiter(
        &mut self,
        key: &ModuleMapKey,
        client: NativeModuleScriptSingleModuleClient,
    ) -> Option<ModuleScriptGraphResumeResult> {
        let client_token = client.token();
        if let Some(result) = self.resume_parser_owned_module_script_joined_fetch(key, client) {
            return Some(result);
        }
        if let Some(result) = self.resume_runtime_owned_module_script_joined_fetch(key, client) {
            return Some(result);
        }
        self.record_runtime_warning(format_args!(
            "module script joined fetch client {:?} had no graph continuation",
            client_token
        ));
        None
    }

    pub(crate) fn compile_native_module_record(
        &mut self,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.compile_native_module_record_in_context(
            context_ptr,
            key,
            source,
            source_url,
            fetch_metadata,
        )
    }

    pub(crate) fn compile_native_module_record_for_frame_realm(
        &mut self,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        let context_ptr = self.frame_realm_context_ptr(realm_id).map_err(|error| {
            ModuleLoadError::new(
                ModuleLoadStage::Compile,
                format!("failed to find FrameRealm {realm_id:?} for module compile: {error}"),
            )
        })?;
        self.compile_native_module_record_in_context(
            context_ptr,
            key,
            source,
            source_url,
            fetch_metadata,
        )
    }

    fn native_module_default_context_ptr(&self) -> *const v8::Global<v8::Context> {
        &self.page_default_context as *const _
    }

    fn compile_native_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        match key.kind() {
            ModuleKind::JavaScript => {
                let Some(source) = source.text_source() else {
                    return Err(ModuleLoadError::new(
                        ModuleLoadStage::Compile,
                        format!("javascript module `{source_url}` did not retain text source"),
                    ));
                };
                self.compile_javascript_module_record_in_context(
                    context_ptr,
                    key,
                    source,
                    source_url,
                    fetch_metadata,
                )
            }
            ModuleKind::Json | ModuleKind::Css => {
                let Some(source) = source.text_source() else {
                    return Err(ModuleLoadError::new(
                        ModuleLoadStage::Compile,
                        format!("synthetic text module `{source_url}` did not retain text source"),
                    ));
                };
                self.compile_synthetic_module_record_in_context(
                    context_ptr,
                    key,
                    source,
                    source_url,
                )
            }
            ModuleKind::WebAssembly => {
                let Some(bytes) = source.binary_source() else {
                    return Err(ModuleLoadError::new(
                        ModuleLoadStage::Compile,
                        format!("WebAssembly module `{source_url}` did not retain binary source"),
                    ));
                };
                self.compile_wasm_module_record_in_context(context_ptr, key, bytes, source_url)
            }
            ModuleKind::ModulePreloadText => Err(ModuleLoadError::new(
                ModuleLoadStage::Compile,
                format!("modulepreload text `{source_url}` is not a module graph record"),
            )),
        }
    }

    fn compile_javascript_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        source: &str,
        source_url: &Url,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();

                let source_string = v8_string(&scope, source)
                    .ok_or_else(|| anyhow::anyhow!("failed to allocate v8 module source string"))?;
                let origin = create_module_script_origin(
                    &mut scope,
                    source_url.as_str(),
                    fetch_metadata,
                );
                let mut compiler_source =
                    v8::script_compiler::Source::new(source_string, Some(&origin));
                let module = v8::script_compiler::compile_module(&scope, &mut compiler_source)
                    .ok_or_else(|| {
                        let exception = scope
                            .exception()
                            .and_then(|exception| exception.to_detail_string(&scope))
                            .map(|message| message.to_rust_string_lossy(&scope))
                            .or_else(|| {
                                scope.message().map(|message| {
                                    message.get(&scope).to_rust_string_lossy(&scope)
                                })
                            })
                            .unwrap_or_else(|| {
                                format!(
                                    "unknown compile exception (caught={}, can_continue={}, terminated={})",
                                    scope.has_caught(),
                                    scope.can_continue(),
                                    scope.has_terminated()
                                )
                            });
                        anyhow::anyhow!(
                            "v8 failed to compile native module `{source_url}`: {}",
                            exception
                        )
                    })?;
                let requests = collect_module_requests(&mut scope, module)?;
                let identity = module_identity_hash_from_v8_module(module);
                let compiled_module = v8::Global::new(scope.as_ref(), module);
                Ok((
                    ModuleRecordEntry::new(key, compiled_module, requests),
                    identity,
                ))
            })
            .map_err(|error| {
                let message = error.to_string();
                let load_error = ModuleLoadError::new(ModuleLoadStage::Compile, message.clone());
                if message.starts_with("v8 failed to compile WebAssembly module `") {
                    load_error
                        .with_error_constructor(ScriptErrorConstructorKind::WebAssemblyCompileError)
                } else {
                    load_error.with_error_constructor(ScriptErrorConstructorKind::SyntaxError)
                }
            })
    }

    fn compile_synthetic_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        _source: &str,
        source_url: &Url,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        self.compile_synthetic_module_record_with_exports_in_context(
            context_ptr,
            key,
            source_url,
            &["default"],
            None,
        )
    }

    fn compile_wasm_module_record_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        bytes: &[u8],
        source_url: &Url,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let scope = try_catch.init();
                let prepared = prepare_wasm_module_record(&scope, bytes)?.ok_or_else(|| {
                    let exception = v8_exception_message_or(
                        &scope,
                        scope.exception(),
                        "unknown wasm compile exception",
                    );
                    anyhow::anyhow!(
                        "v8 failed to compile WebAssembly module `{source_url}`: {exception}"
                    )
                })?;
                let requests = if prepared.has_reserved_name_link_error {
                    Vec::new()
                } else {
                    wasm_module_requests_for_imports(prepared.record.imports())
                };
                let export_name_refs = prepared
                    .record
                    .exports()
                    .iter()
                    .map(|export| export.name())
                    .collect::<Vec<_>>();
                let module_name = v8_string(&scope, source_url.as_str()).ok_or_else(|| {
                    anyhow::anyhow!("failed to allocate WebAssembly synthetic module name")
                })?;
                let export_names = export_name_refs
                    .iter()
                    .map(|name| {
                        v8_string(&scope, name)
                            .ok_or_else(|| anyhow::anyhow!("failed to allocate wasm export name"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let module = v8::Module::create_synthetic_module(
                    &scope,
                    module_name,
                    &export_names,
                    synthetic_module_evaluation_steps,
                );
                let identity = module_identity_hash_from_v8_module(module);
                let compiled_module = v8::Global::new(scope.as_ref(), module);
                Ok((
                    ModuleRecordEntry::new_with_wasm_module(
                        key,
                        compiled_module,
                        requests,
                        prepared.record,
                    ),
                    identity,
                ))
            })
            .map_err(|error| {
                ModuleLoadError::new(ModuleLoadStage::Compile, error.to_string())
                    .with_error_constructor(ScriptErrorConstructorKind::WebAssemblyCompileError)
            })
    }

    fn compile_synthetic_module_record_with_exports_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        key: ModuleMapKey,
        source_url: &Url,
        export_names: &[&str],
        wasm_record: Option<(Vec<ModuleRequestRecord>, WasmModuleRecord)>,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let scope = try_catch.init();

                let module_name = v8_string(&scope, source_url.as_str()).ok_or_else(|| {
                    anyhow::anyhow!("failed to allocate v8 synthetic module name")
                })?;
                let export_names = export_names
                    .iter()
                    .map(|name| {
                        v8_string(&scope, name).ok_or_else(|| {
                            anyhow::anyhow!("failed to allocate synthetic export name")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let module = v8::Module::create_synthetic_module(
                    &scope,
                    module_name,
                    &export_names,
                    synthetic_module_evaluation_steps,
                );
                let identity = module_identity_hash_from_v8_module(module);
                let compiled_module = v8::Global::new(scope.as_ref(), module);
                let entry = match wasm_record {
                    Some((requests, wasm_module)) => ModuleRecordEntry::new_with_wasm_module(
                        key,
                        compiled_module,
                        requests,
                        wasm_module,
                    ),
                    None => ModuleRecordEntry::new(key, compiled_module, Vec::new()),
                };
                Ok((entry, identity))
            })
            .map_err(|error| ModuleLoadError::new(ModuleLoadStage::Compile, error.to_string()))
    }

    pub(crate) fn instantiate_native_module_graph(
        &mut self,
        graph: &crate::module_runtime::ModuleGraphHandle,
    ) -> std::result::Result<(), ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.instantiate_native_module_graph_in_context(context_ptr, graph)
    }

    fn instantiate_native_module_graph_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        graph: &crate::module_runtime::ModuleGraphHandle,
    ) -> std::result::Result<(), ModuleLoadError> {
        let root_entry = graph.root_entry;
        let graph_urls = graph
            .entries
            .iter()
            .map(|entry_id| self.document_runtime.native_module_entry_url(*entry_id))
            .collect::<Vec<_>>();
        let has_wasm_entry = graph.entries.iter().any(|entry_id| {
            self.document_runtime
                .native_module_wasm_record(*entry_id)
                .is_some()
        });
        let document_modulator = self.document_runtime.native_document_modulator_ptr();
        let root_module = self
            .document_runtime
            .native_compiled_module(root_entry)
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Instantiate,
                    format!("native root module entry {root_entry:?} is not compiled"),
                )
            })?;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let scope = try_catch.init();

                let root_module = v8::Local::new(&scope, &root_module);
                let _resolver_scope = ResolverScopeGuard::new(document_modulator);
                match root_module.instantiate_module2(
                    &scope,
                    resolve_static_module_callback,
                    resolve_static_source_callback,
                ) {
                    Some(true) => Ok(()),
                    Some(false) => Err(anyhow::anyhow!("v8 reported module instantiate failure")),
                    None => {
                        let exception = scope
                            .exception()
                            .and_then(|exception| exception.to_string(&scope))
                            .map(|message| message.to_rust_string_lossy(&scope))
                            .unwrap_or_else(|| "unknown instantiate exception".to_owned());
                        Err(anyhow::anyhow!(
                            "{}",
                            canonical_native_module_instantiate_error(&exception, &graph_urls)
                        ))
                    }
                }
            })
            .map_err(|error| {
                let message = error.to_string();
                let load_error =
                    ModuleLoadError::new(ModuleLoadStage::Instantiate, message.clone());
                if message.contains("does not provide an export named")
                    || message.contains("does not export")
                {
                    load_error.with_error_constructor(ScriptErrorConstructorKind::SyntaxError)
                } else if has_wasm_entry {
                    load_error
                        .with_error_constructor(ScriptErrorConstructorKind::WebAssemblyLinkError)
                } else {
                    load_error
                }
            })?;
        self.document_runtime
            .mark_native_module_instantiated(root_entry);
        Ok(())
    }

    pub(crate) fn evaluate_native_module_graph(
        &mut self,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<Option<v8::Global<v8::Promise>>, ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.evaluate_native_module_graph_with_owner_in_context(
            context_ptr,
            root_entry,
            NativeModuleEvaluationOwner::Script,
        )
        .map(|result| result.promise)
    }

    pub(crate) fn evaluate_native_dynamic_module_graph(
        &mut self,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleEvaluation, ModuleLoadError> {
        let context_ptr = self.native_module_default_context_ptr();
        self.evaluate_native_module_graph_with_owner_in_context(
            context_ptr,
            root_entry,
            NativeModuleEvaluationOwner::DynamicImport,
        )
        .map(|result| NativeDynamicModuleEvaluation {
            target: DynamicModuleEvaluationTarget::new(root_entry, result.module),
            promise: result.promise,
        })
    }

    fn start_native_dynamic_module_import_evaluation(
        &mut self,
        graph: ModuleGraphHandle,
    ) -> std::result::Result<DynamicModuleImportEvaluationStart, ModuleLoadError> {
        match self
            .document_runtime
            .native_module_entry_state(graph.root_entry)
        {
            ModuleMapEntryState::Compiled => {
                self.instantiate_native_module_graph(&graph)?;
                self.start_native_module_graph_evaluation(graph.root_entry)
            }
            ModuleMapEntryState::Instantiated | ModuleMapEntryState::Evaluating => {
                self.start_native_module_graph_evaluation(graph.root_entry)
            }
            ModuleMapEntryState::Evaluated => {
                let module = self
                    .document_runtime
                    .native_compiled_module(graph.root_entry)
                    .ok_or_else(|| {
                        ModuleLoadError::new(
                            ModuleLoadStage::Evaluate,
                            "native dynamic import root was evaluated without a compiled module",
                        )
                    })?;
                Ok(DynamicModuleImportEvaluationStart::Completed(
                    DynamicModuleEvaluationTarget::new(graph.root_entry, module),
                ))
            }
            ModuleMapEntryState::Fetching
            | ModuleMapEntryState::Fetched
            | ModuleMapEntryState::Failed => Err(ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                "native dynamic import root was not ready to evaluate",
            )),
        }
    }

    fn start_native_module_graph_evaluation(
        &mut self,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<DynamicModuleImportEvaluationStart, ModuleLoadError> {
        let evaluation = self.evaluate_native_dynamic_module_graph(root_entry)?;
        let (target, promise) = evaluation.into_parts();
        let Some(promise) = promise else {
            return Ok(DynamicModuleImportEvaluationStart::Completed(target));
        };
        Ok(DynamicModuleImportEvaluationStart::Pending { target, promise })
    }

    fn evaluate_native_module_graph_with_owner_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        root_entry: crate::module_runtime::ModuleEntryId,
        owner: NativeModuleEvaluationOwner,
    ) -> std::result::Result<NativeModuleEvaluationResult, ModuleLoadError> {
        let root_module = self
            .document_runtime
            .native_compiled_module(root_entry)
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    format!("native root module entry {root_entry:?} is not compiled"),
                )
            })?;
        let document_owner_before_evaluation =
            self.current_main_document_task_owner().ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    "native module evaluation has no current main Document owner",
                )
            })?;
        self.document_runtime
            .mark_native_module_evaluating(root_entry);
        let promise = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();

                let root_module = v8::Local::new(&scope, &root_module);
                let Some(value) = root_module.evaluate(&scope) else {
                    let error = scope
                        .exception()
                        .map(|exception| {
                            native_module_evaluation_exception_error(
                                &mut scope,
                                exception,
                                "v8 failed to evaluate native module graph",
                            )
                        })
                        .unwrap_or_else(|| {
                            ModuleLoadError::new(
                                ModuleLoadStage::Evaluate,
                                "v8 failed to evaluate native module graph: unknown exception",
                            )
                        });
                    return Ok(Err(error));
                };
                let promise = v8::Local::<v8::Promise>::try_from(value).ok();
                if owner == NativeModuleEvaluationOwner::DynamicImport
                    && let Some(promise) = promise
                {
                    // Dynamic import owns the module-evaluation promise itself.
                    // Return it before a checkpoint so the import rejection
                    // handler is attached before V8 can report an unhandled
                    // rejection for synchronously rejected module evaluation.
                    match promise.state() {
                        v8::PromiseState::Fulfilled => return Ok(Ok(None)),
                        v8::PromiseState::Rejected | v8::PromiseState::Pending => {
                            let promise = v8::Global::new(scope.as_ref(), promise);
                            return Ok(Ok(Some(promise)));
                        }
                    }
                }
                if let Err(error) = Self::perform_microtask_checkpoints(&mut scope, None) {
                    return Ok(Err(ModuleLoadError::new(
                        ModuleLoadStage::Evaluate,
                        error.to_string(),
                    )));
                }
                if root_module.get_status() == v8::ModuleStatus::Errored {
                    let exception = root_module.get_exception();
                    return Ok(Err(native_module_evaluation_exception_error(
                        &mut scope,
                        exception,
                        "native module graph evaluation rejected",
                    )));
                }
                if let Some(promise) = promise {
                    match promise.state() {
                        v8::PromiseState::Fulfilled => return Ok(Ok(None)),
                        v8::PromiseState::Rejected => {
                            let result = promise.result(&scope);
                            return Ok(Err(native_module_evaluation_exception_error(
                                &mut scope,
                                result,
                                "native module graph evaluation rejected",
                            )));
                        }
                        v8::PromiseState::Pending => {
                            let promise = v8::Global::new(scope.as_ref(), promise);
                            return Ok(Ok(Some(promise)));
                        }
                    }
                }
                Ok(Ok(None))
            })
            .map_err(|error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })??;
        if promise.is_none()
            && self.current_main_document_task_owner() == Some(document_owner_before_evaluation)
        {
            self.document_runtime
                .mark_native_module_evaluated(root_entry);
        }
        Ok(NativeModuleEvaluationResult {
            module: root_module,
            promise,
        })
    }

    pub(crate) fn attach_native_dynamic_module_import_reactions(
        &mut self,
        request: PendingDynamicModuleImport,
        target: DynamicModuleEvaluationTarget,
        promise: v8::Global<v8::Promise>,
    ) -> std::result::Result<(), ModuleLoadError> {
        let context = request.context().clone();
        let document_owner = request.owner();
        let reaction_id = self
            .document_runtime
            .reserve_native_dynamic_module_evaluation_reaction(request, target);
        let attach_result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let reaction_id_value = v8::BigInt::new_from_u64(scope, reaction_id);
                let data = NativeDynamicModuleReactionDataDeclaration {
                    reaction_id: reaction_id_value,
                }
                .bind(scope)
                .map_err(|error| {
                    anyhow::anyhow!("failed to create native dynamic import reaction data: {error}")
                })?;
                let on_fulfilled =
                    v8::Function::builder(native_dynamic_module_reaction_fulfilled_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native dynamic import success reaction"
                            )
                        })?;
                let on_rejected =
                    v8::Function::builder(native_dynamic_module_reaction_rejected_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native dynamic import failure reaction"
                            )
                        })?;
                let promise = v8::Local::new(scope, &promise);
                promise
                    .then2(scope, on_fulfilled, on_rejected)
                    .map(|_| ())
                    .ok_or_else(|| {
                        anyhow::anyhow!("failed to attach native dynamic import reactions")
                    })?;
                // `then` does not invoke the dynamic-import reaction inline. It
                // queues a V8 promise-reaction microtask, even when the module
                // evaluation promise is already settled. Run the browser-style
                // checkpoint for this owner-lane task so the fulfilled/rejected
                // callback can transfer the dynamic import result into
                // DocumentRuntime before the driver decides whether more module
                // work is ready.
                Self::perform_microtask_checkpoints(scope, None)
            });
        if let Err(error) = attach_result {
            if let Some(reaction) = self
                .document_runtime
                .take_native_dynamic_module_evaluation_reaction(reaction_id, document_owner)
            {
                let (request, _) = reaction.into_parts();
                let _ = self.reject_native_dynamic_module_import(request, &error.to_string());
            }
            return Err(ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                error.to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) fn attach_native_module_script_evaluation_reactions(
        &mut self,
        reaction_id: u64,
        promise: v8::Global<v8::Promise>,
    ) -> std::result::Result<(), ModuleLoadError> {
        let document_owner = self.current_main_document_task_owner().ok_or_else(|| {
            ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                "main module evaluation reaction has no current Document owner",
            )
        })?;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let reaction_id_value = v8::BigInt::new_from_u64(scope, reaction_id);
                let data = NativeModuleScriptReactionDataDeclaration {
                    module_script_reaction_id: reaction_id_value,
                    scheduler_lane_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.scheduler_lane_id.0,
                    ),
                    local_window_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.local_window_id.0,
                    ),
                    document_id: v8::BigInt::new_from_u64(scope, document_owner.document_id.0),
                }
                .bind(scope)
                .map_err(|error| {
                    anyhow::anyhow!("failed to create native module script reaction data: {error}")
                })?;
                let on_fulfilled =
                    v8::Function::builder(native_module_script_reaction_fulfilled_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native module script success reaction"
                            )
                        })?;
                let on_rejected =
                    v8::Function::builder(native_module_script_reaction_rejected_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "failed to create native module script failure reaction"
                            )
                        })?;
                let promise = v8::Local::new(scope, &promise);
                promise
                    .then2(scope, on_fulfilled, on_rejected)
                    .map(|_| ())
                    .ok_or_else(|| {
                        anyhow::anyhow!("failed to attach native module script reactions")
                    })?;
                // `then` schedules the module-script evaluation reaction as a
                // V8 microtask, even if the evaluation promise is already
                // settled. Chromium runs module evaluation's error handling
                // before the script runner advances to later pending scripts;
                // drain this owner-lane checkpoint now so TLA rejection cannot
                // be observed after a later dynamic module.
                Self::perform_microtask_checkpoints(scope, None)
            })
            .map_err(|error| ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string()))
    }

    pub(crate) fn attach_child_parser_module_script_evaluation_reactions(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        reaction_id: u64,
        promise: v8::Global<v8::Promise>,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let reaction_id_value = v8::BigInt::new_from_u64(scope, reaction_id);
                let data = NativeChildModuleScriptReactionDataDeclaration {
                    module_script_reaction_id: reaction_id_value,
                    scheduler_lane_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.scheduler_lane_id.0,
                    ),
                    local_window_id: v8::BigInt::new_from_u64(
                        scope,
                        document_owner.local_window_id.0,
                    ),
                    document_id: v8::BigInt::new_from_u64(scope, document_owner.document_id.0),
                    realm_id: v8::BigInt::new_from_i64(scope, realm_id.0),
                }
                .bind(scope)
                .map_err(|error| {
                    anyhow::anyhow!("failed to create child parser module reaction data: {error}")
                })?;
                let on_fulfilled =
                    v8::Function::builder(child_parser_module_reaction_fulfilled_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!("failed to create child parser module success reaction")
                        })?;
                let on_rejected =
                    v8::Function::builder(child_parser_module_reaction_rejected_callback)
                        .data(data.into())
                        .build(scope)
                        .ok_or_else(|| {
                            anyhow::anyhow!("failed to create child parser module failure reaction")
                        })?;
                let promise = v8::Local::new(scope, &promise);
                promise
                    .then2(scope, on_fulfilled, on_rejected)
                    .map(|_| ())
                    .ok_or_else(|| {
                        anyhow::anyhow!("failed to attach child parser module script reactions")
                    })?;
                // Evaluation has just returned a genuinely pending promise
                // after its algorithm-required checkpoint. Attaching the TLA
                // observers is setup, not a second checkpoint boundary. The
                // selected DocumentScriptReady task performs its ordinary
                // task-end checkpoint after the script element load event.
                Ok(())
            })
            .map_err(|error| ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string()))
    }

    #[cfg(test)]
    pub(crate) fn resolve_native_dynamic_module_import(
        &mut self,
        request: PendingDynamicModuleImport,
        target: &DynamicModuleEvaluationTarget,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, request.context());
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::Local::new(scope, request.resolver());
                let root_module = v8::Local::new(scope, target.module());
                let namespace = root_module.get_module_namespace();
                let _ = resolver.resolve(scope, namespace);
                Self::perform_microtask_checkpoints(scope, None)?;
                Ok(())
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn resolve_native_dynamic_module_source_import(
        &mut self,
        request: PendingDynamicModuleImport,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleSourceImportResolution, ModuleLoadError> {
        let Some(wasm_record) = self.document_runtime.native_module_wasm_record(root_entry) else {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Resolve,
                format!(
                    "source-phase dynamic import `{}` is not a WebAssembly module",
                    request.specifier()
                ),
            )
            .with_error_constructor(ScriptErrorConstructorKind::SyntaxError);
            self.reject_native_dynamic_module_import_with_error(request, &error)?;
            return Ok(NativeDynamicModuleSourceImportResolution::Rejected);
        };
        self.resolve_native_dynamic_module_source_import_with_wasm_record(request, wasm_record)
    }

    #[cfg(test)]
    fn resolve_native_dynamic_module_source_import_with_wasm_record(
        &mut self,
        request: PendingDynamicModuleImport,
        wasm_record: WasmModuleRecord,
    ) -> std::result::Result<NativeDynamicModuleSourceImportResolution, ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, request.context());
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::Local::new(scope, request.resolver());
                let Some(source) = wasm_record.source_module(scope) else {
                    let exception = v8_string(scope, "failed to materialize WebAssembly source")
                        .map(|message| v8::Exception::type_error(scope, message))
                        .unwrap_or_else(|| v8::undefined(scope).into());
                    let _ = resolver.reject(scope, exception);
                    Self::perform_microtask_checkpoints(scope, None)?;
                    return Ok(NativeDynamicModuleSourceImportResolution::Rejected);
                };
                let _ = resolver.resolve(scope, source.into());
                Self::perform_microtask_checkpoints(scope, None)?;
                Ok(NativeDynamicModuleSourceImportResolution::Resolved)
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }

    pub(crate) fn reject_native_dynamic_module_import(
        &mut self,
        request: PendingDynamicModuleImport,
        message: &str,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.reject_native_dynamic_module_import_with_constructor(request, message, None)
    }

    #[cfg(test)]
    pub(crate) fn reject_native_dynamic_module_import_with_error(
        &mut self,
        request: PendingDynamicModuleImport,
        error: &ModuleLoadError,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.reject_native_dynamic_module_import_with_constructor(
            request,
            error.message(),
            error.error_constructor(),
        )
    }

    fn reject_native_dynamic_module_import_with_constructor(
        &mut self,
        request: PendingDynamicModuleImport,
        message: &str,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> std::result::Result<(), ModuleLoadError> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, request.context());
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::Local::new(scope, request.resolver());
                let message = v8_string(scope, message);
                let exception = message
                    .and_then(|message| {
                        error_constructor
                            .and_then(|kind| script_error_value(scope, kind, message))
                            .or_else(|| Some(v8::Exception::type_error(scope, message)))
                    })
                    .unwrap_or_else(|| v8::undefined(scope).into());
                let _ = resolver.reject(scope, exception);
                Self::perform_microtask_checkpoints(scope, None)?;
                Ok(())
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })
    }

    pub(crate) fn module_reaction_target_is_current(
        &self,
        target: crate::page_task_queue::RendererPageModuleReactionTarget,
    ) -> bool {
        match target {
            crate::page_task_queue::RendererPageModuleReactionTarget::DocumentModuleScript {
                document_owner,
            } => self.current_main_document_task_owner() == Some(document_owner),
            crate::page_task_queue::RendererPageModuleReactionTarget::ChildParserModule {
                document_owner,
                realm_id,
            } => self.child_parser_module_route_task_is_current(document_owner, realm_id),
            crate::page_task_queue::RendererPageModuleReactionTarget::DynamicModuleImport {
                import_owner,
            } => self.dynamic_module_import_owner_is_current(import_owner),
        }
    }

    /// Apply one module reaction whose exact target has already been accepted
    /// by the Page owner arbiter.
    pub(crate) fn apply_current_page_module_reaction(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentPageModuleReaction,
    ) -> Result<PageModuleReactionApplication> {
        let reaction = authorization.into_task().into_event();
        let application = match reaction {
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationFulfilled {
                reaction_id,
                ..
            } => self
                .apply_native_module_script_evaluation_fulfilled(reaction_id)
                .map(PageModuleReactionApplication::module_state_updated),
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationRejected {
                reaction_id,
                reason,
                error_constructor,
                ..
            } => self
                .apply_native_module_script_evaluation_rejected(
                    reaction_id,
                    reason,
                    error_constructor,
                )
                .map(PageModuleReactionApplication::module_state_updated),
            RendererPageModuleReactionEvent::ChildParserModuleEvaluationFulfilled {
                reaction_id,
                ..
            } => (self.apply_child_parser_module_evaluation_fulfilled(reaction_id) > 0).then_some(
                PageModuleReactionApplication::module_state_updated(
                    PageModuleReactionFollowup::None,
                ),
            ),
            RendererPageModuleReactionEvent::ChildParserModuleEvaluationRejected {
                reaction_id,
                reason,
                error_constructor,
                ..
            } => (self.apply_child_parser_module_evaluation_rejected(
                reaction_id,
                reason,
                error_constructor,
            ) > 0)
                .then_some(PageModuleReactionApplication::module_state_updated(
                    PageModuleReactionFollowup::None,
                )),
            RendererPageModuleReactionEvent::DynamicModuleEvaluationFulfilled {
                import_owner,
                reaction_id,
            } => self
                .apply_native_dynamic_module_evaluation_fulfilled(import_owner, reaction_id)
                .map_err(|error| anyhow::anyhow!(error.message().to_owned()))?
                .then_some(PageModuleReactionApplication::dynamic_import_promise_settled()),
            RendererPageModuleReactionEvent::DynamicModuleEvaluationRejected {
                import_owner,
                reaction_id,
                reason,
            } => self
                .apply_native_dynamic_module_evaluation_rejected(import_owner, reaction_id, reason)
                .map_err(|error| anyhow::anyhow!(error.message().to_owned()))?
                .then_some(PageModuleReactionApplication::dynamic_import_promise_settled()),
        };
        Ok(application.unwrap_or(PageModuleReactionApplication::NoPendingReaction))
    }

    pub(crate) fn discard_stale_page_module_reaction(
        &mut self,
        reaction: &RendererPageModuleReactionEvent,
    ) {
        let (import_owner, reaction_id) = match reaction {
            RendererPageModuleReactionEvent::DynamicModuleEvaluationFulfilled {
                import_owner,
                reaction_id,
            }
            | RendererPageModuleReactionEvent::DynamicModuleEvaluationRejected {
                import_owner,
                reaction_id,
                ..
            } => (*import_owner, *reaction_id),
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationFulfilled { .. }
            | RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationRejected { .. }
            | RendererPageModuleReactionEvent::ChildParserModuleEvaluationFulfilled { .. }
            | RendererPageModuleReactionEvent::ChildParserModuleEvaluationRejected { .. } => {
                return;
            }
        };
        let reaction_claimed = self
            .document_runtime
            .take_native_dynamic_module_evaluation_reaction(reaction_id, import_owner)
            .is_some();
        self.record_runtime_warning(format_args!(
            "ignored stale module reaction: owner={import_owner:?} reaction_id={reaction_id} reaction_claimed={reaction_claimed}"
        ));
    }

    #[cfg(test)]
    pub(crate) fn has_page_module_reaction_for_executor_test(&self) -> bool {
        self._page_task_residence_for_executor_test
            .as_ref()
            .expect("module-reaction executor fixture must retain its production Page source")
            .task_sources()
            .has_module_reaction_for_executor_test()
    }

    /// Publish a document-module reaction without reserving a local reaction
    /// record. This is only for Page authorization tests that need a concrete
    /// spent or stale source ticket.
    #[cfg(test)]
    pub(crate) fn queue_missing_document_module_reaction_for_test(&mut self, reaction_id: u64) {
        let document_owner = self
            .current_main_document_task_owner()
            .expect("module reaction fixture requires a current main Document owner");
        self._context_host
            .borrow_mut()
            .queue_document_module_script_evaluation_fulfilled(document_owner, reaction_id);
    }

    /// Apply only the body of one production module-reaction task in a
    /// low-level ScriptVm semantic fixture.
    ///
    /// This helper deliberately does not model selected-task completion.
    /// Page-root admission, task-end checkpoint ownership, and scheduler
    /// liveness are covered through the PageVm selected-dispatcher test driver.
    #[cfg(test)]
    pub(crate) fn run_page_module_reaction_body_for_test(
        &mut self,
    ) -> Result<Option<crate::page_task_queue::PageModuleReactionTargetEffect>> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("module-reaction executor fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_module_reaction_for_executor_test() else {
            return Ok(None);
        };
        if self.module_reaction_target_is_current(task.owner().target()) {
            let application = self.apply_current_page_module_reaction(
                crate::runtime::AuthorizedCurrentPageModuleReaction::new_for_executor_test(task),
            )?;
            return Ok(Some(match application {
                PageModuleReactionApplication::Applied { current_effect, .. } => {
                    crate::page_task_queue::PageModuleReactionTargetEffect::AppliedToCurrentOwner(
                        current_effect,
                    )
                }
                PageModuleReactionApplication::NoPendingReaction => {
                    crate::page_task_queue::PageModuleReactionTargetEffect::DiscardedMissingReaction
                }
            }));
        }
        self.discard_stale_page_module_reaction(&task.into_event());
        Ok(Some(
            crate::page_task_queue::PageModuleReactionTargetEffect::IgnoredStaleOwner,
        ))
    }

    fn apply_native_module_script_evaluation_fulfilled(
        &mut self,
        reaction_id: u64,
    ) -> Option<PageModuleReactionFollowup> {
        let update = self.mark_module_evaluation_reaction_fulfilled_for_owner(reaction_id)?;
        let (root_entry, followup) = match update {
            DocumentModuleReactionUpdate::ParserOwned(update) => (
                update.root_entry(),
                PageModuleReactionFollowup::main_parser_owned_evaluations(
                    update.queued_ready_action_count(),
                ),
            ),
            DocumentModuleReactionUpdate::RuntimeOwned(update) => (
                update.root_entry,
                PageModuleReactionFollowup::RuntimeOwnedModuleContinuation,
            ),
        };
        self.document_runtime
            .mark_native_module_evaluated(root_entry);
        Some(followup)
    }

    fn apply_native_module_script_evaluation_rejected(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Option<PageModuleReactionFollowup> {
        let update = self.mark_module_evaluation_reaction_rejected_for_owner(
            reaction_id,
            reason,
            error_constructor,
        )?;
        Some(match update {
            DocumentModuleReactionUpdate::ParserOwned(update) => {
                PageModuleReactionFollowup::main_parser_owned_evaluations(
                    update.queued_ready_action_count(),
                )
            }
            DocumentModuleReactionUpdate::RuntimeOwned(_) => {
                PageModuleReactionFollowup::RuntimeOwnedModuleContinuation
            }
        })
    }

    pub(crate) fn apply_native_dynamic_module_evaluation_fulfilled(
        &mut self,
        import_owner: crate::module_runtime::DynamicModuleImportOwner,
        reaction_id: u64,
    ) -> std::result::Result<bool, ModuleLoadError> {
        let Some(reaction) = self
            .document_runtime
            .take_native_dynamic_module_evaluation_reaction(reaction_id, import_owner)
        else {
            return Ok(false);
        };
        let (request, target) = reaction.into_parts();
        let child_owner_parts = request
            .owner()
            .child_parts()
            .map(|(_child_handle, task_owner, realm_id)| (task_owner.document_owner(), realm_id));
        if let Some((owner, realm_id)) = child_owner_parts {
            self.mark_child_native_dynamic_module_evaluated(owner, realm_id, target.root_entry());
        } else {
            self.document_runtime
                .mark_native_module_evaluated(target.root_entry());
        }
        // Commit the exact owner's module-map state before resolving the user
        // Promise. Its reactions may replace the Document when the selected
        // task dispatcher performs the task-end checkpoint.
        self.resolve_native_dynamic_module_import_selected_task_body(request, &target)?;
        Ok(true)
    }

    pub(crate) fn apply_native_dynamic_module_evaluation_rejected(
        &mut self,
        import_owner: crate::module_runtime::DynamicModuleImportOwner,
        reaction_id: u64,
        reason: v8::Global<v8::Value>,
    ) -> std::result::Result<bool, ModuleLoadError> {
        let Some(reaction) = self
            .document_runtime
            .take_native_dynamic_module_evaluation_reaction(reaction_id, import_owner)
        else {
            return Ok(false);
        };
        let (request, _) = reaction.into_parts();
        self.reject_native_dynamic_module_import_reaction_body(request, reason)?;
        Ok(true)
    }
}

fn wasm_module_requests_for_imports(imports: &[WasmImportRecord]) -> Vec<ModuleRequestRecord> {
    wasm_evaluation_import_modules(imports)
        .into_iter()
        .map(|module| {
            ModuleRequestRecord::new(
                module,
                ModuleAttributesKey::empty(),
                ModuleImportPhase::Evaluation,
            )
        })
        .collect()
}

fn native_dynamic_module_reaction_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(reaction_id) = native_dynamic_module_reaction_data(scope, args.data()) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }.queue_native_dynamic_module_evaluation_fulfilled(reaction_id);
}

fn native_dynamic_module_reaction_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(reaction_id) = native_dynamic_module_reaction_data(scope, args.data()) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let reason = args.get(0);
    let reason = v8::Global::new(scope, reason);
    unsafe { &mut *host_ptr }.queue_native_dynamic_module_evaluation_rejected(reaction_id, reason);
}

fn native_dynamic_module_reaction_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<u64> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    get_u64_reaction_data_slot(scope, data, DYNAMIC_MODULE_REACTION_ID_SLOT)
}

fn native_module_script_reaction_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((document_owner, reaction_id)) =
        native_module_script_reaction_data(scope, args.data())
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }
        .queue_document_module_script_evaluation_fulfilled(document_owner, reaction_id);
}

fn native_module_script_reaction_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((document_owner, reaction_id)) =
        native_module_script_reaction_data(scope, args.data())
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let reason = args.get(0);
    let error_constructor = script_error_constructor_kind_from_value(scope, reason);
    let reason = reason
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "unknown promise rejection".to_owned());
    unsafe { &mut *host_ptr }.queue_document_module_script_evaluation_rejected(
        document_owner,
        reaction_id,
        reason,
        error_constructor,
    );
}

fn child_parser_module_reaction_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((document_owner, realm_id, reaction_id)) =
        native_child_module_script_reaction_data(scope, args.data())
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }.queue_child_parser_module_script_evaluation_fulfilled(
        document_owner,
        realm_id,
        reaction_id,
    );
}

fn child_parser_module_reaction_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((document_owner, realm_id, reaction_id)) =
        native_child_module_script_reaction_data(scope, args.data())
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let reason = args.get(0);
    let error_constructor = script_error_constructor_kind_from_value(scope, reason);
    let reason = reason
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "unknown promise rejection".to_owned());
    unsafe { &mut *host_ptr }.queue_child_parser_module_script_evaluation_rejected(
        document_owner,
        realm_id,
        reaction_id,
        reason,
        error_constructor,
    );
}

fn native_module_script_reaction_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(FrameDocumentTaskOwner, u64)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let reaction_id = get_u64_reaction_data_slot(scope, data, MODULE_SCRIPT_REACTION_ID_SLOT)?;
    let document_owner = frame_document_owner_from_module_reaction_data(scope, data)?;
    Some((document_owner, reaction_id))
}

fn native_child_module_script_reaction_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(FrameDocumentTaskOwner, FrameRealmId, u64)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let reaction_id = get_u64_reaction_data_slot(scope, data, MODULE_SCRIPT_REACTION_ID_SLOT)?;
    let document_owner = frame_document_owner_from_module_reaction_data(scope, data)?;
    let realm_id = get_i64_reaction_data_slot(scope, data, MODULE_REACTION_REALM_ID_SLOT)?;
    Some((document_owner, FrameRealmId(realm_id), reaction_id))
}

fn frame_document_owner_from_module_reaction_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
) -> Option<FrameDocumentTaskOwner> {
    let scheduler_lane_id =
        get_u64_reaction_data_slot(scope, data, MODULE_REACTION_SCHEDULER_LANE_ID_SLOT)?;
    let local_window_id =
        get_u64_reaction_data_slot(scope, data, MODULE_REACTION_LOCAL_WINDOW_ID_SLOT)?;
    let document_id = get_u64_reaction_data_slot(scope, data, MODULE_REACTION_DOCUMENT_ID_SLOT)?;
    Some(FrameDocumentTaskOwner::new(
        FrameSchedulerLaneId(scheduler_lane_id),
        LocalWindowId(local_window_id),
        DocumentId(document_id),
    ))
}

fn get_u64_reaction_data_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<u64> {
    let value = data.get(scope, v8str(scope, slot).into())?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = value.u64_value();
    lossless.then_some(value)
}

fn get_i64_reaction_data_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<i64> {
    let value = data.get(scope, v8str(scope, slot).into())?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = value.i64_value();
    lossless.then_some(value)
}

fn synthetic_module_evaluation_steps<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return throw_synthetic_module_error(scope, "synthetic module host is not available");
    };
    let Some((key, source)) = (unsafe { &*host_ptr }).native_module_source_for(module) else {
        return throw_synthetic_module_error(scope, "synthetic module source is not available");
    };
    match key.kind() {
        ModuleKind::Json => {
            let Some(source) = source.text_source() else {
                return throw_synthetic_module_error(scope, "JSON module source is not text");
            };
            evaluate_json_synthetic_module(scope, module, source)
        }
        ModuleKind::Css => {
            let Some(source) = source.text_source() else {
                return throw_synthetic_module_error(scope, "CSS module source is not text");
            };
            evaluate_css_synthetic_module(scope, module, key.url().as_str(), source)
        }
        ModuleKind::WebAssembly => {
            let Some(wasm_record) = (unsafe { &*host_ptr }).native_module_wasm_record_for(module)
            else {
                return throw_synthetic_module_error(
                    scope,
                    "WebAssembly synthetic module record is not available",
                );
            };
            evaluate_wasm_synthetic_module(scope, module, &wasm_record, |scope, import| {
                wasm_import_value(scope, module, import)
            })
        }
        ModuleKind::JavaScript | ModuleKind::ModulePreloadText => throw_synthetic_module_error(
            scope,
            "non-synthetic module reached synthetic module evaluation",
        ),
    }
}

fn evaluate_json_synthetic_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    source: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(json_source) = v8_string(scope, source) else {
        return throw_synthetic_module_error(scope, "failed to allocate JSON module source");
    };
    let value = v8::json::parse(scope, json_source)?;
    set_synthetic_default_export(scope, module, value)
}

fn evaluate_css_synthetic_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    url: &str,
    source: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(sheet) =
        crate::native_bridge::element::css_module_sheet_for_url(scope, url, Some(source))
    else {
        return throw_synthetic_module_error(scope, "failed to create CSS module sheet");
    };
    set_synthetic_default_export(scope, module, sheet.into())
}

fn set_synthetic_default_export<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let export_name = v8str(scope, "default");
    if module
        .set_synthetic_module_export(scope, export_name, value)
        .is_none_or(|ok| !ok)
    {
        return throw_synthetic_module_error(
            scope,
            "failed to set synthetic module default export",
        );
    }
    Some(v8::undefined(scope).into())
}

fn wasm_import_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    referrer: v8::Local<'s, v8::Module>,
    import: &WasmImportRecord,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return throw_wasm_link_error(scope, "wasm import host is not available");
    };
    let attributes = ModuleAttributesKey::empty();
    let Some(dependency) = (unsafe { &*host_ptr }).native_resolved_dependency_module_for(
        referrer,
        import.module(),
        &attributes,
    ) else {
        return throw_wasm_link_error(scope, "wasm import dependency is not available");
    };
    let dependency = v8::Local::new(scope, &dependency);
    ensure_dependency_module_namespace_ready(scope, dependency)?;
    let dependency_wasm_record = (unsafe { &*host_ptr }).native_module_wasm_record_for(dependency);
    wasm_dependency_export_value(
        scope,
        dependency,
        dependency_wasm_record.as_ref(),
        import.name(),
        "failed to allocate wasm import export name",
        "wasm import export is not available",
    )
}

fn ensure_dependency_module_namespace_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
) -> Option<()> {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_synthetic_module_error(scope, "module dependency host is not available");
        return None;
    };
    let document_modulator_ptr = (unsafe { &*host_ptr }).native_document_modulator_ptr();
    let document_modulator = unsafe { &*document_modulator_ptr };
    let mut dependency_modules_for =
        |_: &mut v8::PinScope<'s, '_>, dependency: v8::Local<'s, v8::Module>| {
            document_modulator.evaluation_dependency_modules_for(dependency)
        };
    ensure_wasm_dependency_module_namespace_ready(
        scope,
        module,
        |scope: &mut v8::PinScope<'s, '_>, module: v8::Local<'s, v8::Module>| {
            let _resolver_scope = ResolverScopeGuard::new(document_modulator_ptr);
            match module.instantiate_module2(
                scope,
                resolve_static_module_callback,
                resolve_static_source_callback,
            ) {
                Some(true) => Some(()),
                Some(false) => {
                    throw_synthetic_module_error(
                        scope,
                        "module dependency instantiate returned false",
                    );
                    None
                }
                None => {
                    preserve_current_v8_module_exception(scope);
                    None
                }
            }
        },
        &mut dependency_modules_for,
        |scope| {
            if ScriptVm::perform_microtask_checkpoints(scope, None).is_err() {
                throw_synthetic_module_error(
                    scope,
                    "module dependency microtask checkpoint failed",
                );
                return None;
            }
            Some(())
        },
        WasmDependencyModuleMessages {
            instantiating: "module dependency is still instantiating",
            already_failed: "module dependency already failed",
            evaluation_failed: "module dependency evaluation failed",
            not_instantiated: "module dependency is not instantiated",
            cyclic: "cyclic WebAssembly module evaluation through JavaScript dependencies is not supported yet",
            graph_unavailable: "module dependency graph is not available",
            pending: "module dependency evaluation is pending",
        },
    )
}

fn throw_synthetic_module_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let message = v8_string(scope, message)?;
    let exception = v8::Exception::type_error(scope, message);
    scope.throw_exception(exception);
    None
}

fn canonical_native_module_instantiate_error(exception: &str, graph_urls: &[Url]) -> String {
    if let Some(error) = canonical_missing_export_link_error(exception, graph_urls) {
        return error;
    }
    format!("v8 failed to instantiate native module graph: {exception}")
}

fn canonical_missing_export_link_error(exception: &str, graph_urls: &[Url]) -> Option<String> {
    let module = quoted_value_after(exception, "The requested module ")?;
    let export = quoted_value_after(exception, "does not provide an export named ")?;
    let module = canonical_link_error_module_url(module, graph_urls);
    Some(format!(
        "ModuleLinkFailed: module `{module}` does not export `{export}`"
    ))
}

fn canonical_link_error_module_url(module: &str, graph_urls: &[Url]) -> String {
    if Url::parse(module).is_ok() {
        return module.to_owned();
    }
    let suffix = module.trim_start_matches("./");
    graph_urls
        .iter()
        .find(|url| url.path().ends_with(suffix))
        .map(ToString::to_string)
        .unwrap_or_else(|| module.to_owned())
}

fn quoted_value_after<'a>(message: &'a str, marker: &str) -> Option<&'a str> {
    let start = message.find(marker)? + marker.len();
    let rest = message.get(start..)?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' && quote != '`' {
        return None;
    }
    let value_start = quote.len_utf8();
    let rest = rest.get(value_start..)?;
    let value_end = rest.find(quote)?;
    rest.get(..value_end)
}

fn native_module_evaluation_exception_error(
    scope: &mut v8::PinScope<'_, '_>,
    exception: v8::Local<'_, v8::Value>,
    prefix: &str,
) -> ModuleLoadError {
    let message = exception
        .to_string(scope)
        .map(|message| message.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "unknown module evaluation exception".to_owned());
    let error = ModuleLoadError::new(ModuleLoadStage::Evaluate, format!("{prefix}: {message}"));
    match script_error_constructor_kind_from_value(scope, exception) {
        Some(error_constructor) => error.with_error_constructor(error_constructor),
        None => error,
    }
}

fn script_error_constructor_kind_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<ScriptErrorConstructorKind> {
    if !value.is_native_error() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let prototype = object.get_prototype(scope)?;
    for candidate in [
        ScriptErrorConstructorKind::SyntaxError,
        ScriptErrorConstructorKind::WebAssemblyCompileError,
        ScriptErrorConstructorKind::WebAssemblyLinkError,
        ScriptErrorConstructorKind::Error,
    ] {
        if let Some(candidate_prototype) = script_error_prototype(scope, candidate)
            && prototype.strict_equals(candidate_prototype)
        {
            return Some(candidate);
        }
    }
    None
}

fn script_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor_kind: ScriptErrorConstructorKind,
    message: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Value>> {
    match constructor_kind {
        ScriptErrorConstructorKind::Error => Some(v8::Exception::error(scope, message)),
        ScriptErrorConstructorKind::SyntaxError => {
            Some(v8::Exception::syntax_error(scope, message))
        }
        ScriptErrorConstructorKind::WebAssemblyCompileError => {
            captured_webassembly_error_constructor(
                scope,
                ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
            )
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .and_then(|constructor| constructor.new_instance(scope, &[message.into()]))
            .map(v8::Local::<v8::Value>::from)
        }
        ScriptErrorConstructorKind::WebAssemblyLinkError => captured_webassembly_error_constructor(
            scope,
            ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
        )
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .and_then(|constructor| constructor.new_instance(scope, &[message.into()]))
        .map(v8::Local::<v8::Value>::from),
    }
}

fn script_error_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor_kind: ScriptErrorConstructorKind,
) -> Option<v8::Local<'s, v8::Value>> {
    let empty = v8::String::empty(scope);
    let error = match constructor_kind {
        ScriptErrorConstructorKind::Error => v8::Exception::error(scope, empty),
        ScriptErrorConstructorKind::SyntaxError => v8::Exception::syntax_error(scope, empty),
        ScriptErrorConstructorKind::WebAssemblyCompileError => {
            let constructor = captured_webassembly_error_constructor(
                scope,
                ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
            )
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
            constructor.new_instance(scope, &[])?.into()
        }
        ScriptErrorConstructorKind::WebAssemblyLinkError => {
            let constructor = captured_webassembly_error_constructor(
                scope,
                ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
            )
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
            constructor.new_instance(scope, &[])?.into()
        }
    };
    v8::Local::<v8::Object>::try_from(error)
        .ok()?
        .get_prototype(scope)
}

fn captured_webassembly_error_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, slot)
}

fn module_fetch_csp_request(
    fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
) -> crate::content_security_policy::ContentSecurityPolicyScriptElementRequest<'_> {
    crate::content_security_policy::ContentSecurityPolicyScriptElementRequest {
        nonce: fetch_metadata.nonce(),
        integrity: fetch_metadata.integrity(),
        parser_inserted: fetch_metadata.parser_inserted,
    }
}

fn create_module_script_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: &str,
    fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
) -> v8::ScriptOrigin<'s> {
    let name = v8::String::new(scope, resource_name).expect("v8 string allocation");
    let base_url = Url::parse(resource_name).ok();
    let host_defined_options = base_url.as_ref().and_then(|base_url| {
        crate::util::script_host_defined_options_with_fetch_metadata(
            scope,
            base_url,
            fetch_metadata.nonce(),
            fetch_metadata.parser_inserted,
        )
    });
    v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        -1,
        None,
        false,
        false,
        true,
        host_defined_options,
    )
}

fn collect_module_requests(
    scope: &mut v8::PinScope<'_, '_>,
    module: v8::Local<'_, v8::Module>,
) -> Result<Vec<ModuleRequestRecord>> {
    let requests = module.get_module_requests();
    let mut records = Vec::with_capacity(requests.length());
    for index in 0..requests.length() {
        let Some(request_data) = requests.get(scope, index) else {
            continue;
        };
        let request = v8::Local::<v8::ModuleRequest>::try_from(request_data)
            .map_err(|_| anyhow::anyhow!("module request entry was not a ModuleRequest"))?;
        let specifier = request.get_specifier().to_rust_string_lossy(scope);
        let attributes = module_request_attributes(scope, request);
        if let Some(invalid_key) = attributes.invalid_import_attribute_key() {
            return Err(anyhow::anyhow!("Invalid attribute key \"{invalid_key}\"."));
        }
        records.push(ModuleRequestRecord::new(
            specifier,
            attributes,
            module_import_phase(request.get_phase()),
        ));
    }
    Ok(records)
}

fn module_request_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::ModuleRequest>,
) -> ModuleAttributesKey {
    let attributes = request.get_import_attributes();
    let mut pairs = Vec::with_capacity(attributes.length() / 3);
    let mut index = 0;
    while index + 1 < attributes.length() {
        let key = attributes
            .get(scope, index)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        let value = attributes
            .get(scope, index + 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        if let (Some(key), Some(value)) = (key, value) {
            pairs.push((key, value));
        }
        index += 3;
    }
    ModuleAttributesKey::from_pairs(pairs)
}

fn module_import_phase(phase: v8::ModuleImportPhase) -> ModuleImportPhase {
    match phase {
        v8::ModuleImportPhase::kSource => ModuleImportPhase::Source,
        _ => ModuleImportPhase::Evaluation,
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::{
        DYNAMIC_MODULE_REACTION_ID_SLOT, MainNativeModuleSelectedTaskApplication,
        NativeChildModuleScriptReactionDataDeclaration, NativeDynamicModuleReactionDataDeclaration,
        NativeModuleScriptReactionDataDeclaration, ScriptVmCheckpointingMainNativeModuleTaskBody,
        get_u64_reaction_data_slot, native_child_module_script_reaction_data,
        native_module_script_reaction_data, preserve_current_v8_module_exception,
    };
    use crate::dom::native::{DomHost, NativeDom};
    use crate::ensure_v8_for_test as ensure_v8;
    use crate::frame_owner_model::{
        ChildFrameSemanticTurnKind, DocumentId, FrameDocumentDynamicImportGraphAdvanceFollowup,
        FrameDocumentDynamicImportMissingJoinedTerminalFetch,
        FrameDocumentDynamicImportOwnerAction, FrameDocumentDynamicImportPendingJobResume,
        FrameDocumentOwner, FrameDocumentTaskOwner, FrameRealmId, FrameSchedulerLaneId,
        LocalWindowId,
    };
    use crate::module_runtime::{
        DynamicModuleFetchFailure, DynamicModuleFetchOwnerAdvance, DynamicModuleImportOwner,
        ModuleAttributesKey, ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource,
        ModuleGraphHandle, ModuleImportPhase, ModuleKind, ModuleLoadError, ModuleLoadStage,
        ModuleMapKey, ModuleSource, NativeModuleGraphFetchRequest, NativeModuleGraphJob,
        NativeModuleGraphJobAdvance, PendingDynamicModuleImport,
    };
    use crate::module_script_continuation::NativeDynamicModuleTerminalFanout;
    use crate::network::ResourceRequestClient;
    use crate::script_vm::{ScriptVm, ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness};
    use crate::types::{
        ModuleGraphFetchCompletion, ModuleGraphFetchOrdering, ModuleGraphFetchRequester,
    };
    use crate::util::v8str;
    use moli_fetch::FetchConfig;
    use url::Url;

    fn new_test_vm(url: &str) -> StandaloneScriptVmHarness {
        let _js_runtime = crate::JsRuntime::initialize();
        let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let post_domcontentloaded_page_task_sender =
            page_task_queue.owner_attached_runtime_page_task_sender_for_test();
        let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
        let page_runtime_task_source = page_task_queue.residence();
        ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
            DomHost::from_dom(NativeDom::new(Url::parse(url).expect("test url"))),
            post_domcontentloaded_page_task_sender,
            page_task_front_injection_tx,
        )
        .expect("script vm bootstrap should succeed")
        .finish()
        .map(|mut vm| {
            vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
            vm
        })
        .expect("script vm finish should succeed")
    }

    fn dynamic_import_completion(load_id: u64, request_url: &str) -> ModuleGraphFetchCompletion {
        dynamic_import_completion_with_source(load_id, request_url, "export const value = 1;")
    }

    fn dynamic_import_completion_with_source(
        load_id: u64,
        request_url: &str,
        source: &str,
    ) -> ModuleGraphFetchCompletion {
        let url = Url::parse(request_url).expect("request URL should parse");
        ModuleGraphFetchCompletion {
            load_id,
            requester: ModuleGraphFetchRequester::DynamicImport,
            ordering: ModuleGraphFetchOrdering::Runtime,
            request_url: url.clone(),
            result: Ok(ModuleGraphFetchedSource::new(
                url,
                false,
                ModuleSource::text(source.to_owned()),
            )),
            network_result: None,
        }
    }

    fn dynamic_import_reaction_parts_in_vm(
        vm: &mut ScriptVm,
        owner: DynamicModuleImportOwner,
    ) -> (
        PendingDynamicModuleImport,
        crate::module_runtime::DynamicModuleEvaluationTarget,
    ) {
        let context_ptr: *const v8::Global<v8::Context> = match owner.child_parts() {
            None => &vm.page_default_context,
            Some((_child_handle, _task_owner, realm_id)) => vm
                .frame_realm_context_ptr(realm_id)
                .expect("child dynamic-import reaction test realm must be materialized"),
        };
        vm.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
                let source =
                    v8::String::new(scope, "export default 1;").expect("test module source");
                let source_name =
                    v8::String::new(scope, "https://dynamic-reaction-owner.test/pending.mjs")
                        .expect("test module source name");
                let origin = v8::ScriptOrigin::new(
                    scope,
                    source_name.into(),
                    0,
                    0,
                    false,
                    -1,
                    None,
                    false,
                    false,
                    true,
                    None,
                );
                let mut compiler_source = v8::script_compiler::Source::new(source, Some(&origin));
                let module = v8::script_compiler::compile_module(scope, &mut compiler_source)
                    .expect("test module should compile");
                Ok((
                    PendingDynamicModuleImport::new(
                        v8::Global::new(scope, scope.get_current_context()),
                        v8::Global::new(scope, resolver),
                        owner,
                        "./pending.mjs",
                        Url::parse("https://dynamic-reaction-owner.test/page.html")
                            .expect("test base URL"),
                        ModuleAttributesKey::empty(),
                        ModuleImportPhase::Evaluation,
                    ),
                    crate::module_runtime::DynamicModuleEvaluationTarget::new(
                        ModuleEntryId::for_test(0),
                        v8::Global::new(scope, module),
                    ),
                ))
            })
            .expect("test reaction payload should be created in the ScriptVm isolate")
    }

    fn dynamic_import_request_in_vm(
        vm: &mut ScriptVm,
        specifier: &str,
        base_url: Url,
        phase: ModuleImportPhase,
    ) -> PendingDynamicModuleImport {
        let context_host = vm._context_host.clone();
        vm.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &vm.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let owner = context_host
                    .borrow()
                    .current_dynamic_module_import_owner(scope, None)
                    .expect("test ScriptVm must have a main dynamic-import owner");
                let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
                Ok(PendingDynamicModuleImport::new(
                    v8::Global::new(scope, scope.get_current_context()),
                    v8::Global::new(scope, resolver),
                    owner,
                    specifier,
                    base_url,
                    ModuleAttributesKey::empty(),
                    phase,
                ))
            })
            .expect("test dynamic import request should be created in the ScriptVm isolate")
    }

    fn current_child_dynamic_import_owner(
        vm: &mut ScriptVm,
        child_handle: crate::document_runtime::DomHandle,
    ) -> DynamicModuleImportOwner {
        let realm_id = vm
            .child_frame_realm_store
            .values()
            .find(|realm| realm.child_handle == child_handle)
            .map(|realm| realm.owner_realm_id)
            .expect("test child document must have a current realm");
        vm.with_frame_realm_scope_and_checkpoint_for_test(realm_id, move |scope, host_ptr| {
            unsafe { &*host_ptr }
                .current_dynamic_module_import_owner(scope, Some(child_handle))
                .ok_or_else(|| {
                    anyhow::anyhow!("test child document must have a current dynamic-import owner")
                })
        })
        .expect("test child owner scope should enter")
    }

    async fn commit_child_document_and_run_parser_script_for_dynamic_import_test(
        vm: &mut ScriptVm,
        label: &str,
    ) {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "{label} should commit its srcdoc navigation before script execution"
        );
        while vm
            .run_child_realm_materialization_body_for_test()
            .expect("child realm materialization prerequisite should succeed")
        {
            // Consume only consecutive realm tasks at the child-family head;
            // a script task must never be bypassed to reach a later realm.
        }
        assert!(
            vm.run_child_frame_task_source_once_for_test(
                ChildFrameSemanticTurnKind::DocumentScriptReady
            )
            .await,
            "{label} parser script should run from DocumentScriptReady"
        );
    }

    async fn finish_child_document_after_parser_script_for_dynamic_import_test(
        vm: &mut ScriptVm,
        label: &str,
    ) {
        for transition in ["interactive", "DOMContentLoaded", "complete"] {
            assert!(
                vm.run_child_frame_task_source_once_for_test(
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                )
                .await,
                "{label} should run its {transition} lifecycle turn"
            );
        }
        assert!(
            vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
                .await,
            "{label} should finish through a later HostLoad turn"
        );
    }

    async fn commit_child_document_and_run_parser_script_for_page_executor_test(
        vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
        label: &str,
    ) {
        for expected in [
            ChildFrameSemanticTurnKind::NavigationCommit,
            ChildFrameSemanticTurnKind::RealmMaterialization,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
        ] {
            assert_eq!(
                vm.run_next_child_frame_semantic_turn().await,
                Some(expected),
                "{label} should advance {expected:?} through the real Page-owned realm prerequisite"
            );
        }
    }

    async fn finish_child_document_after_parser_script_for_page_executor_test(
        vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
        label: &str,
    ) {
        for expected in [
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            ChildFrameSemanticTurnKind::HostLoad,
        ] {
            assert_eq!(
                vm.run_next_child_frame_semantic_turn().await,
                Some(expected),
                "{label} should advance {expected:?} after its parser script"
            );
        }
    }

    fn dynamic_import_job_in_vm(
        vm: &mut ScriptVm,
        specifier: &str,
        base_url: Url,
        phase: ModuleImportPhase,
    ) -> NativeModuleGraphJob {
        NativeModuleGraphJob::dynamic_import(dynamic_import_request_in_vm(
            vm, specifier, base_url, phase,
        ))
    }

    #[test]
    fn reaction_data_slots_preserve_u64_values_above_js_safe_integer() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let reaction_id = (1_u64 << 53) + 123;
        let data = NativeDynamicModuleReactionDataDeclaration {
            reaction_id: v8::BigInt::new_from_u64(scope, reaction_id),
        }
        .bind(scope)
        .expect("reaction data declaration should bind");

        assert_eq!(
            get_u64_reaction_data_slot(scope, data, DYNAMIC_MODULE_REACTION_ID_SLOT),
            Some(reaction_id)
        );

        let scheduler_lane_id = (1_u64 << 53) + 125;
        let local_window_id = (1_u64 << 53) + 126;
        let document_id = (1_u64 << 53) + 127;
        let realm_id = -17_i64;
        let document_owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(scheduler_lane_id),
            LocalWindowId(local_window_id),
            DocumentId(document_id),
        );
        let main_data = NativeModuleScriptReactionDataDeclaration {
            module_script_reaction_id: v8::BigInt::new_from_u64(scope, reaction_id),
            scheduler_lane_id: v8::BigInt::new_from_u64(scope, scheduler_lane_id),
            local_window_id: v8::BigInt::new_from_u64(scope, local_window_id),
            document_id: v8::BigInt::new_from_u64(scope, document_id),
        }
        .bind(scope)
        .expect("main reaction data declaration should bind");
        assert_eq!(
            native_module_script_reaction_data(scope, main_data.into()),
            Some((document_owner, reaction_id)),
            "callback data must preserve the exact main Document without Number coercion"
        );
        let child_data = NativeChildModuleScriptReactionDataDeclaration {
            module_script_reaction_id: v8::BigInt::new_from_u64(scope, reaction_id),
            scheduler_lane_id: v8::BigInt::new_from_u64(scope, scheduler_lane_id),
            local_window_id: v8::BigInt::new_from_u64(scope, local_window_id),
            document_id: v8::BigInt::new_from_u64(scope, document_id),
            realm_id: v8::BigInt::new_from_i64(scope, realm_id),
        }
        .bind(scope)
        .expect("child reaction data declaration should bind");
        assert_eq!(
            native_child_module_script_reaction_data(scope, child_data.into()),
            Some((document_owner, FrameRealmId(realm_id), reaction_id)),
            "callback data must preserve the exact child Document and realm without Number coercion"
        );
    }

    #[test]
    fn dynamic_import_fetch_completion_requires_owner_facade() {
        let mut vm = new_test_vm("https://app.example.test/page.html");
        let document_owner = vm
            .current_main_document_task_owner()
            .expect("dynamic import test must have a main document owner");
        vm.eval("import('./dynamic.mjs'); 'queued'")
            .expect("dynamic import should queue from current document isolate");

        let queued_job = vm
            .document_runtime
            .take_next_native_dynamic_module_import()
            .expect("dynamic import callback should enqueue one graph job");
        let queued_owner = queued_job
            .dynamic_import_request()
            .expect("dynamic graph job must retain its request")
            .owner();
        assert_eq!(queued_owner.task_owner(), document_owner);
        assert_eq!(
            queued_owner.execution_context_owner(),
            crate::native_bridge::WindowExecutionContextOwner::Frame(
                document_owner.local_window_id
            ),
            "dynamic import acceptance must capture the exact current Window execution context"
        );
        vm.document_runtime
            .resume_native_dynamic_module_import_front(queued_job);

        assert!(
            matches!(
                vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
                MainNativeModuleSelectedTaskApplication::Applied(_)
            ),
            "dynamic import should start its module graph through the selected body"
        );
        assert!(
            vm.has_inflight_dynamic_module_fetch(),
            "dynamic import root fetch should be suspended in owner state"
        );

        vm.complete_native_module_graph_fetch(dynamic_import_completion(
            0,
            "https://app.example.test/dynamic.mjs",
        ))
        .expect("bare graph completion helper should tolerate stale completions");
        assert!(
            vm.has_inflight_dynamic_module_fetch(),
            "bare graph completion helper must not consume dynamic import owner fetches"
        );
        assert!(
            vm.runtime_observable_lifecycle_errors_for_testing()
                .iter()
                .any(|message| message.contains(
                    "native module graph fetch completion 0 arrived without an in-flight module graph job"
                )),
            "bare helper should record the unmatched completion instead of consuming owner state"
        );

        let target = vm
            .current_main_dynamic_import_graph_fetch_target(0)
            .expect("dynamic import must expose its exact resolver target");
        let completion = dynamic_import_completion(0, "https://app.example.test/dynamic.mjs");
        let actions = vm
            .complete_current_main_dynamic_import_graph_fetch_result(target, completion.result)
            .expect("owner facade should complete dynamic import fetch");
        assert!(
            actions.into_parts().0.is_empty(),
            "dynamic import owner completion should not synthesize module-script run actions"
        );
        assert!(
            !vm.has_inflight_dynamic_module_fetch(),
            "owner facade should consume dynamic import fetch state"
        );
    }

    #[test]
    fn module_reaction_source_consumes_exactly_one_current_target_per_turn() {
        let mut vm = new_test_vm("https://module-reaction-one-turn.test/page.html");
        let document_owner = vm
            .current_main_document_task_owner()
            .expect("module reaction fixture requires a main Document owner");
        for reaction_id in [71, 72] {
            vm._context_host
                .borrow_mut()
                .queue_document_module_script_evaluation_fulfilled(document_owner, reaction_id);
        }

        assert_eq!(
            vm.run_page_module_reaction_body_for_test()
                .expect("first module-reaction turn"),
            Some(crate::page_task_queue::PageModuleReactionTargetEffect::DiscardedMissingReaction)
        );
        assert!(
            vm.has_page_module_reaction_for_executor_test(),
            "the second accepted reaction must remain queued after one bounded turn"
        );
        assert_eq!(
            vm.run_page_module_reaction_body_for_test()
                .expect("second module-reaction turn"),
            Some(crate::page_task_queue::PageModuleReactionTargetEffect::DiscardedMissingReaction)
        );
        assert!(!vm.has_page_module_reaction_for_executor_test());
    }

    #[test]
    fn dynamic_import_owner_survives_main_document_open_in_same_execution_context() {
        let mut vm = new_test_vm("https://dynamic-reaction-owner.test/page.html");
        let request = dynamic_import_request_in_vm(
            &mut vm,
            "./pending.mjs",
            Url::parse("https://dynamic-reaction-owner.test/page.html").expect("base URL"),
            ModuleImportPhase::Evaluation,
        );
        let import_owner = request.owner();
        let retired_document_owner = import_owner.task_owner();

        vm.eval("document.open(); 'replaced'")
            .expect("document.open should rotate the main document owner");
        assert_ne!(
            vm.current_main_document_task_owner(),
            Some(retired_document_owner),
            "test setup must retire the original document owner"
        );
        assert!(
            vm.dynamic_module_import_owner_is_current(import_owner),
            "document.open must preserve the ScriptState-like dynamic-import owner"
        );
        assert_eq!(
            import_owner.execution_context_owner(),
            crate::native_bridge::WindowExecutionContextOwner::Frame(
                vm.current_main_document_task_owner()
                    .expect("replacement owner")
                    .local_window_id
            )
        );
    }

    #[tokio::test]
    async fn child_replacement_retires_registered_dynamic_import_reaction() {
        let mut vm = new_test_vm("https://dynamic-reaction-child.test/page.html");
        vm.eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = "<script>parent.__dynamicReactionChildReady = true;<\/script>";
  body.appendChild(frame);
})()
"#,
        )
        .expect("dynamic reaction child setup should evaluate");
        commit_child_document_and_run_parser_script_for_dynamic_import_test(
            &mut vm,
            "dynamic reaction child",
        )
        .await;
        finish_child_document_after_parser_script_for_dynamic_import_test(
            &mut vm,
            "dynamic reaction child",
        )
        .await;
        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("dynamic reaction child realm should exist");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("dynamic reaction child realm record should exist")
            .child_handle;
        let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);
        let (request, target) = dynamic_import_reaction_parts_in_vm(&mut vm, document_owner);
        let reaction_id = vm
            .document_runtime
            .reserve_native_dynamic_module_evaluation_reaction(request, target);
        vm._context_host
            .borrow_mut()
            .queue_native_dynamic_module_evaluation_fulfilled_for_owner_for_test(
                document_owner,
                reaction_id,
            );
        assert_eq!(
            vm.document_runtime
                .native_dynamic_module_evaluation_reaction_owner(reaction_id),
            Some(document_owner),
            "test must reserve a real V8-backed reaction before replacement"
        );
        assert!(vm.has_page_module_reaction_for_executor_test());

        vm.eval("document.querySelector('iframe').srcdoc = '<p>replacement</p>'; 'queued'")
            .expect("child replacement should queue");
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "child replacement should commit through NavigationCommit"
        );

        assert_eq!(
            vm.document_runtime
                .native_dynamic_module_evaluation_reaction_owner(reaction_id),
            None,
            "owner transaction must drop the retired reaction's V8 context, resolver, and module"
        );
        assert!(
            vm.has_page_module_reaction_for_executor_test(),
            "stable Page source must retain the old exact-owner reaction until arbitration"
        );
        assert_eq!(
            vm.run_page_module_reaction_body_for_test()
                .expect("stale reaction arbitration should succeed"),
            Some(crate::page_task_queue::PageModuleReactionTargetEffect::IgnoredStaleOwner),
            "replacement must discard the queued reaction without applying it to the new realm"
        );
        assert!(!vm.has_page_module_reaction_for_executor_test());
        assert!(
            !vm.dynamic_module_import_owner_is_current(document_owner),
            "navigation must retire the reaction's exact child execution context"
        );
    }

    #[test]
    fn dynamic_import_body_commits_module_before_task_end_document_open_reentry() {
        let mut vm = new_test_vm("https://dynamic-reentry.test/page.html");
        let original_owner = vm
            .current_main_document_task_owner()
            .expect("dynamic reentry test must have a main document owner");
        vm.eval(
            r#"
globalThis.__dynamicEvaluationGate = new Promise(resolve => {
  globalThis.__resolveDynamicEvaluationGate = resolve;
});
import('./dynamic.mjs').then(() => {
  document.open();
  document.write('<p id="replacement">replacement</p>');
  document.close();
});
'queued'
"#,
        )
        .expect("dynamic import should queue");
        assert!(
            matches!(
                vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
                MainNativeModuleSelectedTaskApplication::Applied(_)
            ),
            "dynamic import graph should start a root fetch through the selected body"
        );
        let target = vm
            .current_main_dynamic_import_graph_fetch_target(0)
            .expect("dynamic TLA import must expose its exact resolver target");
        let completion = dynamic_import_completion_with_source(
            0,
            "https://dynamic-reentry.test/dynamic.mjs",
            "export const value = await globalThis.__dynamicEvaluationGate;",
        );
        vm.complete_current_main_dynamic_import_graph_fetch_result(target, completion.result)
            .expect("dynamic TLA graph completion should start evaluation");
        assert_eq!(
            vm.current_main_document_task_owner(),
            Some(original_owner),
            "pending TLA must not replace the document before fulfillment"
        );

        vm.eval("globalThis.__resolveDynamicEvaluationGate(); 'resolved'")
            .expect("dynamic TLA gate should resolve");
        assert!(
            vm.has_page_module_reaction_for_executor_test(),
            "TLA fulfillment should queue the registered dynamic-import reaction"
        );
        assert_eq!(
            vm.run_page_module_reaction_body_for_test()
                .expect("dynamic import fulfillment body should run"),
            Some(
                crate::page_task_queue::PageModuleReactionTargetEffect::AppliedToCurrentOwner(
                    crate::page_task_queue::PageModuleReactionCurrentEffect::DynamicImportPromiseSettled,
                )
            ),
            "fulfilled reaction body should settle one exact import Promise"
        );
        assert_eq!(
            vm.current_main_document_task_owner(),
            Some(original_owner),
            "the body must leave user Promise reactions for selected-task completion"
        );
        vm.perform_script_task_checkpoint(None)
            .expect("selected-task checkpoint should run");
        assert_ne!(
            vm.current_main_document_task_owner(),
            Some(original_owner),
            "the task-end checkpoint should run the user reaction and replace the document"
        );
    }

    #[test]
    fn selected_dynamic_import_rejection_body_defers_user_reaction_to_task_end() {
        let mut vm = new_test_vm("https://dynamic-rejection-body.test/page.html");
        let context_host = vm._context_host.clone();
        let request = vm
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &vm.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let owner = context_host
                    .borrow()
                    .current_dynamic_module_import_owner(scope, None)
                    .expect("test ScriptVm must have a current dynamic-import owner");
                let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
                let promise = resolver.get_promise(scope);
                let global = scope.get_current_context().global(scope);
                assert_eq!(
                    global.set(
                        scope,
                        v8str(scope, "__selectedDynamicImportPromise").into(),
                        promise.into(),
                    ),
                    Some(true),
                );
                Ok(PendingDynamicModuleImport::new(
                    v8::Global::new(scope, scope.get_current_context()),
                    v8::Global::new(scope, resolver),
                    owner,
                    "./failed.mjs",
                    Url::parse("https://dynamic-rejection-body.test/page.html").expect("base URL"),
                    ModuleAttributesKey::empty(),
                    ModuleImportPhase::Evaluation,
                ))
            })
            .expect("selected rejection request should be created");
        vm.eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__selectedDynamicImportReactions = [];
__selectedDynamicImportPromise.catch(() => {
  __selectedDynamicImportReactions.push("rejected");
});
"attached"
"#,
        )
        .expect("rejection observer should attach without running a checkpoint");

        vm.reject_native_dynamic_module_import_with_error_selected_task_body(
            request,
            &ModuleLoadError::new(ModuleLoadStage::Fetch, "forced selected-task failure"),
        )
        .expect("selected rejection body should settle the exact Promise");
        assert_eq!(
            vm.eval_without_microtask_checkpoint_for_test(
                "__selectedDynamicImportReactions.join('|')",
            )
            .expect("body-only observation should succeed"),
            "",
            "the rejection body must not run the user Promise reaction"
        );

        vm.perform_script_task_checkpoint(None)
            .expect("selected-task checkpoint should run");
        assert_eq!(
            vm.eval_without_microtask_checkpoint_for_test(
                "__selectedDynamicImportReactions.join('|')",
            )
            .expect("task-end observation should succeed"),
            "rejected",
            "the task-end checkpoint must run the deferred user reaction"
        );
    }

    #[test]
    fn dynamic_import_owner_action_survives_main_document_open_for_rejection() {
        let mut vm = new_test_vm("https://dynamic-action-owner.test/page.html");
        let base_url = Url::parse("https://dynamic-action-owner.test/page.html").expect("base URL");
        let request = dynamic_import_request_in_vm(
            &mut vm,
            "./failed.mjs",
            base_url,
            ModuleImportPhase::Evaluation,
        );
        let retired_owner = request.owner();

        vm.eval("document.open(); 'replaced'")
            .expect("document.open should rotate the main document owner");
        assert!(
            vm.dynamic_module_import_owner_is_current(retired_owner),
            "document.open must preserve the request's execution-context owner"
        );

        let outcome = vm
            .run_main_dynamic_import_owner_action_with_body(
                FrameDocumentDynamicImportOwnerAction::fetch_failed(
                    request,
                    ModuleLoadError::new(ModuleLoadStage::Fetch, "forced stale fetch failure"),
                ),
                &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
            )
            .expect("stale dynamic import rejection should be consumed");

        assert!(outcome.made_progress());
        assert!(!outcome.stale_owner_was_dropped());
        assert!(
            outcome.dynamic_import_was_rejected(),
            "same-execution-context replacement must settle the original resolver"
        );
    }

    #[test]
    fn child_dynamic_import_resume_pending_job_reports_followup_progress() {
        let mut vm = new_test_vm("https://child-dynamic-resume.test/page.html");
        let job = {
            let _js_runtime = crate::JsRuntime::initialize();
            let mut isolate = v8::Isolate::new(Default::default());
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            let request = PendingDynamicModuleImport::new(
                v8::Global::new(scope, scope.get_current_context()),
                v8::Global::new(scope, resolver),
                DynamicModuleImportOwner::main_for_test(),
                "./dynamic-resume.js",
                Url::parse("https://child-dynamic-resume.test/page.html").expect("base URL"),
                ModuleAttributesKey::empty(),
                ModuleImportPhase::Evaluation,
            );
            NativeModuleGraphJob::dynamic_import(request)
        };

        let followup = vm.apply_child_dynamic_import_followup(
            FrameDocumentDynamicImportGraphAdvanceFollowup::ResumePendingJob(
                FrameDocumentDynamicImportPendingJobResume::new(job),
            ),
        );

        assert!(followup.made_progress());
        assert!(followup.dynamic_import_job_was_resumed());
        assert!(
            vm.document_runtime
                .take_next_native_dynamic_module_import()
                .is_some(),
            "ResumePendingJob should push the dynamic import job back to the runtime queue"
        );
    }

    #[test]
    fn child_dynamic_import_missing_joined_fetch_followup_reports_warning_progress() {
        let mut vm = new_test_vm("https://child-dynamic-missing-fetch.test/page.html");

        let followup = vm.apply_child_dynamic_import_followup(
            FrameDocumentDynamicImportGraphAdvanceFollowup::RecordMissingJoinedTerminalFetch(
                FrameDocumentDynamicImportMissingJoinedTerminalFetch::new(
                    FrameDocumentOwner::new(LocalWindowId(2), DocumentId(3)),
                    FrameRealmId(4),
                    55,
                ),
            ),
        );

        assert!(followup.made_progress());
        assert!(followup.terminal_warning_was_recorded());
        assert!(
            vm.runtime_observable_lifecycle_errors_for_testing()
                .iter()
                .any(|message| message
                    .contains("child dynamic import fetch finish 55 for FrameDocumentOwner")),
            "missing joined fetch follow-up should record a diagnostic warning"
        );
    }

    #[tokio::test]
    async fn child_dynamic_import_need_fetches_queues_waiting_owner_action() {
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
            Url::parse("https://parent-dynamic-owner.test/page.html").expect("page URL"),
            &loader,
        );

        vm.eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-owner.test/nested/frame.html">
    <script>parent.__childDynamicImportOwnerReady = true;<\/script>
  `;
  body.appendChild(frame);
})()
"#,
        )
        .expect("child dynamic import owner setup should evaluate");
        commit_child_document_and_run_parser_script_for_page_executor_test(
            &mut vm,
            "child dynamic import waiting setup frame",
        )
        .await;
        finish_child_document_after_parser_script_for_page_executor_test(
            &mut vm,
            "child dynamic import waiting setup frame",
        )
        .await;

        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child default execution context should be created");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist")
            .child_handle;
        let child_initiator_url = vm
            .child_browsing_context_module_request_initiator_url(child_handle)
            .expect("child document should expose a module request initiator URL");
        let base_url =
            Url::parse("https://child-dynamic-owner.test/nested/frame.html").expect("base URL");
        let fetch_request = NativeModuleGraphFetchRequest::new_for_test(
            base_url.join("dynamic-root.js").expect("dynamic URL"),
            base_url,
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
        );
        let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);
        let job = {
            let _js_runtime = crate::JsRuntime::initialize();
            let mut isolate = v8::Isolate::new(Default::default());
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            let request = PendingDynamicModuleImport::new(
                v8::Global::new(scope, scope.get_current_context()),
                v8::Global::new(scope, resolver),
                document_owner,
                "./dynamic-root.js",
                child_initiator_url,
                ModuleAttributesKey::empty(),
                ModuleImportPhase::Evaluation,
            );
            NativeModuleGraphJob::dynamic_import(request)
        };

        vm.continue_child_native_dynamic_module_import_after_tree_advance(
            job,
            NativeModuleGraphJobAdvance::NeedFetches(vec![fetch_request]),
        )
        .expect("child dynamic import waiting action should queue graph work");

        let task = vm
            .take_dynamic_import_owner_action_for_routing_test()
            .unwrap_or_else(|| {
                panic!(
                    "child dynamic import should queue a dynamic import owner action follow-up; warnings: {:?}",
                    vm.runtime_observable_lifecycle_errors_for_testing()
                )
            });
        assert!(
            matches!(
                task.action(),
                crate::frame_owner_model::FrameDocumentDynamicImportOwnerAction::Waiting { .. }
            ),
            "NeedFetches should queue Waiting as a later dynamic-import owner action"
        );
    }

    #[tokio::test]
    async fn native_dynamic_terminal_fanout_reports_child_ready_followup() {
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
            Url::parse("https://parent-dynamic-generic-ready.test/page.html").expect("page URL"),
            &loader,
        );

        vm.eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-generic-ready.test/nested/frame.html">
    <script>parent.__childDynamicImportGenericReadyOwnerReady = true;<\/script>
  `;
  body.appendChild(frame);
})()
"#,
        )
        .expect("child dynamic import generic ready setup should evaluate");
        commit_child_document_and_run_parser_script_for_page_executor_test(
            &mut vm,
            "child dynamic import generic ready setup frame",
        )
        .await;
        finish_child_document_after_parser_script_for_page_executor_test(
            &mut vm,
            "child dynamic import generic ready setup frame",
        )
        .await;

        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child default execution context should be created");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist")
            .child_handle;
        let child_initiator_url = vm
            .child_browsing_context_module_request_initiator_url(child_handle)
            .expect("child document should expose a module request initiator URL");
        let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);

        let job = {
            let _js_runtime = crate::JsRuntime::initialize();
            let mut isolate = v8::Isolate::new(Default::default());
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            let request = PendingDynamicModuleImport::new(
                v8::Global::new(scope, scope.get_current_context()),
                v8::Global::new(scope, resolver),
                document_owner,
                "./dynamic-generic-complete.js",
                child_initiator_url,
                ModuleAttributesKey::empty(),
                ModuleImportPhase::Evaluation,
            );
            NativeModuleGraphJob::dynamic_import(request)
        };
        let graph = ModuleGraphHandle {
            root_entry: ModuleEntryId::for_test(42),
            entries: vec![ModuleEntryId::for_test(42)],
        };
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady {
            job,
            graph,
        });

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("generic child dynamic import fanout should queue owner action");
        let followup = outcome.child_followup();
        assert_eq!(
            outcome.ready_import_count(),
            1,
            "fanout outcome should expose that a ready dynamic import was handled"
        );
        assert!(
            followup.dynamic_import_owner_action_was_queued(),
            "fanout should report the child dynamic-import owner action follow-up"
        );

        let task = vm
            .take_dynamic_import_owner_action_for_routing_test()
            .unwrap_or_else(|| {
                panic!(
                    "generic child dynamic import completion should queue a ready owner action; warnings: {:?}",
                    vm.runtime_observable_lifecycle_errors_for_testing()
                )
            });
        assert!(
            matches!(
                task.action(),
                crate::frame_owner_model::FrameDocumentDynamicImportOwnerAction::Ready(_)
            ),
            "generic child Complete fallback must queue Ready instead of settling inline"
        );
    }

    #[tokio::test]
    async fn native_dynamic_terminal_fanout_does_not_rebind_to_replacement_child_owner() {
        let mut vm = new_test_vm("https://parent-dynamic-stale-ready.test/page.html");
        vm.eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = "<script>parent.__staleDynamicImportChildReady = true;<\/script>";
  body.appendChild(frame);
})()
"#,
        )
        .expect("stale child dynamic import setup should evaluate");
        commit_child_document_and_run_parser_script_for_dynamic_import_test(
            &mut vm,
            "original stale dynamic import child",
        )
        .await;
        finish_child_document_after_parser_script_for_dynamic_import_test(
            &mut vm,
            "original stale dynamic import child",
        )
        .await;
        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("stale dynamic import test should create a child realm");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("stale dynamic import child realm record should exist")
            .child_handle;
        let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);
        let base_url =
            Url::parse("https://child-dynamic-stale-ready.test/nested/frame.html").expect("url");

        let job = {
            let _js_runtime = crate::JsRuntime::initialize();
            let mut isolate = v8::Isolate::new(Default::default());
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            let request = PendingDynamicModuleImport::new(
                v8::Global::new(scope, scope.get_current_context()),
                v8::Global::new(scope, resolver),
                document_owner,
                "./dynamic-stale-complete.js",
                base_url,
                ModuleAttributesKey::empty(),
                ModuleImportPhase::Evaluation,
            );
            NativeModuleGraphJob::dynamic_import(request)
        };
        vm.eval("document.querySelector('iframe').srcdoc = '<p>replacement child</p>'; 'queued'")
            .expect("replacement child navigation should queue");
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "child replacement must rotate the exact document owner before terminal fanout"
        );
        let (_captured_handle, retired_task_owner, _retired_realm_id) = document_owner
            .child_parts()
            .expect("test request must retain its child owner");
        let replacement_task_owner = vm
            ._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
            .expect("NavigationCommit should install the replacement document owner");
        assert_ne!(
            replacement_task_owner, retired_task_owner,
            "replacement must preserve the frame handle while changing document identity"
        );
        let graph = ModuleGraphHandle {
            root_entry: ModuleEntryId::for_test(77),
            entries: vec![ModuleEntryId::for_test(77)],
        };
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady {
            job,
            graph,
        });

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("retired child dynamic import ready fanout should be handled");
        let followup = outcome.child_followup();

        assert_eq!(
            outcome.ready_import_count(),
            1,
            "fanout outcome should expose that the retired ready import was consumed"
        );
        assert!(
            !followup.made_progress(),
            "stale work must not synthesize a child task-source wake"
        );
        assert!(
            !followup.dynamic_import_owner_action_was_queued(),
            "retired child work must not be rebound to the replacement document owner"
        );
        assert!(
            vm.runtime_observable_lifecycle_errors_for_testing()
                .iter()
                .any(|warning| {
                    warning.contains("child dynamic import continuation")
                        && warning.contains("owner is no longer current")
                }),
            "stale exact-owner drop should remain diagnosable without a synthetic wake"
        );
    }

    #[tokio::test]
    async fn child_dynamic_import_graph_error_queues_failed_owner_action() {
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
            Url::parse("https://parent-dynamic-failed.test/page.html").expect("page URL"),
            &loader,
        );

        vm.eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-failed.test/nested/frame.html">
    <script>parent.__childDynamicImportFailedOwnerReady = true;<\/script>
  `;
  body.appendChild(frame);
})()
"#,
        )
        .expect("child dynamic import failed owner setup should evaluate");
        commit_child_document_and_run_parser_script_for_page_executor_test(
            &mut vm,
            "child dynamic import failed setup frame",
        )
        .await;
        finish_child_document_after_parser_script_for_page_executor_test(
            &mut vm,
            "child dynamic import failed setup frame",
        )
        .await;

        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child default execution context should be created");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist")
            .child_handle;
        let child_initiator_url = vm
            .child_browsing_context_module_request_initiator_url(child_handle)
            .expect("child document should expose a module request initiator URL");
        let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);

        let job = {
            let _js_runtime = crate::JsRuntime::initialize();
            let mut isolate = v8::Isolate::new(Default::default());
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            let request = PendingDynamicModuleImport::new(
                v8::Global::new(scope, scope.get_current_context()),
                v8::Global::new(scope, resolver),
                document_owner,
                "./dynamic-failed.js",
                child_initiator_url,
                ModuleAttributesKey::empty(),
                ModuleImportPhase::Evaluation,
            );
            NativeModuleGraphJob::dynamic_import(request)
        };
        let error = ModuleLoadError::new(ModuleLoadStage::Resolve, "forced graph failure");

        let followup = vm
            .enqueue_child_dynamic_import_graph_advance_failed_owner_action(job, error)
            .expect("child dynamic import failed action should queue graph failure work");
        assert!(
            followup.dynamic_import_owner_action_was_queued(),
            "graph failure should report the queued child dynamic-import owner action"
        );

        let task = vm
            .take_dynamic_import_owner_action_for_routing_test()
            .unwrap_or_else(|| {
                panic!(
                    "child dynamic import should queue a failed owner action follow-up; warnings: {:?}",
                    vm.runtime_observable_lifecycle_errors_for_testing()
                )
            });
        assert!(
            matches!(
                task.action(),
                crate::frame_owner_model::FrameDocumentDynamicImportOwnerAction::Reject(_)
            ),
            "graph errors should queue Reject as a later dynamic-import owner action"
        );
    }

    #[test]
    fn native_dynamic_terminal_fanout_reports_scheduled_fetch_effects() {
        let mut vm = new_test_vm("https://app-dynamic-schedule.test/page.html");
        let base_url = Url::parse("https://app-dynamic-schedule.test/page.html").expect("base URL");
        let fetch_request = NativeModuleGraphFetchRequest::new_for_test(
            base_url
                .join("dynamic-dependency.js")
                .expect("dynamic dependency URL"),
            base_url.clone(),
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
        );
        let job = dynamic_import_job_in_vm(
            &mut vm,
            "./dynamic-dependency.js",
            base_url,
            ModuleImportPhase::Evaluation,
        );
        let scheduled = vm
            .document_runtime
            .suspend_native_dynamic_module_import_fetches(
                vec![fetch_request],
                Vec::new(),
                job,
                vec![None],
            );
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.extend_scheduled_dynamic_import_fetches(scheduled);

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("native dynamic import fanout should schedule fetches");

        assert_eq!(
            outcome.scheduled_dynamic_import_fetch_count(),
            1,
            "fanout outcome should expose concrete fetch scheduling progress"
        );
        assert!(
            !outcome.child_followup().made_progress(),
            "main dynamic fetch scheduling must not synthesize child frame follow-up work"
        );
    }

    #[test]
    fn native_dynamic_waiting_without_joined_clients_reports_job_resume_effect() {
        let mut vm = new_test_vm("https://app-dynamic-resume.test/page.html");
        let base_url = Url::parse("https://app-dynamic-resume.test/page.html").expect("base URL");
        let job = dynamic_import_job_in_vm(
            &mut vm,
            "./dynamic-resume.js",
            base_url,
            ModuleImportPhase::Evaluation,
        );

        let outcome = vm
            .continue_main_native_dynamic_module_import_after_tree_advance_with_body(
                job,
                NativeModuleGraphJobAdvance::WaitingForFetches,
                &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
            )
            .expect("main dynamic waiting advance should resume the pending job");

        assert_eq!(
            outcome.dynamic_import_job_resumed_count(),
            1,
            "main WaitingForFetches without joined clients should expose job-resume progress"
        );
        assert!(
            !outcome.child_followup().made_progress(),
            "main dynamic job resume must not synthesize child frame follow-up work"
        );
        assert!(
            vm.document_runtime
                .take_next_native_dynamic_module_import()
                .is_some(),
            "the resumed job should be available on the runtime dynamic-import queue"
        );
    }

    #[test]
    fn native_dynamic_terminal_fanout_reports_failed_fetch_rejection_effect() {
        let mut vm = new_test_vm("https://app-dynamic-fetch-failure.test/page.html");
        let base_url =
            Url::parse("https://app-dynamic-fetch-failure.test/page.html").expect("base URL");
        let request = dynamic_import_request_in_vm(
            &mut vm,
            "./dynamic-fetch-failure.js",
            base_url,
            ModuleImportPhase::Evaluation,
        );
        let failure = DynamicModuleFetchFailure::for_test(
            request,
            ModuleLoadError::new(ModuleLoadStage::Fetch, "forced dynamic fetch failure"),
        );
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_failed_fetch(failure);

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("failed dynamic fetch fanout should reject the import");

        assert_eq!(outcome.failed_fetch_rejected_count(), 1);
        assert!(
            !outcome.child_followup().made_progress(),
            "main dynamic fetch failure must not synthesize child frame follow-up work"
        );
    }

    #[test]
    fn native_dynamic_terminal_fanout_reports_graph_advance_rejection_effect() {
        let mut vm = new_test_vm("https://app-dynamic-graph-failure.test/page.html");
        let base_url =
            Url::parse("https://app-dynamic-graph-failure.test/page.html").expect("base URL");
        let job = dynamic_import_job_in_vm(
            &mut vm,
            "./dynamic-graph-failure.js",
            base_url,
            ModuleImportPhase::Evaluation,
        );
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_graph_advance_failure(
            job,
            ModuleLoadError::new(ModuleLoadStage::Resolve, "forced dynamic graph failure"),
        );

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("graph failure fanout should reject the import");

        assert_eq!(outcome.graph_advance_failure_handled_count(), 1);
        assert!(
            !outcome.child_followup().made_progress(),
            "main dynamic graph failure must not synthesize child frame follow-up work"
        );
    }

    #[test]
    fn native_dynamic_terminal_fanout_reports_source_import_rejection_effect() {
        let mut vm = new_test_vm("https://app-dynamic-source.test/page.html");
        let base_url = Url::parse("https://app-dynamic-source.test/page.html").expect("base URL");
        let module_key = ModuleMapKey::java_script(
            base_url
                .join("dynamic-source.js")
                .expect("dynamic source URL"),
        );
        let root_entry = vm.document_runtime.insert_native_module_source(
            module_key,
            ModuleSource::text("export const value = 1;".to_owned()),
        );
        let job = dynamic_import_job_in_vm(
            &mut vm,
            "./dynamic-source.js",
            base_url,
            ModuleImportPhase::Source,
        );
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady {
            job,
            graph: ModuleGraphHandle {
                root_entry,
                entries: vec![root_entry],
            },
        });

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("source-phase dynamic import fanout should reject non-Wasm source import");

        assert_eq!(outcome.ready_import_count(), 1);
        assert_eq!(outcome.source_import_rejected_count(), 1);
        assert!(
            !outcome.child_followup().made_progress(),
            "main source-phase dynamic import rejection must not synthesize child follow-up work"
        );
    }

    #[test]
    fn native_dynamic_terminal_fanout_reports_evaluation_rejection_effect() {
        let mut vm = new_test_vm("https://app-dynamic-eval.test/page.html");
        let base_url = Url::parse("https://app-dynamic-eval.test/page.html").expect("base URL");
        let module_key = ModuleMapKey::java_script(
            base_url
                .join("dynamic-eval.js")
                .expect("dynamic evaluation URL"),
        );
        let root_entry = vm.document_runtime.insert_native_module_source(
            module_key,
            ModuleSource::text("export const value = 1;".to_owned()),
        );
        let job = dynamic_import_job_in_vm(
            &mut vm,
            "./dynamic-eval.js",
            base_url,
            ModuleImportPhase::Evaluation,
        );
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady {
            job,
            graph: ModuleGraphHandle {
                root_entry,
                entries: vec![root_entry],
            },
        });

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("evaluation dynamic import fanout should reject a non-ready root");

        assert_eq!(outcome.ready_import_count(), 1);
        assert_eq!(outcome.evaluation_import_rejected_count(), 1);
        assert!(
            !outcome.child_followup().made_progress(),
            "main evaluation dynamic import rejection must not synthesize child follow-up work"
        );
    }

    #[test]
    fn native_dynamic_terminal_fanout_reports_unexpected_complete_warning_followup() {
        let mut vm = new_test_vm("https://app-dynamic-warning.test/page.html");
        let fanout = NativeDynamicModuleTerminalFanout::from_owner_advance(
            DynamicModuleFetchOwnerAdvance::RestoredAfterUnexpectedComplete,
        );

        let outcome = vm
            .handle_dynamic_module_terminal_fanout_for_owner(fanout)
            .expect("native dynamic import fanout should record unexpected-complete warning");
        let followup = outcome.child_followup();

        assert!(followup.made_progress());
        assert!(followup.terminal_warning_was_recorded());
        assert!(
            !followup.dynamic_import_owner_action_was_queued(),
            "warning-only fanout must not enqueue a dynamic-import owner action"
        );
    }

    #[test]
    fn preserve_current_exception_keeps_v8_exception_message() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let try_catch = pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();

        let message =
            v8::String::new(&scope, "original syntax failure").expect("v8 string allocation");
        let exception = v8::Exception::syntax_error(&scope, message);
        scope.throw_exception(exception);
        assert!(preserve_current_v8_module_exception(&mut scope).is_none());

        let exception = scope
            .exception()
            .expect("original exception should still be pending");
        let message = exception
            .to_string(&scope)
            .expect("exception should stringify")
            .to_rust_string_lossy(&scope);
        assert!(message.contains("original syntax failure"));
    }

    #[test]
    fn wasm_compile_with_options_preserves_v8_compile_exception() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let try_catch = pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        let truncated_wasm_module = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01];

        let module = v8::WasmModuleObject::compile_with_options(
            &scope,
            &truncated_wasm_module,
            v8::WasmCompileOptions {
                js_string_builtins: true,
                imported_string_constants_module: None,
            },
        );

        assert!(module.is_none());
        let exception = scope
            .exception()
            .expect("wasm compile exception should be rethrown to caller scope");
        let message = exception
            .to_string(&scope)
            .expect("wasm compile exception should stringify")
            .to_rust_string_lossy(&scope);
        assert!(message.contains("CompileError"), "{message}");
        assert!(message.contains("WebAssembly.Module"), "{message}");
        assert!(!message.contains("unknown wasm compile exception"));
    }

    #[test]
    fn wasm_compile_with_options_does_not_use_page_webassembly_constructor() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let patch_source = v8str(
            scope,
            r#"
            WebAssembly.Module = function() {
                throw new Error("patched WebAssembly.Module should not be used");
            };
            "#,
        );
        let patch_script =
            v8::Script::compile(scope, patch_source, None).expect("patch script should compile");
        patch_script.run(scope).expect("patch script should run");

        let try_catch = pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        let truncated_wasm_module = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01];

        let module = v8::WasmModuleObject::compile_with_options(
            &scope,
            &truncated_wasm_module,
            v8::WasmCompileOptions {
                js_string_builtins: true,
                imported_string_constants_module: None,
            },
        );

        assert!(module.is_none());
        let exception = scope
            .exception()
            .expect("wasm compile exception should be pending");
        let message = exception
            .to_string(&scope)
            .expect("wasm compile exception should stringify")
            .to_rust_string_lossy(&scope);
        assert!(message.contains("CompileError"), "{message}");
        assert!(
            !message.contains("patched WebAssembly.Module should not be used"),
            "{message}"
        );
    }
}
