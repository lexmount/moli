use crate::{
    document_runtime::{
        ConnectedLoadCompletion, ConnectedStyleEventElementKind,
        LiveStylesheetImportLoadCompletion, ReadyConnectedStyleLoad,
    },
    frame_owner_model::FrameDocumentTaskOwner,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
    stylesheet_blocking::StylesheetCompletion,
};

use super::{
    RendererPageDomManipulationRoute, RendererPageDomManipulationTask, RendererPageNetworkingRoute,
    RendererPageNetworkingTask,
};

#[cfg(test)]
use super::dom_manipulation::RendererPageDomManipulationSource;
#[cfg(test)]
use super::{
    PageRuntimeWakeSignal, RendererOwnerWake, RendererOwnerWakeSender,
    networking::RendererPageNetworkingSource,
};

pub(crate) type RendererPageStylesheetTaskOwner = super::RendererPageMainDocumentTaskOwner;

/// One raw stylesheet terminal classified as HTML Networking work.
#[derive(Debug)]
pub(crate) enum RendererPageStylesheetCompletion {
    Blocking(StylesheetCompletion),
    Connected(ConnectedLoadCompletion),
    LiveImport(LiveStylesheetImportLoadCompletion),
}

#[derive(Debug)]
pub(crate) struct RendererPageStylesheetNetworkingTask {
    owner: RendererPageStylesheetTaskOwner,
    completion: RendererPageStylesheetCompletion,
}

impl RendererPageStylesheetNetworkingTask {
    pub(crate) const fn owner(&self) -> RendererPageStylesheetTaskOwner {
        self.owner
    }

    pub(crate) fn into_completion(self) -> RendererPageStylesheetCompletion {
        self.completion
    }
}

/// One already-posted `<link>`/`<style>` load or error event.
///
/// The outer Page task variant records the normative task source:
/// `<style>` events use Networking, while `<link>` events use
/// DOM-manipulation. Keeping the payload source-neutral lets both executors
/// share exact-owner validation and dispatch without weakening that split.
#[derive(Debug)]
pub(crate) struct RendererPageConnectedStyleEventTask {
    owner: RendererPageStylesheetTaskOwner,
    ready: ReadyConnectedStyleLoad,
}

impl RendererPageConnectedStyleEventTask {
    pub(crate) const fn owner(&self) -> RendererPageStylesheetTaskOwner {
        self.owner
    }

    pub(crate) fn into_ready(self) -> ReadyConnectedStyleLoad {
        self.ready
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageStylesheetTaskRouteClosed;

/// PageVm-stamped route pair for the two normative task-source classes used by
/// stylesheet processing. It becomes useful only after it is bound to an exact
/// main-Document epoch.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageStylesheetTaskSender {
    networking: RendererPageNetworkingRoute,
    dom_manipulation: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageStylesheetTaskSender {
    pub(super) fn new(
        networking: RendererPageNetworkingRoute,
        dom_manipulation: RendererPageDomManipulationRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            networking,
            dom_manipulation,
            root_document,
        }
    }

    pub(crate) fn bind_producer(
        &self,
        document_owner: FrameDocumentTaskOwner,
    ) -> RendererPageStylesheetTaskProducer {
        RendererPageStylesheetTaskProducer {
            sender: self.clone(),
            owner: RendererPageStylesheetTaskOwner::new(self.root_document, document_owner),
        }
    }
}

/// Producer bound to one exact main Document.
///
/// Async fetches clone this value at start. Rebinding the current Document
/// therefore cannot retarget an already-running fetch to its replacement.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageStylesheetTaskProducer {
    sender: RendererPageStylesheetTaskSender,
    owner: RendererPageStylesheetTaskOwner,
}

impl RendererPageStylesheetTaskProducer {
    pub(crate) fn send_blocking_completion(
        &self,
        completion: StylesheetCompletion,
    ) -> Result<(), RendererPageStylesheetTaskRouteClosed> {
        self.send_networking_completion(RendererPageStylesheetCompletion::Blocking(completion))
    }

    pub(crate) fn send_connected_completion(
        &self,
        completion: ConnectedLoadCompletion,
    ) -> Result<(), RendererPageStylesheetTaskRouteClosed> {
        self.send_networking_completion(RendererPageStylesheetCompletion::Connected(completion))
    }

    pub(crate) fn send_live_import_completion(
        &self,
        completion: LiveStylesheetImportLoadCompletion,
    ) -> Result<(), RendererPageStylesheetTaskRouteClosed> {
        self.send_networking_completion(RendererPageStylesheetCompletion::LiveImport(completion))
    }

    fn send_networking_completion(
        &self,
        completion: RendererPageStylesheetCompletion,
    ) -> Result<(), RendererPageStylesheetTaskRouteClosed> {
        self.sender
            .networking
            .send(RendererPageNetworkingTask::StylesheetCompletion(
                RendererPageStylesheetNetworkingTask {
                    owner: self.owner,
                    completion,
                },
            ))
            .map_err(|_| RendererPageStylesheetTaskRouteClosed)
    }

    pub(crate) fn send_connected_style_event(
        &self,
        ready: ReadyConnectedStyleLoad,
    ) -> Result<(), RendererPageStylesheetTaskRouteClosed> {
        let kind = ready.element_kind();
        let task = RendererPageConnectedStyleEventTask {
            owner: self.owner,
            ready,
        };
        match kind {
            ConnectedStyleEventElementKind::Style => self
                .sender
                .networking
                .send(RendererPageNetworkingTask::StyleElementEvent(task))
                .map_err(|_| RendererPageStylesheetTaskRouteClosed),
            ConnectedStyleEventElementKind::Link => self
                .sender
                .dom_manipulation
                .send(RendererPageDomManipulationTask::ConnectedStyleEvent(task))
                .map_err(|_| RendererPageStylesheetTaskRouteClosed),
        }
    }
}

/// Narrow production-route residence for standalone `DocumentRuntime` tests.
///
/// The fixture retains the same Networking and DOM-manipulation source types
/// used by a Page owner. It deliberately exposes no local stylesheet
/// completion queue: test fetches publish a typed task, and tests must claim
/// that task before observing the resulting stylesheet state.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct RendererPageStylesheetTaskTestResidence {
    runtime_wake: PageRuntimeWakeSignal,
    networking: RendererPageNetworkingSource,
    dom_manipulation: RendererPageDomManipulationSource,
    _owner_wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
    sender: RendererPageStylesheetTaskSender,
}

#[cfg(test)]
impl RendererPageStylesheetTaskTestResidence {
    pub(crate) fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_PAGE_ID: AtomicU64 = AtomicU64::new(1);

        let page_id = NEXT_PAGE_ID.fetch_add(1, Ordering::Relaxed);
        let root_document =
            RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(page_id), 1);
        let page_token = crate::runtime::RendererPageToken::new_for_testing(root_document.page_id);
        let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(owner_wake_tx, page_token);
        let runtime_wake = PageRuntimeWakeSignal::default();
        let networking = RendererPageNetworkingSource::new_owner_attached(
            runtime_wake.clone(),
            owner_wake.clone(),
        );
        let dom_manipulation = RendererPageDomManipulationSource::new(owner_wake);
        let sender = RendererPageStylesheetTaskSender::new(
            networking.route(),
            dom_manipulation.route(),
            root_document,
        );
        Self {
            runtime_wake,
            networking,
            dom_manipulation,
            _owner_wake_rx: owner_wake_rx,
            sender,
        }
    }

    pub(crate) fn sender(&self) -> RendererPageStylesheetTaskSender {
        self.sender.clone()
    }

    pub(crate) fn main_parser_continuation_sender(
        &self,
    ) -> super::RendererPageMainParserContinuationSender {
        super::RendererPageMainParserContinuationSender::new(
            self.networking.route(),
            self.sender.root_document,
        )
    }

    pub(crate) fn pop_networking_task(&mut self) -> Option<RendererPageStylesheetNetworkingTask> {
        if !matches!(
            self.networking.next_ready_task_owner(),
            Some(super::RendererPageNetworkingOwner::StylesheetCompletion(_))
        ) {
            return None;
        }
        let (_, task) = self.networking.pop_front_task()?;
        let RendererPageNetworkingTask::StylesheetCompletion(task) = task else {
            unreachable!("stylesheet-only test route accepted a non-stylesheet Networking task")
        };
        Some(task)
    }

    pub(crate) async fn wait_for_networking_task(&mut self) -> bool {
        loop {
            if self.networking.has_ready_task() {
                return true;
            }
            self.runtime_wake.wait().await;
        }
    }

    pub(crate) fn pop_connected_style_event(
        &mut self,
    ) -> Option<RendererPageConnectedStyleEventTask> {
        if matches!(
            self.networking.next_ready_task_owner(),
            Some(super::RendererPageNetworkingOwner::StyleElementEvent(_))
        ) {
            let (_, task) = self.networking.pop_front_task()?;
            let RendererPageNetworkingTask::StyleElementEvent(task) = task else {
                unreachable!("style-event owner must identify its Networking task")
            };
            return Some(task);
        }
        let (_, task) = self.dom_manipulation.pop_front()?;
        let RendererPageDomManipulationTask::ConnectedStyleEvent(task) = task else {
            unreachable!("stylesheet-only test route accepted a non-style DOM task")
        };
        Some(task)
    }

    pub(crate) fn has_connected_style_event(&mut self) -> bool {
        matches!(
            self.networking.next_ready_task_owner(),
            Some(super::RendererPageNetworkingOwner::StyleElementEvent(_))
        ) || self.dom_manipulation.has_ready_task()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageStylesheetNetworkingTargetEffect {
    /// The exact main Document accepted the terminal and
    /// may have published a later parser continuation or element event.
    AppliedToCurrentOwner,
    /// The retired task retained its historical network accounting without
    /// acquiring task-end authority over the replacement Document's agent.
    RecordedForStaleOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageStylesheetNetworkingTurnAction {
    pub(crate) owner: RendererPageStylesheetTaskOwner,
    pub(crate) target_effect: PageStylesheetNetworkingTargetEffect,
}

pub(crate) type PageStylesheetNetworkingTurnOutcome =
    PageOwnerTurnOutcome<PageStylesheetNetworkingTurnAction>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageConnectedStyleLoadDelayEffect {
    /// Admission acquired no load-delay token, either because the Document
    /// had already reached the complete transition or because a low-level
    /// unowned fixture constructed the event without one.
    NoBindingRequired,
    /// The event task released the exact load-delay token captured when the
    /// connected style/link operation was admitted.
    ReleasedExactBinding,
    /// Event dispatch synchronously replaced the binding's Document.
    ///
    /// Document replacement owns retirement of the old ledger. The selected
    /// event task therefore leaves both the retired binding and the new
    /// Document's load-delay ledger untouched.
    ExactBindingRetiredWithDocument,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageConnectedStyleEventTargetEffect {
    DispatchedToCurrentOwner {
        load_delay_effect: PageConnectedStyleLoadDelayEffect,
    },
    CurrentOwnerHadNoEvent {
        load_delay_effect: PageConnectedStyleLoadDelayEffect,
    },
    DiscardedStaleOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageConnectedStyleEventTurnAction {
    pub(crate) owner: RendererPageStylesheetTaskOwner,
    pub(crate) target_effect: PageConnectedStyleEventTargetEffect,
}

impl PageConnectedStyleEventTurnAction {
    pub(crate) const fn settled_current_owner(self) -> bool {
        matches!(
            self.target_effect,
            PageConnectedStyleEventTargetEffect::DispatchedToCurrentOwner { .. }
                | PageConnectedStyleEventTargetEffect::CurrentOwnerHadNoEvent { .. }
        )
    }
}

pub(crate) type PageConnectedStyleEventTurnOutcome =
    PageOwnerTurnOutcome<PageConnectedStyleEventTurnAction>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document_runtime::ConnectedStyleEventElementKind,
        frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId},
    };

    fn document_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
    }

    #[test]
    fn captured_element_kind_selects_the_normative_event_task_source() {
        let mut residence = RendererPageStylesheetTaskTestResidence::new();
        let producer = residence.sender().bind_producer(document_owner());

        producer
            .send_connected_style_event(ReadyConnectedStyleLoad::for_owner(
                moli_dom::native::NativeNodeId::new(7),
                true,
                ConnectedStyleEventElementKind::Style,
            ))
            .expect("style event route");
        assert!(matches!(
            residence.networking.next_ready_task_owner(),
            Some(super::super::RendererPageNetworkingOwner::StyleElementEvent(_))
        ));
        assert!(!residence.dom_manipulation.has_ready_task());
        let _ = residence
            .pop_connected_style_event()
            .expect("style event must be resident in Networking");

        producer
            .send_connected_style_event(ReadyConnectedStyleLoad::for_owner(
                moli_dom::native::NativeNodeId::new(8),
                true,
                ConnectedStyleEventElementKind::Link,
            ))
            .expect("link event route");
        assert!(!residence.networking.has_ready_task());
        assert!(residence.dom_manipulation.has_ready_task());
        let _ = residence
            .pop_connected_style_event()
            .expect("link event must be resident in DOM-manipulation");
    }
}
