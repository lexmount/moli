use super::ProtocolWorkflowState;
use crate::conn::{
    CommandOwnerScope, CommandWorkflowContext, DocumentNavigationToken, NavigationDispatchState,
    NavigationLoadOutcome, TargetRootDocumentProtocolAttachmentIdentity,
};
use crate::domains::page::PreparedRendererNavigationResult;
use anyhow::{Context as _, Result};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_superseded_fetch_navigation(
        self,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        context: &mut CommandWorkflowContext,
    ) -> crate::conn::PreparedRendererNavigationFinish {
        let mut output = crate::domains::command_output::CommandOutputPlan::default();
        output.extend_background_events(
            crate::domains::page::superseded_intercepted_navigation_events(self.connection, &state),
        );
        context.retain_output(output);
        self.connection
            .prepare_renderer_navigation_finish(&state.owner, &token)
    }

    pub(crate) fn prepare_renderer_navigation_result_with_output(
        self,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        response: Result<NavigationLoadOutcome>,
        context: &mut CommandWorkflowContext,
        output: crate::domains::page::NavigationCommandOutput,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> PreparedRendererNavigationResult {
        PreparedRendererNavigationResult::prepare_with_output(
            self.connection,
            token,
            state,
            response,
            context,
            output,
            prefix,
        )
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
