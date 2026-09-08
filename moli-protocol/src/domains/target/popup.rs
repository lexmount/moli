use crate::conn::{
    CdpSessionRoute, CommandOwnerScope, PopupTargetNavigationKind,
    PopupTargetNavigationOwnerAction, PreparedTargetAttach, TargetAttachSessionCommit,
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
    if snapshot.document.is_none()
        && !conn.target_has_waiting_for_debugger_session(target_id)
        && let Some(navigation) = PopupTargetNavigationOwnerAction::capture(
            conn,
            &browser_context_id,
            target_id,
            snapshot
                .popup
                .as_ref()
                .expect("popup creation")
                .requested_url
                .clone(),
            PopupTargetNavigationKind::InitialDocument,
        )
    {
        conn.publish_popup_target_navigation_owner_action(navigation);
    }
    out
}

pub(crate) fn observe_reused_popup_navigation(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    browser_context_id: &str,
    target_id: &str,
    url: &str,
) {
    let navigation = popup_target_has_loaded_page(conn, browser_context_id, target_id)
        .then(|| {
            PopupTargetNavigationOwnerAction::capture(
                conn,
                browser_context_id,
                target_id,
                url.to_owned(),
                PopupTargetNavigationKind::NamedTargetReuse,
            )
        })
        .flatten();
    if conn
        .browser_context_by_id_mut(browser_context_id)
        .is_some_and(|context| context.update_target_url(target_id, url.to_owned()))
    {
        emit_target_info_changed_for_target_background_event(
            conn,
            out,
            browser_context_id,
            target_id,
        );
        if let Some(navigation) = navigation {
            conn.publish_popup_target_navigation_owner_action(navigation);
        }
    }
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

pub(super) async fn start_target_url_navigation_if_allowed_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    target_id: &str,
) {
    if conn.target_has_waiting_for_debugger_session(target_id) {
        return;
    }
    let Some(route) = conn.target_session_route_for_target_id(target_id) else {
        return;
    };
    let Some(browser_context_id) = route.browser_context_id().map(str::to_owned) else {
        return;
    };
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        return;
    };
    if !browser_context.target_needs_initial_document_navigation(target_id) {
        return;
    }
    let Some(target_url) = browser_context
        .devtools_target_info(target_id)
        .map(|target_info| target_info.url)
    else {
        return;
    };
    let owner_scope = CommandOwnerScope::for_route(route);
    crate::domains::page::navigate_command_owner_from_renderer_background_events_async(
        conn,
        out,
        &owner_scope,
        &target_url,
    )
    .await;
    emit_target_info_changed_for_target_background_event(conn, out, &browser_context_id, target_id);
}

pub(crate) fn schedule_initial_document_target_url_navigation_after_debugger_resume(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> bool {
    let Some((_, Some(target_id))) = conn.target_owner_identity_for_session(session_id) else {
        return false;
    };
    schedule_initial_document_target_url_navigation_after_debugger_barrier_release_for_target(
        conn, &target_id,
    )
}

pub(crate) fn schedule_initial_document_target_url_navigation_after_debugger_barrier_release_for_target(
    conn: &mut CdpConnection,
    target_id: &str,
) -> bool {
    if conn.target_has_waiting_for_debugger_session(target_id) {
        return false;
    }
    let Some(route) = conn.target_session_route_for_target_id(target_id) else {
        return false;
    };
    if !matches!(&route, crate::conn::CdpSessionRoute::PageTarget { .. }) {
        return false;
    }
    let Some(browser_context_id) = route.browser_context_id().map(str::to_owned) else {
        return false;
    };
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        return false;
    };
    if !browser_context.target_needs_initial_document_navigation(target_id) {
        return false;
    }
    let Some(target_url) = browser_context
        .devtools_target_info(target_id)
        .map(|target_info| target_info.url)
    else {
        return false;
    };
    let Some(action) = PopupTargetNavigationOwnerAction::capture(
        conn,
        &browser_context_id,
        target_id,
        target_url,
        PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume,
    ) else {
        return false;
    };
    conn.publish_popup_target_navigation_owner_action(action);
    true
}

fn popup_target_has_loaded_page(
    conn: &CdpConnection,
    browser_context_id: &str,
    target_id: &str,
) -> bool {
    let Some(browser_context) = conn.browser_context_by_id(browser_context_id) else {
        return false;
    };
    browser_context.target_has_loaded_page(target_id)
}

pub(crate) async fn complete_popup_target_navigation_owner_action_async(
    conn: &mut CdpConnection,
    action: PopupTargetNavigationOwnerAction,
) -> crate::conn::CdpTurnOutcome {
    let (owner_scope, browser_context_id, target_id, url, kind) = action.into_parts();
    let target_is_current = conn
        .target_owner_identity_for_owner(&owner_scope)
        .is_some_and(|(current_browser_context_id, current_target_id)| {
            current_browser_context_id == browser_context_id
                && current_target_id.as_deref() == Some(target_id.as_str())
        });
    if !target_is_current || !popup_target_has_loaded_page(conn, &browser_context_id, &target_id) {
        tracing::debug!(
            browser_context_id,
            target_id,
            url,
            ?kind,
            "dropping popup navigation after its exact target owner retired"
        );
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            Vec::new(),
            conn.take_scheduler_events(),
        );
    }

    let mut protocol_events = Vec::new();
    match kind {
        PopupTargetNavigationKind::InitialDocument
        | PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume => {
            // Revalidate the barrier when the queued owner action actually
            // runs. Another inspector session can attach after this action is
            // scheduled; that new session must be able to pause the initial
            // document before any target-URL request starts.
            if conn.target_has_waiting_for_debugger_session(&target_id)
                || !conn
                    .browser_context_by_id(&browser_context_id)
                    .is_some_and(|browser_context| {
                        browser_context.target_needs_initial_document_navigation(&target_id)
                    })
            {
                return crate::conn::CdpTurnOutcome::new_with_protocol_events(
                    Vec::new(),
                    conn.take_scheduler_events(),
                );
            }
        }
        PopupTargetNavigationKind::NamedTargetReuse => {}
    }
    crate::domains::page::navigate_command_owner_from_renderer_background_events_async(
        conn,
        &mut protocol_events,
        &owner_scope,
        &url,
    )
    .await;
    if matches!(
        kind,
        PopupTargetNavigationKind::InitialDocument
            | PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume
    ) {
        emit_target_info_changed_for_target_background_event(
            conn,
            &mut protocol_events,
            &browser_context_id,
            &target_id,
        );
    }
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

fn emit_target_info_changed_for_target_background_event(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    browser_context_id: &str,
    target_id: &str,
) {
    out.extend(
        conn.target_info_changed_event_plan_for_observable_target(browser_context_id, target_id),
    );
}
