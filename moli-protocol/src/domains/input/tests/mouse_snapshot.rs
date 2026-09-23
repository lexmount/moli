use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn cdp_mouse_requires_a_snapshot_then_consumes_geometry_and_screenshot_publications() {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, r#"<html><body style='margin:0;min-height:100px'>
        <button id='target' style='position:absolute;left:0;top:0;width:100px;height:100px'>go</button>
        <script>
        window.events=[];
        for(const type of ['mousemove','mousedown','mouseup','click','wheel'])
            document.addEventListener(type,e=>events.push([type,e.target.id]));
        </script></body></html>"#).await;
    for (id, event) in [
        (701, "mouseMoved"),
        (702, "mousePressed"),
        (703, "mouseReleased"),
        (704, "mouseWheel"),
    ] {
        ctx.process_async(json!({
            "id":id, "method":"Input.dispatchMouseEvent",
            "params":{"type":event,"x":20,"y":20,"button":"left","deltaX":0,"deltaY":10}
        }))
        .await;
        let response = ctx.take_response_by_id(id);
        assert_eq!(response["error"]["code"], -32000, "{response}");
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("coordinate input requires an existing layout snapshot"),
            "{response}"
        );
    }
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(events)").await,
        "[]"
    );

    let object_id = resolve_selector_object_id(&mut ctx, "#target", 710).await;
    ctx.process_async(json!({"id":720,"method":"DOM.getBoxModel","params":{"objectId":object_id}}))
        .await;
    assert!(ctx.take_response_by_id(720)["result"]["model"].is_object());
    assert!(
        evaluate_bool(
            &mut ctx,
            "document.getElementById('target').style.left='300px';true"
        )
        .await
    );
    for (id, event, buttons) in [
        (721, "mouseMoved", 0),
        (722, "mousePressed", 1),
        (723, "mouseReleased", 0),
    ] {
        ctx.process_async(json!({
            "id":id,"method":"Input.dispatchMouseEvent",
            "params":{"type":event,"x":20,"y":20,"button":"left","buttons":buttons,"clickCount":1}
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
    }
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(events)").await,
        r#"[["mousemove","target"],["mousedown","target"],["mouseup","target"],["click","target"]]"#
    );

    ctx.process_async(json!({"id":724,"method":"Page.captureScreenshot"}))
        .await;
    assert!(
        ctx.take_response_by_id(724)["result"]["data"]
            .as_str()
            .is_some_and(|data| !data.is_empty())
    );
    for (id, x) in [(725, 20), (726, 320)] {
        ctx.process_async(json!({"id":id,"method":"Input.dispatchMouseEvent","params":{"type":"mouseMoved","x":x,"y":20}})).await;
        ctx.expect_result(id, json!({}), None);
    }
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify(events.slice(-2))").await,
        r#"[["mousemove",""],["mousemove","target"]]"#
    );
}
