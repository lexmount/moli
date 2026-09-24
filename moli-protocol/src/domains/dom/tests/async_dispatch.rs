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
