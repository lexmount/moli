use std::time::{Duration, Instant};

use moli_layout::LayoutFlushReason;

use super::{JsContextHost, OwnerDispatchScope, WindowDocumentTaskTarget};
use crate::page_task_queue::RendererPageRenderingUpdateTaskId;

const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

#[derive(Default)]
pub(super) struct RealtimeFrameState {
    next_id: u64,
    pending: Option<(RendererPageRenderingUpdateTaskId, WindowDocumentTaskTarget)>,
    #[cfg(test)]
    completed_for_test: Option<tokio::sync::oneshot::Sender<()>>,
}

impl RealtimeFrameState {
    pub(super) fn is_idle(&self) -> bool {
        self.pending.is_none()
    }

    pub(super) fn retire_document(&mut self) {
        // Do not reuse ids across document.open(), or a stale claimed task
        // could retire the replacement's payload during discard.
        self.pending = None;
        #[cfg(test)]
        self.completed_for_test.take();
    }
}

impl JsContextHost {
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
        let mut frame = state.frame.borrow_mut();
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

    /// Run the layout phase under the existing exact-owner Page task
    /// authorization/completion. This body does not dispatch page callbacks.
    pub(crate) fn apply_realtime_rendering_opportunity(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            self.current_realtime_rendering_opportunity() == Some((id, target)),
            "authorized realtime opportunity lost its exact payload"
        );
        self.discard_realtime_rendering_opportunity(id);
        // Empty Documents have no layout root. A later parser/DOM commit will
        // request a new opportunity; do not automatically retry the same input.
        if self
            .dom_host()
            .dom()
            .document_element_handle_for_document(self.document_handle())
            .is_none()
        {
            return Ok(false);
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
        // No self-renewal: a clean result and no new semantic demand are idle.
        // Layout failure likewise consumes this attempt; new input may retry.
        Ok(false)
    }
}
