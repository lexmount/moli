use crate::conn::{
    BrowserContext, CdpConnection, CdpSessionRoute, Cmd, CommandOwnerScope, EmulatedDeviceMetrics,
    EmulatedGeolocationOverrideState, EmulatedViewportSurface, EmulationPolicyChange,
    PagePolicyUpdateKind, RendererCommandCorrelation, RendererCommandDescriptor,
    WindowSurfaceState,
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
use moli_core::page::{
    CompletedDevToolsIoCommandDispatch, CompletedPageCommand, PendingDevToolsIoCommandDispatch,
    PendingPageCommand,
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
    IoAdapterReply(PendingDevToolsIoCommandDispatch),
    IoSessionOutput {
        pending: PendingDevToolsIoCommandDispatch,
        correlation: RendererCommandCorrelation,
    },
}

enum CompletedEmulationRendererDispatch {
    Pages(Vec<CompletedEmulationPageCommand>),
    IoAdapterReply(Result<CompletedDevToolsIoCommandDispatch, String>),
    IoSessionOutput {
        completed: Result<CompletedDevToolsIoCommandDispatch, String>,
        correlation: RendererCommandCorrelation,
    },
}

struct PendingEmulationPageCommand {
    target: PendingEmulationPageTarget,
    operation: PendingEmulationPageOperation,
    source: EmulationPageCommandSource,
    pending: PendingPageCommand,
}

struct CompletedEmulationPageCommand {
    target: PendingEmulationPageTarget,
    operation: PendingEmulationPageOperation,
    source: EmulationPageCommandSource,
    completed: Result<CompletedPageCommand, String>,
}

#[derive(Clone, Copy)]
enum EmulationPageCommandSource {
    Browser(Option<moli_core::browser::RendererPageResidenceIdentity>),
}

impl EmulationPageCommandSource {
    fn browser(context: &BrowserContext, target_id: &str) -> Self {
        Self::Browser(context.target_renderer_page_residence_identity(target_id))
    }

    fn browser_for_owner(conn: &CdpConnection, owner: &CommandOwnerScope) -> Self {
        Self::Browser(conn.resolved_page_owner_identity_for_owner(owner).and_then(
            |(context_id, target_id)| {
                conn.browser_context_by_id(&context_id)?
                    .target_renderer_page_residence_identity(&target_id)
            },
        ))
    }
}

#[derive(Clone)]
enum PendingEmulationPageTarget {
    SessionOwner {
        owner_scope: CommandOwnerScope,
    },
    BrowserContextTarget {
        browser_context_id: String,
        target_id: String,
    },
}

pub(crate) enum EmulationCommandTaskStep {
    Pending(PendingEmulationCommandDispatch),
    Complete(CommandOutputPlan),
}

enum PendingEmulationPageOperation {
    Policy(PagePolicyUpdateKind),
    RebuildResourceRuntime,
}

impl PendingEmulationPageOperation {
    fn has_authoritative_replay_state(&self) -> bool {
        // Idle state belongs to a Document; other settings have authoritative replay policy.
        !matches!(self, Self::Policy(PagePolicyUpdateKind::SetIdleOverride))
    }
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
                        source,
                        pending,
                    } = pending;
                    let completed_page = pending.wait().await.map_err(|error| error.to_string());
                    completed.push(CompletedEmulationPageCommand {
                        target,
                        operation,
                        source,
                        completed: completed_page,
                    });
                }
                CompletedEmulationRendererDispatch::Pages(completed)
            }
            PendingEmulationRendererDispatch::IoAdapterReply(pending) => {
                CompletedEmulationRendererDispatch::IoAdapterReply(
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
            Some(start_focus_emulation_enabled_command(conn, cmd))
        }
        Some(EmulationAction::SetDeviceMetricsOverride) => {
            Some(start_device_metrics_override_command(conn, cmd))
        }
        Some(EmulationAction::ClearDeviceMetricsOverride) => {
            Some(start_clear_device_metrics_override_command(conn, cmd))
        }
        Some(EmulationAction::SetCpuThrottlingRate) => Some(EmulationCommandTaskStep::Complete(
            cpu_throttling_rate_command_output_plan(cmd),
        )),
        Some(EmulationAction::SetAutomationOverride) => {
            Some(start_automation_override_command(conn, cmd))
        }
        Some(EmulationAction::SetDataSaverOverride) => {
            Some(start_data_saver_override_command(conn, cmd))
        }
        Some(EmulationAction::SetHardwareConcurrencyOverride) => {
            Some(start_hardware_concurrency_override_command(conn, cmd))
        }
        Some(EmulationAction::SetTouchEmulationEnabled) => {
            Some(start_touch_emulation_enabled_command(conn, cmd))
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
        Some(EmulationAction::SetDefaultBackgroundColorOverride) => Some(
            EmulationCommandTaskStep::Complete(default_background_color_command(conn, cmd)),
        ),
        None => Some(EmulationCommandTaskStep::Complete(
            CommandOutputPlan::error(-32601, "UnknownMethod"),
        )),
    }
}

fn start_focus_emulation_enabled_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetFocusEmulationEnabledParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::success());
    }
    if let Err(message) = page_session::update_page_emulation_state(
        conn,
        cmd.session_id,
        EmulationPolicyChange::FocusEnabled(params.enabled),
    ) {
        let code = if message == "BrowserContextNotLoaded" {
            -31998
        } else {
            -32000
        };
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(code, message));
    }
    let pending = match start_surface_override_page_commands(conn, cmd) {
        Ok(pending) => pending,
        Err(error) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error));
        }
    };
    if pending.is_empty() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::success());
    }
    EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
        command_id: cmd.id,
        session_id: cmd.session_id.map(str::to_owned),
        pending: PendingEmulationRendererDispatch::Pages(pending),
    })
}

fn start_touch_emulation_enabled_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let params: params::SetTouchEmulationEnabledParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let max_touch_points = params.max_touch_points.unwrap_or(1);
    if !(1..=16).contains(&max_touch_points) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "Touch points must be between 1 and 16",
        ));
    }
    if conn.browser_context.is_none() {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    }
    if let Err(message) = page_session::update_page_emulation_state(
        conn,
        cmd.session_id,
        EmulationPolicyChange::MaxTouchPoints(if params.enabled {
            max_touch_points as u32
        } else {
            0
        }),
    ) {
        let code = if message == "BrowserContextNotLoaded" {
            -31998
        } else {
            -32000
        };
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(code, message));
    }
    start_navigator_override_page_command(conn, cmd)
}

fn start_hardware_concurrency_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let value = match cmd.get_params::<params::SetHardwareConcurrencyOverrideParams>() {
        Ok(Some(params)) => params.hardware_concurrency,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let Some(value) = u32::try_from(value)
        .ok()
        .filter(|value| *value <= i32::MAX as u32)
        .and_then(std::num::NonZeroU32::new)
    else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "HardwareConcurrency must be a positive int32",
        ));
    };
    if let Err(message) =
        conn.update_navigator_emulation_for_session_owner(cmd.session_id, |sessions, key| {
            sessions.session_mut(key).hardware_concurrency = Some(value);
        })
    {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-31998, message));
    }
    start_navigator_override_page_command(conn, cmd)
}

fn start_data_saver_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let value = match cmd.get_params::<params::SetDataSaverOverrideParams>() {
        Ok(params) => params.and_then(|params| params.data_saver_enabled),
        Err(_) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if let Err(message) =
        conn.update_navigator_emulation_for_session_owner(cmd.session_id, |sessions, key| {
            sessions.session_mut(key).data_saver = value;
        })
    {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-31998, message));
    }
    start_navigator_override_page_command(conn, cmd)
}

fn start_automation_override_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let enabled = match cmd.get_params::<params::SetAutomationOverrideParams>() {
        Ok(Some(params)) => params.enabled,
        _ => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    if let Err(message) = conn
        .update_navigator_emulation_for_session_owner(cmd.session_id, |sessions, key| {
            sessions.set_automation(key, enabled)
        })
    {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(-31998, message));
    }
    start_navigator_override_page_command(conn, cmd)
}

fn start_navigator_override_page_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> EmulationCommandTaskStep {
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let overrides = conn
        .navigation_load_inputs_for_owner(&owner_scope)
        .navigator_overrides;
    let Some((context_id, target_id)) = page_configuration_owner(conn, &owner_scope) else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };
    let context = conn
        .browser_context_by_id(&context_id)
        .expect("resolved context remains registered");
    let pending = match context.start_set_navigator_overrides_for_target(&target_id, &overrides) {
        Ok(pending) => pending,
        Err(error) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                error.to_string(),
            ));
        }
    };
    EmulationCommandTaskStep::Pending(single_pending_emulation_dispatch(
        cmd.id,
        owner_scope,
        PendingEmulationPageOperation::Policy(PagePolicyUpdateKind::SetNavigatorOverrides),
        pending,
        EmulationPageCommandSource::browser(context, &target_id),
    ))
}

fn cpu_throttling_rate_command_output_plan(cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params = match cmd.get_params::<params::SetCpuThrottlingRateParams>() {
        Ok(Some(params)) if params.rate.is_finite() => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    if params.rate > 1.0 {
        return CommandOutputPlan::error(-32000, "CPU throttling is not supported");
    }
    CommandOutputPlan::success()
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
    match page_session::update_page_emulation_state(
        conn,
        cmd.session_id,
        EmulationPolicyChange::EmitTouchEventsForMouse(params.enabled),
    ) {
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
    if !conn.apply_emulation_override_for_session_owner(
        cmd.session_id,
        EmulationPolicyChange::ScriptExecutionDisabled(params.value),
    ) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(binding) = conn
        .runtime_session_owner_slot_for_owner(&owner)
        .ok()
        .and_then(|slot| slot.current_renderer_inspection_binding())
    else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };
    let attachment_id = binding.attachment().id();
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    let response_delivery = cmd.terminal_response_delivery();
    if cmd.id.is_none()
        || response_delivery == moli_page_types::RendererInspectorResponseDelivery::AdapterReply
    {
        let pending = match binding.start_set_script_execution_disabled(
            renderer_inspector_session_id,
            params.value,
            None,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
        };
        return EmulationCommandTaskStep::Pending(PendingEmulationCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            pending: PendingEmulationRendererDispatch::IoAdapterReply(pending),
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
        "Emulation session output must not allocate an adapter-reply receiver",
    );
    let pending = conn
        .runtime_session_owner_slot_for_owner(&owner)
        .ok()
        .and_then(|slot| slot.current_renderer_inspection_binding())
        .filter(|binding| binding.attachment().id() == attachment_id)
        .ok_or_else(|| "Emulation renderer attachment changed before IO dispatch".to_owned())
        .and_then(|binding| {
            binding
                .start_set_script_execution_disabled(
                    renderer_inspector_session_id,
                    params.value,
                    Some(response),
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
    let locale_override = params.locale.filter(|value| !value.is_empty());
    if let Err(message) =
        conn.set_devtools_locale_override_for_session_owner(cmd.session_id, locale_override)
    {
        let code = if message == "BrowserContextNotLoaded" {
            -31998
        } else {
            -32000
        };
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(code, message));
    }
    // The session owns a process-wide ICU claim. Isolate entry/foreground
    // notifications refresh native V8 caches; navigation must not re-acquire it.
    EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})))
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
    let Some((context_id, target_id)) = page_configuration_owner(conn, &owner_scope) else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };

    let context = conn
        .browser_context_by_id_mut(&context_id)
        .expect("resolved context remains registered");
    match context.start_set_idle_override_for_target(&target_id, idle_override) {
        Ok(pending) => EmulationCommandTaskStep::Pending(single_pending_emulation_dispatch(
            cmd.id,
            owner_scope,
            PendingEmulationPageOperation::Policy(PagePolicyUpdateKind::SetIdleOverride),
            pending,
            EmulationPageCommandSource::browser(context, &target_id),
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
    if let Err(message) =
        conn.set_devtools_timezone_override_for_session_owner(cmd.session_id, timezone_override)
    {
        let code = if message == "BrowserContextNotLoaded" {
            -31998
        } else if message == "Invalid timezone id" {
            -32602
        } else {
            -32000
        };
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(code, message));
    }
    EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})))
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
    if !conn.apply_emulation_override_for_session_owner(
        cmd.session_id,
        EmulationPolicyChange::Geolocation(override_state),
    ) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let pending = match start_surface_override_page_commands(conn, cmd) {
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

fn default_background_color_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: params::SetDefaultBackgroundColorOverrideParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => Default::default(),
        Err(error) => return CommandOutputPlan::error(-32602, error),
    };
    if params.color.as_ref().is_some_and(|color| {
        [color.r, color.g, color.b]
            .into_iter()
            .any(|channel| i32::try_from(channel).is_err())
    }) {
        return CommandOutputPlan::error(-32602, "Color channels must be int32 values");
    }
    let color = params.color.map(|color| {
        // Blink's Color constructor clamps the channels and quantizes alpha.
        [
            color.r.clamp(0, 255) as u8,
            color.g.clamp(0, 255) as u8,
            color.b.clamp(0, 255) as u8,
            (color.a.unwrap_or(1.0).clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    });
    match page_session::update_page_emulation_state(
        conn,
        cmd.session_id,
        EmulationPolicyChange::DefaultBackgroundColor(color),
    ) {
        // Paint is demand-driven; each capture samples the target's current base color.
        Ok(()) => CommandOutputPlan::success(),
        Err(error) => CommandOutputPlan::error(-31998, error),
    }
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
    if !conn.apply_emulation_override_for_session_owner(
        cmd.session_id,
        EmulationPolicyChange::Media(overrides.clone()),
    ) {
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
        let Some((context_id, target_id)) = page_configuration_owner(conn, &owner_scope) else {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
        };

        let context = conn
            .browser_context_by_id(&context_id)
            .expect("resolved context remains registered");
        match context.start_set_emulated_media_for_target(&target_id, &page_overrides) {
            Ok(pending) => vec![PendingEmulationPageCommand {
                source: EmulationPageCommandSource::browser(context, &target_id),
                target: PendingEmulationPageTarget::SessionOwner { owner_scope },
                operation: PendingEmulationPageOperation::Policy(
                    PagePolicyUpdateKind::SetEmulatedMedia,
                ),
                pending,
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
                source: EmulationPageCommandSource::browser_for_owner(conn, &owner_scope),
                target: PendingEmulationPageTarget::SessionOwner { owner_scope },
                operation: PendingEmulationPageOperation::RebuildResourceRuntime,
                pending,
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
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let previous = conn.target_session_owner_emulated_device_metrics_for_owner(&owner);
    let mut base = conn
        .target_owner_identity_for_owner(&owner)
        .and_then(|(context_id, _)| conn.browser_context_by_id(&context_id))
        .and_then(|context| context.emulation_defaults().device_metrics.as_ref())
        .map(EmulatedDeviceMetrics::viewport_surface)
        .unwrap_or_default();
    if let Some(geometry) =
        conn.target_owner_identity_for_owner(&owner)
            .and_then(|(context_id, target_id)| {
                conn.browser_context_by_id(&context_id)?
                    .target_window_surface(target_id.as_deref()?)
            })
    {
        if geometry.width != 0 {
            base.inner_width = geometry.width;
            base.outer_width = geometry.width;
        }
        if geometry.height != 0 {
            base.inner_height = geometry.height;
            base.outer_height = geometry.height;
        }
    }
    let metrics = match device::metrics_from_cdp(params, previous.as_ref(), &base) {
        Ok(metrics) => metrics,
        Err(error) => {
            return EmulationCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
                error,
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
    match start_apply_device_metrics(conn, cmd.id, metrics, owner) {
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
    if !conn.apply_emulation_override_for_session_owner(
        cmd.session_id,
        EmulationPolicyChange::DeviceMetrics(None),
    ) {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let viewport_surface = conn
        .navigation_load_inputs_for_owner(&owner_scope)
        .viewport_surface;
    let Some((context_id, target_id)) = page_configuration_owner(conn, &owner_scope) else {
        return EmulationCommandTaskStep::Complete(CommandOutputPlan::result(json!({})));
    };

    let context = conn
        .browser_context_by_id(&context_id)
        .expect("resolved context remains registered");
    let pending_viewport =
        match context.start_set_viewport_surface_for_target(&target_id, viewport_surface) {
            Ok(pending) => pending,
            Err(error) => {
                return EmulationCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    error.to_string(),
                ));
            }
        };
    EmulationCommandTaskStep::Pending(single_pending_emulation_dispatch(
        cmd.id,
        owner_scope,
        PendingEmulationPageOperation::Policy(PagePolicyUpdateKind::SetViewportSurface),
        pending_viewport,
        EmulationPageCommandSource::browser(context, &target_id),
    ))
}

fn start_devtools_set_viewport_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsSetViewportCommand,
    owner_scope: CommandOwnerScope,
) -> Result<Option<PendingEmulationCommandDispatch>, DevToolsError> {
    if conn.browser_context.is_none()
        && conn.target_owner_identity_for_owner(&owner_scope).is_none()
    {
        return Ok(None);
    }
    let metrics = set_viewport_metrics_from_command(conn, &owner_scope, &command)?;
    start_apply_device_metrics(conn, command_id, metrics, owner_scope)
}

fn start_apply_device_metrics(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    metrics: EmulatedDeviceMetrics,
    owner_scope: CommandOwnerScope,
) -> Result<Option<PendingEmulationCommandDispatch>, DevToolsError> {
    if !conn.apply_emulation_override_for_owner(
        &owner_scope,
        EmulationPolicyChange::DeviceMetrics(Some(metrics.clone())),
    ) {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    }
    let Some((context_id, target_id)) = page_configuration_owner(conn, &owner_scope) else {
        return Ok(None);
    };
    let context = conn
        .browser_context_by_id(&context_id)
        .expect("resolved context remains registered");
    let pending = context
        .start_set_viewport_surface_for_target(&target_id, Some(metrics.viewport_surface()))
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
    Ok(Some(single_pending_emulation_dispatch(
        command_id,
        owner_scope,
        PendingEmulationPageOperation::Policy(PagePolicyUpdateKind::SetViewportSurface),
        pending,
        EmulationPageCommandSource::browser(context, &target_id),
    )))
}

fn set_viewport_metrics_from_command(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    command: &DevToolsSetViewportCommand,
) -> Result<EmulatedDeviceMetrics, DevToolsError> {
    let current_metrics = conn.target_session_owner_emulated_device_metrics_for_owner(owner);
    set_viewport_metrics_from_current(current_metrics.as_ref(), command)
}

fn set_viewport_metrics_from_current(
    current_metrics: Option<&EmulatedDeviceMetrics>,
    command: &DevToolsSetViewportCommand,
) -> Result<EmulatedDeviceMetrics, DevToolsError> {
    let current = (current_metrics).map_or_else(
        EmulatedViewportSurface::default,
        crate::conn::EmulatedDeviceMetrics::viewport_surface,
    );
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
        view: None,
        outer_width: width,
        outer_height: height,
        device_scale_factor,
        screen_width: command.screen_width.unwrap_or(width),
        screen_height: command.screen_height.unwrap_or(height),
        // Preserve the existing WebDriver/BiDi headless work area. CDP device
        // emulation supplies its own available screen dimensions instead.
        screen_avail_height: command.screen_height.unwrap_or(height).min(1040),
        window_x: current.window_x,
        window_y: current.window_y,
        screen_orientation: current.screen_orientation,
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
            execute_devtools_set_timezone_override_command(conn, command)
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
        browser_context.set_default_extra_headers(command.headers.clone());
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
        let result = start_extra_headers_for_current_route(conn, &route, command.headers.clone());
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
        let result = start_geolocation_override_for_current_route(
            conn,
            &route,
            command
                .override_state
                .map(emulated_geolocation_override_state),
        );
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
        browser_context.set_default_geolocation_override(
            command
                .override_state
                .map(emulated_geolocation_override_state),
        );
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
        let target = pending_emulation_target_for_route(conn, &route)?;
        let result = start_surface_override_for_route(conn, target, &route);
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
    let owner = CommandOwnerScope::for_route(route.clone());
    if !conn.apply_emulation_override_for_owner(
        &owner,
        EmulationPolicyChange::Geolocation(override_state),
    ) {
        return Err(devtools_emulation_owner_error(
            "BrowserContextNotLoaded".to_owned(),
        ));
    }
    let target = pending_emulation_target_for_route(conn, route)?;
    start_surface_override_for_route(conn, target, route)
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
        let result =
            start_network_conditions_for_current_route(conn, &route, command.network_conditions);
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
        browser_context.set_default_network_conditions(
            command.network_conditions.map(emulated_network_conditions),
        );
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
        let result = start_network_conditions_update_for_current_route(conn, &route);
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

fn start_network_conditions_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    network_conditions: Option<DevToolsNetworkConditions>,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let owner = CommandOwnerScope::for_route(route.clone());
    if !conn.apply_emulation_override_for_owner(
        &owner,
        EmulationPolicyChange::NetworkConditions(
            network_conditions.map(emulated_network_conditions),
        ),
    ) {
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
    let target = pending_emulation_target_for_route(conn, route)?;
    let effective_offline = match &target {
        PendingEmulationPageTarget::BrowserContextTarget {
            browser_context_id,
            target_id,
        } => conn
            .browser_context_by_id(browser_context_id)
            .is_some_and(|browser_context| {
                browser_context.effective_network_offline_for_target(target_id)
            }),
        PendingEmulationPageTarget::SessionOwner { .. } => false,
    };
    let owner = CommandOwnerScope::for_route(route.clone());
    let network_update = conn
        .start_set_network_offline_for_owner(&owner, effective_offline)
        .map_err(devtools_emulation_owner_error)?;
    let mut pending = Vec::new();
    if let Some(network_update) = network_update {
        pending.push(PendingEmulationPageCommand {
            source: EmulationPageCommandSource::browser_for_owner(conn, &owner),
            target: target.clone(),
            operation: PendingEmulationPageOperation::Policy(
                PagePolicyUpdateKind::SetNetworkConditions,
            ),
            pending: network_update,
        });
    }
    pending.extend(
        start_surface_override_for_route(conn, target, route)
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?,
    );
    Ok(pending)
}

fn start_extra_headers_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    headers: moli_fetch::RequestHeaders,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(conn, route)?;
    let owner = CommandOwnerScope::for_route(route.clone());
    let pending = conn
        .start_set_target_extra_http_headers_for_owner(&owner, headers)
        .map_err(devtools_emulation_owner_error)?;
    Ok(pending
        .map(|pending| {
            vec![PendingEmulationPageCommand {
                source: EmulationPageCommandSource::browser_for_owner(conn, &owner),
                target,
                operation: PendingEmulationPageOperation::Policy(
                    PagePolicyUpdateKind::SetExtraHttpHeaders,
                ),
                pending,
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
    let target = pending_emulation_target_for_route(conn, route)?;
    let headers = match &target {
        PendingEmulationPageTarget::BrowserContextTarget {
            browser_context_id,
            target_id,
        } => conn
            .browser_context_by_id(browser_context_id)
            .map(|browser_context| browser_context.effective_extra_headers_for_target(target_id)),
        PendingEmulationPageTarget::SessionOwner { .. } => None,
    };
    let Some(headers) = headers else {
        return Ok(Vec::new());
    };
    let Some((context_id, target_id)) = target.resolve(conn) else {
        return Ok(Vec::new());
    };
    let Some(context) = conn
        .browser_context_by_id(&context_id)
        .filter(|context| context.target_has_loaded_page(&target_id))
    else {
        return Ok(Vec::new());
    };
    let pending = context
        .start_set_extra_http_headers_for_target(&target_id, &headers)
        .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error.to_string()))?;
    Ok(vec![PendingEmulationPageCommand {
        source: EmulationPageCommandSource::browser(context, &target_id),
        target,
        operation: PendingEmulationPageOperation::Policy(PagePolicyUpdateKind::SetExtraHttpHeaders),
        pending,
    }])
}

impl PendingEmulationPageTarget {
    fn resolve(&self, conn: &CdpConnection) -> Option<(String, String)> {
        match self {
            Self::BrowserContextTarget {
                browser_context_id,
                target_id,
            } => Some((browser_context_id.clone(), target_id.clone())),
            Self::SessionOwner { owner_scope } => {
                conn.resolved_page_owner_identity_for_owner(owner_scope)
            }
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
        let result =
            start_user_agent_override_for_current_route(conn, &route, command.user_agent.clone());
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
    let fallback_identity = conn.base_browser_identity().clone();
    for browser_context_id in &browser_context_ids {
        let browser_context = conn
            .browser_context_by_id_mut(browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context
            .set_default_user_agent_override(command.user_agent.clone(), &fallback_identity);
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
        let result = start_user_agent_loader_update_for_current_route(conn, &route);
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
    let target = pending_emulation_target_for_route(conn, route)?;
    let owner = CommandOwnerScope::for_route(route.clone());
    let pending = conn
        .start_set_base_user_agent_override_for_owner(&owner, user_agent)
        .map_err(devtools_emulation_owner_error)?;
    Ok(pending.map(|pending| PendingEmulationPageCommand {
        source: EmulationPageCommandSource::browser_for_owner(conn, &owner),
        target,
        operation: PendingEmulationPageOperation::RebuildResourceRuntime,
        pending,
    }))
}

fn start_user_agent_loader_update_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
) -> Result<Option<PendingEmulationPageCommand>, DevToolsError> {
    let target = pending_emulation_target_for_route(conn, route)?;
    let owner = CommandOwnerScope::for_route(route.clone());
    let pending = conn
        .start_rebuild_resource_runtime_for_owner(&owner)
        .map_err(devtools_emulation_owner_error)?;
    Ok(pending.map(|pending| PendingEmulationPageCommand {
        source: EmulationPageCommandSource::browser_for_owner(conn, &owner),
        target,
        operation: PendingEmulationPageOperation::RebuildResourceRuntime,
        pending,
    }))
}

/// One process cannot promise independent BiDi environments. Reject a batch
/// before changing any owner rather than applying a prefix and then conflicting.
/// Choosing one owner already updates every Page/Worker's native defaults.
fn single_environment_owner<T>(owners: &[T]) -> Result<&T, DevToolsError> {
    let [owner] = owners else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "Locale/timezone overrides are process-wide; select one configuration owner",
        ));
    };
    Ok(owner)
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
    let target_id = single_environment_owner(&command.target_ids)?;
    let route = emulation_route_for_target(
        conn,
        target_id,
        "ChildFrameContextNotSupportedForLocaleOverride",
    )?;
    let pending = start_locale_override_for_current_route(conn, &route, command.locale)?;
    complete_emulation_page_updates(conn, devtools_command_session_id(&command.context), pending)
        .await
}

async fn execute_devtools_set_locale_override_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetLocaleOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    let fallback_identity = conn.base_browser_identity().clone();
    let browser_context_id = single_environment_owner(&browser_context_ids)?;
    conn.browser_context_by_id_mut(browser_context_id)
        .expect("resolved browser context must remain addressable")
        .set_default_locale_override(command.locale, &fallback_identity)
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::InvalidArgument, message))?;
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
        let result = start_locale_update_for_current_route(conn, &route);
        pending.extend(result?);
    }
    complete_emulation_page_updates(conn, session_id, pending).await
}

fn start_locale_override_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
    locale: Option<String>,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let owner = CommandOwnerScope::for_route(route.clone());
    conn.set_base_locale_override_for_owner(&owner, locale)
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::InvalidArgument, message))?;
    start_locale_update_for_current_route(conn, route)
}

fn start_locale_update_for_current_route(
    conn: &mut CdpConnection,
    route: &CdpSessionRoute,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    // BiDi locale also controls transport language. Date/Intl use the already
    // committed process default, not a second renderer-side configuration.
    Ok(
        start_user_agent_loader_update_for_current_route(conn, route)?
            .into_iter()
            .collect(),
    )
}

fn execute_devtools_set_timezone_override_command(
    conn: &mut CdpConnection,
    command: DevToolsSetTimezoneOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if !command.target_ids.is_empty() {
        return execute_devtools_set_timezone_override_for_targets(conn, command);
    }
    if !command.browser_context_ids.is_empty() {
        return execute_devtools_set_timezone_override_for_browser_contexts(conn, command);
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::InvalidArgument,
        "TimezoneOverrideRequiresContextOrUserContext",
    ))
}

fn execute_devtools_set_timezone_override_for_targets(
    conn: &mut CdpConnection,
    command: DevToolsSetTimezoneOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let target_id = single_environment_owner(&command.target_ids)?;
    let route = emulation_route_for_target(
        conn,
        target_id,
        "ChildFrameContextNotSupportedForTimezoneOverride",
    )?;
    let owner = CommandOwnerScope::for_route(route);
    conn.set_base_timezone_override_for_owner(&owner, command.timezone)
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::InvalidArgument, message))?;
    Ok(DevToolsCommandResult::Empty)
}

fn execute_devtools_set_timezone_override_for_browser_contexts(
    conn: &mut CdpConnection,
    command: DevToolsSetTimezoneOverrideCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let browser_context_ids = resolve_bidi_browser_context_ids(conn, &command.browser_context_ids)?;
    let browser_context_id = single_environment_owner(&browser_context_ids)?;
    conn.browser_context_by_id_mut(browser_context_id)
        .expect("resolved browser context must remain addressable")
        .set_default_timezone_override(command.timezone)
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::InvalidArgument, message))?;
    Ok(DevToolsCommandResult::Empty)
}

fn pending_emulation_target_for_route(
    _conn: &CdpConnection,
    route: &CdpSessionRoute,
) -> Result<PendingEmulationPageTarget, DevToolsError> {
    match route {
        CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            ..
        } => Ok(PendingEmulationPageTarget::BrowserContextTarget {
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
        routes.extend(browser_context.page_targets.iter().map(|target| {
            CdpSessionRoute::PageTarget {
                browser_context_id: browser_context.id.clone(),
                target_id: target.target_id().to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
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
        let mut command = command;
        command.context.session_id = None;
        return match start_devtools_set_viewport_command(
            conn,
            None,
            command,
            CommandOwnerScope::for_route(route),
        ) {
            Ok(Some(pending)) => {
                let completed = pending.wait().await;
                complete_pending_devtools_emulation_command(conn, completed)
            }
            Ok(None) => Ok(DevToolsCommandResult::Empty),
            Err(error) => Err(error),
        };
    }
    let owner = CommandOwnerScope::capture(
        conn,
        command.context.session_id.as_ref().map(|id| id.as_str()),
    );
    match start_devtools_set_viewport_command(conn, None, command, owner) {
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
        let mut command = command;
        command.context.session_id = None;
        return execute_devtools_set_window_state_for_owner(
            conn,
            command,
            CommandOwnerScope::for_route(route),
        )
        .await;
    }
    let owner = CommandOwnerScope::capture(
        conn,
        command.context.session_id.as_ref().map(|id| id.as_str()),
    );
    execute_devtools_set_window_state_for_owner(conn, command, owner).await
}

async fn execute_devtools_set_window_state_for_owner(
    conn: &mut CdpConnection,
    command: crate::devtools_runtime::DevToolsSetWindowStateCommand,
    owner: CommandOwnerScope,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let state = target_window_surface_state_from_devtools(command.state);
    if conn
        .set_window_surface_state_for_owner(&owner, state)
        .is_none()
    {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    }
    let pending = start_session_surface_override_page_command_for_owner(conn, &owner)
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
    let mut window_state_context = command.context.clone();
    window_state_context.session_id = None;
    window_state_context.target_id = Some(command.client_window.clone());
    let result = execute_devtools_set_window_state_for_owner(
        conn,
        crate::devtools_runtime::DevToolsSetWindowStateCommand {
            context: window_state_context,
            state: command.state,
        },
        CommandOwnerScope::for_route(route.clone()),
    )
    .await;

    match result {
        Ok(_) => {
            let owner = CommandOwnerScope::for_route(route);
            let _ = conn.set_window_surface_geometry_for_owner(
                &owner,
                command.width,
                command.height,
                command.x,
                command.y,
            );
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

fn target_window_surface_state_from_devtools(state: DevToolsWindowState) -> WindowSurfaceState {
    match state {
        DevToolsWindowState::Normal => WindowSurfaceState::Normal,
        DevToolsWindowState::Maximized => WindowSurfaceState::Maximized,
        DevToolsWindowState::Minimized => WindowSurfaceState::Minimized,
        DevToolsWindowState::Fullscreen => WindowSurfaceState::Fullscreen,
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
            .and_then(|context| context.emulation_defaults().device_metrics.as_ref());
        let metrics = set_viewport_metrics_from_current(current_default, &command)?;
        let browser_context = conn
            .browser_context_by_id_mut(&browser_context_id)
            .expect("resolved browser context must remain addressable");
        browser_context.set_default_device_metrics(metrics.clone());
        pending.extend(start_browser_context_default_device_metrics_page_commands(
            browser_context,
            &metrics,
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

fn start_browser_context_default_device_metrics_page_commands(
    context: &mut BrowserContext,
    metrics: &EmulatedDeviceMetrics,
) -> Result<Vec<PendingEmulationPageCommand>, DevToolsError> {
    let mut pending = Vec::new();
    let viewport_surface = Some(metrics.viewport_surface());
    for target in context
        .page_targets
        .active(context.selected_web_contents_id())
        .into_iter()
        .chain(context.background_targets())
    {
        let target_id = target.target_id();
        if !context.target_has_loaded_page(target_id)
            || context
                .target_emulation_policy(target_id)
                .is_none_or(|policy| policy.emulated_device_metrics.is_some())
        {
            continue;
        }
        pending.push(PendingEmulationPageCommand {
            source: EmulationPageCommandSource::browser(context, target_id),
            target: PendingEmulationPageTarget::BrowserContextTarget {
                browser_context_id: context.id.clone(),
                target_id: target_id.to_owned(),
            },
            operation: PendingEmulationPageOperation::Policy(
                PagePolicyUpdateKind::SetViewportSurface,
            ),
            pending: context
                .start_set_viewport_surface_for_target(target_id, viewport_surface)
                .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?,
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
        let CompletedEmulationPageCommand {
            target,
            operation,
            source,
            completed,
        } = completed_page;
        let completion = match completed {
            Ok(completion) => completion,
            Err(_)
                if pending_emulation_page_configuration_will_be_replayed(
                    conn, &target, &operation, source,
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(DevToolsError::new(DevToolsErrorKind::Internal, error));
            }
        };
        finish_pending_emulation_page_command(conn, operation, target, completion)
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
    }
    Ok(DevToolsCommandResult::Empty)
}

pub(crate) fn complete_pending_emulation_command(
    conn: &mut CdpConnection,
    completed: CompletedEmulationCommandDispatch,
) -> CommandOutputPlan {
    let session_id = completed.session_id.clone();
    let completed_pages = match completed.completed {
        CompletedEmulationRendererDispatch::Pages(completed_pages) => completed_pages,
        CompletedEmulationRendererDispatch::IoAdapterReply(completed) => {
            return match completed {
                Ok(CompletedDevToolsIoCommandDispatch::Dispatched) => {
                    CommandOutputPlan::result(json!({}))
                }
                Ok(CompletedDevToolsIoCommandDispatch::SessionResponse { .. }) => {
                    CommandOutputPlan::error(
                        -32000,
                        "adapter-reply Emulation dispatch used session output",
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
        let CompletedEmulationPageCommand {
            target,
            operation,
            source,
            completed,
        } = completed_page;
        let completion = match completed {
            Ok(completion) => completion,
            Err(_)
                if pending_emulation_page_configuration_will_be_replayed(
                    conn, &target, &operation, source,
                ) =>
            {
                continue;
            }
            Err(error) => return CommandOutputPlan::error(-32000, error),
        };
        let result = finish_pending_emulation_page_command(conn, operation, target, completion);
        if let Err(error) = result {
            return CommandOutputPlan::error(-32000, error);
        }
    }
    CommandOutputPlan::result(json!({}))
}

fn pending_emulation_page_configuration_will_be_replayed(
    conn: &CdpConnection,
    target: &PendingEmulationPageTarget,
    operation: &PendingEmulationPageOperation,
    source: EmulationPageCommandSource,
) -> bool {
    if !operation.has_authoritative_replay_state() {
        return false;
    }
    let Some((context_id, target_id)) = target.resolve(conn) else {
        return false;
    };
    let Some(context) = conn.browser_context_by_id(&context_id) else {
        return false;
    };
    let Some(_) = context.page_target(&target_id) else {
        return false;
    };
    // Replayed policy may settle on the retired renderer, but it must not update a replacement.
    match source {
        EmulationPageCommandSource::Browser(Some(page)) => {
            context.target_renderer_page_residence_identity(&target_id) != Some(page)
        }
        EmulationPageCommandSource::Browser(None) => false,
    }
}

fn page_configuration_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Option<(String, String)> {
    let route = conn.resolved_page_owner_identity_for_owner(owner)?;
    conn.browser_context_by_id(&route.0)?
        .target_has_loaded_page(&route.1)
        .then_some(route)
}

pub(crate) async fn dispose_page_session_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> anyhow::Result<()> {
    if conn.emulation_disposal_is_effectively_noop_for_session_owner(session_id)
        && conn.disable_emulation_session_handler_for_session_owner(session_id)
    {
        return Ok(());
    }
    conn.set_emulation_renderer_cleanup_pending_for_session_owner(session_id, true);
    let mut first_error = conn
        .clear_devtools_emulation_session_policy_async(session_id)
        .await
        .err();
    if !conn.disable_emulation_session_handler_for_session_owner(session_id) {
        return first_error.map_or(Ok(()), Err);
    }
    let owner = CommandOwnerScope::capture(conn, Some(session_id));
    let load_inputs = conn.navigation_load_inputs_for_owner(&owner);
    // A retry must install current effective Browser policy, even after the raw contribution was removed.
    if let Some((context_id, target_id)) = page_configuration_owner(conn, &owner)
        && let Some(context) = conn.browser_context_by_id_mut(&context_id)
        && let Err(error) = context
            .reconcile_target_runtime_policy_async(
                &target_id,
                load_inputs.script_execution_disabled,
                &load_inputs.emulated_media,
                load_inputs.network_offline,
                load_inputs.viewport_surface,
            )
            .await
    {
        first_error
            .get_or_insert_with(|| anyhow::anyhow!("failed to clear detached session {error}"));
    }
    record_emulation_disposal_result(
        &mut first_error,
        "page surfaces",
        apply_session_surface_state_async(conn, session_id).await,
    );
    if let Some(error) = first_error {
        return Err(error);
    }
    conn.set_emulation_renderer_cleanup_pending_for_session_owner(session_id, false);
    Ok(())
}

async fn apply_session_surface_state_async(
    conn: &mut CdpConnection,
    session_id: &str,
) -> anyhow::Result<()> {
    let Some(CdpSessionRoute::PageTarget {
        browser_context_id,
        target_id,
        ..
    }) = conn.session_route(Some(session_id))
    else {
        return Ok(());
    };
    let Some(browser_context) = conn.browser_context_by_id_mut(&browser_context_id) else {
        return Ok(());
    };
    if browser_context.is_active_target(&target_id) {
        browser_context
            .apply_surface_overrides_to_loaded_page_async()
            .await
    } else {
        browser_context
            .apply_background_target_surface_overrides_async(&target_id)
            .await
            .map(|_applied| ())
    }
}

fn record_emulation_disposal_result(
    first_error: &mut Option<anyhow::Error>,
    surface: &'static str,
    result: anyhow::Result<()>,
) {
    if let Err(error) = result {
        first_error.get_or_insert_with(|| {
            anyhow::anyhow!("failed to clear detached session {surface}: {error}")
        });
    }
}

fn single_pending_emulation_dispatch(
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    operation: PendingEmulationPageOperation,
    pending: PendingPageCommand,
    source: EmulationPageCommandSource,
) -> PendingEmulationCommandDispatch {
    let session_id = owner_scope.session_id().map(str::to_owned);
    PendingEmulationCommandDispatch {
        command_id,
        session_id: session_id.clone(),
        pending: PendingEmulationRendererDispatch::Pages(vec![PendingEmulationPageCommand {
            source,
            target: PendingEmulationPageTarget::SessionOwner { owner_scope },
            operation,
            pending,
        }]),
    }
}

fn emulation_command_is_context_wide(conn: &CdpConnection, session_id: Option<&str>) -> bool {
    match session_id {
        None => true,
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
    let Some(context) = conn.browser_context.as_ref() else {
        return Ok(Vec::new());
    };
    let mut pending = Vec::new();
    for target in context.page_targets.iter() {
        let target_id = target.target_id();
        if !context.target_has_loaded_page(target_id) {
            continue;
        }
        pending.push(PendingEmulationPageCommand {
            source: EmulationPageCommandSource::browser(context, target_id),
            target: PendingEmulationPageTarget::BrowserContextTarget {
                browser_context_id: context.id.clone(),
                target_id: target_id.to_owned(),
            },
            operation: PendingEmulationPageOperation::Policy(
                PagePolicyUpdateKind::SetEmulatedMedia,
            ),
            pending: context.start_set_emulated_media_for_target(target_id, overrides)?,
        });
    }
    Ok(pending)
}

fn start_surface_override_page_commands(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    if cmd.session_id.is_some() {
        return start_session_surface_override_page_command(conn, cmd.session_id);
    }
    let Some(browser_context) = conn.browser_context.as_mut() else {
        return Ok(Vec::new());
    };
    let navigator_overrides = browser_context.active_navigator_overrides();
    let document_activity = browser_context.active_document_activity();
    let browser_context_id = browser_context.id.clone();
    let Some(target_id) = browser_context.active_target_id_owned() else {
        return Ok(Vec::new());
    };
    if !browser_context.target_has_loaded_page(&target_id) {
        return Ok(Vec::new());
    }
    start_surface_override_page_command(
        PendingEmulationPageTarget::BrowserContextTarget {
            browser_context_id,
            target_id: target_id.clone(),
        },
        browser_context,
        &target_id,
        navigator_overrides,
        document_activity,
    )
}

fn start_session_surface_override_page_command(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let owner = CommandOwnerScope::capture(conn, session_id);
    start_session_surface_override_page_command_for_owner(conn, &owner)
}

fn start_session_surface_override_page_command_for_owner(
    conn: &mut CdpConnection,
    owner_scope: &CommandOwnerScope,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let (document_activity, navigator_overrides) = {
        let Some((browser_context_id, target_id)) =
            conn.target_owner_identity_for_owner(owner_scope)
        else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some(browser_context) = conn.browser_context_by_id(&browser_context_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        if let Some(target_id) = target_id.as_deref()
            && browser_context.background_target(target_id).is_some()
        {
            (
                browser_context
                    .document_activity_for_target(target_id)
                    .expect("resolved target"),
                browser_context
                    .navigator_overrides_for_target(target_id)
                    .expect("resolved target"),
            )
        } else {
            (
                browser_context.active_document_activity(),
                browser_context.active_navigator_overrides(),
            )
        }
    };
    let Some((context_id, target_id)) = page_configuration_owner(conn, owner_scope) else {
        return Ok(Vec::new());
    };
    let context = conn
        .browser_context_by_id(&context_id)
        .expect("resolved context remains registered");
    start_surface_override_page_command(
        PendingEmulationPageTarget::SessionOwner {
            owner_scope: owner_scope.clone(),
        },
        context,
        &target_id,
        navigator_overrides,
        document_activity,
    )
}

fn start_surface_override_for_route(
    conn: &mut CdpConnection,
    target: PendingEmulationPageTarget,
    route: &CdpSessionRoute,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let (document_activity, navigator_overrides) = match &target {
        PendingEmulationPageTarget::BrowserContextTarget {
            browser_context_id,
            target_id,
        } => {
            let Some(browser_context) = conn.browser_context_by_id(browser_context_id) else {
                return Err("BrowserContextNotLoaded".to_owned());
            };
            (
                browser_context
                    .document_activity_for_target(target_id)
                    .unwrap_or_default(),
                browser_context
                    .navigator_overrides_for_target(target_id)
                    .unwrap_or_default(),
            )
        }
        PendingEmulationPageTarget::SessionOwner { owner_scope } => {
            return start_session_surface_override_page_command_for_owner(conn, owner_scope);
        }
    };
    let owner = CommandOwnerScope::for_route(route.clone());
    let Some((context_id, target_id)) = page_configuration_owner(conn, &owner) else {
        return Ok(Vec::new());
    };
    let context = conn
        .browser_context_by_id(&context_id)
        .expect("resolved context remains registered");
    start_surface_override_page_command(
        target,
        context,
        &target_id,
        navigator_overrides,
        document_activity,
    )
}

fn start_surface_override_page_command(
    target: PendingEmulationPageTarget,
    context: &BrowserContext,
    target_id: &str,
    navigator_overrides: moli_page_types::NavigatorOverrides,
    document_activity: moli_page_types::DocumentActivity,
) -> Result<Vec<PendingEmulationPageCommand>, String> {
    let native_update =
        context.start_set_navigator_overrides_for_target(target_id, &navigator_overrides)?;
    let activity_update =
        context.start_set_document_activity_for_target(target_id, document_activity)?;
    Ok(vec![
        PendingEmulationPageCommand {
            source: EmulationPageCommandSource::browser(context, target_id),
            target: target.clone(),
            operation: PendingEmulationPageOperation::Policy(
                PagePolicyUpdateKind::SetNavigatorOverrides,
            ),
            pending: native_update,
        },
        PendingEmulationPageCommand {
            source: EmulationPageCommandSource::browser(context, target_id),
            target,
            operation: PendingEmulationPageOperation::Policy(
                PagePolicyUpdateKind::SetDocumentActivity,
            ),
            pending: activity_update,
        },
    ])
}

fn finish_pending_emulation_page_command(
    conn: &mut CdpConnection,
    operation: PendingEmulationPageOperation,
    target: PendingEmulationPageTarget,
    completion: CompletedPageCommand,
) -> Result<(), String> {
    match operation {
        PendingEmulationPageOperation::RebuildResourceRuntime => {
            if let PendingEmulationPageTarget::SessionOwner { owner_scope } = target {
                return conn.finish_rebuild_resource_runtime_for_owner(&owner_scope, completion);
            }
            if let Some((context_id, target_id)) = target.resolve(conn)
                && let Some(context) = conn.browser_context_by_id_mut(&context_id)
            {
                return context.finish_target_resource_runtime_update(&target_id, completion);
            }
            BrowserContext::finish_unobserved_resource_runtime_update(completion)
        }
        PendingEmulationPageOperation::Policy(kind) => {
            if let Some((context_id, target_id)) = target.resolve(conn)
                && let Some(context) = conn.browser_context_by_id_mut(&context_id)
            {
                return context.finish_target_page_policy_update(&target_id, kind, completion);
            }
            BrowserContext::finish_unobserved_page_policy_update(completion)
        }
    }
}
