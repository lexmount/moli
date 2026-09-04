use moli_core::page::{
    RendererDocumentLifecycleIdentity, RendererJavaScriptDialogId, RendererJavaScriptDialogSource,
    RendererPendingJavaScriptDialog,
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use super::{
    PageTargetHost,
    web_contents::{
        JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogKey,
        JavaScriptDialogSnapshot,
    },
};
use super::{TargetPageProtocolAttachmentIdentity, TargetPageResidenceIdentity};

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
    pub(in crate::conn::state) key: JavaScriptDialogKey,
}

impl TargetJavaScriptDialog {
    pub(crate) fn new(source_frame_id: String, key: JavaScriptDialogKey) -> Self {
        Self {
            source_frame_id,
            key,
        }
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

// In-place AgentHost -> Browser bridge, replaced by typed commands in Commit 22.
// Session authority stays here; WebContents only receives exact neutral keys.
impl PageTargetHost {
    pub(crate) fn install_javascript_dialog(
        &mut self,
        session: &moli_page_types::DevToolsSessionKey,
        page_owner: TargetPageResidenceIdentity,
        source_frame_id: String,
        dialog: RendererPendingJavaScriptDialog,
    ) -> bool {
        if self.current_document_id() != Some(page_owner.document_id()) {
            let _ = dialog.finish(false, String::new());
            return false;
        }
        let key = self
            .runtime_slot
            .page_slot_mut()
            .contents
            .javascript_dialogs
            .install(page_owner.document_id(), dialog);
        self.devtools_sessions
            .ensure_session(session)
            .page_session_state
            .javascript_dialog_state
            .push(TargetJavaScriptDialog::new(source_frame_id, key));
        true
    }

    fn javascript_dialog_key(
        &self,
        session: &moli_page_types::DevToolsSessionKey,
    ) -> Option<JavaScriptDialogKey> {
        let dialog = self
            .devtools_sessions
            .session(session)?
            .page_session_state
            .javascript_dialog_state
            .peek_next()?;
        (Some(dialog.key.document) == self.current_document_id()).then_some(dialog.key)
    }

    pub(crate) fn javascript_dialog_snapshot(
        &self,
        session: &moli_page_types::DevToolsSessionKey,
    ) -> Option<JavaScriptDialogSnapshot> {
        let key = self.javascript_dialog_key(session)?;
        self.runtime_slot
            .page_slot()
            .contents
            .javascript_dialogs
            .snapshot(key)
    }

    pub(crate) fn set_javascript_dialog_prompt_text(
        &mut self,
        session: &moli_page_types::DevToolsSessionKey,
        prompt_text: String,
    ) -> Result<(), JavaScriptDialogError> {
        let key = self
            .javascript_dialog_key(session)
            .ok_or(JavaScriptDialogError::NotFound)?;
        self.runtime_slot
            .page_slot_mut()
            .contents
            .javascript_dialogs
            .set_prompt_text(key, prompt_text)
    }

    pub(crate) fn handle_javascript_dialog(
        &mut self,
        session: &moli_page_types::DevToolsSessionKey,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<(String, JavaScriptDialogClosed)> {
        let Some(key) = self.javascript_dialog_key(session) else {
            self.dismiss_devtools_javascript_dialogs(session);
            return None;
        };
        let projection = self
            .devtools_sessions
            .ensure_session(session)
            .page_session_state
            .javascript_dialog_state
            .pop_next()?;
        let outcome = self
            .runtime_slot
            .page_slot_mut()
            .contents
            .javascript_dialogs
            .finish(key, accepted, prompt_text)?;
        Some((projection.source_frame_id, outcome))
    }

    pub(crate) fn dismiss_devtools_javascript_dialogs(
        &mut self,
        session: &moli_page_types::DevToolsSessionKey,
    ) {
        let projections = self
            .devtools_sessions
            .ensure_session(session)
            .page_session_state
            .javascript_dialog_state
            .take_pending();
        self.dismiss_javascript_dialog_projections(projections);
    }

    pub(in crate::conn::state) fn dismiss_javascript_dialog_projections(
        &mut self,
        projections: Vec<TargetJavaScriptDialog>,
    ) {
        for projection in projections {
            self.runtime_slot
                .page_slot_mut()
                .contents
                .javascript_dialogs
                .dismiss(projection.key);
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
