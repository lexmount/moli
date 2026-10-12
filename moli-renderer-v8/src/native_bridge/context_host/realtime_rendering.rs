use std::time::{Duration, Instant};

use moli_layout::LayoutFlushReason;

use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::page_task_queue::RendererPageRenderingUpdateTaskId;

const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

struct RunningRealtimeFrame {
    ticket: (RendererPageRenderingUpdateTaskId, WindowDocumentTaskTarget),
    // The root is first. Only an explicit document.open transaction may move
    // these exact targets: it preserves the Web Document and Window, while
    // replacing Moli's task owner. Navigation never transfers this batch.
    documents: Vec<WindowDocumentTaskTarget>,
}

#[derive(Default)]
pub(super) struct RealtimeFrameState {
    next_id: u64,
    pending: Option<(RendererPageRenderingUpdateTaskId, WindowDocumentTaskTarget)>,
    running: Option<RunningRealtimeFrame>,
    source_requested: bool,
    #[cfg(test)]
    completed_for_test: Option<tokio::sync::oneshot::Sender<()>>,
}

impl RealtimeFrameState {
    pub(super) fn is_idle(&self) -> bool {
        self.pending.is_none() && self.running.is_none()
    }

    pub(super) fn retire_document(&mut self) {
        // Do not reuse ids across document.open(), or a stale claimed task
        // could retire the replacement's payload during discard.
        self.pending = None;
        // The running body can outlive its Document through author callbacks.
        // Only that body's settlement may release the non-reentrancy guard.
        self.source_requested = false;
        #[cfg(test)]
        self.completed_for_test.take();
    }
}

impl JsContextHost {
    pub(super) fn rebind_realtime_document_after_open(
        &self,
        retired: FrameDocumentTaskOwner,
        current: FrameDocumentTaskOwner,
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        debug_assert_eq!(retired.local_window_id, current.local_window_id);
        let retired = WindowDocumentOwner::Frame(retired);
        let current = WindowDocumentOwner::Frame(current);
        self.rebind_realtime_animation_frame_document(retired, current);
        {
            let mut frame = state.frame.borrow_mut();
            // Unstarted tasks still need a new exact-owner ticket. A running
            // body keeps its original ticket and non-reentrancy guard, but
            // follows the explicitly preserved Document in later phases.
            if frame
                .pending
                .is_some_and(|(_, target)| target.owner() == retired)
            {
                frame.pending = None;
            }
            if let Some(running) = &mut frame.running {
                for target in &mut running.documents {
                    if target.owner() == retired {
                        *target = WindowDocumentTaskTarget::new(current, target.dispatch_scope());
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn observe_next_realtime_opportunity_for_test(
        &self,
        completed: tokio::sync::oneshot::Sender<()>,
    ) {
        self.realtime_layout_state
            .as_ref()
            .expect("Realtime test observer")
            .frame
            .borrow_mut()
            .completed_for_test = Some(completed);
    }

    /// Source commits register semantic demand; the stable Page source owns
    /// deadline residence. This neither computes layout nor calls page JS.
    pub(crate) fn request_realtime_rendering_opportunity(&self) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        state.frame.borrow_mut().source_requested = true;
        self.refresh_realtime_rendering_opportunity();
    }

    /// Reconcile semantic demand with the single deadline residence. rAF
    /// registration/cancellation does not manufacture a layout source change.
    pub(crate) fn refresh_realtime_rendering_opportunity(&self) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        // Policy is installed before Page task capabilities during bootstrap.
        // The capability installation requests the initial opportunity once
        // the stable owner route exists; source commits still update inputs.
        if self.page_task_capabilities.get().is_none() {
            return;
        }
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(OwnerDispatchScope::Top)
        else {
            return;
        };
        if state.frame.borrow().running.is_some() {
            // This also covers document.open during a callback. There can
            // never be a second opportunity nested inside the running body.
            return;
        }
        let callbacks_pending = self.has_runnable_realtime_animation_frames();
        let mut frame = state.frame.borrow_mut();
        if !self.document_activity().visible || (!frame.source_requested && !callbacks_pending) {
            frame.pending = None;
            return;
        }
        if frame
            .pending
            .is_some_and(|(_, pending_target)| pending_target == target)
        {
            return;
        }
        frame.pending = None;
        let Some(next_id) = frame.next_id.checked_add(1) else {
            tracing::error!("realtime rendering opportunity id exhausted");
            return;
        };
        frame.next_id = next_id;
        let id = RendererPageRenderingUpdateTaskId::Frame(next_id);
        let deadline = Instant::now() + FRAME_INTERVAL;
        frame.pending = Some((id, target));
        if self
            .page_rendering_update_sender()
            .schedule_frame(target, id, deadline)
            .is_err()
        {
            frame.pending = None;
            tracing::debug!(
                ?target,
                ?id,
                "retired realtime opportunity after route closure"
            );
        }
    }

    pub(crate) fn current_realtime_rendering_opportunity(
        &self,
    ) -> Option<(RendererPageRenderingUpdateTaskId, WindowDocumentTaskTarget)> {
        let (id, target) = self
            .realtime_layout_state
            .as_ref()?
            .frame
            .borrow()
            .pending?;
        self.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        )
        .then_some((id, target))
    }

    pub(crate) fn discard_realtime_rendering_opportunity(
        &self,
        id: RendererPageRenderingUpdateTaskId,
    ) -> bool {
        let Some(state) = &self.realtime_layout_state else {
            return false;
        };
        let mut frame = state.frame.borrow_mut();
        if frame
            .pending
            .is_some_and(|(pending_id, _)| pending_id == id)
        {
            frame.pending = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn begin_realtime_rendering_opportunity(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            self.current_realtime_rendering_opportunity() == Some((id, target)),
            "authorized realtime opportunity lost its exact payload"
        );
        self.discard_realtime_rendering_opportunity(id);
        let state = self.realtime_layout_state.as_ref().expect("Realtime frame");
        let mut frame = state.frame.borrow_mut();
        anyhow::ensure!(frame.running.is_none(), "reentrant rendering opportunity");
        if !self.document_activity().visible {
            return Ok(false);
        }
        let documents = self.realtime_animation_frame_documents();
        anyhow::ensure!(
            documents.first() == Some(&target),
            "rendering root changed before its batch"
        );
        frame.running = Some(RunningRealtimeFrame {
            ticket: (id, target),
            documents,
        });
        Ok(true)
    }

    pub(crate) fn realtime_rendering_opportunity_is_current(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(root) = self.realtime_rendering_document(id, target, 0) else {
            return false;
        };
        self.window_document_owner_is_current_for_dispatch_scope(
            root.owner(),
            root.dispatch_scope(),
        )
    }

    pub(crate) fn realtime_rendering_document(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
        index: usize,
    ) -> Option<WindowDocumentTaskTarget> {
        let frame = self.realtime_layout_state.as_ref()?.frame.borrow();
        let running = frame.running.as_ref()?;
        (running.ticket == (id, root))
            .then(|| running.documents.get(index).copied())
            .flatten()
    }

    /// No page callbacks are invoked while native style/layout borrows exist.
    pub(crate) fn run_realtime_layout_phase(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> anyhow::Result<()> {
        if !self.realtime_rendering_opportunity_is_current(id, target) {
            return Ok(());
        }
        // This phase consumes all source changes preceding it, including
        // writes in rAF and its microtasks. Synchronous getters did not consume
        // lifecycle demand, but their geometry can satisfy this phase.
        self.realtime_layout_state
            .as_ref()
            .expect("Realtime frame")
            .frame
            .borrow_mut()
            .source_requested = false;
        // Empty Documents have no layout root. A later parser/DOM commit will
        // request a new opportunity; do not automatically retry the same input.
        if self
            .dom_host()
            .dom()
            .document_element_handle_for_document(self.document_handle())
            .is_none()
        {
            return Ok(());
        }
        self.ensure_current_screen_layout(LayoutFlushReason::RenderingOpportunity)?;
        #[cfg(test)]
        if let Some(completed) = self
            .realtime_layout_state
            .as_ref()
            .and_then(|state| state.frame.borrow_mut().completed_for_test.take())
        {
            let _ = completed.send(());
        }
        Ok(())
    }

    pub(crate) fn finish_realtime_rendering_opportunity(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
        succeeded: bool,
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        {
            let mut frame = state.frame.borrow_mut();
            let Some(running) = frame
                .running
                .as_ref()
                .filter(|running| running.ticket == (id, target))
            else {
                // Callback navigation may already have installed a new
                // document's request. Never settle it with the old ticket.
                return;
            };
            let current_root = running.documents[0];
            frame.running = None;
            if !succeeded
                && self.window_document_owner_is_current_for_dispatch_scope(
                    current_root.owner(),
                    current_root.dispatch_scope(),
                )
            {
                // Retain uncalled callbacks but require new explicit demand
                // after a fatal layout/checkpoint/watchdog failure.
                frame.source_requested = false;
                return;
            }
        }
        self.prune_realtime_animation_frames();
        self.refresh_realtime_rendering_opportunity();
    }
}
