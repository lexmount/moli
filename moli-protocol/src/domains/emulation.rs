use crate::conn::{
    BrowserContext, CdpConnection, CdpSessionRoute, Cmd, CommandOwnerScope, EmulatedDeviceMetrics,
    EmulatedGeolocationOverrideState, EmulatedViewportSurface, PageTargetHost,
    RendererCommandCorrelation, RendererCommandDescriptor, RuntimeInspectorAsyncCompletionReceiver,
    TargetWindowSurfaceState,
};
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandResult, DevToolsDevicePixelRatioSetting, DevToolsError,
    DevToolsErrorKind, DevToolsGeolocationOverride, DevToolsGeolocationOverrideState,
    DevToolsNetworkConditions, DevToolsSetClientWindowStateCommand,
    DevToolsSetClientWindowStateResult, DevToolsSetExtraHeadersCommand,
    DevToolsSetGeolocationOverrideCommand, DevToolsSetLocaleOverrideCommand,
    DevToolsSetNetworkConditionsCommand, DevToolsSetTimezoneOverrideCommand,
    DevToolsSetUserAgentOverrideCommand, DevToolsSetViewportCommand, DevToolsTargetId,
    DevToolsViewportSetting, DevToolsWindowState,
};
use crate::domains::actions::EmulationAction;
use crate::domains::command_output::CommandOutputPlan;
use moli_core::{
    RendererRuntimeInspectorResponseSender,
    page::{
        CompletedDevToolsIoCommandDispatch, CompletedPageCommand, PendingDevToolsIoCommandDispatch,
        PendingPageCommand,
    },
};
use serde_json::json;

mod device;
mod media;
mod page_session;
mod params;
#[cfg(test)]
mod tests;

pub(crate) struct PendingEmulationCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    pending: PendingEmulationRendererDispatch,
}

pub(crate) struct CompletedEmulationCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    completed: CompletedEmulationRendererDispatch,
}

enum PendingEmulationRendererDispatch {
    Pages(Vec<PendingEmulationPageCommand>),
    IoCommandReply(PendingDevToolsIoCommandDispatch),
    IoSessionOutput {
        pending: PendingDevToolsIoCommandDispatch,
        correlation: RendererCommandCorrelation,
    },
}

enum CompletedEmulationRendererDispatch {
    Pages(Vec<CompletedEmulationPageCommand>),
    IoCommandReply(Result<CompletedDevToolsIoCommandDispatch, String>),
    IoSessionOutput {
        completed: Result<CompletedDevToolsIoCommandDispatch, String>,
        correlation: RendererCommandCorrelation,
    },
}

struct PendingEmulationPageCommand {
    target: PendingEmulationPageTarget,
    operation: PendingEmulationPageOperation,
    pending: PendingPageCommand,
    runtime_response_rx: Option<RuntimeInspectorAsyncCompletionReceiver>,
}

struct CompletedEmulationPageCommand {
    target: PendingEmulationPageTarget,
    operation: PendingEmulationPageOperation,
    completed: Result<CompletedPageCommand, String>,
}

#[derive(Clone)]
enum PendingEmulationPageTarget {
    SessionOwner {
        owner_scope: CommandOwnerScope,
    },
    BrowserContextActive {
        browser_context_id: String,
    },
    BrowserContextBackground {
        browser_context_id: String,
        target_id: String,
    },
}

pub(crate) enum EmulationCommandTaskStep {
    Pending(PendingEmulationCommandDispatch),
    Complete(CommandOutputPlan),
}

enum PendingEmulationPageOperation {
    SetExtraHttpHeaders,
    SetLocaleOverride,
    SetNetworkConditions,
    SetCpuThrottlingRate,
    SetIdleOverride,
    SetTimezoneOverride,
    SetEmulatedMedia,
    SetViewportSurface,
    SetUserAgentLoader,
    ReplaceBrowserResourceRuntime,
    RuntimeProtocolMessage,
}

impl PendingEmulationCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedEmulationCommandDispatch {
        let completed = match self.pending {
            PendingEmulationRendererDispatch::Pages(pending_pages) => {
                let mut completed = Vec::with_capacity(pending_pages.len());
                for pending in pending_pages {
                    let PendingEmulationPageCommand {
                        target,
                        operation,
                        pending,
                        runtime_response_rx,
                    } = pending;
                    let completed_page = pending.wait().await.map_err(|error| error.to_string());
                    if completed_page.is_ok()
                        && let Some(response_rx) = runtime_response_rx
                    {
                        let _ = response_rx.await;
                    }
                    completed.push(CompletedEmulationPageCommand {
                        target,
                        operation,
                        completed: completed_page,
                    });
                }
                CompletedEmulationRendererDispatch::Pages(completed)
            }
            PendingEmulationRendererDispatch::IoCommandReply(pending) => {
                CompletedEmulationRendererDispatch::IoCommandReply(
                    pending.wait().await.map_err(|error| error.to_string()),
                )
            }
            PendingEmulationRendererDispatch::IoSessionOutput {
                pending,
                correlation,
            } => CompletedEmulationRendererDispatch::IoSessionOutput {
                completed: pending.wait().await.map_err(|error| error.to_string()),
                correlation,
            },
        };
        CompletedEmulationCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            completed,
        }
    }
}

impl CompletedEmulationCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

pub(crate) fn try_start_emulation_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<EmulationCommandTaskStep> {
    match cmd.parse_action::<EmulationAction>() {
        Some(EmulationAction::Enable | EmulationAction::Disable) => Some(
            EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({}))),
        ),
        Some(EmulationAction::SetFocusEmulationEnabled) => {
            Some(EmulationCommandTaskStep::Complete(
                focus_emulation_enabled_command_output_plan(conn, cmd),
            ))
        }
        Some(EmulationAction::SetDeviceMetricsOverride) => {
            Some(start_device_metrics_override_command(conn, cmd))
        }
        Some(EmulationAction::ClearDeviceMetricsOverride) => {
            Some(start_clear_device_metrics_override_command(conn, cmd))
        }
        Some(EmulationAction::SetCpuThrottlingRate) => {
            Some(start_cpu_throttling_rate_command(conn, cmd))
        }
        Some(EmulationAction::SetTouchEmulationEnabled) => {
            Some(EmulationCommandTaskStep::Complete(
                touch_emulation_enabled_command_output_plan(conn, cmd),
            ))
        }
        Some(EmulationAction::SetEmitTouchEventsForMouse) => {
            Some(EmulationCommandTaskStep::Complete(
                emit_touch_events_for_mouse_command_output_plan(conn, cmd),
            ))
        }
        Some(EmulationAction::SetScriptExecutionDisabled) => {
            Some(start_script_execution_disabled_command(conn, cmd))
        }
        Some(EmulationAction::SetGeolocationOverride) => {
            Some(start_geolocation_override_command(conn, cmd))
        }
        Some(EmulationAction::ClearGeolocationOverride) => {
            Some(start_clear_geolocation_override_command(conn, cmd))
        }
        Some(EmulationAction::SetIdleOverride) => Some(start_idle_override_command(conn, cmd)),
        Some(EmulationAction::ClearIdleOverride) => {
            Some(start_clear_idle_override_command(conn, cmd))
        }
        Some(EmulationAction::SetLocaleOverride) => Some(start_locale_override_command(conn, cmd)),
        Some(EmulationAction::SetTimezoneOverride) => {
            Some(start_timezone_override_command(conn, cmd))
        }
        Some(EmulationAction::SetUserAgentOverride) => {
            Some(start_user_agent_override_command(conn, cmd))
        }
        Some(EmulationAction::SetEmulatedMedia) => Some(start_emulated_media_command(conn, cmd)),
        None => Some(EmulationCommandTaskStep::Complete(
            CommandOutputPlan::error(-32601, "UnknownMethod"),
        )),
    }
}

fn focus_emulation_enabled_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: params::SetFocusEmulationEnabledParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if conn.browser_context.is_none() {
        return CommandOutputPlan::result(json!({}));
    }
    match page_session::mutate_page_session_state(conn, cmd.session_id, |state| {
        *state.focus_emulation_enabled = params.enabled;
    }) {
        Ok(()) => CommandOutputPlan::result(json!({})),
        Err(message) if message == "BrowserContextNotLoaded" => {
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded")
        }
        Err(message) => CommandOutputPlan::error(-32000, message),
    }
}

fn touch_emulation_enabled_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: params::SetTouchEmulationEnabledParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if conn.browser_context.is_none() {
        return CommandOutputPlan::result(json!({}));
    }
    match page_session::mutate_page_session_state(conn, cmd.session_id, |state| {
        *state.touch_emulation_enabled = params.enabled;
    }) {
        Ok(()) => CommandOutputPlan::result(json!({})),
        Err(message) if message == "BrowserContextNotLoaded" => {
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded")
        }
        Err(message) => CommandOutputPlan::error(-32000, message),
    }
}

fn start_cpu_throttling_rate_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetCpuThrottlingRateParams =
        match cmd.get_params::<params::SetCpuThrottlingRateParams>() {
            Ok(Some(params)) if params.rate.is_finite() => params,
            _ => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32602,
                    "InvalidParams",
                ));
            }
        };
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    if !conn.mutate_emulation_session_state_for_session_owner(cmd.session_id, |state| {
        if let Some(state) = state {
            *state.cpu_throttling_rate = params.rate;
        }
    }) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };
    match page.start_set_cpu_throttling_rate(params.rate) {
        Ok(pending) => EmulationCommandTaskStep::Pending(single_pending_emulation_dispatch(
            cmd.id,
            owner_scope,
            PendingEmulationPageOperation::SetCpuThrottlingRate,
            pending,
            None,
        )),
        Err(error) => {
            EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn emit_touch_events_for_mouse_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: params::SetEmitTouchEventsForMouseParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if conn.browser_context.is_none() {
        return CommandOutputPlan::result(json!({}));
    }
    match page_session::mutate_page_session_state(conn, cmd.session_id, |state| {
        *state.emit_touch_events_for_mouse = params.enabled;
    }) {
        Ok(()) => CommandOutputPlan::result(json!({})),
        Err(message) if message == "BrowserContextNotLoaded" => {
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded")
        }
        Err(message) => CommandOutputPlan::error(-32000, message),
    }
}

fn start_script_execution_disabled_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetScriptExecutionDisabledParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    if !conn.mutate_emulation_session_state_for_session_owner(cmd.session_id, |state| {
        if let Some(state) = state {
            *state.script_execution_disabled = params.value;
        }
    }) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let Some(attachment_id) = loaded_page_mut_for_session(conn, cmd.session_id)
        .and_then(|page| page.renderer_agent_attachment_id())
    else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let response_delivery = cmd.terminal_response_delivery(
        moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
    );
    if cmd.id.is_none()
        || response_delivery == moli_page_types::RendererInspectorResponseDelivery::CommandReply
    {
        let page = loaded_page_mut_for_session(conn, cmd.session_id)
            .expect("the captured Emulation Page must remain loaded synchronously");
        let pending = page.start_set_script_execution_disabled_from_io(params.value);
        return EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            pending: PendingEmulationRendererDispatch::IoCommandReply(pending),
        });
    }
    let command_id = cmd
        .id
        .expect("session output requires a frontend command id");
    let descriptor = RendererCommandDescriptor::set_script_execution_disabled(
        cmd.json.to_owned(),
        cmd.renderer_policy(),
        params.value,
        response_delivery,
    );
    let prepared = match conn.try_register_renderer_call_for_session_owner(
        cmd.session_id,
        command_id,
        Some(attachment_id),
        descriptor,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
        }
    };
    let (correlation, response, response_rx) = prepared.into_parts();
    debug_assert!(
        response_rx.is_none(),
        "Emulation session output must not allocate a command-reply receiver",
    );
    let pending = loaded_page_mut_for_session(conn, cmd.session_id)
        .filter(|page| page.renderer_agent_attachment_id() == Some(attachment_id))
        .ok_or_else(|| "Emulation renderer attachment changed before IO dispatch".to_owned())
        .and_then(|page| {
            page.start_set_script_execution_disabled_from_io_with_response(
                renderer_inspector_session_id,
                params.value,
                response,
            )
            .map_err(|error| error.to_string())
        });
    let pending = match pending {
        Ok(pending) => pending,
        Err(error) => {
            let removed = conn.take_renderer_call_if_correlation_matches_for_session_owner(
                cmd.session_id,
                correlation,
            );
            debug_assert!(removed);
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
        }
    };
    EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
        command_id: cmd.id,
        session_id: cmd.session_id.map(str::to_owned),
        pending: PendingEmulationRendererDispatch::IoSessionOutput {
            pending,
            correlation,
        },
    })
}

fn start_locale_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetLocaleOverrideParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    let locale_override = params.locale.clone().filter(|value| !value.is_empty());
    if let Err(message) =
        conn.set_devtools_locale_override_for_session_owner(cmd.session_id, locale_override.clone())
    {
        let code = if message == "BrowserContextNotLoaded" {
            -31998
        } else {
            -32000
        };
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(code, message));
    }
    let pending = if emulation_command_is_context_wide(conn, cmd.session_id) {
        match start_context_locale_override_page_commands(conn) {
            Ok(pending) => pending,
            Err(error) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
            }
        }
    } else {
        match start_session_locale_override_page_commands(conn, cmd.session_id) {
            Ok(pending) => pending,
            Err(error) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
            }
        }
    };
    if pending.is_empty() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
        command_id: cmd.id,
        session_id: cmd.session_id.map(str::to_owned),
        pending: PendingEmulationRendererDispatch::Pages(pending),
    })
}

fn start_idle_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetIdleOverrideParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    start_update_idle_override_command(
        conn,
        cmd,
        Some(moli_core::page::EmulatedIdleOverride {
            is_user_active: params.is_user_active,
            is_screen_unlocked: params.is_screen_unlocked,
        }),
    )
}

fn start_clear_idle_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    if cmd.get_params::<params::ClearIdleOverrideParams>().is_err() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "InvalidParams",
        ));
    }
    start_update_idle_override_command(conn, cmd, None)
}

fn start_update_idle_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    idle_override: Option<moli_core::page::EmulatedIdleOverride>,
) -> EmulationCommandTaskStep {
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };
    match page.start_set_idle_override(idle_override) {
        Ok(pending) => EmulationCommandTaskStep::Pending(single_pending_emulation_dispatch(
            cmd.id,
            owner_scope,
            PendingEmulationPageOperation::SetIdleOverride,
            pending,
            None,
        )),
        Err(error) => {
            EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn start_timezone_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetTimezoneOverrideParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let timezone_override = {
        let trimmed = params.timezone_id.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    };
    if let Err(message) = conn
        .set_devtools_timezone_override_for_session_owner(cmd.session_id, timezone_override.clone())
    {
        let code = if message == "BrowserContextNotLoaded" {
            -31998
        } else {
            -32000
        };
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(code, message));
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };
    match page.start_set_timezone_override(timezone_override.as_deref()) {
        Ok(pending) => EmulationCommandTaskStep::Pending(single_pending_emulation_dispatch(
            cmd.id,
            owner_scope,
            PendingEmulationPageOperation::SetTimezoneOverride,
            pending,
            None,
        )),
        Err(error) => {
            EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn start_geolocation_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetGeolocationOverrideParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => params::SetGeolocationOverrideParams::default(),
        Err(_) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let override_state = match media::geolocation_override_from_params(params) {
        Ok(value) => value,
        Err(()) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    start_update_geolocation_override_command(conn, cmd, Some(override_state))
}

fn start_clear_geolocation_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    if cmd
        .get_params::<params::ClearGeolocationOverrideParams>()
        .is_err()
    {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "InvalidParams",
        ));
    }
    start_update_geolocation_override_command(conn, cmd, None)
}

fn start_update_geolocation_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    override_state: Option<EmulatedGeolocationOverrideState>,
) -> EmulationCommandTaskStep {
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    if !conn.mutate_emulation_session_state_for_session_owner(cmd.session_id, |state| {
        if let Some(state) = state {
            *state.geolocation_override = override_state.clone();
        }
    }) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let pending = match start_geolocation_surface_override_page_commands(conn, cmd) {
        Ok(pending) => pending,
        Err(error) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
        }
    };
    if pending.is_empty() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
        command_id: cmd.id,
        session_id: cmd.session_id.map(str::to_owned),
        pending: PendingEmulationRendererDispatch::Pages(pending),
    })
}

fn start_emulated_media_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetEmulatedMediaParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    let overrides = media::emulated_media_overrides_from_params(params);
    if !conn.mutate_emulation_session_state_for_session_owner(cmd.session_id, |state| {
        if let Some(state) = state {
            *state.emulated_media = overrides.clone();
        }
    }) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let page_overrides: moli_core::page::EmulatedMediaOverrides = (&overrides).into();
    let pending = if emulation_command_is_context_wide(conn, cmd.session_id) {
        match start_context_emulated_media_page_commands(conn, &page_overrides) {
            Ok(pending) => pending,
            Err(error) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
            }
        }
    } else {
        let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
        let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
        };
        match page.start_set_emulated_media(&page_overrides) {
            Ok(pending) => vec![PendingEmulationPageCommand {
                target: PendingEmulationPageTarget::SessionOwner { owner_scope },
                operation: PendingEmulationPageOperation::SetEmulatedMedia,
                pending,
                runtime_response_rx: None,
            }],
            Err(error) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
        }
    };
    if pending.is_empty() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
        command_id: cmd.id,
        session_id: cmd.session_id.map(str::to_owned),
        pending: PendingEmulationRendererDispatch::Pages(pending),
    })
}

fn start_user_agent_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let base_identity = conn.base_browser_identity().clone();
    let browser_identity = match crate::domains::network::settings::user_agent_override_for_command(
        cmd,
        &base_identity,
    ) {
        Ok(browser_identity) => browser_identity,
        Err(plan) => return EmulationCommandTaskStep::Complete(plan),
    };
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    match conn.start_set_devtools_browser_identity_override_for_session_owner(
        cmd.session_id,
        browser_identity,
    ) {
        Ok(Some(pending)) => EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            pending: PendingEmulationRendererDispatch::Pages(vec![PendingEmulationPageCommand {
                target: PendingEmulationPageTarget::SessionOwner { owner_scope },
                operation: PendingEmulationPageOperation::SetUserAgentLoader,
                pending,
                runtime_response_rx: None,
            }]),
        }),
        Ok(None) => EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({}))),
        Err(message) if message == "BrowserContextNotLoaded" => EmulationCommandTaskStep::Complete(
            CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
        ),
        Err(message) => {
            EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message))
        }
    }
}

fn start_device_metrics_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetDeviceMetricsOverrideParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if conn.browser_context.is_none()
        && conn
            .target_owner_identity_for_session(cmd.session_id)
            .is_none()
    {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    let (Ok(width), Ok(height)) = (u32::try_from(params.width), u32::try_from(params.height))
    else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "InvalidParams",
        ));
    };
    let screen_width = match params.screen_width {
        Some(value) => match value.try_into() {
            Ok(value) => value,
            Err(_) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32602,
                    "InvalidParams",
                ));
            }
        },
        None => width,
    };
    let screen_height = match params.screen_height {
        Some(value) => match value.try_into() {
            Ok(value) => value,
            Err(_) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32602,
                    "InvalidParams",
                ));
            }
        },
        None => height,
    };
    let command = DevToolsSetViewportCommand {
        context: cmd.devtools_command_context(None::<&str>, None::<&str>),
        browser_context_ids: Vec::new(),
        viewport: DevToolsViewportSetting::Dimensions { width, height },
        device_pixel_ratio: DevToolsDevicePixelRatioSetting::Scale(params.device_scale_factor),
        screen_width: Some(screen_width),
        screen_height: Some(screen_height),
    };
    match start_devtools_set_viewport_command(conn, cmd.id, command) {
        Ok(Some(pending)) => EmulationCommandTaskStep::Pending(pending),
        Ok(None) => EmulationCommandTaskStep::Complete(CommandOutputPlan::success()),
        Err(error) => {
            EmulationCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error))
        }
    }
}

fn start_clear_device_metrics_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    if conn.browser_context.is_none()
        && conn
            .target_owner_identity_for_session(cmd.session_id)
            .is_none()
    {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    if !conn.mutate_emulation_session_state_for_session_owner(cmd.session_id, |state| {
        if let Some(state) = state {
            *state.emulated_device_metrics = None;
        }
    }) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let runtime_call_id = conn.next_internal_runtime_command_id();
    let Some(page) = loaded_page_mut_for_session(conn, cmd.session_id) else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };
    let pending_viewport = match page.start_set_viewport_surface(None) {
        Ok(pending) => pending,
        Err(error) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                error.to_string(),
            ));
        }
    };
    match start_runtime_emulation_protocol_message(
        page,
        runtime_call_id,
        device::LIVE_DEVICE_METRICS_CLEAR_SCRIPT.to_owned(),
    ) {
        Ok((pending_runtime, runtime_response_rx)) => {
            let session_id = owner_scope.session_id().map(str::to_owned);
            EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
                command_id: cmd.id,
                session_id: session_id.clone(),
                pending: PendingEmulationRendererDispatch::Pages(vec![
                    PendingEmulationPageCommand {
                        target: PendingEmulationPageTarget::SessionOwner {
                            owner_scope: owner_scope.clone(),
                        },
                        operation: PendingEmulationPageOperation::SetViewportSurface,
                        pending: pending_viewport,
                        runtime_response_rx: None,
                    },
                    PendingEmulationPageCommand {
                        target: PendingEmulationPageTarget::SessionOwner { owner_scope },
                        operation: PendingEmulationPageOperation::RuntimeProtocolMessage,
                        pending: pending_runtime,
                        runtime_response_rx,
                    },
                ]),
            })
        }
        Err(error) => EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error)),
    }
}

fn start_devtools_set_viewport_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsSetViewportCommand,
) -> Result<Option<PendingEmulationCommandDispatch>, DevToolsError> {
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    if conn.browser_context.is_none()
        && conn.target_owner_identity_for_session(session_id).is_none()
    {
        return Ok(None);
    }
    let metrics = set_viewport_metrics_from_command(conn, session_id, &command)?;
    let had_existing_device_metrics = conn
        .target_session_owner_emulated_device_metrics(session_id)
        .is_some();
    if !conn.mutate_emulation_session_state_for_session_owner(session_id, |state| {
        if let Some(state) = state {
            *state.emulated_device_metrics = Some(metrics.clone());
        }
    }) {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    }
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let runtime_call_id = conn.next_internal_runtime_command_id();
    let Some(page) = loaded_page_mut_for_session(conn, session_id) else {
        return Ok(None);
    };
    let session_id = session_id.map(str::to_owned);
    let viewport_surface = Some(metrics.viewport_surface().to_page_viewport_surface());
    let pending_viewport = page
        .start_set_viewport_surface(viewport_surface)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))?;
    let script =
        device::live_device_metrics_override_script(&metrics, !had_existing_device_metrics);
    let (pending_runtime, runtime_response_rx) =
        start_runtime_emulation_protocol_message(page, runtime_call_id, script)
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
    Ok(Some(PendingEmulationCommandDispatch {
        command_id,
        session_id: session_id.clone(),
        pending: PendingEmulationRendererDispatch::Pages(vec![
            PendingEmulationPageCommand {
                target: PendingEmulationPageTarget::SessionOwner {
                    owner_scope: owner_scope.clone(),
                },
                operation: PendingEmulationPageOperation::SetViewportSurface,
                pending: pending_viewport,
                runtime_response_rx: None,
            },
            PendingEmulationPageCommand {
                target: PendingEmulationPageTarget::SessionOwner { owner_scope },
                operation: PendingEmulationPageOperation::RuntimeProtocolMessage,
                pending: pending_runtime,
                runtime_response_rx,
            },
        ]),
    }))
}

fn set_viewport_metrics_from_command(
    conn: &CdpConnection,
    session_id: Option<&str>,
    command: &DevToolsSetViewportCommand,
) -> Result<EmulatedDeviceMetrics, DevToolsError> {
    let current_metrics = conn.target_session_owner_emulated_device_metrics(session_id);
    set_viewport_metrics_from_current(current_metrics.as_ref(), command)
}

fn set_viewport_metrics_from_current(
    current_metrics: Option<&EmulatedDeviceMetrics>,
    command: &DevToolsSetViewportCommand,
) -> Result<EmulatedDeviceMetrics, DevToolsError> {
    let current = EmulatedViewportSurface::from_metrics(current_metrics);
    let default = EmulatedViewportSurface::default();
    let (width, height) = match command.viewport {
        DevToolsViewportSetting::Unchanged => (current.inner_width, current.inner_height),
        DevToolsViewportSetting::Default => (default.inner_width, default.inner_height),
        DevToolsViewportSetting::Dimensions { width, height } => (width, height),
    };
    let device_scale_factor = match command.device_pixel_ratio {
        DevToolsDevicePixelRatioSetting::Unchanged => current.device_pixel_ratio,
        DevToolsDevicePixelRatioSetting::Default => default.device_pixel_ratio,
        DevToolsDevicePixelRatioSetting::Scale(value) => value,
    };
    if !device_scale_factor.is_finite() || device_scale_factor <= 0.0 {
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "InvalidParams",
        ));
    }
    Ok(EmulatedDeviceMetrics {
        width,
        height,
        device_scale_factor,
        screen_width: command.screen_width.unwrap_or(width),
        screen_height: command.screen_height.unwrap_or(height),
    })
}

pub(crate) async fn execute_devtools_emulation_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    match command {
        DevToolsCommand::SetViewport(command) => {
            execute_devtools_set_viewport_command_async(conn, command).await
        }
        DevToolsCommand::SetWindowState(command) => {
            execute_devtools_set_window_state_command_async(conn, command).await
        }
        DevToolsCommand::SetClientWindowState(command) => {
            execute_devtools_set_client_window_state_command_async(conn, command).await
        }
        DevToolsCommand::SetUserAgentOverride(command) => {
            execute_devtools_set_user_agent_override_command_async(conn, command).await
        }
        DevToolsCommand::SetLocaleOverride(command) => {
            execute_devtools_set_locale_override_command_async(conn, command).await
        }
        DevToolsCommand::SetTimezoneOverride(command) => {
            execute_devtools_set_timezone_override_command_async(conn, command).await
        }
        DevToolsCommand::SetGeolocationOverride(command) => {
            execute_devtools_set_geolocation_override_command_async(conn, command).await
        }
        DevToolsCommand::SetNetworkConditions(command) => {
            execute_devtools_set_network_conditions_command_async(conn, command).await
        }
        DevToolsCommand::SetExtraHeaders(command) => {
            execute_devtools_set_extra_headers_command_async(conn, command).await
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    }
}

async fn execute_devtools_set_extra_headers_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetExtraHeadersCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.target_ids.is_empty() {
        return execute_devtools_set_extra_headers_for_targets(conn, command).await;
    }
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_extra_headers_for_browser_contexts(conn, command).await;
    }
    execute_devtools_set_extra_headers_global(conn, command).await
}

async fn execute_devtools_set_extra_headers_global(
    conn: &mut CdpConnection,
    command: DevToolsSetExtraHeadersCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    conn.set_global_extra_headers(command.headers.clone());
    let routes = top_level_target_routes_for_browser_contexts(conn, None);
    execute_extra_headers_updates_for_routes(
        conn,
        devtools_command_session_id(&command.context),
        routes,
    )
    .await
}

async fn execute_devtools_set_extra_headers_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetExtraHeadersCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    for browser_context_id in &browser_context_ids {
        let browser_context = conn
            .browser_context_by_id_mut(browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context.default_extra_headers = command.headers.clone();
    }
    let routes = top_level_target_routes_for_browser_contexts(conn, Some(&browser_context_ids));
    execute_extra_headers_updates_for_routes(
        conn,
        devtools_command_session_id(&command.context),
        routes,
    )
    .await
}

async fn execute_devtools_set_extra_headers_for_targets(
    conn: &mut CdpConnection,
    command: DevToolsSetExtraHeadersCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for target_id in &command.target_ids {
        let route = emulation_route_for_target(
            conn,
            target_id,
            "ChildFrameContextNotSupportedForSetExtraHeaders",
        )?;
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_extra_headers_for_current_route(
                route_scope.conn_mut(),
                &route,
                command.headers.clone(),
            )
        };
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, devtools_command_session_id(&command.context), pending)
        .await
}

async fn execute_devtools_set_network_conditions_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetNetworkConditionsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.target_ids.is_empty() {
        return execute_devtools_set_network_conditions_for_targets(conn, command).await;
    }
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_network_conditions_for_browser_contexts(conn, command).await;
    }
    execute_devtools_set_network_conditions_global(conn, command).await
}

async fn execute_devtools_set_geolocation_override_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetGeolocationOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.target_ids.is_empty() {
        return execute_devtools_set_geolocation_override_for_targets(conn, command).await;
    }
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_geolocation_override_for_browser_contexts(conn, command).await;
    }
    execute_devtools_set_geolocation_override_global(conn, command).await
}

async fn execute_devtools_set_geolocation_override_global(
    conn: &mut CdpConnection,
    command: DevToolsSetGeolocationOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    conn.set_global_geolocation_override(
        command
            .override_state
            .map(emulated_geolocation_override_state),
    );
    let routes = top_level_target_routes_for_browser_contexts(conn, None);
    execute_geolocation_surface_updates_for_routes(
        conn,
        devtools_command_session_id(&command.context),
        routes,
    )
    .await
}

async fn execute_devtools_set_geolocation_override_for_targets(
    conn: &mut CdpConnection,
    command: DevToolsSetGeolocationOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for target_id in &command.target_ids {
        let route = emulation_route_for_target(
            conn,
            target_id,
            "ChildFrameContextNotSupportedForGeolocationOverride",
        )?;
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_geolocation_override_for_current_route(
                route_scope.conn_mut(),
                &route,
                command
                    .override_state
                    .map(emulated_geolocation_override_state),
            )
        };
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, devtools_command_session_id(&command.context), pending)
        .await
}

async fn execute_devtools_set_geolocation_override_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetGeolocationOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    for browser_context_id in &browser_context_ids {
        let browser_context = conn
            .browser_context_by_id_mut(browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context.default_geolocation_override = command
            .override_state
            .map(emulated_geolocation_override_state);
    }
    let routes = top_level_target_routes_for_browser_contexts(conn, Some(&browser_context_ids));
    execute_geolocation_surface_updates_for_routes(
        conn,
        devtools_command_session_id(&command.context),
        routes,
    )
    .await
}

async fn execute_geolocation_surface_updates_for_routes(
    conn: &mut CdpConnection,
    session_id: Option<String>,
    routes: Vec<CdpSessionRoute>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for route in routes {
        let target = pending_emulation_target_for_route(&route)?;
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route);
            start_surface_override_for_route(route_scope.conn_mut(), target)
        };
        pending.extend(
            result.map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?,
        );
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

fn start_geolocation_override_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    override_state: Option<EmulatedGeolocationOverrideState>,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    if !conn.mutate_emulation_session_state_for_session_owner(None, |state| {
        if let Some(state) = state {
            *state.geolocation_override = override_state;
        }
    }) {
        return Err(devtools_emulation_owner_error(
            "BrowserContextNotLoaded".to_owned(),
        ));
    }
    let target = pending_emulation_target_for_route(route)?;
    start_surface_override_for_route(conn, target)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))
}

async fn execute_devtools_set_network_conditions_global(
    conn: &mut CdpConnection,
    command: DevToolsSetNetworkConditionsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    conn.set_global_network_conditions(command.network_conditions.map(emulated_network_conditions));
    let routes = top_level_target_routes_for_browser_contexts(conn, None);
    execute_network_conditions_updates_for_routes(
        conn,
        devtools_command_session_id(&command.context),
        routes,
    )
    .await
}

async fn execute_devtools_set_network_conditions_for_targets(
    conn: &mut CdpConnection,
    command: DevToolsSetNetworkConditionsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for target_id in &command.target_ids {
        let route = emulation_route_for_target(
            conn,
            target_id,
            "ChildFrameContextNotSupportedForNetworkConditions",
        )?;
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_network_conditions_for_current_route(
                route_scope.conn_mut(),
                &route,
                command.network_conditions,
            )
        };
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, devtools_command_session_id(&command.context), pending)
        .await
}

async fn execute_devtools_set_network_conditions_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetNetworkConditionsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    for browser_context_id in &browser_context_ids {
        let browser_context = conn
            .browser_context_by_id_mut(browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context.default_network_conditions =
            command.network_conditions.map(emulated_network_conditions);
    }
    let routes = top_level_target_routes_for_browser_contexts(conn, Some(&browser_context_ids));
    execute_network_conditions_updates_for_routes(
        conn,
        devtools_command_session_id(&command.context),
        routes,
    )
    .await
}

async fn execute_network_conditions_updates_for_routes(
    conn: &mut CdpConnection,
    session_id: Option<String>,
    routes: Vec<CdpSessionRoute>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for route in routes {
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_network_conditions_update_for_current_route(route_scope.conn_mut(), &route)
        };
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

fn start_network_conditions_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    network_conditions: Option<DevToolsNetworkConditions>,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    if !conn.mutate_emulation_session_state_for_session_owner(None, |state| {
        if let Some(state) = state {
            *state.network_conditions = network_conditions.map(emulated_network_conditions);
        }
    }) {
        return Err(devtools_emulation_owner_error(
            "BrowserContextNotLoaded".to_owned(),
        ));
    }
    start_network_conditions_update_for_current_route(conn, route)
}

fn start_network_conditions_update_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(route)?;
    let effective_offline = match &target {
        PendingEmulationPageTarget::BrowserContextActive { browser_context_id } => conn
            .browser_context_by_id(browser_context_id)
            .is_some_and(|browser_context| browser_context.effective_active_network_offline()),
        PendingEmulationPageTarget::BrowserContextBackground {
            browser_context_id,
            target_id,
        } => conn
            .browser_context_by_id(browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.effective_parked_network_offline(target_id)
            }),
        PendingEmulationPageTarget::SessionOwner { .. } => false,
    };
    let network_update = conn
        .start_set_emulated_network_conditions_for_session_owner(
            None,
            effective_offline,
            0.0,
            -1.0,
            -1.0,
            None,
        )
        .map_err(devtools_emulation_owner_error)?;
    let mut pending = Vec::new();
    if let Some(network_update) = network_update {
        pending.push(PendingEmulationPageCommand {
            target: target.clone(),
            operation: PendingEmulationPageOperation::SetNetworkConditions,
            pending: network_update,
            runtime_response_rx: None,
        });
    }
    pending.extend(
        start_surface_override_for_route(conn, target)
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?,
    );
    Ok(pending)
}

fn start_extra_headers_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    headers: Vec<(String, String)>,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(route)?;
    let pending = conn
        .start_set_target_extra_http_headers_for_session_owner(None, headers)
        .map_err(devtools_emulation_owner_error)?;
    Ok(pending
        .map(|pending| {
            vec![PendingEmulationPageCommand {
                target,
                operation: PendingEmulationPageOperation::SetExtraHttpHeaders,
                pending,
                runtime_response_rx: None,
            }]
        })
        .unwrap_or_default())
}

async fn execute_extra_headers_updates_for_routes(
    conn: &mut CdpConnection,
    session_id: Option<String>,
    routes: Vec<CdpSessionRoute>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for route in routes {
        pending.extend(start_extra_headers_update_for_route(conn, &route)?);
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

fn start_extra_headers_update_for_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(route)?;
    let headers = match &target {
        PendingEmulationPageTarget::BrowserContextActive { browser_context_id } => conn
            .browser_context_by_id(browser_context_id)
            .map(|browser_context| browser_context.effective_extra_headers()),
        PendingEmulationPageTarget::BrowserContextBackground {
            browser_context_id,
            target_id,
        } => conn
            .browser_context_by_id(browser_context_id)
            .map(|browser_context| browser_context.effective_parked_extra_headers(target_id)),
        PendingEmulationPageTarget::SessionOwner { .. } => None,
    };
    let Some(headers) = headers else {
        return Ok(Vec::new());
    };
    let Some(page) = loaded_page_mut_for_pending_emulation_target(conn, &target) else {
        return Ok(Vec::new());
    };
    let pending = page
        .start_set_extra_http_headers(&headers)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))?;
    Ok(vec![PendingEmulationPageCommand {
        target,
        operation: PendingEmulationPageOperation::SetExtraHttpHeaders,
        pending,
        runtime_response_rx: None,
    }])
}

fn loaded_page_mut_for_pending_emulation_target<'a>(
    conn: &'a mut CdpConnection,
    target: &PendingEmulationPageTarget,
) -> Option<&'a moli_core::page::Page> {
    match target {
        PendingEmulationPageTarget::BrowserContextActive { browser_context_id } => conn
            .browser_context_by_id_mut(browser_context_id)
            .and_then(|browser_context| {
                browser_context.active_target.runtime_slot.loaded_page_mut()
            })
            .map(|page| &*page),
        PendingEmulationPageTarget::BrowserContextBackground {
            browser_context_id,
            target_id,
        } => conn
            .browser_context_by_id_mut(browser_context_id)
            .and_then(|browser_context| browser_context.background_target_mut(target_id))
            .and_then(|target| target.loaded_page_mut())
            .map(|page| &*page),
        PendingEmulationPageTarget::SessionOwner { owner_scope } => {
            loaded_page_mut_for_session(conn, owner_scope.session_id()).map(|page| &*page)
        }
    }
}

fn emulated_network_conditions(
    conditions: DevToolsNetworkConditions,
) -> crate::conn::EmulatedNetworkConditions {
    if conditions.offline {
        crate::conn::EmulatedNetworkConditions::offline()
    } else {
        unreachable!("only offline BiDi network conditions are currently supported")
    }
}

fn emulated_geolocation_override(
    override_state: DevToolsGeolocationOverride,
) -> crate::conn::EmulatedGeolocationOverride {
    crate::conn::EmulatedGeolocationOverride {
        latitude: override_state.latitude,
        longitude: override_state.longitude,
        accuracy: override_state.accuracy,
        altitude: override_state.altitude,
        altitude_accuracy: override_state.altitude_accuracy,
        heading: override_state.heading,
        speed: override_state.speed,
    }
}

fn emulated_geolocation_override_state(
    override_state: DevToolsGeolocationOverrideState,
) -> EmulatedGeolocationOverrideState {
    match override_state {
        DevToolsGeolocationOverrideState::Position(position) => {
            EmulatedGeolocationOverrideState::Position(emulated_geolocation_override(position))
        }
        DevToolsGeolocationOverrideState::PositionUnavailable => {
            EmulatedGeolocationOverrideState::PositionUnavailable
        }
    }
}

async fn execute_devtools_set_user_agent_override_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetUserAgentOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.target_ids.is_empty() {
        return execute_devtools_set_user_agent_override_for_targets(conn, command).await;
    }
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_user_agent_override_for_browser_contexts(conn, command).await;
    }
    conn.set_global_browser_identity_override_from_user_agent(command.user_agent.clone());
    let routes = top_level_target_routes_for_browser_contexts(conn, None);
    execute_user_agent_loader_updates_for_routes(
        conn,
        command
            .context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str().to_owned()),
        routes,
    )
    .await
}

async fn execute_devtools_set_user_agent_override_for_targets(
    conn: &mut CdpConnection,
    command: DevToolsSetUserAgentOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for target_id in &command.target_ids {
        let route = emulation_route_for_target(
            conn,
            target_id,
            "ChildFrameContextNotSupportedForUserAgentOverride",
        )?;
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_user_agent_override_for_current_route(
                route_scope.conn_mut(),
                &route,
                command.user_agent.clone(),
            )
        };
        if let Some(pending_command) = result? {
            pending.push(pending_command);
        }
    }
    complete_emulation_page_updates(
        conn,
        command
            .context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str().to_owned()),
        pending,
    )
    .await
}

async fn execute_devtools_set_user_agent_override_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetUserAgentOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    let browser_identity = command.user_agent.as_ref().map(|user_agent| {
        moli_browser_profile::BrowserIdentityProfile::new(
            user_agent.clone(),
            conn.fetch_config().browser_identity().accept_language(),
        )
    });
    for browser_context_id in &browser_context_ids {
        let browser_context = conn
            .browser_context_by_id_mut(browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context.default_browser_identity_override = browser_identity.clone();
    }
    let routes = top_level_target_routes_for_browser_contexts(conn, Some(&browser_context_ids));
    execute_user_agent_loader_updates_for_routes(
        conn,
        command
            .context
            .session_id
            .as_ref()
            .map(|session_id| session_id.as_str().to_owned()),
        routes,
    )
    .await
}

async fn execute_user_agent_loader_updates_for_routes(
    conn: &mut CdpConnection,
    session_id: Option<String>,
    routes: Vec<CdpSessionRoute>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for route in routes {
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_user_agent_loader_update_for_current_route(route_scope.conn_mut(), &route)
        };
        if let Some(pending_command) = result? {
            pending.push(pending_command);
        }
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

async fn complete_emulation_page_updates(
    conn: &mut CdpConnection,
    session_id: Option<String>,
    pending: Vec<PendingEmulationPageCommand>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if pending.is_empty() {
        return Ok(DevToolsCommandResult::Empty);
    }
    complete_pending_devtools_emulation_command(
        conn,
        PendingEmulationCommandDispatch {
            command_id: None,
            session_id,
            pending: PendingEmulationRendererDispatch::Pages(pending),
        }
        .wait()
        .await,
    )
}

fn start_user_agent_override_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    user_agent: Option<String>,
) -> Result<Option<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(route)?;
    let pending = conn
        .start_set_base_user_agent_override_for_session_owner(None, user_agent)
        .map_err(devtools_emulation_owner_error)?;
    if let Some(pending) = pending {
        return Ok(Some(PendingEmulationPageCommand {
            target,
            operation: PendingEmulationPageOperation::ReplaceBrowserResourceRuntime,
            pending,
            runtime_response_rx: None,
        }));
    }
    start_user_agent_loader_update_for_current_route(conn, route)
}

fn start_user_agent_loader_update_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
) -> Result<Option<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(route)?;
    let load_inputs = conn.navigation_load_inputs_for_session_owner(None);
    let resource_runtime = conn
        .build_registered_browser_resource_runtime_for_navigation_load_inputs(&load_inputs)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
    let Some(page) = loaded_page_mut_for_session(conn, None) else {
        return Ok(None);
    };
    let pending = page
        .start_replace_browser_resource_runtime(&resource_runtime)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))?;
    Ok(Some(PendingEmulationPageCommand {
        target,
        operation: PendingEmulationPageOperation::ReplaceBrowserResourceRuntime,
        pending,
        runtime_response_rx: None,
    }))
}

async fn execute_devtools_set_locale_override_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetLocaleOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.target_ids.is_empty() {
        return execute_devtools_set_locale_override_for_targets(conn, command).await;
    }
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_locale_override_for_browser_contexts(conn, command).await;
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::InvalidArgument,
        "LocaleOverrideRequiresContextOrUserContext",
    ))
}

async fn execute_devtools_set_locale_override_for_targets(
    conn: &mut CdpConnection,
    command: DevToolsSetLocaleOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for target_id in &command.target_ids {
        let route = emulation_route_for_target(
            conn,
            target_id,
            "ChildFrameContextNotSupportedForLocaleOverride",
        )?;
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_locale_override_for_current_route(
                route_scope.conn_mut(),
                &route,
                command.locale.clone(),
            )
        };
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, devtools_command_session_id(&command.context), pending)
        .await
}

async fn execute_devtools_set_locale_override_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetLocaleOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    for browser_context_id in &browser_context_ids {
        let browser_context = conn
            .browser_context_by_id_mut(browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context.default_locale_override = command.locale.clone();
    }
    let routes = top_level_target_routes_for_browser_contexts(conn, Some(&browser_context_ids));
    execute_locale_updates_for_routes(conn, devtools_command_session_id(&command.context), routes)
        .await
}

async fn execute_locale_updates_for_routes(
    conn: &mut CdpConnection,
    session_id: Option<String>,
    routes: Vec<CdpSessionRoute>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for route in routes {
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_locale_update_for_current_route(route_scope.conn_mut(), &route)
        };
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

fn start_locale_override_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    locale: Option<String>,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    if !conn.set_base_locale_override_for_session_owner(None, locale) {
        return Err(devtools_emulation_owner_error(
            "BrowserContextNotLoaded".to_owned(),
        ));
    }
    start_locale_update_for_current_route(conn, route)
}

fn start_locale_update_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(route)?;
    let Some(locale_override) = locale_override_for_session(conn, None) else {
        return Ok(Vec::new());
    };
    let Some(page) = loaded_page_mut_for_session(conn, None) else {
        return Ok(Vec::new());
    };
    start_locale_override_page_command(target, page, locale_override.as_deref())
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))
}

async fn execute_devtools_set_timezone_override_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetTimezoneOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.target_ids.is_empty() {
        return execute_devtools_set_timezone_override_for_targets(conn, command).await;
    }
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_timezone_override_for_browser_contexts(conn, command).await;
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::InvalidArgument,
        "TimezoneOverrideRequiresContextOrUserContext",
    ))
}

async fn execute_devtools_set_timezone_override_for_targets(
    conn: &mut CdpConnection,
    command: DevToolsSetTimezoneOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for target_id in &command.target_ids {
        let route = emulation_route_for_target(
            conn,
            target_id,
            "ChildFrameContextNotSupportedForTimezoneOverride",
        )?;
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_timezone_override_for_current_route(
                route_scope.conn_mut(),
                &route,
                command.timezone.clone(),
            )
        };
        if let Some(pending_command) = result? {
            pending.push(pending_command);
        }
    }
    complete_emulation_page_updates(conn, devtools_command_session_id(&command.context), pending)
        .await
}

async fn execute_devtools_set_timezone_override_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetTimezoneOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    for browser_context_id in &browser_context_ids {
        let browser_context = conn
            .browser_context_by_id_mut(browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context.default_timezone_override = command.timezone.clone();
    }
    let routes = top_level_target_routes_for_browser_contexts(conn, Some(&browser_context_ids));
    execute_timezone_updates_for_routes(conn, devtools_command_session_id(&command.context), routes)
        .await
}

async fn execute_timezone_updates_for_routes(
    conn: &mut CdpConnection,
    session_id: Option<String>,
    routes: Vec<CdpSessionRoute>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut pending = Vec::new();
    for route in routes {
        let result = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route.clone());
            start_timezone_update_for_current_route(route_scope.conn_mut(), &route)
        };
        if let Some(pending_command) = result? {
            pending.push(pending_command);
        }
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

fn start_timezone_override_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    timezone: Option<String>,
) -> Result<Option<PendingEmulationPageCommand>, DevToolsError> {
    if !conn.set_base_timezone_override_for_session_owner(None, timezone) {
        return Err(devtools_emulation_owner_error(
            "BrowserContextNotLoaded".to_owned(),
        ));
    }
    start_timezone_update_for_current_route(conn, route)
}

fn start_timezone_update_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
) -> Result<Option<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(route)?;
    let load_inputs = conn.navigation_load_inputs_for_session_owner(None);
    let Some(page) = loaded_page_mut_for_session(conn, None) else {
        return Ok(None);
    };
    let pending = page
        .start_set_timezone_override(load_inputs.timezone_override.as_deref())
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))?;
    Ok(Some(PendingEmulationPageCommand {
        target,
        operation: PendingEmulationPageOperation::SetTimezoneOverride,
        pending,
        runtime_response_rx: None,
    }))
}

fn pending_emulation_target_for_route(
    route: &CdpSessionRoute,
) -> Result<PendingEmulationPageTarget, DevToolsError> {
    match route {
        CdpSessionRoute::ActiveTarget {
            browser_context_id, ..
        } => Ok(PendingEmulationPageTarget::BrowserContextActive {
            browser_context_id: browser_context_id.clone(),
        }),
        CdpSessionRoute::PageTargetHost {
            browser_context_id,
            target_id,
        } => Ok(PendingEmulationPageTarget::BrowserContextBackground {
            browser_context_id: browser_context_id.clone(),
            target_id: target_id.clone(),
        }),
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "UnsupportedEmulationTarget",
        )),
    }
}

fn emulation_route_for_target(
    conn: &CdpConnection,
    target_id: &DevToolsTargetId,
    child_frame_error: &'static str,
) -> Result<CdpSessionRoute, DevToolsError> {
    if let Some(route) = conn.target_session_route_for_target_id(target_id.as_str()) {
        return Ok(route);
    }
    if conn.has_attached_child_frame_id(target_id.as_str()) {
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            child_frame_error,
        ));
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchTarget",
    ))
}

fn devtools_command_session_id(
    context: &crate::devtools_runtime::DevToolsCommandContext,
) -> Option<String> {
    context
        .session_id
        .as_ref()
        .map(|session_id| session_id.as_str().to_owned())
}

fn resolve_bidi_browser_context_ids(
    conn: &mut CdpConnection,
    browser_context_ids: &[crate::devtools_runtime::DevToolsBrowserContextId],
) -> Result<Vec<String>, DevToolsError> {
    let mut resolved = Vec::new();
    for browser_context_id in browser_context_ids {
        let browser_context_id = browser_context_id.as_str();
        if browser_context_id == "default" {
            let mut default_context_ids = conn
                .browser_contexts()
                .filter(|context| is_moli_internal_default_user_context(&context.id))
                .map(|context| context.id.clone())
                .collect::<Vec<_>>();
            if default_context_ids.is_empty() {
                let id = conn.default_browser_context_id().to_owned();
                conn.insert_browser_context(conn.new_browser_context(id.clone()));
                default_context_ids.push(id);
            }
            resolved.extend(default_context_ids);
            continue;
        }
        if !conn.has_browser_context_id(browser_context_id) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownBrowserContextId",
            ));
        }
        resolved.push(browser_context_id.to_owned());
    }
    resolved.sort();
    resolved.dedup();
    Ok(resolved)
}

fn top_level_target_routes_for_browser_contexts(
    conn: &CdpConnection,
    browser_context_ids: Option<&[String]>,
) -> Vec<CdpSessionRoute> {
    let mut routes = Vec::new();
    for browser_context in conn.browser_contexts() {
        if let Some(browser_context_ids) = browser_context_ids
            && !browser_context_ids
                .iter()
                .any(|id| id == &browser_context.id)
        {
            continue;
        }
        if browser_context.active_target_id().is_some() {
            routes.push(CdpSessionRoute::ActiveTarget {
                browser_context_id: browser_context.id.clone(),
                target_id: None,
            });
        }
        routes.extend(browser_context.background_targets().map(|target| {
            CdpSessionRoute::PageTargetHost {
                browser_context_id: browser_context.id.clone(),
                target_id: target.target_id().to_owned(),
            }
        }));
    }
    routes
}

fn devtools_emulation_owner_error(error: String) -> DevToolsError {
    if error == "BrowserContextNotLoaded" {
        DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "BrowserContextNotLoaded")
    } else {
        DevToolsError::new(DevToolsErrorKind::Internal, error)
    }
}

async fn execute_devtools_set_viewport_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetViewportCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_viewport_for_browser_contexts(conn, command).await;
    }
    if let Some(target_id) = command.context.target_id.as_ref() {
        let route = if let Some(route) = conn.target_session_route_for_target_id(target_id.as_str())
        {
            route
        } else if conn.has_attached_child_frame_id(target_id.as_str()) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "ChildFrameContextNotSupportedForSetViewport",
            ));
        } else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "NoSuchTarget",
            ));
        };
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        let mut command = command;
        command.context.session_id = None;
        return match start_devtools_set_viewport_command(route_scope.conn_mut(), None, command) {
            Ok(Some(pending)) => {
                let completed = pending.wait().await;
                complete_pending_devtools_emulation_command(route_scope.conn_mut(), completed)
            }
            Ok(None) => Ok(DevToolsCommandResult::Empty),
            Err(error) => Err(error),
        };
    }
    match start_devtools_set_viewport_command(conn, None, command) {
        Ok(Some(pending)) => {
            let completed = pending.wait().await;
            complete_pending_devtools_emulation_command(conn, completed)
        }
        Ok(None) => Ok(DevToolsCommandResult::Empty),
        Err(error) => Err(error),
    }
}

async fn execute_devtools_set_window_state_command_async(
    conn: &mut CdpConnection,
    command: crate::devtools_runtime::DevToolsSetWindowStateCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if let Some(target_id) = command.context.target_id.as_ref() {
        let route = emulation_route_for_target(
            conn,
            target_id,
            "ChildFrameContextNotSupportedForSetWindowState",
        )?;
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        let mut command = command;
        command.context.session_id = None;
        return execute_devtools_set_window_state_for_current_route(
            route_scope.conn_mut(),
            command,
        )
        .await;
    }
    execute_devtools_set_window_state_for_current_route(conn, command).await
}

async fn execute_devtools_set_window_state_for_current_route(
    conn: &mut CdpConnection,
    command: crate::devtools_runtime::DevToolsSetWindowStateCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let state = target_window_surface_state_from_devtools(command.state);
    if conn
        .with_target_owner_state_for_session_mut(session_id, |owner_state| {
            owner_state.set_window_surface_state(state);
        })
        .is_none()
    {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    }
    let pending = start_session_surface_override_page_command(conn, session_id)
        .map_err(devtools_emulation_owner_error)?;
    complete_emulation_page_updates(conn, devtools_command_session_id(&command.context), pending)
        .await
}

async fn execute_devtools_set_client_window_state_command_async(
    conn: &mut CdpConnection,
    command: DevToolsSetClientWindowStateCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let route = conn
        .target_session_route_for_target_id(command.client_window.as_str())
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    let mut route_scope = conn.scoped_none_session_owner_route_override(route);
    let conn = route_scope.conn_mut();

    let mut window_state_context = command.context.clone();
    window_state_context.session_id = None;
    window_state_context.target_id = Some(command.client_window.clone());
    let result = execute_devtools_set_window_state_for_current_route(
        conn,
        crate::devtools_runtime::DevToolsSetWindowStateCommand {
            context: window_state_context,
            state: command.state,
        },
    )
    .await;

    match result {
        Ok(_) => {
            let _ = conn.with_target_owner_state_for_session_mut(None, |owner_state| {
                owner_state.set_window_surface_geometry(
                    command.width,
                    command.height,
                    command.x,
                    command.y,
                );
            });
            super::target::devtools_client_window_info_for_target(conn, &command.client_window)
                .map(|client_window| {
                    DevToolsCommandResult::ClientWindow(DevToolsSetClientWindowStateResult {
                        client_window,
                    })
                })
                .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))
        }
        Err(error) => Err(error),
    }
}

fn target_window_surface_state_from_devtools(
    state: DevToolsWindowState,
) -> TargetWindowSurfaceState {
    match state {
        DevToolsWindowState::Normal => TargetWindowSurfaceState::Normal,
        DevToolsWindowState::Maximized => TargetWindowSurfaceState::Maximized,
        DevToolsWindowState::Minimized => TargetWindowSurfaceState::Minimized,
        DevToolsWindowState::Fullscreen => TargetWindowSurfaceState::Fullscreen,
    }
}

async fn execute_devtools_set_viewport_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetViewportCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_set_viewport_browser_context_ids(conn, &command)?;
    let mut pending = Vec::new();
    for browser_context_id in browser_context_ids {
        let current_default = conn
            .browser_context_by_id(&browser_context_id)
            .and_then(|context| context.default_emulated_device_metrics.as_ref());
        let metrics = set_viewport_metrics_from_current(current_default, &command)?;
        let browser_context = conn
            .browser_context_by_id(&browser_context_id)
            .expect("resolved browser context must remain addressable");
        let runtime_command_count =
            browser_context_default_device_metrics_runtime_command_count(browser_context);
        let mut runtime_call_ids = (0..runtime_command_count)
            .map(|_| conn.next_internal_runtime_command_id())
            .collect::<Vec<_>>();
        let browser_context = conn
            .browser_context_by_id_mut(&browser_context_id)
            .expect("resolved browser context must remain addressable");
        let had_existing_default = browser_context.default_emulated_device_metrics.is_some();
        browser_context.default_emulated_device_metrics = Some(metrics.clone());
        pending.extend(start_browser_context_default_device_metrics_page_commands(
            browser_context,
            &metrics,
            had_existing_default,
            &mut runtime_call_ids,
        )?);
    }
    if pending.is_empty() {
        return Ok(DevToolsCommandResult::Empty);
    }
    complete_pending_devtools_emulation_command(
        conn,
        PendingEmulationCommandDispatch {
            command_id: None,
            session_id: command
                .context
                .session_id
                .as_ref()
                .map(|session_id| session_id.as_str().to_owned()),
            pending: PendingEmulationRendererDispatch::Pages(pending),
        }
        .wait()
        .await,
    )
}

fn resolve_set_viewport_browser_context_ids(
    conn: &mut CdpConnection,
    command: &DevToolsSetViewportCommand,
) -> Result<Vec<String>, DevToolsError> {
    let mut resolved = Vec::new();
    for browser_context_id in &command.browser_context_ids {
        let browser_context_id = browser_context_id.as_str();
        if command.context.protocol == crate::devtools_runtime::DevToolsProtocol::WebDriverBidi
            && browser_context_id == "default"
        {
            let mut default_context_ids = conn
                .browser_contexts()
                .filter(|context| is_moli_internal_default_user_context(&context.id))
                .map(|context| context.id.clone())
                .collect::<Vec<_>>();
            if default_context_ids.is_empty() {
                let id = conn.default_browser_context_id().to_owned();
                conn.insert_browser_context(conn.new_browser_context(id.clone()));
                default_context_ids.push(id);
            }
            resolved.extend(default_context_ids);
            continue;
        }
        if !conn.has_browser_context_id(browser_context_id) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownBrowserContextId",
            ));
        }
        resolved.push(browser_context_id.to_owned());
    }
    resolved.sort();
    resolved.dedup();
    Ok(resolved)
}

fn is_moli_internal_default_user_context(browser_context_id: &str) -> bool {
    browser_context_id == "BID-default"
        || browser_context_id
            .strip_prefix("BID-")
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn browser_context_default_device_metrics_runtime_command_count(
    browser_context: &BrowserContext,
) -> usize {
    let active_count = usize::from(browser_context.page_targets.active().is_some_and(|target| {
        target.emulated_device_metrics.is_none() && target.runtime_slot.loaded_page().is_some()
    }));
    let background_count = browser_context
        .background_targets()
        .filter(|target| {
            let target_id = target.target_id();
            browser_context
                .parked_page_session_state(target_id)
                .is_none_or(|state| state.emulated_device_metrics.is_none())
                && target.loaded_page().is_some()
        })
        .count();
    active_count + background_count
}

fn start_browser_context_default_device_metrics_page_commands(
    browser_context: &mut BrowserContext,
    metrics: &EmulatedDeviceMetrics,
    had_existing_default: bool,
    runtime_call_ids: &mut Vec<u64>,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let browser_context_id = browser_context.id.clone();
    let mut pending = Vec::new();
    let viewport_surface = Some(metrics.viewport_surface().to_page_viewport_surface());
    if let Some(active_target) = browser_context.page_targets.active_mut()
        && active_target.emulated_device_metrics.is_none()
        && let Some(page) = active_target.runtime_slot.loaded_page_mut()
    {
        pending.push(PendingEmulationPageCommand {
            target: PendingEmulationPageTarget::BrowserContextActive {
                browser_context_id: browser_context_id.clone(),
            },
            operation: PendingEmulationPageOperation::SetViewportSurface,
            pending: page
                .start_set_viewport_surface(viewport_surface)
                .map_err(|error| {
                    DevToolsError::new(DevToolsErrorKind::Internal, error.to_string())
                })?,
            runtime_response_rx: None,
        });
        let (pending_runtime, runtime_response_rx) = start_runtime_emulation_protocol_message(
            page,
            runtime_call_ids.pop().ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::Internal, "MissingRuntimeInspectorCallId")
            })?,
            device::live_device_metrics_override_script(metrics, !had_existing_default),
        )
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
        pending.push(PendingEmulationPageCommand {
            target: PendingEmulationPageTarget::BrowserContextActive {
                browser_context_id: browser_context_id.clone(),
            },
            operation: PendingEmulationPageOperation::RuntimeProtocolMessage,
            pending: pending_runtime,
            runtime_response_rx,
        });
    }
    for index in 0..browser_context.background_target_count() {
        let target_id = browser_context
            .background_target_at(index)
            .expect("background target index must remain valid")
            .target_id()
            .to_owned();
        let has_target_override = browser_context
            .parked_page_session_state(&target_id)
            .is_some_and(|state| state.emulated_device_metrics.is_some());
        if has_target_override {
            continue;
        }
        let Some(page) = browser_context
            .background_target_at_mut(index)
            .and_then(PageTargetHost::loaded_page_mut)
        else {
            continue;
        };
        pending.push(PendingEmulationPageCommand {
            target: PendingEmulationPageTarget::BrowserContextBackground {
                browser_context_id: browser_context_id.clone(),
                target_id: target_id.clone(),
            },
            operation: PendingEmulationPageOperation::SetViewportSurface,
            pending: page
                .start_set_viewport_surface(viewport_surface)
                .map_err(|error| {
                    DevToolsError::new(DevToolsErrorKind::Internal, error.to_string())
                })?,
            runtime_response_rx: None,
        });
        let (pending_runtime, runtime_response_rx) = start_runtime_emulation_protocol_message(
            page,
            runtime_call_ids.pop().ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::Internal, "MissingRuntimeInspectorCallId")
            })?,
            device::live_device_metrics_override_script(metrics, !had_existing_default),
        )
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
        pending.push(PendingEmulationPageCommand {
            target: PendingEmulationPageTarget::BrowserContextBackground {
                browser_context_id: browser_context_id.clone(),
                target_id,
            },
            operation: PendingEmulationPageOperation::RuntimeProtocolMessage,
            pending: pending_runtime,
            runtime_response_rx,
        });
    }
    Ok(pending)
}

fn complete_pending_devtools_emulation_command(
    conn: &mut CdpConnection,
    completed: CompletedEmulationCommandDispatch,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let CompletedEmulationRendererDispatch::Pages(completed_pages) = completed.completed else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "DevTools emulation command completed through the CDP-only IO receiver",
        ));
    };
    for completed_page in completed_pages {
        let completion = completed_page
            .completed
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
        finish_pending_emulation_page_command(
            conn,
            completed_page.operation,
            completed_page.target,
            completion,
        )
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
    }
    Ok(DevToolsCommandResult::Empty)
}

fn start_runtime_emulation_protocol_message(
    page: &moli_core::page::Page,
    command_id: u64,
    expression: String,
) -> Result<
    (
        PendingPageCommand,
        Option<RuntimeInspectorAsyncCompletionReceiver>,
    ),
    String,
> {
    let raw_json = runtime_evaluate_json(command_id, expression);
    let call_id = i32::try_from(command_id)
        .map_err(|_| format!("runtime inspector command id {command_id} does not fit i32"))?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let attachment_id = page
        .renderer_agent_attachment_id()
        .ok_or_else(|| "renderer page has no DevTools attachment".to_owned())?;
    page.start_runtime_protocol_message_with_deferred_response(
        raw_json,
        RendererRuntimeInspectorResponseSender::new(call_id, tx)
            .with_renderer_agent_attachment(attachment_id),
    )
    .map(|pending| (pending, Some(rx)))
    .map_err(|error| error.to_string())
}

fn runtime_evaluate_json(command_id: u64, expression: String) -> String {
    json!({
        "id": command_id,
        "method": "Runtime.evaluate",
        "params": { "expression": expression }
    })
    .to_string()
}

pub(crate) fn complete_pending_emulation_command(
    conn: &mut CdpConnection,
    completed: CompletedEmulationCommandDispatch,
) -> CommandOutputPlan {
    let session_id = completed.session_id.clone();
    let completed_pages = match completed.completed {
        CompletedEmulationRendererDispatch::Pages(completed_pages) => completed_pages,
        CompletedEmulationRendererDispatch::IoCommandReply(completed) => {
            return match completed {
                Ok(CompletedDevToolsIoCommandDispatch::Dispatched) => {
                    CommandOutputPlan::result(json!({}))
                }
                Ok(CompletedDevToolsIoCommandDispatch::SessionResponse { .. }) => {
                    CommandOutputPlan::error(
                        -32000,
                        "command-reply Emulation dispatch used session output",
                    )
                }
                Err(error) => CommandOutputPlan::error(-32000, error),
            };
        }
        CompletedEmulationRendererDispatch::IoSessionOutput {
            completed: Ok(CompletedDevToolsIoCommandDispatch::SessionResponse { predecessor, .. }),
            ..
        } => {
            let mut plan = CommandOutputPlan::default();
            plan.set_renderer_output_predecessor(predecessor);
            return plan;
        }
        CompletedEmulationRendererDispatch::IoSessionOutput {
            completed,
            correlation,
        } => {
            if !conn.take_renderer_call_if_correlation_matches_for_session_owner(
                session_id.as_deref(),
                correlation,
            ) {
                return CommandOutputPlan::default();
            }
            return match completed {
                Ok(CompletedDevToolsIoCommandDispatch::Dispatched) => CommandOutputPlan::error(
                    -32000,
                    "Emulation IO dispatch completed without publishing its session response",
                ),
                Ok(CompletedDevToolsIoCommandDispatch::SessionResponse { .. }) => unreachable!(),
                Err(error) => CommandOutputPlan::error(-32000, error),
            };
        }
    };
    for completed_page in completed_pages {
        let completion = match completed_page.completed {
            Ok(completion) => completion,
            Err(error) => return CommandOutputPlan::error(-32000, error),
        };
        let result = finish_pending_emulation_page_command(
            conn,
            completed_page.operation,
            completed_page.target,
            completion,
        );
        if let Err(error) = result {
            return CommandOutputPlan::error(-32000, error);
        }
    }
    CommandOutputPlan::result(json!({}))
}

fn loaded_page_mut_for_session<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
) -> Option<&'a mut moli_core::page::Page> {
    conn.loaded_page_mut_for_protocol_access(session_id).ok()
}

pub(crate) async fn clear_emulated_media_for_detached_session_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> Result<(), String> {
    // Chromium's InspectorEmulationAgent::disable clears media overrides even
    // though the resulting settings are observable by every session on the
    // target. Match that contract when detach preserves the loaded page.
    let overrides = crate::conn::EmulatedMediaOverrides::default();
    let mut changed = false;
    if !conn.mutate_emulation_session_state_for_session_owner(Some(session_id), |state| {
        if let Some(state) = state {
            changed = *state.emulated_media != overrides;
            *state.emulated_media = overrides.clone();
        }
    }) {
        return Ok(());
    }
    if !changed {
        return Ok(());
    }

    let page_overrides: moli_core::page::EmulatedMediaOverrides = (&overrides).into();
    let Some(page) = loaded_page_mut_for_session(conn, Some(session_id)) else {
        return Ok(());
    };
    page.set_emulated_media_async(&page_overrides)
        .await
        .map_err(|error| format!("failed to clear detached session emulated media: {error}"))
}

fn single_pending_emulation_dispatch(
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    operation: PendingEmulationPageOperation,
    pending: PendingPageCommand,
    runtime_response_rx: Option<RuntimeInspectorAsyncCompletionReceiver>,
) -> PendingEmulationCommandDispatch {
    let session_id = owner_scope.session_id().map(str::to_owned);
    PendingEmulationCommandDispatch {
        command_id,
        session_id: session_id.clone(),
        pending: PendingEmulationRendererDispatch::Pages(vec![PendingEmulationPageCommand {
            target: PendingEmulationPageTarget::SessionOwner { owner_scope },
            operation,
            pending,
            runtime_response_rx,
        }]),
    }
}

fn emulation_command_is_context_wide(conn: &CdpConnection, session_id: Option<&str>) -> bool {
    match session_id {
        None => conn.none_session_owner_route_override().is_none(),
        Some(session_id) => matches!(
            conn.session_route(Some(session_id)),
            Some(CdpSessionRoute::Browser)
        ),
    }
}

fn start_context_emulated_media_page_commands(
    conn: &mut CdpConnection,
    overrides: &moli_core::page::EmulatedMediaOverrides,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let Some(browser_context) = conn.browser_context.as_mut() else {
        return Ok(Vec::new());
    };
    let browser_context_id = browser_context.id.clone();
    let mut pending = Vec::new();
    if let Some(page) = browser_context.active_target.runtime_slot.loaded_page_mut() {
        pending.push(PendingEmulationPageCommand {
            target: PendingEmulationPageTarget::BrowserContextActive {
                browser_context_id: browser_context_id.clone(),
            },
            operation: PendingEmulationPageOperation::SetEmulatedMedia,
            pending: page
                .start_set_emulated_media(overrides)
                .map_err(|error| error.to_string())?,
            runtime_response_rx: None,
        });
    }
    for target in browser_context.background_targets_mut() {
        let target_id = target.target_id().to_owned();
        let Some(page) = target.loaded_page_mut() else {
            continue;
        };
        pending.push(PendingEmulationPageCommand {
            target: PendingEmulationPageTarget::BrowserContextBackground {
                browser_context_id: browser_context_id.clone(),
                target_id,
            },
            operation: PendingEmulationPageOperation::SetEmulatedMedia,
            pending: page
                .start_set_emulated_media(overrides)
                .map_err(|error| error.to_string())?,
            runtime_response_rx: None,
        });
    }
    Ok(pending)
}

fn start_session_locale_override_page_commands(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let Some(locale_override) = locale_override_for_session(conn, session_id) else {
        return Ok(Vec::new());
    };
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let Some(page) = loaded_page_mut_for_session(conn, session_id) else {
        return Ok(Vec::new());
    };
    start_locale_override_page_command(
        PendingEmulationPageTarget::SessionOwner { owner_scope },
        page,
        locale_override.as_deref(),
    )
}

fn start_context_locale_override_page_commands(
    conn: &mut CdpConnection,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let mut pending = Vec::new();
    for browser_context in conn
        .browser_context
        .iter_mut()
        .chain(conn.inactive_browser_contexts.iter_mut())
    {
        let browser_context_id = browser_context.id.clone();
        let active_locale = browser_context.effective_active_locale_override_owned();
        if let Some(page) = browser_context.active_target.runtime_slot.loaded_page_mut() {
            pending.extend(start_locale_override_page_command(
                PendingEmulationPageTarget::BrowserContextActive {
                    browser_context_id: browser_context_id.clone(),
                },
                page,
                active_locale.as_deref(),
            )?);
        }
        for index in 0..browser_context.background_target_count() {
            let target_id = browser_context
                .background_target_at(index)
                .expect("background target index must remain valid")
                .target_id()
                .to_owned();
            let Some(page) = browser_context
                .background_target_at_mut(index)
                .and_then(PageTargetHost::loaded_page_mut)
            else {
                continue;
            };
            pending.extend(start_locale_override_page_command(
                PendingEmulationPageTarget::BrowserContextBackground {
                    browser_context_id: browser_context_id.clone(),
                    target_id,
                },
                page,
                active_locale.as_deref(),
            )?);
        }
    }
    Ok(pending)
}

fn start_geolocation_surface_override_page_commands(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    if cmd.session_id.is_some() {
        return start_session_surface_override_page_command(conn, cmd.session_id);
    }
    let runtime_call_id = conn.next_internal_runtime_command_id();
    let Some(browser_context) = conn.browser_context.as_mut() else {
        return Ok(Vec::new());
    };
    let Some(script) = browser_context.generated_surface_override_script_for_active_target() else {
        return Ok(Vec::new());
    };
    let browser_context_id = browser_context.id.clone();
    let Some(page) = browser_context.active_target.runtime_slot.loaded_page_mut() else {
        return Ok(Vec::new());
    };
    start_surface_override_page_command(
        PendingEmulationPageTarget::BrowserContextActive { browser_context_id },
        page,
        script,
        runtime_call_id,
    )
    .map(|pending| vec![pending])
}

fn start_session_surface_override_page_command(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let script = {
        let Some((browser_context_id, target_id)) =
            conn.target_owner_identity_for_session(session_id)
        else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        if let Some(target_id) = target_id.as_deref()
            && browser_context.background_target(target_id).is_some()
        {
            browser_context.generated_surface_override_script_for_parked_target(target_id)
        } else {
            browser_context.generated_surface_override_script_for_active_target()
        }
    };
    let Some(script) = script else {
        return Ok(Vec::new());
    };
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let runtime_call_id = conn.next_internal_runtime_command_id();
    let Some(page) = loaded_page_mut_for_session(conn, session_id) else {
        return Ok(Vec::new());
    };
    start_surface_override_page_command(
        PendingEmulationPageTarget::SessionOwner { owner_scope },
        page,
        script,
        runtime_call_id,
    )
    .map(|pending| vec![pending])
}

fn start_surface_override_for_route(
    conn: &mut CdpConnection,
    target: PendingEmulationPageTarget,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let script = match &target {
        PendingEmulationPageTarget::BrowserContextActive { browser_context_id } => {
            let Some(browser_context) = conn.browser_context_by_id(browser_context_id) else {
                return Err("BrowserContextNotLoaded".to_owned());
            };
            browser_context.generated_surface_override_script_for_active_target()
        }
        PendingEmulationPageTarget::BrowserContextBackground {
            browser_context_id,
            target_id,
        } => {
            let Some(browser_context) = conn.browser_context_by_id(browser_context_id) else {
                return Err("BrowserContextNotLoaded".to_owned());
            };
            browser_context.generated_surface_override_script_for_parked_target(target_id)
        }
        PendingEmulationPageTarget::SessionOwner { owner_scope } => {
            return start_session_surface_override_page_command(conn, owner_scope.session_id());
        }
    };
    let Some(script) = script else {
        return Ok(Vec::new());
    };
    let runtime_call_id = conn.next_internal_runtime_command_id();
    let Some(page) = loaded_page_mut_for_session(conn, None) else {
        return Ok(Vec::new());
    };
    start_surface_override_page_command(target, page, script, runtime_call_id)
        .map(|pending| vec![pending])
}

fn start_surface_override_page_command(
    target: PendingEmulationPageTarget,
    page: &moli_core::page::Page,
    script: crate::conn::DocumentStartScript,
    runtime_call_id: u64,
) -> Result<PendingEmulationPageCommand, String> {
    let (pending, runtime_response_rx) =
        start_runtime_emulation_protocol_message(page, runtime_call_id, script.source)?;
    Ok(PendingEmulationPageCommand {
        target,
        operation: PendingEmulationPageOperation::RuntimeProtocolMessage,
        pending,
        runtime_response_rx,
    })
}

fn start_locale_override_page_command(
    target: PendingEmulationPageTarget,
    page: &moli_core::page::Page,
    locale_override: Option<&str>,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let locale_update = page
        .start_set_locale_override(locale_override)
        .map_err(|error| format!("failed to update page locale override: {error}"))?;
    Ok(vec![PendingEmulationPageCommand {
        target,
        operation: PendingEmulationPageOperation::SetLocaleOverride,
        pending: locale_update,
        runtime_response_rx: None,
    }])
}

fn locale_override_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> Option<Option<String>> {
    let (browser_context_id, target_id) = conn.target_owner_identity_for_session(session_id)?;
    let browser_context = conn.browser_context_by_id(&browser_context_id)?;
    if let Some(target_id) = target_id
        && browser_context.background_target(&target_id).is_some()
    {
        return Some(browser_context.effective_parked_locale_override_owned(&target_id));
    }
    Some(browser_context.effective_active_locale_override_owned())
}

fn finish_pending_emulation_page_command(
    conn: &mut CdpConnection,
    operation: PendingEmulationPageOperation,
    target: PendingEmulationPageTarget,
    completion: CompletedPageCommand,
) -> Result<(), String> {
    match target {
        PendingEmulationPageTarget::SessionOwner { owner_scope } => {
            let mut route_scope = owner_scope.enter(conn);
            if matches!(operation, PendingEmulationPageOperation::SetUserAgentLoader) {
                return route_scope
                    .conn_mut()
                    .finish_rebuild_resource_runtime_for_session_owner(
                        owner_scope.session_id(),
                        completion,
                    );
            }
            let page = route_scope
                .conn_mut()
                .loaded_page_mut_for_interruptible_protocol_access(owner_scope.session_id())
                .ok();
            finish_emulation_page_operation_on_current_attachment(page, operation, completion)
        }
        PendingEmulationPageTarget::BrowserContextActive { browser_context_id } => {
            let page = conn
                .browser_context_by_id_mut(&browser_context_id)
                .and_then(|browser_context| {
                    browser_context.active_target.runtime_slot.loaded_page_mut()
                });
            finish_emulation_page_operation_on_current_attachment(page, operation, completion)
        }
        PendingEmulationPageTarget::BrowserContextBackground {
            browser_context_id,
            target_id,
        } => {
            let page = conn
                .browser_context_by_id_mut(&browser_context_id)
                .and_then(|browser_context| browser_context.background_target_mut(&target_id))
                .and_then(|target| target.loaded_page_mut());
            finish_emulation_page_operation_on_current_attachment(page, operation, completion)
        }
    }
}

fn finish_emulation_page_operation_on_current_attachment(
    page: Option<&mut moli_core::page::Page>,
    operation: PendingEmulationPageOperation,
    completion: CompletedPageCommand,
) -> Result<(), String> {
    let completion_attachment = completion.renderer_agent_attachment_id();
    if let Some(page) = page
        && page.renderer_agent_attachment_id() == completion_attachment
    {
        return finish_emulation_page_operation(page, operation, completion);
    }

    // The renderer command has already settled successfully. A cross-Document
    // navigation may replace its Page before the protocol actor decodes that
    // frozen completion; do not turn that success into NoDocumentLoaded or
    // apply the old PageState snapshot to the replacement attachment. The
    // authoritative emulation state was stored before dispatch and is replayed
    // while the replacement Page is installed.
    let output = match operation {
        PendingEmulationPageOperation::RuntimeProtocolMessage => {
            completion.into_runtime_protocol_message_command_turn()
        }
        _ => completion.into_unit_page_command_turn(),
    };
    output
        .map(drop)
        .map_err(|error| format!("stale Emulation command returned an unexpected reply: {error}"))
}

fn finish_emulation_page_operation(
    page: &mut moli_core::page::Page,
    operation: PendingEmulationPageOperation,
    completion: CompletedPageCommand,
) -> Result<(), String> {
    match operation {
        PendingEmulationPageOperation::SetExtraHttpHeaders => page
            .finish_set_extra_http_headers(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetLocaleOverride => page
            .finish_set_locale_override(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetNetworkConditions => page
            .finish_set_network_offline(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetCpuThrottlingRate => page
            .finish_set_cpu_throttling_rate(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetIdleOverride => page
            .finish_set_idle_override(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetTimezoneOverride => page
            .finish_set_timezone_override(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetEmulatedMedia => page
            .finish_set_emulated_media(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetViewportSurface => page
            .finish_set_viewport_surface(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::ReplaceBrowserResourceRuntime => page
            .finish_replace_browser_resource_runtime(completion)
            .map_err(|error| error.to_string()),
        PendingEmulationPageOperation::SetUserAgentLoader => {
            unreachable!("user agent loader rebuild finishes through the session owner")
        }
        PendingEmulationPageOperation::RuntimeProtocolMessage => page
            .finish_runtime_protocol_message(completion)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    }
}
