//! Unified frame owner model.
//!
//! This module owns the exact frame, `WindowProxy`, `LocalWindow`, `Document`,
//! realm, lifecycle, and currentness facts shared by the main frame and child
//! frames. It also defines the typed owner actions projected from those facts.
//!
//! It does not create worker-like loops for individual frames or own V8
//! execution. Main and child frames stay on the renderer owner loop; workers
//! retain their separate isolate and loop model.
mod classic_script_tasks;
mod frame_task_sources;
mod ids;
mod lifecycle_blockers;
mod lifecycle_tasks;
mod load_delivery_tasks;
mod load_event_gate;
mod module_clients;
mod module_graph;
mod navigation_tasks;
mod records;
mod script_events;
mod store;
mod task_sources;

pub(crate) use classic_script_tasks::{
    FrameClassicDocumentScriptExecutionAction, FrameClassicDocumentScriptExecutionStart,
    FrameDocumentClassicCompletionFinishAction, FrameDocumentClassicCompletionFollowup,
    FrameDocumentClassicCompletionLifecycleFollowup,
    FrameDocumentClassicCompletionScriptEventAction,
    FrameDocumentClassicCompletionScriptEventFollowup,
    FrameDocumentClassicDeferredCompletionApplication, FrameDocumentClassicExecutionFollowup,
    FrameDocumentClassicParserResumeApplication, FrameDocumentClassicParserResumeCompletionAction,
    FrameDocumentClassicParserResumeSkipReason, FrameDocumentClassicPrepareApplication,
    FrameDocumentClassicPrepareDropReason, FrameDocumentClassicPrepareFollowup,
    FrameDocumentClassicScriptBeginExecutionAction, FrameDocumentClassicScriptCompletionAction,
    FrameDocumentClassicScriptCompletionTarget, FrameDocumentClassicScriptExecutionFinish,
    FrameDocumentClassicScriptReadyTarget, FrameDocumentClassicScriptScheduling,
    FrameDocumentClassicScriptSourceFailureAction, FrameDocumentClassicScriptSourceFailureTarget,
    FrameDocumentClassicScriptSourceLoadClient,
    FrameDocumentClassicScriptSourceLoadCompletionAction,
    FrameDocumentClassicScriptSourceLoadOwner, FrameDocumentClassicScriptSourceLoadRequest,
    FrameDocumentClassicScriptSourceLoadStartOutcome, FrameDocumentClassicScriptSourceLoadTask,
    FrameDocumentClassicScriptTarget, FrameDocumentClassicSourceFailureReportApplication,
    FrameDocumentClassicSourceFailureReportFollowup,
    FrameDocumentClassicSourceFailureReportSkipReason,
    frame_document_classic_script_begin_execution_action,
    frame_document_classic_script_source_load_client_action,
    frame_document_classic_script_source_load_completion_action,
    frame_document_classic_script_source_load_request_action,
    frame_script_job_kind_from_parser_classic_ready_kind,
};
pub(crate) use frame_task_sources::{
    FrameDocumentDynamicClassicExecutionFollowup, FrameDocumentDynamicClassicPrepareFollowup,
    FrameDocumentDynamicClassicPrepareSkipReason, FrameDocumentDynamicClassicScriptExecutionAction,
    FrameDocumentExternalClassicExecutionFollowup, FrameDocumentExternalClassicExecutionResult,
    FrameDocumentExternalClassicPostExecutionAction, FrameDocumentExternalClassicPrepareFollowup,
    FrameDocumentExternalClassicPrepareSkipReason, FrameDocumentExternalClassicScriptExecution,
    FrameDocumentExternalClassicScriptExecutionAction, FrameDocumentJavascriptUrlCompletion,
    FrameDocumentJavascriptUrlExecutionFollowup, FrameDocumentJavascriptUrlExecutionResult,
    FrameDocumentJavascriptUrlPostExecutionAction,
    FrameDocumentJavascriptUrlPostExecutionApplication, FrameDocumentJavascriptUrlPrepareFollowup,
    FrameDocumentJavascriptUrlPrepareSkipReason, FrameDocumentJavascriptUrlScriptExecutionAction,
    FrameDocumentJavascriptUrlScriptExecutionTarget, FrameDocumentRealmBoundScriptWork,
    FrameDocumentScriptExecutionFollowup, FrameDocumentScriptExecutionResult,
    FrameDocumentScriptExecutionWork, FrameDocumentScriptPrepareFollowup,
    FrameDocumentScriptReadyTaskWork, FrameDocumentScriptWorkAdmission,
    FrameDocumentUnboundScriptWork, PendingChildDocumentScriptExecutionWork,
    PendingChildDynamicDocumentScript, PendingChildExternalClassicDocumentScript,
    PendingChildJavascriptUrlDocumentScript,
};
pub(crate) use lifecycle_tasks::{
    ChildDocumentAsyncClassicScriptLoadDelay, DocumentLinkEventOwner,
    FrameDocumentCompleteLifecycleAction, FrameDocumentDomContentLoadedLifecycleAction,
    FrameDocumentImageLoadEventBinding, FrameDocumentInteractiveLifecycleAction,
    FrameDocumentLifecycleAction, FrameDocumentLifecycleTaskEffect,
    FrameDocumentMediaLoadDelayBinding, FrameDocumentUnloadLifecycleAction,
    MainDocumentCompleteLifecycleAction, MainDocumentDomContentLoadedLifecycleAction,
    MainDocumentImageLoadDelayBinding, MainDocumentInteractiveLifecycleAction,
    MainDocumentMediaLoadDelayBinding, MainDocumentScriptLoadDelayKind,
    MainDocumentScriptLoadDelayLease, MainDocumentScriptLoadDelayRelease,
    MainDocumentStyleLoadEventBinding, StylesheetSubresourceLoadDelayBinding,
};
pub(crate) use load_delivery_tasks::{
    FrameDocumentLoadDeliveryAction, FrameDocumentLoadDeliveryAdmission,
    FrameDocumentLoadDeliveryPhase, FrameDocumentLoadDeliveryProgress,
    FrameDocumentLoadDeliveryTask,
};
pub(crate) use module_clients::ChildDocumentModuleFetchTarget;
pub(crate) use module_clients::FrameDocumentDynamicImportTerminalWork;
pub(crate) use module_clients::FrameDocumentModuleClientEntryId;
pub(crate) use module_clients::FrameDocumentModuleClientId;
pub(crate) use module_clients::FrameDocumentModuleClientRegistration;
pub(crate) use module_clients::FrameDocumentModuleClientReservation;
pub(crate) use module_clients::FrameDocumentModuleDependencyFetchStartOutcome;
pub(crate) use module_clients::FrameDocumentModuleDependencyFetchTask;
pub(crate) use module_clients::FrameDocumentModuleDependencyTerminalWork;
pub(crate) use module_clients::FrameDocumentModuleFetchClientStart;
pub(crate) use module_clients::FrameDocumentModuleFetchDisposition;
pub(crate) use module_clients::FrameDocumentModuleFetchTerminalResult;
pub(crate) use module_clients::FrameDocumentModuleScriptTerminalBatchTask;
pub(crate) use module_clients::FrameDocumentModuleScriptTerminalTask;
pub(crate) use module_clients::FrameDocumentModuleScriptTerminalWork;
pub(crate) use module_clients::FrameDocumentModuleTerminalBatch;
pub(crate) use module_clients::FrameDocumentModuleTerminalWarning;
pub(crate) use module_clients::FrameDocumentModuleTerminalWarningRecord;
pub(crate) use module_clients::FrameDocumentModulepreloadEventAction;
pub(crate) use module_clients::FrameDocumentModulepreloadEventActionHooks;
pub(crate) use module_clients::FrameDocumentModulepreloadEventActionRunner;
pub(crate) use module_clients::FrameDocumentModulepreloadFetchTask;
pub(crate) use module_clients::FrameDocumentModulepreloadLinkClient;
pub(crate) use module_clients::FrameDocumentModulepreloadMaterializedWork;
pub(crate) use module_clients::FrameDocumentModulepreloadTerminalOutcome;
pub(crate) use module_clients::FrameDocumentModulepreloadTerminalWork;
pub(crate) use module_clients::FrameDocumentModulepreloadWorkAwaitingRealm;
pub(crate) use module_clients::FrameDocumentParserModuleRootFetchStart;
pub(crate) use module_clients::FrameDocumentParserModuleRootStartKind;
pub(crate) use module_clients::FrameDocumentParserModuleRootStartTask;
pub(crate) use module_clients::FrameDocumentParserRootModuleClient;
pub(crate) use module_clients::FrameDocumentParserRootTerminalClient;
pub(crate) use module_clients::FrameDocumentParserRootTerminalWork;
pub(crate) use module_clients::FrameDocumentStaticDependencyModuleClient;
#[cfg(test)]
pub(crate) use module_graph::ChildDynamicModuleFetchAction;
#[cfg(test)]
pub(crate) use module_graph::FrameDocumentDynamicImportPendingJobResume;
#[cfg(test)]
pub(crate) use module_graph::FrameDocumentDynamicImportUnexpectedCompleteWarning;
#[cfg(test)]
pub(crate) use module_graph::FrameDocumentModulepreloadStartAction;
pub(crate) use module_graph::{
    ChildDocumentModulatorStore, ChildDynamicModuleCompletedFetchRestoreAction,
    ChildDynamicModuleInflightFetch, ChildDynamicModuleOwnerFetchCompletionSettlementAction,
    ChildDynamicModuleOwnerFetchWithoutNetworkSettlementAction,
    FrameDocumentDynamicImportEvaluationReadyAction,
    FrameDocumentDynamicImportEvaluationReadyResult,
    FrameDocumentDynamicImportGraphAdvanceFollowup,
    FrameDocumentDynamicImportJoinedFetchRestoreResult,
    FrameDocumentDynamicImportMissingJoinedTerminalClient,
    FrameDocumentDynamicImportMissingJoinedTerminalFetch, FrameDocumentDynamicImportOwnerAction,
    FrameDocumentDynamicImportOwnerActionDiagnostic, FrameDocumentDynamicImportOwnerActionHooks,
    FrameDocumentDynamicImportOwnerActionQueueHooks,
    FrameDocumentDynamicImportOwnerActionQueueRequest,
    FrameDocumentDynamicImportOwnerActionQueueRunner,
    FrameDocumentDynamicImportOwnerActionQueueTrace, FrameDocumentDynamicImportOwnerActionRunner,
    FrameDocumentDynamicImportOwnerFetchSettlementResult,
    FrameDocumentDynamicImportOwnerTerminalRestoreAction,
    FrameDocumentDynamicImportQueueTaskOwnerResult, FrameDocumentDynamicImportRejectAction,
    FrameDocumentDynamicImportRejectResult, FrameDocumentDynamicImportSourceReadyAction,
    FrameDocumentDynamicImportSourceReadyResult, FrameDocumentDynamicImportSourceWasmRecordLookup,
    FrameDocumentDynamicImportTerminalClientAction,
    FrameDocumentDynamicImportTerminalClientFinishResult,
    FrameDocumentDynamicImportTerminalOutcome, FrameDocumentDynamicImportTerminalPreparedAction,
    FrameDocumentDynamicImportWaitingFetchScheduleAction,
    FrameDocumentDynamicImportWaitingFetchScheduleResult,
    FrameDocumentModuleScriptGraphNotification, FrameDocumentModuleScriptTerminalFollowup,
    FrameDocumentModuleScriptTerminalHooks, FrameDocumentModuleScriptTerminalOutcome,
    FrameDocumentModuleScriptTerminalRunner, FrameDocumentModuleTerminalQueueFollowup,
    FrameDocumentModulepreloadFetchCompletionAction,
    FrameDocumentModulepreloadFetchCompletionHooks,
    FrameDocumentModulepreloadFetchCompletionRunner, FrameDocumentModulepreloadFetchFinishResult,
    FrameDocumentModulepreloadStartActionHooks, FrameDocumentModulepreloadStartActionRunner,
    FrameDocumentModulepreloadStartOutcome,
    FrameDocumentParserModuleTreeAdvanceDependencyFetchResult,
    FrameDocumentParserModuleTreeAdvanceFailureTrace, FrameDocumentParserModuleTreeAdvanceHooks,
    FrameDocumentParserModuleTreeAdvanceRunner, frame_document_parser_module_tree_advance_action,
    module_script_graph_failed_work_from_root_client,
    module_script_graph_failed_work_from_tree_job, trace_child_module_dependency_failure,
    trace_child_parser_module_root_failure,
};
pub(crate) use navigation_tasks::{
    ChildDocumentNavigationFetchTarget, FrameLaneNavigationCommitTask,
    FrameNavigationCommitReservationResult,
};
#[cfg(test)]
pub(crate) use records::FrameFunctionConstructorSource;
pub(crate) use records::{
    ChildFrameOwnerSnapshot, DocumentCreationKind, DocumentId, DocumentLoadDelayTokenId,
    FrameDocumentDescendantLoadCompletion, FrameDocumentDescendantLoadParent,
    FrameDocumentLoadDispatchFinish, FrameDocumentLocalWindowTransition,
    FrameDocumentNavigationLoadBinding, FrameDocumentOwner, FrameDocumentOwnerTransition,
    FrameDocumentTaskOwner, FrameDocumentTaskRealmCurrentness, FrameId,
    FrameLocalWindowOwnerTransition, FrameOwnerDocumentTarget, FrameRealmId,
    FrameRealmMaterializationRequest, FrameRequestId, FrameRequestKind, FrameSchedulerLaneId,
    FrameScriptJob, FrameScriptJobKind, FrameScriptSource, LocalWindowId,
    MainDocumentLoadCompletionState, MainDocumentOwnerTransition,
};
pub(crate) use script_events::{
    FrameDocumentScriptElementEvent, FrameDocumentScriptElementEventKind,
};
pub(crate) use store::FrameOwnerStore;
#[cfg(test)]
pub(crate) use task_sources::ChildFrameSemanticTurnKind;
