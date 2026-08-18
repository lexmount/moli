use crate::runtime::PageOwnerTurnOutcome;

use super::{
    PageWindowDocumentTaskTargetEffect, PageWindowDocumentTaskTurnAction,
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    window_document_task_source::{
        RendererPageWindowDocumentTaskRoute, RendererPageWindowDocumentTaskSender,
        RendererPageWindowDocumentTaskSource,
    },
};
#[cfg(test)]
use super::{RendererOwnerWakeSender, WindowDocumentTaskTarget};

/// PageVm-local key for one pending Document rendering-update payload.
///
/// V8 objects and mutable coalescing state remain in the `JsContextHost` that
/// accepted the update. The stable Page source carries only this key and the
/// immutable Document target, so a replacement PageVm cannot consume a
/// naturally reused local id.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageRenderingUpdateTaskId(u64);

impl RendererPageRenderingUpdateTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Concrete operation in the HTML rendering task source.
///
/// Keep task kind separate from source class: later resize, animation-frame,
/// or observer work may share this source without acquiring another fairness
/// slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageRenderingUpdateTaskKind {
    DocumentScrollEvents,
    AnimationStartScan,
    PostParseAutofocus,
}

pub(crate) type RendererPageRenderingUpdateOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageRenderingUpdateTask = RendererPageWindowDocumentTask<
    RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
>;
pub(crate) type RendererPageRenderingUpdateHead = RendererPageRenderingUpdateTask;
pub(super) type RendererPageRenderingUpdateRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
>;
pub(crate) type RendererPageRenderingUpdateSender = RendererPageWindowDocumentTaskSender<
    RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
>;
pub(super) type RendererPageRenderingUpdateSource = RendererPageWindowDocumentTaskSource<
    RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
>;

pub(crate) type PageRenderingUpdateTargetEffect = PageWindowDocumentTaskTargetEffect;
pub(crate) type PageRenderingUpdateTurnAction = PageWindowDocumentTaskTurnAction<
    RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
>;

pub(crate) type PageRenderingUpdateTurnOutcome =
    PageOwnerTurnOutcome<PageRenderingUpdateTurnAction>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PageId,
        frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        },
        native_bridge::{OwnerDispatchScope, WindowDocumentOwner},
        runtime::{RendererDocumentToken, RendererPageToken},
    };

    fn root_document() -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(PageId::new_for_testing(71), 3)
    }

    fn target(document_id: u64) -> WindowDocumentTaskTarget {
        WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(9),
                LocalWindowId(10),
                DocumentId(document_id),
            )),
            OwnerDispatchScope::Top,
        )
    }

    #[test]
    fn rendering_source_is_fifo_and_coalesces_its_ready_edge() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(PageId::new_for_testing(71)),
        );
        let mut source = RendererPageRenderingUpdateSource::new(
            owner_wake,
            RendererOwnerWakeSender::signal_rendering_update_task,
        );
        let sender = RendererPageRenderingUpdateSender::new(source.route(), root_document());

        sender
            .send(
                target(11),
                RendererPageRenderingUpdateTaskId::from_raw(1),
                RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
            )
            .expect("first rendering task should enqueue");
        sender
            .send(
                target(12),
                RendererPageRenderingUpdateTaskId::from_raw(2),
                RendererPageRenderingUpdateTaskKind::AnimationStartScan,
            )
            .expect("cross-kind rendering task should enqueue second");

        assert!(matches!(
            wake_rx.try_recv(),
            Ok(crate::page_task_queue::RendererOwnerWake::Page {
                source: crate::page_task_queue::RendererOwnerWakeSource::RenderingUpdateTask,
                ..
            })
        ));
        assert!(
            wake_rx.try_recv().is_err(),
            "a nonempty source must not publish a duplicate readiness edge"
        );

        let (_, first) = source.pop_front().expect("first task should remain queued");
        let (_, second) = source
            .pop_front()
            .expect("second task should remain queued");
        assert_eq!(
            first.task_id(),
            RendererPageRenderingUpdateTaskId::from_raw(1)
        );
        assert_eq!(
            second.task_id(),
            RendererPageRenderingUpdateTaskId::from_raw(2)
        );
        assert_eq!(
            second.kind(),
            RendererPageRenderingUpdateTaskKind::AnimationStartScan
        );
    }

    #[test]
    fn retired_rendering_source_rejects_without_a_fallback_route() {
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(PageId::new_for_testing(72)),
        );
        let source = RendererPageRenderingUpdateSource::new(
            owner_wake,
            RendererOwnerWakeSender::signal_rendering_update_task,
        );
        let sender = RendererPageRenderingUpdateSender::new(source.route(), root_document());
        drop(source);

        assert!(
            sender
                .send(
                    target(13),
                    RendererPageRenderingUpdateTaskId::from_raw(3),
                    RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
                )
                .is_err()
        );
    }
}
