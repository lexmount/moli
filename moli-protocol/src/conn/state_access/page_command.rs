use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::{CommandDispatchContext, CommandOwnerScope};
use moli_core::page::CompletedPageCommand;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn page_query_host_is_current(
        self,
        host: &crate::conn::ProtocolPageHostScope,
    ) -> bool {
        host.is_current(self.connection)
    }
    pub(crate) fn complete_finite_page_command(
        self,
        completed: crate::domains::page::CompletedPageCommandDispatch,
        context: &mut CommandDispatchContext,
    ) -> crate::domains::page::PageCommandTaskStep {
        crate::domains::page::complete_finite_page_command(self.connection, completed, context)
    }
    pub(crate) fn start_page_isolated_world(
        self,
        command_id: Option<u64>,
        owner: CommandOwnerScope,
        task: crate::domains::page::CreateIsolatedWorldCommandTask,
    ) -> crate::domains::page::PageCommandTaskStep {
        crate::domains::page::start_isolated_world_runtime(self.connection, command_id, owner, task)
    }

    pub(crate) fn finish_page_isolated_world(
        self,
        command_id: Option<u64>,
        owner: CommandOwnerScope,
        task: crate::domains::page::CreateIsolatedWorldCommandTask,
        completed: Result<CompletedPageCommand>,
        context: &mut CommandDispatchContext,
    ) -> Result<
        crate::domains::page::PreparedCreatedIsolatedWorld,
        crate::domains::page::PageCommandTaskStep,
    > {
        crate::domains::page::finish_isolated_world_runtime(
            self.connection,
            command_id,
            owner,
            task,
            completed,
            context,
        )
    }
    pub(crate) fn prepare_page_navigation(
        self,
        completed: crate::domains::page::CompletedNavigateLoadCommand,
    ) -> crate::domains::page::PreparedPageNavigationCompletion {
        completed.prepare_owned(self.connection)
    }

    pub(crate) fn prepare_page_navigation_request(
        self,
        completed: crate::domains::page::CompletedContinueNavigationWithoutRequestPauseCommand,
    ) -> crate::domains::page::PreparedPageNavigationCompletion {
        completed.prepare_owned(self.connection)
    }

    pub(crate) fn finish_page_stop_lifecycle(
        self,
        owner: crate::conn::TargetPageResidenceIdentity,
        completed: CompletedPageCommand,
        context: &mut CommandDispatchContext,
    ) -> Result<()> {
        let output = self
            .connection
            .settle_page_command_turn_for_owner(&owner, completed);
        let completion = context.consume_renderer_command_turn_output(output);
        match completion.reply() {
            moli_core::RendererPageReply::Unit => Ok(()),
            _ => Err(
                crate::domains::page::PageCommandCompletionError::UnexpectedStopLifecycleReply
                    .into(),
            ),
        }
    }

    pub(crate) fn finish_page_crash_events(
        self,
        action: crate::domains::page::PageTargetTerminationOwnerAction,
        events: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        crate::domains::page::PreparedPageCrash::finish_events(self.connection, action, events)
    }

    pub(crate) fn finish_page_activation(
        self,
        output: crate::domains::target::activation::TargetActivationOutput,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        output.finish(self.connection)
    }

    pub(crate) fn finish_page_preload_runtime_activity(
        self,
        page: crate::conn::TargetPageResidenceIdentity,
        completed: CompletedPageCommand,
        context: &mut CommandDispatchContext,
    ) -> Result<Option<(i64, bool)>> {
        crate::domains::page::PreparedPageAddPreload::finish_runtime_activity(
            self.connection,
            &page,
            completed,
            context,
        )
    }

    pub(crate) fn prepare_page_preload_listeners(
        self,
        owner: CommandOwnerScope,
        page: Option<crate::conn::TargetPageResidenceIdentity>,
        execution_context_id: i64,
    ) -> Option<crate::domains::runtime::PreparedBidiPreloadChannelListeners> {
        if self
            .connection
            .target_page_residence_identity_for_owner(&owner)
            != page
        {
            return None;
        }
        crate::domains::runtime::PreparedBidiPreloadChannelListeners::prepare(
            self.connection,
            &owner,
            execution_context_id,
        )
    }

    pub(crate) fn prepare_page_bring_to_front(
        self,
        route: crate::conn::CdpSessionRoute,
    ) -> Result<
        Option<crate::domains::target::activation::PreparedTargetActivation>,
        crate::automation::DevToolsError,
    > {
        crate::domains::page::prepare_page_bring_to_front(self.connection, route)
    }
}

#[cfg(test)]
mod tests;
