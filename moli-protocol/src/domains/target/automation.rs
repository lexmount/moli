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
/// Creation requires Page installation and setup to succeed. On failure the
/// registered Target remains available for retry, without creation output.
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
        Self::prepare_registered_target(conn, execution)
    }

    fn prepare_registered_target(
        conn: &mut CdpConnection,
        execution: creation::DevToolsCreateTargetExecution,
    ) -> Result<Self, DevToolsError> {
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
                    let page = task.install(driver, context).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::{AutomationContext, FrontendProtocol};
    use crate::domains::command_output::CommandOutputPlan;
    use crate::testing::TestContext;
    use serde_json::json;

    #[tokio::test]
    async fn page_build_failure_fails_creation_and_keeps_the_target_recoverable() {
        let mut ctx = TestContext::new();
        ctx.process_async(
            json!({"id":1,"method":"Target.setDiscoverTargets","params":{"discover":true}}),
        )
        .await;
        let (prepared, owner, target, scheduler) = ctx.with_fixture(|conn| {
            let execution = creation::execute_devtools_create_target_command(
                conn,
                DevToolsCreateTargetCommand {
                    context: AutomationContext {
                        protocol: FrontendProtocol::WebDriverBidi,
                        session_id: None,
                        target_id: None,
                        browser_context_id: None,
                    },
                    url: "about:blank".into(),
                    browser_context_id: None,
                    activate: false,
                },
            )
            .unwrap();
            let target = execution.result.target_id.as_str().to_owned();
            let owner = CommandOwnerScope::for_route(
                conn.target_session_route_for_target_id(&target).unwrap(),
            );
            let build_owner = conn.initial_document_page_owner_for_owner(&owner).unwrap();
            // The normal installer joins this build. Its terminal failure is
            // controlled by the fixture rather than renderer timing.
            conn.runtime_session_owner_slot_mut_for_owner(&owner)
                .unwrap()
                .start_initial_document_page_build();
            let prepared =
                PreparedAutomationTargetCreation::prepare_registered_target(conn, execution)
                    .unwrap();
            conn.fail_initial_document_page_build_for_owner(
                &build_owner,
                "injected initial Page failure".into(),
            );
            (prepared, owner, target, conn.take_scheduler_events())
        });
        let host = prepared.host().clone();
        let mut output = CommandDispatchContext::default();
        let (mut context, _recovery) = CommandWorkflowContext::take_from(&mut output, None, None);
        let error = prepared
            .run(&mut ProtocolWorkflowDriver::new(ctx.handle()), &mut context)
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind, DevToolsErrorKind::Internal);
        assert!(error.message.contains("injected initial Page failure"));
        let completed = context.complete(CommandOutputPlan::default());
        ctx.with_fixture(|conn| {
            completed.finish(conn, &mut output);
        });
        let (events, _, after) = output.take_renderer_fenced_protocol_events();
        assert!(!events.into_iter().chain(after).any(|event| {
            let message = event.into_protocol_message();
            message["method"] == "Target.targetCreated"
                && message["params"]["targetInfo"]["targetId"] == target
        }));
        ctx.route_direct_command_output_for_test(Vec::new(), scheduler)
            .await;
        ctx.wait_until_scheduler_state("failed initialization retired", |conn| {
            let (browser, Some(target)) = conn.target_owner_identity_for_owner(&owner).unwrap()
            else {
                panic!("registered target");
            };
            conn.browser_context_by_id(&browser)
                .unwrap()
                .page_target(&target)
                .unwrap()
                .owner_state
                .initial_document_work
                .is_none()
        })
        .await;
        let retry = ctx.with_fixture(|conn| {
            assert!(host.is_current(conn));
            assert!(
                !conn
                    .runtime_session_owner_slot_for_owner(&owner)
                    .unwrap()
                    .has_loaded_page()
            );
            conn.ensure_target_initialization(&owner).unwrap().unwrap()
        });
        assert!(ctx.drive_owned_wait(retry.wait()).await.is_ok());
        ctx.with_fixture(|conn| {
            assert!(
                conn.runtime_session_owner_slot_for_owner(&owner)
                    .unwrap()
                    .has_loaded_page()
            )
        });
        // Materializing the retained Target does not retroactively turn the
        // failed creation into a successful creation announcement.
        assert!(
            !ctx.sent
                .iter()
                .any(|message| message["method"] == "Target.targetCreated"
                    && message["params"]["targetInfo"]["targetId"] == target)
        );
    }
}
