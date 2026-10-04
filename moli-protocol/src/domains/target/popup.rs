use crate::conn::{
    CdpSessionRoute, CommandOwnerScope, PopupTargetActivationAction, PopupTargetNavigationKind,
    PopupTargetNavigationOwnerAction, PreparedTargetAttach, TargetAttachSessionCommit,
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
    activation: moli_core::page::RendererPendingPopupActivation,
    opener: Option<PopupTargetOpenerIdentity>,
}

impl PopupTargetCreation {
    pub(crate) fn new(
        browser_context_id: String,
        activation: moli_core::page::RendererPendingPopupActivation,
        opener: Option<PopupTargetOpenerIdentity>,
    ) -> Self {
        Self {
            browser_context_id,
            activation,
            opener,
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
        activation,
        opener,
    } = creation;
    let moli_core::page::RendererPopupActivationParts {
        source,
        disposition,
        navigation_requested,
        navigation_initiator,
        same_origin_with_target,
        popup_id,
        browsing_context_name,
        auxiliary_window,
        document_response,
        initial_document_environment,
        pending_auxiliary_page,
        url,
        target_name,
        session_storage_store,
        initial_empty_document_storage_key,
    } = activation.into_parts();
    let exposes_opener = matches!(
        source,
        moli_core::page::RendererPopupActivationSource::Window {
            exposes_opener: true,
            ..
        }
    );
    let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
        tracing::debug!(
            browser_context_id,
            ?popup_id,
            ?target_name,
            "dropping accepted popup action after its browser context was removed"
        );
        return None;
    };

    // The renderer has already chosen a browsing context. Its name may have
    // changed before this accepted action is consumed, so only actions that
    // lack a concrete identity may perform name resolution here.
    let popup_id = auxiliary_window
        .as_ref()
        .map(|window| window.id())
        .or(popup_id);
    let existing_target_id = match (browsing_context_name.as_ref(), popup_id) {
        (Some(name), _) => browser_context.target_id_for_browsing_context_name(name),
        (None, Some(popup_id)) => browser_context.target_id_for_popup_id(popup_id),
        (None, None) if exposes_opener => browser_context.target_id_for_window_name(&target_name),
        (None, None) => None,
    }
    .map(str::to_owned);
    if let Some(existing_target_id) = existing_target_id {
        if !navigation_requested {
            if disposition != moli_core::page::RendererPopupDisposition::Background
                && let Some(activation) = PopupTargetActivationAction::capture(
                    conn,
                    &browser_context_id,
                    &existing_target_id,
                )
            {
                conn.publish_popup_target_activation_action(activation);
            }
            return Some(existing_target_id);
        }
        // Compare the current Document before publishing the requested URL.
        // A same-origin caller navigating a named Window to its current URL
        // replaces the entry; a cross-origin caller must not learn that URL
        // through a change in history.length (Chromium crbug.com/1208614).
        let replace_current = browser_context
            .page_target(&existing_target_id)
            .zip(navigation_initiator.as_ref())
            .is_some_and(|(target, initiator)| {
                target
                    .loaded_page()
                    .is_some_and(|page| page.final_url().as_str() == url)
                    && same_origin_with_target.unwrap_or_else(|| {
                        moli_url::WebOrigin::from_serialized(
                            target.target_identity().security_origin(),
                        )
                        .same_origin(&initiator.origin())
                    })
            });
        let navigation =
            popup_target_has_loaded_page(conn, &browser_context_id, &existing_target_id)
                .then(|| {
                    PopupTargetNavigationOwnerAction::capture(
                        conn,
                        &browser_context_id,
                        &existing_target_id,
                        url.clone(),
                        PopupTargetNavigationKind::NamedTargetReuse { replace_current },
                    )
                })
                .flatten();
        let activation = (disposition != moli_core::page::RendererPopupDisposition::Background)
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
                conn.publish_popup_target_navigation_owner_action(
                    navigation
                        .with_document_response(document_response)
                        .with_initial_document_environment(initial_document_environment)
                        .with_navigation_initiator(navigation_initiator),
                );
            }
            if let Some(activation) = activation {
                conn.publish_popup_target_activation_action(activation);
            }
        }
        return target_url_updated.then_some(existing_target_id);
    }

    if browsing_context_name.is_some() && auxiliary_window.is_none() {
        // An ordinary related Page was selected, then retired before projection.
        // It must never be recreated as a new auxiliary target.
        return None;
    }

    // The renderer has already accepted an auxiliary-context action. Even when
    // noopener blocks script access, Chromium preserves the creator target and
    // frame as DevTools attribution for the new popup Page target.
    let opener = opener.filter(|opener| {
        conn.browser_context_by_id(&browser_context_id)
            .and_then(|browser_context| browser_context.devtools_target_info(&opener.target_id))
            .is_some()
    });
    let can_access_opener = exposes_opener && opener.is_some();
    let popup_creator = if can_access_opener || pending_auxiliary_page.is_some() {
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
        if let Some(page) = pending_auxiliary_page.as_ref() {
            assert_eq!(
                Some(page.popup_id()),
                popup_id,
                "popup adoption must preserve its accepted identity"
            );
        }
        let initial_empty = pending_auxiliary_page.is_some() && exposes_opener;
        let window_id = if disposition == moli_core::page::RendererPopupDisposition::NewWindow {
            browser_context.page_targets.allocate_window_id()
        } else {
            opener
                .as_ref()
                .and_then(|opener| browser_context.page_target(&opener.target_id))
                .map(|target| target.window_id)
                .unwrap_or_else(|| browser_context.page_targets.default_window_id())
        };
        browser_context.stage_popup_background_target(
            target_id.clone(),
            auto_attached_background_session_id.clone(),
            url,
            Some("about:blank".to_owned()),
            popup_creator,
            session_storage_store,
            initial_empty_document_storage_key,
            pending_auxiliary_page,
            window_id,
        );
        let history = &mut browser_context
            .page_target_mut(&target_id)
            .expect("staged popup target")
            .owner_state
            .navigation_history_state;
        history.mark_auxiliary_document_entry(initial_empty);
        if requested_url != "about:blank" {
            history.mark_replace_initial_empty_document("link");
        }
        if let Some(opener) = opener {
            browser_context.remember_target_opener(
                &target_id,
                opener.target_id,
                opener.frame_id,
                can_access_opener,
            );
        }
        if let Some(name) = browsing_context_name {
            browser_context.bind_target_browsing_context_name(&target_id, name);
        } else {
            browser_context.remember_target_window_name(&target_name, &target_id);
        }
        if let Some(window) = auxiliary_window {
            browser_context
                .renderer_runtime()
                .bind_auxiliary_window(&target_id, window);
        }
        browser_context.remember_target_popup_id(popup_id, &target_id);
        browser_context
            .page_target_mut(&target_id)?
            .owner_state
            .pending_popup_navigation = Some(crate::conn::PendingPopupNavigation {
            url: requested_url.clone(),
            response: document_response,
            initiator: navigation_initiator,
        });
    }

    let tab_target_id = conn.register_top_level_page_target(&target_id);
    let auto_attached_tab_sessions = auto_attached_tab_sessions
        .into_iter()
        .map(|(owner_session_id, session_id)| {
            let route = conn.prepare_auto_attached_tab_session_binding(
                &tab_target_id,
                session_id.clone(),
                owner_session_id.as_deref(),
            );
            let route = route.expect("created popup tab target must remain addressable");
            (owner_session_id, session_id, route)
        })
        .collect::<Vec<_>>();
    let auto_attached_page_sessions = auto_attached_page_sessions
        .into_iter()
        .enumerate()
        .map(|(index, (owner_session_id, session_id))| {
            let route = if index == 0 && auto_attached_background_session_id.is_some() {
                CdpSessionRoute::PageTarget {
                    browser_context_id: browser_context_id.clone(),
                    target_id: target_id.clone(),
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                }
            } else {
                conn.prepare_auto_attached_page_session_binding_in_browser_context(
                    &browser_context_id,
                    &target_id,
                    session_id.clone(),
                )
                .expect("newly created popup target must remain addressable")
            };
            (owner_session_id, session_id, route)
        })
        .collect::<Vec<_>>();

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
    if disposition != moli_core::page::RendererPopupDisposition::Background
        && let Some(activation) =
            PopupTargetActivationAction::capture(conn, &browser_context_id, &target_id)
    {
        conn.publish_popup_target_activation_action(activation);
    }
    Some(target_id)
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
    let (
        owner_scope,
        browser_context_id,
        target_id,
        url,
        kind,
        mut document_response,
        initial_document_environment,
        mut navigation_initiator,
    ) = action.into_parts();
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
        PopupTargetNavigationKind::NamedTargetReuse { .. }
        | PopupTargetNavigationKind::WindowReference(_) => {}
    }
    if matches!(
        kind,
        PopupTargetNavigationKind::InitialDocument
            | PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume
    ) && let Some(pending) = conn
        .browser_context_by_id_mut(&browser_context_id)
        .and_then(|context| context.page_target_mut(&target_id))
        .and_then(|target| target.owner_state.pending_popup_navigation.take())
        .filter(|pending| pending.url == url)
    {
        document_response = pending.response;
        navigation_initiator = pending.initiator;
    }
    let auxiliary_navigation = match kind {
        PopupTargetNavigationKind::WindowReference(kind) => Some(kind),
        PopupTargetNavigationKind::NamedTargetReuse { replace_current } => {
            Some(if replace_current {
                moli_core::page::RendererAuxiliaryNavigationKind::Replace
            } else {
                moli_core::page::RendererAuxiliaryNavigationKind::Assign
            })
        }
        _ => None,
    };
    let request_kind = if matches!(
        kind,
        PopupTargetNavigationKind::WindowReference(
            moli_core::page::RendererAuxiliaryNavigationKind::Reload
        )
    ) {
        moli_fetch::BrowserNavigationRequestKind::Reload
    } else {
        moli_fetch::BrowserNavigationRequestKind::Navigate
    };
    crate::domains::page::navigate_command_owner_from_renderer_request_background_events_async(
        conn,
        &mut protocol_events,
        owner_scope,
        &url,
        "GET",
        None,
        &[],
        request_kind,
        None,
        auxiliary_navigation,
        document_response,
        initial_document_environment,
        navigation_initiator,
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
    let target_is_current = conn
        .target_owner_identity_for_owner(&owner_scope)
        .is_some_and(|(current_browser_context_id, current_target_id)| {
            current_browser_context_id == browser_context_id
                && current_target_id.as_deref() == Some(target_id.as_str())
        })
        && popup_target_has_loaded_page(conn, &browser_context_id, &target_id);
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
        conn.select_browser_focus_for_target(target_id);
        conn.apply_browser_document_activity_async()
            .await
            .map_err(|error| error.to_string())?;
        Ok(Vec::new())
    } else {
        match conn
            .select_page_target_for_connection_async(target_id)
            .await
        {
            Ok(Some(activation)) => Ok(activation.into_protocol_events()),
            Ok(None) => Err("PopupTargetUnavailable".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    };
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    result
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
