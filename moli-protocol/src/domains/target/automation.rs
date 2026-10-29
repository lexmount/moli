use anyhow::{Context as _, Result};

use crate::automation::{
    AutomationCommand, AutomationResult, DevToolsCreateTargetCommand, DevToolsCreateTargetResult,
    DevToolsError, DevToolsErrorKind,
};
use crate::conn::{
    CdpConnection, CommandDispatchContext, CommandOwnerScope, CommandWorkflowContext,
    PendingInitialDocumentPageBuild, ProtocolPageHostScope, ProtocolWorkflowDriver,
};

use super::{TargetCommandTaskStep, TargetCreationCommit, browser_context, closing, creation};

/// Creation owns the original host and initial document work before its first
/// poll. Listener setup uses the same finite Runtime capabilities as navigation.
pub(crate) struct PreparedAutomationTargetCreation {
    result: DevToolsCreateTargetResult,
    commit: TargetCreationCommit,
    host: ProtocolPageHostScope,
    initial_document: Result<Option<PendingInitialDocumentPageBuild>>,
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
        let initial_document = conn.start_initial_document_page_ensure_for_owner(host.owner());
        Ok(Self {
            result: execution.result,
            commit: execution.commit,
            host,
            initial_document,
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
        let mut realm_ids = Vec::new();
        // Initial document materialization was historically best-effort. A
        // failed build still resets only its original owner before creation
        // event validation decides whether the target can be published.
        let initial = async {
            if let Some(pending) = self.initial_document? {
                let diagnostics = match pending.wait().await {
                    Ok(completed) => completed.install_using(driver).await?,
                    Err(failed) => {
                        let error = driver
                            .with_state(move |state| state.reset_failed_initial_document(failed))?;
                        tracing::debug!(?error, "failed to await create-target initial document");
                        return Ok(());
                    }
                };
                if let Some(predecessor) = diagnostics.renderer_output_predecessor {
                    context
                        .context_mut()
                        .set_renderer_output_predecessor(predecessor);
                }
                realm_ids = diagnostics
                    .initial_runtime_realms
                    .into_iter()
                    .filter(|realm| realm.realm_id.as_deref().is_some_and(|id| !id.is_empty()))
                    .map(|realm| realm.context_id)
                    .collect();
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = initial {
            tracing::debug!(?error, target_id = %self.result.target_id.as_str(),
                "failed to materialize create-target initial document");
        }
        let host = self.host.clone();
        let has_preload = driver
            .with_state(move |state| state.created_target_has_channel_preload(&host))
            .map_err(target_creation_error)?;
        if has_preload {
            if realm_ids.is_empty() {
                let host = self.host.clone();
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
                let host = self.host.clone();
                let listeners = driver
                    .with_state(move |state| {
                        state.prepare_created_target_channel_listeners(&host, id)
                    })
                    .map_err(target_creation_error)?;
                if let Some(listeners) = listeners {
                    let mut events = Vec::new();
                    listeners.run(driver, &mut events).await;
                    context.context_mut().protocol_events_mut().extend(events);
                }
            }
        }
        if let Some(activation) = self.commit.activation() {
            match activation.clone().complete_using(driver).await {
                Ok(events) => context.context_mut().protocol_events_mut().extend(events),
                Err(error) => {
                    tracing::warn!(%error, "failed to complete target creation activation")
                }
            }
        }
        context
            .with_state(driver, move |state, context| {
                state.finish_automation_target_creation(
                    self.host,
                    self.commit,
                    context.context_mut(),
                )
            })
            .map_err(target_creation_error)??;
        Ok(AutomationResult::CreateTarget(self.result))
    }
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
