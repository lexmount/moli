//! Minimal CDP DOMSnapshot domain.
//!
//! The protocol shape is table/string-pool based, unlike the DOM domain's
//! object tree. We capture the current renderer-owned live DOM on demand and
//! include resolved computed styles plus lightweight geometry for automation clients.

use moli_core::page::{
    CompletedPageCommand, Page, PendingPageCommand, RendererDomSnapshotCaptureOptions,
};
use serde::Deserialize;

use crate::conn::{CdpConnection, Cmd};
use crate::domains::actions::DomSnapshotAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) struct PendingDomSnapshotCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedDomSnapshotCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    completed: Result<CompletedPageCommand, String>,
}

pub(crate) enum DomSnapshotCommandDispatchStep {
    Pending(PendingDomSnapshotCommandDispatch),
    Complete(CommandOutputPlan),
}

impl PendingDomSnapshotCommandDispatch {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub async fn wait(self) -> CompletedDomSnapshotCommandDispatch {
        CompletedDomSnapshotCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedDomSnapshotCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

pub(crate) fn try_start_dom_snapshot_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> DomSnapshotCommandDispatchStep {
    match cmd.parse_action::<DomSnapshotAction>() {
        Some(DomSnapshotAction::Enable | DomSnapshotAction::Disable) => {
            DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::success())
        }
        Some(DomSnapshotAction::CaptureSnapshot) => start_capture_snapshot_command(conn, cmd),
        None => DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        )),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureSnapshotParams {
    #[serde(default)]
    computed_styles: Vec<String>,
    #[serde(default)]
    include_paint_order: bool,
    #[serde(default, rename = "includeDOMRects")]
    include_dom_rects: bool,
    #[serde(default)]
    include_blended_background_colors: bool,
    #[serde(default)]
    include_text_color_opacities: bool,
}

impl From<CaptureSnapshotParams> for RendererDomSnapshotCaptureOptions {
    fn from(params: CaptureSnapshotParams) -> Self {
        Self {
            computed_styles: params.computed_styles,
            include_paint_order: params.include_paint_order,
            include_dom_rects: params.include_dom_rects,
            include_blended_background_colors: params.include_blended_background_colors,
            include_text_color_opacities: params.include_text_color_opacities,
        }
    }
}

fn start_capture_snapshot_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> DomSnapshotCommandDispatchStep {
    if let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id) {
        return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(-32000, message));
    }
    let params: CaptureSnapshotParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => CaptureSnapshotParams::default(),
        Err(_) => {
            return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };

    start_capture_snapshot_command_with_params(conn, cmd.id, cmd.session_id, params)
}

fn start_capture_snapshot_command_with_params(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    params: CaptureSnapshotParams,
) -> DomSnapshotCommandDispatchStep {
    let frame_id = top_frame_id_for_session(conn, session_id).unwrap_or_default();
    let Some(page) = loaded_page_mut_for_session(conn, session_id) else {
        return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    };
    let pending = match page.start_dom_snapshot_capture(frame_id, params.into()) {
        Ok(pending) => pending,
        Err(error) => {
            return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
                -32000,
                error.to_string(),
            ));
        }
    };

    DomSnapshotCommandDispatchStep::Pending(PendingDomSnapshotCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        pending,
    })
}

pub(crate) fn complete_pending_dom_snapshot_command(
    conn: &mut CdpConnection,
    completed: CompletedDomSnapshotCommandDispatch,
) -> DomSnapshotCommandDispatchStep {
    let session_id = completed.session_id.as_deref();
    let Some(page) = loaded_page_mut_for_session(conn, session_id) else {
        return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32000,
            "NoDocumentLoaded",
        ));
    };
    let completion = match completed.completed {
        Ok(completion) => completion,
        Err(error) => {
            return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
                -32000, error,
            ));
        }
    };
    let payload = match page.finish_dom_snapshot_capture(completion) {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
                -32000,
                "NoDocumentLoaded",
            ));
        }
        Err(error) => {
            return DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::error(
                -32000,
                format!("Could not capture DOM snapshot: {error}"),
            ));
        }
    };
    DomSnapshotCommandDispatchStep::Complete(CommandOutputPlan::result(
        payload.into_protocol_payload(),
    ))
}

fn loaded_page_mut_for_session<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
) -> Option<&'a mut Page> {
    conn.loaded_page_mut_for_protocol_access(session_id).ok()
}

fn top_frame_id_for_session(conn: &CdpConnection, session_id: Option<&str>) -> Option<String> {
    conn.target_session_owner_frame_tree_identity(session_id)
        .map(|(frame_id, _, _, _)| frame_id)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        conn::{BrowserContext, CdpCommandTaskStep, PageTargetHost},
        domains::page::LOADER_ID,
        testing::{TestContext, wait_until_renderer_document_load, wait_until_scheduler_message},
    };

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut bc = BrowserContext::new("BID-1".to_owned());
        bc.set_active_target_id("TID-1".to_owned());
        ctx.conn.browser_context = Some(bc);
        ctx.install_navigation_fixture_for_session_owner(&format!("data:text/html,{html}"), None)
            .await;
        wait_until_renderer_document_load(ctx, None, "TID-1", LOADER_ID).await;
        wait_until_scheduler_message(ctx, "initial DOMSnapshot fixture load output", |message| {
            message["method"] == json!("Page.loadEventFired")
        })
        .await;
        ctx.sent.clear();
    }

    async fn process_via_command_dispatch(ctx: &mut TestContext, msg: serde_json::Value) {
        let raw = serde_json::to_string(&msg).expect("test command should serialize");
        let step = ctx.conn.start_command_dispatch(&raw);
        let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
        ctx.sent.extend(messages);
    }

    async fn complete_pending_command_task_for_test(
        ctx: &mut TestContext,
        pending: crate::conn::PendingCdpCommandDispatch,
    ) -> Vec<serde_json::Value> {
        ctx.complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
            .await
            .0
    }

    async fn wait_until_runtime_value(
        ctx: &mut TestContext,
        command_id_base: u64,
        description: &str,
        expression: &str,
        expected: serde_json::Value,
    ) {
        let mut last_value = serde_json::Value::Null;
        for offset in 0..32 {
            let command_id = command_id_base + offset;
            process_via_command_dispatch(
                ctx,
                json!({
                    "id": command_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": expression,
                        "returnByValue": true
                    }
                }),
            )
            .await;
            last_value = ctx.take_response_by_id(command_id)["result"]["result"]["value"].clone();
            if last_value == expected {
                return;
            }
        }
        panic!("timed out waiting for {description}; last value={last_value}");
    }

    fn take_capture_snapshot(ctx: &mut TestContext, id: u64) -> serde_json::Value {
        ctx.take_response_by_id(id)["result"].clone()
    }

    fn strings(result: &serde_json::Value) -> Vec<String> {
        result["strings"]
            .as_array()
            .expect("strings table")
            .iter()
            .map(|value| value.as_str().expect("string").to_owned())
            .collect()
    }

    fn decode_string(strings: &[String], value: &serde_json::Value) -> String {
        let index = value.as_u64().expect("string index") as usize;
        strings.get(index).expect("string table entry").clone()
    }

    fn decoded_node_names(result: &serde_json::Value) -> Vec<String> {
        decoded_node_names_for_document(result, 0)
    }

    fn decoded_node_names_for_document(
        result: &serde_json::Value,
        document_index: usize,
    ) -> Vec<String> {
        let strings = strings(result);
        result["documents"][document_index]["nodes"]["nodeName"]
            .as_array()
            .expect("node names")
            .iter()
            .map(|index| decode_string(&strings, index))
            .collect()
    }

    fn decoded_layout_style_for_node(
        result: &serde_json::Value,
        document_index: usize,
        node_index: usize,
    ) -> Vec<String> {
        let strings = strings(result);
        let layout = &result["documents"][document_index]["layout"];
        let layout_index = layout["nodeIndex"]
            .as_array()
            .expect("layout node indices")
            .iter()
            .position(|index| index.as_u64() == Some(node_index as u64))
            .unwrap_or_else(|| panic!("node {node_index} should have a layout row: {layout}"));
        layout["styles"][layout_index]
            .as_array()
            .expect("layout style row")
            .iter()
            .map(|index| decode_string(&strings, index))
            .collect()
    }

    fn content_document_index_for_owner_node(
        result: &serde_json::Value,
        document_index: usize,
        owner_node_index: usize,
    ) -> Option<usize> {
        let content_document_index =
            &result["documents"][document_index]["nodes"]["contentDocumentIndex"];
        let owner_node_indices = content_document_index["index"].as_array()?;
        let document_indices = content_document_index["value"].as_array()?;
        let owner_entry = owner_node_indices
            .iter()
            .position(|index| index.as_u64() == Some(owner_node_index as u64))?;
        document_indices[owner_entry]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
    }

    fn assert_all_backend_node_ids_are_renderer_owned(result: &serde_json::Value) {
        let documents = result["documents"].as_array().expect("documents");
        for (document_index, document) in documents.iter().enumerate() {
            let backend_node_ids = document["nodes"]["backendNodeId"]
                .as_array()
                .unwrap_or_else(|| {
                    panic!("document {document_index} should include backendNodeId table")
                });
            assert!(
                !backend_node_ids.is_empty(),
                "document {document_index} should include at least one backendNodeId"
            );
            for (node_index, value) in backend_node_ids.iter().enumerate() {
                let backend_node_id = value
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .unwrap_or_else(|| {
                        panic!(
                            "document {document_index} node {node_index} should have u32 backendNodeId: {value}"
                        )
                    });
                assert!(
                    moli_core::page::is_renderer_backend_node_id(backend_node_id),
                    "document {document_index} node {node_index} should use renderer backend id namespace: {backend_node_id}"
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enable_and_disable_return_empty_result_without_browser_context() {
        let mut ctx = TestContext::new();

        ctx.process_async(json!({"id": 1, "method": "DOMSnapshot.enable"}))
            .await;
        ctx.process_async(json!({"id": 2, "method": "DOMSnapshot.disable"}))
            .await;

        ctx.expect_result(1, json!({}), None);
        ctx.expect_result(2, json!({}), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_returns_string_table_backed_node_and_layout_tables() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><html><head><title>Snapshot Title</title>\
             <style>#target{display:block;color:rgb(10,20,30)}\
             #inline{display:inline;color:rgb(40,50,60)}.hidden{display:none}</style></head>\
             <body><div id='target' data-state='ready'><span id='inline'>hello</span></div>\
             <p class='hidden'>hidden</p></body></html>",
        )
        .await;

        ctx.process_async(json!({
            "id": 10,
            "method": "DOMSnapshot.captureSnapshot",
            "params": {
                "computedStyles": ["display", "color"],
                "includePaintOrder": true,
                "includeDOMRects": true,
                "includeBlendedBackgroundColors": true,
                "includeTextColorOpacities": true
            }
        }))
        .await;

        let result = take_capture_snapshot(&mut ctx, 10);
        let strings = strings(&result);
        let document = &result["documents"][0];
        assert_eq!(
            decode_string(&strings, &document["title"]),
            "Snapshot Title"
        );
        assert_eq!(decode_string(&strings, &document["encodingName"]), "UTF-8");
        assert_eq!(decode_string(&strings, &document["frameId"]), "TID-1");

        let node_names = decoded_node_names(&result);
        for expected in [
            "#document",
            "HTML",
            "HEAD",
            "TITLE",
            "BODY",
            "DIV",
            "SPAN",
            "#text",
        ] {
            assert!(
                node_names.iter().any(|name| name == expected),
                "missing node {expected}; names={node_names:?}"
            );
        }

        let div_index = node_names
            .iter()
            .position(|name| name == "DIV")
            .expect("div node");
        let span_index = node_names
            .iter()
            .position(|name| name == "SPAN")
            .expect("span node");
        let hidden_index = node_names
            .iter()
            .position(|name| name == "P")
            .expect("hidden paragraph node");
        assert_eq!(
            document["nodes"]["parentIndex"][span_index].as_i64(),
            Some(div_index as i64)
        );

        let div_attributes = document["nodes"]["attributes"][div_index]
            .as_array()
            .expect("div attributes");
        let decoded_attributes = div_attributes
            .iter()
            .map(|index| decode_string(&strings, index))
            .collect::<Vec<_>>();
        assert!(
            decoded_attributes
                .windows(2)
                .any(|pair| pair == ["id", "target"])
        );
        assert!(
            decoded_attributes
                .windows(2)
                .any(|pair| pair == ["data-state", "ready"])
        );

        let node_count = document["nodes"]["nodeName"].as_array().unwrap().len();
        assert_eq!(
            document["nodes"]["backendNodeId"].as_array().unwrap().len(),
            node_count
        );
        let layout = &document["layout"];
        let layout_count = layout["nodeIndex"].as_array().unwrap().len();
        assert!(layout_count > 0);
        assert_eq!(layout["styles"].as_array().unwrap().len(), layout_count);
        assert_eq!(layout["bounds"].as_array().unwrap().len(), layout_count);
        assert_eq!(layout["text"].as_array().unwrap().len(), layout_count);
        assert_eq!(
            decoded_layout_style_for_node(&result, 0, 0),
            Vec::<String>::new()
        );
        assert_eq!(
            decoded_layout_style_for_node(&result, 0, div_index),
            ["block", "rgb(10, 20, 30)"]
        );
        assert_eq!(
            decoded_layout_style_for_node(&result, 0, span_index),
            ["inline", "rgb(40, 50, 60)"]
        );
        assert_eq!(
            decoded_layout_style_for_node(&result, 0, hidden_index),
            ["none", "rgb(0, 0, 0)"]
        );
        for bounds in layout["bounds"].as_array().unwrap() {
            assert_eq!(bounds, &json!([0.0, 0.0, 1.0, 1.0]));
        }
        assert_eq!(
            layout["paintOrders"].as_array().unwrap().len(),
            layout_count
        );
        assert_eq!(
            layout["stackingContexts"]["index"],
            json!([0]),
            "DOMSnapshot stackingContexts is RareBooleanData keyed by layout index"
        );
        assert_eq!(
            layout["offsetRects"].as_array().unwrap().len(),
            layout_count
        );
        assert_eq!(
            layout["scrollRects"].as_array().unwrap().len(),
            layout_count
        );
        assert_eq!(
            layout["clientRects"].as_array().unwrap().len(),
            layout_count
        );
        for rects in ["offsetRects", "scrollRects", "clientRects"] {
            for rect in layout[rects].as_array().unwrap() {
                assert_eq!(rect, &json!([0.0, 0.0, 1.0, 1.0]));
            }
        }
        assert_eq!(
            layout["blendedBackgroundColors"].as_array().unwrap().len(),
            layout_count
        );
        assert_eq!(
            layout["textColorOpacities"].as_array().unwrap().len(),
            layout_count
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_dispatch_returns_resolved_styles_with_lightweight_geometry() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<html><head><title>Dispatch Snapshot</title></head><body><section style='display:inline'>ok</section></body></html>",
        )
        .await;

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 110,
                "method": "DOMSnapshot.captureSnapshot",
                "params": { "computedStyles": ["display"], "includeDOMRects": true }
            }),
        )
        .await;

        let result = take_capture_snapshot(&mut ctx, 110);
        let strings = strings(&result);
        assert_eq!(
            decode_string(&strings, &result["documents"][0]["title"]),
            "Dispatch Snapshot"
        );
        let layout = &result["documents"][0]["layout"];
        let layout_count = layout["nodeIndex"].as_array().unwrap().len();
        assert!(layout_count > 0);
        assert_eq!(layout["styles"].as_array().unwrap().len(), layout_count);
        let node_names = decoded_node_names(&result);
        let section_index = node_names
            .iter()
            .position(|name| name == "SECTION")
            .expect("section node");
        assert_eq!(
            decoded_layout_style_for_node(&result, 0, section_index),
            ["inline"]
        );
        assert_eq!(
            layout["clientRects"].as_array().unwrap().len(),
            layout_count
        );
        for bounds in layout["bounds"].as_array().unwrap() {
            assert_eq!(bounds, &json!([0.0, 0.0, 1.0, 1.0]));
        }
        assert_eq!(ctx.sent, Vec::<serde_json::Value>::new());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_dispatch_starts_renderer_live_snapshot_command() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<html><head><title>Pending Snapshot</title></head><body><article>live</article></body></html>",
        )
        .await;

        let raw = json!({
            "id": 120,
            "method": "DOMSnapshot.captureSnapshot",
            "params": { "computedStyles": ["display"] }
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("DOMSnapshot.captureSnapshot should await renderer live DOM capture");
        assert_eq!(pending.kind_name(), "DOMSnapshot");

        let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
        assert_eq!(
            messages.len(),
            1,
            "unexpected DOMSnapshot output: {messages:?}"
        );
        let response = &messages[0];
        assert_eq!(response["id"], json!(120));
        let result = &response["result"];
        let strings = strings(result);
        assert_eq!(
            decode_string(&strings, &result["documents"][0]["title"]),
            "Pending Snapshot"
        );
        assert!(
            decoded_node_names(result)
                .iter()
                .any(|name| name == "ARTICLE")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_backend_node_ids_use_renderer_registry() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<html><head><title>Backend Snapshot</title></head><body><article id='target'>live</article></body></html>",
        )
        .await;

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 130,
                "method": "DOMSnapshot.captureSnapshot",
                "params": { "computedStyles": [] }
            }),
        )
        .await;

        let result = take_capture_snapshot(&mut ctx, 130);
        assert_all_backend_node_ids_are_renderer_owned(&result);
        let node_names = decoded_node_names(&result);
        let article_index = node_names
            .iter()
            .position(|name| name == "ARTICLE")
            .unwrap_or_else(|| panic!("missing ARTICLE node in DOMSnapshot: {node_names:?}"));
        let backend_node_id = result["documents"][0]["nodes"]["backendNodeId"][article_index]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .expect("ARTICLE should have u32 backendNodeId");
        assert!(
            moli_core::page::is_renderer_backend_node_id(backend_node_id),
            "DOMSnapshot should use renderer backend id namespace for live nodes: {backend_node_id}"
        );

        ctx.process_async(json!({
            "id": 131,
            "method": "DOM.resolveNode",
            "params": { "backendNodeId": backend_node_id }
        }))
        .await;
        let resolved = ctx.take_response_by_id(131);
        assert_eq!(resolved["result"]["object"]["subtype"], json!("node"));
        let object_id = resolved["result"]["object"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("resolved backend node should return objectId: {resolved}"));

        ctx.process_async(json!({
            "id": 132,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return [this.localName, this.id, this.textContent].join('|'); }",
                "returnByValue": true
            }
        }))
        .await;
        let checked = ctx.take_response_by_id(132);
        assert_eq!(
            checked["result"]["result"]["value"],
            json!("article|target|live")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_projects_generated_marker_with_host_parent_and_pseudo_type() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><html><head><style>\
             #marker { display: list-item; }\
             #suppressed { display: list-item; list-style: none; }\
             </style></head><body>\
             <div id='marker'>marker</div>\
             <div id='suppressed'>suppressed</div>\
             </body></html>",
        )
        .await;

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 133,
                "method": "DOMSnapshot.captureSnapshot",
                "params": { "computedStyles": [] }
            }),
        )
        .await;

        let result = take_capture_snapshot(&mut ctx, 133);
        let strings = strings(&result);
        let node_names = decoded_node_names(&result);
        let host_index = node_names
            .iter()
            .position(|name| name == "DIV")
            .expect("marker host");
        let marker_indices = node_names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| (name == "::marker").then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(
            marker_indices.len(),
            1,
            "list-style:none host must not add a marker: {node_names:?}"
        );
        let marker_index = marker_indices[0];
        let nodes = &result["documents"][0]["nodes"];
        assert_eq!(
            nodes["parentIndex"][marker_index],
            json!(host_index),
            "DOMSnapshot associates the inspector-only marker with its originating element"
        );

        let pseudo_type = &nodes["pseudoType"];
        let pseudo_entry = pseudo_type["index"]
            .as_array()
            .expect("pseudoType indices")
            .iter()
            .position(|index| index.as_u64() == Some(marker_index as u64))
            .expect("marker pseudoType entry");
        assert_eq!(
            decode_string(&strings, &pseudo_type["value"][pseudo_entry]),
            "marker"
        );
        assert_ne!(
            nodes["backendNodeId"][marker_index], nodes["backendNodeId"][host_index],
            "DOMSnapshot marker backend identity must not alias the host"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_excludes_text_control_user_agent_shadow_tree() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><html><body><input id='input' value='alpha'></body></html>",
        )
        .await;

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 134,
                "method": "DOMSnapshot.captureSnapshot",
                "params": { "computedStyles": [] }
            }),
        )
        .await;

        let result = take_capture_snapshot(&mut ctx, 134);
        let node_names = decoded_node_names(&result);
        assert!(
            node_names.iter().any(|name| name == "INPUT"),
            "author control should remain in DOMSnapshot: {node_names:?}"
        );
        assert!(
            node_names.iter().all(|name| name != "#document-fragment"),
            "Chromium DOMSnapshot does not include text-control UA shadow DOM: {node_names:?}"
        );
        assert!(
            result["documents"][0]["nodes"]
                .get("shadowRootType")
                .is_none_or(|value| {
                    value["index"]
                        .as_array()
                        .is_none_or(|indices| indices.is_empty())
                }),
            "DOMSnapshot should not label any UA shadow node: {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_includes_live_child_frame_documents() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><html><head><title>Parent Snapshot</title></head><body>\
             <iframe id='child' srcdoc=\"<html><head><title>Child Snapshot</title></head>\
             <body><main id='inside' style='display:inline;color:rgb(70,80,90)'>child live</main></body></html>\"></iframe>\
             </body></html>",
        )
        .await;

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 140,
                "method": "DOMSnapshot.captureSnapshot",
                "params": { "computedStyles": ["display", "color"] }
            }),
        )
        .await;

        let result = take_capture_snapshot(&mut ctx, 140);
        assert_all_backend_node_ids_are_renderer_owned(&result);
        let strings = strings(&result);
        let documents = result["documents"].as_array().expect("documents");
        assert!(
            documents.len() >= 2,
            "DOMSnapshot should include child frame document tables: {result}"
        );

        let child_index = documents
            .iter()
            .position(|document| decode_string(&strings, &document["title"]) == "Child Snapshot")
            .unwrap_or_else(|| panic!("missing child document in DOMSnapshot: {result}"));
        let child_document = &documents[child_index];
        assert_ne!(
            decode_string(&strings, &child_document["frameId"]),
            "TID-1",
            "child document should keep its own frame id"
        );

        let parent_index = documents
            .iter()
            .position(|document| decode_string(&strings, &document["title"]) == "Parent Snapshot")
            .unwrap_or_else(|| panic!("missing parent document in DOMSnapshot: {result}"));
        let parent_document = &documents[parent_index];
        let parent_node_names = decoded_node_names_for_document(&result, parent_index);
        let iframe_index = parent_node_names
            .iter()
            .position(|name| name == "IFRAME")
            .unwrap_or_else(|| panic!("missing parent IFRAME node: {parent_node_names:?}"));
        let content_document_index = &parent_document["nodes"]["contentDocumentIndex"];
        let owner_node_indices = content_document_index["index"]
            .as_array()
            .expect("contentDocumentIndex.index");
        let document_indices = content_document_index["value"]
            .as_array()
            .expect("contentDocumentIndex.value");
        let owner_entry = owner_node_indices
            .iter()
            .position(|index| index.as_u64() == Some(iframe_index as u64))
            .unwrap_or_else(|| {
                panic!("missing iframe contentDocumentIndex entry: {content_document_index}")
            });
        assert_eq!(
            document_indices[owner_entry].as_u64(),
            Some(child_index as u64),
            "iframe contentDocumentIndex should point at the child document table"
        );

        let child_node_names = decoded_node_names_for_document(&result, child_index);
        let main_index = child_node_names
            .iter()
            .position(|name| name == "MAIN")
            .unwrap_or_else(|| panic!("missing child MAIN node: {child_node_names:?}"));
        let child_backend_node_id = child_document["nodes"]["backendNodeId"][main_index]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .expect("child MAIN should have u32 backendNodeId");
        assert!(
            moli_core::page::is_renderer_backend_node_id(child_backend_node_id),
            "child DOMSnapshot nodes should use renderer backend ids: {child_backend_node_id}"
        );
        assert_eq!(
            decoded_layout_style_for_node(&result, child_index, main_index),
            ["inline", "rgb(70, 80, 90)"]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_includes_detached_child_frame_documents() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><html><head><title>Parent Detached Snapshot</title></head><body></body></html>",
        )
        .await;

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 141,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"(() => {
                      const frame = document.createElement("iframe");
                      frame.id = "detached-child";
                      frame.setAttribute("srcdoc", `<!doctype html><html><head><title>Detached Snapshot</title></head><body><main id="detached-inside">detached</main><iframe id="nested-detached" srcdoc="<!doctype html><html><head><title>Nested Detached Snapshot</title></head><body><section id='nested-inside'>nested</section></body></html>"></iframe></body></html>`);
                      document.body.appendChild(frame);
                      return frame.isConnected;
                    })()"#,
                    "returnByValue": true
                }
            }),
        )
        .await;
        assert_eq!(
            ctx.take_response_by_id(141)["result"]["result"]["value"],
            json!(true)
        );
        wait_until_runtime_value(
            &mut ctx,
            143,
            "detached nested srcdoc documents to commit",
            r#"(() => {
              const frame = document.getElementById("detached-child");
              const child = frame && frame.contentDocument;
              const nested = child && child.getElementById("nested-detached");
              const nestedDocument = nested && nested.contentDocument;
              return JSON.stringify({
                childTitle: child && child.title,
                childMain: !!(child && child.getElementById("detached-inside")),
                nestedTitle: nestedDocument && nestedDocument.title,
                nestedSection: !!(nestedDocument && nestedDocument.getElementById("nested-inside"))
              });
            })()"#,
            json!(
                r#"{"childTitle":"Detached Snapshot","childMain":true,"nestedTitle":"Nested Detached Snapshot","nestedSection":true}"#
            ),
        )
        .await;

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 142,
                "method": "DOMSnapshot.captureSnapshot",
                "params": { "computedStyles": ["display"] }
            }),
        )
        .await;

        let result = take_capture_snapshot(&mut ctx, 142);
        assert_all_backend_node_ids_are_renderer_owned(&result);
        let strings = strings(&result);
        let documents = result["documents"].as_array().expect("documents");
        let parent_index = documents
            .iter()
            .position(|document| {
                decode_string(&strings, &document["title"]) == "Parent Detached Snapshot"
            })
            .unwrap_or_else(|| panic!("missing parent detached snapshot document: {result}"));
        let child_index = documents
            .iter()
            .position(|document| decode_string(&strings, &document["title"]) == "Detached Snapshot")
            .unwrap_or_else(|| panic!("missing detached child document: {result}"));
        let nested_index = documents
            .iter()
            .position(|document| {
                decode_string(&strings, &document["title"]) == "Nested Detached Snapshot"
            })
            .unwrap_or_else(|| panic!("missing nested detached child document: {result}"));

        let parent_node_names = decoded_node_names_for_document(&result, parent_index);
        let parent_iframe_index = parent_node_names
            .iter()
            .position(|name| name == "IFRAME")
            .unwrap_or_else(|| panic!("missing parent iframe node: {parent_node_names:?}"));
        assert_eq!(
            content_document_index_for_owner_node(&result, parent_index, parent_iframe_index),
            Some(child_index),
            "parent iframe should point at detached child document table"
        );

        let child_node_names = decoded_node_names_for_document(&result, child_index);
        assert!(
            child_node_names.iter().any(|name| name == "MAIN"),
            "detached child table should include parsed child DOM: {child_node_names:?}"
        );
        let detached_main_index = child_node_names
            .iter()
            .position(|name| name == "MAIN")
            .expect("detached MAIN node");
        assert_eq!(
            decoded_layout_style_for_node(&result, child_index, detached_main_index),
            ["block"],
            "the retained dynamic child document should use its live style owner"
        );
        let nested_iframe_index = child_node_names
            .iter()
            .position(|name| name == "IFRAME")
            .unwrap_or_else(|| panic!("missing nested iframe node: {child_node_names:?}"));
        assert_eq!(
            content_document_index_for_owner_node(&result, child_index, nested_iframe_index),
            Some(nested_index),
            "nested iframe should use the detached parent document's local owner id namespace"
        );

        let nested_node_names = decoded_node_names_for_document(&result, nested_index);
        assert!(
            nested_node_names.iter().any(|name| name == "SECTION"),
            "nested detached table should include parsed nested DOM: {nested_node_names:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_targets_loaded_background_owner_without_promotion() {
        let mut ctx = TestContext::new();
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<html><head><title>Background Snapshot</title></head><body><main>owner</main></body></html>",
            )
            .await
            .expect("background page should load");

        let mut background = PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            page.final_url().as_str().to_owned(),
        );
        background.replace_loaded_page(Some(page));

        let mut bc = BrowserContext::new("BID-DS-BG".to_owned());
        bc.set_active_target_id("TID-active".to_owned());
        bc.attach_active_session("SID-active".to_owned());
        bc.insert_page_target_host(background);
        ctx.conn.browser_context = Some(bc);

        ctx.process_async(json!({
            "id": 101,
            "sessionId": "SID-background",
            "method": "DOMSnapshot.captureSnapshot",
            "params": { "computedStyles": ["display"] }
        }))
        .await;

        let response = ctx.take_response_by_id(101);
        assert_eq!(response["sessionId"], "SID-background");
        let result = response["result"].clone();
        let strings = strings(&result);
        let document = &result["documents"][0];
        assert_eq!(
            decode_string(&strings, &document["title"]),
            "Background Snapshot"
        );
        assert_eq!(
            decode_string(&strings, &document["frameId"]),
            "TID-background"
        );
        assert!(
            decoded_node_names(&result)
                .iter()
                .any(|name| name == "MAIN")
        );
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .and_then(BrowserContext::active_target_id),
            Some("TID-active")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_snapshot_targets_inactive_owner_without_activation() {
        let mut ctx = TestContext::new();
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<html><head><title>Inactive Snapshot</title></head><body><section>inactive</section></body></html>",
            )
            .await
            .expect("inactive page should load");

        let mut active = BrowserContext::new("BID-active".to_owned());
        active.set_active_target_id("TID-active".to_owned());
        active.attach_active_session("SID-active".to_owned());
        ctx.conn.browser_context = Some(active);

        let mut inactive = BrowserContext::new("BID-inactive".to_owned());
        inactive.set_active_target_id("TID-inactive".to_owned());
        inactive.set_target_url(page.final_url().as_str().to_owned());
        inactive.attach_active_session("SID-inactive".to_owned());
        inactive.replace_loaded_page(Some(page));
        ctx.conn.inactive_browser_contexts.push(inactive);

        ctx.process_async(json!({
            "id": 111,
            "sessionId": "SID-inactive",
            "method": "DOMSnapshot.captureSnapshot",
            "params": { "computedStyles": [] }
        }))
        .await;

        let response = ctx.take_response_by_id(111);
        assert_eq!(response["sessionId"], "SID-inactive");
        let result = response["result"].clone();
        let strings = strings(&result);
        let document = &result["documents"][0];
        assert_eq!(
            decode_string(&strings, &document["title"]),
            "Inactive Snapshot"
        );
        assert_eq!(
            decode_string(&strings, &document["frameId"]),
            "TID-inactive"
        );
        assert_eq!(
            ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
            Some("BID-active")
        );
    }

    #[tokio::test]
    async fn capture_snapshot_reports_no_document_without_loaded_page() {
        let mut ctx = TestContext::new();
        ctx.conn.browser_context = Some(BrowserContext::new("BID-1".to_owned()));

        ctx.process_async(json!({
            "id": 11,
            "method": "DOMSnapshot.captureSnapshot",
            "params": { "computedStyles": [] }
        }))
        .await;

        ctx.expect_error(11, -32000, "NoDocumentLoaded");
    }

    #[tokio::test]
    async fn capture_snapshot_uses_fresh_initial_document_without_adapter() {
        let mut ctx = TestContext::new();

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 111,
                "method": "Target.createTarget",
                "params": { "url": "about:blank" }
            }),
        )
        .await;

        ctx.expect_event("Target.targetCreated", None);
        let create_response = ctx.take_response_by_id(111);
        assert!(
            create_response["result"]["targetId"].as_str().is_some(),
            "Target.createTarget should return target id: {create_response}"
        );

        process_via_command_dispatch(
            &mut ctx,
            json!({
                "id": 112,
                "method": "DOMSnapshot.captureSnapshot",
                "params": { "computedStyles": [] }
            }),
        )
        .await;

        let response = ctx.take_response_by_id(112);
        let result = response["result"].clone();
        let strings = strings(&result);
        let document = &result["documents"][0];
        assert_eq!(
            decode_string(&strings, &document["documentURL"]),
            "about:blank"
        );
        assert_eq!(
            decode_string(&strings, &document["frameId"]),
            create_response["result"]["targetId"]
                .as_str()
                .expect("target id")
        );
        assert!(
            decoded_node_names(&result)
                .iter()
                .any(|name| name == "#document")
        );
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_target
                .runtime_slot
                .has_loaded_page(),
            "Target.createTarget should install the initial about:blank page before DOMSnapshot"
        );
    }
}
