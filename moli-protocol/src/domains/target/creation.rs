use serde::Deserialize;

use crate::conn::{PreparedTargetAttach, TargetActivationTransition, TargetAttachSessionCommit};
use crate::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsCreateTargetResult, DevToolsTargetId,
};

use super::*;

pub(super) struct DevToolsCreateTargetExecution {
    pub(super) result: DevToolsCreateTargetResult,
    pub(super) commit: TargetCreationCommit,
}

pub(super) struct TargetCreationCommit {
    page_target_id: String,
    tab_target_id: String,
    activation: Option<TargetActivationTransition>,
    attached_tab_sessions: Vec<TargetAttachSessionCommit>,
    attached_sessions: Vec<TargetAttachSessionCommit>,
}

impl TargetCreationCommit {
    pub(super) fn page_target_id(&self) -> &str {
        &self.page_target_id
    }

    pub(super) fn activation(&self) -> Option<&TargetActivationTransition> {
        self.activation.as_ref()
    }
}

#[derive(Clone, Copy)]
enum CreateTargetResultHost {
    Page,
    Tab,
}

pub(super) fn emit_target_creation_protocol_events(
    conn: &mut CdpConnection,
    events: TargetCreationCommit,
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
                .devtools_target_info(&events.page_target_id)
                .is_some()
        })
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId"))?;
    let target_info = bc
        .devtools_target_info(&events.page_target_id)
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId"))?;
    if let Some(message) = super::transient_no_page_devtools_target_info_error(conn, &target_info) {
        return Err(DevToolsError::new(DevToolsErrorKind::Internal, message));
    }
    if has_discovery {
        for event in conn.target_created_event_plan(&events.page_target_id) {
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
        events.page_target_id,
        target_info,
        events.attached_sessions,
    ));
    for event in event_plan {
        out.push_target_background_event(event);
    }
    Ok(())
}

pub(super) fn push_target_created_events(
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
pub(super) struct CreateTargetParams {
    #[serde(default = "default_blank")]
    pub(super) url: String,
    pub(super) browser_context_id: Option<String>,
    pub(super) for_tab: Option<bool>,
    pub(super) background: Option<bool>,
    pub(super) focus: Option<bool>,
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
            for_tab: None,
            background: None,
            focus: None,
        },
        Err(e) => {
            return super::target_command_error(-32602, e);
        }
    };
    let result_host = if params.for_tab.unwrap_or(false) {
        CreateTargetResultHost::Tab
    } else {
        CreateTargetResultHost::Page
    };
    let command = match build_cdp_create_target_command(cmd, params) {
        Ok(command) => command,
        Err(message) => return super::target_command_error(-32602, message),
    };
    start_devtools_create_target_command_with_result_host(
        conn,
        cmd.id,
        cmd.session_id,
        command,
        result_host,
    )
}

pub(super) fn build_cdp_create_target_command(
    cmd: &Cmd<'_>,
    params: CreateTargetParams,
) -> Result<DevToolsCreateTargetCommand, &'static str> {
    let should_focus = params.focus.unwrap_or(!params.background.unwrap_or(false));
    let create_in_background = params.background.unwrap_or(false) || params.focus == Some(false);
    if should_focus && create_in_background {
        return Err("Can't focus a target in the background. Use background=false instead.");
    }
    Ok(DevToolsCreateTargetCommand {
        context: cmd.devtools_command_context(None::<&str>, params.browser_context_id.as_deref()),
        url: params.url,
        browser_context_id: params
            .browser_context_id
            .map(DevToolsBrowserContextId::from),
        activate: !create_in_background,
    })
}

pub(super) fn start_devtools_create_target_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCreateTargetCommand,
) -> TargetCommandTaskStep {
    start_devtools_create_target_command_with_result_host(
        conn,
        command_id,
        command_session_id,
        command,
        CreateTargetResultHost::Page,
    )
}

fn start_devtools_create_target_command_with_result_host(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCreateTargetCommand,
    result_host: CreateTargetResultHost,
) -> TargetCommandTaskStep {
    let mut plan = CommandOutputPlan::default();
    let execution = execute_devtools_create_target_command(conn, command);
    let (created_target_id, creation_commit) = match execution {
        Ok(execution) => {
            let target_id = execution.result.target_id.clone();
            let response_target_id = match result_host {
                CreateTargetResultHost::Page => execution.result.target_id,
                CreateTargetResultHost::Tab => {
                    DevToolsTargetId::from(execution.commit.tab_target_id.as_str())
                }
            };
            plan.extend(CommandOutputPlan::from_devtools_result(
                DevToolsCommandResult::CreateTarget(DevToolsCreateTargetResult {
                    target_id: response_target_id,
                }),
            ));
            (target_id, execution.commit)
        }
        Err(error) => {
            plan.extend(CommandOutputPlan::from_devtools_error(error));
            return TargetCommandTaskStep::Complete(plan);
        }
    };
    let initial_document_route =
        conn.target_session_route_for_target_id(created_target_id.as_str());
    let pending_initial_document = if let Some(route) = initial_document_route {
        let owner = crate::conn::CommandOwnerScope::for_implicit_route(Some(route));
        conn.start_initial_document_page_ensure_for_owner(&owner)
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
                    creation_commit,
                    initial_document: Some(Box::new(initial_document)),
                }),
            })
        }
        Ok(None) => {
            let mut output_plan = CommandOutputPlan::default();
            let mut protocol_events = Vec::new();
            if let Err(error) =
                emit_target_creation_protocol_events(conn, creation_commit, &mut protocol_events)
            {
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
    conn.ensure_browser_context_for_implicit_target_creation();

    let target_id = conn.gen_target_id();
    let default_target_id = conn.default_target_id();
    let (has_active_target, previous_active_target_id) = {
        let browser_context = conn
            .browser_context
            .as_ref()
            .expect("browser context must exist before target creation");
        (
            browser_context.active_target_identity().is_some()
                && !browser_context
                    .active_target_is_unclaimed_default_placeholder(default_target_id),
            browser_context.active_target_id_owned(),
        )
    };
    let claims_default_placeholder =
        !has_active_target && previous_active_target_id.as_deref() == Some(default_target_id);
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
    let activation = activating_created_target
        .then(|| TargetActivationTransition::new(target_id.clone(), previous_active_target_id));
    let initial_empty_document_url = create_target_initial_empty_document_url(&command.url);
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
            let claimed_default_placeholder =
                claims_default_placeholder && bc.rekey_active_target(target_id.clone());
            if !claimed_default_placeholder {
                bc.set_active_target_id(target_id.clone());
            }
            bc.set_target_url(command.url.clone());
            bc.begin_active_target_initial_empty_document(initial_empty_document_url.clone());
            bc.page_target_mut(&target_id)
                .expect("newly selected target must exist")
                .owner_state
                .target_crash_state
                .clear();
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
    if !creating_background_target {
        conn.notify_target_host_activated(&target_id);
    }
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
        commit: TargetCreationCommit {
            page_target_id: target_id,
            tab_target_id,
            activation,
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

pub(super) fn top_level_page_auto_attach_owner_sessions(
    conn: &CdpConnection,
) -> Vec<Option<String>> {
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

pub(super) fn top_level_tab_auto_attach_owner_sessions(
    conn: &CdpConnection,
) -> Vec<Option<String>> {
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
