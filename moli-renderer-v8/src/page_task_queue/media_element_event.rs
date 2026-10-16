use super::{RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal};
use crate::{
    native_bridge::WindowTaskTarget,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};
use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource};

/// Host-local key for one payload in the HTML media-element event task source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageMediaElementEventTaskId(u64);

impl RendererPageMediaElementEventTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Family-local operation carried by the media-element event source.
///
/// Text-track fetch start and terminal work deliberately do not appear here:
/// their HTML task-source contracts are stable-state/networking or DOM
/// manipulation, not media-element event delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageMediaElementEventTaskKind {
    Playback,
    Seeking,
    VolumeChange,
    SeekCompletion,
    LoadEventPhase,
    TextTrackListEvent,
    RemotePlaybackAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageMediaElementEventOwner {
    root_document: RendererDocumentToken,
    target: WindowTaskTarget,
}

impl RendererPageMediaElementEventOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: WindowTaskTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> WindowTaskTarget {
        self.target
    }
}

/// Scheduler envelope for a concrete captured LocalWindow task whose mutable
/// V8/DOM payload remains in the creating `JsContextHost` ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageMediaElementEventTask {
    owner: RendererPageMediaElementEventOwner,
    task_id: RendererPageMediaElementEventTaskId,
    kind: RendererPageMediaElementEventTaskKind,
}

impl RendererPageMediaElementEventTask {
    pub(crate) const fn new(
        owner: RendererPageMediaElementEventOwner,
        task_id: RendererPageMediaElementEventTaskId,
        kind: RendererPageMediaElementEventTaskKind,
    ) -> Self {
        Self {
            owner,
            task_id,
            kind,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageMediaElementEventOwner {
        self.owner
    }

    pub(crate) const fn task_id(&self) -> RendererPageMediaElementEventTaskId {
        self.task_id
    }

    pub(crate) const fn kind(&self) -> RendererPageMediaElementEventTaskKind {
        self.kind
    }
}

/// Common settlement of one captured LocalWindow event task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageMediaElementEventTargetEffect {
    DispatchedToCurrentOwner,
    CurrentOwnerHadNoEventTarget,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageMediaElementEventOwner>,
    },
}

/// Common owner-turn fact emitted after a captured LocalWindow task settles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageMediaElementEventTurnAction {
    pub(crate) owner: RendererPageMediaElementEventOwner,
    pub(crate) task_id: RendererPageMediaElementEventTaskId,
    pub(crate) kind: RendererPageMediaElementEventTaskKind,
    pub(crate) target_effect: PageMediaElementEventTargetEffect,
}

/// Media task route carrying a captured LocalWindow owner, a Host-local payload id, and a family-local
/// operation kind.
///
/// Mutable DOM/V8 payloads remain in the creating `JsContextHost`; this route
/// carries the immutable scheduler envelope and publishes one empty-to-nonempty
/// readiness edge.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageMediaElementEventRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageMediaElementEventTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageMediaElementEventRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageMediaElementEventSender {
        RendererPageMediaElementEventSender::new(self.clone(), root_document)
    }

    fn send(
        &self,
        task: RendererPageMediaElementEventTask,
    ) -> Result<(), RendererPageMediaElementEventRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
            .map_err(|_| RendererPageMediaElementEventRouteClosed)
    }

    fn same_source_as(&self, source: &RendererPageMediaElementEventSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageMediaElementEventRouteClosed;

/// PageVm-stamped producer capability for a LocalWindow task-source
/// family. A producer cannot choose another root Page namespace.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageMediaElementEventSender {
    route: RendererPageMediaElementEventRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageMediaElementEventSender {
    pub(crate) fn new(
        route: RendererPageMediaElementEventRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    pub(crate) fn send(
        &self,
        target: WindowTaskTarget,
        task_id: RendererPageMediaElementEventTaskId,
        kind: RendererPageMediaElementEventTaskKind,
    ) -> Result<(), RendererPageMediaElementEventRouteClosed> {
        self.route.send(RendererPageMediaElementEventTask::new(
            RendererPageMediaElementEventOwner::new(self.root_document, target),
            task_id,
            kind,
        ))
    }
}

/// Unique Page-lifetime consumer for one captured LocalWindow task-source
/// family.
#[derive(Debug)]
pub(crate) struct RendererPageMediaElementEventSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageMediaElementEventTask>,
        RendererPageTaskReadySignal,
    >,
}

impl RendererPageMediaElementEventSource {
    pub(crate) fn new(
        owner_wake: RendererOwnerWakeSender,
        source: RendererOwnerWakeSource,
    ) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageTaskReadySignal::new(owner_wake, source)),
        }
    }

    pub(crate) fn route(&self) -> RendererPageMediaElementEventRoute {
        RendererPageMediaElementEventRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageMediaElementEventOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageMediaElementEventTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageMediaElementEventRoute) -> bool {
        route.same_source_as(self)
    }
}
pub(crate) type PageMediaElementEventTurnOutcome =
    PageOwnerTurnOutcome<PageMediaElementEventTurnAction>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PageId,
        frame_owner_model::LocalWindowId,
        native_bridge::{OwnerDispatchScope, WindowExecutionContextOwner, WindowTaskTarget},
        page_task_queue::{RendererOwnerWake, RendererOwnerWakeSender, RendererOwnerWakeSource},
        runtime::{RendererDocumentToken, RendererPageToken},
    };

    fn root_document() -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(PageId::new_for_testing(81), 4)
    }

    fn target(local_window_id: u64) -> WindowTaskTarget {
        WindowTaskTarget::new(
            OwnerDispatchScope::Top,
            WindowExecutionContextOwner::Frame(LocalWindowId(local_window_id)),
        )
    }

    #[test]
    fn media_event_source_is_cross_kind_fifo_with_one_ready_edge() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(root_document().page_id),
        );
        let mut source = RendererPageMediaElementEventSource::new(
            owner_wake,
            RendererOwnerWakeSource::MediaElementEventTask,
        );
        let sender = source.route().sender(root_document());

        for (raw, kind) in [
            (1, RendererPageMediaElementEventTaskKind::Seeking),
            (2, RendererPageMediaElementEventTaskKind::LoadEventPhase),
            (3, RendererPageMediaElementEventTaskKind::TextTrackListEvent),
        ] {
            sender
                .send(
                    target(raw),
                    RendererPageMediaElementEventTaskId::from_raw(raw),
                    kind,
                )
                .expect("media event task should enqueue");
        }

        assert!(matches!(
            wake_rx.try_recv(),
            Ok(RendererOwnerWake::Page {
                source: RendererOwnerWakeSource::MediaElementEventTask,
                ..
            })
        ));
        assert!(wake_rx.try_recv().is_err());
        for expected in 1..=3 {
            let (_, task) = source.pop_front().expect("media task should remain queued");
            assert_eq!(
                task.task_id(),
                RendererPageMediaElementEventTaskId::from_raw(expected)
            );
        }
    }

    #[test]
    fn retired_media_event_source_rejects_without_timer_fallback() {
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageMediaElementEventSource::new(
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(root_document().page_id),
            ),
            RendererOwnerWakeSource::MediaElementEventTask,
        );
        let sender = source.route().sender(root_document());
        drop(source);

        assert!(
            sender
                .send(
                    target(9),
                    RendererPageMediaElementEventTaskId::from_raw(9),
                    RendererPageMediaElementEventTaskKind::SeekCompletion,
                )
                .is_err()
        );
    }
}
