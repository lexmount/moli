use crate::{
    browser::{
        DocumentHandle, WebContentsHandle,
        web_contents::{
            JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogKey,
            JavaScriptDialogSnapshot,
        },
    },
    page::RendererPendingJavaScriptDialog,
};

use super::BrowserContext;

impl BrowserContext {
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn clear_web_contents_javascript_dialogs_for_test(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?.javascript_dialogs.clear();
        Ok(())
    }

    pub fn web_contents_has_pending_javascript_dialog(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        Ok(!self.web_contents(handle)?.javascript_dialogs.is_empty())
    }

    pub(in crate::browser) fn install_document_javascript_dialog(
        &mut self,
        document: DocumentHandle,
        dialog: RendererPendingJavaScriptDialog,
    ) -> Result<Option<JavaScriptDialogKey>, String> {
        if let Err(error) = self.ensure_document_current(document) {
            let _ = dialog.finish(false, String::new());
            return Err(error);
        }
        self.web_contents_mut(document.web_contents())?
            .javascript_dialogs
            .mark_admitted(dialog.id());
        let source = dialog.source_document();
        if self
            .document(document)?
            .lifecycle
            .snapshot()
            .is_some_and(|snapshot| {
                snapshot.frame == source.frame
                    && snapshot.document == source.document
                    && (snapshot.epoch.0 > source.epoch.0
                        || (snapshot.epoch == source.epoch && snapshot.terminated.is_some()))
            })
        {
            // Native progress can precede the concrete frontend FIFO. Preserve
            // the historical opening output, but never resurrect its modal
            // capability after document.open/termination retired that epoch.
            let _ = dialog.finish(false, String::new());
            return Ok(None);
        }
        Ok(Some(
            self.web_contents_mut(document.web_contents())?
                .javascript_dialogs
                .install(document.id(), dialog),
        ))
    }

    pub(in crate::browser) fn javascript_dialog_snapshots(
        &self,
    ) -> Vec<crate::browser::JavaScriptDialogOpened> {
        self.web_contents_handles()
            .flat_map(|contents| self.web_contents_javascript_dialog_snapshots(contents))
            .collect()
    }

    pub(in crate::browser) fn web_contents_javascript_dialog_snapshots(
        &self,
        contents: WebContentsHandle,
    ) -> Vec<crate::browser::JavaScriptDialogOpened> {
        self.document_handle_for_web_contents(contents)
            .ok()
            .flatten()
            .into_iter()
            .flat_map(|document| {
                self.web_contents(contents)
                    .expect("live contents")
                    .javascript_dialogs
                    .snapshots(document)
            })
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn install_document_javascript_dialog_for_test(
        &mut self,
        document: DocumentHandle,
        dialog: RendererPendingJavaScriptDialog,
    ) -> Result<Option<JavaScriptDialogKey>, String> {
        self.install_document_javascript_dialog(document, dialog)
    }

    pub fn document_javascript_dialog_snapshot(
        &self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) -> Option<JavaScriptDialogSnapshot> {
        self.ensure_document_current(document).ok()?;
        self.web_contents(document.web_contents())
            .ok()?
            .javascript_dialogs
            .snapshot(key)
    }

    pub fn set_document_javascript_dialog_prompt_text(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
        prompt_text: String,
    ) -> Result<(), JavaScriptDialogError> {
        self.ensure_document_current(document)
            .map_err(|_| JavaScriptDialogError::NotFound)?;
        self.web_contents_mut(document.web_contents())
            .map_err(|_| JavaScriptDialogError::NotFound)?
            .javascript_dialogs
            .set_prompt_text(key, prompt_text)
    }

    pub fn finish_document_javascript_dialog(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<JavaScriptDialogClosed> {
        self.ensure_document_current(document).ok()?;
        self.web_contents_mut(document.web_contents())
            .ok()?
            .javascript_dialogs
            .finish(key, accepted, prompt_text)
    }
}
