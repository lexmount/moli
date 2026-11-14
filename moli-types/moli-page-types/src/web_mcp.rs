//! Native WebMCP operations and producer-owned tool observations.
//! A missing frame ID denotes this renderer agent's local root; child IDs are
//! captured from the frame owner at production, before the Document retires.

use crate::DevToolsSessionKey;

#[derive(Clone, Debug)]
pub enum RendererWebMcpCommand {
    Enable,
    Disable,
    InvokeTool {
        frame_id: Option<String>,
        name: String,
        input: String,
    },
    CancelInvocation {
        invocation_id: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererWebMcpError {
    NotEnabled,
    InvalidParams(&'static str),
    SchedulingFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererWebMcpTool {
    pub frame_id: Option<String>,
    pub name: String,
    pub description: String,
    pub input_schema: Option<String>,
    pub annotations: Option<RendererWebMcpAnnotations>,
    pub autosubmit: bool,
    pub backend_node_id: Option<u32>,
    pub stack_trace: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RendererWebMcpAnnotations {
    pub read_only: bool,
    pub consequential: bool,
    pub untrusted_content: bool,
    pub debugging: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererWebMcpToolId {
    pub frame_id: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererWebMcpEvent {
    ToolsAdded(Vec<RendererWebMcpTool>),
    ToolsRemoved(Vec<RendererWebMcpToolId>),
    ToolInvoked {
        tool: RendererWebMcpToolId,
        invocation_id: u64,
        input: String,
    },
    ToolResponded {
        invocation_id: u64,
        result: RendererWebMcpResult,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererWebMcpResult {
    Completed(String),
    Canceled,
    Error {
        message: String,
        exception: Option<serde_json::Value>,
    },
}

impl RendererWebMcpResult {
    pub fn navigation_failed() -> Self {
        Self::Error {
            message: "Tool navigation did not produce an active destination Document".into(),
            exception: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererWebMcpObservation {
    pub session: DevToolsSessionKey,
    pub event: RendererWebMcpEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererWebMcpNavigation {
    pub invocation_id: u64,
    pub origin: url::Origin,
}
