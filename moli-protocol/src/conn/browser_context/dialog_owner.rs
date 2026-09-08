use moli_core::browser::JavaScriptDialogOpened;

use crate::conn::{
    CdpConnection, CommandOwnerScope, TargetPageResidenceIdentity,
    state::{JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogSnapshot},
};

// DevTools resolves its projection once; Browser calls below receive only the
// exact physical Document capability and neutral dialog key.
impl CdpConnection {
    #[cfg(test)]
    pub(crate) fn admit_javascript_dialog_for_test(
        &self,
        page_owner: &TargetPageResidenceIdentity,
        dialog: moli_core::page::RendererPendingJavaScriptDialog,
    ) -> (
        moli_core::browser::BrowserContextHandle,
        JavaScriptDialogOpened,
    ) {
        let document = self
            .browser_context_by_id(page_owner.browser_context_id())
            .and_then(|context| context.document_handle_for_target(page_owner.target_id().unwrap()))
            .filter(|document| document.id() == page_owner.document_id())
            .expect("test source Document");
        let context = self
            .browser
            .context_handle(document.web_contents().context())
            .unwrap();
        let opening = dialog.opening();
        let key = moli_core::browser::web_contents::JavaScriptDialogKey::new(
            document.id(),
            opening.source_document,
            opening.id,
        );
        context
            .install_document_javascript_dialog_for_test(document, dialog)
            .unwrap();
        (
            context,
            JavaScriptDialogOpened {
                document,
                key,
                opening,
            },
        )
    }

    pub(crate) fn project_javascript_dialog_for_session(
        &mut self,
        session_id: Option<&str>,
        page_owner: TargetPageResidenceIdentity,
        source_frame_id: String,
        dialog: JavaScriptDialogOpened,
    ) -> bool {
        let scope = match session_id {
            Some(session) => CommandOwnerScope::for_session(session),
            None => CommandOwnerScope::for_page_residence(&page_owner),
        };
        let Some(owner) = self.target_session_owner_mut_for_owner(&scope) else {
            if let Ok(context) = self
                .browser
                .context_handle(dialog.document.web_contents().context())
            {
                context.dismiss_document_javascript_dialog(dialog.document, dialog.key);
            }
            return false;
        };
        let Some(_) = owner
            .browser_context
            .document_handle_for_target(&owner.target_id)
            .filter(|document| document.id() == page_owner.document_id())
        else {
            owner
                .browser_context
                .dismiss_document_javascript_dialog(dialog.document, dialog.key);
            return false;
        };
        let document = dialog.document;
        let key = dialog.key;
        if owner.browser_context.project_javascript_dialog_for_session(
            &owner.target_id,
            &owner.session_key,
            source_frame_id,
            document,
            key,
        ) {
            true
        } else {
            owner
                .browser_context
                .dismiss_document_javascript_dialog(document, key);
            false
        }
    }

    pub(crate) fn javascript_dialog_snapshot_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<JavaScriptDialogSnapshot> {
        let owner = self.target_session_owner_ref_for_owner(owner)?;
        let (document, key) = owner
            .browser_context
            .projected_javascript_dialog_for_session(&owner.target_id, &owner.session_key)?;
        owner
            .browser_context
            .document_javascript_dialog_snapshot(document, key)
    }

    pub(crate) fn set_javascript_dialog_prompt_text_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        prompt_text: String,
    ) -> Result<(), JavaScriptDialogError> {
        let owner = self
            .target_session_owner_mut_for_owner(owner)
            .ok_or(JavaScriptDialogError::NotFound)?;
        let (document, key) = owner
            .browser_context
            .projected_javascript_dialog_for_session(&owner.target_id, &owner.session_key)
            .ok_or(JavaScriptDialogError::NotFound)?;
        owner
            .browser_context
            .set_document_javascript_dialog_prompt_text(document, key, prompt_text)
    }

    pub(crate) fn handle_javascript_dialog_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<(String, JavaScriptDialogClosed)> {
        let owner = self.target_session_owner_mut_for_owner(owner)?;
        let Some((document, key)) = owner
            .browser_context
            .projected_javascript_dialog_for_session(&owner.target_id, &owner.session_key)
        else {
            let projections = owner
                .browser_context
                .take_projected_javascript_dialogs_for_session(
                    &owner.target_id,
                    &owner.session_key,
                );
            owner
                .browser_context
                .dismiss_projected_javascript_dialogs(projections);
            return None;
        };
        let projection = owner
            .browser_context
            .pop_projected_javascript_dialog_for_session(&owner.target_id, &owner.session_key)?;
        let (source_frame_id, projected_document, projected_key) = projection.into_parts();
        debug_assert_eq!(projected_document, document);
        debug_assert_eq!(projected_key, key);
        let outcome = owner.browser_context.finish_document_javascript_dialog(
            document,
            key,
            accepted,
            prompt_text,
        )?;
        Some((source_frame_id, outcome))
    }
}
