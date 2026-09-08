use super::BrowserContext;
use moli_core::browser::{BrowserContextHandle, DocumentHandle, JavaScriptDialogOpened};
use moli_core::page::{
    RendererDocumentLifecycleIdentity, RendererJavaScriptDialogId, RendererJavaScriptDialogOpening,
    RendererJavaScriptDialogSource,
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use crate::conn::state::TargetPageProtocolAttachmentIdentity;
use moli_core::browser::web_contents::{
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

/// One concrete dialog observation waiting for protocol projection.
///
/// The exact source attachment and weak Page-dialog scope authorize the
/// capture. The actual request stays in its original Browser Document, even
/// when a lightweight popup projects the opening through another Target.
/// Dropping undelivered output requests native dismissal by exact key.
#[derive(Debug)]
pub(crate) struct TargetPreparedJavaScriptDialog {
    source_attachment: TargetPageProtocolAttachmentIdentity,
    source_dialog_scope: TargetJavaScriptDialogScopeObserver,
    route: TargetPreparedJavaScriptDialogRoute,
    browser_context: BrowserContextHandle,
    native_dialog: Option<JavaScriptDialogOpened>,
}

impl PartialEq for TargetPreparedJavaScriptDialog {
    fn eq(&self, other: &Self) -> bool {
        self.source_attachment == other.source_attachment
            && self.source_dialog_scope == other.source_dialog_scope
            && self.route == other.route
            && self.native_dialog == other.native_dialog
    }
}

impl TargetPreparedJavaScriptDialog {
    pub(crate) fn capture(
        source_attachment: TargetPageProtocolAttachmentIdentity,
        source_dialog_scope: TargetJavaScriptDialogScopeObserver,
        root_frame_id: &str,
        browser_context: BrowserContextHandle,
        native_dialog: JavaScriptDialogOpened,
    ) -> Self {
        let route = match &native_dialog.opening.source {
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
            browser_context,
            native_dialog: Some(native_dialog),
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
        self.opening().id
    }

    pub(crate) fn source_document(&self) -> RendererDocumentLifecycleIdentity {
        self.opening().source_document
    }

    pub(crate) fn source_url(&self) -> &str {
        &self.opening().source_url
    }

    pub(crate) fn message(&self) -> &str {
        &self.opening().message
    }

    pub(crate) fn dialog_type(&self) -> &str {
        &self.opening().dialog_type
    }

    pub(crate) fn default_prompt(&self) -> &str {
        &self.opening().default_prompt
    }

    pub(crate) fn dismiss(mut self) {
        self.dismiss_inner();
    }

    pub(crate) fn into_native_dialog(mut self) -> JavaScriptDialogOpened {
        self.native_dialog
            .take()
            .expect("prepared dialog must retain its exact native key")
    }

    fn opening(&self) -> &RendererJavaScriptDialogOpening {
        &self
            .native_dialog
            .as_ref()
            .expect("prepared dialog must retain its opening until settlement")
            .opening
    }

    fn dismiss_inner(&mut self) {
        if let Some(dialog) = self.native_dialog.take() {
            self.browser_context
                .dismiss_document_javascript_dialog(dialog.document, dialog.key);
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
    target_document: DocumentHandle,
    document: DocumentHandle,
    pub(in crate::conn::state) key: JavaScriptDialogKey,
}

impl TargetJavaScriptDialog {
    pub(crate) fn new(
        source_frame_id: String,
        target_document: DocumentHandle,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) -> Self {
        debug_assert_eq!(document.id(), key.document);
        Self {
            source_frame_id,
            target_document,
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
    #[cfg(test)]
    pub(crate) fn install_document_javascript_dialog_for_test(
        &mut self,
        document: DocumentHandle,
        dialog: moli_core::page::RendererPendingJavaScriptDialog,
    ) -> Result<Option<JavaScriptDialogKey>, String> {
        self.browser_context
            .install_document_javascript_dialog_for_test(document, dialog)
    }

    pub(crate) fn project_javascript_dialog_for_session(
        &mut self,
        target_id: &str,
        session: &moli_page_types::DevToolsSessionKey,
        source_frame_id: String,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) -> bool {
        let Some(target_document) = self.document_handle_for_target(target_id) else {
            return false;
        };
        if key.document != document.id()
            || document.web_contents().context() != self.browser_context.id()
            || self
                .browser_context
                .ensure_document_current(document)
                .is_err()
        {
            return false;
        }
        if self
            .document_javascript_dialog_snapshot(document, key)
            .is_none()
        {
            // Preserve a historical FIFO opening, without reviving a request
            // already dismissed by native document lifecycle progress.
            return true;
        }
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .ensure_session(session)
            .page_session_state
            .javascript_dialog_state
            .push(TargetJavaScriptDialog::new(
                source_frame_id,
                target_document,
                document,
                key,
            ));
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
        (self.document_handle_for_target(target_id) == Some(dialog.target_document)
            && self
                .browser_context
                .ensure_document_current(dialog.document)
                .is_ok())
        .then_some((dialog.document, dialog.key))
    }

    pub(crate) fn document_javascript_dialog_snapshot(
        &self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) -> Option<JavaScriptDialogSnapshot> {
        self.browser_context
            .document_javascript_dialog_snapshot(document, key)
    }

    pub(crate) fn set_document_javascript_dialog_prompt_text(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
        prompt_text: String,
    ) -> Result<(), JavaScriptDialogError> {
        self.browser_context
            .set_document_javascript_dialog_prompt_text(document, key, prompt_text)
    }

    pub(crate) fn finish_document_javascript_dialog(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<JavaScriptDialogClosed> {
        self.browser_context
            .finish_document_javascript_dialog(document, key, accepted, prompt_text)
    }

    pub(crate) fn dismiss_document_javascript_dialog(
        &mut self,
        document: DocumentHandle,
        key: JavaScriptDialogKey,
    ) {
        self.browser_context
            .dismiss_document_javascript_dialog(document, key);
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
    use super::TargetJavaScriptDialogScope;

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
}
