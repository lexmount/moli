use crate::{
    conn::{BackgroundProtocolEvent, CdpConnection, CommandOwnerScope},
    domains::{
        activity::{
            ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink,
            ProtocolOutputSlot,
        },
        command_output::protocol_message_background_event,
    },
};
use moli_page_types::{
    RendererWebMcpEvent, RendererWebMcpObservation, RendererWebMcpResult, RendererWebMcpTool,
};
use serde_json::{Value, json};

#[derive(Debug, Default)]
pub(in crate::domains) struct WebMcpPreparedOutputSlot {
    events: Vec<BackgroundProtocolEvent>,
}

impl WebMcpPreparedOutputSlot {
    pub(in crate::domains) fn extend(&mut self, mut other: Self) {
        self.events.append(&mut other.events);
    }
}

pub(in crate::domains) async fn project_async(
    context: &mut ProtocolOutputProjectionContext<'_>,
    payloads: &mut ProtocolOutputPayloads,
) {
    if let Some(slot) = payloads.web_mcp_mut() {
        context
            .command
            .protocol_events_mut()
            .append(&mut slot.events);
    }
}

pub(in crate::domains) fn append_observation(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    observation: &RendererWebMcpObservation,
    sink: &mut impl ProtocolOutputSink,
) {
    // The producer selected the interested session. Resolve its exact route at
    // ingress, rather than delivering a historical event to a newly enabled one.
    let session_id = conn
        .page_event_session_ids_for_owner(owner)
        .into_iter()
        .find(|session| {
            moli_page_types::DevToolsSessionKey::from_wire_session_id(
                conn.target_renderer_runtime_inspector_session_id_for_session(session.as_deref())
                    .as_deref(),
            ) == observation.session
        });
    let Some(session_id) = session_id else {
        return;
    };
    let Some((root, _, _, _)) = conn.target_session_owner_frame_tree_identity_for_owner(owner)
    else {
        return;
    };
    let (method, params) = match &observation.event {
        RendererWebMcpEvent::ToolsAdded(tools) => (
            "WebMCP.toolsAdded",
            json!({"tools": tools.iter().map(|tool| tool_json(tool, &root)).collect::<Vec<_>>()}),
        ),
        RendererWebMcpEvent::ToolsRemoved(tools) => (
            "WebMCP.toolsRemoved",
            json!({"tools": tools.iter().map(|tool| json!({"name":tool.name,"frameId":tool.frame_id.as_ref().unwrap_or(&root)})).collect::<Vec<_>>()}),
        ),
        RendererWebMcpEvent::ToolInvoked {
            tool,
            invocation_id,
            input,
        } => (
            "WebMCP.toolInvoked",
            json!({"toolName":tool.name,"frameId":tool.frame_id.as_ref().unwrap_or(&root),"invocationId":invocation_id.to_string(),"input":input}),
        ),
        RendererWebMcpEvent::ToolResponded {
            invocation_id,
            result,
        } => {
            if let RendererWebMcpResult::Error {
                exception: Some(exception),
                ..
            } = result
            {
                let event_owner = session_id
                    .as_deref()
                    .map(CommandOwnerScope::for_session)
                    .unwrap_or_else(|| owner.clone());
                conn.register_runtime_remote_object_ids_from_value_for_owner(
                    &event_owner,
                    exception,
                );
            }
            (
                "WebMCP.toolResponded",
                response_json(*invocation_id, result),
            )
        }
    };
    let mut message = json!({"method":method,"params":params});
    if let Some(session_id) = session_id {
        message["sessionId"] = json!(session_id);
    }
    let event = protocol_message_background_event(message);
    sink.push_produced_slot(ProtocolOutputSlot::WebMcp);
    sink.push_prepared_payload(
        WebMcpPreparedOutputSlot {
            events: vec![event],
        }
        .into(),
    );
}

fn response_json(id: u64, result: &RendererWebMcpResult) -> Value {
    let mut params = json!({"invocationId":id.to_string()});
    match result {
        RendererWebMcpResult::Completed(output) => {
            params["status"] = json!("Completed");
            params["output"] = serde_json::from_str(output).unwrap_or_else(|_| json!(output));
        }
        RendererWebMcpResult::Canceled => {
            params["status"] = json!("Canceled");
            params["errorText"] = json!("");
        }
        RendererWebMcpResult::Error { message, exception } => {
            params["status"] = json!("Error");
            params["errorText"] = json!(message);
            if let Some(exception) = exception {
                params["exception"] = exception.clone();
            }
        }
    }
    params
}

fn tool_json(tool: &RendererWebMcpTool, root: &str) -> Value {
    let mut result = json!({"name":tool.name,"description":tool.description,"frameId":tool.frame_id.as_deref().unwrap_or(root)});
    if let Some(schema) = &tool.input_schema
        && let Ok(Value::Object(schema)) = serde_json::from_str(schema)
    {
        result["inputSchema"] = Value::Object(schema);
    }
    if let Some(annotations) = tool.annotations {
        result["annotations"] = json!({"readOnly":annotations.read_only,"consequential":annotations.consequential,"untrustedContent":annotations.untrusted_content,"debugging":annotations.debugging});
    }
    if tool.autosubmit {
        if result.get("annotations").is_none() {
            result["annotations"] = json!({});
        }
        result["annotations"]["autosubmit"] = json!(true);
    }
    if let Some(backend_node_id) = tool.backend_node_id {
        result["backendNodeId"] = json!(backend_node_id);
    }
    if let Some(stack_trace) = &tool.stack_trace {
        result["stackTrace"] = stack_trace.clone();
    }
    result
}

pub(in crate::domains) fn navigation_failed(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    id: u64,
) -> Vec<BackgroundProtocolEvent> {
    let params = response_json(id, &RendererWebMcpResult::navigation_failed());
    conn.page_event_session_ids_for_owner(owner)
        .into_iter()
        .filter_map(|session_id| {
            let event_owner = session_id
                .as_deref()
                .map(CommandOwnerScope::for_session)
                .unwrap_or_else(|| owner.clone());
            if !conn
                .target_devtools_session_state_for_owner(&event_owner)
                .is_some_and(|state| state.web_mcp_enabled)
            {
                return None;
            }
            let mut message = json!({"method":"WebMCP.toolResponded","params":params});
            if let Some(session) = session_id {
                message["sessionId"] = json!(session);
            }
            Some(protocol_message_background_event(message))
        })
        .collect()
}
