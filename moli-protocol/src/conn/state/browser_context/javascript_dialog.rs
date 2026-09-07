use super::BrowserContext;
use moli_core::browser::{DocumentHandle, WebContentsHandle};
use moli_core::page::{
    RendererDocumentLifecycleIdentity, RendererJavaScriptDialogId, RendererJavaScriptDialogSource,
    RendererPendingJavaScriptDialog,
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use crate::conn::state::TargetPageProtocolAttachmentIdentity;
#[cfg(test)]
use crate::conn::state::TargetPageResidenceIdentity;
use crate::conn::state::web_contents::{
    JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogKey, JavaScriptDialogSnapshot,
};

/// Stable lifetime of one target Page's JavaScript-dialog output.
///
/// `TargetRuntimeSlot` owns this scope independently of foldable protocol
/// session settings. Prepared renderer output observes it through a weak
/// handle; Document/Page retirement invalidates the old scope before
/// installing a fresh one.
#[derive(Clone, Debug)]
pub(crate) struct TargetJavaScriptDialogScope {
    inner: Arc<TargetJavaScriptDialogScopeInner>,
}

#[derive(Debug)]
struct TargetJavaScriptDialogScopeInner {
    current: AtomicBool,
}

#[derive(Clone, Debug)]
pub(crate) struct TargetJavaScriptDialogScopeObserver {
    inner: Weak<TargetJavaScriptDialogScopeInner>,
}

impl TargetJavaScriptDialogScope {
    fn new() -> Self {
        Self {
            inner: Arc::new(TargetJavaScriptDialogScopeInner {
                current: AtomicBool::new(true),
            }),
        }
    }

    pub(crate) fn observe(&self) -> TargetJavaScriptDialogScopeObserver {
        TargetJavaScriptDialogScopeObserver {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub(crate) fn observes(&self, observer: &TargetJavaScriptDialogScopeObserver) -> bool {
        let Some(observed) = observer.inner.upgrade() else {
            return false;
        };
        Arc::ptr_eq(&self.inner, &observed) && observed.current.load(Ordering::Acquire)
    }

    pub(crate) fn retire(&mut self) {
        self.inner.current.store(false, Ordering::Release);
        *self = Self::new();
    }
}

impl Default for TargetJavaScriptDialogScope {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for TargetJavaScriptDialogScopeObserver {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for TargetJavaScriptDialogScopeObserver {}

#[cfg(test)]
impl TargetJavaScriptDialogScopeObserver {
    pub(crate) fn stale_for_absent_owner_test() -> Self {
        Self { inner: Weak::new() }
    }
}

/// Destination policy frozen when a renderer dialog leaves its source Page.
///
/// Root and child-frame dialogs already belong to the attachment that captured
/// them. A lightweight popup has not necessarily acquired a protocol target
/// yet, so it retains the renderer popup/document identity until that target
/// is created. It must never fall back to the opener's root frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetPreparedJavaScriptDialogRoute {
    AttachedPage {
        source_frame_id: String,
    },
    LightweightPopup {
        popup_id: u64,
        popup_document_id: u64,
    },
}

/// One concrete dialog output between renderer capture and protocol install.
///
/// The exact source attachment and weak Page-dialog scope authorize the
/// capture. The optional renderer payload is a one-shot capability: consuming
/// this value installs it under one destination Page, while dropping an
/// unresolved value dismisses it so a blocking renderer call cannot hang.
#[derive(Debug, PartialEq)]
pub(crate) struct TargetPreparedJavaScriptDialog {
    source_attachment: TargetPageProtocolAttachmentIdentity,
    source_dialog_scope: TargetJavaScriptDialogScopeObserver,
    route: TargetPreparedJavaScriptDialogRoute,
    renderer_dialog: Option<RendererPendingJavaScriptDialog>,
}

impl TargetPreparedJavaScriptDialog {
    pub(crate) fn capture(
        source_attachment: TargetPageProtocolAttachmentIdentity,
        source_dialog_scope: TargetJavaScriptDialogScopeObserver,
        root_frame_id: &str,
        renderer_dialog: RendererPendingJavaScriptDialog,
    ) -> Self {
        let route = match renderer_dialog.source() {
            RendererJavaScriptDialogSource::RootFrame => {
                TargetPreparedJavaScriptDialogRoute::AttachedPage {
                    source_frame_id: root_frame_id.to_owned(),
                }
            }
            RendererJavaScriptDialogSource::ChildFrame { frame_id, .. } => {
                TargetPreparedJavaScriptDialogRoute::AttachedPage {
                    source_frame_id: frame_id.clone(),
                }
            }
            RendererJavaScriptDialogSource::LightweightPopup {
                popup_id,
                popup_document_id,
            } => TargetPreparedJavaScriptDialogRoute::LightweightPopup {
                popup_id: *popup_id,
                popup_document_id: *popup_document_id,
            },
        };
        Self {
            source_attachment,
            source_dialog_scope,
            route,
            renderer_dialog: Some(renderer_dialog),
        }
    }

    pub(crate) fn source_attachment(&self) -> &TargetPageProtocolAttachmentIdentity {
        &self.source_attachment
    }

    pub(crate) fn source_dialog_scope(&self) -> &TargetJavaScriptDialogScopeObserver {
        &self.source_dialog_scope
    }

    pub(crate) fn route(&self) -> &TargetPreparedJavaScriptDialogRoute {
        &self.route
    }

    pub(crate) fn popup_id(&self) -> Option<u64> {
        match &self.route {
            TargetPreparedJavaScriptDialogRoute::AttachedPage { .. } => None,
            TargetPreparedJavaScriptDialogRoute::LightweightPopup { popup_id, .. } => {
                Some(*popup_id)
            }
        }
    }

    pub(crate) fn id(&self) -> RendererJavaScriptDialogId {
        self.renderer_dialog().id()
    }

    pub(crate) fn source_document(&self) -> RendererDocumentLifecycleIdentity {
        self.renderer_dialog().source_document()
    }

    pub(crate) fn source_url(&self) -> &str {
        self.renderer_dialog().source_url()
    }

    pub(crate) fn message(&self) -> &str {
        self.renderer_dialog().message()
    }

    pub(crate) fn dialog_type(&self) -> &str {
        self.renderer_dialog().dialog_type()
    }

    pub(crate) fn default_prompt(&self) -> &str {
        self.renderer_dialog().default_prompt()
    }

    pub(crate) fn dismiss(mut self) {
        self.dismiss_inner();
    }

    pub(crate) fn into_renderer_dialog(mut self) -> RendererPendingJavaScriptDialog {
        self.renderer_dialog
            .take()
            .expect("prepared dialog must own its renderer payload")
    }

    fn renderer_dialog(&self) -> &RendererPendingJavaScriptDialog {
        self.renderer_dialog
            .as_ref()
            .expect("prepared dialog must retain its renderer payload until settlement")
    }

    fn dismiss_inner(&mut self) {
        if let Some(dialog) = self.renderer_dialog.take() {
            let _ = dialog.finish(false, String::new());
        }
    }
}

impl Drop for TargetPreparedJavaScriptDialog {
    fn drop(&mut self) {
        self.dismiss_inner();
    }
}

/// A session's projection of a Browser-owned dialog, not its completion owner.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TargetJavaScriptDialog {
    source_frame_id: String,
    document: DocumentHandle,
    pub(in crate::conn::state) key: JavaScriptDialogKey,
}

impl TargetJavaScriptDialog {
    pub(crate) fn new(
        source_frame_id: String,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) -> Self {
        debug_assert_eq!(document.id(), key.document);
        Self {
            source_frame_id,
            document,
            key,
        }
    }

    pub(crate) fn into_parts(self) -> (String, DocumentHandle, JavaScriptDialogKey) {
        (self.source_frame_id, self.document, self.key)
    }

    #[cfg(test)]
    pub(crate) fn document_id(&self) -> moli_core::browser::DocumentId {
        self.key.document
    }

    #[cfg(test)]
    pub(crate) fn source_frame_id(&self) -> &str {
        &self.source_frame_id
    }
}

/// Clone/clear only affects frontend visibility. Browser owns all modal work.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TargetJavaScriptDialogState {
    pending_dialogs: Vec<TargetJavaScriptDialog>,
}

impl TargetJavaScriptDialogState {
    pub(crate) fn clear(&mut self) {
        self.pending_dialogs.clear();
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.pending_dialogs.is_empty()
    }

    pub(crate) fn push(&mut self, dialog: TargetJavaScriptDialog) {
        self.pending_dialogs.push(dialog);
    }

    fn peek_next(&self) -> Option<&TargetJavaScriptDialog> {
        self.pending_dialogs.first()
    }

    fn pop_next(&mut self) -> Option<TargetJavaScriptDialog> {
        (!self.pending_dialogs.is_empty()).then(|| self.pending_dialogs.remove(0))
    }

    pub(in crate::conn::state) fn take_pending(&mut self) -> Vec<TargetJavaScriptDialog> {
        std::mem::take(&mut self.pending_dialogs)
    }

    #[cfg(test)]
    pub(crate) fn pending_dialogs(&self) -> &[TargetJavaScriptDialog] {
        &self.pending_dialogs
    }
}

impl BrowserContext {
    fn validate_javascript_dialog_document(&self, document: DocumentHandle) -> Result<(), String> {
        match self.document_handle_for_web_contents(document.web_contents())? {
            Some(current) if current == document => Ok(()),
            Some(_) => Err("Document changed".into()),
            None => Err("NoDocumentLoaded".into()),
        }
    }

    pub(crate) fn web_contents_has_pending_javascript_dialog(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        Ok(!self
            .physical
            .web_contents(handle)?
            .javascript_dialogs
            .is_empty())
    }

    pub(crate) fn install_document_javascript_dialog(
        &mut self,
        document: DocumentHandle,
        dialog: RendererPendingJavaScriptDialog,
    ) -> Result<JavaScriptDialogKey, String> {
        if let Err(error) = self.validate_javascript_dialog_document(document) {
            let _ = dialog.finish(false, String::new());
            return Err(error);
        }
        Ok(self
            .physical
            .web_contents_mut(document.web_contents())?
            .javascript_dialogs
            .install(document.id(), dialog))
    }

    pub(crate) fn project_javascript_dialog_for_session(
        &mut self,
        target_id: &str,
        session: &moli_page_types::DevToolsSessionKey,
        source_frame_id: String,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) -> bool {
        if self.document_handle_for_target(target_id) != Some(document)
            || key.document != document.id()
        {
            return false;
        }
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .ensure_session(session)
            .page_session_state
            .javascript_dialog_state
            .push(TargetJavaScriptDialog::new(source_frame_id, document, key));
        true
    }

    pub(crate) fn projected_javascript_dialog_for_session(
        &self,
        target_id: &str,
        session: &moli_page_types::DevToolsSessionKey,
    ) -> Option<(DocumentHandle, JavaScriptDialogKey)> {
        let dialog = self
            .page_targets
            .get(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .session(session)?
            .page_session_state
            .javascript_dialog_state
            .peek_next()?;
        (self.document_handle_for_target(target_id) == Some(dialog.document)
            && self
                .validate_javascript_dialog_document(dialog.document)
                .is_ok())
        .then_some((dialog.document, dialog.key))
    }

    pub(crate) fn document_javascript_dialog_snapshot(
        &self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) -> Option<JavaScriptDialogSnapshot> {
        self.validate_javascript_dialog_document(document).ok()?;
        self.physical
            .web_contents(document.web_contents())
            .ok()?
            .javascript_dialogs
            .snapshot(key)
    }

    pub(crate) fn set_document_javascript_dialog_prompt_text(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
        prompt_text: String,
    ) -> Result<(), JavaScriptDialogError> {
        self.validate_javascript_dialog_document(document)
            .map_err(|_| JavaScriptDialogError::NotFound)?;
        self.physical
            .web_contents_mut(document.web_contents())
            .map_err(|_| JavaScriptDialogError::NotFound)?
            .javascript_dialogs
            .set_prompt_text(key, prompt_text)
    }

    pub(crate) fn finish_document_javascript_dialog(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<JavaScriptDialogClosed> {
        self.validate_javascript_dialog_document(document).ok()?;
        self.physical
            .web_contents_mut(document.web_contents())
            .ok()?
            .javascript_dialogs
            .finish(key, accepted, prompt_text)
    }

    pub(crate) fn dismiss_document_javascript_dialog(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) {
        if self.validate_javascript_dialog_document(document).is_ok()
            && let Ok(contents) = self.physical.web_contents_mut(document.web_contents())
        {
            contents.javascript_dialogs.dismiss(key);
        }
    }

    pub(crate) fn pop_projected_javascript_dialog_for_session(
        &mut self,
        target_id: &str,
        session: &moli_page_types::DevToolsSessionKey,
    ) -> Option<TargetJavaScriptDialog> {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .ensure_session(session)
            .page_session_state
            .javascript_dialog_state
            .pop_next()
    }

    pub(crate) fn take_projected_javascript_dialogs_for_session(
        &mut self,
        target_id: &str,
        session: &moli_page_types::DevToolsSessionKey,
    ) -> Vec<TargetJavaScriptDialog> {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .ensure_session(session)
            .page_session_state
            .javascript_dialog_state
            .take_pending()
    }

    pub(in crate::conn) fn dismiss_projected_javascript_dialogs(
        &mut self,
        projections: Vec<TargetJavaScriptDialog>,
    ) {
        for projection in projections {
            self.dismiss_document_javascript_dialog(projection.document, projection.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{
        PageId,
        page::{
            RendererDocumentLifecycleIdentity, RendererDocumentToken, RendererFrameToken,
            RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
            RendererJavaScriptDialogSource, RendererLifecycleEpoch,
            RendererPendingJavaScriptDialog,
        },
    };

    use super::{
        TargetJavaScriptDialogScope, TargetPageProtocolAttachmentIdentity,
        TargetPageResidenceIdentity, TargetPreparedJavaScriptDialog,
    };

    #[test]
    fn dropping_page_scope_invalidates_its_prepared_observer() {
        let scope = TargetJavaScriptDialogScope::default();
        let observer = scope.observe();
        drop(scope);

        assert!(
            !TargetJavaScriptDialogScope::default().observes(&observer),
            "dropping a Page scope must make its weak prepared-output observer stale"
        );
    }

    #[test]
    fn retiring_one_page_scope_invalidates_observers_across_shared_clones() {
        let mut scope = TargetJavaScriptDialogScope::default();
        let snapshot = scope.clone();
        let observer = snapshot.observe();

        scope.retire();

        assert!(!scope.observes(&observer));
        assert!(
            !snapshot.observes(&observer),
            "retirement must invalidate every snapshot sharing the old scope"
        );
    }

    #[test]
    fn dropping_uninstalled_prepared_dialog_dismisses_its_one_shot_completion() {
        let page_id = PageId::new_for_testing(1);
        let source_document = RendererDocumentLifecycleIdentity {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, 1),
            epoch: RendererLifecycleEpoch(1),
        };
        let completion = RendererJavaScriptDialogCompletion::pending();
        let scope = TargetJavaScriptDialogScope::default();
        let prepared = TargetPreparedJavaScriptDialog::capture(
            TargetPageProtocolAttachmentIdentity::new(
                TargetPageResidenceIdentity::new_for_test(
                    "BID-dialog-drop".to_owned(),
                    Some("TID-dialog-drop".to_owned()),
                    1,
                ),
                Some("SID-dialog-drop".to_owned()),
            ),
            scope.observe(),
            "TID-dialog-drop",
            RendererPendingJavaScriptDialog::new(
                RendererJavaScriptDialogId::new(1),
                source_document,
                RendererJavaScriptDialogSource::LightweightPopup {
                    popup_id: 3,
                    popup_document_id: 4,
                },
                "about:blank".to_owned(),
                "alert".to_owned(),
                "dismiss on drop".to_owned(),
                String::new(),
                Some(completion.clone()),
            ),
        );

        drop(prepared);

        assert!(!completion.finish(true, String::new()));
        assert!(!completion.wait().accepted);
    }
}
