use anyhow::Result;
use moli_page_types::RendererInspectorResponseDelivery;
use moli_protocol_cdp::CdpCommandParseError;

use super::{ProtocolWorkflowAccess, ProtocolWorkflowDriver, ProtocolWorkflowState};
use crate::conn::{
    CdpCommandTaskStep, CdpConnection, CdpRendererOwnerTurnOutcome, CommandDispatchContext,
    CommandFailureCleanup, CommandOwnerScope, CommandResponseFlushContext, CommandWorkflowContext,
    CompletedCdpCommandDispatch, CompletedCommandWorkflow, ParsedCdpCommand,
    PendingCdpCommandDispatch, command_workflow::CommandWorkflowContextReturn,
};
use crate::domains::command_output::CommandOutputPlan;

#[derive(Debug, thiserror::Error)]
#[error("{0} command completion still requires exclusive protocol state")]
struct ExclusiveCommandCompletion(&'static str);

/// A command admitted in its original state turn. Its renderer call is already
/// registered before an asynchronous task can be polled or a peer can run.
pub(crate) struct PreparedProtocolCommand {
    pending: Option<PendingCdpCommandDispatch>,
    context: CommandWorkflowContext,
    recovery: CommandWorkflowContextReturn,
    owner: CommandOwnerScope,
}

impl PreparedProtocolCommand {
    pub(crate) fn new(
        conn: &mut CdpConnection,
        command: Result<ParsedCdpCommand, CdpCommandParseError>,
        delivery: Option<RendererInspectorResponseDelivery>,
        response_flush: CommandResponseFlushContext,
    ) -> Self {
        let command_id = match command.as_ref() {
            Ok(command) => Some(command.request().id()),
            Err(error) => error.command_id(),
        };
        let session_id = command
            .as_ref()
            .ok()
            .and_then(ParsedCdpCommand::command_output_session_id)
            .map(str::to_owned);
        let owner = CommandOwnerScope::capture(conn, session_id.as_deref());
        let mut dispatch = CommandDispatchContext::new(response_flush);
        if let Some(delivery) = delivery {
            dispatch.set_terminal_response_delivery_override(delivery);
        }
        let (mut context, recovery) =
            CommandWorkflowContext::take_from(&mut dispatch, command_id, session_id.clone());
        let admitted =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match command.as_ref() {
                Ok(command) => {
                    conn.start_parsed_command_dispatch_with_context(command, context.context_mut())
                }
                Err(error) => conn.complete_with_output_plan(
                    context.context_mut(),
                    CommandOutputPlan::error_without_session(
                        error.response_code(),
                        error.response_message(),
                    ),
                    error.command_id(),
                    None,
                ),
            }));
        let step = match admitted {
            Ok(step) => step,
            Err(_) => {
                let response =
                    CommandOutputPlan::error(-32000, crate::OwnedProtocolWaitFailure::Panicked);
                match context.context_mut().take_command_failure_cleanup() {
                    Some(cleanup) => conn.recover_command_output_cleanup(
                        cleanup,
                        response,
                        command_id,
                        session_id.as_deref(),
                        context.context_mut(),
                    ),
                    None => conn.complete_with_output_plan(
                        context.context_mut(),
                        response,
                        command_id,
                        session_id.as_deref(),
                    ),
                }
            }
        };
        Self {
            pending: retain_step(conn, &mut context, step),
            context,
            recovery,
            owner,
        }
    }

    pub(crate) fn recovery_scope(&self) -> CommandWorkflowContextReturn {
        self.recovery.clone()
    }

    pub(crate) fn output_owner(&self) -> CommandOwnerScope {
        self.owner.clone()
    }

    pub(crate) fn wait_lifetime(&self) -> Option<crate::conn::CommandWaitLifetime> {
        self.pending
            .as_ref()
            .and_then(PendingCdpCommandDispatch::wait_lifetime)
    }

    pub(crate) fn recover(
        conn: &mut CdpConnection,
        recovery: CommandWorkflowContextReturn,
        owner: CommandOwnerScope,
        response: CommandOutputPlan,
        response_flush: CommandResponseFlushContext,
    ) -> Self {
        let command_id = recovery.command_id();
        let session_id = recovery.session_id().map(str::to_owned);
        let mut dispatch = CommandDispatchContext::new(response_flush);
        let (mut context, scope) =
            CommandWorkflowContext::take_from(&mut dispatch, command_id, session_id.clone());
        let step = conn.recover_command_output_cleanup(
            CommandFailureCleanup::Workflow(recovery),
            response,
            command_id,
            session_id.as_deref(),
            context.context_mut(),
        );
        Self {
            pending: retain_step(conn, &mut context, step),
            context,
            recovery: scope,
            owner,
        }
    }

    pub(crate) async fn run_using(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> Result<CompletedCommandWorkflow> {
        while let Some(pending) = self.pending.take() {
            let completed = Box::pin(pending.wait()).await;
            self.pending = driver
                .complete_protocol_command(completed, &mut self.context)
                .await?;
        }
        let owner = self.owner;
        self.context
            .with_state(driver, move |state, context| {
                crate::domains::activity::project_protocol_local_command_outputs(
                    state.connection,
                    &owner,
                    context.context_mut(),
                );
            })
            .await?;
        Ok(self.context.complete(CommandOutputPlan::default()))
    }
}

fn retain_step(
    conn: &mut CdpConnection,
    context: &mut CommandWorkflowContext,
    step: CdpCommandTaskStep,
) -> Option<PendingCdpCommandDispatch> {
    match step {
        CdpCommandTaskStep::Pending(pending) => Some(*pending),
        CdpCommandTaskStep::Complete(outcome) => {
            let (events, after, boundary, post_response, scheduler_events, predecessor) =
                outcome.into_renderer_owner_turn_parts();
            let mut output = CommandOutputPlan::default();
            output.extend_background_events(events);
            if let Some(boundary) = boundary {
                output.insert_renderer_output_boundary(boundary);
            } else {
                assert!(
                    after.is_empty(),
                    "post-renderer output requires its boundary"
                );
            }
            output.extend_background_events(after);
            output.extend_background_events(post_response);
            if let Some(predecessor) = predecessor {
                output.set_renderer_output_predecessor(predecessor);
            }
            // Neither a discarded state reply nor its caller's recovery may
            // replace this already accepted terminal or lose its exact fence.
            context.retain_finalized_command(output);
            conn.extend_scheduler_events(scheduler_events);
            None
        }
    }
}

impl ProtocolWorkflowDriver<'_> {
    async fn complete_protocol_command(
        &mut self,
        completed: CompletedCdpCommandDispatch,
        context: &mut CommandWorkflowContext,
    ) -> Result<Option<PendingCdpCommandDispatch>> {
        match &mut self.access {
            ProtocolWorkflowAccess::Inline(conn) => {
                let step = Box::pin(conn.complete_pending_command_dispatch_with_context(
                    completed,
                    context.context_mut(),
                ))
                .await;
                Ok(retain_step(conn, context, step))
            }
            ProtocolWorkflowAccess::Owned(_) => {
                context
                    .with_state(self, move |state, context| {
                        match state
                            .connection
                            .try_complete_ready_command_dispatch_with_context(
                                completed,
                                context.context_mut(),
                            ) {
                            Ok(step) => Ok(retain_step(state.connection, context, step)),
                            Err(completed) => {
                                Err(ExclusiveCommandCompletion(completed.kind_name()))
                            }
                        }
                    })
                    .await?
            }
        }
        .map_err(Into::into)
    }
}

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_target_message(
        self,
        completed: CompletedCommandWorkflow,
        session_id: Option<String>,
        strip_implicit_session: bool,
        response: CommandOutputPlan,
        context: &mut CommandWorkflowContext,
    ) {
        let outcome = finish_command(self.connection, completed, Some(context.context_mut()));
        let output = crate::domains::target::retain_target_message_output(
            self.connection,
            outcome,
            session_id,
            strip_implicit_session,
            response,
        );
        context.retain_terminal(output);
        // The inner command and its cleanup are consumed in this operation.
        // Losing its ready reply must not try to recover that consumed scope.
        context.context_mut().take_command_failure_cleanup();
    }

    pub(crate) fn recover_protocol_command(
        self,
        recovery: CommandWorkflowContextReturn,
        owner: CommandOwnerScope,
        response: CommandOutputPlan,
        response_flush: CommandResponseFlushContext,
    ) -> PreparedProtocolCommand {
        PreparedProtocolCommand::recover(self.connection, recovery, owner, response, response_flush)
    }

    pub(crate) fn finish_protocol_command(
        self,
        completed: CompletedCommandWorkflow,
    ) -> CdpRendererOwnerTurnOutcome {
        finish_command(self.connection, completed, None)
    }
}

fn finish_command(
    connection: &mut CdpConnection,
    completed: CompletedCommandWorkflow,
    parent: Option<&mut CommandDispatchContext>,
) -> CdpRendererOwnerTurnOutcome {
    let command_id = completed.command_id;
    let session_id = completed.session_id.clone();
    let mut context = CommandDispatchContext::default();
    let output = completed.finish(connection, &mut context);
    let CdpCommandTaskStep::Complete(outcome) = connection.complete_with_output_plan(
        &mut context,
        output,
        command_id,
        session_id.as_deref(),
    ) else {
        unreachable!("a completed command has no external wait");
    };
    if let Some(parent) = parent {
        for permit in context.take_runtime_output_projection_permits() {
            parent.retain_runtime_output_projection_permit(permit);
        }
    }
    outcome
}
