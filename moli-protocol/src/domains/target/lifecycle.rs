use serde::Deserialize;

use crate::conn::{
    BackgroundProtocolEvent, CdpTargetFilter, CdpTargetFilterEntry, PopupTargetActivationAction,
    PopupTargetNavigationKind, PopupTargetNavigationOwnerAction, PreparedTargetAttach,
    TargetAttachSessionCommit,
};
use crate::devtools_runtime::{
    DevToolsActivateTargetCommand, DevToolsBrowserContextId, DevToolsCloseTargetCommand,
    DevToolsCloseTargetResult, DevToolsCommand, DevToolsCommandResult, DevToolsCreateTargetCommand,
    DevToolsCreateTargetResult, DevToolsError, DevToolsErrorKind, DevToolsGetTargetInfoCommand,
    DevToolsGetTargetInfoResult, DevToolsTargetId, DevToolsTargetInfo,
};

use crate::domains::command_output::CommandOutputPlan;

use super::*;

pub(super) struct DevToolsCreateTargetExecution {
    pub(super) result: DevToolsCreateTargetResult,
    pub(super) protocol_events: CreatedTargetProtocolEvents,
}

pub(super) struct CreatedTargetProtocolEvents {
    target_id: String,
    attached_tab_sessions: Vec<TargetAttachSessionCommit>,
    attached_sessions: Vec<TargetAttachSessionCommit>,
}

impl CreatedTargetProtocolEvents {
    pub(super) fn target_id(&self) -> &str {
        &self.target_id
    }
}

pub(super) fn emit_created_target_protocol_events(
    conn: &mut CdpConnection,
    events: CreatedTargetProtocolEvents,
    out: &mut impl events::CdpTargetAutomationEventSink,
) -> Result<(), DevToolsError> {
    let has_discovery = conn.has_any_target_discovery();
    if !has_discovery
        && events.attached_tab_sessions.is_empty()
        && events.attached_sessions.is_empty()
    {
        return Ok(());
    }
    let bc = conn
        .browser_contexts()
        .find(|browser_context| {
            browser_context
                .devtools_target_info(&events.target_id)
                .is_some()
        })
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId"))?;
    let target_info = bc
        .devtools_target_info(&events.target_id)
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId"))?;
    if let Some(message) = super::transient_no_page_devtools_target_info_error(conn, &target_info) {
        return Err(DevToolsError::new(DevToolsErrorKind::Internal, message));
    }
    if has_discovery {
        for event in conn.target_created_event_plan(&events.target_id) {
            out.push_target_background_event(event);
        }
    }
    if let Some(tab_target_info) = conn.tab_target_info_for_page_target_info(&target_info) {
        let tab_target_id = tab_target_info
            .target_id
            .as_ref()
            .map(|target_id| target_id.as_str().to_owned())
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::Internal, "MissingTabTargetId"))?;
        let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
            tab_target_id,
            tab_target_info,
            events.attached_tab_sessions,
        ));
        for event in event_plan {
            out.push_target_background_event(event);
        }
    }
    let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
        events.target_id,
        target_info,
        events.attached_sessions,
    ));
    for event in event_plan {
        out.push_target_background_event(event);
    }
    Ok(())
}

fn push_target_created_events(
    conn: &mut CdpConnection,
    out: &mut impl events::CdpTargetAutomationEventSink,
    page_target_id: &str,
) {
    for event in conn.target_created_event_plan(page_target_id) {
        out.push_target_background_event(event);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTargetParams {
    #[serde(default = "default_blank")]
    url: String,
    browser_context_id: Option<String>,
}

fn default_blank() -> String {
    "about:blank".into()
}

pub(super) fn start_create_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    if let Some(session_id) = cmd.session_id
        && matches!(
            conn.session_route(Some(session_id)),
            Some(
                crate::conn::CdpSessionRoute::SharedWorkerTarget { .. }
                    | crate::conn::CdpSessionRoute::DedicatedWorkerTarget { .. }
            )
        )
    {
        return super::target_command_error(-31998, "DirectSessionRouteRequired");
    }

    let params: CreateTargetParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        Ok(None) => CreateTargetParams {
            url: "about:blank".into(),
            browser_context_id: None,
        },
        Err(e) => {
            return super::target_command_error(-32602, e);
        }
    };
    let command = build_cdp_create_target_command(cmd, params);
    super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::CreateTarget(command),
    )
}

fn build_cdp_create_target_command(
    cmd: &Cmd<'_>,
    params: CreateTargetParams,
) -> DevToolsCreateTargetCommand {
    DevToolsCreateTargetCommand {
        context: cmd.devtools_command_context(None::<&str>, params.browser_context_id.as_deref()),
        url: params.url,
        browser_context_id: params
            .browser_context_id
            .map(DevToolsBrowserContextId::from),
        activate: false,
    }
}

pub(super) fn start_devtools_create_target_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCreateTargetCommand,
) -> TargetCommandTaskStep {
    let mut plan = CommandOutputPlan::default();
    let execution = execute_devtools_create_target_command(conn, command);
    let (created_target_id, created_target_protocol_events) = match execution {
        Ok(execution) => {
            let target_id = execution.result.target_id.clone();
            plan.extend(CommandOutputPlan::from_devtools_result(
                DevToolsCommandResult::CreateTarget(execution.result),
            ));
            (target_id, execution.protocol_events)
        }
        Err(error) => {
            plan.extend(CommandOutputPlan::from_devtools_error(error));
            return TargetCommandTaskStep::Complete(plan);
        }
    };
    let initial_document_route =
        conn.target_session_route_for_target_id(created_target_id.as_str());
    let pending_initial_document = if let Some(route) = initial_document_route.clone() {
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        route_scope
            .conn_mut()
            .start_initial_document_page_ensure_for_session_owner(None)
    } else {
        Ok(None)
    };
    match pending_initial_document {
        Ok(Some(initial_document)) => {
            TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
                command_id,
                session_id: command_session_id.map(str::to_owned),
                kind: Box::new(PendingTargetCommandKind::CreateTarget {
                    response_plan: plan,
                    protocol_events: created_target_protocol_events,
                    initial_document_route,
                    initial_document: Some(Box::new(initial_document)),
                }),
            })
        }
        Ok(None) => {
            let mut output_plan = CommandOutputPlan::default();
            let mut protocol_events = Vec::new();
            if let Err(error) = emit_created_target_protocol_events(
                conn,
                created_target_protocol_events,
                &mut protocol_events,
            ) {
                return TargetCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
                    error,
                ));
            }
            for event in protocol_events {
                output_plan.push_background_event(event);
            }
            output_plan.extend(plan);
            TargetCommandTaskStep::Complete(output_plan)
        }
        Err(message) => TargetCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message)),
    }
}

pub(super) fn execute_devtools_create_target_command(
    conn: &mut CdpConnection,
    command: DevToolsCreateTargetCommand,
) -> Result<DevToolsCreateTargetExecution, DevToolsError> {
    let restore_browser_context_id = previously_active_browser_context_id(conn);
    if let Err(error) = activate_browser_context_for_create_target(conn, &command) {
        restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
        return Err(error);
    }
    if conn.browser_context.is_none() && conn.inactive_browser_contexts.is_empty() {
        let id = conn.gen_bc_id();
        conn.insert_browser_context(conn.new_browser_context(id));
    }

    let target_id = conn.gen_target_id();
    let default_target_id = conn.default_target_id();
    let browser_context = conn
        .browser_context
        .as_ref()
        .expect("browser context must exist before target creation");
    let has_active_target = browser_context.active_target_identity().is_some()
        && !browser_context.active_target_is_unclaimed_default_placeholder(default_target_id);
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
    let creating_background_target = has_active_target && !command.activate;
    let auto_attached_background_session_id = if creating_background_target {
        auto_attached_page_sessions
            .first()
            .map(|(_, session_id)| session_id.clone())
    } else {
        None
    };
    let activating_created_target = has_active_target && command.activate;
    let initial_empty_document_url = create_target_initial_empty_document_url(&command.url);
    if activating_created_target {
        conn.handoff_navigation_engine_for_active_target_demotion();
    }
    {
        let bc = conn.browser_context.as_mut().unwrap();
        if creating_background_target {
            bc.stage_background_target(
                target_id.clone(),
                auto_attached_background_session_id.clone(),
                command.url.clone(),
                Some(initial_empty_document_url.clone()),
                None,
            );
        } else if activating_created_target {
            bc.stage_active_target_demoting_current(
                target_id.clone(),
                None,
                command.url.clone(),
                Some(initial_empty_document_url.clone()),
            );
        } else {
            bc.set_active_target_id(target_id.clone());
            bc.set_target_url(command.url.clone());
            bc.begin_active_target_initial_empty_document(initial_empty_document_url.clone());
            bc.active_target.owner_state.target_crash_state.clear();
        }
        if command.url != initial_empty_document_url {
            // Chromium gives Target.createTarget(url) one initial
            // auto_toplevel history entry for url. The implementation still
            // materializes an internal about:blank document for target setup,
            // so replace that bookkeeping entry when the requested URL commits.
            bc.mark_target_initial_url_replaces_empty_document(&target_id);
        }
    }
    let tab_target_id = conn.register_top_level_page_target(&target_id);
    let created_target_id = target_id.clone();

    let mut attached_tab_sessions = Vec::new();
    for (owner_session_id, session_id) in &auto_attached_tab_sessions {
        let waiting_for_debugger =
            conn.auto_attach_owner_waits_for_debugger_on_start(owner_session_id.as_deref());
        let assigned = conn.prepare_auto_attached_tab_session_binding(
            &tab_target_id,
            session_id.clone(),
            owner_session_id.as_deref(),
        );
        debug_assert!(assigned, "created tab target must remain addressable");
        attached_tab_sessions.push(conn.prepare_auto_attach_session_commit(
            session_id.clone(),
            owner_session_id.clone(),
            waiting_for_debugger,
        ));
    }

    let mut attached_sessions = Vec::new();
    for (index, (owner_session_id, session_id)) in auto_attached_page_sessions.iter().enumerate() {
        let waiting_for_debugger =
            conn.auto_attach_owner_waits_for_debugger_on_start(owner_session_id.as_deref());
        if creating_background_target && index == 0 {
            attached_sessions.push(conn.prepare_auto_attach_session_commit(
                session_id.clone(),
                owner_session_id.clone(),
                waiting_for_debugger,
            ));
        } else {
            let assigned =
                conn.prepare_auto_attached_page_session_binding(&target_id, session_id.clone());
            debug_assert!(
                assigned,
                "newly created target must remain addressable for auto attach"
            );
            attached_sessions.push(conn.prepare_auto_attach_session_commit(
                session_id.clone(),
                owner_session_id.clone(),
                waiting_for_debugger,
            ));
        }
    }
    if activating_created_target {
        conn.apply_active_engine_fetch_overrides();
        conn.invalidate_resource_runtime();
    }

    // Chromium parity: Target.createTarget acks from target lifecycle, while
    // the initial document page build remains target-owned pending work. CDP
    // observation commands must only see an already-installed Page or a real
    // no-document state; they do not create the initial document themselves.
    // See content/browser/devtools/protocol/target_handler.cc:1291.
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());

    Ok(DevToolsCreateTargetExecution {
        result: DevToolsCreateTargetResult {
            target_id: DevToolsTargetId::from(created_target_id),
        },
        protocol_events: CreatedTargetProtocolEvents {
            target_id,
            attached_tab_sessions,
            attached_sessions,
        },
    })
}

fn create_target_initial_empty_document_url(target_url: &str) -> String {
    if url::Url::parse(target_url)
        .ok()
        .as_ref()
        .is_some_and(moli_url::is_about_blank)
    {
        target_url.to_owned()
    } else {
        "about:blank".to_owned()
    }
}

fn top_level_page_auto_attach_owner_sessions(conn: &CdpConnection) -> Vec<Option<String>> {
    conn.auto_attach_owner_sessions_for_target_type("page")
        .into_iter()
        .filter(|owner_session_id| {
            super::browser_level_auto_attach_owner_session_allowed(
                conn,
                owner_session_id.as_deref(),
            )
        })
        .collect()
}

fn top_level_tab_auto_attach_owner_sessions(conn: &CdpConnection) -> Vec<Option<String>> {
    conn.auto_attach_owner_sessions_for_target_type("tab")
        .into_iter()
        .filter(|owner_session_id| {
            super::browser_level_auto_attach_owner_session_allowed(
                conn,
                owner_session_id.as_deref(),
            )
        })
        .collect()
}

fn activate_browser_context_for_create_target(
    conn: &mut CdpConnection,
    command: &DevToolsCreateTargetCommand,
) -> Result<(), DevToolsError> {
    if let Some(wanted_id) = command.browser_context_id.as_ref() {
        if wanted_id.as_str() == conn.default_browser_context_id()
            && !conn.has_browser_context_id(wanted_id.as_str())
        {
            conn.insert_browser_context(conn.new_browser_context(wanted_id.as_str().to_owned()));
        }
        if conn.activate_browser_context_by_id(wanted_id.as_str()) {
            return Ok(());
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "UnknownBrowserContextId",
        ));
    }

    let Some(reference_target_id) = command.context.target_id.as_ref() else {
        return Ok(());
    };
    let Some(route) = conn.target_session_route_for_target_id(reference_target_id.as_str()) else {
        if conn
            .target_session_route_for_child_frame_id(reference_target_id.as_str())
            .is_some()
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "ReferenceContextNotTopLevel",
            ));
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    };
    let Some(browser_context_id) = route.browser_context_id() else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    };
    if conn.activate_browser_context_by_id(browser_context_id) {
        Ok(())
    } else {
        Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ))
    }
}

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
    if !conn.auto_attach_wait_for_debugger_on_start
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
        PopupTargetNavigationKind::InitialDocument => {
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
    if kind == PopupTargetNavigationKind::InitialDocument {
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
    if let Err(error) = activate_popup_target_async(conn, &browser_context_id, &target_id).await {
        tracing::debug!(
            browser_context_id,
            target_id,
            %error,
            "popup target could not be activated"
        );
    }
    crate::conn::CdpTurnOutcome::new_with_protocol_events(Vec::new(), conn.take_scheduler_events())
}

async fn activate_popup_target_async(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    target_id: &str,
) -> Result<(), String> {
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
        Ok(())
    } else {
        match conn
            .promote_background_target_to_active_for_connection_async(target_id)
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) => Err("PopupTargetUnavailable".to_owned()),
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloseTargetParams {
    target_id: String,
}

pub(super) fn start_close_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: CloseTargetParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    let command = build_cdp_close_target_command(cmd, params);
    super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::CloseTarget(command),
    )
}

fn build_cdp_close_target_command(
    cmd: &Cmd<'_>,
    params: CloseTargetParams,
) -> DevToolsCloseTargetCommand {
    DevToolsCloseTargetCommand {
        context: cmd.devtools_command_context(Some(params.target_id.as_str()), None::<&str>),
        target_id: DevToolsTargetId::from(params.target_id),
    }
}

pub(super) fn start_devtools_close_target_command(
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCloseTargetCommand,
) -> TargetCommandTaskStep {
    pending_close_target_command(command_id, command_session_id, command)
}

pub(super) async fn complete_close_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCloseTargetCommand,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> CommandOutputPlan {
    let mut side_effects = events::TargetProtocolSideEffects::default();
    match execute_devtools_close_target_command_async(
        conn,
        command,
        &mut side_effects,
        command_context,
    )
    .await
    {
        Ok(result) => {
            let mut plan =
                CommandOutputPlan::from_devtools_result(DevToolsCommandResult::CloseTarget(result));
            plan.extend(side_effects.into_plan());
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) async fn execute_devtools_close_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCloseTargetCommand,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> Result<DevToolsCloseTargetResult, DevToolsError> {
    let target_id = command.target_id.into_string();
    let restore_browser_context_id = previously_active_browser_context_id(conn);
    let result = close_target_inner_async(conn, out, command_context, target_id).await;
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    result
}

async fn close_target_inner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
    target_id: String,
) -> Result<DevToolsCloseTargetResult, DevToolsError> {
    let target_id = conn
        .page_target_id_for_tab_target_id(&target_id)
        .map(str::to_owned)
        .unwrap_or(target_id);
    if let Err(message) = select_browser_context_for_target(conn, &target_id) {
        return Err(DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message));
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_shared_worker_target(&target_id))
    {
        if !worker_target::close_shared_worker_target_for_target_close_async(
            conn,
            &target_id,
            command_context,
        )
        .await
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownTargetId",
            ));
        }
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_dedicated_worker_target(&target_id))
    {
        if !worker_target::close_dedicated_worker_target_for_target_close_async(
            conn,
            &target_id,
            command_context,
        )
        .await
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownTargetId",
            ));
        }
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn
        .browser_context
        .as_ref()
        .is_some_and(|bc| bc.has_service_worker_target(&target_id))
    {
        return Ok(DevToolsCloseTargetResult { success: true });
    }
    if conn.browser_context.as_ref().is_some_and(|bc| {
        !matches!(
            bc.active_target_identity(),
            Some((ref active_target_id, _)) if active_target_id == &target_id
        )
    }) {
        let target_route = conn
            .target_session_route_for_target_id(&target_id)
            .ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId")
            })?;
        let session_id = conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.background_target(&target_id))
            .and_then(|target| target.session_id().map(str::to_owned));
        let owner_scope = crate::conn::CommandOwnerScope::from_session_and_owner_route(
            session_id.as_deref(),
            session_id.is_none().then_some(target_route),
        );
        let renderer_output_predecessor =
            events::fail_pending_fetch_state_for_target_background_events_async(
                conn,
                out.background_events_mut(),
                session_id.as_deref(),
                "Target closed",
            )
            .await;
        settle_target_close_after_pending_fetches_async(
            conn,
            out,
            command_context,
            renderer_output_predecessor,
            owner_scope,
            target_id,
        )
        .await;
        return Ok(DevToolsCloseTargetResult { success: true });
    }

    let session_id = match conn.browser_context.as_ref() {
        Some(bc) => {
            if !bc.has_active_target() && bc.background_targets.is_empty() {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "TargetNotLoaded",
                ));
            }
            match bc.active_target_identity() {
                Some((_active_target_id, session_id)) => session_id,
                None => {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::NoSuchTarget,
                        "TargetNotLoaded",
                    ));
                }
            }
        }
        None => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "BrowserContextNotLoaded",
            ));
        }
    };

    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        out.background_events_mut(),
        command_context.protocol_events_mut(),
        session_id.as_deref(),
        "Target closed",
    );
    let renderer_output_predecessor =
        events::fail_pending_fetch_state_for_target_background_events_async(
            conn,
            out.background_events_mut(),
            session_id.as_deref(),
            "Target closed",
        )
        .await;
    let owner_scope = crate::conn::CommandOwnerScope::capture(conn, session_id.as_deref());
    settle_target_close_after_pending_fetches_async(
        conn,
        out,
        command_context,
        renderer_output_predecessor,
        owner_scope,
        target_id,
    )
    .await;
    Ok(DevToolsCloseTargetResult { success: true })
}

/// Preserves the final renderer publication without changing the ordinary
/// `Target.closeTarget` transaction boundary.
///
/// Most target closes produce no renderer output. Those closes still complete
/// synchronously and return their detach/destroy side effects with the command,
/// matching Chromium's target-domain behavior. A paused request can first
/// produce a terminal renderer record, however. Only that case must defer
/// target retirement until the command's exact cursor has crossed ordered
/// ingress; otherwise retiring the route would discard that final record.
async fn settle_target_close_after_pending_fetches_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    owner_scope: crate::conn::CommandOwnerScope,
    target_id: String,
) {
    let action = crate::domains::page::PageTargetTerminationOwnerAction::new(
        owner_scope,
        target_id,
        crate::domains::page::PageTargetTerminationKind::TargetClose,
    );
    if let Some(predecessor) = renderer_output_predecessor {
        command_context.set_renderer_output_predecessor(predecessor);
        conn.publish_page_target_termination_owner_action(action);
        return;
    }

    let outcome =
        crate::domains::page::complete_page_target_termination_owner_action_async(conn, action)
            .await;
    let (events, scheduler_events) = outcome.into_protocol_event_parts();
    out.extend_background_events(events);
    conn.extend_scheduler_events(scheduler_events);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetTargetInfoParams {
    target_id: Option<String>,
}

pub(super) fn get_target_info(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: GetTargetInfoParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        Ok(None) => GetTargetInfoParams { target_id: None },
        Err(e) => {
            return CommandOutputPlan::error_without_session(-32602, e);
        }
    };
    let command = build_cdp_get_target_info_command(cmd, params);
    match super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::GetTargetInfo(command),
    ) {
        TargetCommandTaskStep::Complete(plan) => plan,
        TargetCommandTaskStep::Pending(_) => {
            CommandOutputPlan::error_without_session(-32000, "UnexpectedPending")
        }
    }
}

fn build_cdp_get_target_info_command(
    cmd: &Cmd<'_>,
    params: GetTargetInfoParams,
) -> DevToolsGetTargetInfoCommand {
    DevToolsGetTargetInfoCommand {
        context: cmd.devtools_command_context(params.target_id.as_deref(), None::<&str>),
        target_id: params.target_id.map(DevToolsTargetId::from),
    }
}

pub(super) fn start_devtools_get_target_info_command(
    conn: &CdpConnection,
    command: DevToolsGetTargetInfoCommand,
) -> CommandOutputPlan {
    match execute_devtools_get_target_info_command(conn, command) {
        Ok(result) => {
            CommandOutputPlan::from_devtools_result(DevToolsCommandResult::GetTargetInfo(result))
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) fn execute_devtools_get_target_info_command(
    conn: &CdpConnection,
    command: DevToolsGetTargetInfoCommand,
) -> Result<DevToolsGetTargetInfoResult, DevToolsError> {
    let Some(target_id) = command.target_id.as_ref() else {
        return Ok(DevToolsGetTargetInfoResult {
            target_info: super::browser_context::devtools_browser_target_info(),
        });
    };
    let wanted = target_id.as_str();
    if wanted == super::browser_context::DEVTOOLS_BROWSER_TARGET_ID {
        return Ok(DevToolsGetTargetInfoResult {
            target_info: super::browser_context::devtools_browser_target_info(),
        });
    }
    if conn.browser_context.is_none() && conn.inactive_browser_contexts.is_empty() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    }
    let target_exists = conn.browser_contexts().any(|bc| {
        bc.has_active_target()
            || !bc.background_targets.is_empty()
            || bc.has_any_shared_worker_targets()
            || bc.has_any_dedicated_worker_targets()
            || bc.has_any_service_worker_targets()
    });
    if !target_exists {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "TargetNotLoaded",
        ));
    }
    let target_info = conn.tab_target_info(wanted);
    if let Some(target_info) = target_info {
        return Ok(DevToolsGetTargetInfoResult { target_info });
    }
    let target_info = conn
        .browser_contexts()
        .find_map(|browser_context| browser_context.devtools_target_info(wanted))
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId"))?;
    if let Some(message) = super::transient_no_page_devtools_target_info_error(conn, &target_info) {
        return Err(DevToolsError::new(DevToolsErrorKind::Internal, message));
    }
    Ok(DevToolsGetTargetInfoResult { target_info })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoAttachParams {
    auto_attach: bool,
    #[serde(rename = "waitForDebuggerOnStart")]
    wait_for_debugger_on_start: bool,
    filter: Option<Vec<TargetFilterEntry>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoAttachRelatedParams {
    target_id: String,
    #[serde(rename = "waitForDebuggerOnStart")]
    wait_for_debugger_on_start: bool,
    filter: Option<Vec<TargetFilterEntry>>,
}

#[derive(Deserialize)]
struct TargetFilterEntry {
    #[serde(default)]
    exclude: bool,
    #[serde(rename = "type")]
    target_type: Option<String>,
}

#[derive(Debug)]
struct ServiceWorkerAutoAttachRelatedTarget {
    target_id: String,
    browser_context_id: String,
    registration_id: u64,
    version_id: u64,
    script_url: String,
    scope_url: String,
}

pub(super) fn auto_attach_related(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: AutoAttachRelatedParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return CommandOutputPlan::error_without_session(-32602, "InvalidParams");
        }
    };
    if cmd.session_id.is_some() && !conn.is_browser_session_id(cmd.session_id) {
        return CommandOutputPlan::error_without_session(
            -32000,
            "Target.autoAttachRelated is only supported on the Browser target",
        );
    }

    let restore_browser_context_id = previously_active_browser_context_id(conn);
    let target = match service_worker_auto_attach_related_target(conn, params.target_id.as_str()) {
        Ok(target) => target,
        Err(plan) => {
            restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
            return plan;
        }
    };
    let allow_service_worker_targets =
        cdp_target_filter_from_entries(params.filter).matches("service_worker");
    conn.replace_service_worker_auto_attach_related_owner(
        cmd.session_id,
        &target.browser_context_id,
        target.registration_id,
        target.version_id,
        target.script_url,
        target.scope_url,
        allow_service_worker_targets,
        params.wait_for_debugger_on_start,
    );
    let mut plan = CommandOutputPlan::default();
    if allow_service_worker_targets
        && !owner_already_auto_attached_to_target(conn, cmd.session_id, &target.target_id)
        && let Some((session_id, target_info)) =
            attach_service_worker_target_for_auto_attach_related(conn, &target.target_id)
    {
        let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
            &target.target_id,
            target_info,
            [conn.prepare_auto_attach_session_commit(
                session_id,
                cmd.session_id.map(str::to_owned),
                false,
            )],
        ));
        for event in event_plan {
            plan.push_background_event(event);
        }
    }
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    // Chromium completes autoAttachRelated only after its existing-target
    // attach pass. Existing targets are already running, so they cannot be
    // paused on start even when the policy requests it.
    plan.push_success();
    plan
}

fn service_worker_auto_attach_related_target(
    conn: &mut CdpConnection,
    target_id: &str,
) -> Result<ServiceWorkerAutoAttachRelatedTarget, CommandOutputPlan> {
    if let Err(message) = select_browser_context_for_target(conn, target_id) {
        return Err(CommandOutputPlan::error_without_session(-31998, message));
    }
    let Some(browser_context) = conn.browser_context.as_ref() else {
        return Err(CommandOutputPlan::error_without_session(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let Some(target) = browser_context.service_worker_target(target_id) else {
        return Err(CommandOutputPlan::error_without_session(
            -32000,
            "Target does not support auto-attaching",
        ));
    };
    Ok(ServiceWorkerAutoAttachRelatedTarget {
        target_id: target.target_id.clone(),
        browser_context_id: browser_context.id.clone(),
        registration_id: target.renderer_registration_id,
        version_id: target.renderer_version_id,
        script_url: target.script_url.clone(),
        scope_url: target.scope_url.clone(),
    })
}

fn attach_service_worker_target_for_auto_attach_related(
    conn: &mut CdpConnection,
    target_id: &str,
) -> Option<(String, DevToolsTargetInfo)> {
    let session_id = conn.gen_session_id();
    let target_info = conn
        .prepare_auto_attached_service_worker_session_binding_info(target_id, session_id.clone())?;
    Some((session_id, target_info))
}

fn owner_already_auto_attached_to_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .into_iter()
        .any(|session_id| {
            matches!(
                conn.session_route(Some(&session_id)),
                Some(crate::conn::CdpSessionRoute::ServiceWorkerTarget {
                    target_id: attached_target_id,
                    ..
                }) if attached_target_id == target_id
            )
        })
}

fn owner_already_auto_attached_to_page_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .into_iter()
        .any(|session_id| match conn.session_route(Some(&session_id)) {
            Some(crate::conn::CdpSessionRoute::ActiveTarget {
                browser_context_id,
                target_id: route_target_id,
            }) => {
                route_target_id.as_deref() == Some(target_id)
                    || conn
                        .browser_context_by_id(&browser_context_id)
                        .and_then(BrowserContext::active_target_id)
                        == Some(target_id)
            }
            Some(crate::conn::CdpSessionRoute::BackgroundTarget {
                target_id: route_target_id,
                ..
            })
            | Some(crate::conn::CdpSessionRoute::AuxiliaryTarget {
                target_id: route_target_id,
                ..
            }) => route_target_id == target_id,
            _ => false,
        })
}

fn owner_already_auto_attached_to_tab_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    tab_target_id: &str,
) -> bool {
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .into_iter()
        .any(|session_id| {
            matches!(
                conn.session_route(Some(&session_id)),
                Some(crate::conn::CdpSessionRoute::TabTarget {
                    tab_target_id: attached_target_id,
                    ..
                }) if attached_target_id == tab_target_id
            )
        })
}

fn should_auto_attach_page_target_for_owner(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
    target_has_primary_session: bool,
) -> bool {
    if owner_session_id.is_none() && target_has_primary_session {
        return false;
    }
    if owner_session_is_attached_to_page_target(conn, owner_session_id, target_id) {
        return false;
    }
    !owner_already_auto_attached_to_page_target(conn, owner_session_id, target_id)
}

fn should_auto_attach_tab_target_for_owner(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    tab_target_id: &str,
) -> bool {
    if owner_session_id.is_none()
        && conn
            .primary_session_id_for_tab_target_id(tab_target_id)
            .is_some()
    {
        return false;
    }
    !owner_already_auto_attached_to_tab_target(conn, owner_session_id, tab_target_id)
}

fn owner_session_is_attached_to_page_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    let Some(owner_session_id) = owner_session_id else {
        return false;
    };
    page_session_route_matches_target(conn, owner_session_id, target_id)
}

fn page_session_route_matches_target(
    conn: &CdpConnection,
    session_id: &str,
    target_id: &str,
) -> bool {
    match conn.session_route(Some(session_id)) {
        Some(crate::conn::CdpSessionRoute::ActiveTarget {
            browser_context_id,
            target_id: route_target_id,
        }) => {
            route_target_id.as_deref() == Some(target_id)
                || conn
                    .browser_context_by_id(&browser_context_id)
                    .and_then(BrowserContext::active_target_id)
                    == Some(target_id)
        }
        Some(crate::conn::CdpSessionRoute::BackgroundTarget {
            target_id: route_target_id,
            ..
        })
        | Some(crate::conn::CdpSessionRoute::AuxiliaryTarget {
            target_id: route_target_id,
            ..
        }) => route_target_id == target_id,
        _ => false,
    }
}

fn cdp_target_filter_from_entries(filter: Option<Vec<TargetFilterEntry>>) -> CdpTargetFilter {
    match filter {
        Some(entries) => CdpTargetFilter::from_entries(
            entries
                .into_iter()
                .map(|entry| CdpTargetFilterEntry {
                    exclude: entry.exclude,
                    target_type: entry.target_type,
                })
                .collect(),
        ),
        None => CdpTargetFilter::default_auto_attach(),
    }
}

pub(super) fn start_set_auto_attach_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: AutoAttachParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    if !params.auto_attach
        && params
            .filter
            .as_ref()
            .is_some_and(|filter| !filter.is_empty())
    {
        return super::target_command_error(
            -32602,
            "Target filter should be empty when disabling auto-attach",
        );
    }
    let target_filter = cdp_target_filter_from_entries(params.filter);
    let owner_is_browser_or_root =
        cmd.session_id.is_none() || conn.is_browser_session_id(cmd.session_id);
    if params.auto_attach
        && owner_is_browser_or_root
        && target_filter.matches("tab")
        && target_filter.matches("page")
    {
        return super::target_command_error(
            -32602,
            "Filter should not simultaneously allow \"tab\" and \"page\", page targets are attached via tab targets",
        );
    }
    let owner_was_enabled = conn.has_auto_attach_owner(cmd.session_id);
    let legacy_disable_all = !params.auto_attach
        && !owner_was_enabled
        && conn.auto_attach_owner_count() == 0
        && conn.auto_attach;
    if params.auto_attach {
        conn.install_default_browser_target_for_auto_attach_if_enabled();
    }
    let pause_service_workers_on_start = params.auto_attach
        && params.wait_for_debugger_on_start
        && target_filter.matches("service_worker");
    let pause_dedicated_workers_on_start =
        params.auto_attach && params.wait_for_debugger_on_start && target_filter.matches("worker");
    conn.set_auto_attach_owner(
        cmd.session_id,
        params.auto_attach,
        params.wait_for_debugger_on_start,
        target_filter,
    );
    super::set_service_worker_pause_on_start_owner(
        conn,
        cmd.session_id,
        pause_service_workers_on_start,
    );
    super::set_dedicated_worker_pause_on_start_owner(
        conn,
        cmd.session_id,
        pause_dedicated_workers_on_start,
    );
    pending_set_auto_attach_command(
        cmd.id,
        cmd.session_id,
        params.auto_attach,
        cmd.session_id,
        owner_was_enabled,
        legacy_disable_all,
    )
}

pub(super) async fn complete_set_auto_attach_command_async(
    conn: &mut CdpConnection,
    auto_attach: bool,
    owner_session_id: Option<&str>,
    owner_was_enabled: bool,
    legacy_disable_all: bool,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> CommandOutputPlan {
    let mut side_effects = events::TargetProtocolSideEffects::default();
    let restore_browser_context_id = previously_active_browser_context_id(conn);
    set_auto_attach_inner_async(
        conn,
        &mut side_effects,
        auto_attach,
        owner_session_id,
        owner_was_enabled,
        legacy_disable_all,
        command_context,
    )
    .await;
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    // Chromium runs the AddClient existing-target sweep before invoking the
    // SetAutoAttach completion callback. Puppeteer consumes attachedToTarget
    // during that sweep and assumes every pre-existing session is registered
    // by the time the command response resolves.
    let mut plan = side_effects.into_plan();
    plan.push_success();
    plan
}

async fn set_auto_attach_inner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    auto_attach: bool,
    owner_session_id: Option<&str>,
    owner_was_enabled: bool,
    legacy_disable_all: bool,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    if !auto_attach && !legacy_disable_all {
        detach_auto_attached_sessions_for_owner_async(conn, out, owner_session_id, command_context)
            .await;
        return;
    }
    if auto_attach
        && !owner_was_enabled
        && let Some(owner_session_id) = owner_session_id
        && let Some(tab_target_id) = conn
            .tab_target_id_for_session_id(owner_session_id)
            .map(str::to_owned)
    {
        auto_attach_child_page_for_tab_session_async(conn, out, owner_session_id, &tab_target_id)
            .await;
        return;
    }

    let context_ids: Vec<String> = conn.browser_contexts().map(|bc| bc.id.clone()).collect();
    for context_id in context_ids {
        if !conn.activate_browser_context_by_id_async(&context_id).await {
            continue;
        }
        if auto_attach {
            if owner_was_enabled {
                continue;
            }
            let attach_page_targets = conn
                .auto_attach_owner_allows_target_type(owner_session_id, "page")
                && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
            let attach_tab_targets = conn
                .auto_attach_owner_allows_target_type(owner_session_id, "tab")
                && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
            let attach_shared_worker_targets = conn
                .auto_attach_owner_allows_target_type(owner_session_id, "shared_worker")
                && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
            let attach_dedicated_worker_targets =
                conn.auto_attach_owner_allows_target_type(owner_session_id, "worker");
            let attach_service_worker_targets =
                conn.auto_attach_owner_allows_target_type(owner_session_id, "service_worker");
            // waitForDebuggerOnStart applies to targets created after the
            // policy is installed. Chromium reports every target found by the
            // initial AddClient sweep as already running.
            let waiting_for_debugger = false;
            let pending_attach_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if !attach_page_targets {
                    Vec::new()
                } else {
                    let mut target_ids = Vec::new();
                    if let Some(target_id) = bc.active_target_id_owned()
                        && should_auto_attach_page_target_for_owner(
                            conn,
                            owner_session_id,
                            &target_id,
                            bc.has_active_session(),
                        )
                    {
                        target_ids.push(target_id);
                    }
                    target_ids.extend(
                        bc.background_targets
                            .iter()
                            .filter(|target| {
                                should_auto_attach_page_target_for_owner(
                                    conn,
                                    owner_session_id,
                                    target.target_id(),
                                    target.has_session(),
                                )
                            })
                            .map(|target| target.target_id().to_owned()),
                    );
                    target_ids
                }
            };
            let pending_attach_tab_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing tab targets");
                if !attach_tab_targets {
                    Vec::new()
                } else {
                    let mut tab_target_ids = Vec::new();
                    if let Some(page_target_id) = bc.active_target_id()
                        && let Some(tab_target_id) =
                            conn.tab_target_id_for_page_target_id(page_target_id)
                        && should_auto_attach_tab_target_for_owner(
                            conn,
                            owner_session_id,
                            tab_target_id,
                        )
                    {
                        tab_target_ids.push(tab_target_id.to_owned());
                    }
                    tab_target_ids.extend(bc.background_targets.iter().filter_map(|target| {
                        let tab_target_id =
                            conn.tab_target_id_for_page_target_id(target.target_id())?;
                        should_auto_attach_tab_target_for_owner(
                            conn,
                            owner_session_id,
                            tab_target_id,
                        )
                        .then(|| tab_target_id.to_owned())
                    }));
                    tab_target_ids
                }
            };
            let pending_attach_shared_worker_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if attach_shared_worker_targets {
                    bc.shared_worker_targets
                        .values()
                        .map(|target| target.target_id.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            };
            let pending_attach_service_worker_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if attach_service_worker_targets {
                    bc.service_worker_targets
                        .values()
                        .map(|target| target.target_id.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            };
            let pending_attach_dedicated_worker_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if attach_dedicated_worker_targets {
                    bc.dedicated_worker_targets
                        .values()
                        .filter(|target| {
                            super::worker_target::dedicated_worker_auto_attach_owner_session_allowed(
                                conn,
                                owner_session_id,
                                &target.owner_page,
                            )
                        })
                        .map(|target| target.target_id.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            };
            let attached_targets = pending_attach_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_tab_targets = pending_attach_tab_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_shared_worker_targets = pending_attach_shared_worker_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_service_worker_targets = pending_attach_service_worker_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_dedicated_worker_targets = pending_attach_dedicated_worker_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let promote_target_id = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when considering auto-attach promotion");
                if !attach_page_targets || bc.has_loaded_page() {
                    None
                } else {
                    bc.background_targets
                        .iter()
                        .rposition(|target| !target.has_session() && target.has_loaded_page())
                        .map(|index| bc.background_targets[index].target_id().to_owned())
                        .or_else(|| {
                            bc.background_targets
                                .iter()
                                .find(|target| !target.has_session())
                                .map(|target| target.target_id().to_owned())
                        })
                }
            };
            {
                for (target_id, session_id) in &attached_targets {
                    let assigned = conn
                        .prepare_auto_attached_page_session_binding(target_id, session_id.clone());
                    debug_assert!(assigned, "attached target must remain addressable");
                }
                for (target_id, session_id) in &attached_shared_worker_targets {
                    let assigned = conn.prepare_auto_attached_shared_worker_session_binding(
                        target_id,
                        session_id.clone(),
                    );
                    debug_assert!(
                        assigned,
                        "attached shared worker target must remain addressable"
                    );
                }
                for (target_id, session_id) in &attached_dedicated_worker_targets {
                    let assigned = conn.prepare_auto_attached_dedicated_worker_session_binding(
                        target_id,
                        session_id.clone(),
                    );
                    debug_assert!(
                        assigned,
                        "attached dedicated worker target must remain addressable"
                    );
                }
                for (target_id, session_id) in &attached_service_worker_targets {
                    let assigned = conn.prepare_auto_attached_service_worker_session_binding(
                        target_id,
                        session_id.clone(),
                    );
                    debug_assert!(
                        assigned,
                        "attached service worker target must remain addressable"
                    );
                }
            }
            for (target_id, session_id) in &attached_tab_targets {
                let assigned = conn.prepare_auto_attached_tab_session_binding(
                    target_id,
                    session_id.clone(),
                    owner_session_id,
                );
                debug_assert!(assigned, "attached tab target must remain addressable");
            }
            if let Some(promote_target_id) = promote_target_id {
                match conn
                    .promote_background_target_to_active_for_connection_async(&promote_target_id)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {}
                    Err(message) => {
                        panic!(
                            "same-context target should remain promotable during auto-attach: {message}"
                        );
                    }
                }
            }
            ensure_initial_document_for_attached_page_targets_async(conn, &attached_targets).await;
            for (target_id, session_id) in &attached_targets {
                if let Err(message) = conn
                    .apply_runtime_binding_state_for_session_owner_async(Some(session_id))
                    .await
                    && message != "NoDocumentLoaded"
                {
                    tracing::warn!(
                        %message,
                        target_id = target_id.as_str(),
                        session_id = session_id.as_str(),
                        "failed to apply renderer binding state during target auto-attach"
                    );
                }
            }
            for (target_id, session_id) in attached_tab_targets {
                let ti = conn
                    .tab_target_info(&target_id)
                    .expect("attached tab target must remain addressable");
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached target must remain addressable")
                };
                if let Some(message) =
                    super::transient_no_page_devtools_target_info_error(conn, &ti)
                {
                    warn_target_protocol_side_effect_failure(
                        &target_id,
                        "emit_attached_to_target",
                        &message,
                    );
                }
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_shared_worker_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached shared worker target must remain addressable")
                };
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_dedicated_worker_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached dedicated worker target must remain addressable")
                };
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_service_worker_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached service worker target must remain addressable")
                };
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            continue;
        }

        if let Some((target_id, Some(session_id))) = conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_identity)
        {
            events::fail_pending_fetch_state_for_target_background_events_async(
                conn,
                out.background_events_mut(),
                Some(&session_id),
                "Target detached",
            )
            .await;

            let event_plan = conn
                .detach_active_target_session_binding_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await
                .expect("clearing session-scoped state during auto-attach reset should succeed");
            out.extend_background_events(event_plan);
        }

        let detached_background_targets =
            conn.background_target_session_detach_cleanup_plans(None, None);
        for cleanup_plan in detached_background_targets {
            if let Some(event_plan) = conn
                .detach_background_target_session_binding_event_plan_async(cleanup_plan)
                .await
                .expect("clearing background target session-scoped state during auto-attach reset should succeed")
            {
                out.extend_background_events(event_plan);
            }
        }

        let shared_worker_sessions_to_release = conn
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.shared_worker_targets
                    .values()
                    .flat_map(|target| target.session_ids())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for session_id in &shared_worker_sessions_to_release {
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
        }

        let shared_worker_event_plan = conn
            .detach_all_shared_worker_target_sessions_event_plan_async(None, None)
            .await;
        out.extend_background_events(shared_worker_event_plan);

        let dedicated_worker_sessions_to_release = conn
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.dedicated_worker_targets
                    .values()
                    .flat_map(|target| target.session_ids())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for session_id in &dedicated_worker_sessions_to_release {
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
        }
        let dedicated_worker_event_plan = conn
            .detach_all_dedicated_worker_target_sessions_event_plan_async(None, None)
            .await;
        out.extend_background_events(dedicated_worker_event_plan);

        let service_worker_sessions_to_detach = conn
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.service_worker_targets
                    .values()
                    .flat_map(|target| target.session_ids())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let service_worker_event_plan = conn
            .detach_all_service_worker_target_sessions_event_plan_async(None, None)
            .await;
        for session_id in service_worker_sessions_to_detach {
            super::set_service_worker_pause_on_start_owner(conn, Some(&session_id), false);
        }
        out.extend_background_events(service_worker_event_plan);
    }
}

async fn auto_attach_child_page_for_tab_session_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    tab_session_id: &str,
    tab_target_id: &str,
) {
    if !conn.auto_attach_owner_allows_target_type(Some(tab_session_id), "page") {
        return;
    }
    let Some(page_target_id) = conn
        .page_target_id_for_tab_target_id(tab_target_id)
        .map(str::to_owned)
    else {
        return;
    };
    let Some(browser_context_id) = conn.browser_context_id_for_tab_target_id(tab_target_id) else {
        return;
    };
    if !conn
        .activate_browser_context_by_id_async(&browser_context_id)
        .await
    {
        return;
    }
    let target_has_primary_session = {
        let Some(bc) = conn.browser_context.as_ref() else {
            return;
        };
        if bc.active_target_id() == Some(page_target_id.as_str()) {
            bc.has_active_session()
        } else {
            bc.background_target(&page_target_id)
                .is_some_and(|target| target.has_session())
        }
    };
    if !should_auto_attach_page_target_for_owner(
        conn,
        Some(tab_session_id),
        &page_target_id,
        target_has_primary_session,
    ) {
        return;
    }
    let session_id = conn.gen_session_id();
    let assigned =
        conn.prepare_auto_attached_page_session_binding(&page_target_id, session_id.clone());
    if !assigned {
        return;
    }
    ensure_initial_document_for_attached_page_targets_async(
        conn,
        &[(page_target_id.clone(), session_id.clone())],
    )
    .await;
    if let Err(message) = conn
        .apply_runtime_binding_state_for_session_owner_async(Some(&session_id))
        .await
        && message != "NoDocumentLoaded"
    {
        tracing::warn!(
            %message,
            target_id = page_target_id.as_str(),
            session_id = session_id.as_str(),
            "failed to apply renderer binding state during tab child page auto-attach"
        );
    }
    let prepared_session = conn.prepare_auto_attach_session_commit(
        session_id.clone(),
        Some(tab_session_id.to_owned()),
        false,
    );
    let Some(target_info) = conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.devtools_target_info(&page_target_id))
    else {
        conn.rollback_prepared_attach_session_without_event_async(&prepared_session)
            .await;
        return;
    };
    if let Some(message) = super::transient_no_page_devtools_target_info_error(conn, &target_info) {
        warn_target_protocol_side_effect_failure(
            &page_target_id,
            "emit_tab_child_attached_to_target",
            &message,
        );
        conn.rollback_prepared_attach_session_without_event_async(&prepared_session)
            .await;
        return;
    }
    let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
        &page_target_id,
        target_info,
        [prepared_session],
    ));
    out.extend_background_events(event_plan);
}

pub(super) async fn detach_auto_attached_sessions_for_owner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    owner_session_id: Option<&str>,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    let session_ids = conn.auto_attached_session_cascade_for_owner(owner_session_id);
    for session_id in session_ids {
        detach_attached_session_for_owner_async(conn, out, &session_id, command_context).await;
    }
}

pub(super) async fn detach_attached_sessions_for_owner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    owner_session_id: Option<&str>,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    let session_ids = conn.attached_session_cascade_for_owner(owner_session_id);
    for session_id in session_ids {
        detach_attached_session_for_owner_async(conn, out, &session_id, command_context).await;
    }
}

pub(super) async fn release_attached_sessions_for_root_frontend_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    // Root-owned browser sessions include the scheduler's private page-control
    // session and must outlive a browser frontend disconnect. A target session
    // owned directly by that frontend still belongs to the release cascade.
    let session_ids = conn.attached_session_cascade_for_root_frontend();
    for session_id in session_ids {
        let detach_plan = conn.auto_attached_session_detach_plan(&session_id);
        let preserves_other_frontends = matches!(
            detach_plan.cleanup_plan().map(|plan| plan.action()),
            Some(
                crate::conn::TargetBindingCleanupAction::ActiveTargetPrimaryAutoAttached
                    | crate::conn::TargetBindingCleanupAction::BackgroundTargetPrimaryAutoAttached {
                        ..
                    }
            )
        );
        if !preserves_other_frontends {
            detach_attached_session_for_owner_async(conn, out, &session_id, command_context).await;
            continue;
        }

        let Some(browser_context_id) = detach_plan.browser_context_id().map(str::to_owned) else {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
            continue;
        };
        if !conn
            .activate_browser_context_by_id_async(&browser_context_id)
            .await
        {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
            continue;
        }
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            out.background_events_mut(),
            command_context.protocol_events_mut(),
            Some(&session_id),
            "Target detached",
        );
        super::clear_detached_target_fetch_state_background_events_async(
            conn,
            out.background_events_mut(),
            &session_id,
        )
        .await;
        let _ = conn
            .detach_runtime_inspector_session_for_session_owner_async(Some(&session_id))
            .await;
        clear_emulated_media_for_detached_session_best_effort(conn, &session_id).await;
        if !conn.release_primary_target_session_binding_without_event(&session_id) {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        }
    }
}

async fn detach_attached_session_for_owner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    session_id: &str,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    if conn.is_browser_session_id(Some(session_id)) {
        conn.cancel_tracing_for_session_owner_async(Some(session_id))
            .await;
        let detached = conn.detach_browser_session_owner_without_event(session_id);
        debug_assert!(detached.is_some());
        return;
    }
    let detach_plan = conn.auto_attached_session_detach_plan(session_id);
    let Some(browser_context_id) = detach_plan.browser_context_id().map(str::to_owned) else {
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    };
    if !conn
        .activate_browser_context_by_id_async(&browser_context_id)
        .await
    {
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    }

    let Some(cleanup_plan) = detach_plan.cleanup_plan() else {
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    };
    match cleanup_plan.action().clone() {
        crate::conn::TargetBindingCleanupAction::ActiveTargetPrimaryAutoAttached => {
            let target_id = conn
                .browser_context
                .as_ref()
                .and_then(BrowserContext::active_target_identity)
                .and_then(|(target_id, current_session_id)| {
                    (current_session_id.as_deref() == Some(session_id))
                        .then(|| target_id.to_owned())
                });
            let Some(target_id) = target_id else {
                conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
                return;
            };
            // A parent-session detach cascade bypasses the direct
            // Target.detachFromTarget path. Release the renderer inspector
            // here before resetting protocol state so the replacement primary
            // session gets a fresh Runtime.enable context inventory.
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            let _ = conn
                .detach_runtime_inspector_session_for_session_owner_async(Some(session_id))
                .await;
            events::fail_pending_fetch_state_for_target_background_events_async(
                conn,
                out.background_events_mut(),
                Some(session_id),
                "Target detached",
            )
            .await;
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::AuxiliaryTarget { target_id } => {
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            let _ = conn
                .detach_runtime_inspector_session_for_session_owner_async(Some(session_id))
                .await;
            clear_emulated_media_for_detached_session_best_effort(conn, session_id).await;
            super::clear_detached_target_fetch_state_background_events_async(
                conn,
                out.background_events_mut(),
                session_id,
            )
            .await;
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::BackgroundTargetPrimaryAutoAttached {
            target_id,
        } => {
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            let _ = conn
                .detach_runtime_inspector_session_for_session_owner_async(Some(session_id))
                .await;
            clear_emulated_media_for_detached_session_best_effort(conn, session_id).await;
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::TabTarget { tab_target_id } => {
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        tab_target_id,
                        session_id,
                        None,
                        None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::SharedWorkerTarget { target_id } => {
            let renderer_detach = conn.browser_context.as_ref().and_then(|bc| {
                bc.shared_worker_target(&target_id)
                    .map(|target| (bc.renderer_runtime(), target.renderer_instance_id))
            });
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_shared_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.to_owned()),
                );
            }
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::DedicatedWorkerTarget { target_id } => {
            let renderer_detach = conn.browser_context.as_ref().and_then(|bc| {
                bc.dedicated_worker_target(&target_id)
                    .map(|target| (bc.renderer_runtime(), target.renderer_instance_id))
            });
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_dedicated_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.to_owned()),
                );
            }
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::ServiceWorkerTarget { target_id } => {
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            super::set_service_worker_pause_on_start_owner(conn, Some(session_id), false);
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::None => {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        }
    }
}

pub(super) async fn clear_emulated_media_for_detached_session_best_effort(
    conn: &mut CdpConnection,
    session_id: &str,
) {
    if let Err(error) =
        crate::domains::emulation::clear_emulated_media_for_detached_session_async(conn, session_id)
            .await
    {
        tracing::warn!(
            session_id,
            error,
            "failed to clear emulated media while detaching target session"
        );
    }
}

async fn ensure_initial_document_for_attached_page_targets_async(
    conn: &mut CdpConnection,
    attached_targets: &[(String, String)],
) {
    for (target_id, _session_id) in attached_targets {
        if !conn.browser_contexts().any(|browser_context| {
            browser_context.target_has_pending_initial_document_page_build(target_id)
        }) {
            continue;
        }
        let Some(route) = conn.target_session_route_for_target_id(target_id) else {
            continue;
        };
        let pending = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route);
            match route_scope
                .conn_mut()
                .start_initial_document_page_ensure_for_session_owner(None)
            {
                Ok(pending) => pending,
                Err(message) => {
                    warn_target_protocol_side_effect_failure(
                        target_id,
                        "start_initial_document_page_ensure",
                        &message,
                    );
                    continue;
                }
            }
        };
        let Some(pending) = pending else {
            continue;
        };
        match pending.wait().await {
            Ok(completed) => {
                if let Err(message) = conn
                    .complete_initial_document_page_build_for_owner(completed)
                    .await
                {
                    warn_target_protocol_side_effect_failure(
                        target_id,
                        "complete_initial_document_page_build",
                        &message,
                    );
                }
            }
            Err(failed) => {
                let message = conn.reset_failed_initial_document_page_build_for_owner(failed);
                warn_target_protocol_side_effect_failure(
                    target_id,
                    "reset_failed_initial_document_page_build",
                    &message,
                );
            }
        }
    }
}

fn warn_target_protocol_side_effect_failure(
    target_id: &str,
    operation: &'static str,
    message: &str,
) {
    tracing::warn!(
        target_id,
        operation,
        %message,
        "target protocol side effect failed; continuing auto-attach event emission"
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivateTargetParams {
    target_id: String,
}

pub(super) fn start_activate_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: ActivateTargetParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    let command = build_cdp_activate_target_command(cmd, params);
    super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::ActivateTarget(command),
    )
}

fn build_cdp_activate_target_command(
    cmd: &Cmd<'_>,
    params: ActivateTargetParams,
) -> DevToolsActivateTargetCommand {
    DevToolsActivateTargetCommand {
        context: cmd
            .devtools_command_context(Some(params.target_id.as_str()), Option::<&str>::None),
        target_id: DevToolsTargetId::from(params.target_id),
    }
}

pub(super) async fn complete_activate_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsActivateTargetCommand,
) -> CommandOutputPlan {
    match execute_devtools_activate_target_command_async(conn, command).await {
        Ok(()) => CommandOutputPlan::success(),
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) async fn execute_devtools_activate_target_command_async(
    conn: &mut CdpConnection,
    command: DevToolsActivateTargetCommand,
) -> Result<(), DevToolsError> {
    let target_id = command.target_id.as_str().to_owned();
    let previously_active_browser_context_id = previously_active_browser_context_id(conn);
    if let Err(message) = select_browser_context_for_target(conn, &target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Err(DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message));
    }
    let bc = match conn.browser_context.as_ref() {
        Some(bc) => bc,
        None => {
            restore_previously_active_browser_context(
                conn,
                previously_active_browser_context_id.as_deref(),
            );
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "BrowserContextNotLoaded",
            ));
        }
    };
    if bc.has_shared_worker_target(&target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Ok(());
    }
    if bc.has_dedicated_worker_target(&target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Ok(());
    }
    if bc.has_service_worker_target(&target_id) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Ok(());
    }
    if !bc.has_active_target() && bc.background_targets.is_empty() {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "TargetNotLoaded",
        ));
    }
    if !matches!(
        bc.active_target_identity(),
        Some((ref active_target_id, _)) if active_target_id == &target_id
    ) && bc.background_target(&target_id).is_some()
    {
        conn.handoff_navigation_engine_for_target_activation(&target_id);
        let promoted = match conn.browser_context.as_mut() {
            Some(browser_context) => {
                browser_context
                    .promote_background_target_to_active_slot_async(&target_id)
                    .await
            }
            None => {
                restore_previously_active_browser_context(
                    conn,
                    previously_active_browser_context_id.as_deref(),
                );
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "BrowserContextNotLoaded",
                ));
            }
        };
        match promoted {
            Ok(true) => conn.refresh_active_browser_context_loader_async().await,
            Ok(false) => {
                restore_previously_active_browser_context(
                    conn,
                    previously_active_browser_context_id.as_deref(),
                );
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "UnknownTargetId",
                ));
            }
            Err(message) => {
                restore_previously_active_browser_context(
                    conn,
                    previously_active_browser_context_id.as_deref(),
                );
                return Err(DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message));
            }
        }
    } else if !matches!(
        bc.active_target_identity(),
        Some((ref active_target_id, _)) if active_target_id == &target_id
    ) {
        restore_previously_active_browser_context(
            conn,
            previously_active_browser_context_id.as_deref(),
        );
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "UnknownTargetId",
        ));
    }

    restore_previously_active_browser_context(
        conn,
        previously_active_browser_context_id.as_deref(),
    );
    Ok(())
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::DevToolsProtocol;
    use serde_json::Value;

    use crate::conn::Cmd;

    use super::{
        ActivateTargetParams, CloseTargetParams, CreateTargetParams,
        build_cdp_activate_target_command, build_cdp_close_target_command,
        build_cdp_create_target_command,
    };

    #[test]
    fn cdp_create_target_builds_protocol_neutral_target_command() {
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(31),
            "Target.createTarget",
            &params,
            Some("SID-create"),
            r#"{"id":31,"method":"Target.createTarget"}"#,
        );

        let command = build_cdp_create_target_command(
            &cmd,
            CreateTargetParams {
                url: "https://example.com/new".to_owned(),
                browser_context_id: Some("BID-create".to_owned()),
            },
        );

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-create")
        );
        assert_eq!(command.context.target_id, None);
        assert_eq!(
            command
                .context
                .browser_context_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("BID-create")
        );
        assert_eq!(command.url, "https://example.com/new");
        assert_eq!(
            command.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("BID-create")
        );
        assert!(!command.activate);
    }

    #[test]
    fn cdp_close_target_builds_protocol_neutral_target_command() {
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(32),
            "Target.closeTarget",
            &params,
            Some("SID-close"),
            r#"{"id":32,"method":"Target.closeTarget"}"#,
        );

        let command = build_cdp_close_target_command(
            &cmd,
            CloseTargetParams {
                target_id: "TID-close".to_owned(),
            },
        );

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-close")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-close")
        );
        assert_eq!(command.context.browser_context_id, None);
        assert_eq!(command.target_id.as_str(), "TID-close");
    }

    #[test]
    fn cdp_activate_target_builds_protocol_neutral_target_command() {
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(33),
            "Target.activateTarget",
            &params,
            Some("SID-activate"),
            r#"{"id":33,"method":"Target.activateTarget"}"#,
        );

        let command = build_cdp_activate_target_command(
            &cmd,
            ActivateTargetParams {
                target_id: "TID-activate".to_owned(),
            },
        );

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-activate")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-activate")
        );
        assert_eq!(command.context.browser_context_id, None);
        assert_eq!(command.target_id.as_str(), "TID-activate");
    }
}
