use crate::conn::{BrowserContext, CdpCommandTaskStep, CdpSchedulerEvent, PageTargetHost};
use crate::domains::page::LOADER_ID;
use crate::testing::{
    TestContext, wait_until_renderer_document_load, wait_until_scheduler_message,
};
use serde_json::{Value, json};

async fn complete_pending_command_task_for_test(
    ctx: &mut TestContext,
    mut pending: crate::conn::PendingCdpCommandDispatch,
) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
    loop {
        let completed = pending.wait().await;
        match ctx.conn.complete_pending_command_dispatch(completed).await {
            CdpCommandTaskStep::Pending(next) => pending = *next,
            CdpCommandTaskStep::Complete(outcome) => return outcome.into_parts(),
        }
    }
}

async fn complete_command_task_for_test(
    ctx: &mut TestContext,
    step: CdpCommandTaskStep,
) -> Vec<Value> {
    match step {
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
        CdpCommandTaskStep::Pending(pending) => {
            complete_pending_command_task_for_test(ctx, *pending)
                .await
                .0
        }
    }
}

async fn load_page_async(ctx: &mut TestContext, html: &str) {
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    let data_url = format!("data:text/html,{html}");
    ctx.conn.browser_context = Some(bc);
    ctx.enable_page_events_for_test(None);
    ctx.install_navigation_fixture_for_session_owner(&data_url, None)
        .await;
    wait_until_renderer_document_load(ctx, None, "TID-1", LOADER_ID).await;
    wait_until_scheduler_message(ctx, "accessibility fixture load output", |message| {
        message["method"] == json!("Page.loadEventFired")
    })
    .await;
    ctx.sent.clear();
}

async fn complete_child_frame_lifecycle(ctx: &mut TestContext) {
    let owner = crate::conn::CommandOwnerScope::capture(&ctx.conn, None);
    let pending = ctx
        .conn
        .start_child_frame_lifecycle_work_for_owner(owner, std::time::Duration::from_secs(2))
        .expect("loaded page should expose child-frame lifecycle work");
    let completed = pending
        .wait()
        .await
        .expect("child-frame lifecycle work should complete");
    assert!(
        ctx.conn
            .complete_child_frame_lifecycle_work_for_session_owner(completed)
            .expect("child-frame lifecycle completion should apply"),
        "child-frame lifecycle should settle before inspecting the nested frame tree"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn accessibility_top_frame_snapshot_commands_use_explicit_dispatch() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button id='go'>Go</button></body></html>",
    )
    .await;
    ctx.sent.clear();

    let full_tree_raw = json!({
        "id": 1,
        "method": "Accessibility.getFullAXTree"
    })
    .to_string();
    let full_tree_step = ctx.conn.start_command_dispatch(&full_tree_raw);
    let full_tree = complete_command_task_for_test(&mut ctx, full_tree_step).await;
    let nodes = full_tree[0]["result"]["nodes"]
        .as_array()
        .expect("nodes array");
    let button = find_ax_node(nodes, "button", "Go");
    let button_ax_id = button["nodeId"].as_str().expect("button AX id").to_owned();
    let button_backend_id = renderer_backend_dom_node_id(button);

    for (id, method, params) in [
        (
            2,
            "Accessibility.getRootAXNode",
            json!({ "frameId": "TID-1" }),
        ),
        (
            3,
            "Accessibility.getChildAXNodes",
            json!({ "id": button_ax_id }),
        ),
        (
            4,
            "Accessibility.getAXNodeAndAncestors",
            json!({ "backendNodeId": button_backend_id }),
        ),
        (
            5,
            "Accessibility.queryAXTree",
            json!({
                "backendNodeId": button_backend_id,
                "role": "button",
                "accessibleName": "Go"
            }),
        ),
        (
            6,
            "Accessibility.getPartialAXTree",
            json!({
                "backendNodeId": button_backend_id,
                "fetchRelatives": false
            }),
        ),
    ] {
        let raw = json!({
            "id": id,
            "method": method,
            "params": params
        })
        .to_string();
        let step = ctx.conn.start_command_dispatch(&raw);
        let messages = complete_command_task_for_test(&mut ctx, step).await;
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], id);
        assert!(
            messages[0].get("result").is_some(),
            "{method} should complete successfully: {:?}",
            messages[0]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn accessibility_get_full_tree_rejects_while_main_document_navigation_is_pending() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>previous</button></body></html>",
    )
    .await;
    ctx.sent.clear();
    let browser_context = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context should exist");
    browser_context.attach_active_session("SID-1".to_owned());
    browser_context
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("active navigation should start");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 50,
        "method": "Accessibility.getFullAXTree",
        "sessionId": "SID-1"
    }))
    .await;

    ctx.expect_error(50, -32000, "Navigation is changing the document");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_reads_live_renderer_dom_when_page_snapshot_is_stale() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button id='target'>old</button></body></html>",
    )
    .await;

    let mutation_completion = {
        let page = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_page_target_mut()
            .runtime_slot
            .loaded_page_mut()
            .expect("loaded page");
        let mutation = json!({
            "id": 910,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "document.getElementById('target').textContent = 'live'; 'done';",
                "returnByValue": true
            }
        });
        let pending = page
            .start_runtime_protocol_message(mutation.to_string())
            .expect("runtime mutation should start");
        pending
            .wait()
            .await
            .expect("runtime mutation should complete")
    };

    ctx.process_async(json!({
        "id": 911,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes
            .iter()
            .any(|node| node["name"]["value"] == json!("live"))
    );
    assert!(
        !nodes
            .iter()
            .any(|node| node["name"]["value"] == json!("old"))
    );

    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .loaded_page_mut()
        .expect("loaded page");
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

async fn enable_runtime_async(ctx: &mut TestContext) {
    ctx.process_async(json!({
        "id": 900,
        "method": "Runtime.enable"
    }))
    .await;
    let _ = ctx.take_all();
}

async fn child_frame_id_for_single_iframe_async(ctx: &mut TestContext) -> String {
    ctx.process_async(json!({
        "id": 901,
        "method": "Page.getFrameTree"
    }))
    .await;
    let message = ctx.take_one();
    message["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned()
}

async fn dom_document_node_id_async(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    command_id: u64,
) -> u32 {
    let mut command = json!({
        "id": command_id,
        "method": "DOM.getDocument"
    });
    if let Some(session_id) = session_id {
        command["sessionId"] = json!(session_id);
    }
    ctx.process_async(command).await;
    let response = ctx.take_response_by_id(command_id);
    response["result"]["root"]["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("DOM.getDocument should return a frontend nodeId: {response}"))
}

async fn child_frame_renderer_backend_node_id_for_selector_async(
    ctx: &mut TestContext,
    child_frame_id: &str,
    selector: &str,
    base_id: u64,
) -> u32 {
    child_frame_renderer_dom_node_ids_for_selector_async(ctx, child_frame_id, selector, base_id)
        .await
        .backend_node_id
}

struct ChildFrameRendererDomNodeIds {
    frontend_node_id: u32,
    backend_node_id: u32,
}

async fn child_frame_renderer_dom_node_ids_for_selector_async(
    ctx: &mut TestContext,
    child_frame_id: &str,
    selector: &str,
    base_id: u64,
) -> ChildFrameRendererDomNodeIds {
    ctx.process_async(json!({
        "id": base_id,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": child_frame_id, "worldName": format!("ax-backend-{base_id}") }
    }))
    .await;
    let context_id = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(base_id))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .expect("child isolated world id");

    ctx.process_async(json!({
        "id": base_id + 1,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "expression": format!("document.querySelector({selector:?})")
        }
    }))
    .await;
    let object_id = ctx.take_response_by_id(base_id + 1)["result"]["result"]["objectId"]
        .as_str()
        .expect("child element object id")
        .to_owned();

    ctx.process_async(json!({
        "id": base_id + 2,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id, "depth": 0 }
    }))
    .await;
    let described = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(base_id + 2))
        .expect("describeNode response");
    let frontend_node_id = described["result"]["node"]["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("describeNode should return nodeId: {described}"));
    let backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("describeNode should return backendNodeId: {described}"));
    assert_ne!(
        frontend_node_id, 0,
        "describeNode should return a nonzero frontend nodeId: {described}"
    );
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "child describeNode should return renderer backend id: {described}"
    );
    ChildFrameRendererDomNodeIds {
        frontend_node_id,
        backend_node_id,
    }
}

async fn nested_child_frame_id_for_single_nested_iframe_async(ctx: &mut TestContext) -> String {
    ctx.process_async(json!({
        "id": 902,
        "method": "Page.getFrameTree"
    }))
    .await;
    let message = ctx.take_one();
    message["result"]["frameTree"]["childFrames"][0]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("nested child frame id")
        .to_owned()
}

fn find_ax_node<'a>(nodes: &'a [Value], role: &str, name: &str) -> &'a Value {
    nodes
        .iter()
        .find(|node| node["role"]["value"] == role && node["name"]["value"] == name)
        .unwrap_or_else(|| panic!("expected AX node role={role} name={name} in {nodes:?}"))
}

fn renderer_backend_dom_node_id(node: &Value) -> u32 {
    let backend_node_id = node["backendDOMNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("expected backendDOMNodeId in AX node: {node}"));
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "AX backendDOMNodeId should be renderer-owned: {node}"
    );
    backend_node_id
}

fn renderer_backed_ax_node_id(node: &Value) -> String {
    let backend_node_id = renderer_backend_dom_node_id(node);
    let node_id = node["nodeId"]
        .as_str()
        .unwrap_or_else(|| panic!("expected AX nodeId in AX node: {node}"));
    assert_eq!(
        node_id,
        format!("AX-{backend_node_id}"),
        "DOM-backed AX nodeId should use renderer backend identity: {node}"
    );
    node_id.to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn accessibility_loaded_page_methods_target_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-A".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><body><p>Intro</p><button>Owner</button></body></html>",
        Some("SID-background"),
    )
    .await;

    ctx.process_async(json!({
        "id": 201,
        "sessionId": "SID-background",
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let full_tree = ctx.take_one();
    assert_eq!(full_tree["sessionId"], "SID-background");
    let nodes = full_tree["result"]["nodes"]
        .as_array()
        .expect("nodes array");
    let button = find_ax_node(nodes, "button", "Owner");
    let button_ax_id = renderer_backed_ax_node_id(button);
    let button_backend_id = renderer_backend_dom_node_id(button);
    let document_node_id = dom_document_node_id_async(&mut ctx, Some("SID-background"), 1205).await;

    ctx.process_async(json!({
        "id": 202,
        "sessionId": "SID-background",
        "method": "Accessibility.getRootAXNode"
    }))
    .await;
    let root = ctx.take_response_by_id(202);
    assert_eq!(root["sessionId"], "SID-background");
    assert_eq!(root["result"]["node"]["role"]["value"], "RootWebArea");

    ctx.process_async(json!({
        "id": 203,
        "sessionId": "SID-background",
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": button_ax_id }
    }))
    .await;
    let children = ctx.take_response_by_id(203);
    assert_eq!(children["sessionId"], "SID-background");
    assert!(
        children["result"]["nodes"]
            .as_array()
            .expect("button child nodes")
            .iter()
            .any(|node| node["role"]["value"] == "StaticText" && node["name"]["value"] == "Owner")
    );

    ctx.process_async(json!({
        "id": 204,
        "sessionId": "SID-background",
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "backendNodeId": button_backend_id }
    }))
    .await;
    let chain = ctx.take_response_by_id(204);
    assert_eq!(chain["sessionId"], "SID-background");
    assert_eq!(
        chain["result"]["nodes"]
            .as_array()
            .expect("ancestor chain")
            .first()
            .expect("inspected node")["nodeId"],
        button_ax_id
    );

    ctx.process_async(json!({
        "id": 205,
        "sessionId": "SID-background",
        "method": "Accessibility.queryAXTree",
        "params": {
            "nodeId": document_node_id,
            "role": "button",
            "accessibleName": "Owner"
        }
    }))
    .await;
    let query = ctx.take_response_by_id(205);
    assert_eq!(query["sessionId"], "SID-background");
    assert_eq!(query["result"]["nodes"][0]["nodeId"], button_ax_id);

    ctx.process_async(json!({
        "id": 206,
        "sessionId": "SID-background",
        "method": "Accessibility.getPartialAXTree",
        "params": {
            "backendNodeId": button_backend_id,
            "fetchRelatives": false
        }
    }))
    .await;
    let partial = ctx.take_response_by_id(206);
    assert_eq!(partial["sessionId"], "SID-background");
    assert_eq!(partial["result"]["nodes"][0]["nodeId"], button_ax_id);

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some("TID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn accessibility_loaded_page_methods_target_inactive_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut active = BrowserContext::new("BID-active".to_owned());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active".to_owned());
    ctx.conn.browser_context = Some(active);

    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.set_target_url("about:blank".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<html><body><button>Inactive</button></body></html>",
        Some("SID-inactive"),
    )
    .await;

    ctx.process_async(json!({
        "id": 211,
        "sessionId": "SID-inactive",
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let full_tree = ctx.take_one();
    assert_eq!(full_tree["sessionId"], "SID-inactive");
    let nodes = full_tree["result"]["nodes"]
        .as_array()
        .expect("nodes array");
    assert_eq!(
        find_ax_node(nodes, "button", "Inactive")["name"]["value"],
        "Inactive"
    );

    ctx.process_async(json!({
        "id": 212,
        "sessionId": "SID-inactive",
        "method": "Accessibility.getRootAXNode"
    }))
    .await;
    let root = ctx.take_one();
    assert_eq!(root["sessionId"], "SID-inactive");
    assert_eq!(root["result"]["node"]["role"]["value"], "RootWebArea");
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-active")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 1, "method": "Accessibility.getFullAXTree"}))
        .await;
    ctx.expect_error(1, -31998, "BrowserContextNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_requires_loaded_page() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({"id": 2, "method": "Accessibility.getFullAXTree"}))
        .await;
    ctx.expect_error(2, -32000, "NoDocumentLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_uses_fresh_initial_document_without_adapter() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 20,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    ctx.expect_event("Target.targetCreated", None);
    let create_response = ctx.take_response_by_id(20);
    assert!(
        create_response["result"]["targetId"].as_str().is_some(),
        "Target.createTarget should return target id: {create_response}"
    );

    ctx.process_async(json!({
        "id": 21,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let response = ctx.take_response_by_id(21);
    let nodes = response["result"]["nodes"].as_array().expect("nodes array");
    assert!(!nodes.is_empty());
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .runtime_slot
            .has_loaded_page(),
        "Target.createTarget should install the initial about:blank page before Accessibility"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_rejects_foreign_frame() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 3,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": "TID-OTHER" }
    }))
    .await;
    ctx.expect_error(
        3,
        -32000,
        "Frame with the given id does not belong to the target.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_accepts_matching_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 33,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": "TID-1" }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 33);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(!nodes.is_empty());
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_returns_child_frame_tree_for_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><main><button>Go</button></main></body></html>\"></iframe></body></html>",
    ).await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 331,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(!nodes.is_empty());
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
    renderer_backend_dom_node_id(&nodes[0]);
    assert!(
        nodes
            .iter()
            .any(|node| { node["role"]["value"] == "button" && node["name"]["value"] == "Go" }),
        "child frame AX tree should include child button node"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_get_full_ax_tree_returns_child_frame_tree_for_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><main><button>Go</button></main></body></html>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 3310,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(!nodes.is_empty());
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
    assert!(
        nodes
            .iter()
            .any(|node| { node["role"]["value"] == "button" && node["name"]["value"] == "Go" }),
        "child frame AX tree should include child button node"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_child_frame_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><button>Pending</button></body></html>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;

    let raw = json!({
        "id": 3311,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame getFullAXTree should start as a pending command");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame tree lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(3311))
        .expect("Accessibility.getFullAXTree response");
    let nodes = response["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Pending")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_returns_nested_child_frame_tree_for_nested_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><iframe srcdoc='<!doctype html><html><body><main><button>Nested</button></main></body></html>'></iframe></body></html>\"></iframe></body></html>",
    ).await;
    complete_child_frame_lifecycle(&mut ctx).await;
    let nested_child_frame_id =
        nested_child_frame_id_for_single_nested_iframe_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 332,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": nested_child_frame_id }
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(!nodes.is_empty());
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
    assert!(
        nodes
            .iter()
            .any(|node| { node["role"]["value"] == "button" && node["name"]["value"] == "Nested" }),
        "nested child frame AX tree should include nested button node"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_root_ax_node_returns_document_root() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 34,
        "method": "Accessibility.getRootAXNode"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 34);
    assert_eq!(message["result"]["node"]["role"]["value"], "RootWebArea");
    renderer_backend_dom_node_id(&message["result"]["node"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_root_ax_node_returns_child_frame_root_for_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><p>Child</p></body></html>\"></iframe></body></html>",
    ).await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 341,
        "method": "Accessibility.getRootAXNode",
        "params": { "frameId": child_frame_id }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["result"]["node"]["role"]["value"], "RootWebArea");
    renderer_backend_dom_node_id(&message["result"]["node"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_root_ax_node_child_frame_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><p>Child</p></body></html>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;

    let raw = json!({
        "id": 3411,
        "method": "Accessibility.getRootAXNode",
        "params": { "frameId": child_frame_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame getRootAXNode should start as a pending command");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame root lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(3411))
        .expect("Accessibility.getRootAXNode response");
    assert_eq!(response["result"]["node"]["role"]["value"], "RootWebArea");
    renderer_backend_dom_node_id(&response["result"]["node"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_root_ax_node_rejects_foreign_frame() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 35,
        "method": "Accessibility.getRootAXNode",
        "params": { "frameId": "TID-OTHER" }
    }))
    .await;
    ctx.expect_error(
        35,
        -32000,
        "Frame with the given id does not belong to the target.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_root_ax_node_requires_loaded_page() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 350,
        "method": "Accessibility.getRootAXNode"
    }))
    .await;
    ctx.expect_error(350, -32000, "NoDocumentLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_child_ax_nodes_returns_immediate_children() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p><button>Go</button></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 35,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let full_tree = ctx.take_one();
    let root_ax_id = {
        let nodes = full_tree["result"]["nodes"]
            .as_array()
            .expect("nodes array");
        renderer_backed_ax_node_id(&nodes[0])
    };

    ctx.process_async(json!({
        "id": 36,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": root_ax_id }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 36);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 3);
    renderer_backend_dom_node_id(&nodes[0]);
    assert_eq!(nodes[0]["role"]["value"], "none");
    renderer_backend_dom_node_id(&nodes[1]);
    assert_eq!(nodes[1]["role"]["value"], "none");
    renderer_backend_dom_node_id(&nodes[2]);
    assert_eq!(nodes[2]["role"]["value"], "main");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_child_ax_nodes_returns_child_frame_nodes_for_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><main><p>Hello</p><button>Go</button></main></body></html>\"></iframe></body></html>",
    ).await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 361,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let main_ax_id = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["role"]["value"] == "main"))
        .map(renderer_backed_ax_node_id)
        .expect("main AX node id");

    ctx.process_async(json!({
        "id": 362,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": main_ax_id, "frameId": child_frame_id }
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes
            .iter()
            .any(|node| { node["role"]["value"] == "button" && node["name"]["value"] == "Go" }),
        "child frame AX children should include child button node"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_child_ax_nodes_child_frame_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body><main><button>Pending Child</button></main></body></html>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 3611,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let main_ax_id = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["role"]["value"] == "main"))
        .map(renderer_backed_ax_node_id)
        .expect("main AX node id");

    let raw = json!({
        "id": 3612,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": main_ax_id, "frameId": child_frame_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame getChildAXNodes should start as a pending command");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame children lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(3612))
        .expect("Accessibility.getChildAXNodes response");
    let nodes = response["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "button"
                && node["name"]["value"] == "Pending Child")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_child_ax_nodes_validates_ax_id_and_frame() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 37,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": "bad" }
    }))
    .await;
    ctx.expect_error(37, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 38,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": "AX-999" }
    }))
    .await;
    ctx.expect_error(38, -32000, "Could not find node with given id");

    ctx.process_async(json!({
        "id": 39,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let full_tree = ctx.take_one();
    let root_ax_id = {
        let nodes = full_tree["result"]["nodes"]
            .as_array()
            .expect("nodes array");
        renderer_backed_ax_node_id(&nodes[0])
    };

    ctx.process_async(json!({
        "id": 40,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": root_ax_id, "frameId": "TID-OTHER" }
    }))
    .await;
    ctx.expect_error(40, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_child_ax_nodes_requires_loaded_page() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 390,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": "AX-1" }
    }))
    .await;
    ctx.expect_error(390, -32000, "NoDocumentLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_child_ax_nodes_rejects_ax_zero_and_returns_empty_for_leaf() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><p>Hello</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 391,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": "AX-0" }
    }))
    .await;
    ctx.expect_error(391, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 392,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let text_node_id = nodes
        .iter()
        .find(|node| node["role"]["value"] == "StaticText" && node["name"]["value"] == "Hello")
        .and_then(|node| node["nodeId"].as_str())
        .expect("static text node id")
        .to_owned();

    ctx.process_async(json!({
        "id": 393,
        "method": "Accessibility.getChildAXNodes",
        "params": { "id": text_node_id }
    }))
    .await;
    let leaf_message = ctx.take_one();
    assert_eq!(leaf_message["id"], 393);
    assert_eq!(leaf_message["result"]["nodes"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_returns_root_and_descendants() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p><button>Go</button></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 4,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 4);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.len() >= 5);
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
    renderer_backend_dom_node_id(&nodes[0]);
    assert!(
        nodes[0]["childIds"]
            .as_array()
            .is_some_and(|children| !children.is_empty())
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Go")
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "StaticText" && node["name"]["value"] == "Hello")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_depth_zero_returns_first_semantic_layer_like_chromium() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p><button>Go</button></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 44,
        "method": "Accessibility.getFullAXTree",
        "params": { "depth": 0 }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 44);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 4);
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
    assert_eq!(nodes[1]["role"]["value"], "none");
    assert_eq!(nodes[2]["role"]["value"], "none");
    assert_eq!(nodes[3]["role"]["value"], "main");
    assert!(
        nodes
            .iter()
            .all(|node| node["role"]["value"] != "paragraph")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_exposes_basic_roles_and_document_name() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><a href='/x'>Docs</a><img aria-label='Hero'></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 45,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 45);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes[0]["name"]
            .as_object()
            .and_then(|name| name.get("value"))
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.starts_with("data:text/html,"))
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "link" && node["name"]["value"] == "Docs")
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "image" && node["name"]["value"] == "Hero")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_exposes_chromium_axnode_core_shape() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><head><title>Test Page</title></head><body><h1>Hello</h1></body></html>",
    ).await;

    ctx.process_async(json!({
        "id": 451,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 451);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(!nodes.is_empty());

    let document = &nodes[0];
    renderer_backend_dom_node_id(document);
    assert_eq!(document["ignored"], false);
    assert_eq!(document["role"]["type"], "role");
    assert_eq!(document["role"]["value"], "RootWebArea");
    assert_eq!(document["name"]["type"], "computedString");
    assert_eq!(document["name"]["value"], "Test Page");
    let properties = document["properties"].as_array().expect("properties array");
    assert!(!properties.is_empty());
    assert!(
        document["childIds"]
            .as_array()
            .is_some_and(|children| !children.is_empty())
    );

    let heading = nodes
        .iter()
        .find(|node| node["role"]["value"] == "heading")
        .expect("heading node");
    let level = heading["properties"]
        .as_array()
        .expect("heading properties")
        .iter()
        .find(|property| property["name"] == "level")
        .expect("heading level property");
    assert_eq!(level["value"]["type"], "integer");
    assert_eq!(level["value"]["value"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_button_and_listitem_properties_match_chromium_shape() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button><ul><li>First</li><li><ol><li>Nested</li></ol></li></ul></body></html>",
    ).await;

    ctx.process_async(json!({
        "id": 456,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 456);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");

    let button = nodes
        .iter()
        .find(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Go")
        .expect("button node");
    let button_properties = button["properties"].as_array().expect("button properties");
    let invalid = button_properties
        .iter()
        .find(|property| property["name"] == "invalid")
        .expect("button invalid property");
    assert_eq!(invalid["value"]["type"], "token");
    assert_eq!(invalid["value"]["value"], "false");
    let focusable = button_properties
        .iter()
        .find(|property| property["name"] == "focusable")
        .expect("button focusable property");
    assert_eq!(focusable["value"]["type"], "booleanOrUndefined");
    assert_eq!(focusable["value"]["value"], true);

    let listitems = nodes
        .iter()
        .filter(|node| node["role"]["value"] == "listitem")
        .collect::<Vec<_>>();
    assert!(listitems.len() >= 3);
    let levels = listitems
        .iter()
        .filter_map(|node| {
            node["properties"]
                .as_array()
                .and_then(|properties| {
                    properties
                        .iter()
                        .find(|property| property["name"] == "level")
                })
                .map(|level| level["value"]["value"].clone())
        })
        .collect::<Vec<_>>();
    assert!(levels.contains(&json!(1)));
    assert!(levels.contains(&json!(2)));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_checkbox_properties_match_chromium_shape() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><input type='checkbox' checked></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 457,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let checkbox = nodes
        .iter()
        .find(|node| node["role"]["value"] == "checkbox")
        .expect("checkbox node");
    let properties = checkbox["properties"]
        .as_array()
        .expect("checkbox properties");
    let invalid = properties
        .iter()
        .find(|property| property["name"] == "invalid")
        .expect("checkbox invalid property");
    assert_eq!(invalid["value"]["type"], "token");
    assert_eq!(invalid["value"]["value"], "false");
    let focusable = properties
        .iter()
        .find(|property| property["name"] == "focusable")
        .expect("checkbox focusable property");
    assert_eq!(focusable["value"]["type"], "booleanOrUndefined");
    assert_eq!(focusable["value"]["value"], true);
    let checked = properties
        .iter()
        .find(|property| property["name"] == "checked")
        .expect("checkbox checked property");
    assert_eq!(checked["value"]["type"], "tristate");
    assert_eq!(checked["value"]["value"], "true");
    assert!(checkbox.get("value").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_textarea_properties_match_chromium_shape() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><textarea readonly required>hi</textarea></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 458,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let textarea = nodes
        .iter()
        .find(|node| node["role"]["value"] == "textbox")
        .expect("textarea node");
    assert!(textarea.get("name").is_none());
    assert_eq!(textarea["value"]["value"], "hi");
    let properties = textarea["properties"]
        .as_array()
        .expect("textarea properties");
    let multiline = properties
        .iter()
        .find(|property| property["name"] == "multiline")
        .expect("textarea multiline property");
    assert_eq!(multiline["value"]["type"], "boolean");
    assert_eq!(multiline["value"]["value"], true);
    let readonly = properties
        .iter()
        .find(|property| property["name"] == "readonly")
        .expect("textarea readonly property");
    assert_eq!(readonly["value"]["value"], true);
    let required = properties
        .iter()
        .find(|property| property["name"] == "required")
        .expect("textarea required property");
    assert_eq!(required["value"]["value"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_radio_properties_match_chromium_shape() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><input type='radio' checked></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 460,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let radio = nodes
        .iter()
        .find(|node| node["role"]["value"] == "radio")
        .expect("radio node");
    let properties = radio["properties"].as_array().expect("radio properties");

    let invalid = properties
        .iter()
        .find(|property| property["name"] == "invalid")
        .expect("radio invalid property");
    assert_eq!(invalid["value"]["type"], "token");
    assert_eq!(invalid["value"]["value"], "false");

    let focusable = properties
        .iter()
        .find(|property| property["name"] == "focusable")
        .expect("radio focusable property");
    assert_eq!(focusable["value"]["type"], "booleanOrUndefined");
    assert_eq!(focusable["value"]["value"], true);

    let checked = properties
        .iter()
        .find(|property| property["name"] == "checked")
        .expect("radio checked property");
    assert_eq!(checked["value"]["type"], "tristate");
    assert_eq!(checked["value"]["value"], "true");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_reflects_dynamic_status_name_and_live_region_defaults() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body>\
         <div id='status' role='status' aria-live='polite' aria-label='INITIALIZING'>pending</div>\
         <div role='unknown STATUS' aria-live='ASSERTIVE' aria-atomic='false'\
              aria-relevant='removals text' aria-label='EXPLICIT'>pending</div>\
         <script>\
           window.addEventListener('load', () => {\
             document.getElementById('status').setAttribute('aria-label', 'READY-9307');\
           });\
         </script>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 4641,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    for (accessible_name, live, atomic, relevant) in [
        ("READY-9307", "polite", true, "additions text"),
        ("EXPLICIT", "ASSERTIVE", false, "removals text"),
    ] {
        let status = find_ax_node(nodes, "status", accessible_name);
        let properties = status["properties"].as_array().expect("status properties");
        for (name, kind, value) in [
            ("live", "token", json!(live)),
            ("atomic", "boolean", json!(atomic)),
            ("relevant", "tokenList", json!(relevant)),
        ] {
            let property = properties
                .iter()
                .find(|property| property["name"] == name)
                .unwrap_or_else(|| panic!("expected {name} in status properties: {properties:?}"));
            assert_eq!(property["value"]["type"], kind);
            assert_eq!(property["value"]["value"], value);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_respects_depth_limit() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 5,
        "method": "Accessibility.getFullAXTree",
        "params": { "depth": 1 }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 5);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 4);
    assert_eq!(nodes[0]["role"]["value"], "RootWebArea");
    assert_eq!(nodes[1]["role"]["value"], "none");
    assert_eq!(nodes[2]["role"]["value"], "none");
    assert_eq!(nodes[3]["role"]["value"], "main");
    assert!(
        nodes
            .iter()
            .all(|node| node["role"]["value"] != "paragraph")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_treats_negative_depth_as_unbounded() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><p>Hello</p><button>Go</button></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 46,
        "method": "Accessibility.getFullAXTree",
        "params": { "depth": -1 }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 46);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.len() >= 5);
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Go")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_aria_label_overrides_direct_text() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button aria-label='ARIA'>Visible</button></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 47,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 47);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let button = nodes
        .iter()
        .find(|node| node["role"]["value"] == "button")
        .expect("button ax node");
    assert_eq!(button["name"]["value"], "ARIA");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_aria_labelledby_overrides_aria_label_and_joins_idrefs() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body>\
         <span id='first'>Referenced</span><span id='second'><b>label</b></span>\
         <button aria-labelledby='first second' aria-label='ARIA'>Visible</button>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 471,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 471);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let button = nodes
        .iter()
        .find(|node| node["role"]["value"] == "button")
        .expect("button ax node");
    assert_eq!(button["name"]["value"], "Referenced label");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_uses_explicit_and_implicit_html_labels() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body>\
         <label for='email'><span>Email</span> address</label>\
         <input id='email' type='text'>\
         <label>Accept <span>terms</span><input type='checkbox'></label>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 472,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 472);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes.iter().any(|node| {
            node["role"]["value"] == "textbox" && node["name"]["value"] == "Email address"
        }),
        "explicit label should name the textbox"
    );
    assert!(
        nodes.iter().any(|node| {
            node["role"]["value"] == "checkbox" && node["name"]["value"] == "Accept terms"
        }),
        "wrapping label should name the checkbox"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_projects_dom_only_nodes_out_of_chromium_ax_order() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><head><title>Controls</title></head><body>\
         <input type='checkbox' aria-label='Wifi'>\
         <input type='checkbox' aria-label='Bluetooth'>\
         <button aria-label='Locked' disabled>Visible text</button>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 473,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let first_roles = nodes
        .iter()
        .take(6)
        .map(|node| node["role"]["value"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        first_roles,
        [
            "RootWebArea",
            "none",
            "generic",
            "checkbox",
            "checkbox",
            "button"
        ]
    );
    assert!(
        nodes
            .iter()
            .all(|node| node["role"]["value"] != "StaticText" || node.get("name").is_some()),
        "whitespace-only text nodes must not leak into the AX tree"
    );
    assert_eq!(nodes[1]["ignored"], true);
    assert_eq!(nodes[1]["ignoredReasons"][0]["name"], "uninteresting");
    assert_eq!(nodes[1]["parentId"], nodes[0]["nodeId"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_marks_aria_hidden_subtree_ignored() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body>\
         <button>Real Action</button>\
         <div id='decoy' aria-hidden='true'><button>Ghost Action</button></div>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 474,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes.iter().any(|node| {
            node["role"]["value"] == "button" && node["name"]["value"] == "Real Action"
        }),
        "visible control should remain exposed"
    );
    let hidden = nodes
        .iter()
        .find(|node| node["ignoredReasons"][0]["name"] == "ariaHiddenSubtree")
        .expect("aria-hidden AX node");
    assert_eq!(hidden["ignored"], true);
    assert_eq!(hidden["role"]["value"], "none");
    assert!(hidden.get("name").is_none());
    assert!(
        nodes
            .iter()
            .all(|node| node["name"]["value"] != "Ghost Action")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_uses_runtime_checked_and_effective_disabled_state() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body>\
         <input id='wifi' type='checkbox' aria-label='Wifi'>\
         <input id='mixed' type='checkbox' aria-label='Mixed'>\
         <fieldset disabled>\
           <legend><button aria-label='Legend action'>Legend</button></legend>\
           <button aria-label='Blocked action'>Blocked</button>\
         </fieldset>\
         <script>\
           document.getElementById('wifi').checked = true;\
           document.getElementById('mixed').indeterminate = true;\
         </script>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 475,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    let wifi = find_ax_node(nodes, "checkbox", "Wifi");
    assert!(wifi["properties"].as_array().is_some_and(|properties| {
        properties.iter().any(|property| {
            property["name"] == "checked"
                && property["value"]["type"] == "tristate"
                && property["value"]["value"] == "true"
        })
    }));
    let mixed = find_ax_node(nodes, "checkbox", "Mixed");
    assert!(mixed["properties"].as_array().is_some_and(|properties| {
        properties
            .iter()
            .any(|property| property["name"] == "checked" && property["value"]["value"] == "mixed")
    }));

    let blocked = find_ax_node(nodes, "button", "Blocked action");
    assert!(blocked["properties"].as_array().is_some_and(|properties| {
        properties
            .iter()
            .any(|property| property["name"] == "disabled" && property["value"]["value"] == true)
    }));
    let legend = find_ax_node(nodes, "button", "Legend action");
    assert!(legend["properties"].as_array().is_some_and(|properties| {
        properties
            .iter()
            .all(|property| property["name"] != "disabled")
            && properties
                .iter()
                .any(|property| property["name"] == "focusable")
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_full_ax_tree_exposes_iframe_container_role() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe title='Child' srcdoc='<button>Go</button>'></iframe></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 476,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.iter().any(|node| node["role"]["value"] == "Iframe"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_returns_chain_for_backend_node_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main><button>Go</button></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 48,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let tree = ctx.take_one();
    let nodes = tree["result"]["nodes"].as_array().expect("nodes array");
    let button = nodes
        .iter()
        .find(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Go")
        .expect("button AX node");
    let button_backend_id = renderer_backend_dom_node_id(button);

    ctx.process_async(json!({
        "id": 49,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "backendNodeId": button_backend_id }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 49);
    let chain = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(chain.len() >= 4);
    assert_eq!(chain[0]["role"]["value"], "button");
    assert_eq!(chain[0]["name"]["value"], "Go");
    assert_eq!(chain.last().unwrap()["role"]["value"], "RootWebArea");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_supports_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button id='go'>Go</button></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 50,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('go')" }
    }))
    .await;
    let object_id = ctx.take_response_by_id(50)["result"]["result"]["objectId"]
        .as_str()
        .expect("object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 51,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "objectId": object_id }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 51);
    let chain = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(chain[0]["role"]["value"], "button");
    assert_eq!(chain[0]["name"]["value"], "Go");
    assert_eq!(chain.last().unwrap()["role"]["value"], "RootWebArea");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_object_id_completes_with_single_renderer_command() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button id='go'>Go</button></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 510,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('go')" }
    }))
    .await;
    let object_id = ctx.take_response_by_id(510)["result"]["result"]["objectId"]
        .as_str()
        .expect("object id")
        .to_owned();

    let raw = json!({
        "id": 511,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "objectId": object_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("object id AX ancestors lookup should start as a pending renderer command");
    let completed = pending.wait().await;
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("top-frame object id AX lookup should complete after one renderer command");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "AX top-frame object lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let message = messages
        .iter()
        .find(|message| message["id"] == json!(511))
        .expect("Accessibility.getAXNodeAndAncestors response");
    let chain = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(chain[0]["role"]["value"], "button");
    assert_eq!(chain[0]["name"]["value"], "Go");
    assert_eq!(chain.last().unwrap()["role"]["value"], "RootWebArea");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_validates_reference() {
    let mut ctx = TestContext::new();
    load_page_async(&mut ctx, "<!doctype html><html><body></body></html>").await;

    ctx.process_async(json!({
        "id": 52,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": {}
    }))
    .await;
    ctx.expect_error(52, -32000, "Could not find node with given id");

    ctx.process_async(json!({
        "id": 53,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "backendNodeId": 999 }
    }))
    .await;
    ctx.expect_error(53, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn accessibility_backend_node_id_does_not_fallback_to_low_frontend_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 5301,
        "method": "Accessibility.queryAXTree",
        "params": { "backendNodeId": 1, "role": "button" }
    }))
    .await;
    ctx.expect_error(5301, -32000, "Could not find node with given id");

    let document_node_id = dom_document_node_id_async(&mut ctx, None, 5302).await;
    ctx.process_async(json!({
        "id": 5303,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": document_node_id, "role": "button" }
    }))
    .await;
    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "button");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_requires_context_loaded_page_and_bound_node() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 530,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "nodeId": 1 }
    }))
    .await;
    ctx.expect_error(530, -31998, "BrowserContextNotLoaded");

    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({
        "id": 531,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "nodeId": 1 }
    }))
    .await;
    ctx.expect_error(531, -32000, "NoDocumentLoaded");

    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button></body></html>",
    )
    .await;
    ctx.process_async(json!({
        "id": 532,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "nodeId": 1, "frameId": "TID-OTHER" }
    }))
    .await;
    ctx.expect_error(532, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_rejects_unknown_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 533,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "objectId": "bogus-object-id" }
    }))
    .await;
    ctx.expect_error(533, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_returns_child_frame_chain_for_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<button>Go</button>\"></iframe></body></html>",
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 5330,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let button = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Go")
        })
        .expect("child button AX node");
    let button_backend_id = renderer_backend_dom_node_id(button);
    ctx.process_async(json!({
        "id": 5331,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "backendNodeId": button_backend_id, "frameId": child_frame_id }
    }))
    .await;
    let message = ctx.take_one();
    let chain = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(chain.len() >= 3);
    assert_eq!(chain[0]["role"]["value"], "button");
    assert_eq!(chain[0]["name"]["value"], "Go");
    assert_eq!(chain.last().unwrap()["role"]["value"], "RootWebArea");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_child_frame_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<main><button>Pending Chain</button></main>\"></iframe></body></html>",
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 53301,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let button = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["role"]["value"] == "button"))
        .expect("button AX node");
    let button_backend_id = renderer_backend_dom_node_id(button);

    let raw = json!({
        "id": 53302,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "backendNodeId": button_backend_id, "frameId": child_frame_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame getAXNodeAndAncestors should start as a pending command");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame ancestor lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(53302))
        .expect("Accessibility.getAXNodeAndAncestors response");
    let chain = response["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(chain.first().unwrap()["role"]["value"], "button");
    assert_eq!(chain.first().unwrap()["name"]["value"], "Pending Chain");
    assert_eq!(chain.last().unwrap()["role"]["value"], "RootWebArea");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_ax_node_and_ancestors_accepts_child_frame_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<button>Go</button>\"></iframe></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 5332,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": child_frame_id, "worldName": "ax-child-object-ancestors" }
    }))
    .await;
    let context_id = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(5332))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .expect("child isolated world id");
    ctx.process_async(json!({
        "id": 5333,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "expression": "document.querySelector('button')"
        }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("child button object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 5334,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "objectId": object_id, "frameId": child_frame_id }
    }))
    .await;
    let message = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(5334))
        .expect("accessibility response");
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.iter().any(|node| node["role"]["value"] == "button"));
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_get_ax_node_and_ancestors_accepts_child_frame_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<button>Go</button>\"></iframe></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 53320,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": child_frame_id, "worldName": "ax-child-object-ancestors-async" }
    }))
    .await;
    let context_id = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(53320))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .expect("child isolated world id");
    ctx.process_async(json!({
        "id": 53321,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "expression": "document.querySelector('button')"
        }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("child button object id")
        .to_owned();
    let raw = json!({
        "id": 53322,
        "method": "Accessibility.getAXNodeAndAncestors",
        "params": { "objectId": object_id, "frameId": child_frame_id }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame object id AX lookup should start as a pending renderer command");
    let completed = pending.wait().await;
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("child-frame object id AX lookup should complete after one renderer command");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame object lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let message = messages
        .iter()
        .find(|message| message["id"] == json!(53322))
        .expect("accessibility response");
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.iter().any(|node| node["role"]["value"] == "button"));
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_ax_reference_accepts_renderer_backend_node_id_live() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<main><button>Go</button><a href='/x'>Docs</a></main>\"></iframe></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    let backend_node_id = child_frame_renderer_backend_node_id_for_selector_async(
        &mut ctx,
        &child_frame_id,
        "button",
        53330,
    )
    .await;

    let commands = [
        (
            53333,
            "Accessibility.getAXNodeAndAncestors",
            json!({
                "backendNodeId": backend_node_id,
                "frameId": child_frame_id.clone(),
            }),
        ),
        (
            53334,
            "Accessibility.queryAXTree",
            json!({
                "backendNodeId": backend_node_id,
                "frameId": child_frame_id.clone(),
                "role": "button",
                "accessibleName": "Go",
            }),
        ),
        (
            53335,
            "Accessibility.getPartialAXTree",
            json!({
                "backendNodeId": backend_node_id,
                "frameId": child_frame_id.clone(),
            }),
        ),
    ];

    for (id, method, params) in commands {
        let raw = json!({
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .unwrap_or_else(|| {
                panic!("{method} with child renderer backend id should start as a pending command")
            });
        let completed = pending.wait().await;
        let CdpCommandTaskStep::Complete(outcome) =
            ctx.conn.complete_pending_command_dispatch(completed).await
        else {
            panic!(
                "{method} with child renderer backend id should complete after one renderer command"
            );
        };
        let (messages, scheduler_events) = outcome.into_parts();
        assert!(
            scheduler_events.is_empty(),
            "{method} should not enqueue scheduler events: {scheduler_events:?}"
        );
        let response = messages
            .iter()
            .find(|message| message["id"] == json!(id))
            .unwrap_or_else(|| panic!("{method} response should be present: {messages:?}"));
        let nodes = response["result"]["nodes"]
            .as_array()
            .unwrap_or_else(|| panic!("{method} should return accessibility nodes: {response}"));
        assert!(
            nodes
                .iter()
                .any(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Go"),
            "{method} should resolve the child-frame backend id to the button AX node: {response}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_ax_reference_accepts_bound_renderer_frontend_node_id_live() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<main><button>Go</button><a href='/x'>Docs</a></main>\"></iframe></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    let ids = child_frame_renderer_dom_node_ids_for_selector_async(
        &mut ctx,
        &child_frame_id,
        "button",
        53340,
    )
    .await;
    assert_ne!(
        ids.frontend_node_id, ids.backend_node_id,
        "frontend node id should stay separate from renderer backend identity"
    );

    let commands = [
        (
            53343,
            "Accessibility.getAXNodeAndAncestors",
            json!({
                "nodeId": ids.frontend_node_id,
                "frameId": child_frame_id.clone(),
            }),
        ),
        (
            53344,
            "Accessibility.queryAXTree",
            json!({
                "nodeId": ids.frontend_node_id,
                "frameId": child_frame_id.clone(),
                "role": "button",
                "accessibleName": "Go",
            }),
        ),
        (
            53345,
            "Accessibility.getPartialAXTree",
            json!({
                "nodeId": ids.frontend_node_id,
                "frameId": child_frame_id.clone(),
            }),
        ),
    ];

    for (id, method, params) in commands {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .await;
        let response = ctx.take_one();
        let nodes = response["result"]["nodes"]
            .as_array()
            .unwrap_or_else(|| panic!("{method} should return accessibility nodes: {response}"));
        assert!(
            nodes
                .iter()
                .any(|node| node["role"]["value"] == "button" && node["name"]["value"] == "Go"),
            "{method} should resolve the child-frame frontend node id through renderer binding: {response}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_filters_by_role_and_name() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button><button>Stay</button><a href='/x'>Docs</a></body></html>",
    ).await;
    let document_node_id = dom_document_node_id_async(&mut ctx, None, 5400).await;

    ctx.process_async(json!({
        "id": 54,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": document_node_id, "role": "button", "accessibleName": "Go" }
    }))
    .await;

    let message = ctx.take_one();
    assert_eq!(message["id"], 54);
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "button");
    assert_eq!(nodes[0]["name"]["value"], "Go");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_supports_object_id_and_empty_match() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main id='root'><a href='/x'>Docs</a></main></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 55,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('root')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 56,
        "method": "Accessibility.queryAXTree",
        "params": { "objectId": object_id, "role": "link", "accessibleName": "Docs" }
    }))
    .await;
    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "link");
    assert_eq!(nodes[0]["name"]["value"], "Docs");

    ctx.process_async(json!({
        "id": 57,
        "method": "Accessibility.queryAXTree",
        "params": { "objectId": object_id, "role": "button" }
    }))
    .await;
    let empty = ctx.take_one();
    assert_eq!(empty["result"]["nodes"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_requires_context_loaded_page_and_bound_node() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 580,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": 1 }
    }))
    .await;
    ctx.expect_error(580, -31998, "BrowserContextNotLoaded");

    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({
        "id": 581,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": 1 }
    }))
    .await;
    ctx.expect_error(581, -32000, "NoDocumentLoaded");

    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button></body></html>",
    )
    .await;
    ctx.process_async(json!({
        "id": 582,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": 1, "frameId": "TID-OTHER" }
    }))
    .await;
    ctx.expect_error(582, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_role_only_name_only_and_unfiltered_paths_work() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button><button>Stay</button><a href='/x'>Docs</a></body></html>",
    ).await;
    let document_node_id = dom_document_node_id_async(&mut ctx, None, 5830).await;

    ctx.process_async(json!({
        "id": 583,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": document_node_id, "role": "button" }
    }))
    .await;
    let role_only = ctx.take_one();
    let role_nodes = role_only["result"]["nodes"]
        .as_array()
        .expect("nodes array");
    assert_eq!(role_nodes.len(), 2);
    assert!(
        role_nodes
            .iter()
            .all(|node| node["role"]["value"] == "button")
    );

    ctx.process_async(json!({
        "id": 584,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": document_node_id, "accessibleName": "Docs" }
    }))
    .await;
    let name_only = ctx.take_one();
    let name_nodes = name_only["result"]["nodes"]
        .as_array()
        .expect("nodes array");
    assert!(!name_nodes.is_empty());
    assert!(
        name_nodes
            .iter()
            .any(|node| node["role"]["value"] == "link" && node["name"]["value"] == "Docs")
    );

    ctx.process_async(json!({
        "id": 585,
        "method": "Accessibility.queryAXTree",
        "params": { "nodeId": document_node_id }
    }))
    .await;
    let unfiltered = ctx.take_one();
    let all_nodes = unfiltered["result"]["nodes"]
        .as_array()
        .expect("nodes array");
    assert!(all_nodes.len() >= 4);
    assert_eq!(all_nodes[0]["role"]["value"], "RootWebArea");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_rejects_unknown_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><button>Go</button></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 586,
        "method": "Accessibility.queryAXTree",
        "params": { "objectId": "bogus-object-id" }
    }))
    .await;
    ctx.expect_error(586, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_returns_child_frame_matches_for_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<button>Go</button><a href='/x'>Docs</a>\"></iframe></body></html>",
    ).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 5860,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let root_backend_id = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.first())
        .map(renderer_backend_dom_node_id)
        .expect("child root AX backend id");
    ctx.process_async(json!({
        "id": 5861,
        "method": "Accessibility.queryAXTree",
        "params": {
            "backendNodeId": root_backend_id,
            "frameId": child_frame_id,
            "role": "button",
            "accessibleName": "Go"
        }
    }))
    .await;
    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "button");
    assert_eq!(nodes[0]["name"]["value"], "Go");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_child_frame_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<button>Pending Query</button><a href='/x'>Docs</a>\"></iframe></body></html>",
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 58609,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let root_backend_id = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.first())
        .map(renderer_backend_dom_node_id)
        .expect("child root AX backend id");
    let raw = json!({
        "id": 58610,
        "method": "Accessibility.queryAXTree",
        "params": {
            "backendNodeId": root_backend_id,
            "frameId": child_frame_id,
            "role": "button",
            "accessibleName": "Pending Query"
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame queryAXTree should start as a pending command");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame query should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(58610))
        .expect("Accessibility.queryAXTree response");
    let nodes = response["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "button");
    assert_eq!(nodes[0]["name"]["value"], "Pending Query");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_ax_tree_accepts_child_frame_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<button>Go</button><a href='/x'>Docs</a>\"></iframe></body></html>",
    ).await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 5862,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": child_frame_id, "worldName": "ax-child-object-query" }
    }))
    .await;
    let context_id = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(5862))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .expect("child isolated world id");
    ctx.process_async(json!({
        "id": 5863,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "expression": "document.querySelector('button')"
        }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("child button object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 5864,
        "method": "Accessibility.queryAXTree",
        "params": {
            "objectId": object_id,
            "frameId": child_frame_id,
            "role": "button",
            "accessibleName": "Go"
        }
    }))
    .await;
    let message = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(5864))
        .expect("accessibility response");
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "button");
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_query_ax_tree_accepts_child_frame_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<button>Go</button><a href='/x'>Docs</a>\"></iframe></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 58620,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": child_frame_id, "worldName": "ax-child-object-query-async" }
    }))
    .await;
    let context_id = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(58620))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .expect("child isolated world id");
    ctx.process_async(json!({
        "id": 58621,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "expression": "document.querySelector('button')"
        }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("child button object id")
        .to_owned();
    let raw = json!({
        "id": 58622,
        "method": "Accessibility.queryAXTree",
        "params": {
            "objectId": object_id,
            "frameId": child_frame_id,
            "role": "button",
            "accessibleName": "Go"
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame object id AX query should start as a pending renderer command");
    let completed = pending.wait().await;
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("child-frame object id AX query should complete after one renderer command");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame object query should not enqueue scheduler events: {scheduler_events:?}"
    );
    let message = messages
        .iter()
        .find(|message| message["id"] == json!(58622))
        .expect("accessibility response");
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "button");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_partial_ax_tree_without_relatives_returns_only_target_node() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main id='root'><button>Go</button></main></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 5860,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('root')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 587,
        "method": "Accessibility.getPartialAXTree",
        "params": { "objectId": object_id, "fetchRelatives": false }
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "main");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_partial_ax_tree_with_relatives_includes_subtree_and_ancestors() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main id='root'><button>Go</button></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 5871,
        "method": "Accessibility.getFullAXTree"
    }))
    .await;
    let full_tree = ctx.take_one();
    let main = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["role"]["value"] == "main"))
        .expect("main AX node");
    let main_backend_id = renderer_backend_dom_node_id(main);

    ctx.process_async(json!({
        "id": 588,
        "method": "Accessibility.getPartialAXTree",
        "params": { "backendNodeId": main_backend_id }
    }))
    .await;

    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(
        nodes
            .iter()
            .any(|node| node["backendDOMNodeId"] == json!(main_backend_id))
    );
    assert!(nodes.iter().any(|node| node["role"]["value"] == "button"));
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "RootWebArea")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_partial_ax_tree_supports_object_id_and_validates_reference() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><main id='root'><button>Go</button></main></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    ctx.process_async(json!({
        "id": 589,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('root')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 590,
        "method": "Accessibility.getPartialAXTree",
        "params": { "objectId": object_id, "fetchRelatives": false }
    }))
    .await;
    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "main");

    ctx.process_async(json!({
        "id": 591,
        "method": "Accessibility.getPartialAXTree",
        "params": { "objectId": "bogus-object-id" }
    }))
    .await;
    ctx.expect_error(591, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_partial_ax_tree_returns_child_frame_nodes_for_child_frame_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<main><button>Go</button></main>\"></iframe></body></html>",
    ).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 5910,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let main = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["role"]["value"] == "main"))
        .expect("child main AX node");
    let main_backend_id = renderer_backend_dom_node_id(main);
    ctx.process_async(json!({
        "id": 5911,
        "method": "Accessibility.getPartialAXTree",
        "params": { "backendNodeId": main_backend_id, "frameId": child_frame_id }
    }))
    .await;
    let message = ctx.take_one();
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.iter().any(|node| node["role"]["value"] == "main"));
    assert!(nodes.iter().any(|node| node["role"]["value"] == "button"));
    assert!(
        nodes
            .iter()
            .any(|node| node["role"]["value"] == "RootWebArea")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_partial_ax_tree_child_frame_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<main><button>Pending Partial</button></main>\"></iframe></body></html>",
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 59110,
        "method": "Accessibility.getFullAXTree",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let full_tree = ctx.take_one();
    let button = full_tree["result"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["role"]["value"] == "button"))
        .expect("button AX node");
    let button_backend_id = renderer_backend_dom_node_id(button);

    let raw = json!({
        "id": 59111,
        "method": "Accessibility.getPartialAXTree",
        "params": {
            "backendNodeId": button_backend_id,
            "frameId": child_frame_id,
            "fetchRelatives": false
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("child-frame getPartialAXTree should start as a pending command");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame partial tree lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(59111))
        .expect("Accessibility.getPartialAXTree response");
    let nodes = response["result"]["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["role"]["value"], "button");
    assert_eq!(nodes[0]["name"]["value"], "Pending Partial");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_partial_ax_tree_accepts_child_frame_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<main><button>Go</button></main>\"></iframe></body></html>",
    ).await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 5912,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": child_frame_id, "worldName": "ax-child-object-partial" }
    }))
    .await;
    let context_id = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(5912))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .expect("child isolated world id");
    ctx.process_async(json!({
        "id": 5913,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "expression": "document.querySelector('button')"
        }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("child button object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 5914,
        "method": "Accessibility.getPartialAXTree",
        "params": { "objectId": object_id, "frameId": child_frame_id }
    }))
    .await;
    let message = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(5914))
        .expect("accessibility response");
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.iter().any(|node| node["role"]["value"] == "button"));
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_get_partial_ax_tree_accepts_child_frame_object_id() {
    let mut ctx = TestContext::new();
    load_page_async(
        &mut ctx,
        "<!doctype html><html><body><iframe srcdoc=\"<main><button>Go</button></main>\"></iframe></body></html>",
    )
    .await;
    enable_runtime_async(&mut ctx).await;

    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx).await;
    ctx.process_async(json!({
        "id": 59120,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": child_frame_id, "worldName": "ax-child-object-partial-async" }
    }))
    .await;
    let context_id = ctx
        .take_all()
        .into_iter()
        .find(|message| message["id"] == json!(59120))
        .and_then(|message| message["result"]["executionContextId"].as_i64())
        .expect("child isolated world id");
    ctx.process_async(json!({
        "id": 59121,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": context_id,
            "expression": "document.querySelector('button')"
        }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("child button object id")
        .to_owned();
    let raw = json!({
        "id": 59122,
        "method": "Accessibility.getPartialAXTree",
        "params": { "objectId": object_id, "frameId": child_frame_id }
    })
    .to_string();
    let pending = ctx.conn.try_start_pending_command_dispatch(&raw).expect(
        "child-frame object id partial AX lookup should start as a pending renderer command",
    );
    let completed = pending.wait().await;
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!(
            "child-frame object id partial AX lookup should complete after one renderer command"
        );
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "AX child-frame object partial lookup should not enqueue scheduler events: {scheduler_events:?}"
    );
    let message = messages
        .iter()
        .find(|message| message["id"] == json!(59122))
        .expect("accessibility response");
    let nodes = message["result"]["nodes"].as_array().expect("nodes array");
    assert!(nodes.iter().any(|node| node["role"]["value"] == "button"));
}
