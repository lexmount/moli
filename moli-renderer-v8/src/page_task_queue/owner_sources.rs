use crate::{
    page_resource_completion::RendererPageResourceCompletionSender,
    resource_ready::RendererPageTaskReadyMetadata, runtime::RendererDocumentToken,
};

#[cfg(test)]
use super::main_document_runtime::PageMainDocumentRuntimeActionKind;
#[cfg(test)]
use crate::page_resource_completion::{
    RendererPageResourceCompletion, RendererPageResourceCompletionOwner,
};

use std::time::Instant;

#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use super::{
    PageRuntimeWakeSignal, RendererOwnerWakeSender,
    child_frame_task::{
        RendererPageChildFrameTask, RendererPageChildFrameTaskOwner,
        RendererPageChildFrameTaskRoute, RendererPageChildFrameTaskSender,
        RendererPageChildFrameTaskSource,
    },
    child_module_dependency_fetch_start::{
        RendererPageChildModuleDependencyFetchStartOwner,
        RendererPageChildModuleDependencyFetchStartRoute,
        RendererPageChildModuleDependencyFetchStartSender,
        RendererPageChildModuleDependencyFetchStartSource,
        RendererPageChildModuleDependencyFetchStartTask,
    },
    child_module_script_terminal::{
        RendererPageChildModuleScriptTerminalOwner, RendererPageChildModuleScriptTerminalRoute,
        RendererPageChildModuleScriptTerminalSender, RendererPageChildModuleScriptTerminalSource,
        RendererPageChildModuleScriptTerminalTask,
    },
    child_modulepreload_event_action::{
        RendererPageChildModulepreloadEventActionOwner,
        RendererPageChildModulepreloadEventActionRoute,
        RendererPageChildModulepreloadEventActionSender,
        RendererPageChildModulepreloadEventActionSource,
        RendererPageChildModulepreloadEventActionTask,
    },
    dedicated_worker_client_event::{
        RendererPageDedicatedWorkerClientEventOwner, RendererPageDedicatedWorkerClientEventRoute,
        RendererPageDedicatedWorkerClientEventSender, RendererPageDedicatedWorkerClientEventSource,
        RendererPageDedicatedWorkerClientEventTask,
    },
    dom_manipulation::{
        RendererPageDomManipulationOwner, RendererPageDomManipulationRoute,
        RendererPageDomManipulationSender, RendererPageDomManipulationSource,
        RendererPageDomManipulationTask,
    },
    dynamic_import_owner_action::{
        RendererPageDynamicImportOwnerActionRoute, RendererPageDynamicImportOwnerActionSender,
        RendererPageDynamicImportOwnerActionSource, RendererPageDynamicImportOwnerActionTask,
    },
    file_reading::{
        RendererPageFileReadingOwner, RendererPageFileReadingRoute, RendererPageFileReadingSender,
        RendererPageFileReadingSource, RendererPageFileReadingTask,
    },
    indexed_db_task::{
        RendererPageIndexedDbTask, RendererPageIndexedDbTaskOwner, RendererPageIndexedDbTaskRoute,
        RendererPageIndexedDbTaskSender, RendererPageIndexedDbTaskSource,
    },
    internal_loading::{
        RendererPageInternalLoadingOwner, RendererPageInternalLoadingRoute,
        RendererPageInternalLoadingSender, RendererPageInternalLoadingSource,
        RendererPageInternalLoadingTask,
    },
    main_document_runtime::{
        RendererPageMainDocumentRuntimeOwner, RendererPageMainDocumentRuntimeRoute,
        RendererPageMainDocumentRuntimeSender, RendererPageMainDocumentRuntimeSource,
        RendererPageMainDocumentRuntimeTask,
    },
    main_parser_continuation::RendererPageMainParserContinuationSender,
    media_element_event::{
        RendererPageMediaElementEventOwner, RendererPageMediaElementEventRoute,
        RendererPageMediaElementEventSender, RendererPageMediaElementEventSource,
        RendererPageMediaElementEventTask,
    },
    message_port_delivery::{
        RendererPageMessagePortDeliveryOwner, RendererPageMessagePortDeliveryRoute,
        RendererPageMessagePortDeliverySender, RendererPageMessagePortDeliverySource,
        RendererPageMessagePortDeliveryTask,
    },
    misc_platform_api::{
        RendererPageMiscPlatformApiOwner, RendererPageMiscPlatformApiRoute,
        RendererPageMiscPlatformApiSender, RendererPageMiscPlatformApiSource,
        RendererPageMiscPlatformApiTask,
    },
    module_reaction::{
        RendererPageModuleReactionOwner, RendererPageModuleReactionRoute,
        RendererPageModuleReactionSender, RendererPageModuleReactionSource,
        RendererPageModuleReactionTask,
    },
    modulepreload_start::{
        RendererPageModulepreloadStartRoute, RendererPageModulepreloadStartSender,
        RendererPageModulepreloadStartSource, RendererPageModulepreloadStartTask,
    },
    navigation_and_traversal::{
        RendererPageNavigationAndTraversalHead, RendererPageNavigationAndTraversalRoute,
        RendererPageNavigationAndTraversalSender, RendererPageNavigationAndTraversalSource,
        RendererPageNavigationAndTraversalTask,
    },
    networking::{
        RendererPageNetworkingOwner, RendererPageNetworkingRoute, RendererPageNetworkingSource,
        RendererPageNetworkingTask,
    },
    opfs_task::{
        RendererPageOpfsTask, RendererPageOpfsTaskOwner, RendererPageOpfsTaskRoute,
        RendererPageOpfsTaskSender, RendererPageOpfsTaskSource,
    },
    rendering_update::{
        RendererPageRenderingUpdateHead, RendererPageRenderingUpdateRoute,
        RendererPageRenderingUpdateSender, RendererPageRenderingUpdateSource,
        RendererPageRenderingUpdateTask,
    },
    service_worker_client_message::{
        RendererPageServiceWorkerClientMessageOwner, RendererPageServiceWorkerClientMessageRoute,
        RendererPageServiceWorkerClientMessageSource, RendererPageServiceWorkerClientMessageTask,
    },
    service_worker_internal::{
        RendererPageServiceWorkerInternalRoute, RendererPageServiceWorkerInternalSource,
        RendererPageServiceWorkerInternalTask,
    },
    service_worker_tasks::RendererPageServiceWorkerTaskSender,
    shared_worker_client_event::{
        RendererPageSharedWorkerClientEventOwner, RendererPageSharedWorkerClientEventRoute,
        RendererPageSharedWorkerClientEventSender, RendererPageSharedWorkerClientEventSource,
        RendererPageSharedWorkerClientEventTask,
    },
    stylesheet_task::RendererPageStylesheetTaskSender,
    text_track_load::RendererPageTextTrackLoadSender,
    user_interaction::{
        RendererPageUserInteractionOwner, RendererPageUserInteractionRoute,
        RendererPageUserInteractionSender, RendererPageUserInteractionSource,
        RendererPageUserInteractionTask,
    },
    v8_foreground_task::{
        RendererPageV8ForegroundTask, RendererPageV8ForegroundTaskOwner,
        RendererPageV8ForegroundTaskSender, RendererPageV8ForegroundTaskSource,
    },
    webcrypto_task::{
        RendererPageWebCryptoTask, RendererPageWebCryptoTaskOwner, RendererPageWebCryptoTaskRoute,
        RendererPageWebCryptoTaskSender, RendererPageWebCryptoTaskSource,
    },
    websocket_event::{
        RendererPageWebSocketHead, RendererPageWebSocketOwner, RendererPageWebSocketReadiness,
        RendererPageWebSocketRoute, RendererPageWebSocketSender, RendererPageWebSocketSource,
        RendererPageWebSocketTask,
    },
    window_message::{
        RendererPageWindowMessageOwner, RendererPageWindowMessageRoute,
        RendererPageWindowMessageSender, RendererPageWindowMessageSource,
        RendererPageWindowMessageTask,
    },
    worker_host_bridge::RendererWorkerHostBridgeEventSender,
};

/// The unique consumer side of every scheduler-visible ordinary Page source.
///
/// This value is created in the owner-local isolate reservation before the
/// first PageVm exists and is moved exactly once into the stable Page slot on
/// attach. It is intentionally not cloneable: only the owner-local arbiter may
/// inspect, dequeue, or clear runnable work.
#[derive(Debug)]
pub(crate) struct RendererPageOwnedTaskSources {
    dom_manipulation: RendererPageDomManipulationSource,
    user_interaction: RendererPageUserInteractionSource,
    file_reading: RendererPageFileReadingSource,
    misc_platform_api: RendererPageMiscPlatformApiSource,
    navigation_and_traversal: RendererPageNavigationAndTraversalSource,
    rendering_update: RendererPageRenderingUpdateSource,
    media_element_event: RendererPageMediaElementEventSource,
    dedicated_worker_client_event: RendererPageDedicatedWorkerClientEventSource,
    shared_worker_client_event: RendererPageSharedWorkerClientEventSource,
    service_worker_internal: RendererPageServiceWorkerInternalSource,
    service_worker_client_message: RendererPageServiceWorkerClientMessageSource,
    webcrypto_task: RendererPageWebCryptoTaskSource,
    indexed_db_task: RendererPageIndexedDbTaskSource,
    opfs_task: RendererPageOpfsTaskSource,
    internal_loading: RendererPageInternalLoadingSource,
    main_document_runtime: RendererPageMainDocumentRuntimeSource,
    child_module_dependency_fetch_start: RendererPageChildModuleDependencyFetchStartSource,
    child_module_script_terminal: RendererPageChildModuleScriptTerminalSource,
    child_modulepreload_event_action: RendererPageChildModulepreloadEventActionSource,
    child_frame_task: RendererPageChildFrameTaskSource,
    v8_foreground_task: RendererPageV8ForegroundTaskSource,
    module_reaction: RendererPageModuleReactionSource,
    networking: RendererPageNetworkingSource,
    websocket: RendererPageWebSocketSource,
    modulepreload_start: RendererPageModulepreloadStartSource,
    dynamic_import_owner_action: RendererPageDynamicImportOwnerActionSource,
    window_message: RendererPageWindowMessageSource,
    message_port_delivery: RendererPageMessagePortDeliverySource,
}

/// Cloneable producer-only routes corresponding to one stable Page source set.
///
/// PageVm generations may clone these routes and stamp exact Document/realm
/// identity onto individual tasks, but this type provides no consumer access.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageTaskProducerRoutes {
    dom_manipulation: RendererPageDomManipulationRoute,
    user_interaction: RendererPageUserInteractionRoute,
    file_reading: RendererPageFileReadingRoute,
    misc_platform_api: RendererPageMiscPlatformApiRoute,
    navigation_and_traversal: RendererPageNavigationAndTraversalRoute,
    rendering_update: RendererPageRenderingUpdateRoute,
    media_element_event: RendererPageMediaElementEventRoute,
    dedicated_worker_client_event: RendererPageDedicatedWorkerClientEventRoute,
    shared_worker_client_event: RendererPageSharedWorkerClientEventRoute,
    service_worker_internal: RendererPageServiceWorkerInternalRoute,
    service_worker_client_message: RendererPageServiceWorkerClientMessageRoute,
    webcrypto_task: RendererPageWebCryptoTaskRoute,
    indexed_db_task: RendererPageIndexedDbTaskRoute,
    opfs_task: RendererPageOpfsTaskRoute,
    internal_loading: RendererPageInternalLoadingRoute,
    main_document_runtime: RendererPageMainDocumentRuntimeRoute,
    child_module_dependency_fetch_start: RendererPageChildModuleDependencyFetchStartRoute,
    child_module_script_terminal: RendererPageChildModuleScriptTerminalRoute,
    child_modulepreload_event_action: RendererPageChildModulepreloadEventActionRoute,
    child_frame_task: RendererPageChildFrameTaskRoute,
    v8_foreground_task: RendererPageV8ForegroundTaskSender,
    module_reaction: RendererPageModuleReactionRoute,
    networking: RendererPageNetworkingRoute,
    websocket: RendererPageWebSocketRoute,
    modulepreload_start: RendererPageModulepreloadStartRoute,
    dynamic_import_owner_action: RendererPageDynamicImportOwnerActionRoute,
    window_message: RendererPageWindowMessageRoute,
    message_port_delivery: RendererPageMessagePortDeliveryRoute,
}

/// A source-head snapshot consumed only by the owner-local Page scheduler.
///
/// It identifies the candidate source and carries only the metadata required
/// for arbitration and source-local eligibility. Exact executable identity is
/// part of each task payload and is interpreted only after dequeue by the lane
/// executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageReadyDescriptor {
    ActionWindow {
        deadline: Instant,
    },
    DomManipulation {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageDomManipulationOwner,
    },
    UserInteraction {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageUserInteractionOwner,
    },
    FileReading {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageFileReadingOwner,
    },
    MiscPlatformApi {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageMiscPlatformApiOwner,
    },
    NavigationAndTraversal {
        ready: RendererPageTaskReadyMetadata,
        head: RendererPageNavigationAndTraversalHead,
    },
    RenderingUpdate {
        ready: RendererPageTaskReadyMetadata,
        head: RendererPageRenderingUpdateHead,
    },
    MediaElementEvent {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageMediaElementEventOwner,
    },
    DedicatedWorkerClientEvent {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageDedicatedWorkerClientEventOwner,
    },
    SharedWorkerClientEvent {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageSharedWorkerClientEventOwner,
    },
    ServiceWorkerInternal {
        ready: RendererPageTaskReadyMetadata,
        root_document: RendererDocumentToken,
    },
    ServiceWorkerClientMessage {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageServiceWorkerClientMessageOwner,
    },
    WebCryptoTask {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageWebCryptoTaskOwner,
    },
    IndexedDbTask {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageIndexedDbTaskOwner,
    },
    OpfsTask {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageOpfsTaskOwner,
    },
    InternalLoading {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageInternalLoadingOwner,
    },
    MainDocumentRuntime {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageMainDocumentRuntimeOwner,
    },
    ChildModuleDependencyFetchStart {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageChildModuleDependencyFetchStartOwner,
    },
    ChildModuleScriptTerminal {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageChildModuleScriptTerminalOwner,
    },
    ChildModulepreloadEventAction {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageChildModulepreloadEventActionOwner,
    },
    ChildFrameTask {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageChildFrameTaskOwner,
    },
    V8ForegroundTask {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageV8ForegroundTaskOwner,
    },
    ModuleReaction {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageModuleReactionOwner,
    },
    WindowMessage {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageWindowMessageOwner,
        task_id: super::window_message::RendererPageWindowMessageTaskId,
    },
    MessagePortDelivery {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageMessagePortDeliveryOwner,
        port_id: crate::types::MessagePortId,
    },
    DynamicImportOwnerAction {
        ready: RendererPageTaskReadyMetadata,
    },
    ModulepreloadStart {
        ready: RendererPageTaskReadyMetadata,
    },
    Networking {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageNetworkingOwner,
    },
    WebSocket {
        ready: RendererPageTaskReadyMetadata,
        owner: RendererPageWebSocketOwner,
        readiness: RendererPageWebSocketReadiness,
    },
    Timer {
        deadline: Instant,
    },
}

impl RendererPageReadyDescriptor {
    pub(crate) const fn ready_metadata(self) -> Option<RendererPageTaskReadyMetadata> {
        match self {
            Self::DomManipulation { ready, .. }
            | Self::UserInteraction { ready, .. }
            | Self::FileReading { ready, .. }
            | Self::MiscPlatformApi { ready, .. }
            | Self::NavigationAndTraversal { ready, .. }
            | Self::RenderingUpdate { ready, .. }
            | Self::MediaElementEvent { ready, .. }
            | Self::DedicatedWorkerClientEvent { ready, .. }
            | Self::SharedWorkerClientEvent { ready, .. }
            | Self::ServiceWorkerInternal { ready, .. }
            | Self::ServiceWorkerClientMessage { ready, .. }
            | Self::WebCryptoTask { ready, .. }
            | Self::IndexedDbTask { ready, .. }
            | Self::OpfsTask { ready, .. }
            | Self::InternalLoading { ready, .. }
            | Self::MainDocumentRuntime { ready, .. }
            | Self::ChildModuleDependencyFetchStart { ready, .. }
            | Self::ChildModuleScriptTerminal { ready, .. }
            | Self::ChildModulepreloadEventAction { ready, .. }
            | Self::ChildFrameTask { ready, .. }
            | Self::V8ForegroundTask { ready, .. }
            | Self::ModuleReaction { ready, .. }
            | Self::WindowMessage { ready, .. }
            | Self::MessagePortDelivery { ready, .. }
            | Self::DynamicImportOwnerAction { ready }
            | Self::ModulepreloadStart { ready }
            | Self::Networking { ready, .. }
            | Self::WebSocket { ready, .. } => Some(ready),
            Self::ActionWindow { .. } | Self::Timer { .. } => None,
        }
    }

    pub(crate) const fn source_kind(self) -> RendererPageTaskSourceKind {
        match self {
            Self::DomManipulation { .. } => RendererPageTaskSourceKind::DomManipulation,
            Self::UserInteraction { .. } => RendererPageTaskSourceKind::UserInteraction,
            Self::FileReading { .. } => RendererPageTaskSourceKind::FileReading,
            Self::MiscPlatformApi { .. } => RendererPageTaskSourceKind::MiscPlatformApi,
            Self::NavigationAndTraversal { .. } => {
                RendererPageTaskSourceKind::NavigationAndTraversal
            }
            Self::RenderingUpdate { .. } => RendererPageTaskSourceKind::RenderingUpdate,
            Self::MediaElementEvent { .. } => RendererPageTaskSourceKind::MediaElementEvent,
            Self::DedicatedWorkerClientEvent { .. } => {
                RendererPageTaskSourceKind::DedicatedWorkerClientEvent
            }
            Self::SharedWorkerClientEvent { .. } => {
                RendererPageTaskSourceKind::SharedWorkerClientEvent
            }
            Self::ServiceWorkerInternal { .. } => RendererPageTaskSourceKind::ServiceWorkerInternal,
            Self::ServiceWorkerClientMessage { .. } => {
                RendererPageTaskSourceKind::ServiceWorkerClientMessage
            }
            Self::WebCryptoTask { .. } => RendererPageTaskSourceKind::WebCryptoTask,
            Self::IndexedDbTask { .. } => RendererPageTaskSourceKind::IndexedDbTask,
            Self::OpfsTask { .. } => RendererPageTaskSourceKind::OpfsTask,
            Self::InternalLoading { .. } => RendererPageTaskSourceKind::InternalLoading,
            Self::MainDocumentRuntime { .. } => RendererPageTaskSourceKind::MainDocumentRuntime,
            Self::ChildModuleDependencyFetchStart { .. } => {
                RendererPageTaskSourceKind::ChildModuleDependencyFetchStart
            }
            Self::ChildModuleScriptTerminal { .. } => {
                RendererPageTaskSourceKind::ChildModuleScriptTerminal
            }
            Self::ChildModulepreloadEventAction { .. } => {
                RendererPageTaskSourceKind::ChildModulepreloadEventAction
            }
            Self::ChildFrameTask { .. } => RendererPageTaskSourceKind::ChildFrameTask,
            Self::V8ForegroundTask { .. } => RendererPageTaskSourceKind::V8ForegroundTask,
            Self::ModuleReaction { .. } => RendererPageTaskSourceKind::ModuleReaction,
            Self::WindowMessage { .. } => RendererPageTaskSourceKind::WindowMessage,
            Self::MessagePortDelivery { .. } => RendererPageTaskSourceKind::MessagePortDelivery,
            Self::DynamicImportOwnerAction { .. } => {
                RendererPageTaskSourceKind::DynamicImportOwnerAction
            }
            Self::ModulepreloadStart { .. } => RendererPageTaskSourceKind::ModulepreloadStart,
            Self::Networking { .. } | Self::WebSocket { .. } => {
                RendererPageTaskSourceKind::Networking
            }
            Self::ActionWindow { .. } => RendererPageTaskSourceKind::ActionWindow,
            Self::Timer { .. } => RendererPageTaskSourceKind::Timer,
        }
    }

    pub(crate) const fn runnable_since(self) -> Instant {
        match self {
            Self::DomManipulation { ready, .. }
            | Self::UserInteraction { ready, .. }
            | Self::FileReading { ready, .. }
            | Self::MiscPlatformApi { ready, .. }
            | Self::NavigationAndTraversal { ready, .. }
            | Self::RenderingUpdate { ready, .. }
            | Self::MediaElementEvent { ready, .. }
            | Self::DedicatedWorkerClientEvent { ready, .. }
            | Self::SharedWorkerClientEvent { ready, .. }
            | Self::ServiceWorkerInternal { ready, .. }
            | Self::ServiceWorkerClientMessage { ready, .. }
            | Self::WebCryptoTask { ready, .. }
            | Self::IndexedDbTask { ready, .. }
            | Self::OpfsTask { ready, .. }
            | Self::InternalLoading { ready, .. }
            | Self::MainDocumentRuntime { ready, .. }
            | Self::ChildModuleDependencyFetchStart { ready, .. }
            | Self::ChildModuleScriptTerminal { ready, .. }
            | Self::ChildModulepreloadEventAction { ready, .. }
            | Self::ChildFrameTask { ready, .. }
            | Self::V8ForegroundTask { ready, .. }
            | Self::ModuleReaction { ready, .. }
            | Self::WindowMessage { ready, .. }
            | Self::MessagePortDelivery { ready, .. }
            | Self::DynamicImportOwnerAction { ready }
            | Self::ModulepreloadStart { ready }
            | Self::Networking { ready, .. }
            | Self::WebSocket { ready, .. } => ready.ready_at,
            Self::ActionWindow { deadline } | Self::Timer { deadline } => deadline,
        }
    }

    pub(crate) const fn enqueue_order(self) -> Option<u64> {
        match self.ready_metadata() {
            Some(ready) => Some(ready.order),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum RendererPageTaskSourceKind {
    ActionWindow,
    Timer,
    DomManipulation,
    UserInteraction,
    FileReading,
    MiscPlatformApi,
    NavigationAndTraversal,
    RenderingUpdate,
    MediaElementEvent,
    DedicatedWorkerClientEvent,
    SharedWorkerClientEvent,
    ServiceWorkerInternal,
    ServiceWorkerClientMessage,
    WebCryptoTask,
    IndexedDbTask,
    OpfsTask,
    InternalLoading,
    MainDocumentRuntime,
    ChildModuleDependencyFetchStart,
    ChildModuleScriptTerminal,
    ChildModulepreloadEventAction,
    ChildFrameTask,
    V8ForegroundTask,
    ModuleReaction,
    WindowMessage,
    MessagePortDelivery,
    DynamicImportOwnerAction,
    ModulepreloadStart,
    Networking,
}

impl RendererPageTaskSourceKind {
    pub(crate) const ALL: [Self; 29] = [
        Self::ActionWindow,
        Self::Timer,
        Self::DomManipulation,
        Self::UserInteraction,
        Self::FileReading,
        Self::MiscPlatformApi,
        Self::NavigationAndTraversal,
        Self::RenderingUpdate,
        Self::MediaElementEvent,
        Self::DedicatedWorkerClientEvent,
        Self::SharedWorkerClientEvent,
        Self::ServiceWorkerInternal,
        Self::ServiceWorkerClientMessage,
        Self::WebCryptoTask,
        Self::IndexedDbTask,
        Self::OpfsTask,
        Self::InternalLoading,
        Self::MainDocumentRuntime,
        Self::ChildModuleDependencyFetchStart,
        Self::ChildModuleScriptTerminal,
        Self::ChildModulepreloadEventAction,
        Self::ChildFrameTask,
        Self::V8ForegroundTask,
        Self::ModuleReaction,
        Self::WindowMessage,
        Self::MessagePortDelivery,
        Self::DynamicImportOwnerAction,
        Self::ModulepreloadStart,
        Self::Networking,
    ];
}

/// The one concrete ordinary task selected for an admitted Page turn.
///
/// Typed payloads leave their unique source before the PageVm is checked out
/// into an async executor.
#[derive(Debug)]
pub(crate) enum RendererPageSchedulerTask {
    ActionWindow {
        deadline: Instant,
    },
    DomManipulation(RendererPageDomManipulationTask),
    UserInteraction(RendererPageUserInteractionTask),
    FileReading(RendererPageFileReadingTask),
    MiscPlatformApi(RendererPageMiscPlatformApiTask),
    NavigationAndTraversal(RendererPageNavigationAndTraversalTask),
    RenderingUpdate(RendererPageRenderingUpdateTask),
    MediaElementEvent(RendererPageMediaElementEventTask),
    DedicatedWorkerClientEvent(RendererPageDedicatedWorkerClientEventTask),
    SharedWorkerClientEvent(RendererPageSharedWorkerClientEventTask),
    ServiceWorkerInternal(RendererPageServiceWorkerInternalTask),
    ServiceWorkerClientMessage(RendererPageServiceWorkerClientMessageTask),
    WebCryptoTask(RendererPageWebCryptoTask),
    IndexedDbTask(RendererPageIndexedDbTask),
    OpfsTask(RendererPageOpfsTask),
    InternalLoading(RendererPageInternalLoadingTask),
    MainDocumentRuntime(RendererPageMainDocumentRuntimeTask),
    ChildModuleDependencyFetchStart(Box<RendererPageChildModuleDependencyFetchStartTask>),
    ChildModuleScriptTerminal(RendererPageChildModuleScriptTerminalTask),
    ChildModulepreloadEventAction(RendererPageChildModulepreloadEventActionTask),
    ChildFrameTask(RendererPageChildFrameTask),
    V8ForegroundTask(RendererPageV8ForegroundTask),
    ModuleReaction(RendererPageModuleReactionTask),
    WindowMessage(RendererPageWindowMessageTask),
    MessagePortDelivery {
        task: RendererPageMessagePortDeliveryTask,
        same_attachment_task_is_ready: bool,
    },
    DynamicImportOwnerAction(RendererPageDynamicImportOwnerActionTask),
    ModulepreloadStart(RendererPageModulepreloadStartTask),
    Networking(RendererPageNetworkingTask),
    WebSocket(RendererPageWebSocketTask),
    Timer {
        deadline: Instant,
    },
}

impl RendererPageOwnedTaskSources {
    pub(crate) fn new(
        runtime_wake: PageRuntimeWakeSignal,
        owner_wake: RendererOwnerWakeSender,
    ) -> (Self, RendererPageTaskProducerRoutes) {
        let dom_manipulation = RendererPageDomManipulationSource::new(owner_wake.clone());
        let user_interaction = RendererPageUserInteractionSource::new(
            owner_wake.clone(),
            RendererOwnerWakeSender::signal_user_interaction_task,
        );
        let file_reading = RendererPageFileReadingSource::new(
            owner_wake.clone(),
            RendererOwnerWakeSender::signal_file_reading_task,
        );
        let misc_platform_api = RendererPageMiscPlatformApiSource::new(
            owner_wake.clone(),
            RendererOwnerWakeSender::signal_misc_platform_api_task,
        );
        let navigation_and_traversal =
            RendererPageNavigationAndTraversalSource::new(owner_wake.clone());
        let rendering_update = RendererPageRenderingUpdateSource::new(
            owner_wake.clone(),
            RendererOwnerWakeSender::signal_rendering_update_task,
        );
        let media_element_event = RendererPageMediaElementEventSource::new(
            owner_wake.clone(),
            RendererOwnerWakeSender::signal_media_element_event_task,
        );
        let dedicated_worker_client_event =
            RendererPageDedicatedWorkerClientEventSource::new(owner_wake.clone());
        let shared_worker_client_event =
            RendererPageSharedWorkerClientEventSource::new(owner_wake.clone());
        let service_worker_internal =
            RendererPageServiceWorkerInternalSource::new(owner_wake.clone());
        let service_worker_client_message =
            RendererPageServiceWorkerClientMessageSource::new(owner_wake.clone());
        let webcrypto_task = RendererPageWebCryptoTaskSource::new(owner_wake.clone());
        let indexed_db_task = RendererPageIndexedDbTaskSource::new(owner_wake.clone());
        let opfs_task = RendererPageOpfsTaskSource::new(owner_wake.clone());
        let internal_loading = RendererPageInternalLoadingSource::new(owner_wake.clone());
        let main_document_runtime = RendererPageMainDocumentRuntimeSource::new(owner_wake.clone());
        let child_module_dependency_fetch_start =
            RendererPageChildModuleDependencyFetchStartSource::new(owner_wake.clone());
        let child_module_script_terminal =
            RendererPageChildModuleScriptTerminalSource::new(owner_wake.clone());
        let child_modulepreload_event_action =
            RendererPageChildModulepreloadEventActionSource::new(owner_wake.clone());
        let child_frame_task = RendererPageChildFrameTaskSource::new(owner_wake.clone());
        let v8_foreground_task = RendererPageV8ForegroundTaskSource::new(owner_wake.clone());
        let module_reaction = RendererPageModuleReactionSource::new(owner_wake.clone());
        let window_message = RendererPageWindowMessageSource::new(owner_wake.clone());
        let message_port_delivery = RendererPageMessagePortDeliverySource::new(owner_wake.clone());
        let networking =
            RendererPageNetworkingSource::new(runtime_wake.clone(), owner_wake.clone());
        let websocket = RendererPageWebSocketSource::new(runtime_wake, owner_wake.clone());
        let modulepreload_start = RendererPageModulepreloadStartSource::new(owner_wake.clone());
        let dynamic_import_owner_action =
            RendererPageDynamicImportOwnerActionSource::new(owner_wake.clone());
        let routes = RendererPageTaskProducerRoutes {
            dom_manipulation: dom_manipulation.route(),
            user_interaction: user_interaction.route(),
            file_reading: file_reading.route(),
            misc_platform_api: misc_platform_api.route(),
            navigation_and_traversal: navigation_and_traversal.route(),
            rendering_update: rendering_update.route(),
            media_element_event: media_element_event.route(),
            dedicated_worker_client_event: dedicated_worker_client_event.route(),
            shared_worker_client_event: shared_worker_client_event.route(),
            service_worker_internal: service_worker_internal.route(),
            service_worker_client_message: service_worker_client_message.route(),
            webcrypto_task: webcrypto_task.route(),
            indexed_db_task: indexed_db_task.route(),
            opfs_task: opfs_task.route(),
            internal_loading: internal_loading.route(),
            main_document_runtime: main_document_runtime.route(),
            child_module_dependency_fetch_start: child_module_dependency_fetch_start.route(),
            child_module_script_terminal: child_module_script_terminal.route(),
            child_modulepreload_event_action: child_modulepreload_event_action.route(),
            child_frame_task: child_frame_task.route(),
            v8_foreground_task: v8_foreground_task.sender(),
            module_reaction: module_reaction.route(),
            window_message: window_message.route(),
            message_port_delivery: message_port_delivery.route(),
            networking: networking.route(),
            websocket: websocket.route(),
            modulepreload_start: modulepreload_start.route(),
            dynamic_import_owner_action: dynamic_import_owner_action.route(),
        };
        (
            Self {
                dom_manipulation,
                user_interaction,
                file_reading,
                misc_platform_api,
                navigation_and_traversal,
                rendering_update,
                media_element_event,
                dedicated_worker_client_event,
                shared_worker_client_event,
                service_worker_internal,
                service_worker_client_message,
                webcrypto_task,
                indexed_db_task,
                opfs_task,
                internal_loading,
                main_document_runtime,
                child_module_dependency_fetch_start,
                child_module_script_terminal,
                child_modulepreload_event_action,
                child_frame_task,
                v8_foreground_task,
                module_reaction,
                networking,
                websocket,
                modulepreload_start,
                dynamic_import_owner_action,
                window_message,
                message_port_delivery,
            },
            routes,
        )
    }

    pub(crate) fn routes_match(&self, routes: &RendererPageTaskProducerRoutes) -> bool {
        self.dom_manipulation
            .route_matches(&routes.dom_manipulation)
            && self
                .user_interaction
                .route_matches(&routes.user_interaction)
            && self.file_reading.route_matches(&routes.file_reading)
            && self
                .misc_platform_api
                .route_matches(&routes.misc_platform_api)
            && self
                .navigation_and_traversal
                .route_matches(&routes.navigation_and_traversal)
            && self
                .rendering_update
                .route_matches(&routes.rendering_update)
            && self
                .media_element_event
                .route_matches(&routes.media_element_event)
            && self
                .dedicated_worker_client_event
                .route_matches(&routes.dedicated_worker_client_event)
            && self
                .shared_worker_client_event
                .route_matches(&routes.shared_worker_client_event)
            && self
                .service_worker_internal
                .route_matches(&routes.service_worker_internal)
            && self
                .service_worker_client_message
                .route_matches(&routes.service_worker_client_message)
            && self.webcrypto_task.route_matches(&routes.webcrypto_task)
            && self.indexed_db_task.route_matches(&routes.indexed_db_task)
            && self.opfs_task.route_matches(&routes.opfs_task)
            && self
                .internal_loading
                .route_matches(&routes.internal_loading)
            && self
                .main_document_runtime
                .route_matches(&routes.main_document_runtime)
            && self
                .child_module_dependency_fetch_start
                .route_matches(&routes.child_module_dependency_fetch_start)
            && self
                .child_module_script_terminal
                .route_matches(&routes.child_module_script_terminal)
            && self
                .child_modulepreload_event_action
                .route_matches(&routes.child_modulepreload_event_action)
            && self
                .child_frame_task
                .route_matches(&routes.child_frame_task)
            && self
                .v8_foreground_task
                .route_matches(&routes.v8_foreground_task)
            && self.module_reaction.route_matches(&routes.module_reaction)
            && self.window_message.route_matches(&routes.window_message)
            && self
                .message_port_delivery
                .route_matches(&routes.message_port_delivery)
            && routes.networking.same_source_as(&self.networking)
            && routes.websocket.same_source_as(&self.websocket)
            && self
                .modulepreload_start
                .route_matches(&routes.modulepreload_start)
            && self
                .dynamic_import_owner_action
                .route_matches(&routes.dynamic_import_owner_action)
    }

    pub(crate) fn next_internal_loading_deadline(
        &mut self,
        current_owner: Option<RendererPageInternalLoadingOwner>,
    ) -> Option<Instant> {
        self.internal_loading.next_deadline_for_owner(current_owner)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn local_internal_loading_deadline(
        &self,
        current_owner: Option<RendererPageInternalLoadingOwner>,
    ) -> Option<Instant> {
        self.internal_loading
            .local_deadline_for_owner(current_owner)
    }

    pub(crate) fn producer_routes(&self) -> RendererPageTaskProducerRoutes {
        RendererPageTaskProducerRoutes {
            dom_manipulation: self.dom_manipulation.route(),
            user_interaction: self.user_interaction.route(),
            file_reading: self.file_reading.route(),
            misc_platform_api: self.misc_platform_api.route(),
            navigation_and_traversal: self.navigation_and_traversal.route(),
            rendering_update: self.rendering_update.route(),
            media_element_event: self.media_element_event.route(),
            dedicated_worker_client_event: self.dedicated_worker_client_event.route(),
            shared_worker_client_event: self.shared_worker_client_event.route(),
            service_worker_internal: self.service_worker_internal.route(),
            service_worker_client_message: self.service_worker_client_message.route(),
            webcrypto_task: self.webcrypto_task.route(),
            indexed_db_task: self.indexed_db_task.route(),
            opfs_task: self.opfs_task.route(),
            internal_loading: self.internal_loading.route(),
            main_document_runtime: self.main_document_runtime.route(),
            child_module_dependency_fetch_start: self.child_module_dependency_fetch_start.route(),
            child_module_script_terminal: self.child_module_script_terminal.route(),
            child_modulepreload_event_action: self.child_modulepreload_event_action.route(),
            child_frame_task: self.child_frame_task.route(),
            v8_foreground_task: self.v8_foreground_task.sender(),
            module_reaction: self.module_reaction.route(),
            window_message: self.window_message.route(),
            message_port_delivery: self.message_port_delivery.route(),
            networking: self.networking.route(),
            websocket: self.websocket.route(),
            modulepreload_start: self.modulepreload_start.route(),
            dynamic_import_owner_action: self.dynamic_import_owner_action.route(),
        }
    }

    pub(crate) fn ready_descriptors(&mut self) -> Vec<RendererPageReadyDescriptor> {
        let dom_manipulation = self.dom_manipulation.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::DomManipulation {
                ready,
                owner: self
                    .dom_manipulation
                    .next_ready_owner()
                    .expect("ready DOM-manipulation task must retain its exact owner"),
            }
        });
        let user_interaction = self.user_interaction.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::UserInteraction {
                ready,
                owner: self
                    .user_interaction
                    .next_ready_owner()
                    .expect("ready user-interaction task must retain its exact owner"),
            }
        });
        let file_reading = self.file_reading.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::FileReading {
                ready,
                owner: self
                    .file_reading
                    .next_ready_owner()
                    .expect("ready file-reading task must retain its exact owner"),
            }
        });
        let misc_platform_api = self.misc_platform_api.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::MiscPlatformApi {
                ready,
                owner: self
                    .misc_platform_api
                    .next_ready_owner()
                    .expect("ready miscellaneous-platform task must retain its exact owner"),
            }
        });
        let navigation_and_traversal =
            self.navigation_and_traversal
                .next_ready_metadata()
                .map(
                    |ready| RendererPageReadyDescriptor::NavigationAndTraversal {
                        ready,
                        head: self.navigation_and_traversal.next_ready_head().expect(
                            "ready navigation-and-traversal task must retain its exact head",
                        ),
                    },
                );
        let rendering_update = self.rendering_update.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::RenderingUpdate {
                ready,
                head: self
                    .rendering_update
                    .next_ready_head()
                    .expect("ready rendering-update task must retain its exact head"),
            }
        });
        let media_element_event = self.media_element_event.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::MediaElementEvent {
                ready,
                owner: self
                    .media_element_event
                    .next_ready_owner()
                    .expect("ready media-element event task must retain its exact owner"),
            }
        });
        let dedicated_worker_client_event = self
            .dedicated_worker_client_event
            .next_ready_metadata()
            .map(
                |ready| RendererPageReadyDescriptor::DedicatedWorkerClientEvent {
                    ready,
                    owner: self
                        .dedicated_worker_client_event
                        .next_ready_owner()
                        .expect("ready DedicatedWorker client event must retain its exact owner"),
                },
            );
        let shared_worker_client_event =
            self.shared_worker_client_event
                .next_ready_metadata()
                .map(
                    |ready| RendererPageReadyDescriptor::SharedWorkerClientEvent {
                        ready,
                        owner: self
                            .shared_worker_client_event
                            .next_ready_owner()
                            .expect("ready SharedWorker client event must retain its exact owner"),
                    },
                );
        let webcrypto_task = self.webcrypto_task.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::WebCryptoTask {
                ready,
                owner: self
                    .webcrypto_task
                    .next_ready_owner()
                    .expect("ready WebCrypto task must retain its exact owner"),
            }
        });
        let indexed_db_task = self.indexed_db_task.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::IndexedDbTask {
                ready,
                owner: self
                    .indexed_db_task
                    .next_ready_owner()
                    .expect("ready IndexedDB task must retain its exact owner"),
            }
        });
        let opfs_task = self.opfs_task.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::OpfsTask {
                ready,
                owner: self
                    .opfs_task
                    .next_ready_owner()
                    .expect("ready OPFS task must retain its exact owner"),
            }
        });
        let internal_loading = self.internal_loading.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::InternalLoading {
                ready,
                owner: self
                    .internal_loading
                    .next_ready_owner()
                    .expect("ready internal-loading task must retain its exact Document owner"),
            }
        });
        let main_document_runtime = self
            .main_document_runtime
            .next_ready_metadata()
            .map(|ready| RendererPageReadyDescriptor::MainDocumentRuntime {
                ready,
                owner: self
                    .main_document_runtime
                    .next_ready_owner()
                    .expect("ready main-Document runtime task must retain its exact owner"),
            });
        let child_module_dependency_fetch_start = self
            .child_module_dependency_fetch_start
            .next_ready_metadata()
            .map(
                |ready| RendererPageReadyDescriptor::ChildModuleDependencyFetchStart {
                    ready,
                    owner: self
                        .child_module_dependency_fetch_start
                        .next_ready_owner()
                        .expect(
                            "ready child module dependency fetch start must retain its exact owner",
                        ),
                },
            );
        let child_module_script_terminal =
            self.child_module_script_terminal
                .next_ready_metadata()
                .map(
                    |ready| RendererPageReadyDescriptor::ChildModuleScriptTerminal {
                        ready,
                        owner: self.child_module_script_terminal.next_ready_owner().expect(
                            "ready child module-script terminal must retain its exact owner",
                        ),
                    },
                );
        let service_worker_internal =
            self.service_worker_internal
                .next_ready_metadata()
                .map(|ready| RendererPageReadyDescriptor::ServiceWorkerInternal {
                    ready,
                    root_document: self
                        .service_worker_internal
                        .next_ready_root_document()
                        .expect(
                            "ready ServiceWorker internal task must retain its exact root Document",
                        ),
                });
        let service_worker_client_message = self
            .service_worker_client_message
            .next_ready_metadata()
            .map(
                |ready| RendererPageReadyDescriptor::ServiceWorkerClientMessage {
                    ready,
                    owner: self
                        .service_worker_client_message
                        .next_ready_owner()
                        .expect("ready ServiceWorker client message must retain its exact owner"),
                },
            );
        let child_modulepreload_event_action = self
            .child_modulepreload_event_action
            .next_ready_metadata()
            .map(
                |ready| RendererPageReadyDescriptor::ChildModulepreloadEventAction {
                    ready,
                    owner: self
                        .child_modulepreload_event_action
                        .next_ready_owner()
                        .expect(
                            "ready child modulepreload event action must retain its exact owner",
                        ),
                },
            );
        let child_frame_task = self.child_frame_task.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::ChildFrameTask {
                ready,
                owner: self
                    .child_frame_task
                    .next_ready_owner()
                    .expect("ready child-frame task must retain its exact Document owner"),
            }
        });
        let v8_foreground_task = self.v8_foreground_task.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::V8ForegroundTask {
                ready,
                owner: self
                    .v8_foreground_task
                    .next_ready_owner()
                    .expect("ready V8 foreground task must retain its Page owner"),
            }
        });
        let module_reaction = self.module_reaction.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::ModuleReaction {
                ready,
                owner: self
                    .module_reaction
                    .next_ready_owner()
                    .expect("ready module reaction must retain its exact owner"),
            }
        });
        let window_message = self.window_message.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::WindowMessage {
                ready,
                owner: self
                    .window_message
                    .next_ready_owner()
                    .expect("ready Window.postMessage task must retain its exact owner"),
                task_id: self
                    .window_message
                    .next_ready_task_id()
                    .expect("ready Window.postMessage task must retain its local id"),
            }
        });
        let message_port_delivery = self
            .message_port_delivery
            .next_ready_metadata()
            .map(|ready| RendererPageReadyDescriptor::MessagePortDelivery {
                ready,
                owner: self
                    .message_port_delivery
                    .next_ready_owner()
                    .expect("ready MessagePort delivery must retain its exact owner"),
                port_id: self
                    .message_port_delivery
                    .next_ready_port_id()
                    .expect("ready MessagePort delivery must retain its port id"),
            });
        let dynamic_import = self
            .dynamic_import_owner_action
            .next_ready_metadata()
            .map(|ready| RendererPageReadyDescriptor::DynamicImportOwnerAction { ready });
        let modulepreload = self
            .modulepreload_start
            .next_ready_metadata()
            .map(|ready| RendererPageReadyDescriptor::ModulepreloadStart { ready });
        let networking = self.networking.next_ready_metadata().map(|ready| {
            RendererPageReadyDescriptor::Networking {
                ready,
                owner: self
                    .networking
                    .next_ready_task_owner()
                    .expect("ready networking task must retain its exact owner"),
            }
        });
        let websocket = self.websocket.next_head().map(
            |RendererPageWebSocketHead {
                 ready,
                 owner,
                 readiness,
             }| RendererPageReadyDescriptor::WebSocket {
                ready,
                owner,
                readiness,
            },
        );
        [
            dom_manipulation,
            user_interaction,
            file_reading,
            misc_platform_api,
            navigation_and_traversal,
            rendering_update,
            media_element_event,
            dedicated_worker_client_event,
            shared_worker_client_event,
            service_worker_internal,
            service_worker_client_message,
            webcrypto_task,
            indexed_db_task,
            opfs_task,
            internal_loading,
            main_document_runtime,
            child_module_dependency_fetch_start,
            child_module_script_terminal,
            child_modulepreload_event_action,
            child_frame_task,
            v8_foreground_task,
            module_reaction,
            window_message,
            message_port_delivery,
            dynamic_import,
            modulepreload,
            networking,
            websocket,
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub(crate) fn take_task(
        &mut self,
        descriptor: RendererPageReadyDescriptor,
    ) -> RendererPageSchedulerTask {
        match descriptor {
            RendererPageReadyDescriptor::ActionWindow { deadline } => {
                RendererPageSchedulerTask::ActionWindow { deadline }
            }
            RendererPageReadyDescriptor::DomManipulation { ready, .. } => {
                let (actual, task) = self
                    .dom_manipulation
                    .pop_front()
                    .expect("selected DOM-manipulation task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected DOM-manipulation head changed before dequeue"
                );
                RendererPageSchedulerTask::DomManipulation(task)
            }
            RendererPageReadyDescriptor::UserInteraction { ready, .. } => {
                let (actual, task) = self
                    .user_interaction
                    .pop_front()
                    .expect("selected user-interaction task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected user-interaction head changed before dequeue"
                );
                RendererPageSchedulerTask::UserInteraction(task)
            }
            RendererPageReadyDescriptor::FileReading { ready, .. } => {
                let (actual, task) = self
                    .file_reading
                    .pop_front()
                    .expect("selected file-reading task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected file-reading head changed before dequeue"
                );
                RendererPageSchedulerTask::FileReading(task)
            }
            RendererPageReadyDescriptor::MiscPlatformApi { ready, .. } => {
                let (actual, task) = self
                    .misc_platform_api
                    .pop_front()
                    .expect("selected miscellaneous-platform task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected miscellaneous-platform head changed before dequeue"
                );
                RendererPageSchedulerTask::MiscPlatformApi(task)
            }
            RendererPageReadyDescriptor::NavigationAndTraversal { ready, .. } => {
                let (actual, task) = self
                    .navigation_and_traversal
                    .pop_front()
                    .expect("selected navigation-and-traversal task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected navigation-and-traversal head changed before dequeue"
                );
                RendererPageSchedulerTask::NavigationAndTraversal(task)
            }
            RendererPageReadyDescriptor::RenderingUpdate { ready, .. } => {
                let (actual, task) = self
                    .rendering_update
                    .pop_front()
                    .expect("selected rendering-update task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected rendering-update head changed before dequeue"
                );
                RendererPageSchedulerTask::RenderingUpdate(task)
            }
            RendererPageReadyDescriptor::MediaElementEvent { ready, .. } => {
                let (actual, task) = self
                    .media_element_event
                    .pop_front()
                    .expect("selected media-element event task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected media-element event head changed before dequeue"
                );
                RendererPageSchedulerTask::MediaElementEvent(task)
            }
            RendererPageReadyDescriptor::DedicatedWorkerClientEvent { ready, .. } => {
                let (actual, task) = self
                    .dedicated_worker_client_event
                    .pop_front()
                    .expect("selected DedicatedWorker client event must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected DedicatedWorker client-event head changed before dequeue"
                );
                RendererPageSchedulerTask::DedicatedWorkerClientEvent(task)
            }
            RendererPageReadyDescriptor::SharedWorkerClientEvent { ready, .. } => {
                let (actual, task) = self
                    .shared_worker_client_event
                    .pop_front()
                    .expect("selected SharedWorker client event must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected SharedWorker client-event head changed before dequeue"
                );
                RendererPageSchedulerTask::SharedWorkerClientEvent(task)
            }
            RendererPageReadyDescriptor::ServiceWorkerInternal { ready, .. } => {
                let (actual, task) = self
                    .service_worker_internal
                    .pop_front()
                    .expect("selected ServiceWorker internal task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected ServiceWorker internal head changed before dequeue"
                );
                RendererPageSchedulerTask::ServiceWorkerInternal(task)
            }
            RendererPageReadyDescriptor::ServiceWorkerClientMessage { ready, .. } => {
                let (actual, task) = self
                    .service_worker_client_message
                    .pop_front()
                    .expect("selected ServiceWorker client message must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected ServiceWorker client-message head changed before dequeue"
                );
                RendererPageSchedulerTask::ServiceWorkerClientMessage(task)
            }
            RendererPageReadyDescriptor::WebCryptoTask { ready, .. } => {
                let (actual, task) = self
                    .webcrypto_task
                    .pop_front()
                    .expect("selected WebCrypto task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected WebCrypto head changed before dequeue"
                );
                RendererPageSchedulerTask::WebCryptoTask(task)
            }
            RendererPageReadyDescriptor::IndexedDbTask { ready, .. } => {
                let (actual, task) = self
                    .indexed_db_task
                    .pop_front()
                    .expect("selected IndexedDB task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected IndexedDB head changed before dequeue"
                );
                RendererPageSchedulerTask::IndexedDbTask(task)
            }
            RendererPageReadyDescriptor::OpfsTask { ready, .. } => {
                let (actual, task) = self
                    .opfs_task
                    .pop_front()
                    .expect("selected OPFS task must remain queued");
                assert_eq!(actual, ready, "selected OPFS head changed before dequeue");
                RendererPageSchedulerTask::OpfsTask(task)
            }
            RendererPageReadyDescriptor::InternalLoading { ready, .. } => {
                let (actual, task) = self
                    .internal_loading
                    .pop_front()
                    .expect("selected internal-loading task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected internal-loading head changed before dequeue"
                );
                RendererPageSchedulerTask::InternalLoading(task)
            }
            RendererPageReadyDescriptor::MainDocumentRuntime { ready, .. } => {
                let (actual, task) = self
                    .main_document_runtime
                    .pop_front()
                    .expect("selected main-Document runtime task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected main-Document runtime head changed before dequeue"
                );
                RendererPageSchedulerTask::MainDocumentRuntime(task)
            }
            RendererPageReadyDescriptor::ChildModuleDependencyFetchStart { ready, .. } => {
                let (actual, task) = self
                    .child_module_dependency_fetch_start
                    .pop_front()
                    .expect("selected child module dependency fetch start must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected child module dependency fetch start head changed before dequeue"
                );
                RendererPageSchedulerTask::ChildModuleDependencyFetchStart(Box::new(task))
            }
            RendererPageReadyDescriptor::ChildModuleScriptTerminal { ready, .. } => {
                let (actual, task) = self
                    .child_module_script_terminal
                    .pop_front()
                    .expect("selected child module-script terminal must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected child module-script terminal head changed before dequeue"
                );
                RendererPageSchedulerTask::ChildModuleScriptTerminal(task)
            }
            RendererPageReadyDescriptor::ChildModulepreloadEventAction { ready, .. } => {
                let (actual, task) = self
                    .child_modulepreload_event_action
                    .pop_front()
                    .expect("selected child modulepreload event action must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected child modulepreload event action head changed before dequeue"
                );
                RendererPageSchedulerTask::ChildModulepreloadEventAction(task)
            }
            RendererPageReadyDescriptor::ChildFrameTask { ready, .. } => {
                let (actual, task) = self
                    .child_frame_task
                    .pop_front()
                    .expect("selected child-frame task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected child-frame task head changed before dequeue"
                );
                RendererPageSchedulerTask::ChildFrameTask(task)
            }
            RendererPageReadyDescriptor::V8ForegroundTask { ready, .. } => {
                let (actual, task) = self
                    .v8_foreground_task
                    .pop_front()
                    .expect("selected V8 foreground task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected V8 foreground task head changed before dequeue"
                );
                RendererPageSchedulerTask::V8ForegroundTask(task)
            }
            RendererPageReadyDescriptor::ModuleReaction { ready, .. } => {
                let (actual, task) = self
                    .module_reaction
                    .pop_front()
                    .expect("selected module reaction must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected module-reaction head changed before dequeue"
                );
                RendererPageSchedulerTask::ModuleReaction(task)
            }
            RendererPageReadyDescriptor::WindowMessage { ready, .. } => {
                let (actual, task) = self
                    .window_message
                    .pop_front()
                    .expect("selected Window.postMessage task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected Window.postMessage head changed before dequeue"
                );
                RendererPageSchedulerTask::WindowMessage(task)
            }
            RendererPageReadyDescriptor::MessagePortDelivery { ready, .. } => {
                let (actual, task) = self
                    .message_port_delivery
                    .pop_front()
                    .expect("selected MessagePort delivery must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected MessagePort delivery head changed before dequeue"
                );
                let same_attachment_task_is_ready = self
                    .message_port_delivery
                    .has_ready_task_for(task.owner(), task.port_id());
                RendererPageSchedulerTask::MessagePortDelivery {
                    task,
                    same_attachment_task_is_ready,
                }
            }
            RendererPageReadyDescriptor::DynamicImportOwnerAction { ready } => {
                let (actual, task) = self
                    .dynamic_import_owner_action
                    .pop_front()
                    .expect("selected dynamic-import owner action must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected dynamic-import head changed before dequeue"
                );
                RendererPageSchedulerTask::DynamicImportOwnerAction(task)
            }
            RendererPageReadyDescriptor::ModulepreloadStart { ready } => {
                let (actual, task) = self
                    .modulepreload_start
                    .pop_front()
                    .expect("selected modulepreload start task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected modulepreload head changed before dequeue"
                );
                RendererPageSchedulerTask::ModulepreloadStart(task)
            }
            RendererPageReadyDescriptor::Networking { ready, .. } => {
                let (actual, task) = self
                    .networking
                    .pop_front_task()
                    .expect("selected Page networking task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected networking head changed before dequeue"
                );
                RendererPageSchedulerTask::Networking(task)
            }
            RendererPageReadyDescriptor::WebSocket {
                ready,
                owner,
                readiness,
            } => {
                let (actual, task) = self
                    .websocket
                    .pop_head(RendererPageWebSocketHead {
                        ready,
                        owner,
                        readiness,
                    })
                    .expect("selected WebSocket task must remain queued");
                assert_eq!(
                    actual, ready,
                    "selected WebSocket head changed before dequeue"
                );
                RendererPageSchedulerTask::WebSocket(task)
            }
            RendererPageReadyDescriptor::Timer { deadline } => {
                RendererPageSchedulerTask::Timer { deadline }
            }
        }
    }

    /// Whether any stable source still owns a task payload.
    ///
    /// This is deliberately broader than scheduler eligibility: a current-
    /// Document WebSocketStream event may remain resident while readable-side
    /// backpressure makes it temporarily non-runnable.
    pub(crate) fn has_resident_task(&mut self) -> bool {
        self.dom_manipulation.has_ready_task()
            || self.user_interaction.has_ready_task()
            || self.file_reading.has_ready_task()
            || self.misc_platform_api.has_ready_task()
            || self.navigation_and_traversal.has_ready_task()
            || self.rendering_update.has_ready_task()
            || self.media_element_event.has_ready_task()
            || self.dedicated_worker_client_event.has_ready_task()
            || self.shared_worker_client_event.has_ready_task()
            || self.service_worker_internal.has_ready_task()
            || self.service_worker_client_message.has_ready_task()
            || self.webcrypto_task.has_ready_task()
            || self.indexed_db_task.has_ready_task()
            || self.opfs_task.has_ready_task()
            || self.internal_loading.has_ready_task()
            || self.main_document_runtime.has_ready_task()
            || self.child_module_dependency_fetch_start.has_ready_task()
            || self.child_module_script_terminal.has_ready_task()
            || self.child_modulepreload_event_action.has_ready_task()
            || self.child_frame_task.has_ready_task()
            || self.v8_foreground_task.has_ready_task()
            || self.module_reaction.has_ready_task()
            || self.window_message.has_ready_task()
            || self.message_port_delivery.has_ready_task()
            || self.networking.has_ready_task()
            || self.websocket.has_resident_task()
            || self.modulepreload_start.has_ready_task()
            || self.dynamic_import_owner_action.has_ready_task()
    }

    pub(crate) fn has_ready_networking_task_for(
        &mut self,
        current_document: RendererDocumentToken,
    ) -> bool {
        self.networking.has_ready_task() || self.websocket.has_runnable_task_for(current_document)
    }

    #[cfg(test)]
    pub(crate) fn has_ready_service_worker_task(&mut self) -> bool {
        self.service_worker_internal.has_ready_task()
            || self.service_worker_client_message.has_ready_task()
    }

    pub(crate) fn clear(&mut self) {
        self.dom_manipulation.clear();
        self.user_interaction.clear();
        self.file_reading.clear();
        self.misc_platform_api.clear();
        self.navigation_and_traversal.clear();
        self.rendering_update.clear();
        self.media_element_event.clear();
        self.dedicated_worker_client_event.clear();
        self.shared_worker_client_event.clear();
        self.service_worker_internal.clear();
        self.service_worker_client_message.clear();
        self.webcrypto_task.clear();
        self.indexed_db_task.clear();
        self.opfs_task.clear();
        self.internal_loading.clear();
        self.main_document_runtime.clear();
        self.child_module_dependency_fetch_start.clear();
        self.child_module_script_terminal.clear();
        self.child_modulepreload_event_action.clear();
        self.child_frame_task.clear();
        self.v8_foreground_task.clear();
        self.module_reaction.clear();
        self.window_message.clear();
        self.message_port_delivery.clear();
        self.networking.clear();
        self.websocket.clear();
        self.modulepreload_start.clear();
        self.dynamic_import_owner_action.clear();
    }
}

impl RendererPageTaskProducerRoutes {
    pub(crate) fn v8_foreground_task_sender(&self) -> RendererPageV8ForegroundTaskSender {
        self.v8_foreground_task.clone()
    }

    pub(crate) fn module_reaction_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageModuleReactionSender {
        self.module_reaction.sender(root_document)
    }

    pub(crate) fn dom_manipulation_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageDomManipulationSender {
        RendererPageDomManipulationSender::new(self.dom_manipulation.clone(), root_document)
    }

    pub(crate) fn user_interaction_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageUserInteractionSender {
        self.user_interaction.sender(root_document)
    }

    pub(crate) fn file_reading_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageFileReadingSender {
        self.file_reading.sender(root_document)
    }

    pub(crate) fn misc_platform_api_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageMiscPlatformApiSender {
        self.misc_platform_api.sender(root_document)
    }

    pub(crate) fn navigation_and_traversal_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageNavigationAndTraversalSender {
        RendererPageNavigationAndTraversalSender::new(
            self.navigation_and_traversal.clone(),
            root_document,
        )
    }

    pub(crate) fn rendering_update_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageRenderingUpdateSender {
        RendererPageRenderingUpdateSender::new(self.rendering_update.clone(), root_document)
    }

    pub(crate) fn media_element_event_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageMediaElementEventSender {
        self.media_element_event.sender(root_document)
    }

    pub(crate) fn dedicated_worker_client_event_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageDedicatedWorkerClientEventSender {
        self.dedicated_worker_client_event.sender(root_document)
    }

    pub(crate) fn shared_worker_client_event_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageSharedWorkerClientEventSender {
        self.shared_worker_client_event.sender(root_document)
    }

    pub(crate) fn service_worker_task_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageServiceWorkerTaskSender {
        RendererPageServiceWorkerTaskSender::new(
            self.service_worker_internal.sender(root_document),
            self.service_worker_client_message.sender(root_document),
        )
    }

    pub(crate) fn worker_host_bridge_event_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererWorkerHostBridgeEventSender {
        RendererWorkerHostBridgeEventSender::new(self.networking.clone(), root_document)
    }

    pub(crate) fn webcrypto_task_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWebCryptoTaskSender {
        self.webcrypto_task.sender(root_document)
    }

    pub(crate) fn indexed_db_task_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageIndexedDbTaskSender {
        self.indexed_db_task.sender(root_document)
    }

    pub(crate) fn opfs_task_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageOpfsTaskSender {
        self.opfs_task.sender(root_document)
    }

    pub(crate) fn internal_loading_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageInternalLoadingSender {
        self.internal_loading.sender(root_document)
    }

    pub(crate) fn main_document_runtime_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageMainDocumentRuntimeSender {
        self.main_document_runtime.sender(root_document)
    }

    pub(crate) fn stylesheet_task_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageStylesheetTaskSender {
        RendererPageStylesheetTaskSender::new(
            self.networking.clone(),
            self.dom_manipulation.clone(),
            root_document,
        )
    }

    pub(crate) fn main_parser_continuation_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageMainParserContinuationSender {
        RendererPageMainParserContinuationSender::new(self.networking.clone(), root_document)
    }

    pub(crate) fn child_module_dependency_fetch_start_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageChildModuleDependencyFetchStartSender {
        self.child_module_dependency_fetch_start
            .sender(root_document)
    }

    pub(crate) fn child_module_script_terminal_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageChildModuleScriptTerminalSender {
        self.child_module_script_terminal.sender(root_document)
    }

    pub(crate) fn child_modulepreload_event_action_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageChildModulepreloadEventActionSender {
        self.child_modulepreload_event_action.sender(root_document)
    }

    pub(crate) fn child_frame_task_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageChildFrameTaskSender {
        self.child_frame_task.sender(root_document)
    }

    pub(crate) fn window_message_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWindowMessageSender {
        self.window_message.sender(root_document)
    }

    pub(crate) fn message_port_delivery_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageMessagePortDeliverySender {
        self.message_port_delivery.sender(root_document)
    }

    pub(crate) fn resource_completion_sender(&self) -> RendererPageResourceCompletionSender {
        RendererPageResourceCompletionSender::new(self.networking.clone())
    }

    pub(crate) fn websocket_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWebSocketSender {
        self.websocket.sender(root_document)
    }

    pub(crate) fn text_track_load_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageTextTrackLoadSender {
        RendererPageTextTrackLoadSender::new(
            self.networking.clone(),
            self.dom_manipulation.clone(),
            root_document,
        )
    }

    pub(crate) fn modulepreload_start_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageModulepreloadStartSender {
        self.modulepreload_start.sender(root_document)
    }

    pub(crate) fn dynamic_import_owner_action_sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageDynamicImportOwnerActionSender {
        self.dynamic_import_owner_action.sender(root_document)
    }
}

#[cfg(test)]
mod conformance_tests;

/// Explicit standalone-only handle used by lane executor unit tests.
///
/// Production owner tests never receive this handle: they exercise the unique
/// reservation/slot consumer. Keeping the bypass test-only prevents semantic
/// unit tests from weakening the production ownership type.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct RendererPageOwnedTaskSourcesTestHarness {
    sources: Rc<RefCell<RendererPageOwnedTaskSources>>,
}

#[cfg(test)]
impl RendererPageOwnedTaskSourcesTestHarness {
    pub(crate) fn next_internal_loading_deadline(
        &self,
        current_owner: Option<RendererPageInternalLoadingOwner>,
    ) -> Option<Instant> {
        self.sources
            .borrow_mut()
            .next_internal_loading_deadline(current_owner)
    }

    pub(crate) fn new(sources: RendererPageOwnedTaskSources) -> Self {
        Self {
            sources: Rc::new(RefCell::new(sources)),
        }
    }

    pub(crate) fn has_resident_task(&self) -> bool {
        self.sources.borrow_mut().has_resident_task()
    }

    pub(crate) fn has_ready_networking_task_for(
        &self,
        current_document: RendererDocumentToken,
    ) -> bool {
        self.sources
            .borrow_mut()
            .has_ready_networking_task_for(current_document)
    }

    pub(crate) fn has_ready_service_worker_task(&self) -> bool {
        self.sources.borrow_mut().has_ready_service_worker_task()
    }

    pub(crate) fn has_scheduler_task_for_executor_test(
        &self,
        select: impl FnMut(RendererPageReadyDescriptor) -> bool,
    ) -> bool {
        self.sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .any(select)
    }

    pub(crate) fn next_runnable_websocket_at_for_executor_test(
        &self,
        current_document: RendererDocumentToken,
    ) -> Option<Instant> {
        self.sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::WebSocket {
                        owner,
                        readiness: RendererPageWebSocketReadiness::Ready,
                        ..
                    } if owner.root_document() == current_document
                ) || matches!(
                    descriptor,
                    RendererPageReadyDescriptor::WebSocket { owner, .. }
                        if owner.root_document() != current_document
                )
            })
            .map(RendererPageReadyDescriptor::runnable_since)
    }

    pub(crate) fn next_child_frame_task_target(
        &self,
    ) -> Option<super::RendererPageChildFrameTaskTarget> {
        self.sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find_map(|descriptor| match descriptor {
                RendererPageReadyDescriptor::ChildFrameTask { owner, .. } => Some(owner.target()),
                _ => None,
            })
    }

    /// Dequeue one production scheduler task selected by a narrow semantic
    /// fixture. The selector sees the same descriptor as the owner scheduler;
    /// this helper adds no lane-specific consumer or alternate queue model.
    pub(crate) fn take_scheduler_task_for_executor_test(
        &self,
        mut select: impl FnMut(RendererPageReadyDescriptor) -> bool,
    ) -> Option<RendererPageSchedulerTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| select(*descriptor))?;
        let mut sources = self.sources.borrow_mut();
        let task = sources.take_task(descriptor);
        Some(task)
    }

    /// Dequeue the oldest selected source head using the production
    /// scheduler's ordinary, non-fairness ordering rule.
    ///
    /// This is for semantic fixtures that share several production sources
    /// but do not own a full Page turn scheduler. It preserves cross-source
    /// ready order without claiming to cover the scheduler's bounded-run
    /// fairness state.
    pub(crate) fn take_oldest_scheduler_task_for_executor_test(
        &self,
        mut select: impl FnMut(RendererPageReadyDescriptor) -> bool,
    ) -> Option<RendererPageSchedulerTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .filter(|descriptor| select(*descriptor))
            .min_by_key(|descriptor| {
                (
                    descriptor.runnable_since(),
                    descriptor.enqueue_order().unwrap_or(0),
                    descriptor.source_kind(),
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        Some(sources.take_task(descriptor))
    }

    /// Dequeue one real rendering-update task for a narrow executor test.
    /// Natural wake, fairness, and replacement admission remain owner-loop
    /// integration contracts.
    pub(crate) fn take_rendering_update_for_executor_test(
        &self,
    ) -> Option<RendererPageRenderingUpdateTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::RenderingUpdate { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::RenderingUpdate(task) = sources.take_task(descriptor) else {
            panic!("rendering-update descriptor dequeued a different task variant")
        };
        Some(task)
    }

    /// Dequeue one production Window.postMessage task after applying the same
    /// source-local eligibility predicate as a Page owner turn.
    pub(crate) fn take_window_message_for_executor_test(
        &self,
        mut is_eligible: impl FnMut(
            RendererPageWindowMessageOwner,
            super::window_message::RendererPageWindowMessageTaskId,
        ) -> bool,
    ) -> Option<RendererPageWindowMessageTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::WindowMessage { .. }
                )
            })?;
        let RendererPageReadyDescriptor::WindowMessage { owner, task_id, .. } = descriptor else {
            panic!("filtered Window.postMessage descriptor changed variant")
        };
        if !is_eligible(owner, task_id) {
            return None;
        }
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::WindowMessage(task) = sources.take_task(descriptor) else {
            panic!("Window.postMessage descriptor dequeued a different task variant")
        };
        Some(task)
    }

    /// Dequeue one production MessagePort task for a narrow executor test.
    pub(crate) fn take_message_port_delivery_for_executor_test(
        &self,
    ) -> Option<(RendererPageMessagePortDeliveryTask, bool)> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::MessagePortDelivery { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::MessagePortDelivery {
            task,
            same_attachment_task_is_ready,
        } = sources.take_task(descriptor)
        else {
            panic!("MessagePort descriptor dequeued a different task variant")
        };
        Some((task, same_attachment_task_is_ready))
    }

    pub(crate) fn take_webcrypto_task_for_executor_test(
        &self,
    ) -> Option<RendererPageWebCryptoTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::WebCryptoTask { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::WebCryptoTask(task) = sources.take_task(descriptor) else {
            panic!("WebCrypto descriptor dequeued a different task variant")
        };
        Some(task)
    }

    pub(crate) fn take_indexed_db_task_for_executor_test(
        &self,
    ) -> Option<RendererPageIndexedDbTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::IndexedDbTask { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::IndexedDbTask(task) = sources.take_task(descriptor) else {
            panic!("IndexedDB descriptor dequeued a different task variant")
        };
        Some(task)
    }

    pub(crate) fn take_opfs_task_for_executor_test(&self) -> Option<RendererPageOpfsTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(descriptor, RendererPageReadyDescriptor::OpfsTask { .. })
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::OpfsTask(task) = sources.take_task(descriptor) else {
            panic!("OPFS descriptor dequeued a different task variant")
        };
        Some(task)
    }

    pub(crate) fn take_internal_loading_for_executor_test(
        &self,
    ) -> Option<RendererPageInternalLoadingTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::InternalLoading { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::InternalLoading(task) = sources.take_task(descriptor) else {
            panic!("internal-loading descriptor dequeued a different task variant")
        };
        Some(task)
    }

    /// Dequeue one production main-Document runtime task for a narrow
    /// executor test. Scheduler liveness and cross-source fairness remain
    /// covered by owner-level integration tests.
    pub(crate) fn take_main_document_runtime_for_executor_test(
        &self,
    ) -> Option<RendererPageMainDocumentRuntimeTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::MainDocumentRuntime { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::MainDocumentRuntime(task) = sources.take_task(descriptor)
        else {
            panic!("main-Document runtime descriptor dequeued a different task variant")
        };
        Some(task)
    }

    /// Dequeue only one exact action from the shared main-Document
    /// script-loading source.
    ///
    /// The source head is checked before removal, so a fixture cannot consume
    /// a neighboring runtime-script, parser-module, or post-parse action and
    /// then reinterpret it as the requested action kind.
    pub(crate) fn take_main_document_runtime_action_for_executor_test(
        &self,
        expected_kind: PageMainDocumentRuntimeActionKind,
    ) -> Option<RendererPageMainDocumentRuntimeTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::MainDocumentRuntime { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        if sources.main_document_runtime.next_ready_action_kind() != Some(expected_kind) {
            return None;
        }
        let RendererPageSchedulerTask::MainDocumentRuntime(task) = sources.take_task(descriptor)
        else {
            unreachable!("main-Document runtime descriptor must dequeue its own task variant")
        };
        debug_assert_eq!(task.action_kind(), expected_kind);
        Some(task)
    }

    /// Observe whether the shared main-Document runtime source retains a
    /// concrete action, without bypassing FIFO to execute it.
    ///
    /// This is useful when a selected task must prove publication behind an
    /// earlier action already resident in the same production source.
    pub(crate) fn has_main_document_runtime_action_for_executor_test(
        &self,
        expected_kind: PageMainDocumentRuntimeActionKind,
    ) -> bool {
        self.sources
            .borrow_mut()
            .main_document_runtime
            .has_ready_action_kind(expected_kind)
    }

    pub(crate) fn take_child_module_dependency_fetch_start_for_executor_test(
        &self,
    ) -> Option<RendererPageChildModuleDependencyFetchStartTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::ChildModuleDependencyFetchStart { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::ChildModuleDependencyFetchStart(task) =
            sources.take_task(descriptor)
        else {
            panic!("child module dependency fetch-start descriptor dequeued another variant")
        };
        Some(*task)
    }

    pub(crate) fn take_child_module_script_terminal_for_executor_test(
        &self,
        mut is_eligible: impl FnMut(RendererPageChildModuleScriptTerminalOwner) -> bool,
    ) -> Option<RendererPageChildModuleScriptTerminalTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::ChildModuleScriptTerminal { .. }
                )
            })?;
        let RendererPageReadyDescriptor::ChildModuleScriptTerminal { owner, .. } = descriptor
        else {
            panic!("filtered child module-script terminal descriptor changed variant")
        };
        if !is_eligible(owner) {
            return None;
        }
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::ChildModuleScriptTerminal(task) =
            sources.take_task(descriptor)
        else {
            panic!("child module-script terminal descriptor dequeued another variant")
        };
        Some(task)
    }

    pub(crate) fn take_child_modulepreload_event_action_for_executor_test(
        &self,
        mut is_eligible: impl FnMut(RendererPageChildModulepreloadEventActionOwner) -> bool,
    ) -> Option<RendererPageChildModulepreloadEventActionTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::ChildModulepreloadEventAction { .. }
                )
            })?;
        let RendererPageReadyDescriptor::ChildModulepreloadEventAction { owner, .. } = descriptor
        else {
            panic!("filtered child modulepreload event descriptor changed variant")
        };
        if !is_eligible(owner) {
            return None;
        }
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::ChildModulepreloadEventAction(task) =
            sources.take_task(descriptor)
        else {
            panic!("child modulepreload event descriptor dequeued a different task variant")
        };
        Some(task)
    }

    pub(crate) fn take_module_reaction_for_executor_test(
        &self,
    ) -> Option<RendererPageModuleReactionTask> {
        let descriptor = self
            .sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::ModuleReaction { .. }
                )
            })?;
        let mut sources = self.sources.borrow_mut();
        let RendererPageSchedulerTask::ModuleReaction(task) = sources.take_task(descriptor) else {
            panic!("module-reaction descriptor dequeued a different task variant")
        };
        Some(task)
    }

    pub(crate) fn has_module_reaction_for_executor_test(&self) -> bool {
        self.sources
            .borrow_mut()
            .ready_descriptors()
            .into_iter()
            .any(|descriptor| {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::ModuleReaction { .. }
                )
            })
    }

    pub(crate) fn resource_completion(&self) -> RendererPageResourceCompletionTestSource {
        RendererPageResourceCompletionTestSource {
            sources: self.sources.clone(),
        }
    }

    pub(crate) fn modulepreload_start(&self) -> RendererPageModulepreloadStartTestSource {
        RendererPageModulepreloadStartTestSource {
            sources: self.sources.clone(),
        }
    }

    pub(crate) fn dynamic_import_owner_action(
        &self,
    ) -> RendererPageDynamicImportOwnerActionTestSource {
        RendererPageDynamicImportOwnerActionTestSource {
            sources: self.sources.clone(),
        }
    }

    pub(crate) fn clear(&self) {
        self.sources.borrow_mut().clear();
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct RendererPageResourceCompletionTestSource {
    sources: Rc<RefCell<RendererPageOwnedTaskSources>>,
}

#[cfg(test)]
impl RendererPageResourceCompletionTestSource {
    pub(crate) fn next_ready_owner(&self) -> Option<RendererPageResourceCompletionOwner> {
        self.sources.borrow_mut().networking.next_ready_owner()
    }

    pub(crate) fn has_ready_completion(&self) -> bool {
        self.next_ready_owner().is_some()
    }

    pub(crate) fn pop_front(
        &self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageResourceCompletion,
    )> {
        self.sources.borrow_mut().networking.pop_front()
    }

    pub(crate) fn enqueue_local_for_test(&self, completion: RendererPageResourceCompletion) {
        self.sources
            .borrow_mut()
            .networking
            .enqueue_local_for_test(completion);
    }
}

#[cfg(test)]
impl crate::page_resource_completion::RendererPageResourceCompletionTestSource
    for RendererPageResourceCompletionTestSource
{
    fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.next_ready_owner()?;
        self.sources.borrow_mut().networking.next_ready_metadata()
    }

    fn next_ready_owner(&mut self) -> Option<RendererPageResourceCompletionOwner> {
        RendererPageResourceCompletionTestSource::next_ready_owner(self)
    }

    fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageResourceCompletion,
    )> {
        RendererPageResourceCompletionTestSource::pop_front(self)
    }

    fn has_ready_completion(&mut self) -> bool {
        RendererPageResourceCompletionTestSource::has_ready_completion(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PageId,
        document_runtime::DomHandle,
        frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        },
        native_bridge::{
            OwnerDispatchScope, TextTrackLoadSequenceId, WindowDocumentOwner,
            WindowDocumentTaskTarget,
        },
        page_task_queue::{
            RendererOwnerWakeSource, RendererPageTextTrackLoadTaskId,
            RendererPageTextTrackLoadTaskKind,
        },
        types::{ChildDocumentLoadCompletion, DocumentWriteExternalScriptLoadCompletion},
    };

    fn document_token(lifecycle_document_id: u64) -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(PageId::new_for_testing(9001), lifecycle_document_id)
    }

    fn resource_completion(load_id: u64) -> RendererPageResourceCompletion {
        RendererPageResourceCompletion::document_write_external_script(
            document_token(1),
            DocumentWriteExternalScriptLoadCompletion::for_test(load_id),
        )
    }

    fn child_document_completion(load_id: u64) -> RendererPageResourceCompletion {
        RendererPageResourceCompletion::child_document_load(
            document_token(1),
            ChildDocumentLoadCompletion::for_test(
                load_id,
                moli_dom::NodeId::new(7),
                Err("child load failed for aggregate FIFO test".to_owned()),
            ),
        )
    }

    fn websocket_close_event(socket_id: u64) -> moli_websocket::Event {
        moli_websocket::Event::Close {
            socket_id,
            code: 1005,
            reason: String::new(),
            was_clean: true,
        }
    }

    fn owned_sources() -> (RendererPageOwnedTaskSources, RendererPageTaskProducerRoutes) {
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        RendererPageOwnedTaskSources::new(
            PageRuntimeWakeSignal::default(),
            RendererOwnerWakeSender::new(
                wake_tx,
                crate::runtime::RendererPageToken::new_for_testing(document_token(1).page_id),
            ),
        )
    }

    #[test]
    fn replacement_routes_reuse_the_unique_page_consumer() {
        let (mut sources, initial_routes) = owned_sources();
        let replacement_routes = sources.producer_routes();

        assert!(sources.routes_match(&initial_routes));
        assert!(sources.routes_match(&replacement_routes));
        replacement_routes
            .resource_completion_sender()
            .send(resource_completion(11))
            .expect("replacement producer route should retain the stable Page consumer");

        let descriptor = sources
            .ready_descriptors()
            .into_iter()
            .next()
            .expect("replacement payload should be visible to the stable Page arbiter");
        assert!(matches!(
            sources.take_task(descriptor),
            RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::ResourceCompletion(
                _
            ))
        ));
        assert!(!sources.has_resident_task());
    }

    #[test]
    fn one_selected_source_head_leaves_exactly_one_continuation_candidate() {
        let (mut sources, routes) = owned_sources();
        let sender = routes.resource_completion_sender();
        sender
            .send(resource_completion(17))
            .expect("first resource terminal should enqueue");
        sender
            .send(resource_completion(19))
            .expect("second resource terminal should enqueue");

        let first = sources
            .ready_descriptors()
            .into_iter()
            .next()
            .expect("first resource head should be ready");
        assert!(matches!(
            sources.take_task(first),
            RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::ResourceCompletion(
                _
            ))
        ));
        assert!(sources.has_resident_task());

        let second = sources
            .ready_descriptors()
            .into_iter()
            .next()
            .expect("second resource head should remain ready");
        assert!(matches!(
            sources.take_task(second),
            RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::ResourceCompletion(
                _
            ))
        ));
        assert!(!sources.has_resident_task());
    }

    #[test]
    fn mixed_resource_terminal_variants_share_one_fifo_source_descriptor() {
        let (mut sources, routes) = owned_sources();
        let sender = routes.resource_completion_sender();
        sender
            .send(resource_completion(31))
            .expect("document.write terminal should enqueue");
        sender
            .send(child_document_completion(37))
            .expect("child-document terminal should enqueue behind it");

        let first_descriptors = sources.ready_descriptors();
        assert_eq!(first_descriptors.len(), 1);
        let RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::ResourceCompletion(
            first,
        )) = sources.take_task(first_descriptors[0])
        else {
            panic!("resource descriptor must dequeue a resource terminal")
        };
        assert!(matches!(
            first.terminal(),
            crate::page_resource_completion::RendererPageResourceTerminal::DocumentWriteExternalScript {
                completion
            } if completion.load_id() == 31
        ));
        assert!(sources.has_resident_task());

        let second_descriptors = sources.ready_descriptors();
        assert_eq!(second_descriptors.len(), 1);
        let RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::ResourceCompletion(
            second,
        )) = sources.take_task(second_descriptors[0])
        else {
            panic!("remaining resource descriptor must dequeue the next terminal")
        };
        assert!(matches!(
            second.terminal(),
            crate::page_resource_completion::RendererPageResourceTerminal::ChildDocumentLoad {
                completion
            } if completion.load_id() == 37
        ));
        assert!(!sources.has_resident_task());
    }

    #[test]
    fn resource_terminal_and_text_track_start_share_networking_fifo_and_readiness_edge() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let (mut sources, routes) = RendererPageOwnedTaskSources::new(
            PageRuntimeWakeSignal::default(),
            RendererOwnerWakeSender::new(
                wake_tx,
                crate::runtime::RendererPageToken::new_for_testing(document_token(1).page_id),
            ),
        );
        routes
            .resource_completion_sender()
            .send(resource_completion(41))
            .expect("resource terminal should enter Networking");
        routes
            .text_track_load_sender(document_token(1))
            .send(
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(1),
                        LocalWindowId(2),
                        DocumentId(3),
                    )),
                    OwnerDispatchScope::Top,
                ),
                RendererPageTextTrackLoadTaskId::new(
                    DomHandle::new(9),
                    TextTrackLoadSequenceId::new(5),
                ),
                RendererPageTextTrackLoadTaskKind::Start,
            )
            .expect("text-track start should enter the same Networking source");

        assert_eq!(
            wake_rx
                .try_recv()
                .expect("empty-to-nonempty Networking transition should wake")
                .source_for_test(),
            RendererOwnerWakeSource::NetworkingTask
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "enqueueing behind a ready Networking head must not duplicate the readiness edge"
        );
        let descriptors = sources.ready_descriptors();
        assert_eq!(descriptors.len(), 1);
        assert!(matches!(
            sources.take_task(descriptors[0]),
            RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::ResourceCompletion(
                _
            ))
        ));
        let descriptors = sources.ready_descriptors();
        assert_eq!(descriptors.len(), 1);
        assert!(matches!(
            sources.take_task(descriptors[0]),
            RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::TextTrackLoad(
                task
            )) if task.kind() == RendererPageTextTrackLoadTaskKind::Start
        ));
        assert!(!sources.has_resident_task());
    }

    #[tokio::test]
    async fn blocked_websocket_does_not_hide_other_networking_work() {
        let (mut sources, routes) = owned_sources();
        let current_document = document_token(1);
        let websocket = routes.websocket_sender(current_document);
        assert!(
            websocket
                .event_sender()
                .send(websocket_close_event(51))
                .await
        );
        let descriptor = sources
            .ready_descriptors()
            .into_iter()
            .find(|descriptor| matches!(descriptor, RendererPageReadyDescriptor::WebSocket { .. }))
            .expect("WebSocket ingress should publish one source head");
        let RendererPageSchedulerTask::WebSocket(task) = sources.take_task(descriptor) else {
            panic!("WebSocket descriptor must dequeue its own task")
        };
        task.return_backpressured();

        assert!(
            !sources.has_ready_networking_task_for(current_document),
            "current-Document backpressure alone must not report runnable Networking work"
        );
        routes
            .resource_completion_sender()
            .send(resource_completion(53))
            .expect("unrelated Networking terminal should enqueue behind blocked WebSocket state");
        assert!(sources.has_ready_networking_task_for(current_document));
        let descriptors = sources.ready_descriptors();
        assert!(descriptors.iter().any(|descriptor| matches!(
            descriptor,
            RendererPageReadyDescriptor::WebSocket {
                readiness: RendererPageWebSocketReadiness::Backpressured,
                ..
            }
        )));
        let resource = descriptors
            .into_iter()
            .find(|descriptor| matches!(descriptor, RendererPageReadyDescriptor::Networking { .. }))
            .expect("ordinary Networking head must remain independently visible");
        assert!(matches!(
            sources.take_task(resource),
            RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::ResourceCompletion(
                _
            ))
        ));
        assert!(!sources.has_ready_networking_task_for(current_document));
        assert!(
            sources.has_ready_networking_task_for(document_token(2)),
            "a replacement Document must be able to retire the stale blocked WebSocket task"
        );
    }

    #[test]
    fn dropping_the_unique_page_consumer_closes_old_and_replacement_routes() {
        let (sources, initial_routes) = owned_sources();
        let replacement_routes = sources.producer_routes();
        let initial_sender = initial_routes.resource_completion_sender();
        let replacement_sender = replacement_routes.resource_completion_sender();
        let initial_text_track = initial_routes.text_track_load_sender(document_token(1));
        let replacement_text_track = replacement_routes.text_track_load_sender(document_token(1));
        let target = WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(1),
                LocalWindowId(2),
                DocumentId(3),
            )),
            OwnerDispatchScope::Top,
        );

        drop(sources);

        assert!(initial_sender.send(resource_completion(23)).is_err());
        assert!(replacement_sender.send(resource_completion(29)).is_err());
        assert!(
            initial_text_track
                .send(
                    target,
                    RendererPageTextTrackLoadTaskId::new(
                        DomHandle::new(7),
                        TextTrackLoadSequenceId::new(11),
                    ),
                    RendererPageTextTrackLoadTaskKind::Start,
                )
                .is_err(),
            "a retired Networking consumer must close text-track start routes"
        );
        assert!(
            replacement_text_track
                .send(
                    target,
                    RendererPageTextTrackLoadTaskId::new(
                        DomHandle::new(7),
                        TextTrackLoadSequenceId::new(12),
                    ),
                    RendererPageTextTrackLoadTaskKind::FetchFailureTerminal,
                )
                .is_err(),
            "a retired DOM consumer must close text-track fetch-failure routes"
        );
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct RendererPageModulepreloadStartTestSource {
    sources: Rc<RefCell<RendererPageOwnedTaskSources>>,
}

#[cfg(test)]
impl RendererPageModulepreloadStartTestSource {
    pub(crate) fn enqueue_local_for_test(
        &self,
        root_document: RendererDocumentToken,
        task: crate::frame_owner_model::FrameDocumentModulepreloadFetchTask,
    ) {
        self.sources
            .borrow_mut()
            .modulepreload_start
            .enqueue_local_for_test(root_document, task);
    }

    pub(crate) fn has_ready_task(&self) -> bool {
        self.sources
            .borrow_mut()
            .modulepreload_start
            .has_ready_task()
    }

    pub(crate) fn pop_front(
        &self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageModulepreloadStartTask,
    )> {
        self.sources.borrow_mut().modulepreload_start.pop_front()
    }

    pub(crate) fn next_ready_metadata(&self) -> Option<RendererPageTaskReadyMetadata> {
        self.sources
            .borrow_mut()
            .modulepreload_start
            .next_ready_metadata()
    }

    pub(crate) fn next_ready_owner(&self) -> Option<super::RendererPageModulepreloadStartOwner> {
        self.sources
            .borrow_mut()
            .modulepreload_start
            .next_ready_owner()
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct RendererPageDynamicImportOwnerActionTestSource {
    sources: Rc<RefCell<RendererPageOwnedTaskSources>>,
}

#[cfg(test)]
impl RendererPageDynamicImportOwnerActionTestSource {
    pub(crate) fn enqueue_local_for_test(
        &self,
        root_document: RendererDocumentToken,
        action: crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction,
    ) {
        self.sources
            .borrow_mut()
            .dynamic_import_owner_action
            .enqueue_local_for_test(root_document, action);
    }

    pub(crate) fn has_ready_task(&self) -> bool {
        self.sources
            .borrow_mut()
            .dynamic_import_owner_action
            .has_ready_task()
    }

    pub(crate) fn pop_front(
        &self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageDynamicImportOwnerActionTask,
    )> {
        self.sources
            .borrow_mut()
            .dynamic_import_owner_action
            .pop_front()
    }

    pub(crate) fn next_ready_metadata(&self) -> Option<RendererPageTaskReadyMetadata> {
        self.sources
            .borrow_mut()
            .dynamic_import_owner_action
            .next_ready_metadata()
    }

    pub(crate) fn next_ready_owner(
        &self,
    ) -> Option<super::RendererPageDynamicImportOwnerActionOwner> {
        self.sources
            .borrow_mut()
            .dynamic_import_owner_action
            .next_ready_owner()
    }
}
