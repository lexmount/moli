use super::*;
use crate::domains::{browser, emulation, storage};

/// Domain work admitted before the first task poll. These are real domain
/// operations, not resumable business phases or a second command registry.
enum PreparedConfigurationCommand {
    Browser(Option<browser::PendingBrowserCommandDispatch>),
    Emulation(emulation::PreparedEmulationAutomationCommand),
    Storage(storage::PreparedStorageAutomationCommand),
    Complete(Result<AutomationResult, DevToolsError>),
}

impl PreparedConfigurationCommand {
    fn prepare(conn: &mut CdpConnection, command: AutomationCommand) -> Self {
        match command {
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
            Self::Complete(result) => result,
            Self::Browser(None) => Ok(AutomationResult::Empty),
            Self::Browser(Some(pending)) => {
                let completed = pending.wait().await;
                context
                    .with_state(driver, move |state, _| {
                        state.finish_automation_permission(completed)
                    })
                    .await
                    .map_err(configuration_error)?
            }
            Self::Emulation(pending) => {
                let completed = pending.wait().await;
                context
                    .with_state(driver, move |state, _| {
                        state.finish_automation_emulation(completed)
                    })
                    .await
                    .map_err(configuration_error)?
            }
            Self::Storage(storage::PreparedStorageAutomationCommand::Complete(result)) => result,
            Self::Storage(storage::PreparedStorageAutomationCommand::SetCookies(pending)) => {
                let completed = pending.wait().await;
                context
                    .with_state(driver, move |state, _| {
                        state.finish_automation_storage(completed)
                    })
                    .await
                    .map_err(configuration_error)?
            }
        }
    }
}

fn configuration_error(error: impl std::fmt::Display) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, format!("{error:#}"))
}

pub(super) fn prepare_workflow(
    conn: &mut CdpConnection,
    command: AutomationCommand,
    access: ProtocolStateAccess,
    result: AutomationCommandResult,
) -> PreparedCommandWorkflow {
    let command_context = command.context().clone();
    let host = conn
        .command_owner_scope_for_devtools_context(&command_context)
        .and_then(|owner| ProtocolPageHostScope::capture(conn, &owner).ok());
    let prepared = PreparedConfigurationCommand::prepare(conn, command);
    PreparedCommandWorkflow::new(move |mut context| async move {
        let mut driver = ProtocolWorkflowDriver::owned(access);
        let output = prepared.run(&mut driver, &mut context).await;
        context.materialize_projection_output();
        context.complete_in_owner_turn(
            crate::domains::command_output::CommandOutputPlan::default(),
            move |state, context| {
                state.finish_configuration_automation_output(
                    &command_context,
                    host.as_ref(),
                    context,
                );
                assert!(
                    result.borrow_mut().replace(output).is_none(),
                    "one typed result per automation owner"
                );
                crate::domains::command_output::CommandOutputPlan::default()
            },
        )
    })
    .cancel_on_waiter_drop()
}

pub(super) async fn execute_inline(
    conn: &mut CdpConnection,
    command: AutomationCommand,
) -> AutomationExecutionOutput {
    let prepared = PreparedConfigurationCommand::prepare(conn, command);
    let mut driver = ProtocolWorkflowDriver::inline(conn);
    let (mut context, _recovery) =
        CommandWorkflowContext::take_from(&mut CommandDispatchContext::default(), None, None);
    let result = prepared.run(&mut driver, &mut context).await;
    match driver
        .with_state(move |state| state.finish_inline_automation(context, result))
        .await
    {
        Ok(output) => output,
        Err(error) => AutomationExecutionOutput::new(Err(configuration_error(error))),
    }
}
