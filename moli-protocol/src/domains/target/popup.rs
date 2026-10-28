use crate::conn::{
    CdpSessionRoute, CommandOwnerScope, PopupTargetActivationAction, PopupTargetNavigationKind,
    PopupTargetNavigationOwnerAction, PreparedTargetAttach, TargetAttachSessionCommit,
};
use anyhow::Result;

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

pub(crate) enum PreparedPopupTarget {
    Existing(String),
    New(NewPopupTarget),
}

pub(crate) struct NewPopupTarget {
    browser_context_id: String,
    target_id: String,
    tab_target_id: String,
    auto_attached_tab_sessions: Vec<(Option<String>, String, CdpSessionRoute)>,
    auto_attached_page_sessions: Vec<(Option<String>, String, CdpSessionRoute)>,
    requested_url: String,
    disposition: moli_core::page::RendererPopupDisposition,
}

impl PreparedPopupTarget {
    pub(crate) async fn run_using(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        out: &mut Vec<BackgroundProtocolEvent>,
    ) -> Result<Option<String>> {
        let Self::New(prepared) = self else {
            let Self::Existing(target_id) = self else {
                unreachable!()
            };
            return Ok(Some(target_id));
        };
        let target_id = prepared.target_id.clone();
        let browser_context_id = prepared.browser_context_id.clone();
        let ensure_target = target_id.clone();
        let start =
            driver.with_state(move |state| state.start_popup_initial_document(ensure_target))?;
        let ensured = match start {
            Some(Some(pending)) => match pending.wait().await {
                Ok(completed) => match completed.install_using(driver).await {
                    Ok(_) => true,
                    Err(error) => {
                        tracing::debug!(%error, %target_id, "failed to install popup initial document");
                        false
                    }
                },
                Err(failed) => {
                    driver.with_state(move |state| state.fail_popup_initial_document(failed))?;
                    false
                }
            },
            Some(None) => true,
            None => false,
        };
        let target = if ensured {
            let (target, events) =
                driver.with_state(move |state| state.finish_popup_target(prepared))?;
            out.extend(events);
            target
        } else {
            None
        };
        if target.is_none() {
            let pending = driver.with_state(move |state| {
                state.prepare_popup_rollback(browser_context_id, target_id)
            })?;
            pending.run_using(driver).await?;
        }
        Ok(target)
    }
}

pub(crate) fn prepare_popup_target(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    creation: PopupTargetCreation,
) -> Option<PreparedPopupTarget> {
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
    let Some(browser_context) = conn
        .browser_context_by_id(&browser_context_id)
        .filter(|context| !context.closing)
    else {
        tracing::debug!(
            browser_context_id,
            ?popup_id,
            ?target_name,
            "dropping accepted popup action after its browser context started closing or was removed"
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
            return Some(PreparedPopupTarget::Existing(existing_target_id));
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
        return target_url_updated.then_some(PreparedPopupTarget::Existing(existing_target_id));
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

    Some(PreparedPopupTarget::New(NewPopupTarget {
        browser_context_id,
        target_id,
        tab_target_id,
        auto_attached_tab_sessions,
        auto_attached_page_sessions,
        requested_url,
        disposition,
    }))
}

pub(crate) fn finish_popup_target(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    prepared: NewPopupTarget,
) -> Option<String> {
    let NewPopupTarget {
        browser_context_id,
        target_id,
        tab_target_id,
        auto_attached_tab_sessions,
        auto_attached_page_sessions,
        requested_url,
        disposition,
    } = prepared;
    let target_info = conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|browser_context| browser_context.devtools_target_info(&target_id))?;
    let tab_target_info = conn.tab_target_info(&tab_target_id)?;
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

pub(super) fn prepare_initial_target_url_navigation(
    conn: &mut CdpConnection,
    target_id: &str,
) -> Option<PreparedPopupNavigation> {
    if conn.target_has_waiting_for_debugger_session(target_id) {
        return None;
    }
    let route = conn.target_session_route_for_target_id(target_id)?;
    let browser_context_id = route.browser_context_id()?.to_owned();
    let browser_context = conn.browser_context_by_id(&browser_context_id)?;
    if !browser_context.target_needs_initial_document_navigation(target_id) {
        return None;
    }
    let target_url = browser_context.devtools_target_info(target_id)?.url;
    let action = PopupTargetNavigationOwnerAction::capture(
        conn,
        &browser_context_id,
        target_id,
        target_url,
        PopupTargetNavigationKind::InitialDocument,
    )?;
    PreparedPopupNavigation::prepare(conn, action)
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

pub(crate) struct PreparedPopupNavigation {
    navigation: crate::domains::page::PreparedRendererNavigation,
    completion: Option<(CommandOwnerScope, crate::conn::PageTargetHostIdentity)>,
}

impl PreparedPopupNavigation {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        action: PopupTargetNavigationOwnerAction,
    ) -> Option<Self> {
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
        if !target_is_current
            || !popup_target_has_loaded_page(conn, &browser_context_id, &target_id)
        {
            tracing::debug!(
                browser_context_id,
                target_id,
                url,
                ?kind,
                "dropping popup navigation after its exact target owner retired"
            );
            return None;
        }

        match kind {
            PopupTargetNavigationKind::InitialDocument
            | PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume => {
                // Revalidate the barrier when the queued owner action actually
                // runs. Another inspector session can attach after this action is
                // scheduled; that new session must be able to pause the initial
                // document before any target-URL request starts.
                if conn.target_has_waiting_for_debugger_session(&target_id)
                    || !conn.browser_context_by_id(&browser_context_id).is_some_and(
                        |browser_context| {
                            browser_context.target_needs_initial_document_navigation(&target_id)
                        },
                    )
                {
                    return None;
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
        let completion = matches!(
            kind,
            PopupTargetNavigationKind::InitialDocument
                | PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume
        )
        .then(|| {
            conn.page_target_host_identity_for_owner(&owner_scope)
                .map(|host| (owner_scope.clone(), host))
        })
        .flatten();
        let navigation = crate::domains::page::prepare_renderer_navigation_request(
            conn,
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
        );
        Some(Self {
            navigation,
            completion,
        })
    }

    pub(crate) async fn run(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        self.navigation.run(driver, context).await?;
        if let Some((owner, host)) = self.completion {
            let events =
                driver.with_state(move |state| state.finish_popup_navigation(owner, host))?;
            context.context_mut().protocol_events_mut().extend(events);
        }
        Ok(())
    }
}

pub(crate) fn prepare_popup_target_activation(
    conn: &mut CdpConnection,
    action: &PopupTargetActivationAction,
) -> Result<Option<super::activation::PreparedTargetActivation>, crate::automation::DevToolsError> {
    if !action.is_current(conn)
        || !popup_target_has_loaded_page(conn, action.browser_context_id(), action.target_id())
    {
        return Ok(None);
    }
    super::activation::prepare_target_activation_for_target(conn, action.target_id())
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
