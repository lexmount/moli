use super::{
    JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget,
    window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask},
};
use crate::{
    document_runtime::DomHandle,
    page_task_queue::{RendererPageFontLoadingTaskId, RendererPageFontLoadingTaskKind},
};

struct DocumentFontReadiness {
    document: DomHandle,
    target: WindowDocumentTaskTarget,
    ready: bool,
}

/// Native readiness exists even when script has never requested the wrapper.
/// Promises remain in the Document's FontFaceSet wrapper, not this task ledger.
#[derive(Default)]
pub(super) struct RealtimeFontReadiness {
    documents: Vec<DocumentFontReadiness>,
    checking: Option<WindowDocumentTaskTarget>,
    tasks: ExactWindowDocumentTaskLedger<
        RendererPageFontLoadingTaskId,
        RendererPageFontLoadingTaskKind,
        (),
    >,
}

impl RealtimeFontReadiness {
    pub(super) fn is_idle(&self) -> bool {
        self.checking.is_none() && self.tasks.iter().next().is_none()
    }
}

impl JsContextHost {
    fn realtime_font_target(&self, document: DomHandle) -> Option<WindowDocumentTaskTarget> {
        self.realtime_layout_state.as_ref()?;
        let dispatch = self.owner_dispatch_scope_for_node(document)?;
        // Lightweight popup geometry is not part of the canonical frame tree.
        if matches!(dispatch, OwnerDispatchScope::LightweightPopup(_)) {
            return None;
        }
        self.current_window_document_task_target_for_dispatch_scope(dispatch)
    }

    pub(crate) fn initialize_realtime_font_readiness(&self, document: DomHandle) {
        let Some(target) = self.realtime_font_target(document) else {
            return;
        };
        let state = self.realtime_layout_state.as_ref().unwrap();
        let mut fonts = state.font_readiness.borrow_mut();
        if fonts.documents.iter().any(|entry| entry.target == target) {
            return;
        }
        fonts.documents.push(DocumentFontReadiness {
            document,
            target,
            ready: false,
        });
        drop(fonts);
        self.queue_realtime_font_readiness(document);
    }

    pub(super) fn initialize_realtime_document_fonts(&self) {
        if self.realtime_layout_state.is_none() {
            return;
        }
        for document in self.active_layout_document_handles() {
            self.initialize_realtime_font_readiness(document);
            // Initialization can precede installation of Page capabilities.
            self.queue_realtime_font_readiness(document);
        }
    }

    pub(crate) fn realtime_document_fonts_ready(&self, document: DomHandle) -> Option<bool> {
        let target = self.realtime_font_target(document)?;
        let state = self.realtime_layout_state.as_ref()?;
        Some(
            state
                .font_readiness
                .borrow()
                .documents
                .iter()
                .find(|entry| entry.target == target)
                .is_some_and(|entry| entry.ready),
        )
    }

    pub(super) fn note_realtime_font_loading(&self, document: DomHandle) {
        self.initialize_realtime_font_readiness(document);
        let Some(target) = self.realtime_font_target(document) else {
            return;
        };
        let state = self.realtime_layout_state.as_ref().unwrap();
        if let Some(entry) = state
            .font_readiness
            .borrow_mut()
            .documents
            .iter_mut()
            .find(|entry| entry.target == target)
        {
            entry.ready = false;
        }
    }

    pub(crate) fn queue_realtime_font_readiness(&self, document: DomHandle) {
        let Some(target) = self.realtime_font_target(document) else {
            return;
        };
        if self.page_task_capabilities.get().is_none() {
            return;
        }
        let state = self.realtime_layout_state.as_ref().unwrap();
        let mut fonts = state.font_readiness.borrow_mut();
        if fonts.checking == Some(target) {
            return;
        }
        if !fonts
            .documents
            .iter()
            .any(|entry| entry.target == target && !entry.ready)
        {
            return;
        }
        let kind = RendererPageFontLoadingTaskKind::CheckReadiness;
        if fonts
            .tasks
            .find_slot_index(target, kind, |_| true)
            .is_some()
        {
            return;
        }
        let id = fonts
            .tasks
            .allocate_task_id(RendererPageFontLoadingTaskId::from_raw);
        fonts
            .tasks
            .push(PendingExactWindowDocumentTask::new(id, target, kind, ()));
        if self
            .page_font_loading_sender()
            .send(target, id, kind)
            .is_err()
        {
            fonts.tasks.remove(id);
            tracing::debug!(?target, "font readiness route closed");
        }
    }

    pub(crate) fn current_pending_font_loading_task(
        &self,
        id: RendererPageFontLoadingTaskId,
    ) -> Option<(WindowDocumentTaskTarget, RendererPageFontLoadingTaskKind)> {
        let fonts = self.realtime_layout_state.as_ref()?.font_readiness.borrow();
        let pending = fonts.tasks.pending(id)?;
        let target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((target, pending.kind()))
    }

    pub(crate) fn take_pending_font_loading_task(
        &self,
        id: RendererPageFontLoadingTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageFontLoadingTaskKind,
    ) -> Option<DomHandle> {
        let mut fonts = self
            .realtime_layout_state
            .as_ref()?
            .font_readiness
            .borrow_mut();
        fonts.tasks.remove_exact(id, target, kind)?;
        let document = fonts
            .documents
            .iter()
            .find(|entry| entry.target == target)
            .map(|entry| entry.document)?;
        fonts.checking = Some(target);
        Some(document)
    }

    pub(crate) fn finish_font_readiness_check(&self) {
        if let Some(state) = &self.realtime_layout_state {
            state.font_readiness.borrow_mut().checking = None;
        }
    }

    pub(crate) fn font_readiness_environment_ready(&self, document: DomHandle) -> bool {
        self.dom_host()
            .document_ready_state_for_handle(document)
            .unwrap_or_else(|| self.host_document().ready_state())
            == crate::dom::native::DocumentReadyState::Complete
    }

    pub(super) fn did_publish_realtime_font_layout(&self) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        let pending = state
            .font_readiness
            .borrow()
            .documents
            .iter()
            .filter(|entry| !entry.ready)
            .map(|entry| entry.document)
            .collect::<Vec<_>>();
        for document in pending {
            if self.font_readiness_environment_ready(document)
                && !self.document_web_fonts_have_pending_requests(document)
            {
                self.queue_realtime_font_readiness(document);
            }
        }
    }

    pub(crate) fn discard_pending_font_loading_task(
        &self,
        id: RendererPageFontLoadingTaskId,
    ) -> bool {
        self.realtime_layout_state
            .as_ref()
            .is_some_and(|state| state.font_readiness.borrow_mut().tasks.remove(id).is_some())
    }

    pub(crate) fn mark_realtime_document_fonts_ready(&self, document: DomHandle) {
        let Some(target) = self.realtime_font_target(document) else {
            return;
        };
        let state = self.realtime_layout_state.as_ref().unwrap();
        let mut fonts = state.font_readiness.borrow_mut();
        if let Some(entry) = fonts
            .documents
            .iter_mut()
            .find(|entry| entry.target == target)
        {
            entry.ready = true;
        }
        // A synchronous data font can finish during this check. Its coalesced
        // continuation is redundant once this same input is known ready.
        fonts.tasks.retain(|pending| pending.target() != target);
    }

    pub(crate) fn ensure_realtime_font_layout(&self) -> Result<(), moli_layout::LayoutError> {
        if self
            .dom_host()
            .dom()
            .document_element_handle_for_document(self.document_handle())
            .is_some()
        {
            self.ensure_current_screen_layout(moli_layout::LayoutFlushReason::Explicit)?;
        }
        Ok(())
    }

    pub(super) fn rebind_realtime_font_readiness(
        &self,
        retired: WindowDocumentOwner,
        current: WindowDocumentOwner,
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        let document = {
            let mut fonts = state.font_readiness.borrow_mut();
            fonts
                .tasks
                .retain(|pending| pending.target().owner() != retired);
            fonts
                .documents
                .iter_mut()
                .find(|entry| entry.target.owner() == retired)
                .map(|entry| {
                    entry.target =
                        WindowDocumentTaskTarget::new(current, entry.target.dispatch_scope());
                    entry.document
                })
        };
        if let Some(document) = document {
            self.queue_realtime_font_readiness(document);
        }
    }

    pub(super) fn retire_realtime_font_readiness(&self, retired: WindowDocumentOwner) {
        if let Some(state) = &self.realtime_layout_state {
            let mut fonts = state.font_readiness.borrow_mut();
            fonts
                .tasks
                .retain(|pending| pending.target().owner() != retired);
            fonts
                .documents
                .retain(|entry| entry.target.owner() != retired);
        }
    }
}
