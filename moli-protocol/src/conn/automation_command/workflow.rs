use super::*;
use crate::domains::{browser, emulation, input, storage};

pub(super) enum AutomationCommandSuccess {
    Result(Box<AutomationResult>),
    Navigation(crate::domains::page::AutomationNavigationResult),
}

impl AutomationCommandSuccess {
    pub(super) fn finish(self) -> Result<AutomationResult, DevToolsError> {
        match self {
            Self::Result(result) => Ok(*result),
            Self::Navigation(result) => result.finish(),
        }
    }
}

/// Domain work admitted before the first task poll. These are real domain
/// operations, not resumable business phases or a second command registry.
enum PreparedAutomationCommand {
    PageQuery(crate::domains::page::PreparedAutomationPageQuery),
    Dom(crate::domains::dom::PreparedAutomationDomCommand),
    CreateTarget(Box<crate::domains::target::PreparedAutomationTargetCreation>),
    ActivateTarget(Option<crate::domains::target::activation::PreparedTargetActivation>),
    Input(Result<Option<input::PendingInputCommandDispatch>, DevToolsError>),
    Browser(Option<browser::PendingBrowserCommandDispatch>),
    Emulation(emulation::PreparedEmulationAutomationCommand),
    Storage(storage::PreparedStorageAutomationCommand),
    Complete(Result<AutomationResult, DevToolsError>),
}

impl PreparedAutomationCommand {
    fn prepare(conn: &mut CdpConnection, command: AutomationCommand) -> Self {
        match command {
            command @ (AutomationCommand::GetFrameTree(_)
            | AutomationCommand::GetFrameTrees(_)
            | AutomationCommand::GetLayoutMetrics(_)
            | AutomationCommand::CaptureScreenshot(_)) => {
                match crate::domains::page::PreparedAutomationPageQuery::prepare(conn, command) {
                    Ok(query) => Self::PageQuery(query),
                    Err(error) => Self::Complete(Err(error)),
                }
            }
            command @ (AutomationCommand::QuerySelector(_)
            | AutomationCommand::GetAttributes(_)
            | AutomationCommand::GetText(_)
            | AutomationCommand::GetProperty(_)
            | AutomationCommand::GetOuterHtml(_)
            | AutomationCommand::DescribeNode(_)
            | AutomationCommand::GetFrameOwner(_)
            | AutomationCommand::GetNodeForLocation(_)
            | AutomationCommand::ResolveNode(_)
            | AutomationCommand::ScrollIntoViewIfNeeded(_)
            | AutomationCommand::DomObjectReference(_)
            | AutomationCommand::SetFileInputFiles(_)
            | AutomationCommand::DomGeometry(_)) => {
                match crate::domains::dom::PreparedAutomationDomCommand::prepare(conn, command) {
                    Ok(prepared) => Self::Dom(prepared),
                    Err(error) => Self::Complete(Err(error)),
                }
            }
            AutomationCommand::CreateTarget(command) => {
                match crate::domains::target::PreparedAutomationTargetCreation::prepare(
                    conn, command,
                ) {
                    Ok(prepared) => Self::CreateTarget(Box::new(prepared)),
                    Err(error) => Self::Complete(Err(error)),
                }
            }
            AutomationCommand::ActivateTarget(command) => {
                match crate::domains::target::activation::prepare_target_activation(conn, command) {
                    Ok(prepared) => Self::ActivateTarget(prepared),
                    Err(error) => Self::Complete(Err(error)),
                }
            }
            command @ (AutomationCommand::ElementClick(_)
            | AutomationCommand::DispatchMouseEvent(_)
            | AutomationCommand::DispatchKeyEvent(_)
            | AutomationCommand::DispatchTouchEvent(_)
            | AutomationCommand::DispatchDragEvent(_)
            | AutomationCommand::SynthesizeTapGesture(_)) => {
                Self::Input(input::prepare_automation_input_command(conn, command))
            }
            AutomationCommand::SetPermission(command) => {
                match browser::prepare_devtools_set_permission_command(conn, command) {
                    Ok(pending) => Self::Browser(pending),
                    Err(error) => Self::Complete(Err(error)),
                }
            }
            command @ (AutomationCommand::GetCookies(_)
            | AutomationCommand::SetCookies(_)
            | AutomationCommand::DeleteCookies(_)) => {
                Self::Storage(storage::prepare_storage_automation_command(conn, command))
            }
            command => match emulation::prepare_emulation_automation_command(conn, command) {
                Ok(prepared) => Self::Emulation(prepared),
                Err(error) => Self::Complete(Err(error)),
            },
        }
    }

    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<AutomationResult, DevToolsError> {
        match self {
            Self::PageQuery(query) => query.run(driver, context).await,
            Self::Dom(command) => command.run(driver, context).await,
            Self::CreateTarget(prepared) => prepared.run(driver, context).await,
            Self::ActivateTarget(None) => Ok(AutomationResult::Empty),
            Self::ActivateTarget(Some(prepared)) => {
                let completed = prepared
                    .run(driver)
                    .await
                    .map_err(|error| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, error))?;
                context
                    .with_state(driver, move |state, context| {
                        context
                            .context_mut()
                            .protocol_events_mut()
                            .extend(state.finish_page_activation(completed));
                    })
                    .await
                    .map_err(automation_error)?;
                Ok(AutomationResult::Empty)
            }
            Self::Input(pending) => {
                input::run_automation_input_command(pending, driver, context).await
            }
            Self::Complete(result) => result,
            Self::Browser(None) => Ok(AutomationResult::Empty),
            Self::Browser(Some(pending)) => {
                let completed = pending.wait().await;
                context
                    .with_state(driver, move |state, _| {
                        state.finish_automation_permission(completed)
                    })
                    .await
                    .map_err(automation_error)?
            }
            Self::Emulation(pending) => {
                let completed = pending.wait().await;
                context
                    .with_state(driver, move |state, _| {
                        state.finish_automation_emulation(completed)
                    })
                    .await
                    .map_err(automation_error)?
            }
            Self::Storage(storage::PreparedStorageAutomationCommand::Complete(result)) => result,
            Self::Storage(storage::PreparedStorageAutomationCommand::SetCookies(pending)) => {
                let completed = pending.wait().await;
                context
                    .with_state(driver, move |state, _| {
                        state.finish_automation_storage(completed)
                    })
                    .await
                    .map_err(automation_error)?
            }
        }
    }
}

fn automation_error(error: impl std::fmt::Display) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, format!("{error:#}"))
}

pub(super) fn prepare_workflow(
    conn: &mut CdpConnection,
    command: AutomationCommand,
    access: ProtocolStateAccess,
    result: AutomationCommandResult,
) -> PreparedCommandWorkflow {
    let command_context = command.context().clone();
    let mut host = match &command {
        AutomationCommand::ActivateTarget(command) => conn
            .target_session_route_for_target_id(command.target_id.as_str())
            .map(CommandOwnerScope::for_route),
        _ => conn.command_owner_scope_for_devtools_context(&command_context),
    }
    .and_then(|owner| ProtocolPageHostScope::capture(conn, &owner).ok());
    let prepared = PreparedAutomationCommand::prepare(conn, command);
    let completion_owned = matches!(
        &prepared,
        PreparedAutomationCommand::CreateTarget(_) | PreparedAutomationCommand::ActivateTarget(_)
    );
    if let PreparedAutomationCommand::CreateTarget(creation) = &prepared {
        host = Some(creation.host().clone());
    }
    let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
        let mut driver = ProtocolWorkflowDriver::owned(access);
        let output = prepared.run(&mut driver, &mut context).await;
        context.materialize_projection_output();
        context.complete_in_owner_turn(
            crate::domains::command_output::CommandOutputPlan::default(),
            move |state, context| {
                state.finish_automation_output(&command_context, host.as_ref(), context);
                assert!(
                    result.borrow_mut().replace(output).is_none(),
                    "one typed result per automation owner"
                );
                crate::domains::command_output::CommandOutputPlan::default()
            },
        )
    });
    if completion_owned {
        workflow
    } else {
        workflow.cancel_on_waiter_drop()
    }
}

pub(super) async fn execute_inline(
    conn: &mut CdpConnection,
    command: AutomationCommand,
) -> AutomationExecutionOutput {
    let prepared = PreparedAutomationCommand::prepare(conn, command);
    let mut driver = ProtocolWorkflowDriver::inline(conn);
    let (mut context, _recovery) =
        CommandWorkflowContext::take_from(&mut CommandDispatchContext::default(), None, None);
    let result = prepared.run(&mut driver, &mut context).await;
    match driver
        .with_state(move |state| state.finish_inline_automation(context, result))
        .await
    {
        Ok(output) => output,
        Err(error) => AutomationExecutionOutput::new(Err(automation_error(error))),
    }
}

/// Direct-connection compatibility uses the same admitted command and common
/// wait/recovery implementation; no Fetch business continuation loop remains.
pub(super) async fn execute_protocol_inline(
    conn: &mut CdpConnection,
    command: AutomationCommand,
) -> AutomationExecutionOutput {
    let id = conn.next_internal_runtime_command_id();
    let success = protocol_command_success(&command).expect("protocol automation command");
    let prepared = match PreparedProtocolCommand::for_automation_command(
        conn,
        command,
        id,
        Default::default(),
    ) {
        Ok(prepared) => prepared,
        Err(error) => return AutomationExecutionOutput::new(Err(error)),
    };
    let mut driver = ProtocolWorkflowDriver::inline(conn);
    let (mut context, _scope) =
        CommandWorkflowContext::take_from(&mut CommandDispatchContext::default(), None, None);
    let result = async {
        let completed = prepared
            .run_using(&mut driver)
            .await
            .map_err(automation_error)?;
        context
            .with_state(&mut driver, move |state, context| {
                let response = state
                    .finish_protocol_subcommand(completed, id, context)
                    .ok_or_else(|| {
                        DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            MissingAutomationCommandResponse,
                        )
                    })?;
                if let Some(error) = response.get("error") {
                    Err(crate::domains::command_output::devtools_error_from_cdp_error_value(error))
                } else {
                    success.finish()
                }
            })
            .await
            .map_err(automation_error)?
    }
    .await;
    match driver
        .with_state(move |state| state.finish_inline_automation(context, result))
        .await
    {
        Ok(output) => output,
        Err(error) => AutomationExecutionOutput::new(Err(automation_error(error))),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("MissingAutomationCommandResponse")]
struct MissingAutomationCommandResponse;

pub(super) fn protocol_command_success(
    command: &AutomationCommand,
) -> Option<AutomationCommandSuccess> {
    let result = match command {
        AutomationCommand::CloseTarget(_) => Some(AutomationResult::CloseTarget(
            crate::automation::DevToolsCloseTargetResult { success: true },
        )),
        AutomationCommand::RemoveBrowserContext(_) => Some(AutomationResult::Empty),
        AutomationCommand::AddNetworkIntercept(_)
        | AutomationCommand::RemoveNetworkIntercept(_)
        | AutomationCommand::ContinueInterceptedRequest(_)
        | AutomationCommand::ContinueInterceptedResponse(_)
        | AutomationCommand::ContinueWithAuth(_)
        | AutomationCommand::FailInterceptedRequest(_)
        | AutomationCommand::FulfillInterceptedRequest(_) => Some(
            crate::domains::fetch::devtools_fetch_success_result(command),
        ),
        _ => None,
    };
    result.map(|result| AutomationCommandSuccess::Result(Box::new(result)))
}
