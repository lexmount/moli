use moli_core::page::{
    RendererDocumentLifecycleIdentity, RendererJavaScriptDialogId, RendererJavaScriptDialogSource,
    RendererPendingJavaScriptDialog,
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
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

    pub(crate) fn into_target_dialog(
        mut self,
        destination_page_owner: TargetPageResidenceIdentity,
        source_frame_id: String,
    ) -> TargetJavaScriptDialog {
        TargetJavaScriptDialog::new(
            destination_page_owner,
            source_frame_id,
            self.renderer_dialog
                .take()
                .expect("prepared dialog must own its renderer payload"),
        )
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

/// One dialog installed for a concrete protocol Page residence.
///
/// The renderer payload retains the causal Document, source URL, popup/frame
/// identity and one-shot completion. `source_frame_id` is resolved exactly
/// once during capture, so command handling and event projection never fall
/// back to whichever frame happens to be current later. Prompt text belongs
/// to this same residence; it cannot drift into a parallel queue.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TargetJavaScriptDialog {
    page_owner: TargetPageResidenceIdentity,
    source_frame_id: String,
    renderer_dialog: RendererPendingJavaScriptDialog,
    pending_prompt_text: Option<String>,
}

impl TargetJavaScriptDialog {
    pub(crate) fn new(
        page_owner: TargetPageResidenceIdentity,
        source_frame_id: String,
        renderer_dialog: RendererPendingJavaScriptDialog,
    ) -> Self {
        Self {
            page_owner,
            source_frame_id,
            renderer_dialog,
            pending_prompt_text: None,
        }
    }

    pub(crate) fn page_owner(&self) -> &TargetPageResidenceIdentity {
        &self.page_owner
    }

    pub(crate) fn source_frame_id(&self) -> &str {
        &self.source_frame_id
    }

    pub(crate) fn dialog_type(&self) -> &str {
        self.renderer_dialog.dialog_type()
    }

    pub(crate) fn message(&self) -> &str {
        self.renderer_dialog.message()
    }

    pub(crate) fn default_prompt(&self) -> &str {
        self.renderer_dialog.default_prompt()
    }

    pub(crate) fn finish(&self, accepted: bool, user_input: String) -> bool {
        self.renderer_dialog.finish(accepted, user_input)
    }

    fn set_prompt_text(&mut self, prompt_text: String) {
        self.pending_prompt_text = Some(prompt_text);
    }

    fn into_prompt_text(mut self) -> (Self, Option<String>) {
        let prompt_text = self.pending_prompt_text.take();
        (self, prompt_text)
    }
}

/// Installed modal-dialog state for one protocol attachment and Page lifetime.
///
/// Clearing dismisses every installed renderer completion. In-flight prepared
/// output is authorized separately by the stable Page scope in
/// `TargetRuntimeSlot`; keeping that authority out of this state lets an
/// otherwise-default parked session fold away without losing Page identity.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TargetJavaScriptDialogState {
    pending_dialogs: Vec<TargetJavaScriptDialog>,
}

impl TargetJavaScriptDialogState {
    pub(crate) fn clear(&mut self) {
        for dialog in self.pending_dialogs.drain(..) {
            let _ = dialog.finish(false, String::new());
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending_dialogs.is_empty()
    }

    pub(crate) fn push(&mut self, dialog: TargetJavaScriptDialog) {
        self.pending_dialogs.push(dialog);
    }

    pub(crate) fn peek_next(&self) -> Option<&TargetJavaScriptDialog> {
        self.pending_dialogs.first()
    }

    pub(crate) fn set_next_prompt_text(&mut self, prompt_text: String) -> bool {
        let Some(dialog) = self.pending_dialogs.first_mut() else {
            return false;
        };
        dialog.set_prompt_text(prompt_text);
        true
    }

    pub(crate) fn pop_next_with_prompt_text(
        &mut self,
    ) -> Option<(TargetJavaScriptDialog, Option<String>)> {
        if self.pending_dialogs.is_empty() {
            return None;
        }
        Some(self.pending_dialogs.remove(0).into_prompt_text())
    }

    #[cfg(test)]
    pub(crate) fn pending_dialogs(&self) -> &[TargetJavaScriptDialog] {
        &self.pending_dialogs
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
