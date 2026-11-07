use super::*;

pub(crate) async fn handle_ace_page_task_input_dispatch_outcome(
    driver: &mut ProtocolWorkflowDriver,
    session_id: Option<&str>,
    owner: &TargetPageResidenceIdentity,
    host: &crate::conn::ProtocolPageHostScope,
    outcome: RendererInputDispatchOutcome,
    context: &mut CommandDispatchContext,
) -> Result<Vec<BackgroundProtocolEvent>> {
    let mut effects = InputCommandSideEffects::default();
    handle_input_dispatch_outcome(
        driver,
        &mut effects,
        session_id,
        owner,
        Some(host),
        outcome,
        context,
    )
    .await?;
    Ok(effects.into_events())
}
