use anyhow::Result;

use crate::conn::{
    CommandWorkflowContext, CompletedInitialDocumentPageBuild, FailedInitialDocumentPageBuild,
    ProtocolWorkflowDriver, TargetAttachSessionCommit,
};
use crate::domains::command_output::CommandOutputPlan;

use super::{CdpConnection, PreparedPopupNavigation, TargetCreationCommit};

pub(super) async fn complete_attachment(
    driver: &mut ProtocolWorkflowDriver,
    prepared: TargetAttachSessionCommit,
    target_info: crate::automation::DevToolsTargetInfo,
    initial_document: Option<
        Result<Box<CompletedInitialDocumentPageBuild>, FailedInitialDocumentPageBuild>,
    >,
    context: &mut CommandWorkflowContext,
) -> Result<CommandOutputPlan> {
    let error = match initial_document {
        Some(Ok(completed)) => match completed.install_using(driver).await {
            Ok(diagnostics) => {
                if let Some(predecessor) = diagnostics.renderer_output_predecessor {
                    context
                        .context_mut()
                        .set_renderer_output_predecessor(predecessor);
                }
                None
            }
            Err(error) => Some(CommandOutputPlan::error_without_session(-32000, error)),
        },
        Some(Err(failed)) => {
            let error =
                driver.with_state(move |state| state.reset_failed_initial_document(failed))?;
            Some(CommandOutputPlan::error_without_session(-32000, error))
        }
        None => None,
    };
    if let Some(error) = error {
        rollback_attachment(driver, prepared).await?;
        return Ok(error);
    }
    let owner = crate::conn::CommandOwnerScope::for_route(prepared.route().clone());
    let pending =
        driver.with_state(move |state| state.prepare_target_attachment_bindings(owner))?;
    let bindings = match pending {
        Ok(pending) => match pending.wait().await {
            Ok(completed) => context.with_state(driver, move |state, context| {
                state.finish_target_attachment_bindings(completed, context)
            })?,
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    if let Err(error) = bindings
        && !error.is::<crate::conn::NoDocumentLoaded>()
    {
        tracing::warn!(%error, session_id = prepared.session_id(),
            "target attach renderer binding state apply failed");
    }
    {
        let access = driver.state_access();

        context.defer_owner_turn_completion(move |state, context| {
            match state.commit_target_attachment(prepared.clone(), target_info) {
                Ok(plan) => plan,
                Err(response) => {
                    context.register_command_failure_cleanup(
                        crate::conn::CommandFailureCleanup::Continuation(
                            crate::conn::CommandWorkflowFailureCleanup::new(move |response| {
                                attachment_rollback_workflow(access, prepared, response)
                            }),
                        ),
                    );
                    response
                }
            }
        });
        Ok(CommandOutputPlan::default())
    }
}

pub(super) fn attachment_rollback_workflow(
    access: crate::conn::ProtocolStateHandle,
    prepared: TargetAttachSessionCommit,
    response: CommandOutputPlan,
) -> crate::conn::PreparedCommandWorkflow {
    crate::conn::PreparedCommandWorkflow::new(move |context| async move {
        if let Err(error) =
            rollback_attachment(&mut ProtocolWorkflowDriver::new(access), prepared).await
        {
            tracing::warn!(%error, "failed to roll back retired target attachment");
        }
        context.complete(response)
    })
}

pub(crate) async fn rollback_attachment(
    driver: &mut ProtocolWorkflowDriver,
    prepared: TargetAttachSessionCommit,
) -> Result<crate::conn::TargetEventPlan> {
    let (rollback, disposal) =
        driver.with_state(move |state| state.prepare_target_attachment_rollback(prepared))?;
    let identity = if let Some(disposal) = disposal {
        let identity = disposal.identity();
        disposal
            .run(driver, &mut Vec::new(), &mut Vec::new())
            .await?;
        Some(identity)
    } else {
        None
    };
    driver.with_state(move |state| state.finish_target_attachment_rollback(rollback, identity))?
}

pub(super) async fn complete_creation(
    driver: &mut ProtocolWorkflowDriver,
    response: CommandOutputPlan,
    commit: TargetCreationCommit,
    initial_document: Option<
        Result<Box<CompletedInitialDocumentPageBuild>, FailedInitialDocumentPageBuild>,
    >,
    context: &mut CommandWorkflowContext,
) -> Result<CommandOutputPlan> {
    let activation_events = match commit.activation() {
        Some(activation) => activation.clone().complete_using(driver).await?,
        None => Vec::new(),
    };
    match initial_document {
        Some(Ok(completed)) => {
            let diagnostics = match completed.install_using(driver).await {
                Ok(diagnostics) => diagnostics,
                Err(error) => return Ok(CommandOutputPlan::error(-32000, error)),
            };
            if let Some(predecessor) = diagnostics.renderer_output_predecessor {
                context
                    .context_mut()
                    .set_renderer_output_predecessor(predecessor);
            }
        }
        Some(Err(failed)) => {
            let error =
                driver.with_state(move |state| state.reset_failed_initial_document(failed))?;
            return Ok(CommandOutputPlan::error(-32000, error));
        }
        None => {}
    }
    let navigation = context.with_state(driver, move |state, context| {
        state.prepare_target_creation_outputs(commit, activation_events, context)
    })?;
    context.materialize_projection_output();
    match navigation {
        Ok(Some(navigation)) => {
            navigation.run(driver, context).await?;
        }
        Ok(None) => {}
        Err(error) => return Ok(CommandOutputPlan::from_devtools_error(error)),
    }
    Ok(response)
}

pub(crate) fn prepare_creation_outputs(
    conn: &mut CdpConnection,
    commit: TargetCreationCommit,
    activation_events: Vec<crate::conn::BackgroundProtocolEvent>,
    context: &mut CommandWorkflowContext,
) -> Result<Option<PreparedPopupNavigation>, crate::automation::DevToolsError> {
    let target_id = commit.page_target_id().to_owned();
    let mut events = activation_events;
    super::creation::emit_target_creation_protocol_events(conn, commit, &mut events)?;
    let mut plan = CommandOutputPlan::default();
    plan.extend_background_events(events);
    context.retain_output(plan);
    Ok(super::popup::prepare_initial_target_url_navigation(
        conn, &target_id,
    ))
}
