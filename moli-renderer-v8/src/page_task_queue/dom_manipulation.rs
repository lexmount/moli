use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

use crate::{
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::PageOwnerTurnOutcome,
};

use super::{
    RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal,
    broadcast_channel_delivery::{
        RendererPageBroadcastChannelDeliveryOwner, RendererPageBroadcastChannelDeliverySender,
        RendererPageBroadcastChannelDeliveryTask,
    },
    element_toggle_event::{
        RendererPageElementToggleEventOwner, RendererPageElementToggleEventSender,
        RendererPageElementToggleEventTask,
    },
    file_entry_file_callback::{
        RendererPageFileEntryFileCallbackOwner, RendererPageFileEntryFileCallbackSender,
        RendererPageFileEntryFileCallbackTask,
    },
    hash_change_delivery::{
        RendererPageHashChangeDeliveryOwner, RendererPageHashChangeDeliverySender,
        RendererPageHashChangeDeliveryTask,
    },
    image_load_event::{
        RendererPageImageLoadEventOwner, RendererPageImageLoadEventSender,
        RendererPageImageLoadEventTask,
    },
    popup_close::{
        RendererPagePopupCloseOwner, RendererPagePopupCloseSender, RendererPagePopupCloseTask,
    },
    popup_load_event::{
        RendererPagePopupLoadEventOwner, RendererPagePopupLoadEventSender,
        RendererPagePopupLoadEventTask,
    },
    promise_rejection::{
        RendererPagePromiseRejectionOwner, RendererPagePromiseRejectionSender,
        RendererPagePromiseRejectionTask,
    },
    script_preparation_error::{
        RendererPageScriptPreparationErrorOwner, RendererPageScriptPreparationErrorSender,
        RendererPageScriptPreparationErrorTask,
    },
    storage_event_delivery::{
        RendererPageStorageEventDeliveryOwner, RendererPageStorageEventDeliverySender,
        RendererPageStorageEventDeliveryTask,
    },
    stylesheet_task::{
        PageConnectedStyleEventTurnAction, RendererPageConnectedStyleEventTask,
        RendererPageStylesheetTaskOwner,
    },
    text_track_default_mode::{
        RendererPageTextTrackDefaultModeOwner, RendererPageTextTrackDefaultModeSender,
        RendererPageTextTrackDefaultModeTask,
    },
    text_track_load::{RendererPageTextTrackLoadOwner, RendererPageTextTrackLoadTask},
    view_transition_update::{
        RendererPageViewTransitionUpdateOwner, RendererPageViewTransitionUpdateSender,
        RendererPageViewTransitionUpdateTask,
    },
};

use crate::runtime::RendererDocumentToken;

/// Exact owner of the head of the HTML DOM-manipulation task source.
///
/// Variants remain typed for source-local authorization, while the outer enum
/// ensures different DOM-manipulation APIs share one FIFO and one fairness
/// slot instead of acquiring an API-specific scheduler source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageDomManipulationOwner {
    BroadcastChannel(RendererPageBroadcastChannelDeliveryOwner),
    StorageEvent(RendererPageStorageEventDeliveryOwner),
    HashChange(RendererPageHashChangeDeliveryOwner),
    ElementToggle(RendererPageElementToggleEventOwner),
    FileEntryFileCallback(RendererPageFileEntryFileCallbackOwner),
    ScriptPreparationError(RendererPageScriptPreparationErrorOwner),
    PromiseRejection(RendererPagePromiseRejectionOwner),
    ImageLoadEvent(RendererPageImageLoadEventOwner),
    MainDocumentLifecycle(super::RendererPageMainDocumentLifecycleOwner),
    ChildDocumentLifecycle(super::RendererPageChildFrameTaskOwner),
    ChildHostLoad(super::RendererPageChildFrameTaskOwner),
    PopupLoadEvent(RendererPagePopupLoadEventOwner),
    PopupClose(RendererPagePopupCloseOwner),
    ConnectedStyleEvent(RendererPageStylesheetTaskOwner),
    TextTrackDefaultMode(RendererPageTextTrackDefaultModeOwner),
    TextTrackLoad(RendererPageTextTrackLoadOwner),
    ViewTransitionUpdate(RendererPageViewTransitionUpdateOwner),
}

/// One concrete task in the Page-owned DOM-manipulation source.
#[derive(Debug)]
pub(crate) enum RendererPageDomManipulationTask {
    BroadcastChannel(RendererPageBroadcastChannelDeliveryTask),
    StorageEvent(RendererPageStorageEventDeliveryTask),
    HashChange(RendererPageHashChangeDeliveryTask),
    ElementToggle(RendererPageElementToggleEventTask),
    FileEntryFileCallback(RendererPageFileEntryFileCallbackTask),
    ScriptPreparationError(RendererPageScriptPreparationErrorTask),
    PromiseRejection(RendererPagePromiseRejectionTask),
    ImageLoadEvent(RendererPageImageLoadEventTask),
    MainDocumentLifecycle(super::RendererPageMainDocumentLifecycleTask),
    ChildDocumentLifecycle(super::RendererPageChildFrameTask),
    ChildHostLoad(super::RendererPageChildFrameTask),
    PopupLoadEvent(RendererPagePopupLoadEventTask),
    PopupClose(RendererPagePopupCloseTask),
    ConnectedStyleEvent(RendererPageConnectedStyleEventTask),
    TextTrackDefaultMode(RendererPageTextTrackDefaultModeTask),
    TextTrackLoad(RendererPageTextTrackLoadTask),
    ViewTransitionUpdate(RendererPageViewTransitionUpdateTask),
}

impl RendererPageDomManipulationTask {
    pub(crate) const fn owner(&self) -> RendererPageDomManipulationOwner {
        match self {
            Self::BroadcastChannel(task) => {
                RendererPageDomManipulationOwner::BroadcastChannel(task.owner())
            }
            Self::StorageEvent(task) => {
                RendererPageDomManipulationOwner::StorageEvent(task.owner())
            }
            Self::HashChange(task) => RendererPageDomManipulationOwner::HashChange(task.owner()),
            Self::ElementToggle(task) => {
                RendererPageDomManipulationOwner::ElementToggle(task.owner())
            }
            Self::FileEntryFileCallback(task) => {
                RendererPageDomManipulationOwner::FileEntryFileCallback(task.owner())
            }
            Self::ScriptPreparationError(task) => {
                RendererPageDomManipulationOwner::ScriptPreparationError(task.owner())
            }
            Self::PromiseRejection(task) => {
                RendererPageDomManipulationOwner::PromiseRejection(task.owner())
            }
            Self::ImageLoadEvent(task) => {
                RendererPageDomManipulationOwner::ImageLoadEvent(task.owner())
            }
            Self::MainDocumentLifecycle(task) => {
                RendererPageDomManipulationOwner::MainDocumentLifecycle(task.owner)
            }
            Self::ChildDocumentLifecycle(task) => {
                RendererPageDomManipulationOwner::ChildDocumentLifecycle(task.owner())
            }
            Self::ChildHostLoad(task) => {
                RendererPageDomManipulationOwner::ChildHostLoad(task.owner())
            }
            Self::PopupLoadEvent(task) => {
                RendererPageDomManipulationOwner::PopupLoadEvent(task.owner())
            }
            Self::PopupClose(task) => RendererPageDomManipulationOwner::PopupClose(task.owner()),
            Self::ConnectedStyleEvent(task) => {
                RendererPageDomManipulationOwner::ConnectedStyleEvent(task.owner())
            }
            Self::TextTrackDefaultMode(task) => {
                RendererPageDomManipulationOwner::TextTrackDefaultMode(task.owner())
            }
            Self::TextTrackLoad(task) => {
                RendererPageDomManipulationOwner::TextTrackLoad(task.owner())
            }
            Self::ViewTransitionUpdate(task) => {
                RendererPageDomManipulationOwner::ViewTransitionUpdate(task.owner())
            }
        }
    }

    fn is_cancelled(&self) -> bool {
        matches!(self, Self::ElementToggle(task) if task.is_cancelled())
    }
}

/// Semantic result of one selected HTML DOM-manipulation task.
///
/// Scheduler and owner-loop wiring deliberately stop at this family boundary.
/// Adding another DOM-manipulation API extends this enum and the family-local
/// executor without creating another fairness source or central turn variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageDomManipulationTurnAction {
    BroadcastChannel(super::PageBroadcastChannelDeliveryTurnAction),
    StorageEvent(super::PageStorageEventDeliveryTurnAction),
    HashChange(super::PageHashChangeDeliveryTurnAction),
    ElementToggle(super::PageElementToggleEventTurnAction),
    FileEntryFileCallback(super::PageFileEntryFileCallbackTurnAction),
    ScriptPreparationError(super::PageScriptPreparationErrorTurnAction),
    PromiseRejection(super::PagePromiseRejectionTurnAction),
    ImageLoadEvent(super::PageImageLoadEventTurnAction),
    MainDocumentLifecycle(super::PageMainDocumentLifecycleTurnAction),
    ChildDocumentLifecycle(super::PageChildDocumentLifecycleTurnAction),
    ChildHostLoad(super::PageChildHostLoadTurnAction),
    PopupLoadEvent(super::PagePopupLoadEventTurnAction),
    PopupClose(super::PagePopupCloseTurnAction),
    ConnectedStyleEvent(PageConnectedStyleEventTurnAction),
    TextTrackDefaultMode(super::PageTextTrackDefaultModeTurnAction),
    TextTrackLoad(super::PageTextTrackLoadTurnAction),
    ViewTransitionUpdate(super::PageViewTransitionUpdateTurnAction),
}

pub(crate) type PageDomManipulationTurnOutcome =
    PageOwnerTurnOutcome<PageDomManipulationTurnAction>;

/// PageVm-stamped producer capability for the shared DOM-manipulation source.
///
/// API-specific typed senders are derived from this family capability. The
/// atomic JsContextHost capability set therefore grows by task-source class,
/// not once for every DOM-manipulation API migrated into the same FIFO.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageDomManipulationSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageDomManipulationSender {
    pub(super) fn new(
        route: RendererPageDomManipulationRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    pub(crate) fn broadcast_channel_delivery(&self) -> RendererPageBroadcastChannelDeliverySender {
        RendererPageBroadcastChannelDeliverySender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn storage_event_delivery(&self) -> RendererPageStorageEventDeliverySender {
        RendererPageStorageEventDeliverySender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn hash_change_delivery(&self) -> RendererPageHashChangeDeliverySender {
        RendererPageHashChangeDeliverySender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn element_toggle_event(&self) -> RendererPageElementToggleEventSender {
        RendererPageElementToggleEventSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn file_entry_file_callback(&self) -> RendererPageFileEntryFileCallbackSender {
        RendererPageFileEntryFileCallbackSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn script_preparation_error(&self) -> RendererPageScriptPreparationErrorSender {
        RendererPageScriptPreparationErrorSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn promise_rejection(&self) -> RendererPagePromiseRejectionSender {
        RendererPagePromiseRejectionSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn image_load_event(&self) -> RendererPageImageLoadEventSender {
        RendererPageImageLoadEventSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn main_document_lifecycle(&self) -> super::RendererPageMainDocumentLifecycleSender {
        super::RendererPageMainDocumentLifecycleSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn send_child_document_lifecycle(
        &self,
        target: super::RendererPageChildDocumentLifecycleTarget,
    ) -> Result<(), super::child_frame_task::RendererPageChildFrameTaskRouteClosed> {
        debug_assert!(!matches!(
            target.action(),
            crate::frame_owner_model::FrameDocumentLifecycleAction::Interactive(_)
        ));
        self.route
            .send(RendererPageDomManipulationTask::ChildDocumentLifecycle(
                super::RendererPageChildFrameTask::new(
                    super::RendererPageChildFrameTaskOwner::new(
                        self.root_document,
                        super::RendererPageChildFrameTaskTarget::DocumentLifecycle(target),
                    ),
                ),
            ))
            .map_err(|_| super::child_frame_task::RendererPageChildFrameTaskRouteClosed)
    }

    pub(crate) fn send_child_host_load(
        &self,
        target: super::RendererPageChildHostLoadTarget,
    ) -> Result<(), super::child_frame_task::RendererPageChildFrameTaskRouteClosed> {
        self.route
            .send(RendererPageDomManipulationTask::ChildHostLoad(
                super::RendererPageChildFrameTask::new(
                    super::RendererPageChildFrameTaskOwner::new(
                        self.root_document,
                        super::RendererPageChildFrameTaskTarget::HostLoad(target),
                    ),
                ),
            ))
            .map_err(|_| super::child_frame_task::RendererPageChildFrameTaskRouteClosed)
    }

    pub(crate) fn popup_load_event(&self) -> RendererPagePopupLoadEventSender {
        RendererPagePopupLoadEventSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn popup_close(&self) -> RendererPagePopupCloseSender {
        RendererPagePopupCloseSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn text_track_default_mode(&self) -> RendererPageTextTrackDefaultModeSender {
        RendererPageTextTrackDefaultModeSender::new(self.route.clone(), self.root_document)
    }

    pub(crate) fn view_transition_update(&self) -> RendererPageViewTransitionUpdateSender {
        RendererPageViewTransitionUpdateSender::new(self.route.clone(), self.root_document)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageDomManipulationRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageDomManipulationTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageDomManipulationRoute {
    pub(crate) fn send(
        &self,
        task: RendererPageDomManipulationTask,
    ) -> Result<(), RendererPageDomManipulationRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
            .map_err(|_| RendererPageDomManipulationRouteClosed)
    }

    #[cfg(test)]
    pub(super) fn same_route_as(&self, other: &Self) -> bool {
        self.task_route.same_route_as(&other.task_route)
    }

    fn same_source_as(&self, source: &RendererPageDomManipulationSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageDomManipulationRouteClosed;

/// Unique consumer for the Page's HTML DOM-manipulation task source.
#[derive(Debug)]
pub(crate) struct RendererPageDomManipulationSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageDomManipulationTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageDomManipulationSource {
    pub(crate) fn has_document_lifecycle_task(
        &mut self,
        is_current: impl Fn(RendererPageDomManipulationOwner) -> bool,
    ) -> bool {
        self.source.has_matching_task(|ready| {
            matches!(
                ready.value(),
                RendererPageDomManipulationTask::MainDocumentLifecycle(_)
                    | RendererPageDomManipulationTask::ChildDocumentLifecycle(_)
                    | RendererPageDomManipulationTask::ChildHostLoad(_)
            ) && is_current(ready.value().owner())
        })
    }

    /// Remove cancelled task closures before they become scheduler-visible.
    ///
    /// Element toggle coalescing cancels and reposts at the tail, matching the
    /// HTML algorithm. Cancellation is queue maintenance, not a browser task:
    /// no ready descriptor, fairness charge, output, or microtask checkpoint is
    /// produced for the cancelled closure.
    fn discard_cancelled_fronts(&mut self) {
        while self
            .source
            .front()
            .is_some_and(|ready| ready.value().is_cancelled())
        {
            let _ = self.source.pop_front();
        }
    }

    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(
                owner_wake,
                RendererOwnerWakeSource::DomManipulationTask,
            )),
        }
    }

    pub(crate) fn route(&self) -> RendererPageDomManipulationRoute {
        RendererPageDomManipulationRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.discard_cancelled_fronts();
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageDomManipulationOwner> {
        self.discard_cancelled_fronts();
        self.source.front().map(|ready| ready.value().owner())
    }

    /// Whether this stable source still owns any visible task payload.
    pub(crate) fn has_ready_task(&mut self) -> bool {
        self.discard_cancelled_fronts();
        self.source.front().is_some()
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageDomManipulationTask,
    )> {
        self.discard_cancelled_fronts();
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageDomManipulationRoute) -> bool {
        route.same_source_as(self)
    }
}
