//! Registry mutations and CDP notifications; callers own author task scheduling.

use super::devtools;
use super::state::RegisteredTool;
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use moli_page_types::{RendererWebMcpEvent, RendererWebMcpToolId};

pub(super) fn insert_tool(
    host: &mut JsContextHost,
    document: DomHandle,
    name: String,
    tool: RegisteredTool,
) {
    let observed = devtools::observes_tree(
        host,
        host.native_bridge().web_mcp.documents[&document].frame_tree,
    );
    let snapshot = {
        let entry = host
            .native_bridge_mut()
            .web_mcp
            .documents
            .get_mut(&document)
            .expect("active ModelContext");
        let snapshot = observed.then(|| devtools::protocol_tool(entry, &name, &tool));
        entry.tools.insert(name, tool);
        snapshot
    };
    if let Some(snapshot) = snapshot {
        devtools::emit(host, RendererWebMcpEvent::ToolsAdded(vec![snapshot]));
    }
}

pub(super) fn take_tool(
    host: &mut JsContextHost,
    document: DomHandle,
    name: String,
) -> Option<RegisteredTool> {
    let (tool, frame_id, tree) = {
        let entry = host
            .native_bridge_mut()
            .web_mcp
            .documents
            .get_mut(&document)?;
        let tool = entry.tools.remove(&name)?;
        (tool, entry.frame_id.clone(), entry.frame_tree)
    };
    devtools::emit_in_tree(host, tree, || {
        RendererWebMcpEvent::ToolsRemoved(vec![RendererWebMcpToolId { frame_id, name }])
    });
    Some(tool)
}
