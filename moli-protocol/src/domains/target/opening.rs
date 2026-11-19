use anyhow::Result;

use crate::conn::{
    CommandWorkflowContext, CompletedInitialDocumentPageBuild, FailedInitialDocumentPageBuild,
    ProtocolWorkflowDriver, TargetAttachSessionCommit,
};
use crate::domains::command_output::CommandOutputPlan;

use super::{CdpConnection, PreparedPopupNavigation, TargetCreationCommit};

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
    initial_document: Option<
        Result<Box<CompletedInitialDocumentPageBuild>, FailedInitialDocumentPageBuild>,
    >,
    context: &mut CommandWorkflowContext,
) -> Result<CommandOutputPlan> {
    match initial_document {
        Some(Ok(completed)) => {
            let diagnostics = match completed.install_into_context(driver, context).await {
                Ok(diagnostics) => diagnostics,
                Err(error) => return Ok(CommandOutputPlan::error(-32000, error)),
            };
            context.context_mut().extend_renderer_output_dependencies(
                diagnostics.initialization_output_dependencies,
            );
            if let Some(predecessor) = diagnostics.renderer_output_predecessor {
                context
                    .context_mut()
                    .set_renderer_output_predecessor(predecessor);
            }
        }
        Some(Err(FailedInitialDocumentPageBuild::Target { error })) => {
            return Ok(CommandOutputPlan::from_devtools_error(error));
        }
        Some(Err(failed)) => {
            let error =
                driver.with_state(move |state| state.reset_failed_initial_document(failed))?;
            return Ok(CommandOutputPlan::error(-32000, error));
        }
        None => {}
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
