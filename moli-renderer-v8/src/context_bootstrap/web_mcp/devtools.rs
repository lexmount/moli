use super::bindings::document_owner;
use super::execution;
use super::state::{DocumentTools, RegisteredTool, ToolExecutor};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use moli_page_types::{
    DevToolsSessionKey, RendererWebMcpAnnotations, RendererWebMcpEvent, RendererWebMcpObservation,
    RendererWebMcpTool,
};

use moli_page_types::{RendererWebMcpCommand, RendererWebMcpError};

pub(crate) fn configure_session(
    host: &mut JsContextHost,
    session: DevToolsSessionKey,
    enabled: bool,
) {
    if !enabled {
        host.native_bridge_mut()
            .web_mcp
            .enabled_sessions
            .remove(&session);
        return;
    }
    host.native_bridge_mut()
        .web_mcp
        .enabled_sessions
        .insert(session.clone());
    let tools = host
        .native_bridge()
        .web_mcp
        .documents
        .iter()
        .filter(|(document, entry)| {
            entry.frame_tree.is_none() && document_owner(host, **document) == Some(entry.owner)
        })
        .flat_map(|(_, entry)| {
            entry
                .tools
                .iter()
                .map(|(name, tool)| protocol_tool(entry, name, tool))
        })
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        host.append_live_turn_observation(crate::runtime::RendererProtocolObservation::WebMcp(
            RendererWebMcpObservation {
                session,
                event: RendererWebMcpEvent::ToolsAdded(tools),
            },
        ));
    }
}

pub(crate) fn dispatch_command<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    session: DevToolsSessionKey,
    command: RendererWebMcpCommand,
) -> Result<Option<u64>, RendererWebMcpError> {
    match command {
        RendererWebMcpCommand::Enable | RendererWebMcpCommand::Disable => {
            configure_session(
                unsafe { &mut *host_ptr },
                session,
                matches!(command, RendererWebMcpCommand::Enable),
            );
            Ok(None)
        }
        RendererWebMcpCommand::InvokeTool {
            frame_id,
            name,
            input,
        } => {
            if !unsafe { &*host_ptr }
                .native_bridge()
                .web_mcp
                .enabled_sessions
                .contains(&session)
            {
                return Err(RendererWebMcpError::NotEnabled);
            }
            let found = {
                let host = unsafe { &*host_ptr };
                host.native_bridge()
                    .web_mcp
                    .documents
                    .iter()
                    .find(|(document, entry)| {
                        entry.frame_tree.is_none()
                            && entry.frame_id == frame_id
                            && document_owner(host, **document) == Some(entry.owner)
                    })
                    .map(|(document, entry)| (*document, entry.tools.contains_key(&name)))
            };
            let Some((document, registered)) = found else {
                return Err(RendererWebMcpError::InvalidParams(if frame_id.is_none() {
                    "Tool not found"
                } else {
                    "No frame for given id found"
                }));
            };
            if !registered {
                return Err(RendererWebMcpError::InvalidParams("Tool not found"));
            }
            execution::schedule_invocation(
                scope, host_ptr, document, document, name, input, None, None,
            )
            .map(Some)
            .ok_or(RendererWebMcpError::SchedulingFailed)
        }
        RendererWebMcpCommand::CancelInvocation { invocation_id } => {
            if unsafe { &*host_ptr }
                .native_bridge()
                .web_mcp
                .pending
                .get(&invocation_id)
                .is_none_or(|pending| pending.frame_tree.is_some())
                || !execution::cancel_invocation(scope, host_ptr, invocation_id)
            {
                return Err(RendererWebMcpError::InvalidParams(
                    "No pending execution for invocation id",
                ));
            }
            Ok(None)
        }
    }
}

pub(super) fn document_frame_id(host: &JsContextHost, document: DomHandle) -> Option<String> {
    let child = host.child_browsing_context_host_for_document_handle(document)?;
    host.frame_owner_frame_id_for_child_handle(child)
        .map(|frame| frame.0)
}

pub(super) fn protocol_tool(
    entry: &DocumentTools,
    name: &str,
    tool: &RegisteredTool,
) -> RendererWebMcpTool {
    let metadata = &tool.metadata;
    let (autosubmit, backend_node_id) = match tool.executor {
        ToolExecutor::Form {
            autosubmit,
            backend_node_id,
            ..
        } => (autosubmit, backend_node_id),
        ToolExecutor::Callback(_) => (false, None),
    };
    RendererWebMcpTool {
        autosubmit,
        backend_node_id,
        frame_id: entry.frame_id.clone(),
        name: name.into(),
        description: metadata.description.clone(),
        input_schema: metadata.input_schema.clone(),
        stack_trace: tool.stack_trace.clone(),
        annotations: metadata
            .annotations
            .map(|annotations| RendererWebMcpAnnotations {
                read_only: annotations.read_only,
                consequential: annotations.consequential,
                untrusted_content: annotations.untrusted_content,
                debugging: annotations.debugging,
            }),
    }
}

pub(super) fn capture_registration_stack(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<serde_json::Value> {
    let stack = v8::StackTrace::current_stack_trace(scope, 32)?;
    let frames = (0..stack.get_frame_count())
        .filter_map(|index| {
            let frame = stack.get_frame(scope, index)?;
            let function_name = frame
                .get_function_name(scope)
                .map(|name| name.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let url = frame
                .get_script_name_or_source_url(scope)
                .map(|name| name.to_rust_string_lossy(scope))
                .unwrap_or_default();
            Some(serde_json::json!({
                "functionName":function_name,"url":url,
                "scriptId":frame.get_script_id().to_string(),
                "lineNumber":frame.get_line_number().saturating_sub(1),
                "columnNumber":frame.get_column().saturating_sub(1),
            }))
        })
        .collect::<Vec<_>>();
    (!frames.is_empty()).then(|| serde_json::json!({"callFrames":frames}))
}

pub(crate) fn emit(host: &JsContextHost, event: RendererWebMcpEvent) {
    let mut sessions = host.native_bridge().web_mcp.enabled_sessions.iter();
    let Some(last) = sessions.next_back() else {
        return;
    };
    for session in sessions {
        host.append_live_turn_observation(crate::runtime::RendererProtocolObservation::WebMcp(
            RendererWebMcpObservation {
                session: session.clone(),
                event: event.clone(),
            },
        ));
    }
    host.append_live_turn_observation(crate::runtime::RendererProtocolObservation::WebMcp(
        RendererWebMcpObservation {
            session: last.clone(),
            event,
        },
    ));
}

pub(super) fn observes_tree(host: &JsContextHost, tree: Option<u64>) -> bool {
    tree.is_none() && !host.native_bridge().web_mcp.enabled_sessions.is_empty()
}

pub(super) fn emit_in_tree(
    host: &JsContextHost,
    tree: Option<u64>,
    event: impl FnOnce() -> RendererWebMcpEvent,
) {
    if observes_tree(host, tree) {
        emit(host, event());
    }
}
