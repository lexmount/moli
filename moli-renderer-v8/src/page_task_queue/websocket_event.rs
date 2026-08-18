use moli_websocket::{Event as WebSocketEvent, EventSender as WebSocketEventSender};

use crate::{
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::RendererDocumentToken,
};

use super::{PageRuntimeWakeSignal, RendererOwnerWakeSender};
use crate::runtime::PageOwnerTurnOutcome;

const WEBSOCKET_EVENT_QUEUE_CAPACITY: usize = 1;

/// Exact PageVm generation and socket captured when a WebSocket event enters
/// the Page's networking task source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWebSocketOwner {
    root_document: RendererDocumentToken,
    socket_id: u64,
}

impl RendererPageWebSocketOwner {
    const fn new(root_document: RendererDocumentToken, socket_id: u64) -> Self {
        Self {
            root_document,
            socket_id,
        }
    }

    #[cfg(test)]
    pub(crate) const fn new_for_test(root_document: RendererDocumentToken, socket_id: u64) -> Self {
        Self::new(root_document, socket_id)
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn socket_id(self) -> u64 {
        self.socket_id
    }
}

/// Whether the WebSocket source head can currently be dispatched.
///
/// A backpressured event remains scheduler-visible so an old-Document event
/// can still be selected and retired. The PageVm eligibility boundary hides
/// only a backpressured event belonging to the current Document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageWebSocketReadiness {
    Ready,
    Backpressured,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageWebSocketHead {
    pub(crate) ready: RendererPageTaskReadyMetadata,
    pub(crate) owner: RendererPageWebSocketOwner,
    pub(crate) readiness: RendererPageWebSocketReadiness,
}

#[derive(Debug)]
struct RendererPageWebSocketIngressTask {
    owner: RendererPageWebSocketOwner,
    event: WebSocketEvent,
}

#[derive(Clone, Debug)]
struct RendererPageWebSocketReturnRoute {
    tx: tokio::sync::mpsc::UnboundedSender<RendererPageWebSocketIngressTask>,
}

/// One selected WebSocket event.
///
/// The task owns the only payload copy. If V8 reports readable-stream
/// backpressure, `return_backpressured` moves that same payload back into the
/// stable source before the selected Page turn ends.
#[derive(Debug)]
pub(crate) struct RendererPageWebSocketTask {
    task: RendererPageWebSocketIngressTask,
    return_route: RendererPageWebSocketReturnRoute,
}

/// Execution-produced result of one exact current-target WebSocket body.
///
/// This value is never scheduler metadata and cannot be queued. Both the
/// production Page arbiter and low-level ScriptVm fixtures convert it through
/// the same target/action protocol before submitting task completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageWebSocketBodyEffect {
    /// An EventTarget event was dispatched or WebSocketStream Promise/stream
    /// state was settled.
    CallbackVisibleWorkApplied,
    /// The exact target consumed an internal WebSocket transition without
    /// dispatching or settling callback-visible work.
    InternalStateApplied,
    /// A WebSocketStream readable queue cannot currently accept this message.
    /// The selected payload has not settled and must return to the source.
    ReadableBackpressured,
    /// The socket or its exact Window realm disappeared before application.
    CurrentTargetDisappeared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageWebSocketTargetEffect {
    /// The exact current target dispatched an EventTarget event or settled
    /// WebSocketStream callback-visible state.
    CallbackVisibleWorkAppliedToCurrentDocument,
    /// The exact current target consumed an internal protocol/state
    /// transition without producing callback-visible work.
    InternalStateAppliedToCurrentDocument,
    /// The root Document remained current, but the socket or its exact Window
    /// realm disappeared before the selected body could apply.
    CurrentDocumentTargetDisappeared,
    ParkedForReadableBackpressure,
    DiscardedStaleDocument {
        current_document: RendererDocumentToken,
    },
}

impl PageWebSocketTargetEffect {
    pub(crate) const fn from_current_body(body_effect: PageWebSocketBodyEffect) -> Self {
        match body_effect {
            PageWebSocketBodyEffect::CallbackVisibleWorkApplied => {
                Self::CallbackVisibleWorkAppliedToCurrentDocument
            }
            PageWebSocketBodyEffect::InternalStateApplied => {
                Self::InternalStateAppliedToCurrentDocument
            }
            PageWebSocketBodyEffect::ReadableBackpressured => Self::ParkedForReadableBackpressure,
            PageWebSocketBodyEffect::CurrentTargetDisappeared => {
                Self::CurrentDocumentTargetDisappeared
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageWebSocketTurnAction {
    pub(crate) owner: RendererPageWebSocketOwner,
    pub(crate) target_effect: PageWebSocketTargetEffect,
}

impl PageWebSocketTurnAction {
    pub(crate) const fn outcome(self) -> PageWebSocketTurnOutcome {
        PageOwnerTurnOutcome::new(self)
    }
}

pub(crate) type PageWebSocketTurnOutcome = PageOwnerTurnOutcome<PageWebSocketTurnAction>;

impl RendererPageWebSocketTask {
    pub(crate) const fn owner(&self) -> RendererPageWebSocketOwner {
        self.task.owner
    }

    pub(crate) fn event(&self) -> &WebSocketEvent {
        &self.task.event
    }

    pub(crate) fn return_backpressured(self) {
        let Self { task, return_route } = self;
        assert!(
            return_route.tx.send(task).is_ok(),
            "selected WebSocket task must return to its live Page source"
        );
    }
}

#[derive(Clone, Debug)]
struct RendererPageWebSocketReadySignal {
    runtime_wake: PageRuntimeWakeSignal,
    owner_wake: RendererOwnerWakeSender,
}

impl RendererPageWebSocketReadySignal {
    fn signal(&self) {
        self.runtime_wake.send();
        self.owner_wake.signal_networking_task();
    }
}

/// Page-lifetime producer route for the bounded WebSocket ingress and
/// WebSocketStream pull notifications.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebSocketRoute {
    event_tx: tokio::sync::mpsc::Sender<ReadyPageTask<RendererPageWebSocketIngressTask>>,
    pull_tx: tokio::sync::mpsc::UnboundedSender<ReadyPageTask<u64>>,
    ready_signal: RendererPageWebSocketReadySignal,
}

impl RendererPageWebSocketRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageWebSocketSender {
        RendererPageWebSocketSender {
            route: self.clone(),
            root_document,
        }
    }

    pub(crate) fn same_source_as(&self, source: &RendererPageWebSocketSource) -> bool {
        self.event_tx.same_channel(&source.event_tx) && self.pull_tx.same_channel(&source.pull_tx)
    }
}

/// Exact-Document producer capability installed in one PageVm.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageWebSocketSender {
    route: RendererPageWebSocketRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageWebSocketSender {
    pub(crate) fn event_sender(&self) -> WebSocketEventSender {
        let route = self.route.clone();
        let root_document = self.root_document;
        WebSocketEventSender::with_async_sink(move |event| {
            let route = route.clone();
            async move {
                let Ok(permit) = route.event_tx.reserve().await else {
                    return false;
                };
                let owner = RendererPageWebSocketOwner::new(root_document, event.socket_id());
                permit.send(ReadyPageTask::new(RendererPageWebSocketIngressTask {
                    owner,
                    event,
                }));
                route.ready_signal.signal();
                true
            }
        })
    }

    /// Records that WebSocketStream consumer demand may have made the exact
    /// blocked event runnable again. The source validates the socket id before
    /// changing its residence.
    pub(crate) fn signal_readable_pull(&self, socket_id: u64) {
        if self
            .route
            .pull_tx
            .send(ReadyPageTask::new(socket_id))
            .is_ok()
        {
            self.route.ready_signal.signal();
        }
    }
}

/// Unique Page-lifetime consumer for WebSocket events.
///
/// The network ingress remains bounded independently from the Page scheduler.
/// A WebSocketStream message that cannot yet enter its readable queue moves to
/// `blocked`; it does not occupy the runnable Networking head and therefore
/// cannot hide unrelated networking tasks. Pull demand moves only the matching
/// event back to `ready` and publishes a normal Networking wake.
#[derive(Debug)]
pub(crate) struct RendererPageWebSocketSource {
    event_tx: tokio::sync::mpsc::Sender<ReadyPageTask<RendererPageWebSocketIngressTask>>,
    event_rx: tokio::sync::mpsc::Receiver<ReadyPageTask<RendererPageWebSocketIngressTask>>,
    pull_tx: tokio::sync::mpsc::UnboundedSender<ReadyPageTask<u64>>,
    pull_rx: tokio::sync::mpsc::UnboundedReceiver<ReadyPageTask<u64>>,
    returned_tx: tokio::sync::mpsc::UnboundedSender<RendererPageWebSocketIngressTask>,
    returned_rx: tokio::sync::mpsc::UnboundedReceiver<RendererPageWebSocketIngressTask>,
    ready_signal: RendererPageWebSocketReadySignal,
    ready: Option<ReadyPageTask<RendererPageWebSocketIngressTask>>,
    blocked: Option<ReadyPageTask<RendererPageWebSocketIngressTask>>,
}

impl RendererPageWebSocketSource {
    pub(crate) fn new(
        runtime_wake: PageRuntimeWakeSignal,
        owner_wake: RendererOwnerWakeSender,
    ) -> Self {
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(WEBSOCKET_EVENT_QUEUE_CAPACITY);
        let (pull_tx, pull_rx) = tokio::sync::mpsc::unbounded_channel();
        let (returned_tx, returned_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            event_tx,
            event_rx,
            pull_tx,
            pull_rx,
            returned_tx,
            returned_rx,
            ready_signal: RendererPageWebSocketReadySignal {
                runtime_wake,
                owner_wake,
            },
            ready: None,
            blocked: None,
        }
    }

    pub(crate) fn route(&self) -> RendererPageWebSocketRoute {
        RendererPageWebSocketRoute {
            event_tx: self.event_tx.clone(),
            pull_tx: self.pull_tx.clone(),
            ready_signal: self.ready_signal.clone(),
        }
    }

    pub(crate) fn next_head(&mut self) -> Option<RendererPageWebSocketHead> {
        self.refresh_residence();
        if let Some(ready) = self.ready.as_ref() {
            return Some(RendererPageWebSocketHead {
                ready: ready.metadata(),
                owner: ready.value().owner,
                readiness: RendererPageWebSocketReadiness::Ready,
            });
        }
        self.blocked
            .as_ref()
            .map(|blocked| RendererPageWebSocketHead {
                ready: blocked.metadata(),
                owner: blocked.value().owner,
                readiness: RendererPageWebSocketReadiness::Backpressured,
            })
    }

    pub(crate) fn pop_head(
        &mut self,
        expected: RendererPageWebSocketHead,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageWebSocketTask)> {
        self.refresh_residence();
        let ready = match expected.readiness {
            RendererPageWebSocketReadiness::Ready => self.ready.take(),
            RendererPageWebSocketReadiness::Backpressured => self.blocked.take(),
        }?;
        let (actual, task) = ready.into_parts();
        assert_eq!(actual, expected.ready, "selected WebSocket head changed");
        assert_eq!(
            task.owner, expected.owner,
            "selected WebSocket owner changed"
        );
        Some((
            actual,
            RendererPageWebSocketTask {
                task,
                return_route: RendererPageWebSocketReturnRoute {
                    tx: self.returned_tx.clone(),
                },
            },
        ))
    }

    pub(crate) fn has_resident_task(&mut self) -> bool {
        self.next_head().is_some()
    }

    pub(crate) fn has_runnable_task_for(
        &mut self,
        current_document: RendererDocumentToken,
    ) -> bool {
        self.next_head().is_some_and(|head| {
            matches!(head.readiness, RendererPageWebSocketReadiness::Ready)
                || head.owner.root_document() != current_document
        })
    }

    pub(crate) fn clear(&mut self) {
        self.ready = None;
        self.blocked = None;
        while self.event_rx.try_recv().is_ok() {}
        while self.returned_rx.try_recv().is_ok() {}
        while self.pull_rx.try_recv().is_ok() {}
    }

    fn refresh_residence(&mut self) {
        if let Ok(returned) = self.returned_rx.try_recv() {
            assert!(
                self.ready.is_none() && self.blocked.is_none(),
                "a returned WebSocket event must be the source's only selected head"
            );
            self.blocked = Some(ReadyPageTask::new(returned));
        }
        assert!(
            self.returned_rx.try_recv().is_err(),
            "only one WebSocket event may be selected per Page turn"
        );

        let mut matching_pull = None;
        while let Ok(pull) = self.pull_rx.try_recv() {
            if matching_pull.is_none()
                && self
                    .blocked
                    .as_ref()
                    .is_some_and(|blocked| blocked.value().owner.socket_id() == *pull.value())
            {
                matching_pull = Some(pull.metadata());
            }
        }
        if let Some(pull_ready) = matching_pull {
            let blocked = self
                .blocked
                .take()
                .expect("matching WebSocket pull requires a blocked event");
            self.ready = Some(ReadyPageTask {
                ready_at: pull_ready.ready_at,
                order: pull_ready.order,
                value: blocked.value,
            });
        }

        if self.ready.is_none()
            && self.blocked.is_none()
            && let Ok(ready) = self.event_rx.try_recv()
        {
            self.ready = Some(ready);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PageId, page_task_queue::RendererOwnerWakeSource, runtime::RendererPageToken};

    fn root_document(lifecycle_document_id: u64) -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(PageId::new_for_testing(1), lifecycle_document_id)
    }

    fn close_event(socket_id: u64) -> WebSocketEvent {
        WebSocketEvent::Close {
            socket_id,
            code: 1005,
            reason: String::new(),
            was_clean: true,
        }
    }

    fn source() -> (
        RendererPageWebSocketSource,
        tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
    ) {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
        );
        (
            RendererPageWebSocketSource::new(PageRuntimeWakeSignal::default(), owner_wake),
            wake_rx,
        )
    }

    #[tokio::test]
    async fn bounded_ingress_stamps_exact_document_and_networking_wake() {
        let (mut source, mut wake_rx) = source();
        let sender = source.route().sender(root_document(7));

        assert!(sender.event_sender().send(close_event(11)).await);
        assert_eq!(
            wake_rx
                .recv()
                .await
                .expect("accepted WebSocket event should wake the Page")
                .source_for_test(),
            RendererOwnerWakeSource::NetworkingTask
        );

        let head = source.next_head().expect("accepted event should be ready");
        assert_eq!(head.owner.root_document(), root_document(7));
        assert_eq!(head.owner.socket_id(), 11);
        assert_eq!(head.readiness, RendererPageWebSocketReadiness::Ready);
        let (_, task) = source.pop_head(head).expect("ready event should dequeue");
        assert_eq!(task.owner(), head.owner);
        assert_eq!(task.event().socket_id(), 11);
        assert!(source.next_head().is_none());
    }

    #[tokio::test]
    async fn readable_pull_republishes_only_the_matching_blocked_event() {
        let (mut source, _wake_rx) = source();
        let sender = source.route().sender(root_document(3));
        assert!(sender.event_sender().send(close_event(19)).await);

        let head = source.next_head().expect("event should be ready");
        let (_, task) = source.pop_head(head).expect("event should dequeue");
        task.return_backpressured();

        let blocked = source.next_head().expect("returned event should persist");
        assert_eq!(
            blocked.readiness,
            RendererPageWebSocketReadiness::Backpressured
        );
        assert!(
            !source.has_runnable_task_for(root_document(3)),
            "current-Document backpressure must remove the WebSocket head from arbitration"
        );
        assert!(
            source.has_runnable_task_for(root_document(4)),
            "replacement must be able to select and retire a stale blocked WebSocket head"
        );

        sender.signal_readable_pull(20);
        assert_eq!(
            source
                .next_head()
                .expect("unrelated pull must preserve blocked residence")
                .readiness,
            RendererPageWebSocketReadiness::Backpressured
        );

        sender.signal_readable_pull(19);
        let ready = source
            .next_head()
            .expect("matching pull should republish blocked event");
        assert_eq!(ready.readiness, RendererPageWebSocketReadiness::Ready);
        assert_ne!(ready.ready, blocked.ready);
        let (_, task) = source
            .pop_head(ready)
            .expect("republished event should dequeue");
        assert_eq!(task.event().socket_id(), 19);
    }

    #[tokio::test]
    async fn blocked_event_retains_fifo_residence_ahead_of_later_socket_input() {
        let (mut source, _wake_rx) = source();
        let sender = source.route().sender(root_document(5));
        let event_sender = sender.event_sender();
        assert!(event_sender.send(close_event(31)).await);
        let head = source.next_head().expect("first event should be ready");
        let (_, task) = source.pop_head(head).expect("first event should dequeue");
        task.return_backpressured();

        assert!(event_sender.send(close_event(32)).await);
        let blocked = source
            .next_head()
            .expect("blocked event should stay at head");
        assert_eq!(blocked.owner.socket_id(), 31);
        assert_eq!(
            blocked.readiness,
            RendererPageWebSocketReadiness::Backpressured
        );

        sender.signal_readable_pull(31);
        let first = source
            .next_head()
            .expect("pull should make first event ready");
        let _ = source.pop_head(first).expect("first event should dequeue");
        let second = source
            .next_head()
            .expect("later input should follow first event");
        assert_eq!(second.owner.socket_id(), 32);
    }
}
