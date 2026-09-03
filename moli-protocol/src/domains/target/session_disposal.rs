use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CdpSessionRoute, CommandOwnerScope,
    SessionDisposalPlan, SessionDisposalTarget, TargetEventPlan, TargetSessionDetachCleanupPlan,
};

fn prepare_session_disposal(
    conn: &CdpConnection,
    detachment: TargetSessionDetachCleanupPlan,
) -> anyhow::Result<(SessionDisposalPlan, TargetSessionDetachCleanupPlan)> {
    let session_id = detachment.session_id();
    let route = conn
        .session_route(Some(session_id))
        .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
    let plan = SessionDisposalPlan::for_session_route(session_id, &route)
        .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
    anyhow::ensure!(
        plan.target_id() == Some(detachment.target_id()),
        "UnknownTargetId"
    );
    Ok((plan, detachment))
}

fn prepare_browser_session_disposal(
    conn: &CdpConnection,
    session_id: &str,
) -> anyhow::Result<SessionDisposalPlan> {
    let route = conn
        .session_route(Some(session_id))
        .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
    let plan = SessionDisposalPlan::for_session_route(session_id, &route)
        .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
    anyhow::ensure!(
        matches!(plan.target(), SessionDisposalTarget::Browser),
        "InvalidSessionId"
    );
    Ok(plan)
}

pub(super) struct TargetSessionDisposalOutcome {
    event_plan: TargetEventPlan,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl TargetSessionDisposalOutcome {
    pub(super) fn into_parts(self) -> (TargetEventPlan, Option<moli_core::RendererOutputFence>) {
        (self.event_plan, self.renderer_output_predecessor)
    }
}

/// Runs every domain handler's disposal while `plan` still resolves to a live
/// session. All handlers get a cleanup opportunity even if one fails. A Page
/// whose renderer remains alive after such a failure is failed closed before
/// the binding can be removed, because otherwise renderer-owned scripts or
/// policy could outlive their DevTools session.
async fn dispose_live_session_domains_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    plan: &SessionDisposalPlan,
) -> anyhow::Result<Option<moli_core::RendererOutputFence>> {
    let disposal = crate::domains::session::dispose_live_handlers_async(
        conn,
        background_events,
        protocol_events,
        plan,
    )
    .await;
    if let Some(error) = disposal.first_error() {
        if matches!(plan.target(), SessionDisposalTarget::PageTarget { .. }) {
            recover_failed_page_session_disposal_async(conn, background_events, plan, error)
                .await?;
        } else {
            return Err(anyhow::anyhow!("{error:#}"));
        }
    }
    crate::domains::runtime::detach_session_inspector_async(conn, plan).await?;
    Ok(disposal.into_renderer_output_predecessor())
}

async fn recover_failed_page_session_disposal_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    plan: &SessionDisposalPlan,
    cleanup_error: &anyhow::Error,
) -> anyhow::Result<()> {
    let SessionDisposalTarget::PageTarget {
        browser_context_id,
        target_id,
        session_key,
    } = plan.target()
    else {
        anyhow::bail!("Page cleanup recovery requires a Page target");
    };
    let owner = CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
        browser_context_id: browser_context_id.clone(),
        target_id: target_id.clone(),
        session_key: session_key.clone(),
    });
    let has_live_page = conn
        .runtime_session_owner_slot_for_owner(&owner)
        .ok()
        .is_some_and(|slot| slot.loaded_page().is_some());

    if has_live_page {
        let inspector_session_ids = conn.page_event_session_ids_for_owner(&owner);
        for inspector_session_id in &inspector_session_ids {
            let inspector_owner = inspector_session_id
                .as_deref()
                .map(CommandOwnerScope::for_session)
                .unwrap_or_else(|| owner.clone());
            let _ =
                conn.with_target_devtools_session_state_for_owner_mut(&inspector_owner, |state| {
                    state
                        .runtime_session_state
                        .record_inspector_target_crashed()
                });
        }
        anyhow::ensure!(
            conn.mark_target_crashed_for_owner_async(&owner)
                .await
                .is_some(),
            "failed to close Page after session cleanup failure: {cleanup_error:#}"
        );
        background_events.extend(inspector_session_ids.into_iter().map(|session_id| {
            BackgroundProtocolEvent::inspector_target_crashed(session_id.as_deref())
        }));
        background_events
            .extend(conn.target_crashed_events_for_all_discovery_owners(target_id, "crashed", 1));
    }

    // Once no renderer Page remains, protocol-side ownership records can be
    // discarded without leaving executable renderer state behind.
    crate::domains::page::dispose_session_async(conn, plan.session_id())
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to finalize Page cleanup after renderer retirement: {error:#}; original cleanup failure: {cleanup_error:#}"
            )
        })?;
    tracing::warn!(
        session_id = plan.session_id(),
        target_id,
        %cleanup_error,
        "retired Page renderer after DevTools session cleanup failure"
    );
    Ok(())
}

pub(super) async fn dispose_browser_session_without_event_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> anyhow::Result<TargetEventPlan> {
    let plan = prepare_browser_session_disposal(conn, session_id)?;
    let mut discarded_background_events = Vec::new();
    let mut discarded_protocol_events = Vec::new();
    dispose_live_session_domains_async(
        conn,
        &mut discarded_background_events,
        &mut discarded_protocol_events,
        &plan,
    )
    .await?;
    conn.commit_browser_session_disposal_without_event(&plan)
}

pub(super) async fn dispose_browser_session_event_plan_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> anyhow::Result<TargetEventPlan> {
    let plan = prepare_browser_session_disposal(conn, session_id)?;
    let mut discarded_background_events = Vec::new();
    let mut discarded_protocol_events = Vec::new();
    dispose_live_session_domains_async(
        conn,
        &mut discarded_background_events,
        &mut discarded_protocol_events,
        &plan,
    )
    .await?;
    conn.commit_browser_session_disposal_event_plan(&plan)
}

/// Runs the connection-owned portion of domain disposal after a target has
/// already destroyed its renderer-owned state but before its session binding
/// is removed from the control plane.
pub(crate) async fn dispose_closed_session_domains_async(
    conn: &mut CdpConnection,
    plan: &SessionDisposalPlan,
) {
    crate::domains::session::dispose_closed_handlers_async(conn, plan.session_id()).await;
}

/// Rolls back a prepared attachment after it has become capable of owning
/// renderer/domain state but before its attached event is published. No
/// protocol event is emitted because the frontend never observed the session.
pub(crate) async fn dispose_uncommitted_session_async(
    conn: &mut CdpConnection,
    plan: &SessionDisposalPlan,
) -> anyhow::Result<()> {
    let mut discarded_background_events = Vec::new();
    let mut discarded_protocol_events = Vec::new();
    dispose_live_session_domains_async(
        conn,
        &mut discarded_background_events,
        &mut discarded_protocol_events,
        plan,
    )
    .await?;
    conn.commit_session_disposal(plan)
}

pub(super) async fn dispose_target_session_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    cleanup_plan: TargetSessionDetachCleanupPlan,
) -> anyhow::Result<TargetSessionDisposalOutcome> {
    let (plan, detachment) = prepare_session_disposal(conn, cleanup_plan)?;
    let renderer_output_predecessor =
        dispose_live_session_domains_async(conn, background_events, protocol_events, &plan).await?;

    conn.commit_session_disposal(&plan)?;
    let event_plan = conn.commit_target_session_detachment_event_plan(detachment);
    Ok(TargetSessionDisposalOutcome {
        event_plan,
        renderer_output_predecessor,
    })
}

pub(super) async fn dispose_primary_page_session_preserving_frontend_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
) -> anyhow::Result<Option<moli_core::RendererOutputFence>> {
    let route = conn
        .session_route(Some(session_id))
        .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
    let plan = SessionDisposalPlan::for_session_route(session_id, &route)
        .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
    if !matches!(
        plan.target(),
        SessionDisposalTarget::PageTarget {
            session_key: moli_page_types::DevToolsSessionKey::Primary,
            ..
        }
    ) {
        anyhow::bail!("InvalidSessionId");
    }

    let predecessor =
        dispose_live_session_domains_async(conn, background_events, protocol_events, &plan).await?;
    if !conn.release_primary_target_session_binding_without_event(session_id) {
        anyhow::bail!("InvalidSessionId");
    }
    Ok(predecessor)
}

pub(super) async fn dispose_dedicated_worker_session_after_prepared_state_delta_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    cleanup_plan: TargetSessionDetachCleanupPlan,
) -> anyhow::Result<TargetSessionDisposalOutcome> {
    let (plan, detachment) = prepare_session_disposal(conn, cleanup_plan)?;
    if !matches!(
        plan.target(),
        SessionDisposalTarget::DedicatedWorkerTarget { .. }
    ) {
        anyhow::bail!("InvalidSessionId");
    }

    dispose_live_session_domains_async(conn, background_events, protocol_events, &plan).await?;
    conn.commit_session_disposal(&plan)?;
    let event_plan =
        conn.commit_target_session_detachment_after_prepared_state_delta_event_plan(detachment);
    Ok(TargetSessionDisposalOutcome {
        event_plan,
        renderer_output_predecessor: None,
    })
}

/// Completes disposal after a worker target has transferred and dropped its
/// per-session state. Renderer-owned resources have already gone away with
/// the target, but connection-owned handlers still retire before the control
/// plane binding is removed.
pub(super) async fn dispose_removed_worker_session_async(
    conn: &mut CdpConnection,
    cleanup_plan: TargetSessionDetachCleanupPlan,
) -> anyhow::Result<TargetEventPlan> {
    let (plan, detachment) = prepare_session_disposal(conn, cleanup_plan)?;
    if !matches!(
        plan.target(),
        SessionDisposalTarget::SharedWorkerTarget { .. }
            | SessionDisposalTarget::DedicatedWorkerTarget { .. }
            | SessionDisposalTarget::ServiceWorkerTarget { .. }
    ) {
        anyhow::bail!("InvalidSessionId");
    }
    dispose_closed_session_domains_async(conn, &plan).await;
    Ok(conn.commit_target_session_detachment_event_plan(detachment))
}

/// Emergency completion for a DedicatedWorker retirement whose renderer
/// output failed after the target state was already removed. The renderer is
/// unavailable, so only connection and control-plane ownership remain.
pub(super) fn dispose_removed_dedicated_worker_session_after_failed_retirement(
    conn: &mut CdpConnection,
    cleanup_plan: TargetSessionDetachCleanupPlan,
) -> TargetEventPlan {
    crate::domains::session::dispose_closed_handlers_sync(conn, cleanup_plan.session_id());
    conn.commit_target_session_detachment_after_prepared_state_delta_event_plan(cleanup_plan)
}
