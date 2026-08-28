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
    requested_url: String,
    destination_request: Option<moli_core::page::RendererTopLevelNavigationRequest>,
    reports_requested_url_without_destination: bool,
    target_name: String,
    opener: Option<PopupTargetOpenerIdentity>,
    can_access_opener: bool,
    disposition: moli_core::page::RendererPopupDisposition,
    navigation_referrer: Option<String>,
    initial_document_referrer: Option<String>,
    document_referrer: Option<String>,
    pending_auxiliary_page: Option<moli_core::page::RendererPendingAuxiliaryPage>,
    resolved_target_page: Option<moli_core::page::RendererResolvedPopupTarget>,
    new_target_disposition: Option<moli_core::page::RendererPopupNewTargetDisposition>,
    auxiliary_browsing_context_policy:
        Option<moli_core::page::RendererAuxiliaryBrowsingContextPolicy>,
    service_worker_clients_open_window_continuation:
        Option<moli_core::page::RendererServiceWorkerClientsOpenWindowContinuation>,
    session_storage_store: Option<moli_core::network::SharedWebStorageStore>,
    initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
    drain_pending_javascript_tasks_before_commit: bool,
}

impl PopupTargetCreation {
    pub(crate) fn new(
        browser_context_id: String,
        popup_id: Option<u64>,
        requested_url: String,
        destination_request: Option<moli_core::page::RendererTopLevelNavigationRequest>,
        reports_requested_url_without_destination: bool,
        target_name: String,
        opener: Option<PopupTargetOpenerIdentity>,
        can_access_opener: bool,
        disposition: moli_core::page::RendererPopupDisposition,
        navigation_referrer: Option<String>,
        initial_document_referrer: Option<String>,
        document_referrer: Option<String>,
        pending_auxiliary_page: Option<moli_core::page::RendererPendingAuxiliaryPage>,
        resolved_target_page: Option<moli_core::page::RendererResolvedPopupTarget>,
        new_target_disposition: Option<moli_core::page::RendererPopupNewTargetDisposition>,
        auxiliary_browsing_context_policy: Option<
            moli_core::page::RendererAuxiliaryBrowsingContextPolicy,
        >,
        service_worker_clients_open_window_continuation: Option<
            moli_core::page::RendererServiceWorkerClientsOpenWindowContinuation,
        >,
        session_storage_store: Option<moli_core::network::SharedWebStorageStore>,
        initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
        drain_pending_javascript_tasks_before_commit: bool,
    ) -> Self {
        Self {
            browser_context_id,
            popup_id,
            requested_url,
            destination_request,
            reports_requested_url_without_destination,
            target_name,
            opener,
            can_access_opener,
            disposition,
            navigation_referrer,
            initial_document_referrer,
            document_referrer,
            pending_auxiliary_page,
            resolved_target_page,
            new_target_disposition,
            auxiliary_browsing_context_policy,
            service_worker_clients_open_window_continuation,
            session_storage_store,
            initial_empty_document_storage_key,
            drain_pending_javascript_tasks_before_commit,
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
        requested_url,
        destination_request,
        reports_requested_url_without_destination,
        target_name,
        opener,
        can_access_opener,
        disposition,
        navigation_referrer,
        initial_document_referrer,
        document_referrer,
        pending_auxiliary_page,
        resolved_target_page,
        new_target_disposition,
        auxiliary_browsing_context_policy,
        service_worker_clients_open_window_continuation,
        session_storage_store,
        initial_empty_document_storage_key,
        drain_pending_javascript_tasks_before_commit,
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

    let existing_target_id = if let Some(resolved_target_page) = resolved_target_page {
        let Some(target_id) =
            browser_context.target_id_for_renderer_popup_target(resolved_target_page)
        else {
            tracing::debug!(
                browser_context_id,
                ?popup_id,
                ?target_name,
                owner_local_host_id = resolved_target_page.owner_local_host_id().as_u64(),
                page_id = resolved_target_page.page_id().as_u64(),
                "dropping named popup reuse after its exact renderer Page residence disappeared"
            );
            return None;
        };
        Some(target_id)
    } else if new_target_disposition.is_none() {
        // Unmigrated producers still use the browser-side name projection,
        // even when they optimistically reserved a Page. Migrated producers explicitly record
        // a fresh renderer decision so it cannot be redirected through this
        // map merely because an unrelated target has the same name.
        browser_context
            .target_id_for_window_name(&target_name)
            .map(str::to_owned)
    } else {
        None
    };

    if let Some(existing_target_id) = existing_target_id {
        let navigation = destination_request.as_ref().and_then(|request| {
            popup_target_has_loaded_page(conn, &browser_context_id, &existing_target_id)
                .then(|| {
                    PopupTargetNavigationOwnerAction::capture(
                        conn,
                        &browser_context_id,
                        &existing_target_id,
                        request.clone(),
                        navigation_referrer.clone(),
                        document_referrer.clone(),
                        None,
                        PopupTargetNavigationKind::NamedTargetReuse,
                        service_worker_clients_open_window_continuation.clone(),
                        drain_pending_javascript_tasks_before_commit,
                    )
                })
                .flatten()
        });

        if destination_request.is_none() {
            let resolved = remember_resolved_popup_target(
                conn,
                &browser_context_id,
                popup_id,
                &existing_target_id,
            );
            if resolved {
                publish_popup_target_activation_if_foreground(
                    conn,
                    &browser_context_id,
                    &existing_target_id,
                    disposition,
                );
            }
            return resolved.then_some(existing_target_id);
        }

        let target_url_updated = conn
            .browser_context_by_id_mut(&browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.update_target_url(&existing_target_id, requested_url.clone())
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
            publish_popup_target_activation_if_foreground(
                conn,
                &browser_context_id,
                &existing_target_id,
                disposition,
            );
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
    let waits_for_debugger_on_start = auto_attach_page_owners.iter().any(|owner_session_id| {
        conn.auto_attach_owner_waits_for_debugger_on_start(owner_session_id.as_deref())
    });
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
    {
        let browser_context = conn.browser_context_by_id_mut(&browser_context_id)?;
        browser_context.stage_popup_background_target(
            target_id.clone(),
            auto_attached_background_session_id.clone(),
            if destination_request.is_some() || reports_requested_url_without_destination {
                requested_url.clone()
            } else {
                String::new()
            },
            Some("about:blank".to_owned()),
            popup_creator,
            pending_auxiliary_page,
            auxiliary_browsing_context_policy,
            session_storage_store,
            initial_empty_document_storage_key,
        );
        // A popup's synchronously exposed about:blank is its initial empty
        // Document. The first cross-document commit replaces that entry; a
        // no-commit response must leave this pending replacement intact.
        browser_context.mark_target_initial_url_replaces_empty_document(&target_id);
        if let Some(opener) = opener {
            browser_context.remember_target_opener(
                &target_id,
                opener.target_id,
                opener.frame_id,
                can_access_opener,
            );
        }
        if new_target_disposition.is_none_or(|disposition| !disposition.is_fresh()) {
            browser_context.remember_target_window_name(&target_name, &target_id);
        }
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

    let Some(creation_diagnostics) = ensure_popup_initial_document_page_async(
        conn,
        &target_id,
        initial_document_referrer.as_deref(),
        new_target_disposition
            .is_some_and(|disposition| disposition.carries_initial_name())
            .then_some(target_name.as_str()),
    )
    .await
    else {
        rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id).await;
        return None;
    };

    let top_level_browsing_context_closing =
        creation_diagnostics.top_level_browsing_context_closing;
    let captured_initial_navigation = creation_diagnostics
        .initial_top_level_navigation
        .map(|navigation| *navigation);
    if captured_initial_navigation.is_some() && destination_request.is_some() {
        tracing::error!(
            browser_context_id,
            target_id,
            "popup admission received duplicate activation and target-local initial navigation authorities"
        );
        rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id).await;
        return None;
    }
    if captured_initial_navigation.is_some()
        && service_worker_clients_open_window_continuation.is_some()
    {
        tracing::error!(
            browser_context_id,
            target_id,
            "window-owned target-local navigation carried a ServiceWorker continuation"
        );
        rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id).await;
        return None;
    }
    let (
        destination_request,
        navigation_history_entry_seed,
        navigation_referrer,
        document_referrer,
    ) = if let Some(navigation) = captured_initial_navigation {
        (
            Some(navigation.request().clone()),
            navigation.navigation_history_entry_seed().cloned(),
            None,
            None,
        )
    } else {
        (
            destination_request,
            None,
            navigation_referrer,
            document_referrer,
        )
    };

    if destination_request.is_none() {
        if !conn.stage_popup_target_without_destination_navigation(&target_id) {
            rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id)
                .await;
            return None;
        }
        let target_id = finish_popup_target_creation(
            conn,
            out,
            &browser_context_id,
            target_id,
            tab_target_id,
            auto_attached_tab_sessions,
            auto_attached_page_sessions,
        )
        .await?;
        publish_popup_target_activation_if_foreground(
            conn,
            &browser_context_id,
            &target_id,
            disposition,
        );
        return Some(target_id);
    }
    let request = destination_request.expect("destination request was checked");
    let navigation = if top_level_browsing_context_closing {
        // `open(url); popup.close()` may run completely inside the opener's
        // synchronous V8 turn, before protocol admits the auxiliary Page. Its
        // close record remains in the staged Page FIFO; do not start a network
        // navigation for a browsing context already marked Closing.
        None
    } else {
        let Some(navigation) = PopupTargetNavigationOwnerAction::capture(
            conn,
            &browser_context_id,
            &target_id,
            request,
            navigation_referrer,
            document_referrer,
            navigation_history_entry_seed,
            PopupTargetNavigationKind::InitialDocument,
            service_worker_clients_open_window_continuation,
            drain_pending_javascript_tasks_before_commit,
        ) else {
            rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id)
                .await;
            return None;
        };
        Some(navigation)
    };
    if let Some(navigation) = navigation
        && !conn.stage_initial_popup_target_navigation_owner_action(navigation)
    {
        rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id).await;
        return None;
    }
    let navigation = if top_level_browsing_context_closing || waits_for_debugger_on_start {
        None
    } else {
        let navigation = conn.take_held_popup_target_navigation_owner_action_for_target(&target_id);
        if navigation.is_none() {
            rollback_incomplete_popup_target_async(conn, Some(&browser_context_id), &target_id)
                .await;
            return None;
        }
        navigation
    };

    let target_id = finish_popup_target_creation(
        conn,
        out,
        &browser_context_id,
        target_id,
        tab_target_id,
        auto_attached_tab_sessions,
        auto_attached_page_sessions,
    )
    .await?;
    if let Some(navigation) = navigation {
        conn.publish_popup_target_navigation_owner_action(navigation);
    }
    publish_popup_target_activation_if_foreground(
        conn,
        &browser_context_id,
        &target_id,
        disposition,
    );
    Some(target_id)
}

fn publish_popup_target_activation_if_foreground(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    target_id: &str,
    disposition: moli_core::page::RendererPopupDisposition,
) {
    if disposition == moli_core::page::RendererPopupDisposition::Foreground
        && let Some(activation) =
            PopupTargetActivationAction::capture(conn, browser_context_id, target_id)
    {
        conn.publish_popup_target_activation_action(activation);
    }
}

async fn finish_popup_target_creation(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    browser_context_id: &str,
    target_id: String,
    tab_target_id: String,
    auto_attached_tab_sessions: Vec<(Option<String>, String)>,
    auto_attached_page_sessions: Vec<(Option<String>, String)>,
) -> Option<String> {
    let Some(target_info) = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|browser_context| browser_context.devtools_target_info(&target_id))
    else {
        rollback_incomplete_popup_target_async(conn, Some(browser_context_id), &target_id).await;
        return None;
    };
    let Some(tab_target_info) = conn.tab_target_info(&tab_target_id) else {
        rollback_incomplete_popup_target_async(conn, Some(browser_context_id), &target_id).await;
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
    initial_document_referrer: Option<&str>,
    initial_top_level_browsing_context_name: Option<&str>,
) -> Option<crate::conn::LoadedPageCreationDiagnosticsParts> {
    let route = conn.target_session_route_for_target_id(target_id)?;
    {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
        let pending = match route_scope
            .conn_mut()
            .start_initial_document_page_ensure_with_environment_for_session_owner(
                None,
                initial_document_referrer,
                initial_top_level_browsing_context_name,
            ) {
            Ok(pending) => pending,
            Err(message) => {
                tracing::debug!(
                    target_id,
                    ?message,
                    "failed to start popup initial document page ensure"
                );
                return None;
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
                    return None;
                }
            };
            let diagnostics = match route_scope
                .conn_mut()
                .complete_initial_document_page_build_for_owner_with_creation_diagnostics(completed)
                .await
            {
                Ok(diagnostics) => diagnostics,
                Err(message) => {
                    tracing::debug!(
                        target_id,
                        ?message,
                        "failed to complete popup initial document page ensure"
                    );
                    return None;
                }
            };
            return Some(diagnostics);
        }
    }
    Some(crate::conn::LoadedPageCreationDiagnosticsParts::default())
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
    if conn.auto_attach_wait_for_debugger_on_start {
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
    if let Some(action) =
        conn.take_held_popup_target_navigation_owner_action_for_session_owner(session_id)
    {
        // `runIfWaitingForDebugger` is itself the explicit target-owner
        // admission turn. Consume the same typed action here so the resumed
        // request is observable with that command, while still forbidding a
        // mutable target-URL rescan.
        return Box::pin(
            execute_popup_target_navigation_owner_action_background_events_async(
                conn, out, session_id, action,
            ),
        )
        .await;
    }
    if conn.runtime_session_owner_has_popup_target_navigation_authority(session_id) {
        return false;
    }
    if !conn.runtime_session_owner_should_start_initial_document_navigation(session_id) {
        return false;
    }
    let Some(target_url) = conn.runtime_session_owner_target_url(session_id) else {
        return false;
    };
    // Target.createTarget reaches this boundary while its completed initial
    // document build and response plan are still live. Keep the full Page
    // navigation state machine out of that target-command future's stack.
    Box::pin(
        crate::domains::page::navigate_session_owner_from_renderer_background_events_async(
            conn,
            out,
            session_id,
            &target_url,
        ),
    )
    .await;
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
    let mut protocol_events = Vec::new();
    Box::pin(
        execute_popup_target_navigation_owner_action_background_events_async(
            conn,
            &mut protocol_events,
            None,
            action,
        ),
    )
    .await;
    crate::conn::CdpTurnOutcome::new_with_protocol_events(
        protocol_events,
        conn.take_scheduler_events(),
    )
}

async fn execute_popup_target_navigation_owner_action_background_events_async(
    conn: &mut CdpConnection,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    execution_session_id: Option<&str>,
    action: PopupTargetNavigationOwnerAction,
) -> bool {
    let (owner_scope, claim, service_worker_clients_open_window_continuation) = action.into_parts();
    let browser_context_id = claim.browser_context_id().to_owned();
    let target_id = claim.target_id().to_owned();
    let request = claim.request().clone();
    let url = request.url().to_owned();
    let referrer = claim.referrer().map(str::to_owned);
    let document_referrer = claim.document_referrer().map(str::to_owned);
    let navigation_history_entry_seed = claim.navigation_history_entry_seed().cloned();
    let kind = claim.kind();
    let drain_pending_javascript_tasks_before_commit =
        claim.drain_pending_javascript_tasks_before_commit();
    let mut route_scope = owner_scope.enter(conn);
    let conn = route_scope.conn_mut();
    if kind == PopupTargetNavigationKind::InitialDocument
        && !conn.consume_published_popup_target_navigation_claim_for_session_owner(
            execution_session_id,
            &claim,
        )
    {
        tracing::debug!(
            browser_context_id,
            target_id,
            url,
            ?kind,
            "dropping popup navigation without its exact published target authority"
        );
        return false;
    }
    let target_is_current = conn
        .target_owner_identity_for_session(execution_session_id)
        .is_some_and(|(current_browser_context_id, current_target_id)| {
            current_browser_context_id == browser_context_id
                && current_target_id.as_deref() == Some(target_id.as_str())
        });
    let page_is_current = conn.target_page_residence_identity_is_current_for_session(
        execution_session_id,
        claim.page_owner(),
    );
    if !target_is_current
        || !page_is_current
        || !popup_target_has_loaded_page(conn, &browser_context_id, &target_id)
    {
        tracing::debug!(
            browser_context_id,
            target_id,
            url,
            ?kind,
            page_owner = ?claim.page_owner(),
            "dropping popup navigation after its exact target Page residence retired"
        );
        return false;
    }

    match kind {
        PopupTargetNavigationKind::InitialDocument => {
            if !conn.runtime_session_owner_should_start_claimed_popup_initial_document_navigation(
                execution_session_id,
                &url,
            ) {
                return false;
            }
        }
        PopupTargetNavigationKind::NamedTargetReuse => {}
    }
    Box::pin(
        crate::domains::page::navigate_session_owner_from_renderer_request_background_events_async(
            conn,
            protocol_events,
            execution_session_id,
            &url,
            request.source(),
            referrer.as_deref(),
            document_referrer.as_deref(),
            request.request_method(),
            request.request_body(),
            request.request_headers(),
            request.browser_navigation_kind(),
            navigation_history_entry_seed.as_ref(),
            service_worker_clients_open_window_continuation,
            drain_pending_javascript_tasks_before_commit,
        ),
    )
    .await;
    if kind == PopupTargetNavigationKind::InitialDocument {
        emit_target_info_changed_for_target_background_event(
            conn,
            protocol_events,
            &browser_context_id,
            &target_id,
        );
    }
    true
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
