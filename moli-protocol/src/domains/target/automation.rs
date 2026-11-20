use anyhow::{Context as _, Result};

use crate::automation::{
    AutomationCommand, AutomationResult, DevToolsCreateTargetCommand, DevToolsCreateTargetResult,
    DevToolsError, DevToolsErrorKind,
};
use crate::conn::{
    CdpConnection, CommandDispatchContext, CommandOwnerScope, CommandWorkflowContext,
    ProtocolPageHostScope, ProtocolWorkflowDriver,
};

use super::{TargetCommandTaskStep, TargetCreationCommit, browser_context, closing, creation};

/// Creation setup and initial navigation belong to the Target. The command
/// waits only for setup and owns a claim on its accepted output.
pub(crate) struct PreparedAutomationTargetCreation {
    result: DevToolsCreateTargetResult,
    host: ProtocolPageHostScope,
    initialization: crate::conn::PendingTargetCreation,
}

impl PreparedAutomationTargetCreation {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        command: DevToolsCreateTargetCommand,
    ) -> Result<Self, DevToolsError> {
        let execution = creation::execute_devtools_create_target_command(conn, command)?;
        let owner = conn
            .target_session_route_for_target_id(execution.result.target_id.as_str())
            .map(CommandOwnerScope::for_route)
            .context(crate::conn::NoDocumentLoaded)
            .map_err(target_creation_error)?;
        let host = ProtocolPageHostScope::capture_for_operation(conn, &owner)
            .map_err(target_creation_error)?;
        let reservation = conn.reserve_target_initialization(host.owner())?;
        let setup_host = host.clone();
        let commit = execution.commit;
        let initialization = reservation.submit_creation(
            conn,
            crate::conn::TargetInitializationStage::SetupFinished,
            Box::new(move |mut task, driver, context| {
                Box::pin(async move {
                    let page = task.install(driver).await?;
                    if let Some(predecessor) = page.renderer_output_predecessor.clone() {
                        context
                            .context_mut()
                            .set_renderer_output_predecessor(predecessor);
                    }
                    let realm_ids = page
                        .initial_runtime_realms
                        .iter()
                        .filter(|realm| realm.realm_id.as_deref().is_some_and(|id| !id.is_empty()))
                        .map(|realm| realm.context_id)
                        .collect();
                    setup_created_target(driver, context, setup_host, commit, realm_ids).await?;
                    task.setup_finished(context, &page);
                    task.navigate(driver, context).await?;
                    Ok(page)
                })
            }),
        )?;
        Ok(Self {
            result: execution.result,
            host,
            initialization,
        })
    }

    pub(crate) fn host(&self) -> &ProtocolPageHostScope {
        &self.host
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<AutomationResult, DevToolsError> {
        self.initialization
            .wait()
            .await
            .publish_into(driver, context)?;
        Ok(AutomationResult::CreateTarget(self.result))
    }
}

async fn setup_created_target(
    driver: &mut ProtocolWorkflowDriver,
    context: &mut CommandWorkflowContext,
    host: ProtocolPageHostScope,
    commit: TargetCreationCommit,
    mut realm_ids: Vec<i64>,
) -> Result<(), DevToolsError> {
    let preload_host = host.clone();
    let has_preload = driver
        .with_state(move |state| state.created_target_has_channel_preload(&preload_host))
        .map_err(target_creation_error)?;
    if has_preload {
        if realm_ids.is_empty() {
            let host = host.clone();
            let query = driver
                .with_state(move |state| state.prepare_created_target_default_context(&host))
                .map_err(target_creation_error)?;
            if let Ok(query) = query {
                let mut predecessor = None;
                if let Ok(Some(id)) = query.run(driver, &mut predecessor).await {
                    realm_ids.push(id);
                }
                if let Some(predecessor) = predecessor {
                    context
                        .context_mut()
                        .set_renderer_output_predecessor(predecessor);
                }
            }
        }
        for id in realm_ids {
            let host = host.clone();
            let listeners = driver
                .with_state(move |state| state.prepare_created_target_channel_listeners(&host, id))
                .map_err(target_creation_error)?;
            if let Some(listeners) = listeners {
                let mut events = Vec::new();
                listeners.run(driver, &mut events).await;
                context.context_mut().protocol_events_mut().extend(events);
            }
        }
    }
    if let Some(activation) = commit.activation() {
        match activation.clone().complete_using(driver).await {
            Ok(events) => context.context_mut().protocol_events_mut().extend(events),
            Err(error) => {
                tracing::warn!(%error, "failed to complete target creation activation")
            }
        }
    }
    context
        .with_state(driver, move |state, context| {
            state.finish_automation_target_creation(host, commit, context.context_mut())
        })
        .map_err(target_creation_error)??;
    Ok(())
}

fn target_creation_error(error: anyhow::Error) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, error)
}

pub(crate) fn start_automation_lifecycle_command(
    conn: &mut CdpConnection,
    command: AutomationCommand,
    id: u64,
    context: &mut CommandDispatchContext,
) -> TargetCommandTaskStep {
    match command {
        AutomationCommand::CloseTarget(command) => {
            closing::start_devtools_close_target_command(conn, Some(id), None, command)
        }
        AutomationCommand::RemoveBrowserContext(command) => {
            browser_context::start_automation_remove_browser_context(conn, command, id, context)
        }
        _ => unreachable!("only admitted Target lifecycle commands use the protocol workflow"),
    }
}
