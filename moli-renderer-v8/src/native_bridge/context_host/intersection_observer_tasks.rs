use crate::page_task_queue::{
    RendererPageIntersectionObserverTaskId, RendererPageIntersectionObserverTaskKind,
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(super) type IntersectionObserverTaskState = ExactWindowDocumentTaskLedger<
    RendererPageIntersectionObserverTaskId,
    RendererPageIntersectionObserverTaskKind,
    (),
>;

impl JsContextHost {
    /// Records stay in the existing observer store. This optional ledger only
    /// coalesces notification tasks for their exact owning Document.
    pub(crate) fn queue_intersection_observer_notification(
        &mut self,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let kind = RendererPageIntersectionObserverTaskKind::Notify;
        let ledger = self
            .intersection_observer_tasks
            .get_or_insert_with(Default::default);
        if ledger.find_slot_index(target, kind, |_| true).is_some() {
            return true;
        }
        let id = ledger.allocate_task_id(RendererPageIntersectionObserverTaskId::from_raw);
        ledger.push(PendingExactWindowDocumentTask::new(id, target, kind, ()));
        if self
            .page_intersection_observer_sender()
            .send(target, id, kind)
            .is_ok()
        {
            return true;
        }
        self.discard_pending_intersection_observer_task(id);
        tracing::debug!(?target, "IntersectionObserver notification route closed");
        false
    }

    pub(crate) fn current_pending_intersection_observer_task(
        &self,
        id: RendererPageIntersectionObserverTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageIntersectionObserverTaskKind,
    )> {
        let pending = self.intersection_observer_tasks.as_ref()?.pending(id)?;
        let target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((target, pending.kind()))
    }

    pub(crate) fn take_pending_intersection_observer_task(
        &mut self,
        id: RendererPageIntersectionObserverTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageIntersectionObserverTaskKind,
    ) -> bool {
        self.intersection_observer_tasks
            .as_mut()
            .is_some_and(|ledger| ledger.remove_exact(id, target, kind).is_some())
    }

    pub(crate) fn discard_pending_intersection_observer_task(
        &mut self,
        id: RendererPageIntersectionObserverTaskId,
    ) -> bool {
        self.intersection_observer_tasks
            .as_mut()
            .is_some_and(|ledger| ledger.remove(id).is_some())
    }
}
