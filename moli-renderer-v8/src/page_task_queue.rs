mod broadcast_channel_delivery;
mod child_frame_task;
mod child_module_dependency_fetch_start;
mod child_module_script_terminal;
mod child_modulepreload_event_action;
mod child_navigation_commit;
mod child_realm_materialization;
mod dedicated_worker_client_event;
mod dom_manipulation;
mod dynamic_import_owner_action;
mod element_toggle_event;
mod file_entry_file_callback;
mod file_reading;
mod hash_change_delivery;
mod history_traversal;
mod image_load_event;
mod indexed_db_task;
mod internal_loading;
mod main_document_post_parse;
mod main_document_runtime;
mod main_document_task_owner;
mod main_native_module_task;
mod main_parser_continuation;
mod media_element_event;
mod message_port_delivery;
mod misc_platform_api;
mod module_reaction;
mod modulepreload_start;
mod navigation_and_traversal;
mod navigation_api_task;
mod networking;
mod opfs_task;
mod owner_sources;
mod parse_time;
mod parser_async_module_admission;
mod parser_owned_module_continuation;
mod popup_load_event;
mod post_domcontentloaded_runtime;
mod post_parse_owner_work;
mod rendering_update;
mod resource_completions;
mod senders;
mod service_worker_client_message;
mod service_worker_internal;
mod service_worker_tasks;
mod shared_worker_client_event;
mod storage_event_delivery;
mod stylesheet_task;
mod tasks;
mod text_track_default_mode;
mod text_track_load;
mod user_interaction;
mod v8_foreground_task;
mod view_transition_update;
mod webcrypto_task;
mod websocket_event;
mod window_document_task_source;
mod window_message;
mod worker_host_bridge;

use moli_owner_queue::OwnerTaskSource;

use crate::document_script_scheduler::ParseTimeDocumentScriptEvent;
use crate::native_bridge::WindowDocumentTaskTarget;
use crate::planning::SharedScriptSourceLoad;
use crate::runtime::RendererDocumentToken;

/// PageVm namespace plus the exact Window/Document captured by a queued task.
///
/// Task-source families keep their own typed ids and payloads, but they should
/// not each redefine this authorization identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWindowDocumentTaskOwner {
    root_document: RendererDocumentToken,
    target: WindowDocumentTaskTarget,
}

impl RendererPageWindowDocumentTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: WindowDocumentTaskTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> WindowDocumentTaskTarget {
        self.target
    }
}

/// Scheduler envelope for a concrete exact Window/Document task whose mutable
/// V8/DOM payload remains in the creating `JsContextHost` ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWindowDocumentTask<I, K> {
    owner: RendererPageWindowDocumentTaskOwner,
    task_id: I,
    kind: K,
}

impl<I: Copy, K: Copy> RendererPageWindowDocumentTask<I, K> {
    pub(crate) const fn new(
        owner: RendererPageWindowDocumentTaskOwner,
        task_id: I,
        kind: K,
    ) -> Self {
        Self {
            owner,
            task_id,
            kind,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageWindowDocumentTaskOwner {
        self.owner
    }

    pub(crate) const fn task_id(&self) -> I {
        self.task_id
    }

    pub(crate) const fn kind(&self) -> K {
        self.kind
    }
}

/// Common settlement of one exact Window/Document event task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageWindowDocumentTaskTargetEffect {
    DispatchedToCurrentOwner,
    CurrentOwnerHadNoEventTarget,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageWindowDocumentTaskOwner>,
    },
}

/// Common owner-turn fact emitted after an exact Window/Document task settles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageWindowDocumentTaskTurnAction<I, K> {
    pub(crate) owner: RendererPageWindowDocumentTaskOwner,
    pub(crate) task_id: I,
    pub(crate) kind: K,
    pub(crate) target_effect: PageWindowDocumentTaskTargetEffect,
}

impl<I, K> PageWindowDocumentTaskTurnAction<I, K> {}

pub(crate) use self::broadcast_channel_delivery::{
    PageBroadcastChannelDeliveryDocumentEffect, PageBroadcastChannelDeliveryTurnAction,
    PageBroadcastChannelDeliveryTurnOutcome, RendererPageBroadcastChannelDeliveryOwner,
    RendererPageBroadcastChannelDeliveryProducer, RendererPageBroadcastChannelDeliverySender,
    RendererPageBroadcastChannelDeliveryTask,
};
pub(crate) use self::child_frame_task::{
    PageChildClassicScriptSourceLoadTargetEffect, PageChildClassicScriptSourceLoadTurnAction,
    PageChildClassicScriptSourceLoadTurnOutcome, PageChildDocumentLifecycleTargetEffect,
    PageChildDocumentLifecycleTurnAction, PageChildDocumentLifecycleTurnOutcome,
    PageChildDocumentScriptReadyTargetEffect, PageChildDocumentScriptReadyTurnAction,
    PageChildDocumentScriptReadyTurnOutcome, PageChildHostLoadTargetEffect,
    PageChildHostLoadTurnAction, PageChildHostLoadTurnOutcome,
    PageChildParserModuleRootStartTargetEffect, PageChildParserModuleRootStartTurnAction,
    PageChildParserModuleRootStartTurnOutcome, RendererPageChildClassicScriptSourceLoadTarget,
    RendererPageChildDocumentLifecycleTarget, RendererPageChildDocumentScriptReadyTarget,
    RendererPageChildDocumentScriptReadyTaskId, RendererPageChildFrameTask,
    RendererPageChildFrameTaskOwner, RendererPageChildFrameTaskSender,
    RendererPageChildFrameTaskTarget, RendererPageChildHostLoadTarget,
    RendererPageChildParserModuleRootStartTarget,
};
pub(crate) use self::child_module_dependency_fetch_start::{
    PageChildModuleDependencyFetchStartTargetEffect, PageChildModuleDependencyFetchStartTurnAction,
    PageChildModuleDependencyFetchStartTurnOutcome,
    RendererPageChildModuleDependencyFetchStartEnqueue,
    RendererPageChildModuleDependencyFetchStartOwner,
    RendererPageChildModuleDependencyFetchStartSender,
    RendererPageChildModuleDependencyFetchStartTask,
};
pub(crate) use self::child_module_script_terminal::{
    PageChildModuleScriptTerminalTargetEffect, PageChildModuleScriptTerminalTurnAction,
    PageChildModuleScriptTerminalTurnOutcome, RendererPageChildModuleScriptTerminalOwner,
    RendererPageChildModuleScriptTerminalSender, RendererPageChildModuleScriptTerminalTask,
};
pub(crate) use self::child_modulepreload_event_action::{
    PageChildModulepreloadEventActionTargetEffect, PageChildModulepreloadEventActionTurnAction,
    PageChildModulepreloadEventActionTurnOutcome, RendererPageChildModulepreloadEventActionOwner,
    RendererPageChildModulepreloadEventActionSender, RendererPageChildModulepreloadEventActionTask,
};
pub(crate) use self::child_navigation_commit::{
    PageChildNavigationCommitTargetEffect, PageChildNavigationCommitTurnAction,
    PageChildNavigationCommitTurnOutcome, RendererPageChildNavigationCommitOwner,
    RendererPageChildNavigationCommitSender, RendererPageChildNavigationCommitTask,
};
pub(crate) use self::child_realm_materialization::{
    PageChildRealmMaterializationTargetEffect, PageChildRealmMaterializationTurnAction,
    PageChildRealmMaterializationTurnOutcome, RendererPageChildRealmMaterializationTarget,
};
#[cfg(test)]
pub(crate) use self::dedicated_worker_client_event::RendererDedicatedWorkerClientEventKind;
pub(crate) use self::dedicated_worker_client_event::{
    PageDedicatedWorkerClientEventTargetEffect, PageDedicatedWorkerClientEventTurnAction,
    PageDedicatedWorkerClientEventTurnOutcome, RendererDedicatedWorkerClientEvent,
    RendererDedicatedWorkerMessageEvent, RendererPageDedicatedWorkerClientEventOwner,
    RendererPageDedicatedWorkerClientEventProducer, RendererPageDedicatedWorkerClientEventSender,
    RendererPageDedicatedWorkerClientEventTask,
};
#[cfg(test)]
pub(crate) use self::dom_manipulation::RendererPageDomManipulationOwner;
pub(crate) use self::dom_manipulation::{
    PageDomManipulationTurnAction, PageDomManipulationTurnOutcome,
    RendererPageDomManipulationRoute, RendererPageDomManipulationSender,
    RendererPageDomManipulationTask,
};
pub(crate) use self::dynamic_import_owner_action::{
    PageDynamicImportOwnerActionDocumentEffect, PageDynamicImportOwnerActionTurnAction,
    PageDynamicImportOwnerActionTurnOutcome, RendererPageDynamicImportOwnerActionOwner,
    RendererPageDynamicImportOwnerActionSender, RendererPageDynamicImportOwnerActionTask,
};
pub(crate) use self::element_toggle_event::{
    PageElementToggleEventTargetEffect, PageElementToggleEventTurnAction,
    PageElementToggleEventTurnOutcome, RendererPageElementToggleEventCancellation,
    RendererPageElementToggleEventData, RendererPageElementToggleEventKind,
    RendererPageElementToggleEventOwner, RendererPageElementToggleEventSender,
    RendererPageElementToggleEventState, RendererPageElementToggleEventTask,
    RendererPageElementToggleEventTaskId,
};
pub(crate) use self::file_entry_file_callback::{
    PageFileEntryFileCallbackTargetEffect, PageFileEntryFileCallbackTurnAction,
    PageFileEntryFileCallbackTurnOutcome, RendererPageFileEntryFileCallbackOwner,
    RendererPageFileEntryFileCallbackSender, RendererPageFileEntryFileCallbackTask,
    RendererPageFileEntryFileCallbackTaskId, RendererPageFileEntryFileCallbackTaskKind,
};
pub(crate) use self::file_reading::{
    PageFileReadingTargetEffect, PageFileReadingTurnAction, PageFileReadingTurnOutcome,
    RendererPageFileReadingOwner, RendererPageFileReadingSender, RendererPageFileReadingTask,
    RendererPageFileReadingTaskId, RendererPageFileReadingTaskKind,
};
pub(crate) use self::hash_change_delivery::{
    PageHashChangeDeliveryTargetEffect, PageHashChangeDeliveryTurnAction,
    PageHashChangeDeliveryTurnOutcome, RendererPageHashChangeData,
    RendererPageHashChangeDeliveryOwner, RendererPageHashChangeDeliverySender,
    RendererPageHashChangeDeliveryTask,
};
pub(crate) use self::history_traversal::{
    PageHistoryTraversalTargetEffect, PageHistoryTraversalTurnAction,
    PageHistoryTraversalTurnOutcome, RendererPageHistoryTraversalOwner,
    RendererPageHistoryTraversalProducer, RendererPageHistoryTraversalSender,
    RendererPageHistoryTraversalTask, RendererPageHistoryTraversalTaskId,
    RendererPageHistoryTraversalTaskKind,
};
pub(crate) use self::image_load_event::{
    PageImageLoadEventStalePayloadEffect, PageImageLoadEventTargetEffect,
    PageImageLoadEventTurnAction, PageImageLoadEventTurnOutcome, RendererPageImageLoadEventKind,
    RendererPageImageLoadEventOwner, RendererPageImageLoadEventSender,
    RendererPageImageLoadEventTask, RendererPageImageLoadEventTaskId,
};
pub(crate) use self::indexed_db_task::{
    PageIndexedDbTaskTargetEffect, PageIndexedDbTaskTurnAction, PageIndexedDbTaskTurnOutcome,
    RendererPageIndexedDbTask, RendererPageIndexedDbTaskKind, RendererPageIndexedDbTaskOwner,
    RendererPageIndexedDbTaskSender,
};
pub(crate) use self::internal_loading::{
    PageInternalLoadingTargetEffect, PageInternalLoadingTurnAction, PageInternalLoadingTurnOutcome,
    RendererPageInternalLoadingOwner, RendererPageInternalLoadingSender,
    RendererPageInternalLoadingTask,
};
pub(crate) use self::main_document_post_parse::{
    MainDocumentCompletionRecheckEffect, MainDocumentPostParseCallbackExecution,
    MainDocumentPostParseCallbackSettlement, MainDocumentPostParseExecution,
    MainDocumentPostParseOwner, MainDocumentPostParseStateExecution,
    MainDocumentPostParseTargetEffect, MainDocumentPostParseTaskEnd, MainDocumentPostParseWork,
    MainDocumentScriptLoadDelayEffect,
};
#[cfg(test)]
pub(crate) use self::main_document_runtime::PageMainDocumentRuntimeActionKind;
pub(crate) use self::main_document_runtime::{
    PageMainDocumentRuntimeTargetEffect, PageMainDocumentRuntimeTurnAction,
    PageMainDocumentRuntimeTurnOutcome, PageRuntimeOwnedModuleContinuationTurnAction,
    PageRuntimeScriptAdmissionTargetEffect, PageRuntimeScriptAdmissionTurnAction,
    PageRuntimeScriptContinuationTargetEffect, PageRuntimeScriptContinuationTurnAction,
    RendererPageMainDocumentRuntimeAction, RendererPageMainDocumentRuntimeAdmissionError,
    RendererPageMainDocumentRuntimeOwner, RendererPageMainDocumentRuntimeProducer,
    RendererPageMainDocumentRuntimeRouteClosed, RendererPageMainDocumentRuntimeSender,
    RendererPageMainDocumentRuntimeTask,
};
pub(crate) use self::main_document_task_owner::RendererPageMainDocumentTaskOwner;
pub(crate) use self::main_native_module_task::{
    PageDynamicModuleJobTurnAction, PageMainNativeModuleBodyActivity,
    PageMainNativeModuleSettlement, PageMainNativeModuleTargetEffect,
    PageNativeModuleOwnerEventTurnAction,
};
pub(crate) use self::main_parser_continuation::{
    MainParserContinuationRequest, PageMainParserContinuationTargetEffect,
    PageMainParserContinuationTurnAction, PageMainParserContinuationTurnOutcome,
    RendererPageMainParserContinuationOwner, RendererPageMainParserContinuationProducer,
    RendererPageMainParserContinuationSender, RendererPageMainParserContinuationTask,
};
pub(crate) use self::media_element_event::{
    PageMediaElementEventTargetEffect, PageMediaElementEventTurnAction,
    PageMediaElementEventTurnOutcome, RendererPageMediaElementEventOwner,
    RendererPageMediaElementEventSender, RendererPageMediaElementEventTask,
    RendererPageMediaElementEventTaskId, RendererPageMediaElementEventTaskKind,
};
pub(crate) use self::message_port_delivery::{
    PageMessagePortDeliveryTargetEffect, PageMessagePortDeliveryTurnAction,
    PageMessagePortDeliveryTurnOutcome, RendererPageMessagePortDeliveryOwner,
    RendererPageMessagePortDeliveryProducer, RendererPageMessagePortDeliverySender,
    RendererPageMessagePortDeliveryTask,
};
pub(crate) use self::misc_platform_api::{
    PageMiscPlatformApiTargetEffect, PageMiscPlatformApiTurnAction, PageMiscPlatformApiTurnOutcome,
    RendererPageMiscPlatformApiOwner, RendererPageMiscPlatformApiSender,
    RendererPageMiscPlatformApiTask, RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
};
#[cfg(test)]
pub(crate) use self::module_reaction::PageModuleReactionCurrentEffect;
pub(crate) use self::module_reaction::{
    PageModuleReactionApplication, PageModuleReactionFollowup, PageModuleReactionTargetEffect,
    PageModuleReactionTurnAction, PageModuleReactionTurnOutcome, RendererPageModuleReactionEvent,
    RendererPageModuleReactionOwner, RendererPageModuleReactionSender,
    RendererPageModuleReactionTarget, RendererPageModuleReactionTask,
};
pub(crate) use self::modulepreload_start::{
    PageModulepreloadStartDocumentEffect, PageModulepreloadStartTurnAction,
    PageModulepreloadStartTurnOutcome, RendererPageModulepreloadStartOwner,
    RendererPageModulepreloadStartSender, RendererPageModulepreloadStartTask,
};
#[cfg(test)]
pub(crate) use self::navigation_and_traversal::RendererPageNavigationAndTraversalHead;
pub(crate) use self::navigation_and_traversal::{
    PageNavigationAndTraversalTurnAction, PageNavigationAndTraversalTurnOutcome,
    RendererPageNavigationAndTraversalSender, RendererPageNavigationAndTraversalTask,
};
pub(crate) use self::navigation_api_task::{
    PageNavigationApiTaskTargetEffect, PageNavigationApiTaskTurnAction,
    PageNavigationApiTaskTurnOutcome, RendererPageNavigationApiTask,
    RendererPageNavigationApiTaskId, RendererPageNavigationApiTaskKind,
    RendererPageNavigationApiTaskOwner, RendererPageNavigationApiTaskProducer,
    RendererPageNavigationApiTaskSender,
};
pub(crate) use self::networking::{
    PageNetworkingTurnAction, PageNetworkingTurnOutcome, RendererPageNetworkingRoute,
    RendererPageNetworkingTask,
};
#[cfg(test)]
pub(crate) use self::networking::{RendererPageNetworkingOwner, RendererPageNetworkingSource};
pub(crate) use self::opfs_task::{
    PageOpfsTaskTargetEffect, PageOpfsTaskTurnAction, PageOpfsTaskTurnOutcome,
    RendererPageOpfsTask, RendererPageOpfsTaskId, RendererPageOpfsTaskOwner,
    RendererPageOpfsTaskProducer, RendererPageOpfsTaskSender,
};
#[cfg(test)]
pub(crate) use self::owner_sources::{
    RendererPageModulepreloadStartTestSource, RendererPageOwnedTaskSourcesTestHarness,
    RendererPageResourceCompletionTestSource,
};
pub(crate) use self::owner_sources::{
    RendererPageOwnedTaskSources, RendererPageReadyDescriptor, RendererPageSchedulerTask,
    RendererPageTaskProducerRoutes, RendererPageTaskSourceKind,
};
pub(crate) use self::parser_async_module_admission::{
    PageParserAsyncModuleAdmissionTargetEffect, PageParserAsyncModuleAdmissionTurnAction,
};
pub(crate) use self::parser_owned_module_continuation::{
    PageParserOwnedModuleContinuationBodyActivity, PageParserOwnedModuleContinuationTargetEffect,
    PageParserOwnedModuleContinuationTurnAction,
};
pub(crate) use self::popup_load_event::{
    PagePopupLoadEventTargetEffect, PagePopupLoadEventTurnAction, PagePopupLoadEventTurnOutcome,
    RendererPagePopupLoadEventOwner, RendererPagePopupLoadEventSender,
    RendererPagePopupLoadEventTask,
};
pub(crate) use self::post_parse_owner_work::{
    PostParseLifecycleQueueStats, PostParseLifecycleWork, PostParsePageOwnedWork,
    post_parse_lifecycle_queue_stats,
};
#[cfg(test)]
pub(crate) use self::rendering_update::RendererPageRenderingUpdateHead;
pub(crate) use self::rendering_update::{
    PageRenderingUpdateTargetEffect, PageRenderingUpdateTurnAction, PageRenderingUpdateTurnOutcome,
    RendererPageRenderingUpdateOwner, RendererPageRenderingUpdateSender,
    RendererPageRenderingUpdateTask, RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
};
#[cfg(test)]
pub(crate) use self::resource_completions::RendererResourceCompletionTestHarness;
pub(super) use self::resource_completions::{
    RendererOwnerWake, RendererOwnerWakeSender, RendererOwnerWakeSource,
    RendererResourceCompletionSender, RendererTopLevelNavigationHandoff,
};
pub(crate) use self::senders::PageRuntimeWakeSignal;
#[cfg(test)]
pub(crate) use self::senders::RendererPageTaskTestResidence;
pub(super) use self::senders::{
    MainDocumentRuntimeContinuationSender, PageRuntimeWakeSender, PageTaskSender,
    RendererTopLevelNavigationHandoffSender, RuntimePageTaskSender,
};
pub(crate) use self::senders::{PageRuntimeTaskSource, RendererPageJsContextTaskSenders};
#[cfg(test)]
pub(crate) use self::service_worker_client_message::RendererPageServiceWorkerClientMessageOwner;
pub(crate) use self::service_worker_client_message::{
    PageServiceWorkerClientMessageTargetEffect, PageServiceWorkerClientMessageTurnAction,
    PageServiceWorkerClientMessageTurnOutcome, RendererPageServiceWorkerClientMessageSender,
    RendererPageServiceWorkerClientMessageTask, ServiceWorkerClientMessageCallbackEffect,
    ServiceWorkerClientMessageEventKind,
};
#[cfg(test)]
pub(crate) use self::service_worker_internal::RendererServiceWorkerInternalTaskKind;
pub(crate) use self::service_worker_internal::{
    PageServiceWorkerInternalTargetEffect, PageServiceWorkerInternalTurnAction,
    PageServiceWorkerInternalTurnOutcome, RendererPageServiceWorkerInternalSender,
    RendererPageServiceWorkerInternalTask, RendererServiceWorkerInternalTask,
    ServiceWorkerInternalCallbackEffect,
};
pub(crate) use self::service_worker_tasks::RendererPageServiceWorkerTaskSender;
#[cfg(test)]
pub(crate) use self::service_worker_tasks::RendererPageServiceWorkerTestHarness;
#[cfg(test)]
pub(crate) use self::shared_worker_client_event::RendererSharedWorkerClientEventKind;
pub(crate) use self::shared_worker_client_event::{
    PageSharedWorkerClientEventTargetEffect, PageSharedWorkerClientEventTurnAction,
    PageSharedWorkerClientEventTurnOutcome, RendererPageSharedWorkerClientEventOwner,
    RendererPageSharedWorkerClientEventProducer, RendererPageSharedWorkerClientEventRealmSender,
    RendererPageSharedWorkerClientEventSender, RendererPageSharedWorkerClientEventTask,
};
pub(crate) use self::storage_event_delivery::{
    PageStorageEventDeliveryTargetEffect, PageStorageEventDeliveryTurnAction,
    PageStorageEventDeliveryTurnOutcome, RendererPageStorageEventData,
    RendererPageStorageEventDeliveryOwner, RendererPageStorageEventDeliverySender,
    RendererPageStorageEventDeliveryTask,
};
#[cfg(test)]
pub(crate) use self::stylesheet_task::RendererPageStylesheetTaskTestResidence;
pub(crate) use self::stylesheet_task::{
    PageConnectedStyleEventTargetEffect, PageConnectedStyleEventTurnAction,
    PageConnectedStyleEventTurnOutcome, PageConnectedStyleLoadDelayEffect,
    PageStylesheetNetworkingTargetEffect, PageStylesheetNetworkingTurnAction,
    PageStylesheetNetworkingTurnOutcome, RendererPageConnectedStyleEventTask,
    RendererPageStylesheetCompletion, RendererPageStylesheetNetworkingTask,
    RendererPageStylesheetTaskOwner, RendererPageStylesheetTaskProducer,
    RendererPageStylesheetTaskSender,
};
pub(super) use self::tasks::{
    ContentSecurityPolicyViolationEventTask, MainDocumentMetaRefreshNavigationTask,
    PageOwnedInternalLoadingTask, PageOwnedInternalLoadingTaskEffect, PageTask,
    WindowScriptFailureReportTask,
};
pub(crate) use self::text_track_default_mode::{
    PageTextTrackDefaultModeTargetEffect, PageTextTrackDefaultModeTurnAction,
    PageTextTrackDefaultModeTurnOutcome, RendererPageTextTrackDefaultModeOwner,
    RendererPageTextTrackDefaultModeSender, RendererPageTextTrackDefaultModeTask,
    RendererPageTextTrackDefaultModeTaskId, RendererPageTextTrackDefaultModeTaskKind,
};
pub(crate) use self::text_track_load::{
    PageTextTrackLoadStalePayloadEffect, PageTextTrackLoadTargetEffect,
    PageTextTrackLoadTurnAction, PageTextTrackLoadTurnOutcome, RendererPageTextTrackLoadOwner,
    RendererPageTextTrackLoadRouteClosed, RendererPageTextTrackLoadSender,
    RendererPageTextTrackLoadTask, RendererPageTextTrackLoadTaskId,
    RendererPageTextTrackLoadTaskKind,
};
pub(crate) use self::user_interaction::{
    PageUserInteractionBodyEffect, PageUserInteractionTargetEffect, PageUserInteractionTurnAction,
    PageUserInteractionTurnOutcome, RendererPageUserInteractionEventKind,
    RendererPageUserInteractionOwner, RendererPageUserInteractionSender,
    RendererPageUserInteractionTask, RendererPageUserInteractionTaskId,
    RendererPageUserInteractionTaskKind,
};
#[cfg(test)]
pub(crate) use self::v8_foreground_task::RendererPageV8ForegroundTaskOwner;
pub(crate) use self::v8_foreground_task::{
    PageV8ForegroundTaskEffect, PageV8ForegroundTaskTurnAction, PageV8ForegroundTaskTurnOutcome,
    RendererPageV8ForegroundTask, RendererPageV8ForegroundTaskSender,
};
pub(crate) use self::view_transition_update::{
    PageViewTransitionUpdateTargetEffect, PageViewTransitionUpdateTurnAction,
    PageViewTransitionUpdateTurnOutcome, RendererPageViewTransitionUpdateOwner,
    RendererPageViewTransitionUpdateSender, RendererPageViewTransitionUpdateTask,
    RendererPageViewTransitionUpdateTaskId,
};
pub(crate) use self::webcrypto_task::{
    PageWebCryptoTaskTargetEffect, PageWebCryptoTaskTurnAction, PageWebCryptoTaskTurnOutcome,
    RendererPageWebCryptoTask, RendererPageWebCryptoTaskId, RendererPageWebCryptoTaskOwner,
    RendererPageWebCryptoTaskProducer, RendererPageWebCryptoTaskSender,
};
#[cfg(test)]
pub(crate) use self::websocket_event::RendererPageWebSocketOwner;
pub(crate) use self::websocket_event::{
    PageWebSocketBodyEffect, PageWebSocketTargetEffect, PageWebSocketTurnAction,
    PageWebSocketTurnOutcome, RendererPageWebSocketReadiness, RendererPageWebSocketSender,
    RendererPageWebSocketTask,
};
pub(crate) use self::window_message::{
    PageWindowMessageTargetEffect, PageWindowMessageTurnAction, PageWindowMessageTurnOutcome,
    RendererPageWindowMessageOwner, RendererPageWindowMessageSender, RendererPageWindowMessageTask,
    RendererPageWindowMessageTaskId,
};
pub(crate) use self::worker_host_bridge::{
    PageWorkerHostBridgeCurrentEffect, PageWorkerHostBridgeTargetEffect,
    PageWorkerHostBridgeTurnAction, PageWorkerHostBridgeTurnOutcome,
    RendererPageWorkerHostBridgeOwner, RendererPageWorkerHostBridgeTask,
    RendererWorkerHostBridgeEventSender, is_worker_host_bridge_message,
};

/// Minimal page-owned task queue.
///
/// This queue is deliberately narrow:
/// - FIFO task storage
/// - no general task source taxonomy yet
/// - currently only script-adjacent page tasks flow through it:
///   - script execution turns
///   - DOMContentLoaded dispatch
///   - window load dispatch
///
/// The important structural change is that page-owned JS work now has an
/// explicit queueing step before execution. That makes it possible to evolve
/// toward a more standard task/checkpoint model without keeping
/// `ParseTimeCoordinator` as the place that directly executes whichever ready
/// scripts the scheduler happened to return.
///
/// In other words:
/// - `LocalSet` gives us a stable owner lane
/// - `PageTaskQueue` starts giving that lane explicit work units
/// - a future page/document event loop can grow from this queue instead of from
///   ad-hoc "run the ready script right now" call paths
#[derive(Debug)]
pub(super) struct PageTaskQueue {
    task_source: OwnerTaskSource<PageTask>,
    parse_time_document_script_source: OwnerTaskSource<ParseTimeDocumentScriptEvent>,
    post_parse_work_source: OwnerTaskSource<PostParsePageOwnedWork>,
    page_runtime_task_source: PageRuntimeTaskSource,
}

/// Low-level test fixture over the production Page task route and queue.
///
/// The fixture, rather than [`PageTaskQueue`] or [`PageRuntimeTaskSource`],
/// owns the unique source consumer. Tests can therefore inspect or execute a
/// production task without adding test-only state to either production type.
#[cfg(test)]
pub(crate) struct PageTaskQueueTestHarness {
    queue: PageTaskQueue,
    residence: RendererPageTaskTestResidence,
}

#[cfg(test)]
impl std::ops::Deref for PageTaskQueueTestHarness {
    type Target = PageTaskQueue;

    fn deref(&self) -> &Self::Target {
        &self.queue
    }
}

#[cfg(test)]
impl std::ops::DerefMut for PageTaskQueueTestHarness {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.queue
    }
}

#[cfg(test)]
impl PageTaskQueueTestHarness {
    pub(crate) fn new() -> Self {
        Self::new_with_owner_wake(None)
    }

    pub(crate) fn new_with_owner_wake(owner_wake: Option<RendererOwnerWakeSender>) -> Self {
        let residence = RendererPageTaskTestResidence::new(owner_wake);
        Self::new_with_residence(residence)
    }

    pub(crate) fn new_with_residence(residence: RendererPageTaskTestResidence) -> Self {
        Self {
            queue: PageTaskQueue::new_with_page_runtime_task_source(residence.runtime_source()),
            residence,
        }
    }

    pub(crate) fn task_sources(&self) -> RendererPageOwnedTaskSourcesTestHarness {
        self.residence.task_sources()
    }

    pub(crate) fn residence(&self) -> RendererPageTaskTestResidence {
        self.residence.clone()
    }
}

impl PageTaskQueue {
    pub(super) fn new_with_page_runtime_task_source(
        page_runtime_task_source: PageRuntimeTaskSource,
    ) -> Self {
        Self {
            task_source: OwnerTaskSource::new(),
            parse_time_document_script_source: OwnerTaskSource::new(),
            post_parse_work_source: OwnerTaskSource::new(),
            page_runtime_task_source,
        }
    }

    pub(super) fn resource_completion_sender(
        &self,
    ) -> Option<crate::page_resource_completion::RendererPageResourceCompletionSender> {
        self.page_runtime_task_source.resource_completion_sender()
    }

    pub(super) fn page_task_producer_routes_match(
        &self,
        sources: &RendererPageOwnedTaskSources,
    ) -> bool {
        self.page_runtime_task_source
            .page_task_producer_routes_match(sources)
    }

    pub(super) async fn wait_for_page_runtime_wake(&self) {
        self.page_runtime_task_source.wait().await;
    }

    /// Wait for a producer-side arrival without treating already-buffered local
    /// work as a new wake. This is a test observation hook over production
    /// sources; it does not select or execute a task.
    #[cfg(test)]
    pub(crate) async fn wait_for_injected_task_arrival_without_timeout(&mut self) -> bool {
        tokio::select! {
            biased;
            arrived = self.post_parse_work_source.wait_for_wake_arrival() => arrived,
            _ = self.page_runtime_task_source.wait() => true,
            arrived = self.task_source.wait_for_wake_arrival() => arrived,
        }
    }

    pub(super) fn enqueue_front_post_parse_work_preserving_order<I>(&mut self, work: I)
    where
        I: IntoIterator<Item = PostParsePageOwnedWork>,
    {
        let work: Vec<PostParsePageOwnedWork> = work.into_iter().collect();
        for item in work.into_iter().rev() {
            self.post_parse_work_source
                .enqueue_parser_boundary_local(item);
        }
    }

    pub(super) fn extend_post_parse_work<I>(&mut self, work: I)
    where
        I: IntoIterator<Item = PostParsePageOwnedWork>,
    {
        let mut work: Vec<PostParsePageOwnedWork> = work.into_iter().collect();
        work.sort_by_key(PostParsePageOwnedWork::phase_sort_key);
        self.post_parse_work_source.extend_local(work);
    }

    pub(super) fn post_parse_pop_front(&mut self) -> Option<PostParsePageOwnedWork> {
        self.complete_ready_source_loads();
        self.post_parse_work_source.pop_front()
    }

    pub(super) fn post_parse_front(&mut self) -> Option<&PostParsePageOwnedWork> {
        self.complete_ready_source_loads();
        self.post_parse_work_source.front()
    }

    /// Discard work whose lifetime is bounded by this concrete PageVm.
    ///
    /// Page-runtime sources are deliberately excluded: their container lives
    /// in the stable Page script environment and survives Document
    /// replacement. The stable Page slot clears them only when the Page is
    /// retired.
    pub(super) fn clear_document_owned_tasks(&mut self) {
        self.task_source.clear_local();
        self.parse_time_document_script_source.clear_local();
        self.post_parse_work_source.clear_local();
    }

    pub(super) fn front(&mut self) -> Option<&PageTask> {
        let _ = self.complete_ready_source_loads();
        self.task_source.front()
    }

    pub(crate) fn pending_task_source_load(&mut self) -> Option<SharedScriptSourceLoad> {
        let _ = self.complete_ready_source_loads();
        self.post_parse_work_source
            .front()
            .and_then(PostParsePageOwnedWork::pending_source_load)
    }

    pub(super) fn is_empty(&mut self) -> bool {
        let _ = self.complete_ready_source_loads();
        self.task_source.is_empty()
            && self.parse_time_document_script_source.is_empty()
            && self.post_parse_work_source.is_empty()
    }

    pub(crate) fn complete_ready_source_loads(&mut self) -> bool {
        let source_load_wake = self.page_runtime_task_source.page_runtime_wake_sender();
        let mut completed = false;
        self.post_parse_work_source.with_tasks_mut(|tasks| {
            for task in tasks.iter_mut() {
                completed |= refresh_page_owned_source_load(task, &source_load_wake);
            }
            promote_ready_post_parse_async_work_over_waiting_source_loads(tasks);
        });
        completed
    }
}

fn promote_ready_post_parse_async_work_over_waiting_source_loads(
    tasks: &mut std::collections::VecDeque<PostParsePageOwnedWork>,
) {
    let Some(mut insert_at) = tasks
        .iter()
        .position(PostParsePageOwnedWork::is_waiting_for_source_load)
    else {
        return;
    };
    let mut index = insert_at + 1;
    while index < tasks.len() {
        if tasks[index].is_async_phase_document_script()
            && !tasks[index].is_waiting_for_source_load()
        {
            let work = tasks
                .remove(index)
                .expect("ready async owner work index should still be present");
            tasks.insert(insert_at, work);
            insert_at += 1;
            index += 1;
        } else {
            index += 1;
        }
    }
}

fn refresh_page_owned_source_load(
    work: &mut PostParsePageOwnedWork,
    source_load_wake: &PageRuntimeWakeSender,
) -> bool {
    if work.complete_source_load_if_ready() {
        return true;
    }
    let Some(source_load) = work.claim_source_load_completion_wake() else {
        return false;
    };
    let source_load_wake = source_load_wake.clone();
    source_load.register_completion_wake(move || {
        let _ = source_load_wake.send_document_lifecycle_wake();
    });
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::dom::NodeId;
    use crate::{
        document_script_scheduler::{
            DocumentScriptExecutionLane, PageOwnedDocumentScriptWork, ParseTimeDocumentScriptEvent,
            ParseTimeDocumentScriptTask,
        },
        host::{ScriptEventKind, ScriptEventTask},
        planning::{
            PreparedScript, PreparedScriptSourceLoadOutcome, ScriptSource, SharedScriptSourceLoad,
        },
        runtime::RendererPageToken,
        types::{
            AsyncSubresourceFetchCompletion, ScriptKind, ScriptMode, ScriptRun, ScriptSourceKind,
        },
    };
    use moli_websocket::Event as WebSocketEvent;
    use url::Url;

    fn prepared_script(position: usize) -> PreparedScript {
        PreparedScript {
            position,
            node_id: NodeId::new(position + 1),
            kind: ScriptKind::Classic,
            mode: ScriptMode::Async,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            base_url: Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            initiator_url: Url::parse("https://example.com/index.html").unwrap(),
            host_script_handle: None,
        }
    }

    fn post_parse_document_script_work(
        lane: DocumentScriptExecutionLane,
        script: PreparedScript,
    ) -> PostParsePageOwnedWork {
        PostParsePageOwnedWork::document_script_work(PageOwnedDocumentScriptWork::script(
            lane, script,
        ))
    }

    fn post_parse_document_script_work_waiting_for_source(
        lane: DocumentScriptExecutionLane,
        script: PreparedScript,
        source_load: SharedScriptSourceLoad,
    ) -> PostParsePageOwnedWork {
        PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::script_waiting_for_source(lane, script, source_load),
        )
    }

    fn is_async_phase_document_script_work(work: &PostParsePageOwnedWork, position: usize) -> bool {
        work.as_page_task().is_none()
            && work.is_async_phase_document_script()
            && work
                .as_script()
                .is_some_and(|script| script.position == position)
    }

    fn skipped_script_run(position: usize) -> ScriptRun {
        ScriptRun::skipped(
            NodeId::new(position + 1),
            ScriptKind::Classic,
            ScriptMode::Normal,
            ScriptSourceKind::Inline,
            Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            crate::types::ScriptSkipReason::NotInMainDocument,
        )
    }

    fn record_document_script_run_task(position: usize) -> PageTask {
        PageTask::RecordDocumentScriptRun {
            position,
            run: skipped_script_run(position),
        }
    }

    fn source_load_outcome_ok(source: impl Into<String>) -> PreparedScriptSourceLoadOutcome {
        PreparedScriptSourceLoadOutcome {
            source_result: Ok(source.into()),
            source_bytes: None,
            network_result: None,
        }
    }

    fn async_subresource_completion(internal_id: u64) -> AsyncSubresourceFetchCompletion {
        AsyncSubresourceFetchCompletion {
            internal_id,
            request_url: Url::parse("https://example.com/api").unwrap(),
            request_method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            response_status_text: None,
            skip_fetch_security_validation: false,
            response_filter: None,
            network_error_text: None,
            result: Err("test failure".to_owned()),
        }
    }

    #[tokio::test]
    async fn renderer_resource_completion_sender_wakes_owner_queue() {
        let mut queue = RendererResourceCompletionTestHarness::new();
        let sender = queue.sender();

        sender
            .send_async_subresource(async_subresource_completion(7))
            .expect("resource completion should send");

        assert!(queue.wait_for_arrival_without_timeout().await);
        let completion = queue
            .pop_next_async_subresource_event()
            .expect("completion should be queued");
        assert!(matches!(
            completion,
            crate::types::AsyncSubresourceFetchEvent::Completion(completion)
                if completion.internal_id == 7
        ));
        assert!(!queue.has_ready_completion());
    }

    #[tokio::test]
    async fn distinct_resource_and_websocket_capabilities_signal_the_same_page_owner() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let page_token = RendererPageToken::new_for_testing(crate::PageId::new_for_testing(1));
        let owner_wake = RendererOwnerWakeSender::new(wake_tx, page_token);
        let residence = RendererPageTaskTestResidence::new(Some(owner_wake.clone()));
        let root_document = residence.root_document();
        let senders = residence
            .runtime_source()
            .bound_task_producer_senders(root_document)
            .expect("test Page residence must expose all typed producer routes");
        let (js_context, _, resource_completion, _, _, _) = senders.into_parts();
        let resource_sender = RendererResourceCompletionSender::for_page_scheduler(
            resource_completion,
            root_document,
        );
        let websocket_sender = js_context.websocket().clone();

        resource_sender
            .send_async_subresource(async_subresource_completion(7))
            .expect("async subresource completion should send");
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("async subresource should wake owner")
                .page_id()
                .as_u64(),
            1
        );

        assert!(
            websocket_sender
                .event_sender()
                .send(WebSocketEvent::Close {
                    socket_id: 11,
                    code: 1005,
                    reason: String::new(),
                    was_clean: true,
                })
                .await
        );
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("websocket event should wake owner")
                .page_id()
                .as_u64(),
            1
        );
    }

    #[test]
    fn resource_completion_queue_is_independent_from_page_task_queue_clear() {
        let mut page_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let mut resource_queue = RendererResourceCompletionTestHarness::new();
        let sender = resource_queue.sender();

        page_queue.enqueue_parser_boundary(PageTask::DispatchWindowLoad);
        sender
            .send_async_subresource(async_subresource_completion(11))
            .expect("resource completion should send");

        page_queue.clear_document_owned_tasks();

        assert!(page_queue.parse_time_pop_front().is_none());
        let completion = resource_queue
            .pop_next_async_subresource_event()
            .expect("resource completion must not live in PageTaskQueue");
        assert!(matches!(
            completion,
            crate::types::AsyncSubresourceFetchEvent::Completion(completion)
                if completion.internal_id == 11
        ));
    }

    #[test]
    fn ready_post_parse_async_source_load_becomes_page_owned_script_work() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        queue.extend_post_parse_work([post_parse_document_script_work_waiting_for_source(
            DocumentScriptExecutionLane::AsyncPhase,
            prepared_script(1),
            SharedScriptSourceLoad::ready_ok("window.readyAsync = 1;"),
        )]);

        let front = queue.post_parse_front().expect("ready async work");
        assert!(!front.is_waiting_for_source_load());
        assert!(front.as_page_task().is_none());
        assert!(matches!(
            front,
            PostParsePageOwnedWork::DocumentScript(work)
                if matches!(
                    work.as_ref(),
                    PageOwnedDocumentScriptWork::Script {
                        lane: DocumentScriptExecutionLane::AsyncPhase,
                        script,
                        ..
                    } if matches!(
                        &script.source,
                        ScriptSource::Loaded(source)
                            if source == "window.readyAsync = 1;"
                    )
                )
        ));
    }

    #[test]
    fn failed_post_parse_async_source_load_becomes_page_owned_source_failure() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        queue.extend_post_parse_work([post_parse_document_script_work_waiting_for_source(
            DocumentScriptExecutionLane::AsyncPhase,
            prepared_script(1),
            SharedScriptSourceLoad::ready_err("synthetic source load failure"),
        )]);

        let front = queue.post_parse_front().expect("ready async failure work");
        assert!(!front.is_waiting_for_source_load());
        assert!(front.as_page_task().is_none());
        assert!(matches!(
            front,
            PostParsePageOwnedWork::DocumentScript(work)
                if matches!(
                    work.as_ref(),
                    PageOwnedDocumentScriptWork::AsyncSourceFailure {
                        lane: crate::document_script_scheduler::DocumentScriptSourceFailureLane::AsyncPhase,
                        script,
                        failure,
                        ..
                    } if matches!(&script.source, ScriptSource::External)
                        && failure.message() == "synthetic source load failure"
                )
        ));
    }

    #[tokio::test]
    async fn ready_async_phase_script_bypasses_pending_async_source_load() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        queue.extend_post_parse_work([
            PostParsePageOwnedWork::lifecycle_work(PostParseLifecycleWork::test_domcontentloaded()),
            post_parse_document_script_work_waiting_for_source(
                DocumentScriptExecutionLane::AsyncPhase,
                prepared_script(1),
                SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
            ),
            post_parse_document_script_work(
                DocumentScriptExecutionLane::AsyncPhase,
                prepared_script(2).with_loaded_source("ready".to_owned()),
            ),
            PostParsePageOwnedWork::lifecycle_work(PostParseLifecycleWork::test_window_load()),
        ]);

        assert!(
            queue
                .post_parse_pop_front()
                .is_some_and(|work| work.is_domcontentloaded_task())
        );
        let first_async = queue
            .post_parse_pop_front()
            .expect("ready async should bypass pending async");
        assert!(is_async_phase_document_script_work(&first_async, 2));
        assert!(
            queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_waiting_for_source_load)
        );
    }

    #[tokio::test]
    async fn ready_async_phase_script_bypasses_pending_defer_source_load() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        queue.extend_post_parse_work([
            post_parse_document_script_work_waiting_for_source(
                DocumentScriptExecutionLane::ClassicDefer,
                prepared_script(1),
                SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
            ),
            PostParsePageOwnedWork::lifecycle_work(PostParseLifecycleWork::test_domcontentloaded()),
            post_parse_document_script_work(
                DocumentScriptExecutionLane::AsyncPhase,
                prepared_script(2).with_loaded_source("ready".to_owned()),
            ),
            PostParsePageOwnedWork::lifecycle_work(PostParseLifecycleWork::test_window_load()),
        ]);

        let first = queue
            .post_parse_pop_front()
            .expect("ready async should bypass pending defer");
        assert!(is_async_phase_document_script_work(&first, 2));
        assert!(
            queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_waiting_for_source_load)
        );
    }

    #[test]
    fn lifecycle_page_tasks_do_not_reenter_parse_time_scheduler_followup() {
        assert!(
            !PageTask::SeedDocumentOwnedBlockingStylesheets(Vec::new())
                .allows_parse_time_scheduler_followup_turn()
        );
        assert!(!PageTask::DispatchDomContentLoaded.allows_parse_time_scheduler_followup_turn());
        assert!(!PageTask::DispatchWindowLoad.allows_parse_time_scheduler_followup_turn());
    }

    #[test]
    fn lifecycle_page_tasks_keep_distinct_phase_labels() {
        assert_eq!(
            PageTask::SeedDocumentOwnedBlockingStylesheets(Vec::new()).phase_label(),
            "stylesheet seed task"
        );
        assert_eq!(
            PageTask::DispatchDomContentLoaded.phase_label(),
            "domcontentloaded task"
        );
    }

    #[test]
    fn parser_boundary_tasks_are_visible_after_production_wake_admission() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let sender = queue.parser_boundary_sender();
        sender.send(PageTask::DispatchDomContentLoaded).unwrap();

        assert!(queue.parse_time_front().is_none());
        queue.accept_ready_parse_time_wakes();
        assert!(matches!(
            queue.parse_time_pop_front(),
            Some(PageTask::DispatchDomContentLoaded)
        ));
    }

    #[test]
    fn parser_boundary_task_arriving_after_empty_poll_is_visible_after_admission() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let sender = queue.parser_boundary_sender();

        assert!(queue.parse_time_pop_front().is_none());
        let _ = sender.send(PageTask::DispatchWindowLoad);
        queue.accept_ready_parse_time_wakes();

        assert!(matches!(
            queue.parse_time_pop_front(),
            Some(PageTask::DispatchWindowLoad)
        ));
    }

    #[test]
    fn admitted_parser_boundary_tasks_precede_existing_local_parse_time_work() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        queue.enqueue_parser_boundary(PageTask::DispatchDomContentLoaded);
        let sender = queue.parser_boundary_sender();
        sender
            .send(PageTask::window_script_failure_report(
                WindowScriptFailureReportTask::new("boom", Some("/tmp/failure.mjs".to_owned())),
            ))
            .unwrap();
        queue.accept_ready_parse_time_wakes();

        assert!(matches!(
            queue.parse_time_pop_front(),
            Some(PageTask::ReportWindowScriptFailure(WindowScriptFailureReportTask {
                message,
                filename,
                ..
            })) if message == "boom" && filename.as_deref() == Some("/tmp/failure.mjs")
        ));
        assert!(matches!(
            queue.parse_time_pop_front(),
            Some(PageTask::DispatchDomContentLoaded)
        ));
    }

    #[test]
    fn parse_time_turn_accepts_ready_parser_boundary_work_before_polling() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let sender = queue.parser_boundary_sender();
        sender
            .send(PageTask::window_script_failure_report(
                WindowScriptFailureReportTask::new("boom", None),
            ))
            .unwrap();

        assert!(queue.parse_time_front().is_none());

        queue.accept_ready_parse_time_wakes();

        assert!(matches!(
            queue.parse_time_pop_front(),
            Some(PageTask::ReportWindowScriptFailure(WindowScriptFailureReportTask {
                message,
                ..
            })) if message == "boom"
        ));
    }

    #[test]
    fn open_stream_document_admission_requires_a_concrete_parse_time_payload() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        assert!(
            !queue.admit_ready_parse_time_document_work(),
            "an empty stable source must not manufacture a Document owner turn"
        );

        queue
            .parse_time_document_script_sender()
            .send(ParseTimeDocumentScriptEvent::async_completion(
                NodeId::new(89),
                source_load_outcome_ok("ready"),
            ))
            .expect("test completion should enter the stable source");

        assert!(queue.admit_ready_parse_time_document_work());
        assert!(matches!(
            queue.parse_time_document_script_pop_front(),
            Some(ParseTimeDocumentScriptEvent::AsyncCompletion(_))
        ));
        assert!(!queue.admit_ready_parse_time_document_work());
    }

    #[test]
    fn phase_transition_collects_ready_parse_time_lifecycle_wakes() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        queue.enqueue_parse_time_document_script_task(Some(
            ParseTimeDocumentScriptTask::classic_async_script_for_test(prepared_script(7)),
        ));
        queue
            .parser_boundary_sender()
            .send(PageTask::window_script_failure_report(
                WindowScriptFailureReportTask::new("boom", None),
            ))
            .unwrap();

        let lifecycle_work = queue.take_parse_time_lifecycle_work();

        assert!(matches!(
            lifecycle_work[0].as_lifecycle_work(),
            Some(PostParseLifecycleWork::ReportWindowScriptFailure(
                WindowScriptFailureReportTask { message, .. }
            )) if message == "boom"
        ));
        assert!(matches!(
            queue.parse_time_document_script_pop_front(),
            Some(ParseTimeDocumentScriptEvent::ReadyTask(task))
                if matches!(task.as_ref(), ParseTimeDocumentScriptTask::ClassicAsyncScript(_))
        ));
        assert!(queue.take_parse_time_lifecycle_work().is_empty());
    }

    #[tokio::test]
    async fn waiting_for_injected_arrival_ignores_existing_local_tasks() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        queue.enqueue_parser_boundary(PageTask::DispatchDomContentLoaded);

        let arrived = tokio::time::timeout(
            std::time::Duration::from_millis(5),
            queue.wait_for_injected_task_arrival_without_timeout(),
        )
        .await;

        assert!(arrived.is_err());
        assert!(matches!(
            queue.parse_time_pop_front(),
            Some(PageTask::DispatchDomContentLoaded)
        ));
    }

    #[tokio::test]
    async fn pending_post_parse_source_load_wakes_its_durable_owner_queue() {
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let source_load = SharedScriptSourceLoad::spawn_for_test(async move {
            finish_rx.await.expect("source completion signal");
            Ok("window.asyncReady = true;".to_owned())
        });
        let (owner_wake_tx, mut owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let page_token = RendererPageToken::new_for_testing(crate::PageId::new_for_testing(8));
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new_with_owner_wake(
            Some(RendererOwnerWakeSender::new(owner_wake_tx, page_token)),
        );
        queue.extend_post_parse_work([PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::script_waiting_for_source(
                DocumentScriptExecutionLane::AsyncPhase,
                prepared_script(1),
                source_load,
            ),
        )]);

        assert!(
            queue
                .post_parse_front()
                .expect("pending post-parse script")
                .is_waiting_for_source_load()
        );
        finish_tx.send(()).expect("finish source load");
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                queue.wait_for_injected_task_arrival_without_timeout(),
            )
            .await
            .expect("source terminal should wake the page-owned queue")
        );
        let owner_wake = owner_wake_rx
            .recv()
            .await
            .expect("source terminal should wake the renderer owner");
        assert_eq!(
            owner_wake.source_for_test(),
            RendererOwnerWakeSource::Runtime(
                crate::runtime::RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn
            )
        );
        assert!(
            !queue
                .post_parse_front()
                .expect("ready post-parse script")
                .is_waiting_for_source_load()
        );
    }

    #[tokio::test]
    async fn local_runtime_wake_does_not_manufacture_an_owner_task() {
        let (owner_wake_tx, mut owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let page_token = RendererPageToken::new_for_testing(crate::PageId::new_for_testing(9));
        let source = PageRuntimeTaskSource::new(Some(RendererOwnerWakeSender::new(
            owner_wake_tx,
            page_token,
        )));

        source
            .page_runtime_wake_sender()
            .send_wake()
            .expect("local runtime wake is infallible");
        tokio::time::timeout(std::time::Duration::from_secs(1), source.wait())
            .await
            .expect("local runtime observer should receive its durable wake");
        assert!(
            owner_wake_rx.try_recv().is_err(),
            "a payload-free runtime wake must not manufacture an owner Page turn"
        );
    }

    #[test]
    fn take_parse_time_document_script_events_removes_only_typed_events() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let parser_boundary = queue.parser_boundary_sender();
        for task in [
            PageTask::SeedDocumentOwnedBlockingStylesheets(Vec::new()),
            record_document_script_run_task(1),
            PageTask::DispatchDomContentLoaded,
            PageTask::DispatchWindowLoad,
        ] {
            parser_boundary.send(task).expect("parser-boundary task");
        }
        queue.accept_ready_parse_time_wakes();
        queue.enqueue_parse_time_document_script_task(Some(
            ParseTimeDocumentScriptTask::classic_async_script_for_test(prepared_script(1)),
        ));
        queue.enqueue_parse_time_document_script_event(
            ParseTimeDocumentScriptEvent::async_completion(
                NodeId::new(99),
                source_load_outcome_ok("ready"),
            ),
        );

        let parse_time = queue.take_parse_time_document_script_events();

        assert!(matches!(
            &parse_time[0],
            ParseTimeDocumentScriptEvent::ReadyTask(task)
                if matches!(task.as_ref(), ParseTimeDocumentScriptTask::ClassicAsyncScript(_))
        ));
        assert!(matches!(
            parse_time[1],
            ParseTimeDocumentScriptEvent::AsyncCompletion(_)
        ));
        let lifecycle = queue.take_parse_time_lifecycle_work();
        assert_eq!(lifecycle.len(), 4);
        assert!(lifecycle.iter().any(|work| {
            matches!(
                work.as_lifecycle_work(),
                Some(PostParseLifecycleWork::RecordDocumentScriptRun { position: 1, .. })
            )
        }));
        assert!(queue.take_parse_time_lifecycle_work().is_empty());
    }

    #[tokio::test]
    async fn parse_time_wait_observes_producer_arrival_without_timing_delay() {
        let mut queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let sender = queue.parse_time_document_script_sender();

        let send = async move {
            sender
                .send(ParseTimeDocumentScriptEvent::async_completion(
                    NodeId::new(88),
                    source_load_outcome_ok("ready"),
                ))
                .unwrap();
        };

        let (arrived, ()) = tokio::join!(
            queue.wait_for_parse_time_injected_task_arrival_without_timeout(),
            send
        );
        assert!(arrived);
        let Some(ParseTimeDocumentScriptEvent::AsyncCompletion(completion)) =
            queue.parse_time_document_script_pop_front()
        else {
            panic!("expected async completion event");
        };
        let (node_id, outcome) = completion.into_parts();
        assert_eq!(node_id, NodeId::new(88));
        assert!(matches!(
            outcome.source_result,
            Ok(ref source) if source == "ready"
        ));
    }

    #[test]
    fn script_event_tasks_do_not_reenter_parse_time_async_lane() {
        assert!(
            !PageTask::script_event(ScriptEventTask::new(ScriptEventKind::Load, "load"))
                .allows_parse_time_scheduler_followup_turn()
        );
        assert!(
            !PageTask::script_event(ScriptEventTask::new(ScriptEventKind::Error, "error"))
                .belongs_to_parse_time_async_lane()
        );
        assert!(
            PageTask::script_event(ScriptEventTask::new(ScriptEventKind::Load, "load"))
                .is_script_event_task()
        );
    }
}
