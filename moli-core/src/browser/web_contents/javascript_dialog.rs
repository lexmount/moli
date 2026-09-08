use crate::{
    browser::DocumentId,
    page::{
        RendererDocumentLifecycleIdentity, RendererJavaScriptDialogId,
        RendererPendingJavaScriptDialog,
    },
};

/// Reuses the Browser incarnation and exact renderer source; no new allocator.
/// The source distinguishes a parked popup from its later materialized Page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JavaScriptDialogKey {
    pub document: DocumentId,
    source: RendererDocumentLifecycleIdentity,
    dialog: RendererJavaScriptDialogId,
}

impl JavaScriptDialogKey {
    pub fn new(
        document: DocumentId,
        source: RendererDocumentLifecycleIdentity,
        dialog: RendererJavaScriptDialogId,
    ) -> Self {
        Self {
            document,
            source,
            dialog,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct JavaScriptDialogSnapshot {
    pub dialog_type: String,
    pub message: String,
    pub default_prompt: String,
}

#[derive(Debug)]
pub struct JavaScriptDialogClosed {
    pub dialog_type: String,
    pub user_input: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JavaScriptDialogError {
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
        JavaScriptDialogKey::new(
            self.document,
            self.renderer.source_document(),
            self.renderer.id(),
        )
    }
}

/// Browser-owned modal state. Session snapshots can copy keys, never this owner.
#[derive(Debug, Default)]
pub struct JavaScriptDialogs {
    pending: Vec<JavaScriptDialog>,
    admitted: u64,
    admission: Option<tokio::sync::watch::Sender<u64>>,
}

impl JavaScriptDialogs {
    pub(in crate::browser) fn observe_admission(&mut self) -> tokio::sync::watch::Receiver<u64> {
        self.admission
            .get_or_insert_with(|| tokio::sync::watch::channel(self.admitted).0)
            .subscribe()
    }

    pub(in crate::browser) fn mark_admitted(&mut self, id: RendererJavaScriptDialogId) {
        self.admitted = self.admitted.max(id.sequence());
        if let Some(admission) = &self.admission {
            admission.send_replace(self.admitted);
        }
    }

    pub(in crate::browser) fn snapshots(
        &self,
        document: crate::browser::DocumentHandle,
    ) -> impl Iterator<Item = crate::browser::JavaScriptDialogOpened> + '_ {
        self.pending
            .iter()
            .map(move |dialog| crate::browser::JavaScriptDialogOpened {
                document,
                key: dialog.key(),
                opening: dialog.renderer.opening(),
            })
    }

    pub fn install(
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

    pub fn snapshot(&self, key: JavaScriptDialogKey) -> Option<JavaScriptDialogSnapshot> {
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

    pub fn set_prompt_text(
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

    pub fn finish(
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

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn clear(&mut self) {
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
