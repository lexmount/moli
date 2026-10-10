use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    page_resource_completion::{
        PageResourceCompletionTurnAction, RendererPageResourceCompletion,
        RendererPageResourceCompletionOwner,
    },
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::PageOwnerTurnOutcome,
};

use super::{
    PageConnectedStyleEventTurnAction, PageMainParserContinuationTurnAction, PageRuntimeWakeSignal,
    PageStylesheetNetworkingTurnAction, PageTextTrackLoadTurnAction,
    PageWorkerHostBridgeTurnAction, RendererOwnerWakeSender, RendererPageConnectedStyleEventTask,
    RendererPageMainParserContinuationOwner, RendererPageMainParserContinuationTask,
    RendererPageStylesheetNetworkingTask, RendererPageStylesheetTaskOwner,
    RendererPageTextTrackLoadOwner, RendererPageTextTrackLoadTask,
    RendererPageWorkerHostBridgeOwner, RendererPageWorkerHostBridgeTask,
};

/// Exact owner of the head of the HTML networking task source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageNetworkingOwner {
    ResourceCompletion(RendererPageResourceCompletionOwner),
    MainParserContinuation(RendererPageMainParserContinuationOwner),
    StyleElementEvent(RendererPageStylesheetTaskOwner),
    TextTrackLoad(RendererPageTextTrackLoadOwner),
    StylesheetCompletion(RendererPageStylesheetTaskOwner),
    WorkerHostBridge(RendererPageWorkerHostBridgeOwner),
    WebRtc(super::RendererPageWebRtcOwner),
}

/// One concrete task in the Page-owned HTML networking source.
#[derive(Debug)]
pub(crate) enum RendererPageNetworkingTask {
    ResourceCompletion(Box<RendererPageResourceCompletion>),
    MainParserContinuation(RendererPageMainParserContinuationTask),
    StyleElementEvent(RendererPageConnectedStyleEventTask),
    TextTrackLoad(RendererPageTextTrackLoadTask),
    StylesheetCompletion(RendererPageStylesheetNetworkingTask),
    WorkerHostBridge(RendererPageWorkerHostBridgeTask),
    WebRtc(super::RendererPageWebRtcTask),
}

impl RendererPageNetworkingTask {
    pub(crate) fn owner(&self) -> RendererPageNetworkingOwner {
        match self {
            Self::WebRtc(task) => RendererPageNetworkingOwner::WebRtc(task.owner()),
            Self::ResourceCompletion(completion) => {
                RendererPageNetworkingOwner::ResourceCompletion(completion.owner())
            }
            Self::MainParserContinuation(task) => {
                RendererPageNetworkingOwner::MainParserContinuation(task.owner())
            }
            Self::StyleElementEvent(task) => {
                RendererPageNetworkingOwner::StyleElementEvent(task.owner())
            }
            Self::TextTrackLoad(task) => RendererPageNetworkingOwner::TextTrackLoad(task.owner()),
            Self::StylesheetCompletion(task) => {
                RendererPageNetworkingOwner::StylesheetCompletion(task.owner())
            }
            Self::WorkerHostBridge(task) => {
                RendererPageNetworkingOwner::WorkerHostBridge(task.owner())
            }
        }
    }

    fn release_dequeue_permits(&mut self) {
        if let Self::MainParserContinuation(task) = self {
            task.release_permit_at_dequeue();
        }
    }
}

impl From<RendererPageResourceCompletion> for RendererPageNetworkingTask {
    fn from(completion: RendererPageResourceCompletion) -> Self {
        Self::ResourceCompletion(Box::new(completion))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageNetworkingRoute {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageNetworkingTask>, NetworkingReadySignal>,
}

impl RendererPageNetworkingRoute {
    pub(crate) fn send(
        &self,
        task: RendererPageNetworkingTask,
    ) -> Result<(), RendererPageNetworkingRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
            .map_err(|_| RendererPageNetworkingRouteClosed)
    }

    pub(crate) fn same_source_as(&self, source: &RendererPageNetworkingSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }

    #[cfg(test)]
    pub(crate) fn same_route_as(&self, other: &Self) -> bool {
        self.task_route.same_route_as(&other.task_route)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageNetworkingRouteClosed;

#[derive(Clone, Debug)]
struct NetworkingReadySignal {
    runtime_wake: PageRuntimeWakeSignal,
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for NetworkingReadySignal {
    fn signal_ready(&self) {
        self.runtime_wake.send();
        self.owner_wake.signal_networking_task();
    }
}

/// Unique Page-lifetime consumer for the HTML networking task source.
///
/// Resource terminals and other networking tasks share one FIFO/fairness slot.
/// Every accepted task is intrinsically runnable; exact owner/currentness is
/// settled only after dequeue. Work with a pre-dequeue blocked condition needs
/// its own durable source instead of filtering this queue's head.
#[derive(Debug)]
pub(crate) struct RendererPageNetworkingSource {
    source: OwnerReadyTaskSource<ReadyPageTask<RendererPageNetworkingTask>, NetworkingReadySignal>,
}

impl RendererPageNetworkingSource {
    pub(crate) fn new(
        runtime_wake: PageRuntimeWakeSignal,
        owner_wake: RendererOwnerWakeSender,
    ) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(NetworkingReadySignal {
                runtime_wake,
                owner_wake,
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_owner_attached(
        runtime_wake: PageRuntimeWakeSignal,
        owner_wake: RendererOwnerWakeSender,
    ) -> Self {
        Self::new(runtime_wake, owner_wake)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        Self::new(
            PageRuntimeWakeSignal::default(),
            RendererOwnerWakeSender::new(
                wake_tx,
                crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(
                    1,
                )),
            ),
        )
    }

    pub(crate) fn route(&self) -> RendererPageNetworkingRoute {
        RendererPageNetworkingRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_task_owner(&mut self) -> Option<RendererPageNetworkingOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front_task(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageNetworkingTask)> {
        let (ready, mut task) = self.source.pop_front().map(ReadyPageTask::into_parts)?;
        task.release_dequeue_permits();
        Some((ready, task))
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    #[cfg(test)]
    pub(crate) fn enqueue_local_for_test(&mut self, task: impl Into<RendererPageNetworkingTask>) {
        self.source.enqueue_local(ReadyPageTask::new(task.into()));
    }

    #[cfg(test)]
    pub(crate) fn sender(
        &self,
    ) -> crate::page_resource_completion::RendererPageResourceCompletionSender {
        crate::page_resource_completion::RendererPageResourceCompletionSender::new(self.route())
    }

    #[cfg(test)]
    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageResourceCompletionOwner> {
        match self.source.front()?.value() {
            RendererPageNetworkingTask::ResourceCompletion(completion) => Some(completion.owner()),
            RendererPageNetworkingTask::MainParserContinuation(_)
            | RendererPageNetworkingTask::StyleElementEvent(_)
            | RendererPageNetworkingTask::TextTrackLoad(_)
            | RendererPageNetworkingTask::StylesheetCompletion(_)
            | RendererPageNetworkingTask::WorkerHostBridge(_)
            | RendererPageNetworkingTask::WebRtc(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageResourceCompletion,
    )> {
        match self.source.front()?.value() {
            RendererPageNetworkingTask::ResourceCompletion(_) => {}
            RendererPageNetworkingTask::MainParserContinuation(_)
            | RendererPageNetworkingTask::StyleElementEvent(_)
            | RendererPageNetworkingTask::TextTrackLoad(_)
            | RendererPageNetworkingTask::StylesheetCompletion(_)
            | RendererPageNetworkingTask::WorkerHostBridge(_)
            | RendererPageNetworkingTask::WebRtc(_) => return None,
        }
        let (ready, task) = self.pop_front_task()?;
        let RendererPageNetworkingTask::ResourceCompletion(completion) = task else {
            unreachable!("resource head was checked before networking dequeue")
        };
        Some((ready, *completion))
    }

    #[cfg(test)]
    pub(crate) fn has_ready_completion(&mut self) -> bool {
        matches!(
            self.source.front().map(ReadyPageTask::value),
            Some(RendererPageNetworkingTask::ResourceCompletion(_))
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageNetworkingTurnAction {
    ResourceCompletion(PageResourceCompletionTurnAction),
    MainParserContinuation(PageMainParserContinuationTurnAction),
    StyleElementEvent(PageConnectedStyleEventTurnAction),
    TextTrackLoad(PageTextTrackLoadTurnAction),
    StylesheetCompletion(PageStylesheetNetworkingTurnAction),
    WorkerHostBridge(PageWorkerHostBridgeTurnAction),
    WebRtc(super::PageWebRtcTurnAction),
}

pub(crate) type PageNetworkingTurnOutcome = PageOwnerTurnOutcome<PageNetworkingTurnAction>;
