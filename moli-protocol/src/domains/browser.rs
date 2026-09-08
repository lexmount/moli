use chromiumoxide_cdp::cdp::browser_protocol::browser::{
    CancelDownloadParams, SetWindowBoundsParams,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::str::FromStr;

use crate::conn::{
    CdpConnection, Cmd, CommandOwnerScope, CompletedContextPermissionUpdate,
    PendingContextPermissionUpdate, WindowSurface, WindowSurfaceState,
};
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandResult, DevToolsError, DevToolsErrorKind,
    DevToolsSetDownloadBehaviorCommand, DevToolsSetPermissionCommand,
};
use crate::domains::actions::BrowserAction;
use crate::domains::command_output::CommandOutputPlan;
use crate::version;
use moli_core::browser::DownloadPolicy;
use moli_core::page::PermissionOverrideRegistration;

/// Disables Browser-domain observation owned by one DevTools session.
pub(in crate::domains) fn dispose_session_handler(conn: &mut CdpConnection, session_id: &str) {
    conn.set_browser_download_events_enabled_for_session(Some(session_id), false);
}

pub(crate) struct PendingBrowserCommandDispatch {
    command_id: Option<u64>,
    response_session_id: Option<String>,
    kind: PendingBrowserCommandKind,
}

pub(crate) struct CompletedBrowserCommandDispatch {
    command_id: Option<u64>,
    response_session_id: Option<String>,
    kind: CompletedBrowserCommandKind,
}

pub(crate) enum BrowserCommandTaskStep {
    Pending(PendingBrowserCommandDispatch),
    Complete(CommandOutputPlan),
}

enum PendingBrowserCommandKind {
    OpenDownloadAsStream {
        pending: tokio::task::JoinHandle<Result<Vec<u8>, String>>,
    },
    ApplyPermissionOverrides {
        pending: Vec<PendingContextPermissionUpdate>,
    },
}

enum CompletedBrowserCommandKind {
    OpenDownloadAsStream {
        completed: Result<Vec<u8>, String>,
    },
    ApplyPermissionOverrides {
        completed: Vec<CompletedContextPermissionUpdate>,
    },
}

impl PendingBrowserCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedBrowserCommandDispatch {
        let kind = match self.kind {
            PendingBrowserCommandKind::OpenDownloadAsStream { pending } => {
                CompletedBrowserCommandKind::OpenDownloadAsStream {
                    completed: pending
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|result| result),
                }
            }
            PendingBrowserCommandKind::ApplyPermissionOverrides { pending } => {
                let mut completed = Vec::with_capacity(pending.len());
                for update in pending {
                    completed.push(update.wait().await);
                }
                CompletedBrowserCommandKind::ApplyPermissionOverrides { completed }
            }
        };
        CompletedBrowserCommandDispatch {
            command_id: self.command_id,
            response_session_id: self.response_session_id,
            kind,
        }
    }
}

impl CompletedBrowserCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.response_session_id.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum PermissionSetting {
    Granted,
    Denied,
    Prompt,
}

impl PermissionSetting {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    fn label(self) -> &'static str {
        self.into()
    }
}

fn normalize_permission_setting(value: String) -> String {
    PermissionSetting::parse(&value)
        .map(PermissionSetting::label)
        .unwrap_or(value.as_str())
        .to_owned()
}

pub(crate) fn try_start_browser_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> BrowserCommandTaskStep {
    let Some(action) = cmd.parse_action::<BrowserAction>() else {
        return BrowserCommandTaskStep::Complete(CommandOutputPlan::error(-32601, "UnknownMethod"));
    };
    match action {
        BrowserAction::GetVersion => BrowserCommandTaskStep::Complete(get_version(conn)),
        BrowserAction::GetWindowForTarget => {
            BrowserCommandTaskStep::Complete(get_window_for_target(conn, cmd))
        }
        BrowserAction::SetWindowBounds => {
            BrowserCommandTaskStep::Complete(set_window_bounds(conn, cmd))
        }
        BrowserAction::SetDownloadBehavior => {
            BrowserCommandTaskStep::Complete(set_download_behavior_command_output_plan(conn, cmd))
        }
        BrowserAction::CancelDownload => {
            BrowserCommandTaskStep::Complete(cancel_download(conn, cmd))
        }
        BrowserAction::OpenDownloadAsStream => start_open_download_as_stream_command(conn, cmd),
        BrowserAction::SetPermission => start_set_permission_command(conn, cmd),
        BrowserAction::GrantPermissions => start_grant_permissions_command(conn, cmd),
        BrowserAction::ResetPermissions => start_reset_permissions_command(conn, cmd),
    }
}

fn get_version(conn: &CdpConnection) -> CommandOutputPlan {
    CommandOutputPlan::result(json!({
        "protocolVersion": version::PROTOCOL_VERSION,
        "product": version::PRODUCT,
        "revision": version::REVISION,
        "userAgent": conn.user_agent(),
        "jsVersion": version::js_version(),
    }))
}

fn bounds_json(surface: WindowSurface) -> Value {
    json!({
        "windowState": surface.state.label(),
        "left": surface.x,
        "top": surface.y,
        "width": surface.width,
        "height": surface.height,
    })
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetWindowForTargetParams {
    #[serde(default)]
    target_id: Option<String>,
}

fn get_window_for_target(conn: &CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params = match cmd.get_params::<GetWindowForTargetParams>() {
        Ok(Some(params)) => params,
        Ok(None) => GetWindowForTargetParams::default(),
        Err(_) => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    let handle = match params.target_id {
        Some(target_id) => conn.browser_web_contents_for_target(&target_id),
        None => {
            conn.browser_web_contents_for_owner(&CommandOwnerScope::capture(conn, cmd.session_id))
        }
    };
    let handle = match handle {
        Ok(handle) => handle,
        Err(message) => return CommandOutputPlan::error(-32000, message),
    };
    let surface = match conn.browser_window_surface(handle) {
        Ok(surface) => surface,
        Err(message) => return CommandOutputPlan::error(-32000, message),
    };
    CommandOutputPlan::result(json!({
        "windowId": handle.id().get(),
        "bounds": bounds_json(surface)
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDownloadBehaviorParams {
    behavior: String,
    #[serde(default)]
    download_path: Option<String>,
    #[serde(default)]
    events_enabled: bool,
    #[serde(default)]
    browser_context_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetPermissionParams {
    permission: Value,
    setting: String,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    embedded_origin: Option<String>,
    #[serde(default)]
    browser_context_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrantPermissionsParams {
    permissions: Vec<Value>,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    browser_context_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetPermissionsParams {
    #[serde(default)]
    browser_context_id: Option<String>,
}

fn optional_i64_to_i32(value: Option<i64>) -> Result<Option<i32>, ()> {
    value.map(i32::try_from).transpose().map_err(|_| ())
}

fn optional_i64_to_u32(value: Option<i64>) -> Result<Option<u32>, ()> {
    value.map(u32::try_from).transpose().map_err(|_| ())
}

fn set_window_bounds(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: SetWindowBoundsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    let Ok(window_id) = u64::try_from(*params.window_id.inner()) else {
        return CommandOutputPlan::error(-32602, "InvalidParams");
    };
    let handle = match conn.browser_web_contents_for_window_id(window_id) {
        Ok(handle) => handle,
        Err(_) => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };

    let left = match optional_i64_to_i32(params.bounds.left) {
        Ok(left) => left,
        Err(()) => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    let top = match optional_i64_to_i32(params.bounds.top) {
        Ok(top) => top,
        Err(()) => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    let width = match optional_i64_to_u32(params.bounds.width) {
        Ok(width) => width,
        Err(()) => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    let height = match optional_i64_to_u32(params.bounds.height) {
        Ok(height) => height,
        Err(()) => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };
    let state = match params.bounds.window_state {
        Some(window_state) => match WindowSurfaceState::from_label(window_state.as_ref()) {
            Some(state) => Some(state),
            None => return CommandOutputPlan::error(-32602, "InvalidParams"),
        },
        None => None,
    };
    if let Err(message) =
        conn.update_browser_window_surface(handle, state, width, height, left, top)
    {
        return CommandOutputPlan::error(-32000, message);
    }

    CommandOutputPlan::success()
}

pub(crate) fn set_download_behavior_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: SetDownloadBehaviorParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    if let Some(wanted_id) = params.browser_context_id.as_deref()
        && !conn.has_browser_context_id(wanted_id)
    {
        return CommandOutputPlan::error(-31998, "UnknownBrowserContextId");
    }
    let Some(behavior) = crate::conn::parse_download_behavior(&params.behavior) else {
        return CommandOutputPlan::error(-32602, "InvalidParams");
    };

    conn.configure_download_policy(
        params.browser_context_id.as_deref(),
        DownloadPolicy {
            behavior,
            download_path: params.download_path,
        },
        None,
    )
    .expect("validated BrowserContext remains available");
    conn.set_browser_download_events_enabled_for_session(cmd.session_id, params.events_enabled);

    CommandOutputPlan::success()
}

pub(crate) fn execute_devtools_browser_command(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    match command {
        DevToolsCommand::SetDownloadBehavior(command) => {
            execute_devtools_set_download_behavior(conn, command)
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    }
}

pub(crate) async fn execute_devtools_browser_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    match command {
        DevToolsCommand::SetPermission(command) => {
            execute_devtools_set_permission_command_async(conn, command).await
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    }
}

fn execute_devtools_set_download_behavior(
    conn: &mut CdpConnection,
    command: DevToolsSetDownloadBehaviorCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let target_contexts = match command.user_contexts {
        Some(user_contexts) => {
            if user_contexts.is_empty() {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::InvalidArgument,
                    "user contexts must not be empty",
                ));
            }
            for browser_context_id in &user_contexts {
                if !conn.has_browser_context_id(browser_context_id.as_str()) {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::NoSuchTarget,
                        "UnknownBrowserContextId",
                    ));
                }
            }
            Some(user_contexts)
        }
        None => None,
    };

    let Some(behavior) = command.behavior else {
        match target_contexts {
            Some(user_contexts) => {
                for browser_context_id in user_contexts {
                    conn.reset_download_policy(Some(browser_context_id.as_str()))
                        .expect("validated BrowserContext remains available");
                }
            }
            None => conn
                .reset_download_policy(None)
                .expect("global policy always exists"),
        }
        return Ok(DevToolsCommandResult::Empty);
    };

    let Some(download_behavior) = crate::conn::parse_download_behavior(&behavior.behavior) else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "download behavior is invalid",
        ));
    };

    match target_contexts {
        Some(user_contexts) => {
            for browser_context_id in user_contexts {
                conn.configure_download_policy(
                    Some(browser_context_id.as_str()),
                    DownloadPolicy {
                        behavior: download_behavior,
                        download_path: behavior.download_path.clone(),
                    },
                    Some(behavior.events_enabled),
                )
                .expect("validated BrowserContext remains available");
            }
        }
        None => conn
            .configure_download_policy(
                None,
                DownloadPolicy {
                    behavior: download_behavior,
                    download_path: behavior.download_path,
                },
                Some(behavior.events_enabled),
            )
            .expect("global policy always exists"),
    }
    Ok(DevToolsCommandResult::Empty)
}

async fn execute_devtools_set_permission_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetPermissionCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_id = command.browser_context_id.as_ref().map(|id| id.as_str());
    if validate_browser_context_id(conn, browser_context_id).is_err() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "UnknownBrowserContextId",
        ));
    }
    let browser_context_id = command
        .browser_context_id
        .map(|browser_context_id| browser_context_id.into_string());

    conn.set_permission_override(
        browser_context_id.as_deref(),
        PermissionOverrideRegistration {
            permission: command.permission,
            setting: normalize_permission_setting(command.setting),
            origin: Some(command.origin),
            embedded_origin: command.embedded_origin,
        },
    )
    .expect("validated BrowserContext remains available");

    let pending = conn
        .start_permission_updates()
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
    if pending.is_empty() {
        return Ok(DevToolsCommandResult::Empty);
    }
    let completed = PendingBrowserCommandDispatch {
        command_id: None,
        response_session_id: command
            .context
            .session_id
            .map(|session_id| session_id.into_string()),
        kind: PendingBrowserCommandKind::ApplyPermissionOverrides { pending },
    }
    .wait()
    .await;
    let CompletedBrowserCommandKind::ApplyPermissionOverrides {
        completed: commands,
    } = completed.kind
    else {
        unreachable!("set permission can only wait for permission override commands")
    };
    for command in commands {
        conn.finish_permission_update(command)
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
    }
    Ok(DevToolsCommandResult::Empty)
}

fn cancel_download(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: CancelDownloadParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error(-32602, "InvalidParams");
        }
    };

    match conn.cancel_download(&params.guid) {
        Ok(()) => CommandOutputPlan::success(),
        Err(message) => CommandOutputPlan::error(-32602, message),
    }
}

fn start_open_download_as_stream_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> BrowserCommandTaskStep {
    let params: CancelDownloadParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return browser_error_step(-32602, "InvalidParams");
        }
    };

    match conn.start_open_download_as_stream(&params.guid) {
        Ok(pending) => BrowserCommandTaskStep::Pending(PendingBrowserCommandDispatch {
            command_id: cmd.id,
            response_session_id: cmd.session_id.map(str::to_owned),
            kind: PendingBrowserCommandKind::OpenDownloadAsStream { pending },
        }),
        Err(message) => browser_error_step(-32602, message),
    }
}

fn validate_browser_context_id(
    conn: &CdpConnection,
    browser_context_id: Option<&str>,
) -> Result<(), ()> {
    if let Some(wanted_id) = browser_context_id
        && !conn.has_browser_context_id(wanted_id)
    {
        return Err(());
    }
    Ok(())
}

fn start_set_permission_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> BrowserCommandTaskStep {
    let params: SetPermissionParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return browser_error_step(-32602, "InvalidParams");
        }
    };
    if validate_browser_context_id(conn, params.browser_context_id.as_deref()).is_err() {
        return browser_error_step(-31998, "UnknownBrowserContextId");
    }

    conn.set_permission_override(
        params.browser_context_id.as_deref(),
        PermissionOverrideRegistration {
            permission: params.permission,
            setting: normalize_permission_setting(params.setting),
            origin: params.origin,
            embedded_origin: params.embedded_origin,
        },
    )
    .expect("validated BrowserContext remains available");
    start_apply_permission_overrides_command(conn, cmd)
}

fn start_grant_permissions_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> BrowserCommandTaskStep {
    let params: GrantPermissionsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return browser_error_step(-32602, "InvalidParams");
        }
    };
    if validate_browser_context_id(conn, params.browser_context_id.as_deref()).is_err() {
        return browser_error_step(-31998, "UnknownBrowserContextId");
    }

    for permission in params.permissions {
        conn.set_permission_override(
            params.browser_context_id.as_deref(),
            PermissionOverrideRegistration {
                permission,
                setting: PermissionSetting::Granted.label().to_owned(),
                origin: params.origin.clone(),
                embedded_origin: None,
            },
        )
        .expect("validated BrowserContext remains available");
    }

    start_apply_permission_overrides_command(conn, cmd)
}

fn start_reset_permissions_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> BrowserCommandTaskStep {
    let params: ResetPermissionsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => ResetPermissionsParams {
            browser_context_id: None,
        },
        Err(_) => {
            return browser_error_step(-32602, "InvalidParams");
        }
    };
    if validate_browser_context_id(conn, params.browser_context_id.as_deref()).is_err() {
        return browser_error_step(-31998, "UnknownBrowserContextId");
    }

    conn.clear_permission_overrides(params.browser_context_id.as_deref())
        .expect("validated BrowserContext remains available");

    start_apply_permission_overrides_command(conn, cmd)
}

fn browser_error_step(code: i32, message: impl Into<String>) -> BrowserCommandTaskStep {
    BrowserCommandTaskStep::Complete(CommandOutputPlan::error(code, message))
}

fn browser_success_step() -> BrowserCommandTaskStep {
    BrowserCommandTaskStep::Complete(CommandOutputPlan::success())
}

fn start_apply_permission_overrides_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> BrowserCommandTaskStep {
    let pending = match conn.start_permission_updates() {
        Ok(pending) => pending,
        Err(message) => return browser_error_step(-32000, message),
    };
    if pending.is_empty() {
        return browser_success_step();
    }
    BrowserCommandTaskStep::Pending(PendingBrowserCommandDispatch {
        command_id: cmd.id,
        response_session_id: cmd.session_id.map(str::to_owned),
        kind: PendingBrowserCommandKind::ApplyPermissionOverrides { pending },
    })
}

pub(crate) fn complete_pending_browser_command(
    conn: &mut CdpConnection,
    completed: CompletedBrowserCommandDispatch,
) -> CommandOutputPlan {
    match completed.kind {
        CompletedBrowserCommandKind::OpenDownloadAsStream { completed: bytes } => match bytes {
            Ok(bytes) => {
                let stream = conn.finish_open_download_as_stream(bytes);
                CommandOutputPlan::result(json!({ "stream": stream }))
            }
            Err(message) => CommandOutputPlan::error(-32602, message),
        },
        CompletedBrowserCommandKind::ApplyPermissionOverrides {
            completed: commands,
        } => {
            for command in commands {
                if let Err(error) = conn.finish_permission_update(command) {
                    return CommandOutputPlan::error(-32000, error);
                }
            }
            CommandOutputPlan::success()
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
