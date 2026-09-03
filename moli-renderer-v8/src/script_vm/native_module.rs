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
mod document_scripts;
mod module_fetch;
mod module_graph;
mod module_reactions;

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
#[webapi(plain, data_properties, enumerable)]
struct NativeDynamicModuleReactionDataDeclaration<'scope> {
    reaction_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct NativeModuleScriptReactionDataDeclaration<'scope> {
    module_script_reaction_id: v8::Local<'scope, v8::BigInt>,
    scheduler_lane_id: v8::Local<'scope, v8::BigInt>,
    local_window_id: v8::Local<'scope, v8::BigInt>,
    document_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
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

fn synthetic_text_module_evaluation_steps<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let Some(record) =
        crate::module_runtime::SyntheticTextModuleSource::for_module(context, module)
    else {
        return throw_synthetic_module_error(scope, "synthetic module source is not available");
    };
    let source = record.source();
    match record.key().kind() {
        ModuleKind::Json => evaluate_json_synthetic_module(scope, module, source),
        ModuleKind::Css => {
            evaluate_css_synthetic_module(scope, module, record.key().url().as_str(), source)
        }
        _ => throw_synthetic_module_error(scope, "unexpected synthetic text module kind"),
    }
}
fn wasm_synthetic_module_evaluation_steps<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return throw_synthetic_module_error(scope, "synthetic module host is not available");
    };
    let Some(wasm_record) = (unsafe { &*host_ptr }).native_module_wasm_record_for(module) else {
        return throw_synthetic_module_error(
            scope,
            "WebAssembly synthetic module record is not available",
        );
    };
    evaluate_wasm_synthetic_module(scope, module, &wasm_record, |scope, import| {
        wasm_import_value(scope, module, import)
    })
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

fn native_module_instantiate_load_error(
    message: String,
    caught_error_constructor: Option<ScriptErrorConstructorKind>,
    has_wasm_entry: bool,
) -> ModuleLoadError {
    let fallback_constructor = if has_wasm_entry {
        ScriptErrorConstructorKind::WebAssemblyLinkError
    } else {
        ScriptErrorConstructorKind::SyntaxError
    };
    ModuleLoadError::new(ModuleLoadStage::Instantiate, message)
        .with_error_constructor(caught_error_constructor.unwrap_or(fallback_constructor))
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
        ScriptErrorConstructorKind::TypeError => Some(v8::Exception::type_error(scope, message)),
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
        ScriptErrorConstructorKind::TypeError => v8::Exception::type_error(scope, empty),
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
mod tests;
