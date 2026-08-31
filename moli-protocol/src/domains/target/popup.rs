use crate::conn::{
    PopupTargetActivationAction, PopupTargetNavigationKind, PopupTargetNavigationOwnerAction,
    PreparedTargetAttach,
};

use super::creation::{
    push_target_created_events, top_level_page_auto_attach_owner_sessions,
    top_level_tab_auto_attach_owner_sessions,
};
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PopupTargetOpenerIdentity {
    target_id: String,
    frame_id: String,
}

impl PopupTargetOpenerIdentity {
    pub(crate) fn new(target_id: impl Into<String>, frame_id: impl Into<String>) -> Self {
        Self {
            target_id: target_id.into(),
            frame_id: frame_id.into(),
        }
    }
}

/// A renderer-accepted auxiliary browsing-context action whose destination
/// browser context, DevTools opener identity, and DOM opener access were
/// frozen before protocol emission.
///
/// Unlike a protocol Target.createTarget command, this action must not select
/// the browser context or opener from whichever session happens to drain it.
#[derive(Clone, Debug)]
pub(crate) struct PopupTargetCreation {
    browser_context_id: String,
    popup_id: Option<u64>,
    url: String,
    target_name: String,
    opener: Option<PopupTargetOpenerIdentity>,
    can_access_opener: bool,
    disposition: moli_core::page::RendererPopupDisposition,
    session_storage_store: Option<moli_core::network::SharedWebStorageStore>,
    initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
}

impl PopupTargetCreation {
    pub(crate) fn new(
        browser_context_id: String,
        popup_id: Option<u64>,
        url: String,
        target_name: String,
        opener: Option<PopupTargetOpenerIdentity>,
        can_access_opener: bool,
        disposition: moli_core::page::RendererPopupDisposition,
        session_storage_store: Option<moli_core::network::SharedWebStorageStore>,
        initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Self {
        Self {
            browser_context_id,
            popup_id,
            url,
            target_name,
            opener,
            can_access_opener,
            disposition,
            session_storage_store,
            initial_empty_document_storage_key,
        }
    }
}

pub(crate) async fn create_popup_target_from_renderer_output_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    creation: PopupTargetCreation,
) -> Option<String> {
    let PopupTargetCreation {
        browser_context_id,
        popup_id,
        url,
        target_name,
        opener,
        can_access_opener,
        disposition,
        session_storage_store,
        initial_empty_document_storage_key,
    } = creation;
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        tracing::debug!(
            browser_context_id,
            ?popup_id,
            ?target_name,
            "dropping accepted popup action after its browser context was removed"
        );
        return None;
    };

    if let Some(existing_target_id) = browser_context
        .target_id_for_window_name(&target_name)
        .map(str::to_owned)
    {
        let navigation =
            popup_target_has_loaded_page(conn, &browser_context_id, &existing_target_id)
                .then(|| {
                    PopupTargetNavigationOwnerAction::capture(
                        conn,
                        &browser_context_id,
                        &existing_target_id,
                        url.clone(),
                        PopupTargetNavigationKind::NamedTargetReuse,
                    )
                })
                .flatten();
        let activation = (disposition == moli_core::page::RendererPopupDisposition::Foreground)
            .then(|| {
                PopupTargetActivationAction::capture(conn, &browser_context_id, &existing_target_id)
            })
            .flatten();

        let target_url_updated = conn
            .browser_context_by_id_mut(&browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.update_target_url(&existing_target_id, url.clone())
            });
        if target_url_updated {
            emit_target_info_changed_for_target_background_event(
                conn,
                out,
                &browser_context_id,
                &existing_target_id,
            );
            if let Some(navigation) = navigation {
                conn.publish_popup_target_navigation_owner_action(navigation);
            }
            if let Some(activation) = activation {
                conn.publish_popup_target_activation_action(activation);
            }
        }
        return (target_url_updated
            && remember_resolved_popup_target(
                conn,
                &browser_context_id,
                popup_id,
                &existing_target_id,
            ))
        .then_some(existing_target_id);
    }

    // The renderer has already accepted an auxiliary-context action. Even when
    // noopener blocks script access, Chromium preserves the creator target and
    // frame as DevTools attribution for the new auxiliary target.
    let opener = opener.filter(|opener| {
        conn.browser_context_by_id(&browser_context_id)
            .and_then(|browser_context| browser_context.devtools_target_info(&opener.target_id))
            .is_some()
    });
    let can_access_opener = can_access_opener && opener.is_some();
    let popup_creator = if can_access_opener {
        opener.as_ref().and_then(|opener| {
            conn.browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| {
                    browser_context.initial_empty_document_creator_for_target(&opener.target_id)
                })
        })
    } else {
        None
    };
    let target_id = conn.gen_target_id();
    let auto_attach_page_owners = top_level_page_auto_attach_owner_sessions(conn);
    let auto_attach_tab_owners = top_level_tab_auto_attach_owner_sessions(conn);
    let auto_attached_page_sessions = auto_attach_page_owners
        .iter()
        .map(|owner_session_id| (owner_session_id.clone(), conn.gen_session_id()))
        .collect::<Vec<_>>();
    let auto_attached_tab_sessions = auto_attach_tab_owners
        .iter()
        .map(|owner_session_id| (owner_session_id.clone(), conn.gen_session_id()))
        .collect::<Vec<_>>();
    let auto_attached_background_session_id = auto_attached_page_sessions
        .first()
        .map(|(_, session_id)| session_id.clone());
    let requested_url = url.clone();

    {
        let browser_context = conn.browser_context_by_id_mut(&browser_context_id)?;
        browser_context.stage_popup_background_target(
            target_id.clone(),
            auto_attached_background_session_id.clone(),
            url,
            Some("about:blank".to_owned()),
            popup_creator,
            session_storage_store,
            initial_empty_document_storage_key,
        );
        if let Some(opener) = opener {
            browser_context.remember_target_opener(
                &target_id,
                opener.target_id,
                opener.frame_id,
                can_access_opener,
            );
        }
        browser_context.remember_target_window_name(&target_name, &target_id);
        browser_context.remember_target_popup_id(popup_id, &target_id);
    }

    let tab_target_id = conn.register_top_level_page_target(&target_id);
    for (owner_session_id, session_id) in &auto_attached_tab_sessions {
        let assigned = conn.prepare_auto_attached_tab_session_binding(
            &tab_target_id,
            session_id.clone(),
            owner_session_id.as_deref(),
        );
        assert!(assigned, "created popup tab target must remain addressable");
    }
    for (index, (_, session_id)) in auto_attached_page_sessions.iter().enumerate() {
        if index != 0 || auto_attached_background_session_id.is_none() {
            let assigned = conn.prepare_auto_attached_page_session_binding_in_browser_context(
                &browser_context_id,
                &target_id,
                session_id.clone(),
            );
            assert!(
                assigned,
                "newly created popup target must remain addressable"
            );
        }
    }

    if !ensure_popup_initial_document_page_async(conn, &target_id).await {
        rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id).await;
        return None;
    }

    let Some(target_info) = conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|browser_context| browser_context.devtools_target_info(&target_id))
    else {
        rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id).await;
        return None;
    };
    let Some(tab_target_info) = conn.tab_target_info(&tab_target_id) else {
        rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id).await;
        return None;
    };
    if conn.has_any_target_discovery() {
        push_target_created_events(conn, out, &target_id);
    } else {
        // Chromium's BiDi mapper keeps a target observer alive independently
        // of whether any frontend subscribed to `Target.targetCreated`.
        // Preserve that separation here: CDP discovery controls only the CDP
        // notification, while the accepted auxiliary browsing-context action
        // always publishes one typed automation lifecycle fact. This is
        // especially important for popup creation that settles after the
        // causing Runtime command response.
        out.push(BackgroundProtocolEvent::automation_only(
            events::target_created_automation_event(target_info.clone()),
        ));
    }
    push_committed_auto_attached_session_events(
        conn,
        out,
        &auto_attached_tab_sessions,
        &tab_target_id,
        tab_target_info,
    );
    push_committed_auto_attached_session_events(
        conn,
        out,
        &auto_attached_page_sessions,
        &target_id,
        target_info,
    );
    if !conn.target_has_waiting_for_debugger_session(&target_id)
        && let Some(navigation) = PopupTargetNavigationOwnerAction::capture(
            conn,
            &browser_context_id,
            &target_id,
            requested_url,
            PopupTargetNavigationKind::InitialDocument,
        )
    {
        conn.publish_popup_target_navigation_owner_action(navigation);
    }
    if disposition == moli_core::page::RendererPopupDisposition::Foreground
        && let Some(activation) =
            PopupTargetActivationAction::capture(conn, &browser_context_id, &target_id)
    {
        conn.publish_popup_target_activation_action(activation);
    }
    Some(target_id)
}

fn remember_resolved_popup_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    popup_id: Option<u64>,
    target_id: &str,
) -> bool {
    conn.browser_context_by_id_mut(browser_context_id)
        .is_some_and(|browser_context| {
            if browser_context.devtools_target_info(target_id).is_none() {
                return false;
            }
            browser_context.remember_target_popup_id(popup_id, target_id);
            true
        })
}
async fn ensure_popup_initial_document_page_async(
    conn: &mut CdpConnection,
    target_id: &str,
) -> bool {
    let Some(route) = conn.target_session_route_for_target_id(target_id) else {
        return false;
    };
    {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
        let pending = match route_scope
            .conn_mut()
            .start_initial_document_page_ensure_for_session_owner(None)
        {
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
                    let message = route_scope
                        .conn_mut()
                        .reset_failed_initial_document_page_build_for_owner(failed);
                    tracing::debug!(
                        target_id,
                        ?message,
                        "failed to await popup initial document page ensure"
                    );
                    return false;
                }
            };
            if let Err(message) = route_scope
                .conn_mut()
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
    sessions: &[(Option<String>, String)],
    target_id: &str,
    target_info: DevToolsTargetInfo,
) {
    let sessions = sessions
        .iter()
        .map(|(owner_session_id, session_id)| {
            conn.prepare_auto_attach_session_commit(
                session_id.clone(),
                owner_session_id.clone(),
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

pub(super) async fn rollback_incomplete_popup_target_async(
    conn: &mut CdpConnection,
    browser_context_id: Option<&str>,
    target_id: &str,
) {
    conn.rollback_incomplete_popup_target_without_event_async(browser_context_id, target_id)
        .await;
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
    let started_target_url_navigation = {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        start_initial_document_target_url_navigation_if_needed_background_events_async(
            route_scope.conn_mut(),
            out,
            None,
        )
        .await
    };
    if started_target_url_navigation
        && let Some(browser_context_id) = conn
            .target_session_route_for_target_id(target_id)
            .and_then(|route| route.browser_context_id().map(str::to_owned))
    {
        emit_target_info_changed_for_target_background_event(
            conn,
            out,
            &browser_context_id,
            target_id,
        );
    }
}

pub(crate) async fn start_initial_document_target_url_navigation_if_needed_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
) -> bool {
    if !conn.runtime_session_owner_should_start_initial_document_navigation(session_id) {
        return false;
    }
    let Some(target_url) = conn.runtime_session_owner_target_url(session_id) else {
        return false;
    };
    crate::domains::page::navigate_session_owner_from_renderer_background_events_async(
        conn,
        out,
        session_id,
        &target_url,
    )
    .await;
    true
}

pub(crate) fn schedule_initial_document_target_url_navigation_after_debugger_resume(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> bool {
    if !conn.runtime_session_owner_should_start_initial_document_navigation(session_id) {
        return false;
    }
    let Some(target_url) = conn.runtime_session_owner_target_url(session_id) else {
        return false;
    };
    let Some((browser_context_id, Some(target_id))) =
        conn.target_owner_identity_for_session(session_id)
    else {
        return false;
    };
    let Some(action) = PopupTargetNavigationOwnerAction::capture(
        conn,
        &browser_context_id,
        &target_id,
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
    if browser_context.is_active_target(target_id) {
        return browser_context.has_loaded_page();
    }
    browser_context
        .background_target(target_id)
        .is_some_and(|target| target.has_loaded_page())
}

pub(crate) async fn complete_popup_target_navigation_owner_action_async(
    conn: &mut CdpConnection,
    action: PopupTargetNavigationOwnerAction,
) -> crate::conn::CdpTurnOutcome {
    let (owner_scope, browser_context_id, target_id, url, kind) = action.into_parts();
    let mut route_scope = owner_scope.enter(conn);
    let conn = route_scope.conn_mut();
    let target_is_current = conn.target_owner_identity_for_session(None).is_some_and(
        |(current_browser_context_id, current_target_id)| {
            current_browser_context_id == browser_context_id
                && current_target_id.as_deref() == Some(target_id.as_str())
        },
    );
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
            if !conn.runtime_session_owner_should_start_initial_document_navigation(None) {
                return crate::conn::CdpTurnOutcome::new_with_protocol_events(
                    Vec::new(),
                    conn.take_scheduler_events(),
                );
            }
        }
        PopupTargetNavigationKind::NamedTargetReuse => {}
    }
    crate::domains::page::navigate_session_owner_from_renderer_background_events_async(
        conn,
        &mut protocol_events,
        None,
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

pub(crate) async fn complete_popup_target_activation_action_async(
    conn: &mut CdpConnection,
    action: PopupTargetActivationAction,
) -> crate::conn::CdpTurnOutcome {
    let (owner_scope, browser_context_id, target_id) = action.into_parts();
    let target_is_current = {
        let mut route_scope = owner_scope.enter(conn);
        let conn = route_scope.conn_mut();
        conn.target_owner_identity_for_session(None).is_some_and(
            |(current_browser_context_id, current_target_id)| {
                current_browser_context_id == browser_context_id
                    && current_target_id.as_deref() == Some(target_id.as_str())
            },
        ) && popup_target_has_loaded_page(conn, &browser_context_id, &target_id)
    };
    if !target_is_current {
        tracing::debug!(
            browser_context_id,
            target_id,
            "dropping popup activation after its exact target owner retired"
        );
        return crate::conn::CdpTurnOutcome::new_with_protocol_events(
            Vec::new(),
            conn.take_scheduler_events(),
        );
    }
    let protocol_events =
        match activate_popup_target_async(conn, &browser_context_id, &target_id).await {
            Ok(events) => events,
            Err(error) => {
                tracing::debug!(
                    browser_context_id,
                    target_id,
                    %error,
                    "popup target could not be activated"
                );
                Vec::new()
            }
        };
    crate::conn::CdpTurnOutcome::new_with_protocol_events(
        protocol_events,
        conn.take_scheduler_events(),
    )
}

async fn activate_popup_target_async(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    target_id: &str,
) -> Result<Vec<BackgroundProtocolEvent>, String> {
    let restore_browser_context_id = previously_active_browser_context_id(conn);
    let result = if let Err(message) = select_browser_context_for_target(conn, target_id) {
        Err(message.to_owned())
    } else if conn
        .browser_context
        .as_ref()
        .is_none_or(|browser_context| browser_context.id != browser_context_id)
    {
        Err("PopupTargetBrowserContextChanged".to_owned())
    } else if conn
        .browser_context
        .as_ref()
        .is_some_and(|browser_context| browser_context.is_active_target(target_id))
    {
        Ok(Vec::new())
    } else {
        match conn
            .promote_background_target_to_active_for_connection_async(target_id)
            .await
        {
            Ok(Some(activation)) => Ok(activation.into_protocol_events()),
            Ok(None) => Err("PopupTargetUnavailable".to_owned()),
            Err(message) => Err(message),
        }
    };
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    result
}

pub(crate) fn emit_target_info_changed_for_session_owner_background_event(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
) {
    out.extend(conn.target_info_changed_event_plan_for_session_owner(session_id));
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
