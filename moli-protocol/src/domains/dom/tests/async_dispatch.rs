use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_box_quads_and_scroll_support_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box' style='position:absolute;left:3px;top:4px;width:9px;height:11px'></div></body></html>",
    )
    .await;
    ctx.capture_fixture_layout(None).await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('box')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!object_id.is_empty());

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.getBoxModel",
        "params": { "objectId": object_id.clone() }
    }))
    .await;
    ctx.expect_result(12, axis_aligned_box_model(3.0, 4.0, 9, 11), None);

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getContentQuads",
        "params": { "objectId": object_id.clone() }
    }))
    .await;
    ctx.expect_result(
        13,
        json!({ "quads": [axis_aligned_geometry_quad(3.0, 4.0, 9.0, 11.0)] }),
        None,
    );

    ctx.process_async(json!({
        "id": 14,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    }))
    .await;
    ctx.expect_result(14, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn native_dom_commands_preserve_remote_object_aliases() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-ALIAS");
    navigate_to_data_html_async(&mut ctx, 1,
        "<!doctype html><div id='box' tabindex='0' style='position:absolute;left:3px;top:4px;width:9px;height:11px'></div>").await;
    ctx.capture_fixture_layout(None).await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    ctx.take_all();
    ctx.process_async(json!({"id": 11, "method": "Runtime.evaluate",
        "params": {"expression": "document.getElementById('box')"}}))
        .await;
    let evaluated = take_response_by_id(&mut ctx, 11);
    let object = evaluated["result"]["result"]["objectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let owner = ctx.with_fixture(|conn| crate::conn::CommandOwnerScope::capture(conn, None));
    ctx.with_fixture(|conn| {
        conn.register_runtime_remote_object_alias_for_owner_with_realm(
            &owner,
            "native-node-alias".to_owned(),
            object,
            "alias-realm",
        )
    });
    ctx.process_async(json!({"id": 9, "method": "DOM.getDocument"}))
        .await;
    ctx.take_all();
    for (id, method) in [
        (12, "DOM.describeNode"),
        (13, "DOM.requestNode"),
        (14, "DOM.getOuterHTML"),
        (15, "DOM.getBoxModel"),
        (16, "DOM.getContentQuads"),
        (17, "DOM.focus"),
        (18, "DOM.scrollIntoViewIfNeeded"),
    ] {
        ctx.process_async(json!({"id": id, "method": method,
            "params": {"objectId": "native-node-alias"}}))
            .await;
        let response = take_response_by_id(&mut ctx, id);
        assert!(response.get("error").is_none(), "{method}: {response}");
        match id {
            12 => assert_eq!(response["result"]["node"]["nodeName"], "DIV"),
            13 => assert!(response["result"]["nodeId"].as_u64().unwrap() > 0),
            14 => assert!(
                response["result"]["outerHTML"]
                    .as_str()
                    .unwrap()
                    .contains("box")
            ),
            15 => assert_eq!(response["result"], axis_aligned_box_model(3.0, 4.0, 9, 11)),
            16 => assert_eq!(
                response["result"],
                json!({"quads": [axis_aligned_geometry_quad(3.0, 4.0, 9.0, 11.0)]})
            ),
            _ => assert_eq!(response["result"], json!({})),
        }
        ctx.take_all();
    }
    ctx.process_async(json!({"id": 19, "method": "Runtime.evaluate",
        "params": {"expression": "document.activeElement.id", "returnByValue": true}}))
        .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 19)["result"]["result"]["value"],
        "box"
    );
}
