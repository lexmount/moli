use chromiumoxide_cdp::cdp::browser_protocol::browser::{
    CancelDownloadParams, SetWindowBoundsParams,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::str::FromStr;

use crate::conn::{BrowserWindowBounds, CdpConnection, Cmd};
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandResult, DevToolsError, DevToolsErrorKind,
    DevToolsSetDownloadBehaviorCommand, DevToolsSetPermissionCommand,
};
use crate::domains::actions::BrowserAction;
use crate::domains::command_output::CommandOutputPlan;
use crate::version;
use moli_core::page::{CompletedPageCommand, PendingPageCommand};

const DEV_TOOLS_WINDOW_ID: u32 = 1_923_710_101;
const DOWNLOAD_BEHAVIORS: &[&str] = &["default", "deny", "allow", "allowAndName"];

pub(crate) fn is_valid_download_behavior(behavior: &str) -> bool {
    DOWNLOAD_BEHAVIORS.contains(&behavior)
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
        pending: Vec<PendingBrowserPageCommand>,
    },
}

enum CompletedBrowserCommandKind {
    OpenDownloadAsStream {
        completed: Result<Vec<u8>, String>,
    },
    ApplyPermissionOverrides {
        completed: Vec<CompletedBrowserPageCommand>,
    },
}

struct PendingBrowserPageCommand {
    target: PendingBrowserPageTarget,
    pending: PendingPageCommand,
}

struct CompletedBrowserPageCommand {
    target: PendingBrowserPageTarget,
    completed: Result<CompletedPageCommand, String>,
}

#[derive(Clone)]
struct PendingBrowserPageTarget {
    browser_context_id: String,
    target_id: String,
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
                for page_command in pending {
                    completed.push(CompletedBrowserPageCommand {
                        target: page_command.target,
                        completed: page_command
                            .pending
                            .wait()
                            .await
                            .map_err(|error| error.to_string()),
                    });
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
            BrowserCommandTaskStep::Complete(get_window_for_target(conn))
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

fn bounds_json(bounds: &BrowserWindowBounds) -> Value {
    let mut value = json!({
        "windowState": bounds.window_state,
    });
    let object = value
        .as_object_mut()
        .expect("browser bounds json must be an object");
    if let Some(left) = bounds.left {
        object.insert("left".to_owned(), json!(left));
    }
    if let Some(top) = bounds.top {
        object.insert("top".to_owned(), json!(top));
    }
    if let Some(width) = bounds.width {
        object.insert("width".to_owned(), json!(width));
    }
    if let Some(height) = bounds.height {
        object.insert("height".to_owned(), json!(height));
    }
    value
}

fn get_window_for_target(conn: &CdpConnection) -> CommandOutputPlan {
    CommandOutputPlan::result(json!({
        "windowId": DEV_TOOLS_WINDOW_ID,
        "bounds": bounds_json(&conn.window_bounds)
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

    if *params.window_id.inner() != i64::from(DEV_TOOLS_WINDOW_ID) {
        return CommandOutputPlan::error(-32602, "InvalidParams");
    }

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
    conn.window_bounds.left = left;
    conn.window_bounds.top = top;
    conn.window_bounds.width = width;
    conn.window_bounds.height = height;
    if let Some(window_state) = params.bounds.window_state {
        conn.window_bounds.window_state = window_state.as_ref().to_owned();
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
    if !is_valid_download_behavior(params.behavior.as_str()) {
        return CommandOutputPlan::error(-32602, "InvalidParams");
    }

    match params.browser_context_id {
        Some(browser_context_id) => conn.download_behavior.set_browser_context_policy(
            browser_context_id,
            params.behavior,
            params.download_path,
        ),
        None => conn
            .download_behavior
            .set_global_policy(params.behavior, params.download_path),
    }
    conn.download_behavior
        .set_browser_events_enabled_for_session(cmd.session_id, params.events_enabled);

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
                    conn.download_behavior
                        .reset_browser_context(browser_context_id.as_str());
                }
            }
            None => conn.download_behavior.reset_global(),
        }
        return Ok(DevToolsCommandResult::Empty);
    };

    if !is_valid_download_behavior(behavior.behavior.as_str()) {
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "download behavior is invalid",
        ));
    }

    match target_contexts {
        Some(user_contexts) => {
            for browser_context_id in user_contexts {
                conn.download_behavior.set_browser_context(
                    browser_context_id.into_string(),
                    behavior.behavior.clone(),
                    behavior.download_path.clone(),
                    behavior.events_enabled,
                );
            }
        }
        None => conn.download_behavior.set_global(
            behavior.behavior,
            behavior.download_path,
            behavior.events_enabled,
        ),
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

    conn.permission_overrides.retain(|override_entry| {
        override_entry.permission != command.permission
            || override_entry.origin.as_deref() != Some(command.origin.as_str())
            || override_entry.embedded_origin != command.embedded_origin
            || override_entry.browser_context_id != browser_context_id
    });
    conn.permission_overrides
        .push(crate::conn::PermissionOverride {
            permission: command.permission,
            setting: normalize_permission_setting(command.setting),
            origin: Some(command.origin),
            embedded_origin: command.embedded_origin,
            browser_context_id,
        });

    let pending = start_loaded_page_permission_override_commands(conn)
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
        let completion = command
            .completed
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
        finish_pending_permission_override_command(conn, command.target, completion)
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

    conn.permission_overrides.retain(|override_entry| {
        override_entry.permission != params.permission
            || override_entry.origin != params.origin
            || override_entry.embedded_origin != params.embedded_origin
            || override_entry.browser_context_id != params.browser_context_id
    });
    conn.permission_overrides
        .push(crate::conn::PermissionOverride {
            permission: params.permission,
            setting: normalize_permission_setting(params.setting),
            origin: params.origin,
            embedded_origin: params.embedded_origin,
            browser_context_id: params.browser_context_id,
        });
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
        conn.permission_overrides.retain(|override_entry| {
            override_entry.permission != permission
                || override_entry.origin != params.origin
                || override_entry.embedded_origin.is_some()
                || override_entry.browser_context_id != params.browser_context_id
        });
        conn.permission_overrides
            .push(crate::conn::PermissionOverride {
                permission,
                setting: PermissionSetting::Granted.label().to_owned(),
                origin: params.origin.clone(),
                embedded_origin: None,
                browser_context_id: params.browser_context_id.clone(),
            });
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

    if let Some(browser_context_id) = params.browser_context_id {
        conn.permission_overrides.retain(|entry| {
            entry.browser_context_id.as_deref() != Some(browser_context_id.as_str())
        });
    } else {
        conn.permission_overrides.clear();
    }

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
    let pending = match start_loaded_page_permission_override_commands(conn) {
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

fn start_loaded_page_permission_override_commands(
    conn: &mut CdpConnection,
) -> Result<Vec<PendingBrowserPageCommand>, String> {
    let all_overrides = conn.permission_overrides.clone();
    let mut pending = Vec::new();
    for browser_context in conn
        .browser_context
        .iter_mut()
        .chain(conn.inactive_browser_contexts.iter_mut())
    {
        let browser_context_id = browser_context.id.clone();
        let effective_overrides = all_overrides
            .iter()
            .filter(|entry| {
                entry.browser_context_id.is_none()
                    || entry.browser_context_id.as_deref() == Some(browser_context_id.as_str())
            })
            .map(|entry| moli_core::page::PermissionOverrideRegistration {
                permission: entry.permission.clone(),
                setting: entry.setting.clone(),
                origin: entry.origin.clone(),
                embedded_origin: entry.embedded_origin.clone(),
            })
            .collect::<Vec<_>>();
        for target in browser_context.page_targets.iter_mut() {
            let target_id = target.target_id().to_owned();
            let Some(page) = target.loaded_page_mut() else {
                continue;
            };
            pending.push(PendingBrowserPageCommand {
                target: PendingBrowserPageTarget {
                    browser_context_id: browser_context_id.clone(),
                    target_id,
                },
                pending: page
                    .start_set_permission_overrides(&effective_overrides)
                    .map_err(|error| {
                        format!("failed to update page permission overrides: {error}")
                    })?,
            });
        }
    }
    Ok(pending)
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
                let completion = match command.completed {
                    Ok(completion) => completion,
                    Err(error) => {
                        return CommandOutputPlan::error(-32000, error);
                    }
                };
                if let Err(error) =
                    finish_pending_permission_override_command(conn, command.target, completion)
                {
                    return CommandOutputPlan::error(-32000, error);
                }
            }
            CommandOutputPlan::success()
        }
    }
}

fn finish_pending_permission_override_command(
    conn: &mut CdpConnection,
    target: PendingBrowserPageTarget,
    completion: CompletedPageCommand,
) -> Result<(), String> {
    let page = conn
        .browser_context_by_id_mut(&target.browser_context_id)
        .and_then(|browser_context| browser_context.page_target_mut(&target.target_id))
        .and_then(|target| target.loaded_page_mut())
        .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
    page.finish_set_permission_overrides(completion)
        .map_err(|error| error.to_string())
}

// ────────────────────────────────────────────────────────────────────────────
