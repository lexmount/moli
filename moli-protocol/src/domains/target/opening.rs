use anyhow::Result;

use crate::conn::{
    CommandWorkflowContext, CompletedTargetCreation, ProtocolWorkflowDriver,
    TargetAttachSessionCommit,
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

pub(super) fn complete_creation(
    driver: &mut ProtocolWorkflowDriver,
    response: CommandOutputPlan,
    initial_document: CompletedTargetCreation,
    context: &mut CommandWorkflowContext,
) -> CommandOutputPlan {
    match initial_document.publish_into(driver, context) {
        Ok(_) => response,
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
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
