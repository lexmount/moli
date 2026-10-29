mod dispatch;
mod workflow;
pub(crate) use dispatch::AutomationCommandResult;
pub use dispatch::{
    AutomationCommandTaskStep, CompletedAutomationCommandDispatch, PendingAutomationCommandDispatch,
};

use crate::automation::{
    AutomationCommand, AutomationContext, AutomationResult, DevToolsError, DevToolsErrorKind,
};

use super::*;

pub struct AutomationCommandDispatchOutcome {
    result: Result<AutomationResult, DevToolsError>,
    output: crate::conn::CdpRendererOwnerTurnOutcome,
}

impl AutomationCommandDispatchOutcome {
    pub(crate) fn new_with_protocol_events(
        result: Result<AutomationResult, DevToolsError>,
        scheduler_events: Vec<CdpSchedulerEvent>,
        protocol_events: Vec<BackgroundProtocolEvent>,
    ) -> Self {
        Self {
            result,
            output: crate::conn::CdpTurnOutcome::new_with_protocol_events(
                protocol_events,
                scheduler_events,
            )
            .with_renderer_output_predecessor(None),
        }
    }

    pub(crate) fn with_renderer_output_predecessor(
        mut self,
        predecessor: Option<moli_core::RendererOutputFence>,
    ) -> Self {
        let (before, after, boundary, post, scheduler, _) =
            self.output.into_renderer_owner_turn_parts();
        self.output = crate::conn::CdpTurnOutcome::new_with_protocol_and_post_response_events(
            before, post, scheduler,
        )
        .with_renderer_output_boundary(boundary, after)
        .with_renderer_output_predecessor(predecessor);
        self
    }

    pub fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        self.output.renderer_output_predecessor()
    }

    pub fn into_result_and_owner_turn(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        crate::conn::CdpRendererOwnerTurnOutcome,
    ) {
        (self.result, self.output)
    }

    /// Consumes a command without protocol observations or a renderer fence.
    #[track_caller]
    pub fn into_parts(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
    ) {
        let (result, scheduler, events, predecessor) = self.into_complete_parts();
        assert!(
            events.is_empty(),
            "command output with protocol events requires complete consumption"
        );
        assert!(
            predecessor.is_none(),
            "command output retains its exact renderer cursor"
        );
        (result, scheduler)
    }

    /// Consumes a command without a renderer response fence.
    #[track_caller]
    pub fn into_parts_with_protocol_events(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
        Vec<BackgroundProtocolEvent>,
    ) {
        let (result, scheduler, events, predecessor) = self.into_complete_parts();
        assert!(
            predecessor.is_none(),
            "command output retains its exact renderer cursor"
        );
        (result, scheduler, events)
    }

    /// Commands with an independent renderer boundary must consume the typed
    /// owner turn to preserve the insertion point between their output pieces.
    #[track_caller]
    pub fn into_complete_parts(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
    ) {
        let (mut before, after, boundary, post, scheduler, predecessor) =
            self.output.into_renderer_owner_turn_parts();
        assert!(
            boundary.is_none(),
            "an automation consumer must project its independent renderer boundary"
        );
        assert!(after.is_empty());
        before.extend(post);
        (self.result, scheduler, before, predecessor)
    }
}

impl CdpConnection {
    /// Finite commands finish in the admission turn. Commands with renderer
    /// or network waits remain owned by the common automation workflow.
    pub fn try_execute_immediate_automation_command(
        &mut self,
        command: AutomationCommand,
    ) -> Result<AutomationCommandDispatchOutcome, Box<AutomationCommand>> {
        let context = command.context().clone();
        let (result, events) = match command {
            command @ (AutomationCommand::GetTargets(_)
            | AutomationCommand::GetServiceWorkerLogs(_)
            | AutomationCommand::GetClientWindows(_)
            | AutomationCommand::CreateBrowserContext(_)
            | AutomationCommand::GetBrowserContexts(_)
            | AutomationCommand::GetTargetInfo(_)) => {
                crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(self, command)
            }
            command @ AutomationCommand::SetDownloadBehavior(_) => (
                crate::domains::browser::execute_devtools_browser_command(self, command),
                Vec::new(),
            ),
            command @ (AutomationCommand::PrintToPdf(_)
            | AutomationCommand::GetJavaScriptDialog(_)
            | AutomationCommand::SetJavaScriptDialogPromptText(_)
            | AutomationCommand::HandleJavaScriptDialog(_)
            | AutomationCommand::GetNavigationHistory(_)) => {
                crate::domains::page::execute_immediate_devtools_page_command_with_protocol_events(self, command)
            }
            command @ (AutomationCommand::AddNetworkDataCollector(_)
            | AutomationCommand::RemoveNetworkDataCollector(_)
            | AutomationCommand::SetCacheBehavior(_)) => (
                crate::domains::network::execute_devtools_network_command(self, command),
                Vec::new(),
            ),
            command => return Err(Box::new(command)),
        };
        Ok(self.finish_automation_command_dispatch(context, result, events, None))
    }

    pub(crate) fn finish_automation_command_dispatch(
        &mut self,
        command_context: AutomationContext,
        result: Result<AutomationResult, DevToolsError>,
        mut protocol_events: Vec<BackgroundProtocolEvent>,
        renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    ) -> AutomationCommandDispatchOutcome {
        let mut dispatch_context = CommandDispatchContext::default();
        self.project_protocol_local_outputs_for_direct_command(
            &command_context,
            &mut dispatch_context,
            &mut protocol_events,
        );
        protocol_events.extend(dispatch_context.take_post_response_events());
        let scheduler_events = self.take_scheduler_events();
        AutomationCommandDispatchOutcome::new_with_protocol_events(
            result,
            scheduler_events,
            protocol_events,
        )
        .with_renderer_output_predecessor(renderer_output_predecessor)
    }

    pub(super) fn project_protocol_local_outputs_for_direct_command(
        &mut self,
        context: &AutomationContext,
        dispatch_context: &mut CommandDispatchContext,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        if self.has_pending_javascript_dialog() {
            return;
        }
        let session_id = context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str());
        let owner = match context.target_id.as_ref() {
            Some(target_id) => {
                let Some(route) = self.target_session_route_for_target_id(target_id.as_str())
                else {
                    return;
                };
                CommandOwnerScope::for_route(route)
            }
            None => CommandOwnerScope::capture(self, session_id),
        };
        crate::domains::activity::project_protocol_local_command_outputs(
            self,
            &owner,
            dispatch_context,
        );
        protocol_events.extend(dispatch_context.take_protocol_events());
    }
}
