use serde_json::{Value, json};

use super::{CdpCommandTaskStep, CommandOwnerScope};
use crate::testing::TestContext;

mod document_agents;
mod io;
mod lifecycle;
mod native_commands;
mod runtime_agents;

pub(super) struct InspectionDocumentHandle {
    context: moli_core::browser::BrowserContextHandle,
    document: moli_core::browser::DocumentHandle,
}

impl InspectionDocumentHandle {
    pub(super) async fn runtime_heap_usage_for_test(
        &self,
    ) -> Result<moli_core::page::RendererRuntimeHeapUsage, String> {
        self.context
            .document_runtime_heap_usage_for_test(self.document)
            .await
    }

    pub(super) async fn evaluate_runtime_expression_for_test(
        &self,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        self.context
            .evaluate_document_expression_for_test(self.document, expression, await_promise)
            .await
    }

    pub(super) fn document_title_for_test(&self) -> String {
        self.context
            .document_title(self.document)
            .expect("inspection document must remain current")
    }

    pub(super) fn start_blob_bytes_for_uuid_for_test(
        &self,
        uuid: String,
    ) -> Result<moli_core::browser::PendingDocumentBlobRead, String> {
        self.context.start_document_blob_read(self.document, uuid)
    }

    pub(super) fn finish_blob_bytes_for_uuid_for_test(
        &self,
        completed: moli_core::browser::CompletedDocumentBlobRead,
    ) -> Result<Option<std::sync::Arc<[u8]>>, String> {
        self.context.finish_document_blob_read(completed)
    }
}

pub(super) fn inspection_document_handle(
    conn: &super::CdpConnection,
    owner: &CommandOwnerScope,
) -> InspectionDocumentHandle {
    let (context_id, target_id) = conn.resolved_page_owner_identity_for_owner(owner).unwrap();
    let (context, document) = conn
        .browser_context_by_id(&context_id)
        .unwrap()
        .inspection_document_handle_for_test(&target_id)
        .unwrap();
    InspectionDocumentHandle { context, document }
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_inspection_starts_without_protocol_document_ownership() {
    dom_inspection_round_trip(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_inspection_completes_without_protocol_document_ownership() {
    dom_inspection_round_trip(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_inspection_rejects_frozen_replies_and_follow_ups_after_rebind() {
    use crate::conn::{Cmd, ParsedCdpCommand};
    use crate::domains::dom::{
        DomCommandDispatchStep, DomCommandTaskStep, complete_pending_dom_command_output_plan,
        try_start_dom_command_dispatch,
    };

    for method in [
        "DOM.getDocument",
        "DOM.resolveNode",
        "DOM.setAttributeValue",
    ] {
        let mut ctx = dom_context().await;
        let root = dom_command(&mut ctx, 10, "DOM.getDocument", json!({"depth": -1})).await;
        let node = dom_command(
            &mut ctx,
            11,
            "DOM.querySelector",
            json!({
                "nodeId": root["root"]["nodeId"], "selector": "#inspected",
            }),
        )
        .await["nodeId"]
            .clone();
        let raw = json!({"id": 12, "method": method, "params": {
            "nodeId": node, "name": "data-phase", "value": "stale-mutation",
        }})
        .to_string();
        let parsed = ParsedCdpCommand::parse_str(&raw).unwrap();
        let cmd = Cmd::from_parsed(&parsed).unwrap();
        let DomCommandDispatchStep::Pending(pending) =
            try_start_dom_command_dispatch(&mut ctx.conn, &cmd).unwrap()
        else {
            panic!("{method} must start on the old renderer");
        };
        let completed = pending.wait().await;
        let owner = CommandOwnerScope::capture(&ctx.conn, None);
        let old_document = inspection_document_handle(&ctx.conn, &owner);
        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<body><main id='inspected' data-phase='replacement'>new</main></body>",
            None,
        )
        .await;

        let (step, plan) = complete_pending_dom_command_output_plan(&mut ctx.conn, completed).await;
        assert!(
            matches!(step, DomCommandTaskStep::Complete),
            "a stale lookup must not start a command on the replacement"
        );
        let messages = plan
            .into_background_events(Some(12), None)
            .into_iter()
            .map(|event| event.into_protocol_message())
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            vec![json!({"id": 12, "error": {
                "code": -32000, "message": "Renderer attachment changed",
            }})],
            "{method} must reject the outgoing attachment"
        );
        let root = dom_command(&mut ctx, 13, "DOM.getDocument", json!({"depth": -1})).await;
        let node = dom_command(
            &mut ctx,
            14,
            "DOM.querySelector",
            json!({
                "nodeId": root["root"]["nodeId"], "selector": "#inspected",
            }),
        )
        .await["nodeId"]
            .clone();
        let attributes =
            dom_command(&mut ctx, 15, "DOM.getAttributes", json!({"nodeId": node})).await;
        assert!(
            attributes["attributes"]
                .as_array()
                .unwrap()
                .chunks_exact(2)
                .any(|pair| pair == [json!("data-phase"), json!("replacement")])
        );
        drop(old_document);
    }
}

async fn dom_context() -> TestContext {
    let mut ctx = TestContext::new();
    let mut context = ctx
        .conn
        .new_browser_context_fixture_for_test("BID-dom-inspection");
    context.set_active_target_id("TID-dom-inspection");
    ctx.conn.install_browser_context_fixture_for_test(context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body><main id='inspected' data-phase='before'><span>value</span></main></body>",
        None,
    ).await;
    ctx
}

async fn dom_inspection_round_trip(start_before_move: bool) {
    let mut ctx = dom_context().await;
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let document_handle = |ctx: &TestContext| inspection_document_handle(&ctx.conn, &owner);
    let mut document = (!start_before_move).then(|| document_handle(&ctx));
    let raw = json!({"id": 1, "method": "DOM.getDocument", "params": {"depth": -1}}).to_string();
    let step = ctx.conn.start_command_dispatch(&raw);
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "DOM inspection must start on the live binding without a Protocol Document"
    );
    if start_before_move {
        document = Some(document_handle(&ctx));
    }
    let document = document.unwrap();
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(1))
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "DOM snapshot completion: {response}"
    );
    let root = response["result"]["root"]["nodeId"].as_u64().unwrap();
    let node = dom_command(
        &mut ctx,
        2,
        "DOM.querySelector",
        json!({
            "nodeId": root, "selector": "#inspected",
        }),
    )
    .await["nodeId"]
        .as_u64()
        .unwrap();
    assert!(node > 0);
    let object = dom_command(&mut ctx, 3, "DOM.resolveNode", json!({"nodeId": node})).await;
    assert!(object["object"]["objectId"].is_string());
    let mutation = dom_exchange(
        &mut ctx,
        4,
        "DOM.setAttributeValue",
        json!({
            "nodeId": node, "name": "data-phase", "value": "after",
        }),
    )
    .await;
    let event = mutation
        .iter()
        .position(|message| {
            message["method"] == json!("DOM.attributeModified")
                && message["params"]["nodeId"] == json!(node)
                && message["params"]["name"] == json!("data-phase")
                && message["params"]["value"] == json!("after")
        })
        .expect("the bound DOM agent must publish the real mutation without a Protocol Document");
    let response = mutation
        .iter()
        .position(|message| message["id"] == json!(4))
        .unwrap();
    assert!(
        event < response,
        "the DOM response must retain its renderer output fence: {mutation:?}"
    );
    let attributes = dom_command(&mut ctx, 5, "DOM.getAttributes", json!({"nodeId": node})).await;
    assert!(
        attributes["attributes"]
            .as_array()
            .unwrap()
            .chunks_exact(2)
            .any(|pair| pair == [json!("data-phase"), json!("after")])
    );
    let search = dom_command(
        &mut ctx,
        6,
        "DOM.performSearch",
        json!({"query": "#inspected"}),
    )
    .await;
    assert_eq!(search["resultCount"], json!(1));
    let found = dom_command(
        &mut ctx,
        7,
        "DOM.getSearchResults",
        json!({
            "searchId": search["searchId"], "fromIndex": 0, "toIndex": 1,
        }),
    )
    .await;
    assert_eq!(found["nodeIds"], json!([node]));
    dom_command(
        &mut ctx,
        8,
        "DOM.discardSearchResults",
        json!({"searchId": search["searchId"]}),
    )
    .await;
    let actual = document
        .evaluate_runtime_expression_for_test(
            "document.getElementById('inspected').getAttribute('data-phase')",
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        actual["value"],
        json!("after"),
        "the DOM mutation must reach the actual renderer"
    );
    assert!(ctx.conn.has_loaded_page_for_owner(&owner));
}

async fn dom_command(ctx: &mut TestContext, id: u64, method: &str, params: Value) -> Value {
    dom_exchange(ctx, id, method, params)
        .await
        .into_iter()
        .find(|message| message["id"] == json!(id))
        .unwrap()["result"]
        .clone()
}

async fn dom_exchange(ctx: &mut TestContext, id: u64, method: &str, params: Value) -> Vec<Value> {
    // Observe the frontend stream, including separately transported renderer
    // events, rather than only the adapter completion's return value.
    ctx.process_async(json!({"id": id, "method": method, "params": params}))
        .await;
    let messages = ctx.take_all();
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "{method} failed: {response}"
    );
    messages
}
