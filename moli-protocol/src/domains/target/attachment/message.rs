use anyhow::Result;
use moli_page_types::RendererInspectorResponseDelivery;

use super::{SendMessageParams, *};
use crate::conn::{
    CdpRendererOwnerTurnOutcome, CdpSessionRoute, CommandDispatchContext, CommandFailureCleanup,
    CommandOwnerScope, CommandWorkflowContext, CommandWorkflowContextReturn,
    CommandWorkflowFailureCleanup, ParsedCdpCommand, PreparedCommandWorkflow,
    PreparedProtocolCommand, ProtocolStateAccess, ProtocolWorkflowDriver, SessionDisposalWait,
};

#[derive(Debug, thiserror::Error)]
#[error("Nested target session closed")]
struct NestedTargetSessionClosed;

pub(in crate::domains::target) struct PreparedTargetMessage {
    command: PreparedProtocolCommand,
    session_id: Option<String>,
    strip_implicit_session: bool,
    closed: Option<SessionDisposalWait>,
}

pub(in crate::domains::target) fn start_send_message_to_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    context: &CommandDispatchContext,
) -> TargetCommandTaskStep {
    let params: SendMessageParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return super::super::target_command_error(-32602, "InvalidParams"),
    };
    let prepared = match PreparedTargetMessage::prepare(conn, params, context) {
        Ok(prepared) => prepared,
        Err(response) => return TargetCommandTaskStep::Complete(response),
    };
    match conn.protocol_state_access() {
        Some(access) => TargetCommandTaskStep::Workflow(prepared.into_workflow(access)),
        None => TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: Box::new(PendingTargetCommandKind::SendMessageToTarget(Box::new(
                prepared,
            ))),
        }),
    }
}

impl PreparedTargetMessage {
    fn prepare(
        conn: &mut CdpConnection,
        params: SendMessageParams,
        context: &CommandDispatchContext,
    ) -> Result<Self, CommandOutputPlan> {
        let mut command = ParsedCdpCommand::parse_str(params.message);
        let mut strip_implicit_session = false;
        if let Some(session_id) = params.session_id.as_deref() {
            let Some(route) = conn.session_route(Some(session_id)) else {
                return Err(CommandOutputPlan::error(-31998, "InvalidSessionId"));
            };
            if !matches!(
                route,
                CdpSessionRoute::PageTarget { .. } | CdpSessionRoute::BrowserContext { .. }
            ) {
                return Err(CommandOutputPlan::error(-31998, "InvalidSessionId"));
            }
            if conn.session_is_closing(session_id) {
                return Err(CommandOutputPlan::error(-32001, "Session is closing"));
            }
            if let Ok(parsed) = command.as_ref()
                && parsed.request().session_id().is_none()
            {
                // Preserve the legacy adapter's browser-only Target operations.
                // Page operations use the actual attachment rather than its
                // target's primary session or whichever Page is selected.
                let browser_operation = parsed
                    .method()
                    .strip_prefix("Target.")
                    .and_then(|action| action.parse::<TargetAction>().ok())
                    .is_some_and(super::super::target_action_requires_browser_access);
                if !browser_operation {
                    let parsed = command.expect("validated command");
                    let id = parsed.request().id();
                    command = Ok(parsed
                        .rewrite_frontend_route(id, Some(session_id))
                        .map_err(|error| CommandOutputPlan::error(-32603, error))?);
                    strip_implicit_session = true;
                }
            }
        } else {
            let Some(browser) = conn.browser_context.as_ref() else {
                return Err(CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"));
            };
            let Some((_, session_id)) = browser.active_target_identity() else {
                return Err(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
            };
            if session_id.is_some() {
                return Err(CommandOutputPlan::error(-31998, "InvalidSessionId"));
            }
        }
        let closed = params
            .session_id
            .as_deref()
            .and_then(|id| conn.session_binding_lifetime(id));
        let previous = super::super::previously_active_browser_context_id(conn);
        if let Some(session_id) = params.session_id.as_deref()
            && !conn.activate_browser_context_for_session(session_id)
        {
            return Err(CommandOutputPlan::error(-31998, "InvalidSessionId"));
        }
        let command = PreparedProtocolCommand::new(
            conn,
            command,
            Some(RendererInspectorResponseDelivery::AdapterReply),
            context.response_flush().clone(),
        );
        super::super::restore_previously_active_browser_context(conn, previous.as_deref());
        Ok(Self {
            command,
            session_id: params.session_id,
            strip_implicit_session,
            closed,
        })
    }

    fn into_workflow(self, access: ProtocolStateAccess) -> PreparedCommandWorkflow {
        let wait_lifetime = self.command.wait_lifetime();
        let recovery = self.command.recovery_scope();
        let owner = self.command.output_owner();
        let session_id = self.session_id.clone();
        let strip = self.strip_implicit_session;
        let failure_access = access.clone();
        PreparedCommandWorkflow::new(move |mut context| async move {
            match self
                .run_using(&mut ProtocolWorkflowDriver::owned(access), &mut context)
                .await
            {
                Ok(()) => context.complete(CommandOutputPlan::default()),
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
        .cancel_on_waiter_drop()
        .with_start_terminal(CommandOutputPlan::success())
        .with_wait_lifetime(wait_lifetime)
        .with_failure_cleanup(move |response| {
            recovery_workflow(failure_access, recovery, owner, session_id, strip, response)
        })
    }

    async fn run_using(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let completed = if let Some(closed) = self.closed {
            tokio::select! {
                result = Box::pin(self.command.run_using(driver)) => result?,
                () = closed.wait() => return Err(NestedTargetSessionClosed.into()),
            }
        } else {
            Box::pin(self.command.run_using(driver)).await?
        };
        context
            .with_state(driver, move |state, context| {
                state.finish_target_message(
                    completed,
                    self.session_id,
                    self.strip_implicit_session,
                    CommandOutputPlan::success(),
                    context,
                );
            })
            .await?;
        Ok(())
    }

    pub(in crate::domains::target) async fn run_inline(
        self,
        conn: &mut CdpConnection,
        target: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        let (mut context, _) = CommandWorkflowContext::take_from(target, None, None);
        context.retain_terminal(CommandOutputPlan::success());
        let result = self
            .run_using(&mut ProtocolWorkflowDriver::inline(conn), &mut context)
            .await;
        let completed = match result {
            Ok(()) => context.complete(CommandOutputPlan::default()),
            Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
        };
        completed.finish(conn, target)
    }
}

fn recovery_workflow(
    access: ProtocolStateAccess,
    recovery: CommandWorkflowContextReturn,
    owner: CommandOwnerScope,
    session_id: Option<String>,
    strip: bool,
    response: CommandOutputPlan,
) -> PreparedCommandWorkflow {
    PreparedCommandWorkflow::new(move |mut context| async move {
        let inner_response = match response.command_status() {
            Some(Err(error)) => CommandOutputPlan::from_devtools_error(error),
            _ => CommandOutputPlan::error(-32000, crate::OwnedProtocolWaitFailure::Cancelled),
        };
        let mut driver = ProtocolWorkflowDriver::owned(access.clone());
        let retry_session = session_id.clone();
        let prepared = context
            .with_state(&mut driver, move |state, context| {
                let prepared = state.recover_protocol_command(
                    recovery,
                    owner,
                    inner_response,
                    context.context_mut().response_flush().clone(),
                );
                let recovery = prepared.recovery_scope();
                let owner = prepared.output_owner();
                context.context_mut().register_command_failure_cleanup(
                    CommandFailureCleanup::Continuation(CommandWorkflowFailureCleanup::new(
                        move |response| {
                            recovery_workflow(
                                access,
                                recovery,
                                owner,
                                retry_session,
                                strip,
                                response,
                            )
                        },
                    )),
                );
                prepared
            })
            .await;
        let result: Result<()> = async {
            let completed = Box::pin(prepared?.run_using(&mut driver)).await?;
            context
                .with_state(&mut driver, move |state, context| {
                    state.finish_target_message(completed, session_id, strip, response, context);
                })
                .await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => context.complete(CommandOutputPlan::default()),
            Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
        }
    })
}

pub(crate) fn retain_target_message_output(
    conn: &mut CdpConnection,
    outcome: CdpRendererOwnerTurnOutcome,
    session_id: Option<String>,
    strip: bool,
    mut response: CommandOutputPlan,
) -> CommandOutputPlan {
    let (events, after, boundary, post_response, scheduler, predecessor) =
        outcome.into_renderer_owner_turn_parts();
    conn.extend_scheduler_events(scheduler);
    if let Some(session_id) = session_id {
        response.extend_background_events(events.into_iter().map(|event| {
            BackgroundProtocolEvent::target_received_message_from_target(&session_id, event, strip)
        }));
        if let Some(boundary) = boundary {
            response.insert_renderer_output_boundary(boundary);
        } else {
            assert!(
                after.is_empty(),
                "nested post-renderer output requires its boundary"
            );
        }
        response.extend_background_events(after.into_iter().chain(post_response).map(|event| {
            BackgroundProtocolEvent::target_received_message_from_target(&session_id, event, strip)
        }));
    }
    if let Some(predecessor) = predecessor {
        response.set_renderer_output_predecessor(predecessor);
    }
    response
}
