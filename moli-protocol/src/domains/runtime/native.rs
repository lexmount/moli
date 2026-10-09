use moli_core::page::{
    RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage, RuntimeBindingRegistration,
};
use moli_core::{
    RendererNativeProtocolResponse, RendererNativeProtocolStateUpdate, RendererPageCommand,
    RendererPageReply,
};
use serde_json::json;

use super::bindings::{
    AddBindingParams, clear_runtime_binding_definitions_for_owner,
    persist_runtime_binding_definition_for_owner, remove_runtime_binding_definitions_for_owner,
};
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, RuntimeEnableEventsReplay,
    RuntimeEnableReplayEvent, SessionOwnerRuntimeFrontendEnableResult,
};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::native::{self, NativeCommandStep};
use crate::domains::observable_output::advance_runtime_observable_cursors_to_current_for_owner;
use crate::domains::runtime_context_events::{
    apply_runtime_context_protocol_event_side_effects_for_owner_typed,
    emit_runtime_context_protocol_background_event_typed,
    should_emit_child_default_context_inventory_replay_once_for_owner,
};

pub(in crate::domains) fn try_start_native_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<NativeCommandStep> {
    if matches!(cmd.action, "addBinding" | "removeBinding") {
        return try_start_binding(conn, cmd);
    }
    let enabled = match cmd.action {
        "enable" => true,
        "disable" => false,
        _ => return None,
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let inspector_session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    Some(native::start(
        conn,
        cmd,
        if enabled {
            RendererPageCommand::runtime_enable_events(inspector_session)
        } else {
            RendererPageCommand::runtime_disable_events(inspector_session)
        },
        move |reply| match reply {
            Ok(RendererPageReply::RuntimeInspectorProtocolMessages(output)) => {
                let mut response = RendererNativeProtocolResponse::success(json!({}));
                response.state_updates.push(
                    RendererNativeProtocolStateUpdate::RuntimeSubscription {
                        enabled,
                        output: Box::new(output),
                    },
                );
                response
            }
            Err(error) => RendererNativeProtocolResponse::error(-32000, error.to_string()),
            _ => unreachable!("Runtime subscription commands must return frozen inspector output"),
        },
    ))
}

fn try_start_binding(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let remove = cmd.action == "removeBinding";
    let (name, execution_context_name) = if remove {
        match cmd.get_params::<chromiumoxide_cdp::cdp::js_protocol::runtime::RemoveBindingParams>()
        {
            Ok(Some(params)) => (params.name, None),
            _ => {
                return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
                    -32602,
                    "InvalidParams",
                )));
            }
        }
    } else {
        match cmd.get_params::<AddBindingParams>() {
            // Context-local bindings do not persist. Keep the existing exact
            // context lookup, including child default worlds, for that route.
            Ok(Some(params)) if params.execution_context_id.is_some() => return None,
            Ok(Some(params)) => (params.name, params.execution_context_name),
            _ => {
                return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
                    -32602,
                    "InvalidParams",
                )));
            }
        }
    };
    let binding = RuntimeBindingRegistration {
        devtools_session: None,
        name,
        execution_context_name,
    };
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let inspector_session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    Some(native::start(
        conn,
        cmd,
        RendererPageCommand::runtime_binding_events(inspector_session, binding.clone(), remove),
        move |reply| match reply {
            Ok(RendererPageReply::RuntimeInspectorProtocolMessages(output)) => {
                if let Some(error) = output.messages().iter().find_map(|message| match message {
                    RendererRuntimeInspectorMessage::Protocol(message) => message.get("error"),
                    _ => None,
                }) {
                    return RendererNativeProtocolResponse::error(
                        error["code"]
                            .as_i64()
                            .and_then(|code| i32::try_from(code).ok())
                            .unwrap_or(-32000),
                        error["message"]
                            .as_str()
                            .unwrap_or("Runtime binding command failed"),
                    );
                }
                let mut response = RendererNativeProtocolResponse::success(json!({}));
                response
                    .state_updates
                    .push(RendererNativeProtocolStateUpdate::RuntimeBinding {
                        binding,
                        remove,
                        output: Box::new(output),
                    });
                response
            }
            Err(error) => RendererNativeProtocolResponse::error(-32000, error.to_string()),
            _ => unreachable!("Runtime binding commands must return frozen inspector output"),
        },
    ))
}

pub(in crate::domains) fn project_binding_terminal(
    conn: &mut CdpConnection,
    owner: Option<&CommandOwnerScope>,
    binding: RuntimeBindingRegistration,
    remove: bool,
    output: RendererRuntimeCommandOutput,
) -> Result<Vec<BackgroundProtocolEvent>, String> {
    let Some(owner) = owner else {
        return Ok(Vec::new());
    };
    let replay = conn.prepare_runtime_command_events_for_owner(owner, output)?;
    if remove {
        remove_runtime_binding_definitions_for_owner(conn, owner, &binding.name)?;
    } else {
        persist_runtime_binding_definition_for_owner(
            conn,
            owner,
            binding.name,
            binding.execution_context_name,
        )?;
    }
    Ok(project_replay_events(conn, owner, replay))
}

pub(in crate::domains) fn project_subscription_terminal(
    conn: &mut CdpConnection,
    owner: Option<&CommandOwnerScope>,
    enabled: bool,
    output: RendererRuntimeCommandOutput,
) -> Result<Vec<BackgroundProtocolEvent>, String> {
    let Some(owner) = owner else {
        if !enabled {
            // Disable has already committed in the original renderer. It has
            // no replay to install on a successor, so retirement preserves
            // that success. Enable needs its live owner to deliver the replay.
            return Ok(Vec::new());
        }
        return Err("Runtime.enable completed after session owner disappeared".to_owned());
    };
    let replay = conn.prepare_runtime_subscription_events_for_owner(owner, output)?;
    if enabled {
        project_enable_replay(conn, owner, replay)
    } else {
        apply_runtime_disable_projection_after_success_for_owner(conn, owner)?;
        Ok(project_replay_events(conn, owner, replay))
    }
}

/// All replay state and deduplication advance at the same delivery boundary as
/// the frozen replay. There is no dependency on polling the adapter completion.
pub(super) fn project_enable_replay(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    replay: RuntimeEnableEventsReplay,
) -> Result<Vec<BackgroundProtocolEvent>, String> {
    match conn.set_runtime_frontend_enabled_for_owner(owner, true) {
        SessionOwnerRuntimeFrontendEnableResult::Handled => {}
        SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
            return Err("Runtime.enable succeeded after session owner disappeared".to_owned());
        }
    }
    Ok(project_replay_events(conn, owner, replay))
}

fn project_replay_events(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    replay: RuntimeEnableEventsReplay,
) -> Vec<BackgroundProtocolEvent> {
    let frame_id = conn.runtime_session_owner_frame_id_for_owner(owner);
    let mut events = Vec::new();
    for event in replay.into_events() {
        match event {
            RuntimeEnableReplayEvent::Context(event) => {
                if !should_emit_child_default_context_inventory_replay_once_for_owner(
                    conn,
                    owner,
                    frame_id.as_deref(),
                    &event,
                ) {
                    continue;
                }
                apply_runtime_context_protocol_event_side_effects_for_owner_typed(
                    conn, &event, owner,
                );
                emit_runtime_context_protocol_background_event_typed(
                    &mut events,
                    event,
                    owner.session_id(),
                );
            }
            RuntimeEnableReplayEvent::Background(event) => events.push(event),
        }
    }
    events
}

pub(super) fn apply_runtime_disable_projection_after_success_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> Result<(), String> {
    let was_enabled = conn
        .target_runtime_session_state_for_owner(owner)
        .is_some_and(|state| state.runtime_frontend_enabled);
    match conn.set_runtime_frontend_enabled_for_owner(owner, false) {
        SessionOwnerRuntimeFrontendEnableResult::Handled => {
            advance_runtime_observable_cursors_to_current_for_owner(conn, owner);
            if was_enabled {
                clear_runtime_binding_definitions_for_owner(conn, owner)?;
            }
            Ok(())
        }
        SessionOwnerRuntimeFrontendEnableResult::UnknownSession => {
            Err("Runtime.disable succeeded after session owner disappeared".to_owned())
        }
    }
}
