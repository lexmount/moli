use super::{JsContextHost, OwnerDispatchScope, WindowDocumentTaskTarget};
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
    pub(crate) fn open_beforeunload_dialog_for_dispatch_scope(
        &mut self,
        dispatch_scope: OwnerDispatchScope,
        force_browser_handler: bool,
    ) -> Option<RendererJavaScriptDialogResult> {
        let (target, source_document, source) =
            self.renderer_window_document_source_for_dispatch_scope(dispatch_scope)?;
        let source_url = match dispatch_scope {
            OwnerDispatchScope::Top => self.document_url().to_string(),
            OwnerDispatchScope::Child(handle) => {
                self.document_url_for_child_context(handle).to_string()
            }
        };
        let dialog_id = self.allocate_javascript_dialog_id();
        let dialog = RendererPendingJavaScriptDialog::new(
            dialog_id,
            source_document,
            source,
            source_url,
            "beforeunload".to_owned(),
            String::new(),
            String::new(),
            None,
        );
        if force_browser_handler {
            self.open_modal_javascript_dialog_with_browser_handler(dialog)
        } else {
            self.open_modal_javascript_dialog(target, dialog)
        }
    }

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
        let published = self.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::JavaScriptDialog(dialog.clone()),
        );
        if published {
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
        self.open_modal_javascript_dialog_with_browser_handler(dialog)
    }

    fn open_modal_javascript_dialog_with_browser_handler(
        &mut self,
        dialog: RendererPendingJavaScriptDialog,
    ) -> Option<RendererJavaScriptDialogResult> {
        let (dialog, modal) = self.javascript_dialog_runtime.begin_modal(dialog);
        if self.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::JavaScriptDialog(dialog),
        ) && let Some(predecessor) = self.publish_live_turn_output_prefix()
        {
            modal.publish(predecessor);
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
