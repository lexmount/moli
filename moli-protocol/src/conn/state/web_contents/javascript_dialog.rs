use moli_core::{
    browser::DocumentId,
    page::{
        RendererDocumentLifecycleIdentity, RendererJavaScriptDialogId,
        RendererPendingJavaScriptDialog,
    },
};

/// Reuses the Browser incarnation and exact renderer source; no new allocator.
/// The source distinguishes a parked popup from its later materialized Page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JavaScriptDialogKey {
    pub(in crate::conn) document: DocumentId,
    source: RendererDocumentLifecycleIdentity,
    dialog: RendererJavaScriptDialogId,
}

impl JavaScriptDialogKey {
    pub(crate) fn new(document: DocumentId, dialog: &RendererPendingJavaScriptDialog) -> Self {
        Self {
            document,
            source: dialog.source_document(),
            dialog: dialog.id(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct JavaScriptDialogSnapshot {
    pub(crate) dialog_type: String,
    pub(crate) message: String,
    pub(crate) default_prompt: String,
}

#[derive(Debug)]
pub(crate) struct JavaScriptDialogClosed {
    pub(crate) dialog_type: String,
    pub(crate) user_input: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum JavaScriptDialogError {
    NotFound,
    NotPrompt,
}

#[derive(Debug)]
struct JavaScriptDialog {
    document: DocumentId,
    renderer: RendererPendingJavaScriptDialog,
    prompt_text: Option<String>,
}

impl JavaScriptDialog {
    fn key(&self) -> JavaScriptDialogKey {
        JavaScriptDialogKey::new(self.document, &self.renderer)
    }
}

/// Browser-owned modal state. Session snapshots can copy keys, never this owner.
#[derive(Debug, Default)]
pub(in crate::conn) struct JavaScriptDialogs {
    pending: Vec<JavaScriptDialog>,
}

impl JavaScriptDialogs {
    pub(in crate::conn) fn install(
        &mut self,
        document: DocumentId,
        renderer: RendererPendingJavaScriptDialog,
    ) -> JavaScriptDialogKey {
        let dialog = JavaScriptDialog {
            document,
            renderer,
            prompt_text: None,
        };
        let key = dialog.key();
        assert!(
            !self.pending.iter().any(|dialog| dialog.key() == key),
            "one renderer dialog may be installed only once"
        );
        self.pending.push(dialog);
        key
    }

    pub(in crate::conn) fn snapshot(
        &self,
        key: JavaScriptDialogKey,
    ) -> Option<JavaScriptDialogSnapshot> {
        let dialog = &self
            .pending
            .iter()
            .find(|dialog| dialog.key() == key)?
            .renderer;
        Some(JavaScriptDialogSnapshot {
            dialog_type: dialog.dialog_type().into(),
            message: dialog.message().into(),
            default_prompt: dialog.default_prompt().into(),
        })
    }

    pub(in crate::conn) fn set_prompt_text(
        &mut self,
        key: JavaScriptDialogKey,
        prompt_text: String,
    ) -> Result<(), JavaScriptDialogError> {
        let dialog = self
            .pending
            .iter_mut()
            .find(|dialog| dialog.key() == key)
            .ok_or(JavaScriptDialogError::NotFound)?;
        if dialog.renderer.dialog_type() != "prompt" {
            return Err(JavaScriptDialogError::NotPrompt);
        }
        dialog.prompt_text = Some(prompt_text);
        Ok(())
    }

    pub(in crate::conn) fn finish(
        &mut self,
        key: JavaScriptDialogKey,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<JavaScriptDialogClosed> {
        let index = self.pending.iter().position(|dialog| dialog.key() == key)?;
        let dialog = self.pending.remove(index);
        let user_input = prompt_text.or(dialog.prompt_text).unwrap_or_default();
        dialog
            .renderer
            .finish(accepted, user_input.clone())
            .then(|| JavaScriptDialogClosed {
                dialog_type: dialog.renderer.dialog_type().into(),
                user_input,
            })
    }

    pub(in crate::conn) fn dismiss(&mut self, key: JavaScriptDialogKey) {
        let _ = self.finish(key, false, Some(String::new()));
    }

    pub(in crate::conn) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(in crate::conn) fn clear(&mut self) {
        for dialog in self.pending.drain(..) {
            let _ = dialog.renderer.finish(false, String::new());
        }
    }
}

impl Drop for JavaScriptDialogs {
    fn drop(&mut self) {
        self.clear();
    }
}
