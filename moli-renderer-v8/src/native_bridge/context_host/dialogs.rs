use super::{JsContextHost, WindowDocumentTaskTarget};
use crate::runtime::{
    RendererJavaScriptDialogId, RendererJavaScriptDialogResult, RendererPendingJavaScriptDialog,
};

#[cfg(test)]
pub(super) struct PendingJavaScriptDialogRecord {
    target: WindowDocumentTaskTarget,
    dialog: RendererPendingJavaScriptDialog,
}

#[cfg(test)]
impl PendingJavaScriptDialogRecord {
    fn new(target: WindowDocumentTaskTarget, dialog: RendererPendingJavaScriptDialog) -> Self {
        Self { target, dialog }
    }
}

impl JsContextHost {
    pub(crate) fn set_javascript_dialog_handler_enabled(&mut self, enabled: bool) {
        self.javascript_dialog_handler_enabled = enabled;
    }

    pub(crate) fn javascript_dialog_handler_enabled(&self) -> bool {
        self.javascript_dialog_handler_enabled
            || self
                .browser_context_runtime
                .javascript_dialog_handler_enabled()
    }

    pub(crate) fn record_pending_javascript_dialog(
        &mut self,
        target: WindowDocumentTaskTarget,
        dialog: RendererPendingJavaScriptDialog,
    ) {
        let published = self.append_live_turn_observation(
            crate::runtime::RendererProtocolObservation::JavaScriptDialog(dialog.opening()),
        );
        if published {
            self.javascript_dialog_runtime.record(dialog);
            return;
        }
        #[cfg(test)]
        self.pending_javascript_dialogs
            .push(PendingJavaScriptDialogRecord::new(target, dialog));
        #[cfg(not(test))]
        {
            let _ = target;
            panic!("a production JavaScript dialog must have a concrete renderer output sink");
        }
    }

    pub(crate) fn open_modal_javascript_dialog(
        &mut self,
        target: WindowDocumentTaskTarget,
        dialog: RendererPendingJavaScriptDialog,
    ) -> Option<RendererJavaScriptDialogResult> {
        if !self.javascript_dialog_handler_enabled() {
            self.record_pending_javascript_dialog(target, dialog);
            return None;
        }
        let (dialog, modal) = self.javascript_dialog_runtime.begin_modal(dialog);
        if self.append_live_turn_observation(
            crate::runtime::RendererProtocolObservation::JavaScriptDialog(dialog),
        ) && self.publish_live_turn_output_prefix()
        {
            return Some(modal.wait());
        }
        modal.cancel();
        None
    }

    pub(crate) fn allocate_javascript_dialog_id(&mut self) -> RendererJavaScriptDialogId {
        let id = RendererJavaScriptDialogId::new(self.next_javascript_dialog_id);
        self.next_javascript_dialog_id = self
            .next_javascript_dialog_id
            .checked_add(1)
            .expect("JavaScript dialog sequence exhausted");
        id
    }

    #[cfg(test)]
    pub(crate) fn take_pending_javascript_dialogs(
        &mut self,
    ) -> Vec<RendererPendingJavaScriptDialog> {
        let pending = std::mem::take(&mut self.pending_javascript_dialogs);
        let mut dialogs = Vec::with_capacity(pending.len());
        for pending in pending {
            if self.window_document_owner_is_current_for_dispatch_scope(
                pending.target.owner(),
                pending.target.dispatch_scope(),
            ) {
                dialogs.push(pending.dialog);
            } else {
                let _ = pending.dialog.finish(false, String::new());
            }
        }
        dialogs
    }

    pub(crate) fn pending_javascript_dialog_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_javascript_dialogs.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }
}
