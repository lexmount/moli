use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn css_starts_without_protocol_document() {
    document_agent_round_trip("CSS.enable", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn css_completes_without_protocol_document() {
    document_agent_round_trip("CSS.enable", true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn accessibility_starts_without_protocol_document() {
    document_agent_round_trip("Accessibility.getFullAXTree", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn accessibility_completes_without_protocol_document() {
    document_agent_round_trip("Accessibility.getFullAXTree", true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_snapshot_starts_without_protocol_document() {
    document_agent_round_trip("DOMSnapshot.captureSnapshot", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_snapshot_completes_without_protocol_document() {
    document_agent_round_trip("DOMSnapshot.captureSnapshot", true).await;
}

async fn document_agent_context() -> TestContext {
    let mut ctx = TestContext::new();
    let mut context = ctx
        .conn
        .new_browser_context_fixture_for_test("BID-document-agents");
    context.set_active_target_id("TID-document-agents");
    ctx.conn.install_browser_context_fixture_for_test(context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<style>button { color: rgb(1, 2, 3); }</style><button id='inspected'>Inspect me</button>",
        None,
    ).await;
    ctx
}

async fn document_agent_round_trip(method: &str, start_before_move: bool) {
    let mut ctx = document_agent_context().await;
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let document_handle = |ctx: &mut TestContext| inspection_document_handle(&ctx.conn, &owner);
    let mut document = (!start_before_move).then(|| document_handle(&mut ctx));
    let step = ctx.conn.start_command_dispatch(
        &json!({"id": 1, "method": method, "params": {"computedStyles": ["color"]}}).to_string(),
    );
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "{method} must start on its live binding"
    );
    if start_before_move {
        document = Some(document_handle(&mut ctx));
    }
    let document = document.unwrap();
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(1))
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "{method} completion: {response}"
    );

    match method {
        "CSS.enable" => {
            let sheet = messages
                .iter()
                .find(|message| message["method"] == json!("CSS.styleSheetAdded"))
                .expect("CSS enable must publish the bound renderer inventory")["params"]["header"]
                ["styleSheetId"]
                .clone();
            let root = dom_command(&mut ctx, 2, "DOM.getDocument", json!({"depth": -1})).await;
            let node = dom_command(
                &mut ctx,
                3,
                "DOM.querySelector",
                json!({"nodeId": root["root"]["nodeId"], "selector": "#inspected"}),
            )
            .await["nodeId"]
                .clone();
            let style = dom_command(
                &mut ctx,
                4,
                "CSS.getComputedStyleForNode",
                json!({"nodeId": node}),
            )
            .await;
            assert!(
                style["computedStyle"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|property| property == &json!({"name": "color", "value": "rgb(1, 2, 3)"}))
            );
            dom_command(
                &mut ctx,
                5,
                "CSS.setStyleSheetText",
                json!({"styleSheetId": sheet, "text": "button { color: rgb(4, 5, 6); }"}),
            )
            .await;
            let style = dom_command(
                &mut ctx,
                6,
                "CSS.getStyleSheet",
                json!({"styleSheetId": sheet}),
            )
            .await;
            assert_eq!(
                style["styleSheet"]["text"],
                json!("button { color: rgb(4, 5, 6); }")
            );
            let actual = document
                .evaluate_runtime_expression_for_test(
                    "getComputedStyle(document.getElementById('inspected')).color",
                    false,
                )
                .await
                .unwrap();
            assert_eq!(actual["value"], json!("rgb(4, 5, 6)"));
            dom_command(&mut ctx, 7, "CSS.disable", json!({})).await;
            let replay = dom_exchange(&mut ctx, 8, "CSS.enable", json!({})).await;
            assert!(
                replay
                    .iter()
                    .any(|message| message["method"] == json!("CSS.styleSheetAdded")),
                "CSS disable must reset the renderer agent inventory"
            );
        }
        "Accessibility.getFullAXTree" => {
            assert!(
                response["result"]["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|node| node["role"]["value"] == json!("button")
                        && node["name"]["value"] == json!("Inspect me"))
            );
            let root = dom_command(&mut ctx, 2, "DOM.getDocument", json!({"depth": -1})).await;
            let node = dom_command(
                &mut ctx,
                3,
                "DOM.querySelector",
                json!({"nodeId": root["root"]["nodeId"], "selector": "#inspected"}),
            )
            .await["nodeId"]
                .clone();
            let partial = dom_command(
                &mut ctx,
                4,
                "Accessibility.getPartialAXTree",
                json!({"nodeId": node, "fetchRelatives": false}),
            )
            .await;
            assert!(
                partial["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|node| node["name"]["value"] == json!("Inspect me"))
            );
            let object = dom_command(&mut ctx, 5, "DOM.resolveNode", json!({"nodeId": node})).await;
            let query = dom_command(
                &mut ctx,
                6,
                "Accessibility.queryAXTree",
                json!({"objectId": object["object"]["objectId"], "role": "button"}),
            )
            .await;
            assert_eq!(query["nodes"].as_array().unwrap().len(), 1);
            assert_eq!(query["nodes"][0]["name"]["value"], json!("Inspect me"));
        }
        "DOMSnapshot.captureSnapshot" => {
            let result = &response["result"];
            assert_eq!(result["documents"].as_array().unwrap().len(), 1);
            let strings = result["strings"].as_array().unwrap();
            assert!(strings.contains(&json!("BUTTON")));
            assert!(strings.contains(&json!("Inspect me")));
            assert!(strings.contains(&json!("rgb(1, 2, 3)")));
        }
        _ => unreachable!(),
    }
    assert!(ctx.conn.has_loaded_page_for_owner(&owner));
}

#[tokio::test(flavor = "multi_thread")]
async fn document_agents_reject_old_completion_and_follow_up_after_rebind() {
    for method in [
        "CSS.getComputedStyleForNode",
        "Accessibility.getPartialAXTree",
        "DOMSnapshot.captureSnapshot",
    ] {
        let mut ctx = document_agent_context().await;
        let root = dom_command(&mut ctx, 10, "DOM.getDocument", json!({"depth": -1})).await;
        let node = dom_command(
            &mut ctx,
            11,
            "DOM.querySelector",
            json!({"nodeId": root["root"]["nodeId"], "selector": "#inspected"}),
        )
        .await["nodeId"]
            .clone();
        let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(
            &json!({"id": 12, "method": method, "params": {"nodeId": node, "computedStyles": []}})
                .to_string(),
        ) else {
            panic!("{method} must start on the old renderer")
        };
        let completed = pending.wait().await;
        let owner = CommandOwnerScope::capture(&ctx.conn, None);
        let old_document = inspection_document_handle(&ctx.conn, &owner);
        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<button>Replacement</button>",
            None,
        )
        .await;
        let step = ctx.conn.complete_pending_command_dispatch(completed).await;
        assert!(
            !matches!(&step, CdpCommandTaskStep::Pending(_)),
            "{method}: stale lookup must not dispatch to the replacement"
        );
        let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
        let responses: Vec<_> = messages
            .iter()
            .filter(|message| message["id"] == json!(12))
            .collect();
        assert_eq!(
            responses,
            vec![
                &json!({"id": 12, "error": {"code": -32000, "message": "Renderer attachment changed"}})
            ],
            "{method}"
        );
        drop(old_document);
    }
}
