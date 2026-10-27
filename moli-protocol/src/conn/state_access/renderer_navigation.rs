use super::ProtocolWorkflowState;
use crate::conn::{
    CommandOwnerScope, CommandWorkflowContext, DocumentNavigationToken, NavigationDispatchState,
    NavigationLoadOutcome, TargetRootDocumentProtocolAttachmentIdentity,
};
use crate::domains::page::PreparedRendererNavigationResult;
use anyhow::{Context as _, Result};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_renderer_navigation_result(
        self,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        response: Result<NavigationLoadOutcome>,
        context: &mut CommandWorkflowContext,
    ) -> PreparedRendererNavigationResult {
        PreparedRendererNavigationResult::prepare(self.connection, token, state, response, context)
    }

    pub(crate) fn finish_renderer_navigation_result(
        self,
        owner: CommandOwnerScope,
        loader_id: String,
    ) {
        self.connection
            .clear_pending_document_navigation_for_owner_if_loader_matches(&owner, &loader_id);
    }

    pub(crate) fn finish_renderer_same_document_navigation(
        self,
        owner: CommandOwnerScope,
        binding: Option<TargetRootDocumentProtocolAttachmentIdentity>,
        completed: moli_core::page::CompletedPageCommand,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let output = if binding.as_ref().is_some_and(|binding| {
            self.connection
                .target_root_document_protocol_attachment_identity_is_current(binding)
        }) {
            let (_, output) = self
                .connection
                .loaded_page_mut_for_owner(&owner)
                .context(crate::conn::NoDocumentLoaded)?
                .finish_top_level_same_document_navigation_command_turn(completed)?;
            output
        } else {
            completed.into_output()
        };
        context
            .context_mut()
            .consume_renderer_command_turn_output(output);
        Ok(())
    }
}
