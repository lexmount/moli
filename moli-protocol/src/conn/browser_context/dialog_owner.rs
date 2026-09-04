use moli_core::page::RendererPendingJavaScriptDialog;

use crate::conn::{
    CdpConnection, CommandOwnerScope, TargetPageResidenceIdentity,
    state::{JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogSnapshot},
};

// Exact routing remains in DevTools; the value-only Browser calls replace this
// in-place bridge at Commit 22. No renderer dispatch lane or Page borrow here.
impl CdpConnection {
    pub(crate) fn install_javascript_dialog_for_session(
        &mut self,
        session_id: Option<&str>,
        page_owner: TargetPageResidenceIdentity,
        source_frame_id: String,
        dialog: RendererPendingJavaScriptDialog,
    ) -> bool {
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            let _ = dialog.finish(false, String::new());
            return false;
        };
        owner.mutate_page_state(|target, session| {
            target.install_javascript_dialog(session, page_owner, source_frame_id, dialog)
        })
    }

    pub(crate) fn javascript_dialog_snapshot_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<JavaScriptDialogSnapshot> {
        let owner = self.target_session_owner_ref_for_owner(owner)?;
        owner
            .browser_context
            .page_target(&owner.target_id)?
            .javascript_dialog_snapshot(&owner.session_key)
    }

    pub(crate) fn set_javascript_dialog_prompt_text_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        prompt_text: String,
    ) -> Result<(), JavaScriptDialogError> {
        self.target_session_owner_mut_for_owner(owner)
            .ok_or(JavaScriptDialogError::NotFound)?
            .mutate_page_state(|target, session| {
                target.set_javascript_dialog_prompt_text(session, prompt_text)
            })
    }

    pub(crate) fn handle_javascript_dialog_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<(String, JavaScriptDialogClosed)> {
        self.target_session_owner_mut_for_owner(owner)?
            .mutate_page_state(|target, session| {
                target.handle_javascript_dialog(session, accepted, prompt_text)
            })
    }
}
