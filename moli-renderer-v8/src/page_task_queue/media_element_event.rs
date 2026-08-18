use crate::runtime::PageOwnerTurnOutcome;

use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    window_document_task_source::{
        RendererPageWindowDocumentTaskRoute, RendererPageWindowDocumentTaskSender,
        RendererPageWindowDocumentTaskSource,
    },
};

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
    Seeking,
    SeekCompletion,
    LoadEventPhase,
    TextTrackListEvent,
}

pub(crate) type RendererPageMediaElementEventOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageMediaElementEventTask = RendererPageWindowDocumentTask<
    RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
>;
pub(super) type RendererPageMediaElementEventRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
>;
pub(crate) type RendererPageMediaElementEventSender = RendererPageWindowDocumentTaskSender<
    RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
>;
pub(super) type RendererPageMediaElementEventSource = RendererPageWindowDocumentTaskSource<
    RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
>;

pub(crate) type PageMediaElementEventTargetEffect = PageWindowDocumentTaskTargetEffect;
pub(crate) type PageMediaElementEventTurnAction = PageWindowDocumentTaskTurnAction<
    RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
>;
pub(crate) type PageMediaElementEventTurnOutcome =
    PageOwnerTurnOutcome<PageMediaElementEventTurnAction>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PageId,
        frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        },
        native_bridge::{OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget},
        page_task_queue::{RendererOwnerWake, RendererOwnerWakeSender, RendererOwnerWakeSource},
        runtime::{RendererDocumentToken, RendererPageToken},
    };

    fn root_document() -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(PageId::new_for_testing(81), 4)
    }

    fn target(document_id: u64) -> WindowDocumentTaskTarget {
        WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(12),
                LocalWindowId(13),
                DocumentId(document_id),
            )),
            OwnerDispatchScope::Top,
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
            RendererOwnerWakeSender::signal_media_element_event_task,
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
            RendererOwnerWakeSender::signal_media_element_event_task,
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
