use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, TargetBindingCleanupAction, TargetBindingCleanupPlan,
    TargetEventPlan, TargetSessionDetachCleanupPlan,
};

pub(super) struct TargetSessionDisposalOutcome {
    event_plan: TargetEventPlan,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl TargetSessionDisposalOutcome {
    pub(super) fn into_parts(self) -> (TargetEventPlan, Option<moli_core::RendererOutputFence>) {
        (self.event_plan, self.renderer_output_predecessor)
    }
}

pub(super) async fn dispose_target_session_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    cleanup_plan: TargetSessionDetachCleanupPlan,
) -> anyhow::Result<TargetSessionDisposalOutcome> {
    let session_id = cleanup_plan.session_id().to_owned();
    let route = conn
        .session_route(Some(&session_id))
        .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
    let binding_plan = TargetBindingCleanupPlan::from_route(&session_id, &route);
    let expected_target_id = cleanup_plan.target_id();
    let action_target_id = match binding_plan.action() {
        TargetBindingCleanupAction::None => None,
        TargetBindingCleanupAction::PageTarget { target_id, .. }
        | TargetBindingCleanupAction::SharedWorkerTarget { target_id }
        | TargetBindingCleanupAction::DedicatedWorkerTarget { target_id }
        | TargetBindingCleanupAction::ServiceWorkerTarget { target_id } => Some(target_id.as_str()),
        TargetBindingCleanupAction::TabTarget { tab_target_id } => Some(tab_target_id.as_str()),
    };
    if action_target_id.is_some_and(|target_id| target_id != expected_target_id) {
        anyhow::bail!("UnknownTargetId");
    }

    let mut renderer_output_predecessor = None;
    match binding_plan.action() {
        TargetBindingCleanupAction::PageTarget { .. } => {
            renderer_output_predecessor = dispose_page_session_runtime_state_async(
                conn,
                background_events,
                protocol_events,
                &session_id,
            )
            .await?;
        }
        TargetBindingCleanupAction::SharedWorkerTarget { target_id } => {
            let renderer_detach = conn.browser_context.as_ref().and_then(|browser_context| {
                browser_context
                    .shared_worker_target(target_id)
                    .map(|target| {
                        (
                            browser_context.renderer_runtime(),
                            target.renderer_instance_id,
                        )
                    })
            });
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                &session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                background_events,
                protocol_events,
                Some(&session_id),
                "Target detached",
            );
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_shared_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.clone()),
                );
            }
        }
        TargetBindingCleanupAction::DedicatedWorkerTarget { target_id } => {
            let renderer_detach = conn.browser_context.as_ref().and_then(|browser_context| {
                browser_context
                    .dedicated_worker_target(target_id)
                    .map(|target| {
                        (
                            browser_context.renderer_runtime(),
                            target.renderer_instance_id,
                        )
                    })
            });
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                &session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                background_events,
                protocol_events,
                Some(&session_id),
                "Target detached",
            );
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_dedicated_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.clone()),
                );
            }
        }
        TargetBindingCleanupAction::ServiceWorkerTarget { .. } => {
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                background_events,
                protocol_events,
                Some(&session_id),
                "Target detached",
            );
            super::set_service_worker_pause_on_start_owner(conn, Some(&session_id), false);
        }
        TargetBindingCleanupAction::TabTarget { .. } => {}
        TargetBindingCleanupAction::None => anyhow::bail!("InvalidSessionId"),
    }

    let event_plan = conn
        .detach_session_with_binding_cleanup_event_plan_async(cleanup_plan)
        .await?;
    Ok(TargetSessionDisposalOutcome {
        event_plan,
        renderer_output_predecessor,
    })
}

pub(super) async fn dispose_page_session_runtime_state_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
) -> anyhow::Result<Option<moli_core::RendererOutputFence>> {
    let renderer_output_predecessor =
        super::clear_detached_target_fetch_state_background_events_async(
            conn,
            background_events,
            session_id,
        )
        .await;
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        background_events,
        protocol_events,
        Some(session_id),
        "Target detached",
    );

    // Document-start scripts are renderer-owned resources. Remove them before
    // detaching the Inspector session that carries the cleanup commands.
    conn.remove_document_start_scripts_for_detached_session_async(session_id)
        .await?;
    clear_page_session_target_state_async(conn, session_id).await?;
    if let Err(error) = conn
        .detach_runtime_inspector_session_for_session_owner_async(Some(session_id))
        .await
    {
        tracing::debug!(
            session_id,
            %error,
            "renderer Inspector session was already unavailable during disposal"
        );
    }
    Ok(renderer_output_predecessor)
}

async fn clear_page_session_target_state_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> anyhow::Result<()> {
    crate::domains::emulation::clear_emulated_media_for_detached_session_async(conn, session_id)
        .await?;
    conn.clear_target_session_overrides_async(session_id)
        .await?;
    Ok(())
}
