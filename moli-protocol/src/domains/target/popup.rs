use crate::conn::{
    CdpSessionRoute, CommandOwnerScope, PreparedTargetAttach, TargetAttachSessionCommit,
    TargetStartupOwnerAction,
};

use super::creation::{
    push_target_created_events, top_level_page_auto_attach_owner_sessions,
    top_level_tab_auto_attach_owner_sessions,
};
use super::*;

/// Adopt an already committed native auxiliary context. No failure in this
/// observer is permission to destroy the Browser's independently owned Window.
pub(crate) async fn project_browser_popup_target(
    conn: &mut CdpConnection,
    target_id: &str,
    snapshot: &moli_core::browser::WebContentsSnapshot,
) -> Vec<BackgroundProtocolEvent> {
    let mut out = Vec::new();
    let browser_context_id = conn
        .browser_context_id_for_target(target_id)
        .expect("adopted popup Context")
        .to_owned();
    let tab_target_id = conn.register_top_level_page_target(target_id);
    let mut page_sessions = Vec::new();
    let mut tab_sessions = Vec::new();
    for (target, owners, sessions) in [
        (
            tab_target_id.as_str(),
            top_level_tab_auto_attach_owner_sessions(conn),
            &mut tab_sessions,
        ),
        (
            target_id,
            top_level_page_auto_attach_owner_sessions(conn),
            &mut page_sessions,
        ),
    ] {
        for owner in owners {
            let session = conn.gen_session_id();
            let route = if target == target_id {
                conn.prepare_auto_attached_page_session_binding_in_browser_context(
                    &browser_context_id,
                    target,
                    session.clone(),
                )
            } else {
                conn.prepare_auto_attached_tab_session_binding(
                    target,
                    session.clone(),
                    owner.as_deref(),
                )
            };
            if let Some(route) = route {
                sessions.push((owner, session, route));
            }
        }
    }
    match conn
        .ensure_native_popup_initial_document(snapshot.handle)
        .await
    {
        Ok(events) => out.extend(events),
        Err(error) => {
            tracing::debug!(%error, "native popup initial document did not commit");
            return out;
        }
    }
    if !ensure_popup_initial_document_page_async(conn, target_id).await {
        return out;
    }
    let Some(target_info) = conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|context| context.devtools_target_info(target_id))
    else {
        return out;
    };
    let Some(tab_info) = conn.tab_target_info(&tab_target_id) else {
        return out;
    };
    if conn.has_any_target_discovery() {
        push_target_created_events(conn, &mut out, target_id);
    } else {
        out.push(BackgroundProtocolEvent::automation_only(
            events::target_created_automation_event(target_info.clone()),
        ));
    }
    push_committed_auto_attached_session_events(
        conn,
        &mut out,
        &tab_sessions,
        &tab_target_id,
        tab_info,
    );
    push_committed_auto_attached_session_events(
        conn,
        &mut out,
        &page_sessions,
        target_id,
        target_info,
    );
    out
}

async fn ensure_popup_initial_document_page_async(
    conn: &mut CdpConnection,
    target_id: &str,
) -> bool {
    let Some(route) = conn.target_session_route_for_target_id(target_id) else {
        return false;
    };
    let owner = CommandOwnerScope::for_route(route);
    {
        let pending = match conn.start_initial_document_page_ensure_for_owner(&owner) {
            Ok(pending) => pending,
            Err(message) => {
                tracing::debug!(
                    target_id,
                    ?message,
                    "failed to start popup initial document page ensure"
                );
                return false;
            }
        };
        if let Some(pending) = pending {
            let completed = match pending.wait().await {
                Ok(completed) => completed,
                Err(failed) => {
                    let message = conn.reset_failed_initial_document_page_build_for_owner(failed);
                    tracing::debug!(
                        target_id,
                        ?message,
                        "failed to await popup initial document page ensure"
                    );
                    return false;
                }
            };
            if let Err(message) = conn
                .complete_initial_document_page_build_for_owner(completed)
                .await
            {
                tracing::debug!(
                    target_id,
                    ?message,
                    "failed to complete popup initial document page ensure"
                );
                return false;
            }
        }
    }
    true
}

fn push_committed_auto_attached_session_events(
    conn: &mut CdpConnection,
    out: &mut impl events::CdpTargetAutomationEventSink,
    sessions: &[(Option<String>, String, CdpSessionRoute)],
    target_id: &str,
    target_info: DevToolsTargetInfo,
) {
    let sessions = sessions
        .iter()
        .map(|(owner_session_id, session_id, route)| {
            TargetAttachSessionCommit::auto_attached(
                session_id.clone(),
                owner_session_id.clone(),
                route.clone(),
                conn.auto_attach_owner_waits_for_debugger_on_start(owner_session_id.as_deref()),
            )
        })
        .collect::<Vec<_>>();
    let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
        target_id,
        target_info,
        sessions,
    ));
    for event in event_plan {
        out.push_target_background_event(event);
    }
}

pub(crate) fn schedule_navigation_decision_after_debugger_resume(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> bool {
    let Some((_, Some(target_id))) = conn.target_owner_identity_for_session(session_id) else {
        return false;
    };
    schedule_navigation_decision_after_debugger_barrier_release_for_target(conn, &target_id)
}

pub(crate) fn schedule_navigation_decision_after_debugger_barrier_release_for_target(
    conn: &mut CdpConnection,
    target_id: &str,
) -> bool {
    if conn.target_has_waiting_for_debugger_session(target_id) {
        return false;
    }
    let Some((_, paused)) = conn.native_navigation_decision_for_target(target_id) else {
        return false;
    };
    if !matches!(
        paused.stage,
        moli_core::browser::NavigationDecisionStage::Request { .. }
    ) {
        return false;
    }
    let Some(browser_context_id) = conn.browser_context_id_for_target(target_id) else {
        return false;
    };
    let Some(action) = TargetStartupOwnerAction::capture(conn, browser_context_id, target_id)
    else {
        return false;
    };
    conn.publish_target_startup_owner_action(action);
    true
}

pub(crate) async fn complete_target_startup_owner_action_async(
    conn: &mut CdpConnection,
    action: TargetStartupOwnerAction,
) -> crate::conn::CdpTurnOutcome {
    let (contents, permit) = action.decision();
    let protocol_events = conn
        .project_browser_navigation_decision(contents, Some(permit))
        .await;
    crate::conn::CdpTurnOutcome::new_with_protocol_events(
        protocol_events,
        conn.take_scheduler_events(),
    )
}

pub(crate) fn emit_target_info_changed_for_owner_background_event(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
) {
    out.extend(conn.target_info_changed_event_plan_for_owner(owner));
}
