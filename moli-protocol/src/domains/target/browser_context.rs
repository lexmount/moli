use serde::Deserialize;
use serde_json::json;

use moli_browser_profile::{DEFAULT_PROFILE_PARTITION_ID, ProfilePartitionId};

use crate::conn::{
    BackgroundProtocolEvent, TargetOwnerState, TargetWindowSurfaceState,
    monotonic_timestamp_seconds,
};
use crate::devtools_runtime::{
    DevToolsClientWindowInfo, DevToolsCommand, DevToolsCommandResult,
    DevToolsCreateBrowserContextCommand, DevToolsCreateBrowserContextResult, DevToolsError,
    DevToolsErrorKind, DevToolsGetBrowserContextsCommand, DevToolsGetBrowserContextsResult,
    DevToolsGetClientWindowsCommand, DevToolsGetClientWindowsResult,
    DevToolsGetServiceWorkerLogsCommand, DevToolsGetServiceWorkerLogsResult,
    DevToolsGetTargetsCommand, DevToolsGetTargetsResult, DevToolsProtocol,
    DevToolsRemoveBrowserContextCommand, DevToolsTargetFilterEntry, DevToolsTargetId,
    DevToolsTargetInfo, DevToolsTargetKind, DevToolsWindowState, RuntimeConsoleEvent,
};
use crate::domains::observable_output::runtime_console_message_type_and_text;

use super::*;
use crate::domains::command_output::CommandOutputPlan;

pub(super) fn start_get_targets_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let Some(command) = build_cdp_get_targets_command(cmd) else {
        return super::target_command_error(-32602, "InvalidParams");
    };
    super::start_devtools_target_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::GetTargets(command),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetTargetsParams {
    filter: Option<Vec<GetTargetsFilterEntry>>,
}

#[derive(Deserialize)]
struct GetTargetsFilterEntry {
    #[serde(default)]
    exclude: bool,
    #[serde(rename = "type")]
    target_type: Option<String>,
}

fn build_cdp_get_targets_command(cmd: &Cmd<'_>) -> Option<DevToolsGetTargetsCommand> {
    let params: Option<GetTargetsParams> = cmd.get_params().ok()?;
    Some(DevToolsGetTargetsCommand {
        context: cmd.devtools_command_context(None::<&str>, None::<&str>),
        root: None,
        max_depth: None,
        filter: params.and_then(|params| {
            params.filter.map(|filter| {
                filter
                    .into_iter()
                    .map(|entry| DevToolsTargetFilterEntry {
                        exclude: entry.exclude,
                        target_type: entry.target_type,
                    })
                    .collect()
            })
        }),
    })
}

pub(super) fn start_devtools_get_targets_command(
    conn: &CdpConnection,
    command: DevToolsGetTargetsCommand,
) -> CommandOutputPlan {
    match execute_devtools_get_targets_command(conn, &command) {
        Ok(result) => {
            CommandOutputPlan::from_devtools_result(DevToolsCommandResult::GetTargets(result))
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}

pub(super) fn execute_devtools_get_targets_command(
    conn: &CdpConnection,
    command: &DevToolsGetTargetsCommand,
) -> Result<DevToolsGetTargetsResult, DevToolsError> {
    let owner_session_id = command
        .context
        .session_id
        .as_ref()
        .map(|session| session.as_str());
    let handler_filter = if command.filter.is_none() {
        conn.target_discovery_filter_for_owner(owner_session_id)
    } else {
        None
    };
    let effective_filter = command.filter.as_deref().or(handler_filter.as_deref());
    Ok(DevToolsGetTargetsResult {
        targets: devtools_target_infos(conn, command.root.as_ref(), effective_filter)?,
    })
}

pub(super) fn execute_devtools_get_service_worker_logs_command(
    conn: &mut CdpConnection,
    command: &DevToolsGetServiceWorkerLogsCommand,
) -> Result<DevToolsGetServiceWorkerLogsResult, DevToolsError> {
    let target_ids = devtools_service_worker_log_target_ids(conn, command)?;
    let mut entries = Vec::new();
    let base_timestamp = monotonic_timestamp_seconds();
    let mut entry_index = 0_usize;
    let command_session_id = command
        .context
        .session_id
        .as_ref()
        .map(|session_id| session_id.as_str())
        .unwrap_or("none")
        .to_owned();
    for (browser_context_id, target_id) in target_ids {
        let Some(browser_context) = conn.browser_context_by_id_mut(&browser_context_id) else {
            continue;
        };
        let Some(target) = browser_context.service_worker_target_mut(&target_id) else {
            continue;
        };
        let cursor_id =
            devtools_service_worker_classic_log_cursor_id(&command_session_id, &target_id);
        let messages = target.pending_classic_log_messages(&cursor_id).to_vec();
        let console_end = target.console_message_count();
        target.mark_classic_log_emitted(cursor_id, console_end);
        for message in messages {
            entry_index = entry_index.saturating_add(1);
            let (console_type, text) = runtime_console_message_type_and_text(&message.message);
            entries.push(RuntimeConsoleEvent {
                target_id: Some(DevToolsTargetId::from(target_id.clone())),
                console_type: console_type.to_owned(),
                text: text.to_owned(),
                args: message.args,
                stack: message.stack,
                stack_trace: None,
                execution_context_id: (message.execution_context_id > 0)
                    .then_some(message.execution_context_id),
                timestamp: Some(base_timestamp + (entry_index as f64 * 0.000_001)),
            });
        }
    }
    Ok(DevToolsGetServiceWorkerLogsResult { entries })
}

fn devtools_service_worker_log_target_ids(
    conn: &CdpConnection,
    command: &DevToolsGetServiceWorkerLogsCommand,
) -> Result<Vec<(String, String)>, DevToolsError> {
    let mut target_ids = Vec::new();
    let browser_context_filter = command
        .context
        .browser_context_id
        .as_ref()
        .map(|browser_context_id| browser_context_id.as_str());
    for browser_context in conn.browser_contexts() {
        if browser_context_filter.is_some_and(|filter| browser_context.id != filter) {
            continue;
        }
        for target_info in browser_context.devtools_target_infos() {
            if target_info.kind != DevToolsTargetKind::ServiceWorker {
                continue;
            }
            let Some(target_id) = target_info.target_id.as_ref() else {
                continue;
            };
            if command
                .target_id
                .as_ref()
                .is_some_and(|wanted| wanted != target_id)
            {
                continue;
            }
            target_ids.push((browser_context.id.clone(), target_id.as_str().to_owned()));
        }
    }
    if command.target_id.is_some() && target_ids.is_empty() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "No such service worker target",
        ));
    }
    Ok(target_ids)
}

fn devtools_service_worker_classic_log_cursor_id(session_id: &str, target_id: &str) -> String {
    format!("classic-service-worker-log:{session_id}:{target_id}")
}

pub(super) fn execute_devtools_get_client_windows_command(
    conn: &CdpConnection,
    _command: &DevToolsGetClientWindowsCommand,
) -> Result<DevToolsGetClientWindowsResult, DevToolsError> {
    let mut client_windows = Vec::new();
    for target_info in devtools_target_infos(conn, None, None)? {
        if target_info.kind != DevToolsTargetKind::Page {
            continue;
        }
        let Some(target_id) = target_info.target_id else {
            continue;
        };
        if let Some(info) = devtools_client_window_info_for_target(conn, &target_id) {
            client_windows.push(info);
        }
    }
    Ok(DevToolsGetClientWindowsResult { client_windows })
}

pub(in crate::domains) fn devtools_client_window_info_for_target(
    conn: &CdpConnection,
    target_id: &DevToolsTargetId,
) -> Option<DevToolsClientWindowInfo> {
    let active_browser_context_id = conn
        .browser_context
        .as_ref()
        .map(|context| context.id.as_str());
    for browser_context in conn.browser_contexts() {
        if browser_context.active_target_id() == Some(target_id.as_str()) {
            let active = active_browser_context_id == Some(browser_context.id.as_str());
            return Some(devtools_client_window_info_from_owner_state(
                target_id.clone(),
                active,
                Some(&browser_context.active_target.owner_state),
            ));
        }
        if browser_context
            .background_targets()
            .any(|target| target.target_id() == target_id.as_str())
        {
            return Some(devtools_client_window_info_from_owner_state(
                target_id.clone(),
                false,
                browser_context.parked_target_owner_state(target_id.as_str()),
            ));
        }
    }
    None
}

fn devtools_client_window_info_from_owner_state(
    target_id: DevToolsTargetId,
    active: bool,
    owner_state: Option<&TargetOwnerState>,
) -> DevToolsClientWindowInfo {
    let state = owner_state
        .map(|owner_state| {
            devtools_window_state_from_target_surface(owner_state.window_surface_state)
        })
        .unwrap_or(DevToolsWindowState::Normal);
    let geometry = owner_state.map(|owner_state| owner_state.window_surface_geometry);
    DevToolsClientWindowInfo {
        client_window: target_id,
        active,
        state,
        width: geometry.map(|geometry| geometry.width).unwrap_or(0),
        height: geometry.map(|geometry| geometry.height).unwrap_or(0),
        x: geometry.map(|geometry| geometry.x).unwrap_or(0),
        y: geometry.map(|geometry| geometry.y).unwrap_or(0),
    }
}

fn devtools_window_state_from_target_surface(
    state: TargetWindowSurfaceState,
) -> DevToolsWindowState {
    match state {
        TargetWindowSurfaceState::Normal => DevToolsWindowState::Normal,
        TargetWindowSurfaceState::Maximized => DevToolsWindowState::Maximized,
        TargetWindowSurfaceState::Minimized => DevToolsWindowState::Minimized,
        TargetWindowSurfaceState::Fullscreen => DevToolsWindowState::Fullscreen,
    }
}

pub(super) fn execute_devtools_create_browser_context_command(
    conn: &mut CdpConnection,
    command: DevToolsCreateBrowserContextCommand,
) -> Result<DevToolsCreateBrowserContextResult, DevToolsError> {
    if let Some(partition_id) = command.persistent_partition_id.as_deref() {
        validate_persistent_partition_id(partition_id)
            .map_err(|message| DevToolsError::new(DevToolsErrorKind::InvalidArgument, message))?;
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "PersistentBrowserContextNotSupported",
        ));
    }

    let id = command
        .browser_context_id
        .as_ref()
        .map(|id| id.as_str().to_owned())
        .unwrap_or_else(|| match command.context.protocol {
            DevToolsProtocol::WebDriverBidi => conn.gen_user_browser_context_id(),
            _ => conn.gen_bc_id(),
        });
    if conn.has_browser_context_id(&id) {
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "BrowserContextAlreadyExists",
        ));
    }

    let mut browser_context = conn.new_ephemeral_browser_context(id.clone());
    browser_context.default_tls_verify_host_override =
        command.accept_insecure_certs.map(|accept| !accept);
    browser_context.default_http_proxy_override = command.proxy_server;
    browser_context.default_http_no_proxy_override =
        normalize_proxy_bypass_list_for_loader(command.proxy_bypass_list.as_deref());
    browser_context.proxy_autoconfig_url = command.proxy_autoconfig_url;
    browser_context.proxy_socks_version = command.proxy_socks_version;
    conn.insert_browser_context(browser_context);
    Ok(DevToolsCreateBrowserContextResult {
        browser_context_id: crate::devtools_runtime::DevToolsBrowserContextId::from(id),
    })
}

pub(super) fn devtools_get_browser_contexts_result(
    conn: &CdpConnection,
    _command: &DevToolsGetBrowserContextsCommand,
) -> DevToolsGetBrowserContextsResult {
    DevToolsGetBrowserContextsResult {
        browser_context_ids: conn
            .browser_contexts()
            .map(|context| {
                crate::devtools_runtime::DevToolsBrowserContextId::from(context.id.as_str())
            })
            .collect(),
    }
}

pub(super) async fn execute_devtools_remove_browser_context_command_async(
    conn: &mut CdpConnection,
    command: DevToolsRemoveBrowserContextCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<BackgroundProtocolEvent>,
) {
    let should_emit_internal_lifecycle =
        command.context.protocol == DevToolsProtocol::WebDriverBidi;
    let browser_context_id = command.browser_context_id.into_string();
    if browser_context_id == conn.default_browser_context_id() {
        return (
            Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "DefaultBrowserContextCannotBeRemoved",
            )),
            Vec::new(),
        );
    }
    if !conn.has_browser_context_id(&browser_context_id) {
        return (
            Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownBrowserContextId",
            )),
            Vec::new(),
        );
    }

    let mut protocol_events = Vec::new();
    if should_emit_internal_lifecycle {
        protocol_events.extend(target_destroyed_automation_events_for_browser_context(
            conn,
            &browser_context_id,
        ));
    }
    let mut side_effects = events::TargetProtocolSideEffects::default();
    let mut command_context = crate::conn::CommandDispatchContext::default();
    if let Err(error) = super::browser_context_disposal::execute_browser_context_disposal_async(
        conn,
        browser_context_id,
        &mut side_effects,
        &mut command_context,
    )
    .await
    {
        return (Err(error), Vec::new());
    }
    protocol_events.extend(side_effects.into_background_events());
    protocol_events.extend(command_context.take_protocol_events());
    (Ok(DevToolsCommandResult::Empty), protocol_events)
}

pub(super) const DEVTOOLS_BROWSER_TARGET_ID: &str = "browser";

pub(super) fn devtools_browser_target_info() -> DevToolsTargetInfo {
    DevToolsTargetInfo {
        target_id: Some(DevToolsTargetId::from(DEVTOOLS_BROWSER_TARGET_ID)),
        kind: DevToolsTargetKind::Browser,
        title: String::new(),
        url: String::new(),
        attached: true,
        opener_id: None,
        opener_frame_id: None,
        can_access_opener: false,
        browser_context_id: None,
        moli_popup_id: None,
    }
}

fn devtools_target_infos(
    conn: &CdpConnection,
    root: Option<&DevToolsTargetId>,
    filter: Option<&[DevToolsTargetFilterEntry]>,
) -> Result<Vec<DevToolsTargetInfo>, DevToolsError> {
    let mut targets = Vec::new();
    for target_info in conn.devtools_target_infos() {
        if let Some(message) =
            super::transient_no_page_devtools_target_info_error(conn, &target_info)
        {
            return Err(DevToolsError::new(DevToolsErrorKind::Internal, message));
        }
        if target_info_matches_root(&target_info, root)
            && target_filter_allows_info(filter, &target_info)
        {
            targets.push(target_info);
        }
    }
    Ok(targets)
}

pub(in crate::domains) fn devtools_target_infos_for_discovery(
    conn: &CdpConnection,
    filter: Option<&[DevToolsTargetFilterEntry]>,
) -> Result<Vec<DevToolsTargetInfo>, DevToolsError> {
    let mut targets = devtools_target_infos(conn, None, filter)?;
    let browser_target = devtools_browser_target_info();
    if target_filter_allows_info(filter, &browser_target) {
        targets.push(browser_target);
    }
    Ok(targets)
}

fn target_info_matches_root(
    target_info: &DevToolsTargetInfo,
    root: Option<&DevToolsTargetId>,
) -> bool {
    let Some(root) = root else {
        return true;
    };
    target_info
        .target_id
        .as_ref()
        .is_some_and(|target_id| target_id == root)
}

fn target_filter_allows_info(
    filter: Option<&[DevToolsTargetFilterEntry]>,
    target_info: &DevToolsTargetInfo,
) -> bool {
    target_filter_allows_type(filter, target_info.kind.as_cdp_type())
}

pub(in crate::domains) fn target_filter_allows_type(
    filter: Option<&[DevToolsTargetFilterEntry]>,
    target_type: &str,
) -> bool {
    let Some(filter) = filter else {
        return !matches!(target_type, "browser" | "tab");
    };
    for entry in filter {
        if entry
            .target_type
            .as_deref()
            .is_none_or(|entry_type| entry_type == target_type)
        {
            return !entry.exclude;
        }
    }
    false
}

fn target_destroyed_automation_events_for_browser_context(
    conn: &CdpConnection,
    browser_context_id: &str,
) -> Vec<BackgroundProtocolEvent> {
    let Some(browser_context) = conn.browser_context_by_id(browser_context_id) else {
        return Vec::new();
    };
    let target_infos = browser_context.devtools_target_infos();
    target_infos
        .into_iter()
        .flat_map(|target_info| conn.target_destroyed_automation_events(target_info))
        .collect()
}

pub(super) fn get_browser_contexts(conn: &mut CdpConnection) -> CommandOutputPlan {
    let ids: Vec<&str> = conn.browser_contexts().map(|bc| bc.id.as_str()).collect();
    CommandOutputPlan::result(json!({
        "browserContextIds": ids,
        "defaultBrowserContextId": conn.default_browser_context_id(),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateBrowserContextParams {
    proxy_server: Option<String>,
    proxy_bypass_list: Option<String>,
    persistent_partition_id: Option<String>,
}

pub(super) fn create_browser_context(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params = match cmd.get_params::<CreateBrowserContextParams>() {
        Ok(Some(params)) => params,
        Ok(None) => CreateBrowserContextParams {
            proxy_server: None,
            proxy_bypass_list: None,
            persistent_partition_id: None,
        },
        Err(_) => {
            return CommandOutputPlan::error_without_session(-32602, "InvalidParams");
        }
    };
    if let Some(partition_id) = params.persistent_partition_id.as_deref() {
        let message = validate_persistent_partition_id(partition_id)
            .err()
            .unwrap_or("PersistentBrowserContextNotSupported");
        return CommandOutputPlan::error_without_session(-32602, message);
    }
    let id = conn.gen_bc_id();
    let mut browser_context = conn.new_ephemeral_browser_context(id.clone());
    browser_context.default_http_proxy_override = params.proxy_server;
    browser_context.default_http_no_proxy_override =
        normalize_proxy_bypass_list_for_loader(params.proxy_bypass_list.as_deref());
    conn.insert_browser_context(browser_context);
    CommandOutputPlan::result(json!({ "browserContextId": id }))
}

fn validate_persistent_partition_id(partition_id: &str) -> Result<(), &'static str> {
    if partition_id == DEFAULT_PROFILE_PARTITION_ID {
        return Err("DefaultPersistentBrowserContextNotAllowed");
    }
    ProfilePartitionId::new(partition_id)
        .map(|_| ())
        .map_err(|_| "InvalidPersistentBrowserContextId")
}

fn normalize_proxy_bypass_list_for_loader(proxy_bypass_list: Option<&str>) -> Option<String> {
    let raw = proxy_bypass_list?.trim();
    if raw.is_empty() {
        return None;
    }

    let filtered: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter(|entry| !entry.eq_ignore_ascii_case("<-loopback>"))
        .collect();

    Some(filtered.join(","))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DisposeBcParams {
    browser_context_id: Option<String>,
}

pub(super) fn start_dispose_browser_context_command(cmd: &Cmd<'_>) -> TargetCommandTaskStep {
    let params: DisposeBcParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    let wanted_id = match params.browser_context_id {
        Some(id) => id,
        None => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    pending_dispose_browser_context_command(cmd.id, cmd.session_id, wanted_id)
}

pub(super) async fn complete_dispose_browser_context_command_async(
    conn: &mut CdpConnection,
    wanted_id: String,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> CommandOutputPlan {
    let mut side_effects = events::TargetProtocolSideEffects::default();
    match super::browser_context_disposal::execute_browser_context_disposal_async(
        conn,
        wanted_id,
        &mut side_effects,
        command_context,
    )
    .await
    {
        Ok(()) => {
            let mut plan = CommandOutputPlan::success();
            plan.extend(side_effects.into_plan());
            plan
        }
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    }
}
