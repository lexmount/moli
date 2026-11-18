use super::*;
use crate::domains::native::{self, NativeCommandStep};
use moli_core::{RendererDevToolsPageCommand, RendererDevToolsPageReply};
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeProtocolResponse as Response,
    RendererPageCommand as Command, RendererPageReply as Reply,
};

pub(crate) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let action = cmd.parse_action::<DomDebuggerAction>()?;
    Some(match prepare(conn, cmd, action) {
        Ok(operation) => native::start_operation(conn, cmd, operation),
        Err(plan) => NativeCommandStep::Complete(plan),
    })
}

fn invalid() -> CommandOutputPlan {
    CommandOutputPlan::error(-32602, "Invalid parameters")
}

fn prepare(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: DomDebuggerAction,
) -> Result<Operation, CommandOutputPlan> {
    let session = conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
    match action {
        DomDebuggerAction::GetEventListeners => {
            let params: GetEventListenersParams =
                cmd.get_params().ok().flatten().ok_or_else(invalid)?;
            Ok(Operation::new(
                Command::dom_debugger_get_event_listeners(
                    session,
                    params.object_id,
                    params.depth,
                    params.pierce,
                ),
                |reply| match reply {
                    Ok(Reply::DevTools(RendererDevToolsPageReply::DomDebuggerEventListeners(
                        RendererDomDebuggerEventListenersResolution::Found(listeners),
                    ))) => Response::success(
                        json!({"listeners": listeners.into_iter().map(listener_payload).collect::<Vec<_>>() }),
                    ),
                    Ok(Reply::DevTools(RendererDevToolsPageReply::DomDebuggerEventListeners(
                        RendererDomDebuggerEventListenersResolution::InvalidRemoteObjectId(message),
                    ))) => Response::error(-32000, message),
                    Err(error) => Response::error(-32000, error.to_string()),
                    _ => unreachable!("DOMDebugger listeners reply"),
                },
            ))
        }
        DomDebuggerAction::SetDOMBreakpoint | DomDebuggerAction::RemoveDOMBreakpoint => {
            let params: DomBreakpointParams =
                cmd.get_params().ok().flatten().ok_or_else(invalid)?;
            Ok(Operation::new(
                Command::DevTools(
                    RendererDevToolsPageCommand::DomDebuggerConfigureDomBreakpoint {
                        inspector_session_id: session,
                        frontend_node_id: params.node_id,
                        breakpoint_type: params.r#type,
                        enabled: action == DomDebuggerAction::SetDOMBreakpoint,
                    },
                ),
                |reply| match reply {
                    Ok(Reply::DevTools(RendererDevToolsPageReply::DomDebuggerDomBreakpoint(
                        RendererDomDebuggerDomBreakpointResolution::Configured,
                    ))) => Response::success(json!({})),
                    Ok(Reply::DevTools(RendererDevToolsPageReply::DomDebuggerDomBreakpoint(
                        RendererDomDebuggerDomBreakpointResolution::NodeNotFound,
                    ))) => Response::error(-32000, "Could not find node with given id"),
                    Ok(Reply::DevTools(RendererDevToolsPageReply::DomDebuggerDomBreakpoint(
                        RendererDomDebuggerDomBreakpointResolution::UnknownType(kind),
                    ))) => Response::error(-32000, format!("Unknown DOM breakpoint type: {kind}")),
                    Err(error) => Response::error(-32000, error.to_string()),
                    _ => unreachable!("DOMDebugger DOM breakpoint reply"),
                },
            ))
        }
        DomDebuggerAction::SetEventListenerBreakpoint
        | DomDebuggerAction::RemoveEventListenerBreakpoint => {
            let params: EventListenerBreakpointParams =
                cmd.get_params().ok().flatten().ok_or_else(invalid)?;
            if params.event_name.is_empty() {
                return Err(CommandOutputPlan::error(-32000, "Event name is empty"));
            }
            let breakpoint = RendererDomDebuggerEventListenerBreakpoint::new(
                params.event_name,
                params.target_name,
            );
            let enabled = action == DomDebuggerAction::SetEventListenerBreakpoint;
            // This is session policy replayed on replacement renderers. Record
            // the desired state at admission, as other target configuration
            // does, before a navigation can capture the replay configuration.
            conn.with_target_devtools_session_state_for_session_mut(cmd.session_id, |state| {
                if enabled {
                    state
                        .dom_debugger_event_listener_breakpoints
                        .insert(breakpoint.clone());
                } else {
                    state
                        .dom_debugger_event_listener_breakpoints
                        .remove(&breakpoint);
                }
            })
            .ok_or_else(|| CommandOutputPlan::error(-32000, "NoSuchTarget"))?;
            Ok(Operation::new(
                Command::DevTools(
                    RendererDevToolsPageCommand::DomDebuggerConfigureEventListenerBreakpoint {
                        inspector_session_id: session,
                        breakpoint,
                        enabled,
                    },
                ),
                native::unit_response,
            ))
        }
        DomDebuggerAction::SetXHRBreakpoint | DomDebuggerAction::RemoveXHRBreakpoint => {
            let params: XhrBreakpointParams =
                cmd.get_params().ok().flatten().ok_or_else(invalid)?;
            let breakpoint = RendererDomDebuggerXhrBreakpoint::new(params.url);
            let enabled = action == DomDebuggerAction::SetXHRBreakpoint;
            conn.with_target_devtools_session_state_for_session_mut(cmd.session_id, |state| {
                if enabled {
                    state
                        .dom_debugger_xhr_breakpoints
                        .insert(breakpoint.clone());
                } else {
                    state.dom_debugger_xhr_breakpoints.remove(&breakpoint);
                }
            })
            .ok_or_else(|| CommandOutputPlan::error(-32000, "NoSuchTarget"))?;
            Ok(Operation::new(
                Command::DevTools(
                    RendererDevToolsPageCommand::DomDebuggerConfigureXhrBreakpoint {
                        inspector_session_id: session,
                        breakpoint,
                        enabled,
                    },
                ),
                native::unit_response,
            ))
        }
    }
}
