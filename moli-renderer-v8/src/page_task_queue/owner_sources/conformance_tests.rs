use moli_module_script_tree as module_tree;
use moli_shared_worker::{
    SharedWorkerConnectAction, SharedWorkerDescriptor, SharedWorkerKey, SharedWorkerRegistry,
    SharedWorkerSameSiteCookies,
};
use moli_storage_key::{MoliStorageKey, StoragePartitionRelation};
use std::time::Instant;
use url::Url;

use crate::{
    PageId,
    context_bootstrap::{IndexedDbTaskId, WebCryptoTaskResult},
    document_runtime::DomHandle,
    frame_owner_model::{
        ChildDocumentModuleFetchTarget, DocumentId,
        FrameDocumentDynamicImportTerminalPreparedAction, FrameDocumentDynamicImportTerminalWork,
        FrameDocumentModuleClientEntryId, FrameDocumentModuleClientId,
        FrameDocumentModuleClientRegistration, FrameDocumentModuleClientReservation,
        FrameDocumentModuleDependencyFetchTask, FrameDocumentModuleFetchDisposition,
        FrameDocumentModuleScriptTerminalBatchTask, FrameDocumentModulepreloadEventBinding,
        FrameDocumentModulepreloadFetchTask, FrameDocumentModulepreloadLinkClient,
        FrameDocumentModulepreloadTerminalWork, FrameDocumentStaticDependencyModuleClient,
        FrameDocumentTaskOwner, FrameLaneNavigationCommitTask, FrameRealmId, FrameSchedulerLaneId,
        LocalWindowId,
    },
    module_runtime::{
        ModuleEntryId, ModuleFetchMetadata, ModuleImportPhase, ModuleKind, ModuleMapKey,
        NativeDynamicImportSingleModuleClient, NativeModuleGraphFetchRequest,
        NativeModuleSingleFetchRequest,
    },
    native_bridge::{
        ImageLoadEventId, OwnerDispatchScope, RuntimeObservableContextToken, WindowDocumentOwner,
        WindowDocumentTaskTarget, WindowExecutionContextAccessPolicy,
        WindowExecutionContextIdentity, WindowExecutionContextOwner, WindowTaskTarget,
    },
    page_resource_completion::{
        RendererPageResourceCompletion, RendererPageResourceCompletionSender,
    },
    page_task_queue::{
        MainDocumentMetaRefreshNavigationTask, PageOwnedInternalLoadingTask,
        RendererDedicatedWorkerClientEvent, RendererDedicatedWorkerMessageEvent, RendererOwnerWake,
        RendererPageChildFrameTaskSender, RendererPageChildRealmMaterializationTarget,
        RendererPageDedicatedWorkerClientEventProducer, RendererPageHistoryTraversalSender,
        RendererPageHistoryTraversalTaskId, RendererPageHistoryTraversalTaskKind,
        RendererPageIndexedDbTaskKind, RendererPageIndexedDbTaskSender,
        RendererPageInternalLoadingSender, RendererPageMainDocumentRuntimeAction,
        RendererPageMainDocumentRuntimeSender, RendererPageMediaElementEventSender,
        RendererPageMediaElementEventTaskId, RendererPageMediaElementEventTaskKind,
        RendererPageMessagePortDeliveryProducer, RendererPageMiscPlatformApiSender,
        RendererPageMiscPlatformApiTaskId, RendererPageMiscPlatformApiTaskKind,
        RendererPageModuleReactionEvent, RendererPageModuleReactionSender,
        RendererPageNavigationAndTraversalSender, RendererPageNavigationAndTraversalTask,
        RendererPageNavigationApiTaskId, RendererPageNavigationApiTaskKind,
        RendererPageRenderingUpdateSender, RendererPageRenderingUpdateTaskId,
        RendererPageRenderingUpdateTaskKind, RendererPageSharedWorkerClientEventProducer,
        RendererPageUserInteractionEventKind, RendererPageUserInteractionSender,
        RendererPageUserInteractionTaskId, RendererPageUserInteractionTaskKind,
        RendererPageWebCryptoTaskId, RendererPageWebCryptoTaskSender,
        RendererPageWindowMessageSender, RendererPageWindowMessageTaskId,
    },
    resource_ready::RendererPageTaskReadyMetadata,
    runtime::{RendererDocumentToken, RendererPageToken},
    shared_worker_runtime::SharedWorkerClientEvent,
    types::DocumentWriteExternalScriptLoadCompletion,
};

use super::{
    PageRuntimeWakeSignal, RendererOwnerWakeSender, RendererPageChildFrameTaskSource,
    RendererPageChildModuleDependencyFetchStartSender,
    RendererPageChildModuleDependencyFetchStartSource, RendererPageChildModuleScriptTerminalSender,
    RendererPageChildModuleScriptTerminalSource, RendererPageChildModulepreloadEventActionSender,
    RendererPageChildModulepreloadEventActionSource, RendererPageDedicatedWorkerClientEventSource,
    RendererPageDomManipulationSource, RendererPageDomManipulationTask,
    RendererPageDynamicImportOwnerActionSender, RendererPageDynamicImportOwnerActionSource,
    RendererPageFileReadingSource, RendererPageIndexedDbTaskSource,
    RendererPageInternalLoadingSource, RendererPageMainDocumentRuntimeSource,
    RendererPageMediaElementEventSource, RendererPageMessagePortDeliverySource,
    RendererPageMiscPlatformApiSource, RendererPageModuleReactionSource,
    RendererPageModulepreloadStartSender, RendererPageModulepreloadStartSource,
    RendererPageNavigationAndTraversalSource, RendererPageNetworkingSource,
    RendererPageOwnedTaskSources, RendererPageReadyDescriptor, RendererPageRenderingUpdateSource,
    RendererPageSchedulerTask, RendererPageSharedWorkerClientEventSource,
    RendererPageUserInteractionSource, RendererPageWebCryptoTaskSource,
    RendererPageWindowMessageSource,
};
use crate::page_task_queue::{
    RendererPageBroadcastChannelDeliveryProducer, RendererPageBroadcastChannelDeliverySender,
    RendererPageElementToggleEventCancellation, RendererPageElementToggleEventData,
    RendererPageElementToggleEventKind, RendererPageElementToggleEventSender,
    RendererPageElementToggleEventState, RendererPageElementToggleEventTaskId,
    RendererPageFileEntryFileCallbackSender, RendererPageFileEntryFileCallbackTaskId,
    RendererPageFileEntryFileCallbackTaskKind, RendererPageFileReadingSender,
    RendererPageFileReadingTaskId, RendererPageFileReadingTaskKind, RendererPageHashChangeData,
    RendererPageHashChangeDeliverySender, RendererPageImageLoadEventKind,
    RendererPageImageLoadEventSender, RendererPageImageLoadEventTaskId,
    RendererPageStorageEventData, RendererPageStorageEventDeliverySender,
    RendererPageTextTrackDefaultModeSender, RendererPageTextTrackDefaultModeTaskId,
    RendererPageTextTrackDefaultModeTaskKind,
};

/// Common queue contract for every typed ordinary Page source admitted by the
/// owner scheduler. Lane-specific tests still verify semantic application;
/// this harness owns the repeated FIFO, replacement-route and retirement
/// invariants.
trait TypedPageSourceConformance {
    fn enqueue_initial(&mut self, sequence: u64) -> bool;
    fn enqueue_replacement(&mut self, sequence: u64) -> bool;
    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata>;
    fn take_wake(&mut self) -> Option<RendererOwnerWake>;
    fn retire_consumer(&mut self);
}

fn assert_fifo_replacement_and_route_retirement(mut lane: impl TypedPageSourceConformance) {
    assert!(
        lane.enqueue_initial(1),
        "initial producer payload should be accepted"
    );
    assert!(
        lane.enqueue_replacement(2),
        "replacement producer must reuse the stable Page consumer"
    );
    assert!(
        lane.take_wake().is_some(),
        "the first enqueue must publish the readiness transition"
    );
    assert!(
        lane.take_wake().is_none(),
        "a second enqueue while ready must not duplicate the source wake"
    );

    let first = lane
        .pop_ready_metadata()
        .expect("first accepted payload should remain queued");
    assert!(
        lane.enqueue_initial(3),
        "a producer may append while an older payload remains ready"
    );
    assert!(
        lane.take_wake().is_none(),
        "dequeueing only one of multiple payloads must not rearm the source"
    );
    let second = lane
        .pop_ready_metadata()
        .expect("second accepted payload should remain queued");
    let third = lane
        .pop_ready_metadata()
        .expect("payload appended during the same readiness epoch should remain queued");
    assert!(
        first.order < second.order && second.order < third.order,
        "one source must preserve producer FIFO"
    );
    assert!(lane.pop_ready_metadata().is_none());

    assert!(
        lane.enqueue_replacement(4),
        "a drained source must accept a later readiness epoch"
    );
    assert!(
        lane.take_wake().is_some(),
        "the first enqueue after drain must publish a new wake"
    );
    assert!(lane.take_wake().is_none());
    lane.pop_ready_metadata()
        .expect("the rearmed payload should remain queued");

    lane.retire_consumer();
    assert!(
        !lane.enqueue_initial(3),
        "retiring the unique Page consumer must close the initial producer"
    );
    assert!(
        !lane.enqueue_replacement(4),
        "retiring the unique Page consumer must close the replacement producer"
    );
    assert!(
        lane.take_wake().is_none(),
        "a closed route must not publish a phantom readiness wake"
    );
}

fn document_token(lifecycle_document_id: u64) -> RendererDocumentToken {
    RendererDocumentToken::new_for_testing(PageId::new_for_testing(9101), lifecycle_document_id)
}

struct ResourceCompletionLane {
    source: Option<RendererPageNetworkingSource>,
    initial_sender: RendererPageResourceCompletionSender,
    replacement_sender: RendererPageResourceCompletionSender,
    runtime_wake: PageRuntimeWakeSignal,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct BroadcastChannelDeliveryLane {
    source: Option<RendererPageDomManipulationSource>,
    initial_producer: RendererPageBroadcastChannelDeliveryProducer,
    replacement_producer: RendererPageBroadcastChannelDeliveryProducer,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct HistoryTraversalLane {
    source: Option<RendererPageNavigationAndTraversalSource>,
    initial_sender: RendererPageHistoryTraversalSender,
    replacement_sender: RendererPageHistoryTraversalSender,
    execution_context: WindowExecutionContextIdentity,
    target: WindowTaskTarget,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct RenderingUpdateLane {
    source: Option<RendererPageRenderingUpdateSource>,
    initial_sender: RendererPageRenderingUpdateSender,
    replacement_sender: RendererPageRenderingUpdateSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct MediaElementEventLane {
    source: Option<RendererPageMediaElementEventSource>,
    initial_sender: RendererPageMediaElementEventSender,
    replacement_sender: RendererPageMediaElementEventSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct UserInteractionLane {
    source: Option<RendererPageUserInteractionSource>,
    initial_sender: RendererPageUserInteractionSender,
    replacement_sender: RendererPageUserInteractionSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct FileReadingLane {
    source: Option<RendererPageFileReadingSource>,
    initial_sender: RendererPageFileReadingSender,
    replacement_sender: RendererPageFileReadingSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct MiscPlatformApiLane {
    source: Option<RendererPageMiscPlatformApiSource>,
    initial_sender: RendererPageMiscPlatformApiSender,
    replacement_sender: RendererPageMiscPlatformApiSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct MessagePortDeliveryLane {
    source: Option<RendererPageMessagePortDeliverySource>,
    initial_producer: RendererPageMessagePortDeliveryProducer,
    replacement_producer: RendererPageMessagePortDeliveryProducer,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct DedicatedWorkerClientEventLane {
    source: Option<RendererPageDedicatedWorkerClientEventSource>,
    initial_producer: RendererPageDedicatedWorkerClientEventProducer,
    replacement_producer: RendererPageDedicatedWorkerClientEventProducer,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct SharedWorkerClientEventLane {
    source: Option<RendererPageSharedWorkerClientEventSource>,
    initial_producer: RendererPageSharedWorkerClientEventProducer,
    replacement_producer: RendererPageSharedWorkerClientEventProducer,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct WebCryptoTaskLane {
    source: Option<RendererPageWebCryptoTaskSource>,
    initial_sender: RendererPageWebCryptoTaskSender,
    replacement_sender: RendererPageWebCryptoTaskSender,
    execution_context: WindowExecutionContextIdentity,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct IndexedDbTaskLane {
    source: Option<RendererPageIndexedDbTaskSource>,
    initial_sender: RendererPageIndexedDbTaskSender,
    replacement_sender: RendererPageIndexedDbTaskSender,
    execution_context: WindowExecutionContextIdentity,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct ChildFrameTaskLane {
    source: Option<RendererPageChildFrameTaskSource>,
    initial_sender: RendererPageChildFrameTaskSender,
    replacement_sender: RendererPageChildFrameTaskSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct ModuleReactionLane {
    source: Option<RendererPageModuleReactionSource>,
    initial_sender: RendererPageModuleReactionSender,
    replacement_sender: RendererPageModuleReactionSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct InternalLoadingLane {
    source: Option<RendererPageInternalLoadingSource>,
    initial_sender: RendererPageInternalLoadingSender,
    replacement_sender: RendererPageInternalLoadingSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct MainDocumentRuntimeLane {
    source: Option<RendererPageMainDocumentRuntimeSource>,
    initial_sender: RendererPageMainDocumentRuntimeSender,
    replacement_sender: RendererPageMainDocumentRuntimeSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct ChildModulepreloadEventActionLane {
    source: Option<RendererPageChildModulepreloadEventActionSource>,
    initial_sender: RendererPageChildModulepreloadEventActionSender,
    replacement_sender: RendererPageChildModulepreloadEventActionSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct ChildModuleDependencyFetchStartLane {
    source: Option<RendererPageChildModuleDependencyFetchStartSource>,
    initial_sender: RendererPageChildModuleDependencyFetchStartSender,
    replacement_sender: RendererPageChildModuleDependencyFetchStartSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

struct ChildModuleScriptTerminalLane {
    source: Option<RendererPageChildModuleScriptTerminalSource>,
    initial_sender: RendererPageChildModuleScriptTerminalSender,
    replacement_sender: RendererPageChildModuleScriptTerminalSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

impl ChildModuleScriptTerminalLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source =
            RendererPageChildModuleScriptTerminalSource::new(RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageChildModuleScriptTerminalSender, sequence: u64) -> bool {
        sender
            .send(FrameDocumentModuleScriptTerminalBatchTask::new(
                task_owner(sequence + 1_400),
                FrameRealmId(sequence as i64 + 1_500),
                Vec::new(),
            ))
            .is_ok()
    }
}

impl TypedPageSourceConformance for ChildModuleScriptTerminalLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl ChildModuleDependencyFetchStartLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source =
            RendererPageChildModuleDependencyFetchStartSource::new(RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            wake_rx,
        }
    }

    fn enqueue_with(
        sender: &RendererPageChildModuleDependencyFetchStartSender,
        sequence: u64,
    ) -> bool {
        let (target, task) = module_dependency_fetch_task(sequence);
        sender
            .send(target, task)
            .is_ok_and(|outcome| outcome.was_queued())
    }
}

impl TypedPageSourceConformance for ChildModuleDependencyFetchStartLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl ChildModulepreloadEventActionLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source =
            RendererPageChildModulepreloadEventActionSource::new(RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            wake_rx,
        }
    }

    fn enqueue_with(
        sender: &RendererPageChildModulepreloadEventActionSender,
        sequence: u64,
    ) -> bool {
        sender.send(modulepreload_event_action(sequence)).is_ok()
    }
}

impl TypedPageSourceConformance for ChildModulepreloadEventActionLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl InternalLoadingLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageInternalLoadingSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageInternalLoadingSender, sequence: u64) -> bool {
        sender
            .schedule_at(
                PageOwnedInternalLoadingTask::MetaRefreshNavigation(
                    MainDocumentMetaRefreshNavigationTask::new(
                        task_owner(sequence + 100),
                        0,
                        Url::parse(&format!(
                            "https://page-source-conformance.test/refresh-{sequence}"
                        ))
                        .expect("internal-loading conformance URL"),
                    ),
                ),
                Instant::now(),
            )
            .is_ok()
    }
}

impl MainDocumentRuntimeLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageMainDocumentRuntimeSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageMainDocumentRuntimeSender, sequence: u64) -> bool {
        sender
            .send_for_source_contract_test(
                task_owner(sequence + 200),
                RendererPageMainDocumentRuntimeAction::ContinueRuntimeScriptWork,
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for MainDocumentRuntimeLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl TypedPageSourceConformance for InternalLoadingLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl ModuleReactionLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageModuleReactionSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageModuleReactionSender, sequence: u64) -> bool {
        sender
            .send(
                RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationFulfilled {
                    document_owner: task_owner(sequence),
                    reaction_id: sequence,
                },
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for ModuleReactionLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl ChildFrameTaskLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageChildFrameTaskSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageChildFrameTaskSender, sequence: u64) -> bool {
        sender
            .send_realm_materialization(RendererPageChildRealmMaterializationTarget::new(
                DomHandle::new(sequence as usize + 300),
                task_owner(sequence + 50),
            ))
            .is_ok()
    }
}

impl TypedPageSourceConformance for ChildFrameTaskLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl IndexedDbTaskLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageIndexedDbTaskSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        Self {
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            source: Some(source),
            execution_context: window_execution_context(77),
            wake_rx,
        }
    }

    fn enqueue_with(
        sender: &RendererPageIndexedDbTaskSender,
        execution_context: WindowExecutionContextIdentity,
        sequence: u64,
    ) -> bool {
        sender
            .send(
                execution_context,
                RendererPageIndexedDbTaskKind::RuntimeQueue(IndexedDbTaskId::from_raw(sequence)),
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for IndexedDbTaskLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, self.execution_context, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, self.execution_context, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl WebCryptoTaskLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageWebCryptoTaskSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let initial_sender = route.sender(document_token(1));
        let replacement_sender = route.sender(document_token(2));
        Self {
            source: Some(source),
            initial_sender,
            replacement_sender,
            execution_context: window_execution_context(76),
            wake_rx,
        }
    }

    fn enqueue_with(
        sender: &RendererPageWebCryptoTaskSender,
        execution_context: WindowExecutionContextIdentity,
        sequence: u64,
    ) -> bool {
        let producer = sender.bind_task(
            execution_context,
            RendererPageWebCryptoTaskId::new(sequence),
        );
        assert_eq!(producer.owner().task().task_id(), sequence);
        producer.send(Ok(WebCryptoTaskResult::Bool(true))).is_ok()
    }
}

impl TypedPageSourceConformance for WebCryptoTaskLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, self.execution_context, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, self.execution_context, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

fn allocated_shared_worker_client_id() -> moli_shared_worker::SharedWorkerClientId {
    let registry = SharedWorkerRegistry::<()>::default();
    let key = SharedWorkerKey::new(
        MoliStorageKey::new(
            "https://page-source-conformance.test".to_owned(),
            "https://page-source-conformance.test".to_owned(),
            None,
            StoragePartitionRelation::FirstParty,
        ),
        "https://page-source-conformance.test/shared-worker.js".to_owned(),
        "conformance".to_owned(),
        SharedWorkerSameSiteCookies::All,
    );
    match registry.connect(key, SharedWorkerDescriptor::default()) {
        SharedWorkerConnectAction::StartLoading { client_id, .. } => client_id,
        other => panic!("fresh SharedWorker registry should allocate a loading client: {other:?}"),
    }
}

impl SharedWorkerClientEventLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageSharedWorkerClientEventSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let execution_context = window_execution_context(75);
        let client_id = allocated_shared_worker_client_id();
        let initial_producer = route
            .sender(document_token(1))
            .bind_execution_context(execution_context)
            .bind_client(client_id);
        let replacement_producer = route
            .sender(document_token(2))
            .bind_execution_context(execution_context)
            .bind_client(client_id);
        Self {
            source: Some(source),
            initial_producer,
            replacement_producer,
            wake_rx,
        }
    }
}

impl TypedPageSourceConformance for SharedWorkerClientEventLane {
    fn enqueue_initial(&mut self, _sequence: u64) -> bool {
        self.initial_producer
            .send(SharedWorkerClientEvent::Closed)
            .is_ok()
    }

    fn enqueue_replacement(&mut self, _sequence: u64) -> bool {
        self.replacement_producer
            .send(SharedWorkerClientEvent::Closed)
            .is_ok()
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl DedicatedWorkerClientEventLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source =
            RendererPageDedicatedWorkerClientEventSource::new(RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ));
        let route = source.route();
        let execution_context = window_execution_context(74);
        let worker_id = crate::types::DedicatedWorkerId::new(9);
        let initial_producer = route
            .sender(document_token(1))
            .bind_worker(execution_context, worker_id);
        let replacement_producer = route
            .sender(document_token(2))
            .bind_worker(execution_context, worker_id);
        assert_eq!(initial_producer.owner().root_document(), document_token(1));
        assert_eq!(
            replacement_producer.owner().root_document(),
            document_token(2)
        );
        assert_ne!(initial_producer.owner(), replacement_producer.owner());
        Self {
            source: Some(source),
            initial_producer,
            replacement_producer,
            wake_rx,
        }
    }
}

impl TypedPageSourceConformance for DedicatedWorkerClientEventLane {
    fn enqueue_initial(&mut self, _sequence: u64) -> bool {
        self.initial_producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                RendererDedicatedWorkerMessageEvent::Message(Default::default()),
            ))
            .is_ok()
    }

    fn enqueue_replacement(&mut self, _sequence: u64) -> bool {
        self.replacement_producer
            .send(RendererDedicatedWorkerClientEvent::Message(
                RendererDedicatedWorkerMessageEvent::Message(Default::default()),
            ))
            .is_ok()
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

struct WindowMessageLane {
    source: Option<RendererPageWindowMessageSource>,
    initial_sender: RendererPageWindowMessageSender,
    replacement_sender: RendererPageWindowMessageSender,
    target: WindowTaskTarget,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

impl WindowMessageLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageWindowMessageSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let initial_sender = route.sender(document_token(1));
        let replacement_sender = route.sender(document_token(2));
        let target = WindowTaskTarget::new(
            OwnerDispatchScope::Top,
            WindowExecutionContextOwner::Frame(LocalWindowId(73)),
        );
        Self {
            source: Some(source),
            initial_sender,
            replacement_sender,
            target,
            wake_rx,
        }
    }
}

impl TypedPageSourceConformance for WindowMessageLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        self.initial_sender
            .send(
                self.target,
                RendererPageWindowMessageTaskId::from_raw(sequence),
            )
            .is_ok()
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        self.replacement_sender
            .send(
                self.target,
                RendererPageWindowMessageTaskId::from_raw(sequence),
            )
            .is_ok()
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl HistoryTraversalLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageNavigationAndTraversalSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let execution_context = window_execution_context(76);
        let target = WindowTaskTarget::new(OwnerDispatchScope::Top, execution_context.owner());
        Self {
            source: Some(source),
            initial_sender: super::RendererPageNavigationAndTraversalSender::new(
                route.clone(),
                document_token(1),
            )
            .history_traversal(),
            replacement_sender: super::RendererPageNavigationAndTraversalSender::new(
                route,
                document_token(2),
            )
            .history_traversal(),
            execution_context,
            target,
            wake_rx,
        }
    }

    fn enqueue_with(
        sender: &RendererPageHistoryTraversalSender,
        execution_context: WindowExecutionContextIdentity,
        target: WindowTaskTarget,
        sequence: u64,
    ) -> bool {
        let kind = if sequence.is_multiple_of(2) {
            RendererPageHistoryTraversalTaskKind::ChildCrossDocument
        } else {
            RendererPageHistoryTraversalTaskKind::SameDocument
        };
        sender
            .bind_task(
                execution_context,
                target,
                RendererPageHistoryTraversalTaskId::from_raw(sequence),
                kind,
            )
            .send()
            .is_ok()
    }
}

impl TypedPageSourceConformance for HistoryTraversalLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(
            &self.initial_sender,
            self.execution_context,
            self.target,
            sequence,
        )
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(
            &self.replacement_sender,
            self.execution_context,
            self.target,
            sequence,
        )
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl RenderingUpdateLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageRenderingUpdateSource::new(
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ),
            RendererOwnerWakeSender::signal_rendering_update_task,
        );
        let route = source.route();
        Self {
            source: Some(source),
            initial_sender: RendererPageRenderingUpdateSender::new(
                route.clone(),
                document_token(1),
            ),
            replacement_sender: RendererPageRenderingUpdateSender::new(route, document_token(2)),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageRenderingUpdateSender, sequence: u64) -> bool {
        sender
            .send(
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(task_owner(sequence + 1_600)),
                    OwnerDispatchScope::Top,
                ),
                RendererPageRenderingUpdateTaskId::from_raw(sequence),
                RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for RenderingUpdateLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl MediaElementEventLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageMediaElementEventSource::new(
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ),
            RendererOwnerWakeSender::signal_media_element_event_task,
        );
        let route = source.route();
        Self {
            source: Some(source),
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageMediaElementEventSender, sequence: u64) -> bool {
        sender
            .send(
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(task_owner(sequence + 1_650)),
                    OwnerDispatchScope::Top,
                ),
                RendererPageMediaElementEventTaskId::from_raw(sequence),
                match sequence % 4 {
                    0 => RendererPageMediaElementEventTaskKind::Seeking,
                    1 => RendererPageMediaElementEventTaskKind::SeekCompletion,
                    2 => RendererPageMediaElementEventTaskKind::LoadEventPhase,
                    _ => RendererPageMediaElementEventTaskKind::TextTrackListEvent,
                },
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for MediaElementEventLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl UserInteractionLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageUserInteractionSource::new(
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ),
            RendererOwnerWakeSender::signal_user_interaction_task,
        );
        let route = source.route();
        Self {
            source: Some(source),
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageUserInteractionSender, sequence: u64) -> bool {
        sender
            .send(
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(task_owner(sequence + 1_700)),
                    OwnerDispatchScope::Top,
                ),
                RendererPageUserInteractionTaskId::from_raw(sequence),
                match sequence % 4 {
                    0 => RendererPageUserInteractionTaskKind::Event(
                        RendererPageUserInteractionEventKind::DialogClose,
                    ),
                    1 => RendererPageUserInteractionTaskKind::Event(
                        RendererPageUserInteractionEventKind::DocumentSelectionChange,
                    ),
                    2 => RendererPageUserInteractionTaskKind::Event(
                        RendererPageUserInteractionEventKind::TextControlSelectionChange,
                    ),
                    3 => RendererPageUserInteractionTaskKind::Event(
                        RendererPageUserInteractionEventKind::TextControlSelect,
                    ),
                    _ => RendererPageUserInteractionTaskKind::DataTransferGetAsString,
                },
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for UserInteractionLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl FileReadingLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageFileReadingSource::new(
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ),
            RendererOwnerWakeSender::signal_file_reading_task,
        );
        let route = source.route();
        Self {
            source: Some(source),
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageFileReadingSender, sequence: u64) -> bool {
        sender
            .send(
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(task_owner(sequence + 1_725)),
                    OwnerDispatchScope::Top,
                ),
                RendererPageFileReadingTaskId::from_raw(sequence),
                match sequence % 4 {
                    0 => RendererPageFileReadingTaskKind::DirectoryBatch,
                    1 => RendererPageFileReadingTaskKind::DirectoryTerminalEmpty,
                    2 => RendererPageFileReadingTaskKind::DirectoryOverlappingReadError,
                    _ => RendererPageFileReadingTaskKind::DirectoryTerminalError,
                },
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for FileReadingLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl MiscPlatformApiLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageMiscPlatformApiSource::new(
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(document_token(1).page_id),
            ),
            RendererOwnerWakeSender::signal_misc_platform_api_task,
        );
        let route = source.route();
        Self {
            source: Some(source),
            initial_sender: route.sender(document_token(1)),
            replacement_sender: route.sender(document_token(2)),
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageMiscPlatformApiSender, sequence: u64) -> bool {
        sender
            .send(
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(task_owner(sequence + 1_730)),
                    OwnerDispatchScope::Top,
                ),
                RendererPageMiscPlatformApiTaskId::from_raw(sequence),
                match sequence % 3 {
                    0 => RendererPageMiscPlatformApiTaskKind::LegacyStorageUsageAndQuota,
                    1 => RendererPageMiscPlatformApiTaskKind::LegacyStorageGrantedQuota,
                    _ => RendererPageMiscPlatformApiTaskKind::LegacyStorageError,
                },
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for MiscPlatformApiLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl BroadcastChannelDeliveryLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageDomManipulationSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let initial_sender =
            RendererPageBroadcastChannelDeliverySender::new(route.clone(), document_token(1));
        let replacement_sender =
            RendererPageBroadcastChannelDeliverySender::new(route, document_token(2));
        assert!(initial_sender.same_route_as(&replacement_sender));
        let execution_context = window_execution_context(71);
        let initial_producer = initial_sender.bind_execution_context(execution_context);
        let replacement_producer = replacement_sender.bind_execution_context(execution_context);
        assert_ne!(initial_producer.owner(), replacement_producer.owner());
        Self {
            source: Some(source),
            initial_producer,
            replacement_producer,
            wake_rx,
        }
    }
}

impl TypedPageSourceConformance for BroadcastChannelDeliveryLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        self.initial_producer.send(sequence).is_ok()
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        self.replacement_producer.send(sequence).is_ok()
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl MessagePortDeliveryLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageMessagePortDeliverySource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let execution_context = window_execution_context(72);
        let initial_producer = route
            .sender(document_token(1))
            .bind_execution_context(execution_context);
        let replacement_producer = route
            .sender(document_token(2))
            .bind_execution_context(execution_context);
        assert_ne!(initial_producer.owner(), replacement_producer.owner());
        Self {
            source: Some(source),
            initial_producer,
            replacement_producer,
            wake_rx,
        }
    }
}

impl TypedPageSourceConformance for MessagePortDeliveryLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        self.initial_producer.send(sequence).is_ok()
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        self.replacement_producer.send(sequence).is_ok()
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

impl ResourceCompletionLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        );
        let runtime_wake = PageRuntimeWakeSignal::default();
        let source =
            RendererPageNetworkingSource::new_owner_attached(runtime_wake.clone(), owner_wake);
        let initial_sender = source.sender();
        let replacement_sender = source.sender();
        Self {
            source: Some(source),
            initial_sender,
            replacement_sender,
            runtime_wake,
            wake_rx,
        }
    }

    fn enqueue_with(sender: &RendererPageResourceCompletionSender, sequence: u64) -> bool {
        sender
            .send(
                RendererPageResourceCompletion::document_write_external_script(
                    document_token(sequence),
                    DocumentWriteExternalScriptLoadCompletion::for_test(sequence),
                ),
            )
            .is_ok()
    }
}

impl TypedPageSourceConformance for ResourceCompletionLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.initial_sender, sequence)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        Self::enqueue_with(&self.replacement_sender, sequence)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        let owner_wake = self.wake_rx.try_recv().ok();
        assert_eq!(
            self.runtime_wake.take_ready(),
            owner_wake.is_some(),
            "resource runtime and owner wakes must share one readiness transition",
        );
        owner_wake
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

struct ModulepreloadStartLane {
    source: Option<RendererPageModulepreloadStartSource>,
    initial_sender: RendererPageModulepreloadStartSender,
    replacement_sender: RendererPageModulepreloadStartSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

impl ModulepreloadStartLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageModulepreloadStartSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let initial_sender = route.sender(document_token(1));
        let replacement_sender = route.sender(document_token(2));
        Self {
            source: Some(source),
            initial_sender,
            replacement_sender,
            wake_rx,
        }
    }
}

impl TypedPageSourceConformance for ModulepreloadStartLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        self.initial_sender
            .send(modulepreload_task(sequence))
            .is_ok()
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        self.replacement_sender
            .send(modulepreload_task(sequence))
            .is_ok()
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

struct DynamicImportOwnerActionLane {
    source: Option<RendererPageDynamicImportOwnerActionSource>,
    initial_sender: RendererPageDynamicImportOwnerActionSender,
    replacement_sender: RendererPageDynamicImportOwnerActionSender,
    wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

impl DynamicImportOwnerActionLane {
    fn new() -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageDynamicImportOwnerActionSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(document_token(1).page_id),
        ));
        let route = source.route();
        let initial_sender = route.sender(document_token(1));
        let replacement_sender = route.sender(document_token(2));
        Self {
            source: Some(source),
            initial_sender,
            replacement_sender,
            wake_rx,
        }
    }
}

impl TypedPageSourceConformance for DynamicImportOwnerActionLane {
    fn enqueue_initial(&mut self, sequence: u64) -> bool {
        self.initial_sender
            .send_all(vec![dynamic_import_action(sequence)])
            .is_ok_and(|queued| queued)
    }

    fn enqueue_replacement(&mut self, sequence: u64) -> bool {
        self.replacement_sender
            .send_all(vec![dynamic_import_action(sequence)])
            .is_ok_and(|queued| queued)
    }

    fn pop_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.as_mut()?.pop_front().map(|(ready, _)| ready)
    }

    fn take_wake(&mut self) -> Option<RendererOwnerWake> {
        self.wake_rx.try_recv().ok()
    }

    fn retire_consumer(&mut self) {
        drop(self.source.take());
    }
}

fn task_owner(seed: u64) -> FrameDocumentTaskOwner {
    FrameDocumentTaskOwner::new(
        FrameSchedulerLaneId(seed),
        LocalWindowId(seed + 1),
        DocumentId(seed + 2),
    )
}

fn window_execution_context(seed: u64) -> WindowExecutionContextIdentity {
    WindowExecutionContextIdentity::new(
        WindowExecutionContextOwner::Frame(LocalWindowId(seed)),
        OwnerDispatchScope::Top,
        RuntimeObservableContextToken::from_raw(seed + 1),
        WindowExecutionContextAccessPolicy::EnforceWebOrigin,
    )
}

fn modulepreload_task(sequence: u64) -> FrameDocumentModulepreloadFetchTask {
    let child_handle = DomHandle::new(sequence as usize + 100);
    let source_url = Url::parse(&format!(
        "https://page-source-conformance.test/modulepreload-{sequence}.mjs"
    ))
    .expect("modulepreload conformance URL");
    FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        task_owner(sequence + 10),
        FrameRealmId(sequence as i64 + 20),
        FrameDocumentModulepreloadLinkClient::new(
            child_handle,
            DomHandle::new(sequence as usize + 200),
        ),
        NativeModuleSingleFetchRequest::new(
            source_url.clone(),
            source_url.clone(),
            source_url.clone(),
            ModuleMapKey::java_script(source_url),
            ModuleFetchMetadata::default(),
        ),
    )
}

fn module_dependency_fetch_task(
    sequence: u64,
) -> (
    ChildDocumentModuleFetchTarget,
    FrameDocumentModuleDependencyFetchTask,
) {
    let owner = task_owner(sequence + 700);
    let realm_id = FrameRealmId(sequence as i64 + 800);
    let child_handle = DomHandle::new(sequence as usize + 900);
    let parent_url = Url::parse(&format!(
        "https://page-source-conformance.test/root-{sequence}.mjs"
    ))
    .expect("dependency parent URL");
    let dependency_url = Url::parse(&format!(
        "https://page-source-conformance.test/dependency-{sequence}.mjs"
    ))
    .expect("dependency URL");
    let parent_key = ModuleMapKey::java_script(parent_url.clone());
    let dependency_key = ModuleMapKey::java_script(dependency_url.clone());
    let parent_entry_id = ModuleEntryId::from_raw(sequence as u32 + 1000);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(sequence + 1100),
        sequence,
    };
    let client = FrameDocumentStaticDependencyModuleClient::new(
        parent_entry_id,
        parent_key.clone(),
        "./dependency.mjs".to_owned(),
        ModuleImportPhase::Evaluation,
        tree_client,
    );
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(sequence as u32 + 1200);
    let reservation = FrameDocumentModuleClientReservation::new(
        owner.document_owner(),
        dependency_key.clone(),
        FrameDocumentModuleClientRegistration::new(
            entry_id,
            FrameDocumentModuleClientId::from_raw(sequence + 1300),
            FrameDocumentModuleFetchDisposition::StartedFetch(entry_id),
        ),
    );
    let task = FrameDocumentModuleDependencyFetchTask::from_dependency_fetch_parts(
        owner,
        realm_id,
        dependency_key.clone(),
        client,
        reservation,
        NativeModuleGraphFetchRequest::new_tree_dependency_for_test(
            dependency_url,
            parent_url,
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
            tree_client,
            dependency_key,
            parent_key,
            parent_entry_id,
            "./dependency.mjs".to_owned(),
            ModuleImportPhase::Evaluation,
        ),
    );
    (
        ChildDocumentModuleFetchTarget::new(child_handle, owner, realm_id),
        task,
    )
}

fn modulepreload_event_action(
    sequence: u64,
) -> crate::frame_owner_model::FrameDocumentModulepreloadEventAction {
    let client = FrameDocumentModulepreloadLinkClient::new(
        DomHandle::new(sequence as usize + 400),
        DomHandle::new(sequence as usize + 500),
    );
    FrameDocumentModulepreloadTerminalWork::from_link_error_parts(
        task_owner(sequence + 60),
        FrameRealmId(sequence as i64 + 70),
        client,
    )
    .into_event_action(FrameDocumentModulepreloadEventBinding::new(client, None))
}

fn dynamic_import_action(sequence: u64) -> FrameDocumentDynamicImportTerminalPreparedAction {
    let key = ModuleMapKey::java_script(
        Url::parse(&format!(
            "https://page-source-conformance.test/dynamic-{sequence}.mjs"
        ))
        .expect("dynamic-import conformance URL"),
    );
    let client = NativeDynamicImportSingleModuleClient::new(
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(sequence),
            sequence,
        },
        module_tree::ModuleImportPhase::Evaluation,
    );
    FrameDocumentDynamicImportTerminalPreparedAction::from_terminal_work(
        FrameDocumentDynamicImportTerminalWork::from_terminal_parts(
            task_owner(sequence + 30),
            FrameRealmId(sequence as i64 + 40),
            key,
            client,
        ),
    )
}

#[test]
fn resource_completion_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(ResourceCompletionLane::new());
}

#[test]
fn broadcast_channel_delivery_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(BroadcastChannelDeliveryLane::new());
}

#[test]
fn history_traversal_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(HistoryTraversalLane::new());
}

#[test]
fn rendering_update_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(RenderingUpdateLane::new());
}

#[test]
fn media_element_event_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(MediaElementEventLane::new());
}

#[test]
fn user_interaction_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(UserInteractionLane::new());
}

#[test]
fn file_reading_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(FileReadingLane::new());
}

#[test]
fn misc_platform_api_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(MiscPlatformApiLane::new());
}

#[test]
fn navigation_and_traversal_source_preserves_cross_kind_fifo_and_one_readiness_epoch() {
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut source = RendererPageNavigationAndTraversalSource::new(RendererOwnerWakeSender::new(
        wake_tx,
        RendererPageToken::new_for_testing(document_token(1).page_id),
    ));
    let route = source.route();
    let family = RendererPageNavigationAndTraversalSender::new(route, document_token(1));
    let history = family.history_traversal();
    let child_navigation = family.child_navigation_commit();
    let navigation = family.navigation_api_task();
    let execution_context = window_execution_context(77);
    let target = WindowTaskTarget::new(OwnerDispatchScope::Top, execution_context.owner());

    history
        .bind_task(
            execution_context,
            target,
            RendererPageHistoryTraversalTaskId::from_raw(1),
            RendererPageHistoryTraversalTaskKind::SameDocument,
        )
        .send()
        .expect("history traversal should enqueue first");
    let child_commit =
        FrameLaneNavigationCommitTask::for_test(DomHandle::new(78), task_owner(79), 80);
    child_navigation
        .send(child_commit)
        .expect("child navigation commit should share the ready source");
    navigation
        .bind_task(
            execution_context,
            RendererPageNavigationApiTaskId::from_raw(2),
            RendererPageNavigationApiTaskKind::FinishResult,
        )
        .send()
        .expect("Navigation API task should share the ready source");
    history
        .bind_task(
            execution_context,
            target,
            RendererPageHistoryTraversalTaskId::from_raw(3),
            RendererPageHistoryTraversalTaskKind::ChildCrossDocument,
        )
        .send()
        .expect("second traversal should enqueue third");

    assert!(wake_rx.try_recv().is_ok(), "empty to ready must wake once");
    assert!(
        wake_rx.try_recv().is_err(),
        "one navigation-and-traversal source must not publish per-kind wakes"
    );
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageNavigationAndTraversalTask::HistoryTraversal(task))
            if task.task_id() == RendererPageHistoryTraversalTaskId::from_raw(1)
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageNavigationAndTraversalTask::ChildNavigationCommit(task))
            if task.owner().commit() == child_commit
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageNavigationAndTraversalTask::NavigationApi(task))
            if task.task_id() == RendererPageNavigationApiTaskId::from_raw(2)
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageNavigationAndTraversalTask::HistoryTraversal(task))
            if task.task_id() == RendererPageHistoryTraversalTaskId::from_raw(3)
    ));
    assert!(!source.has_ready_task());
    drop(source);
    assert!(
        child_navigation.send(child_commit).is_err(),
        "retired navigation-and-traversal consumer must close the child-navigation route"
    );
    assert!(
        navigation
            .bind_task(
                execution_context,
                RendererPageNavigationApiTaskId::from_raw(4),
                RendererPageNavigationApiTaskKind::FinishResult,
            )
            .send()
            .is_err(),
        "retired navigation-and-traversal consumer must close every derived route"
    );
}

#[test]
fn dom_manipulation_source_preserves_cross_api_fifo_and_one_readiness_epoch() {
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut source = RendererPageDomManipulationSource::new(RendererOwnerWakeSender::new(
        wake_tx,
        RendererPageToken::new_for_testing(document_token(1).page_id),
    ));
    let route = source.route();
    let broadcast =
        RendererPageBroadcastChannelDeliverySender::new(route.clone(), document_token(1))
            .bind_execution_context(window_execution_context(73));
    let storage = RendererPageStorageEventDeliverySender::new(route.clone(), document_token(1));
    let hash_change = RendererPageHashChangeDeliverySender::new(route.clone(), document_token(1));
    let image = RendererPageImageLoadEventSender::new(route.clone(), document_token(1));
    let text_track = RendererPageTextTrackDefaultModeSender::new(route.clone(), document_token(1));
    let file_entry = RendererPageFileEntryFileCallbackSender::new(route, document_token(1));
    let storage_target = WindowTaskTarget::new(
        OwnerDispatchScope::Child(DomHandle::new(74)),
        WindowExecutionContextOwner::Frame(LocalWindowId(75)),
    );
    let image_target = WindowDocumentTaskTarget::new(
        WindowDocumentOwner::Frame(task_owner(76)),
        OwnerDispatchScope::Child(DomHandle::new(77)),
    );

    broadcast.send(11).expect("first DOM task should enqueue");
    storage
        .send(
            storage_target,
            RendererPageStorageEventData::new(
                "https://page-source-conformance.test/source".to_owned(),
                false,
                Some(vec![b'k' as u16]),
                None,
                Some(vec![b'v' as u16]),
            ),
        )
        .expect("StorageEvent should share the ready DOM source");
    image
        .send(
            image_target,
            RendererPageImageLoadEventTaskId::new(DomHandle::new(78), ImageLoadEventId::new(79)),
            RendererPageImageLoadEventKind::Load,
        )
        .expect("image load should share the ready DOM source");
    text_track
        .send(
            image_target,
            RendererPageTextTrackDefaultModeTaskId::from_raw(80),
            RendererPageTextTrackDefaultModeTaskKind::Apply,
        )
        .expect("text-track default mode should share the ready DOM source");
    file_entry
        .send(
            image_target,
            RendererPageFileEntryFileCallbackTaskId::from_raw(81),
            RendererPageFileEntryFileCallbackTaskKind::Success,
        )
        .expect("FileEntry callback should share the ready DOM source");
    hash_change
        .send(
            storage_target,
            RendererPageHashChangeData::new(
                "https://page-source-conformance.test/#before".to_owned(),
                "https://page-source-conformance.test/#after".to_owned(),
            ),
        )
        .expect("hashchange should share the ready DOM source");
    broadcast.send(12).expect("seventh DOM task should enqueue");

    assert!(wake_rx.try_recv().is_ok(), "empty to ready must wake once");
    assert!(
        wake_rx.try_recv().is_err(),
        "one ready source must not publish per-API wakes"
    );
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::BroadcastChannel(task)) if task.channel_id() == 11
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::StorageEvent(task))
            if task.owner().target() == storage_target
                && task.owner().root_document() == document_token(1)
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::ImageLoadEvent(task))
            if task.owner().target() == image_target
                && task.owner().root_document() == document_token(1)
                && task.task_id().sequence() == ImageLoadEventId::new(79)
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::TextTrackDefaultMode(task))
            if task.owner().target() == image_target
                && task.owner().root_document() == document_token(1)
                && task.task_id() == RendererPageTextTrackDefaultModeTaskId::from_raw(80)
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::FileEntryFileCallback(task))
            if task.owner().target() == image_target
                && task.owner().root_document() == document_token(1)
                && task.task_id() == RendererPageFileEntryFileCallbackTaskId::from_raw(81)
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::HashChange(task))
            if task.owner().target() == storage_target
                && task.owner().root_document() == document_token(1)
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::BroadcastChannel(task)) if task.channel_id() == 12
    ));
    assert!(!source.has_ready_task());
    drop(source);
    assert!(
        storage
            .send(
                storage_target,
                RendererPageStorageEventData::new(
                    "https://page-source-conformance.test/retired".to_owned(),
                    false,
                    None,
                    None,
                    None,
                ),
            )
            .is_err(),
        "a retired shared DOM consumer must close StorageEvent routes without fallback"
    );
    assert!(
        hash_change
            .send(
                storage_target,
                RendererPageHashChangeData::new(
                    "https://page-source-conformance.test/#old".to_owned(),
                    "https://page-source-conformance.test/#new".to_owned(),
                ),
            )
            .is_err(),
        "a retired shared DOM consumer must close hashchange routes without fallback"
    );
    assert!(
        image
            .send(
                image_target,
                RendererPageImageLoadEventTaskId::new(
                    DomHandle::new(78),
                    ImageLoadEventId::new(80),
                ),
                RendererPageImageLoadEventKind::Error,
            )
            .is_err(),
        "a retired shared DOM consumer must close image routes without fallback"
    );
    assert!(
        text_track
            .send(
                image_target,
                RendererPageTextTrackDefaultModeTaskId::from_raw(81),
                RendererPageTextTrackDefaultModeTaskKind::Apply,
            )
            .is_err(),
        "a retired shared DOM consumer must close text-track routes without fallback"
    );
    assert!(
        wake_rx.try_recv().is_err(),
        "a closed route must not publish phantom DOM readiness"
    );
}

#[test]
fn dom_manipulation_source_reposts_a_cancelled_toggle_at_the_cross_api_fifo_tail() {
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut source = RendererPageDomManipulationSource::new(RendererOwnerWakeSender::new(
        wake_tx,
        RendererPageToken::new_for_testing(document_token(1).page_id),
    ));
    let route = source.route();
    let toggle = RendererPageElementToggleEventSender::new(route.clone(), document_token(1));
    let storage = RendererPageStorageEventDeliverySender::new(route, document_token(1));
    let target = WindowDocumentTaskTarget::new(
        WindowDocumentOwner::Frame(task_owner(73)),
        OwnerDispatchScope::Top,
    );
    let storage_target = WindowTaskTarget::new(
        OwnerDispatchScope::Top,
        WindowExecutionContextOwner::Frame(LocalWindowId(74)),
    );
    let cancelled = RendererPageElementToggleEventCancellation::new();

    toggle
        .send(
            target,
            RendererPageElementToggleEventTaskId::from_raw(1),
            RendererPageElementToggleEventKind::Details,
            RendererPageElementToggleEventData::new(
                DomHandle::new(75),
                RendererPageElementToggleEventState::Closed,
                RendererPageElementToggleEventState::Open,
                None,
            ),
            cancelled.clone(),
        )
        .expect("initial toggle closure should enqueue");
    storage
        .send(
            storage_target,
            RendererPageStorageEventData::new(
                "https://page-source-conformance.test/storage".to_owned(),
                false,
                None,
                None,
                None,
            ),
        )
        .expect("another DOM API should enqueue behind the old toggle closure");
    cancelled.cancel();
    toggle
        .send(
            target,
            RendererPageElementToggleEventTaskId::from_raw(2),
            RendererPageElementToggleEventKind::Details,
            RendererPageElementToggleEventData::new(
                DomHandle::new(75),
                RendererPageElementToggleEventState::Closed,
                RendererPageElementToggleEventState::Closed,
                None,
            ),
            RendererPageElementToggleEventCancellation::new(),
        )
        .expect("coalesced toggle replacement should append at the source tail");

    assert!(
        wake_rx.try_recv().is_ok(),
        "initial readiness must wake once"
    );
    assert!(
        wake_rx.try_recv().is_err(),
        "cancellation and reposting during one ready epoch must not duplicate wakes"
    );
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::StorageEvent(task))
            if task.owner().target() == storage_target
    ));
    assert!(matches!(
        source.pop_front().map(|(_, task)| task),
        Some(RendererPageDomManipulationTask::ElementToggle(task))
            if task.task_id() == RendererPageElementToggleEventTaskId::from_raw(2)
    ));
    assert!(
        !source.has_ready_task(),
        "the cancelled old closure must be maintenance, not a visible task turn"
    );
    drop(source);
    assert!(
        toggle
            .send(
                target,
                RendererPageElementToggleEventTaskId::from_raw(3),
                RendererPageElementToggleEventKind::Details,
                RendererPageElementToggleEventData::new(
                    DomHandle::new(75),
                    RendererPageElementToggleEventState::Closed,
                    RendererPageElementToggleEventState::Open,
                    None,
                ),
                RendererPageElementToggleEventCancellation::new(),
            )
            .is_err(),
        "retiring the shared DOM consumer must close the toggle route without fallback"
    );
    assert!(
        wake_rx.try_recv().is_err(),
        "a closed toggle route must not publish phantom readiness"
    );
}

#[test]
fn message_port_delivery_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(MessagePortDeliveryLane::new());
}

#[test]
fn dedicated_worker_client_event_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(DedicatedWorkerClientEventLane::new());
}

#[test]
fn shared_worker_client_event_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(SharedWorkerClientEventLane::new());
}

#[test]
fn webcrypto_task_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(WebCryptoTaskLane::new());
}

#[test]
fn indexed_db_task_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(IndexedDbTaskLane::new());
}

#[test]
fn internal_loading_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(InternalLoadingLane::new());
}

#[test]
fn main_document_runtime_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(MainDocumentRuntimeLane::new());
}

#[test]
fn child_module_dependency_fetch_start_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(ChildModuleDependencyFetchStartLane::new());
}

#[test]
fn child_module_script_terminal_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(ChildModuleScriptTerminalLane::new());
}

#[test]
fn child_module_script_terminal_closed_route_returns_the_original_exact_task() {
    let mut lane = ChildModuleScriptTerminalLane::new();
    lane.retire_consumer();
    let owner = task_owner(1_600);
    let realm_id = FrameRealmId(1_700);
    let terminal = FrameDocumentModuleScriptTerminalBatchTask::new(owner, realm_id, Vec::new());

    let rejected = lane
        .initial_sender
        .send(terminal)
        .expect_err("retired terminal source must reject its payload")
        .into_terminal();

    assert_eq!(rejected.owner(), owner);
    assert_eq!(rejected.realm_id(), realm_id);
    assert!(rejected.into_payload().is_empty());
    assert!(lane.take_wake().is_none());
}

#[test]
fn child_module_dependency_fetch_start_source_deduplicates_only_while_pending() {
    let mut lane = ChildModuleDependencyFetchStartLane::new();
    assert!(lane.enqueue_initial(1));
    assert!(
        !lane.enqueue_initial(1),
        "one exact module client must occupy at most one pending FIFO position"
    );
    assert!(lane.take_wake().is_some());
    assert!(lane.take_wake().is_none());
    lane.pop_ready_metadata()
        .expect("the unique dependency start should remain queued");
    assert!(lane.pop_ready_metadata().is_none());

    assert!(
        lane.enqueue_initial(1),
        "consuming the old head must release its pending-only uniqueness key"
    );
    assert!(lane.take_wake().is_some());
    assert!(lane.take_wake().is_none());
}

#[test]
fn child_module_dependency_fetch_start_clear_releases_pending_uniqueness() {
    let mut lane = ChildModuleDependencyFetchStartLane::new();
    assert!(lane.enqueue_initial(9));
    lane.source
        .as_mut()
        .expect("dependency-start source should still be live")
        .clear();

    assert!(
        lane.enqueue_initial(9),
        "clearing a retired task must also release its pending-only key"
    );
    assert!(lane.pop_ready_metadata().is_some());
    assert!(lane.pop_ready_metadata().is_none());
}

#[test]
fn child_module_dependency_fetch_start_closed_route_returns_the_rejected_task() {
    let mut lane = ChildModuleDependencyFetchStartLane::new();
    lane.retire_consumer();
    let (target, task) = module_dependency_fetch_task(17);
    let expected = task.clone();

    let rejected = lane
        .initial_sender
        .send(target, task)
        .expect_err("retired dependency-start source must reject its payload")
        .into_task();

    assert_eq!(rejected, expected);
    assert!(lane.take_wake().is_none());
}

#[test]
fn child_modulepreload_event_action_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(ChildModulepreloadEventActionLane::new());
}

#[test]
fn child_frame_task_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(ChildFrameTaskLane::new());
}

#[test]
fn module_reaction_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(ModuleReactionLane::new());
}

#[test]
fn child_realm_retirement_reconsiders_one_queued_task_without_duplicating_it() {
    let mut lane = ChildFrameTaskLane::new();
    assert!(lane.enqueue_initial(1));
    assert!(lane.take_wake().is_some());
    assert!(lane.take_wake().is_none());

    lane.initial_sender.signal_reconsideration();
    assert!(lane.take_wake().is_some());
    assert!(lane.take_wake().is_none());
    assert!(lane.pop_ready_metadata().is_some());
    assert!(lane.pop_ready_metadata().is_none());
}

#[test]
fn indexed_db_reconsideration_wakes_a_stale_ready_head_without_duplicating_it() {
    let mut lane = IndexedDbTaskLane::new();
    assert!(lane.enqueue_initial(1));
    assert!(lane.take_wake().is_some());
    assert!(lane.take_wake().is_none());

    lane.initial_sender.signal_reconsideration();
    assert!(lane.take_wake().is_some());
    assert!(lane.take_wake().is_none());
    assert!(lane.pop_ready_metadata().is_some());
    assert!(lane.pop_ready_metadata().is_none());
}

#[test]
fn window_message_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(WindowMessageLane::new());
}

#[test]
fn window_message_reconsideration_wakes_a_blocked_ready_head_without_duplicating_it() {
    let mut lane = WindowMessageLane::new();
    assert!(lane.enqueue_initial(1));
    assert!(lane.take_wake().is_some());
    assert!(lane.take_wake().is_none());

    lane.initial_sender.signal_reconsideration();
    assert!(
        lane.take_wake().is_some(),
        "realm materialization must readmit a blocked head even while the source owns its readiness epoch"
    );
    assert!(lane.take_wake().is_none());
    assert!(lane.pop_ready_metadata().is_some());
    assert!(
        lane.pop_ready_metadata().is_none(),
        "reconsideration is an admission signal, not a second Window.postMessage task"
    );
}

#[test]
fn modulepreload_start_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(ModulepreloadStartLane::new());
}

#[test]
fn dynamic_import_owner_action_source_conforms_to_page_queue_contract() {
    assert_fifo_replacement_and_route_retirement(DynamicImportOwnerActionLane::new());
}

#[test]
fn unified_ready_descriptors_expose_one_fifo_head_per_typed_source() {
    let runtime_wake = PageRuntimeWakeSignal::default();
    let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let owner_wake = RendererOwnerWakeSender::new(
        wake_tx,
        RendererPageToken::new_for_testing(document_token(1).page_id),
    );
    let (mut sources, routes) = RendererPageOwnedTaskSources::new(runtime_wake, owner_wake);
    let (dependency_target, dependency_task) = module_dependency_fetch_task(12);

    routes
        .dom_manipulation_sender(document_token(1))
        .broadcast_channel_delivery()
        .bind_execution_context(window_execution_context(83))
        .send(7)
        .expect("BroadcastChannel delivery should enter the unified source set");
    routes
        .window_message_sender(document_token(1))
        .send(
            WindowTaskTarget::new(
                OwnerDispatchScope::Top,
                WindowExecutionContextOwner::Frame(LocalWindowId(84)),
            ),
            RendererPageWindowMessageTaskId::from_raw(9),
        )
        .expect("Window.postMessage task should enter the unified source set");
    routes
        .message_port_delivery_sender(document_token(1))
        .bind_execution_context(window_execution_context(85))
        .send(10)
        .expect("MessagePort delivery should enter the unified source set");
    routes
        .dedicated_worker_client_event_sender(document_token(1))
        .bind_worker(
            window_execution_context(86),
            crate::types::DedicatedWorkerId::new(12),
        )
        .send(RendererDedicatedWorkerClientEvent::Message(
            RendererDedicatedWorkerMessageEvent::Message(Default::default()),
        ))
        .expect("DedicatedWorker client event should enter the unified source set");
    routes
        .shared_worker_client_event_sender(document_token(1))
        .bind_execution_context(window_execution_context(87))
        .bind_client(allocated_shared_worker_client_id())
        .send(SharedWorkerClientEvent::Closed)
        .expect("SharedWorker client event should enter the unified source set");
    routes
        .webcrypto_task_sender(document_token(1))
        .bind_task(
            window_execution_context(88),
            RendererPageWebCryptoTaskId::new(13),
        )
        .send(Ok(WebCryptoTaskResult::Bool(true)))
        .expect("WebCrypto task should enter the unified source set");
    routes
        .indexed_db_task_sender(document_token(1))
        .send(
            window_execution_context(89),
            RendererPageIndexedDbTaskKind::RuntimeQueue(IndexedDbTaskId::from_raw(14)),
        )
        .expect("IndexedDB task should enter the unified source set");
    routes
        .internal_loading_sender(document_token(1))
        .schedule_at(
            PageOwnedInternalLoadingTask::MetaRefreshNavigation(
                MainDocumentMetaRefreshNavigationTask::new(
                    task_owner(90),
                    0,
                    Url::parse("https://page-source-conformance.test/refresh")
                        .expect("internal-loading descriptor URL"),
                ),
            ),
            Instant::now(),
        )
        .expect("internal-loading task should enter the unified source set");
    routes
        .main_document_runtime_sender(document_token(1))
        .send_for_source_contract_test(
            task_owner(90),
            RendererPageMainDocumentRuntimeAction::ContinueRuntimeScriptWork,
        )
        .expect("main-Document runtime task should enter the unified source set");
    routes
        .child_frame_task_sender(document_token(1))
        .send_realm_materialization(RendererPageChildRealmMaterializationTarget::new(
            DomHandle::new(390),
            task_owner(90),
        ))
        .expect("child-realm materialization should enter the unified source set");
    routes
        .module_reaction_sender(document_token(1))
        .send(
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationFulfilled {
                document_owner: task_owner(90),
                reaction_id: 15,
            },
        )
        .expect("module reaction should enter the unified source set");
    routes
        .modulepreload_start_sender(document_token(1))
        .send(modulepreload_task(11))
        .expect("modulepreload start should enter the unified source set");
    routes
        .child_module_dependency_fetch_start_sender(document_token(1))
        .send(dependency_target, dependency_task)
        .expect("child module dependency start should enter the unified source set");
    routes
        .child_module_script_terminal_sender(document_token(1))
        .send(FrameDocumentModuleScriptTerminalBatchTask::new(
            task_owner(91),
            FrameRealmId(92),
            Vec::new(),
        ))
        .expect("child module terminal should enter the unified source set");
    routes
        .child_modulepreload_event_action_sender(document_token(1))
        .send(modulepreload_event_action(13))
        .expect("child modulepreload event action should enter the unified source set");
    routes
        .rendering_update_sender(document_token(1))
        .send(
            WindowDocumentTaskTarget::new(
                WindowDocumentOwner::Frame(task_owner(93)),
                OwnerDispatchScope::Top,
            ),
            RendererPageRenderingUpdateTaskId::from_raw(18),
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
        )
        .expect("rendering-update task should enter the unified source set");
    routes
        .resource_completion_sender()
        .send(
            RendererPageResourceCompletion::document_write_external_script(
                document_token(1),
                DocumentWriteExternalScriptLoadCompletion::for_test(13),
            ),
        )
        .expect("resource completion should enter the unified source set");
    routes
        .dynamic_import_owner_action_sender(document_token(1))
        .send_all(vec![dynamic_import_action(17)])
        .expect("dynamic-import action should enter the unified source set");

    let descriptors = sources.ready_descriptors();
    assert_eq!(descriptors.len(), 18);
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::DomManipulation { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::WindowMessage { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::MessagePortDelivery { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::DedicatedWorkerClientEvent { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::SharedWorkerClientEvent { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::WebCryptoTask { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::IndexedDbTask { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::InternalLoading { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::MainDocumentRuntime { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::ChildFrameTask { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::ModuleReaction { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::ModulepreloadStart { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::ChildModuleDependencyFetchStart { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::ChildModuleScriptTerminal { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::ChildModulepreloadEventAction { .. }
    )));
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::RenderingUpdate { .. }
    )));
    assert!(
        descriptors
            .iter()
            .any(|descriptor| matches!(descriptor, RendererPageReadyDescriptor::Networking { .. }))
    );
    assert!(descriptors.iter().any(|descriptor| matches!(
        descriptor,
        RendererPageReadyDescriptor::DynamicImportOwnerAction { .. }
    )));

    let dependency_start = descriptors
        .into_iter()
        .find(|descriptor| {
            matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildModuleDependencyFetchStart { .. }
            )
        })
        .expect("dependency-start source should expose its FIFO head");
    assert!(matches!(
        sources.take_task(dependency_start),
        RendererPageSchedulerTask::ChildModuleDependencyFetchStart(_)
    ));
    assert_eq!(
        sources.ready_descriptors().len(),
        17,
        "one Page turn must remove only the selected source head"
    );
}
