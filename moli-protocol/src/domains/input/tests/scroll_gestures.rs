use super::*;
use serde_json::Value;

async fn fixture(script: &str) -> TestContext {
    let mut ctx = TestContext::new_with_layout_policy(LayoutPolicy::Realtime);
    with_loaded_document(&mut ctx, &format!(r#"<html><body style='margin:0;height:2400px'>
        <div id='a' style='width:160px;height:120px;overflow:auto'>
          <div id='content' style='height:1800px;width:100px'>content</div>
        </div>
        <div id='b' style='position:absolute;left:0;top:0;width:160px;height:120px;overflow:auto;display:none'>
          <div style='height:1800px'>overlay</div>
        </div>
        <script>
        window.events=[];window.total=0;
        const a=document.getElementById('a'),b=document.getElementById('b');
        {script}
        </script></body></html>"#)).await;
    ctx.capture_fixture_layout(None).await;
    ctx
}

async fn gesture(ctx: &mut TestContext, extra: Value) {
    let mut params =
        json!({"x":20,"y":20,"yDistance":-300,"speed":800,"gestureSourceType":"mouse"});
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    ctx.process_async(json!({"id":35000,"method":"Input.synthesizeScrollGesture","params":params}))
        .await;
    ctx.expect_result(35000, json!({}), None);
}

async fn snapshot(ctx: &mut TestContext) -> Value {
    serde_json::from_str(
        &evaluate_string(
            ctx,
            "JSON.stringify({events,total,a:a.scrollTop,b:b.scrollTop,page:scrollY})",
        )
        .await,
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_delivers_complete_canceled_wheel_sequence() {
    let mut ctx=fixture("a.addEventListener('wheel',e=>{events.push(e.cancelable);total+=e.deltaY;e.preventDefault();},{passive:false});").await;
    gesture(&mut ctx, json!({})).await;
    let value = snapshot(&mut ctx).await;
    assert!(
        (value["total"].as_f64().unwrap() - 300.0).abs() < 0.001,
        "{value}"
    );
    assert!(
        value["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v == true),
        "{value}"
    );
    assert_eq!(value["a"], 0);
    assert_eq!(value["page"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_continues_after_only_first_wheel_is_canceled() {
    let mut ctx=fixture("a.addEventListener('wheel',e=>{events.push([e.cancelable,e.deltaY]);total+=e.deltaY;if(events.length===1)e.preventDefault();},{passive:false});").await;
    gesture(&mut ctx, json!({})).await;
    let value = snapshot(&mut ctx).await;
    let events = value["events"].as_array().unwrap();
    assert!(events.len() > 1, "{value}");
    assert!(events.iter().all(|v| v[0] == true), "{value}");
    assert!(
        (value["total"].as_f64().unwrap() - 300.0).abs() < 0.001,
        "{value}"
    );
    let first = events[0][1].as_f64().unwrap();
    assert!(
        (value["a"].as_f64().unwrap() - (300.0 - first)).abs() < 0.05,
        "{value}"
    );
    assert_eq!(value["page"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_keeps_target_and_scroller_when_overlay_appears() {
    let mut ctx=fixture("document.addEventListener('wheel',e=>{events.push([e.target.id,e.cancelable]);total+=e.deltaY;b.style.display='block';},{passive:false});").await;
    gesture(&mut ctx, json!({})).await;
    let value = snapshot(&mut ctx).await;
    assert!(
        value["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v[0] == "content"),
        "{value}"
    );
    assert!(
        (value["a"].as_f64().unwrap() - 300.0).abs() < 0.05,
        "{value}"
    );
    assert_eq!(value["b"], 0);
    assert_eq!(value["page"], 0);
    // A separate gesture must hit the overlay and start cancelable again.
    assert!(evaluate_bool(&mut ctx, "events=[];total=0;true").await);
    gesture(&mut ctx, json!({})).await;
    let value = snapshot(&mut ctx).await;
    assert_eq!(value["events"][0][1], true);
    assert!(
        value["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v[0] == ""),
        "{value}"
    );
    assert!(
        (value["b"].as_f64().unwrap() - 300.0).abs() < 0.05,
        "{value}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_repeat_resets_target_and_cancellation_mode() {
    let mut ctx=fixture("document.addEventListener('wheel',e=>{events.push([e.target.id,e.cancelable]);total+=e.deltaY;if(events.length===1)e.preventDefault();if(total>=299.999)b.style.display='block';},{passive:false});").await;
    gesture(&mut ctx, json!({"repeatCount":1,"repeatDelayMs":0})).await;
    let value = snapshot(&mut ctx).await;
    assert!(
        (value["total"].as_f64().unwrap() - 600.0).abs() < 0.001,
        "{value}"
    );
    assert!(
        value["a"].as_f64().unwrap() > 0.0 && value["a"].as_f64().unwrap() < 300.0,
        "{value}"
    );
    assert!(
        (value["b"].as_f64().unwrap() - 300.0).abs() < 0.05,
        "{value}"
    );
    let events = value["events"].as_array().unwrap();
    assert_eq!(
        events.iter().find(|e| e[0] == "").unwrap()[1],
        true,
        "{value}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_retargets_after_cancellation_and_target_removal() {
    for remove in [false, true] {
        let mut ctx=fixture(&format!("document.addEventListener('wheel',e=>{{events.push([e.target.id,e.cancelable,e.deltaY]);total+=e.deltaY;if(events.length===1){{e.preventDefault();b.style.display='block';{} }} }},{{passive:false}});",if remove { "a.remove();" } else { "" })).await;
        gesture(&mut ctx, json!({})).await;
        let value = snapshot(&mut ctx).await;
        assert!(
            (value["total"].as_f64().unwrap() - 300.0).abs() < 0.001,
            "{value}"
        );
        let events = value["events"].as_array().unwrap();
        let first = events[0][2].as_f64().unwrap();
        assert!(
            events.len() > 1 && events[1..].iter().all(|e| e[0] == "" && e[1] == true),
            "{value}"
        );
        assert!(
            (value["b"].as_f64().unwrap() - (300.0 - first)).abs() < 0.05,
            "{value}"
        );
        assert_eq!(value["a"], 0);
        assert_eq!(value["page"], 0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_does_not_chain_after_latched_scroller_hits_boundary_or_is_removed() {
    for script in [
        "document.getElementById('content').style.height='150px';document.addEventListener('wheel',e=>{events.push(e.target.id);total+=e.deltaY;},{passive:false});",
        "document.addEventListener('wheel',e=>{events.push(e.target.id);total+=e.deltaY;if(events.length===1){a.remove();b.style.display='block';}},{passive:false});",
    ] {
        let mut ctx = fixture(script).await;
        gesture(&mut ctx, json!({})).await;
        let value = snapshot(&mut ctx).await;
        assert!(
            (value["total"].as_f64().unwrap() - 300.0).abs() < 0.001,
            "{value}"
        );
        assert_eq!(value["page"], 0, "{value}");
        assert_eq!(value["b"], 0, "{value}");
        if script.starts_with("document.getElementById") {
            assert_eq!(value["a"], 30, "{value}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_coalesces_progress_when_wheel_handler_is_slow() {
    // Each handler deliberately occupies at least 50ms. A 375ms gesture should
    // catch up in at most nine dispatches rather than sending all 23 original
    // fixed steps. Assert the dispatch count, not a wall-clock timeout.
    let mut ctx=fixture("a.addEventListener('wheel',e=>{events.push(e.deltaY);total+=e.deltaY;const end=performance.now()+50;while(performance.now()<end){}},{passive:false});").await;
    gesture(&mut ctx, json!({})).await;
    let value = snapshot(&mut ctx).await;
    assert!(value["events"].as_array().unwrap().len() <= 9, "{value}");
    assert!(
        (value["total"].as_f64().unwrap() - 300.0).abs() < 0.001,
        "{value}"
    );
    assert!(
        (value["a"].as_f64().unwrap() - 300.0).abs() < 0.05,
        "{value}"
    );
    assert_eq!(value["page"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_checks_visible_widget_bounds_after_layout_resize() {
    let mut ctx = fixture("document.addEventListener('wheel',e=>total+=e.deltaY);").await;
    for (id, width, height) in [(35001, 640, 480), (35002, 1000, 0)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Emulation.setDeviceMetricsOverride",
            "params": { "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
    }
    assert!(evaluate_bool(&mut ctx, "innerWidth === 1000").await);
    for (id, x, y) in [(35003, 800, 100), (35004, 640, 100), (35005, 20, 480)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Input.synthesizeScrollGesture",
            "params": { "x": x, "y": y, "yDistance": -10 }
        }))
        .await;
        ctx.expect_error(id, -32000, "Position out of bounds");
    }
    assert!(evaluate_bool(&mut ctx, "total === 0 && scrollY === 0").await);
    gesture(&mut ctx, json!({"x":639,"y":100,"yDistance":-10})).await;
    assert!(evaluate_bool(&mut ctx, "total === 10 && scrollY === 10").await);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_preserves_intercepted_drag() {
    let mut ctx = TestContext::new_with_layout_policy(LayoutPolicy::Realtime);
    with_loaded_document(&mut ctx, DRAG_PAGE).await;
    assert!(
        evaluate_bool(
            &mut ctx,
            "target.style.overflow='auto';target.innerHTML='<div style=\"height:500px\"></div>';true",
        )
        .await
    );
    ctx.capture_fixture_layout(None).await;
    ctx.process_async(json!({
        "id": 35001, "method": "Input.setInterceptDrags", "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(35001, json!({}), None);
    dispatch_drag_mouse(&mut ctx, 35002, "mousePressed", 20, 1).await;
    dispatch_drag_mouse(&mut ctx, 35003, "mouseMoved", 120, 1).await;
    let intercepted = ctx
        .wait_for_scheduler_message("intercepted drag", |message| {
            message["method"] == "Input.dragIntercepted"
        })
        .await;
    for (id, kind) in [(35004, "dragEnter"), (35005, "dragOver"), (35006, "drop")] {
        ctx.process_async(json!({
            "id": id, "method": "Input.dispatchDragEvent",
            "params": { "type": kind, "x": INPUT_HIT_X, "y": 120,
                        "data": intercepted["params"]["data"] }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
        if kind == "dragEnter" {
            gesture(&mut ctx, json!({"y":120,"yDistance":-40})).await;
            assert!(evaluate_bool(&mut ctx, "target.scrollTop === 40").await);
            assert!(evaluate_bool(&mut ctx, "!window.__drag.includes('dragend')").await);
        }
    }
    assert!(
        evaluate_bool(
            &mut ctx,
            "window.__drag.slice(-2).join('|') === 'drop:custom|dragend'",
        )
        .await
    );
}
