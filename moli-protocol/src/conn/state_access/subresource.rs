use anyhow::Result;
use moli_core::page::CompletedPageCommand;

use super::ProtocolWorkflowState;
use crate::domains::fetch::SubresourceAutoContinueCompletion;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_subresource_auto_continue(
        self,
        completion: SubresourceAutoContinueCompletion,
        completed: Result<CompletedPageCommand>,
    ) {
        completion.finish(self.connection, completed);
    }
}

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_subresource_continue_action(
        self,
        owner: crate::conn::CommandOwnerScope,
        action: crate::domains::activity::PreparedSubresourceContinueAction,
    ) -> (
        Vec<crate::conn::BackgroundProtocolEvent>,
        Option<crate::domains::activity::PreparedSubresourceContinueWait>,
    ) {
        let mut events = Vec::new();
        let pending = crate::domains::activity::prepare_subresource_continue_action(
            self.connection,
            &mut events,
            &owner,
            action,
        );
        (events, pending)
    }

    pub(crate) fn finish_subresource_continue_action(
        self,
        completion: crate::domains::activity::SubresourceContinueCompletion,
        completed: Result<CompletedPageCommand>,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        completion.finish(self.connection, completed)
    }
}
