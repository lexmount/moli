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
async fn direct_wheel_chains_the_delta_remaining_at_a_nested_boundary() {
    let mut ctx = fixture("document.getElementById('content').style.height = '150px';").await;
    ctx.process_async(json!({
        "id":35001,"method":"Input.dispatchMouseEvent",
        "params":{"type":"mouseWheel","x":20,"y":20,"deltaX":0,"deltaY":100}
    }))
    .await;
    ctx.expect_result(35001, json!({}), None);
    let value = snapshot(&mut ctx).await;
    assert_eq!(value["a"], 30, "{value}");
    assert_eq!(value["page"], 70, "{value}");
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
async fn scroll_gesture_interrupted_by_ignored_input_restarts_at_new_target() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut ctx = fixture(
                r#"
                b.firstElementChild.id = 'overlay';
                window.firstScroll = new Promise(resolve => a.addEventListener('scroll', () => {
                    b.style.display = 'block';
                    resolve(true);
                }, {once: true}));
                document.addEventListener('wheel', event => {
                    events.push([event.target.id, event.cancelable]);
                    total += event.deltaY;
                }, {passive: false});
            "#,
            )
            .await;
            let raw = json!({
                "id":35001, "method":"Input.synthesizeScrollGesture",
                "params":{"x":20,"y":20,"yDistance":-1200,"speed":300,"gestureSourceType":"mouse"}
            })
            .to_string();
            let CdpCommandTaskStep::Pending(pending) =
                ctx.with_fixture(|conn| conn.start_command_dispatch(&raw))
            else {
                panic!("gesture must wait for its wheel events");
            };
            let running = tokio::task::spawn_local(pending.wait());
            ctx.process_async(json!({
                "id":35002, "method":"Runtime.evaluate",
                "params":{"expression":"firstScroll","awaitPromise":true,"returnByValue":true}
            }))
            .await;
            let first = ctx
                .wait_for_scheduler_message("first default scroll", |message| {
                    message["id"] == 35002
                })
                .await;
            assert_eq!(first["result"]["result"]["value"], true, "{first}");

            ctx.process_async(json!({
                "id":35003, "method":"Input.setIgnoreInputEvents", "params":{"ignore":true}
            }))
            .await;
            ctx.expect_result(35003, json!({}), None);
            let completed = ctx.drive_owned_wait(running).await.unwrap();
            let step = ctx.with_fixture(|conn| {
                conn.complete_command_dispatch_with_context(
                    completed,
                    &mut CommandDispatchContext::default(),
                )
            });
            let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
            assert!(
                messages.contains(&json!({"id":35001,"result":{}})),
                "{messages:?}"
            );
            let interrupted = snapshot(&mut ctx).await;
            assert!(interrupted["a"].as_f64().unwrap() > 0., "{interrupted}");
            assert!(
                interrupted["total"].as_f64().unwrap() < 1200.,
                "{interrupted}"
            );
            assert_eq!(interrupted["b"], 0, "{interrupted}");
            let resume_index = interrupted["events"].as_array().unwrap().len();

            ctx.process_async(json!({
                "id":35004, "method":"Input.setIgnoreInputEvents", "params":{"ignore":false}
            }))
            .await;
            ctx.expect_result(35004, json!({}), None);
            // Stay within the old transaction's position slop and keep its direction.
            gesture(&mut ctx, json!({"x":25,"yDistance":-24,"speed":2000})).await;
            let resumed = snapshot(&mut ctx).await;
            assert_eq!(
                resumed["events"][resume_index],
                json!(["overlay", true]),
                "{resumed}"
            );
            assert_eq!(resumed["a"], interrupted["a"], "{resumed}");
            assert_eq!(resumed["b"], 24, "{resumed}");
            assert_eq!(resumed["page"], 0, "{resumed}");
        })
        .await;
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
async fn scroll_gesture_repeat_starts_cancelable_after_skipping_an_outside_update() {
    use crate::conn::{CommandOwnerScope, ProtocolWorkflowDriver};
    use moli_core::page::RendererSyntheticWheelGesture;

    let mut ctx = fixture(
        "a.addEventListener('wheel', e => {
            e.preventDefault();
            events.push([e.deltaY, e.cancelable, e.defaultPrevented]);
        }, {passive:false});",
    )
    .await;
    let (owner, page) = ctx.with_fixture(|conn| {
        let owner = CommandOwnerScope::capture(conn, None);
        let page = conn
            .target_page_residence_identity_for_owner(&owner)
            .unwrap();
        (owner, page)
    });
    let command = RendererSyntheticWheelGesture::default();
    let first = SyntheticWheel {
        gesture: command.next_sequence(),
        owner,
        page,
        position: (120.0, 20.0),
        delta: (0.0, 25.0),
        end: false,
    };
    let repeat = SyntheticWheel {
        gesture: command.next_sequence(),
        ..first.clone()
    };
    let mut driver = ProtocolWorkflowDriver::new(ctx.handle());
    let mut context = CommandDispatchContext::default();

    // Run the normal protocol update path with a resize between each step.
    for (id, width, delta) in [(35001, 600, 25.0), (35002, 100, 50.0), (35003, 600, 75.0)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Emulation.setDeviceMetricsOverride",
            "params": { "width": width, "height": 400, "deviceScaleFactor": 1, "mobile": false }
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
        let sequence = if id == 35001 { &first } else { &repeat };
        let update = SyntheticWheel {
            delta: (0.0, delta),
            ..sequence.clone()
        };
        assert!(
            ctx.drive_owned_wait(update.dispatch(&mut driver, &mut context))
                .await
                .unwrap(),
        );
    }
    let update = SyntheticWheel {
        delta: (0.0, 100.0),
        ..repeat
    };
    assert!(
        ctx.drive_owned_wait(update.dispatch(&mut driver, &mut context))
            .await
            .unwrap(),
    );

    let value = snapshot(&mut ctx).await;
    assert_eq!(
        value["events"],
        json!([[25, true, true], [75, true, true], [100, true, true]]),
        "{value}"
    );
    assert_eq!(value["a"], 0, "{value}");
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
async fn scroll_gesture_latches_viewport_at_boundary_for_reverse_overscroll() {
    let mut ctx = TestContext::new_with_layout_policy(LayoutPolicy::Realtime);
    with_loaded_document(
        &mut ctx,
        "<!doctype html><body style='margin:0;height:4000px'>content</body>",
    )
    .await;
    ctx.capture_fixture_layout(None).await;

    gesture(
        &mut ctx,
        json!({"x":40,"y":40,"yDistance":100,"yOverscroll":100}),
    )
    .await;

    let scroll_y: f64 = evaluate_string(&mut ctx, "String(scrollY)")
        .await
        .parse()
        .unwrap();
    assert!((scroll_y - 100.0).abs() < 0.05, "scrollY was {scroll_y}");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_relatches_after_an_unconsumed_initial_update() {
    for (initial_page, expected_inner, expected_page) in [(0, 100.0, 0.0), (20, 0.0, 100.0)] {
        let mut ctx = fixture("").await;
        assert!(
            evaluate_bool(
                &mut ctx,
                &format!("scrollTo(0, {initial_page}); scrollY === {initial_page}"),
            )
            .await
        );
        gesture(
            &mut ctx,
            json!({"yDistance":100,"yOverscroll":100,"speed":200}),
        )
        .await;

        let value = snapshot(&mut ctx).await;
        assert!(
            (value["a"].as_f64().unwrap() - expected_inner).abs() < 0.05,
            "initial scrollY {initial_page}: {value}"
        );
        assert!(
            (value["page"].as_f64().unwrap() - expected_page).abs() < 0.05,
            "initial scrollY {initial_page}: {value}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_latches_one_scroller_for_diagonal_delta() {
    let mut ctx = TestContext::new_with_layout_policy(LayoutPolicy::Realtime);
    with_loaded_document(
        &mut ctx,
        r#"<!doctype html><html style="overflow:hidden"><body style="margin:0;overflow:hidden">
        <div id="outer" style="width:200px;height:200px;overflow:auto">
          <div style="width:1000px">
            <div id="inner" style="width:160px;height:160px;overflow:auto">
              <div style="height:1500px">content</div>
            </div>
          </div>
        </div></body></html>"#,
    )
    .await;
    ctx.capture_fixture_layout(None).await;

    gesture(&mut ctx, json!({"xDistance":-120,"yDistance":-120})).await;

    let positions = evaluate_string(
        &mut ctx,
        "JSON.stringify([document.getElementById('outer').scrollLeft, document.getElementById('inner').scrollTop])",
    )
    .await;
    let (outer_x, inner_y): (f64, f64) = serde_json::from_str(&positions).unwrap();
    assert_eq!(outer_x, 0.0, "outer container scrolled horizontally");
    assert!(
        (inner_y - 120.0).abs() < 0.05,
        "inner scrollTop was {inner_y}"
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gestures_share_a_page_queue_and_repeats_give_up_the_turn() {
    let mut ctx = fixture("a.addEventListener('wheel',e=>events.push(e.deltaY));").await;
    let mut sessions = Vec::new();
    for id in [35001, 35002] {
        ctx.process_async(json!({
            "id": id, "method": "Target.attachToTarget",
            "params": { "targetId": "TID-1", "flatten": true }
        }))
        .await;
        sessions.push(
            ctx.take_response_by_id(id)["result"]["sessionId"]
                .as_str()
                .expect("attached session ID")
                .to_owned(),
        );
    }
    let [first, second] = [300, -300].map(|distance| {
        let first = distance > 0;
        let raw = json!({
            "id": if first { 35003 } else { 35004 },
            "sessionId": sessions[usize::from(!first)],
            "method": "Input.synthesizeScrollGesture",
            "params": { "x":20,"y":20,"yDistance":distance,"speed":1200,
                        "repeatCount":u32::from(first),"repeatDelayMs":150 }
        })
        .to_string();
        match ctx.with_fixture(|conn| conn.start_command_dispatch(&raw)) {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(_) => panic!("a gesture must wait for its wheel events"),
        }
    });
    let (first, second) = ctx
        .drive_owned_wait(async { tokio::join!(first.wait(), second.wait()) })
        .await;
    for (completed, id, session) in [(first, 35003, &sessions[0]), (second, 35004, &sessions[1])] {
        let outcome = ctx.with_fixture(|conn| {
            conn.complete_command_dispatch_with_context(
                completed,
                &mut CommandDispatchContext::default(),
            )
        });
        let (messages, _) = ctx.complete_command_task_step_for_test(outcome).await;
        assert!(messages.contains(&json!({"id": id, "result": {}, "sessionId": session})));
    }
    let value = snapshot(&mut ctx).await;
    let mut runs: Vec<f64> = Vec::new();
    for event in value["events"].as_array().unwrap() {
        let delta = event.as_f64().unwrap();
        if let Some(total) = runs.last_mut()
            && total.signum() == delta.signum()
        {
            *total += delta;
        } else {
            runs.push(delta);
        }
    }
    assert_eq!(runs.len(), 3, "{runs:?}");
    for (sum, expected) in runs.into_iter().zip([-300.0, 300.0, -300.0]) {
        assert!((sum - expected).abs() < 0.001, "{value}");
    }
    assert!(value["a"].as_f64().unwrap().abs() < 0.05, "{value}");
    assert_eq!(value["page"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_releases_document_work_before_its_response() {
    for (action, messages, scroll_y) in [
        (
            r#"location.href = "javascript:void(console.log('wheel javascript'))";"#,
            &["wheel javascript"][..],
            "0",
        ),
        (
            r#"document.open();
               document.write("<body style='height:6000px;margin:0'><script>addEventListener('DOMContentLoaded',()=>console.log('wheel DCL'));addEventListener('load',()=>console.log('wheel load'));addEventListener('wheel',()=>console.log('wheel after replacement'));<\/script>");
               document.close();"#,
            &["wheel DCL", "wheel load", "wheel after replacement"][..],
            "2",
        ),
    ] {
        let mut ctx = fixture(&format!(
            "a.addEventListener('wheel',()=>{{{action}}},{{once:true}});"
        ))
        .await;
        ctx.process_async(json!({"id":35001,"method":"Runtime.enable"}))
            .await;
        ctx.expect_result(35001, json!({}), None);
        let response_flush = ctx
            .process_command_holding_response_flush_for_test(json!({
                "id":35000,"method":"Input.synthesizeScrollGesture",
                "params":{"x":20,"y":20,"yDistance":-2,"repeatCount":1,"repeatDelayMs":800}
            }))
            .await;
        let response = ctx.sent.iter().position(|msg| msg["id"] == 35000).unwrap();
        for message in messages {
            assert!(
                ctx.sent[..response].iter().any(|msg| {
                    msg["method"] == "Runtime.consoleAPICalled"
                        && msg["params"]["args"][0]["value"] == *message
                }),
                "expected {message} before the gesture response, got {:?}",
                ctx.sent
            );
        }
        ctx.expect_result(35000, json!({}), None);
        ctx.finish_held_command_response_flush_for_test(response_flush)
            .await;
        assert_eq!(evaluate_string(&mut ctx, "String(scrollY)").await, scroll_y);
    }
}

async fn set_gesture_viewport(ctx: &mut TestContext, width: u32) {
    ctx.process_async(json!({
        "id":35010,"method":"Emulation.setDeviceMetricsOverride",
        "params":{"width":width,"height":400,"deviceScaleFactor":1,"mobile":false}
    }))
    .await;
    ctx.expect_result(35010, json!({}), None);
}

async fn clipped_repeat(cancel_first_round: bool) {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut ctx = fixture(&format!(
                r#"
            window.firstRoundDone=new Promise(resolve=>window.finishFirstRound=resolve);
            window.restored=false;window.firstFinished=false;
            document.addEventListener('wheel',e=>{{
                if({cancel_first_round} || firstFinished)e.preventDefault();
                events.push([e.cancelable,e.defaultPrevented,e.deltaY,restored]);
                total+=e.deltaY;
                if(!firstFinished && total>=299.999){{
                    firstFinished=true;finishFirstRound();
                }}
            }},{{passive:false}});
        "#
            ))
            .await;
            set_gesture_viewport(&mut ctx, 600).await;
            let raw = json!({"id":35000,"method":"Input.synthesizeScrollGesture",
            "params":{"x":500,"y":40,"yDistance":-300,"speed":300,
                      "repeatCount":1,"repeatDelayMs":600,"gestureSourceType":"mouse"}})
            .to_string();
            let CdpCommandTaskStep::Pending(pending) =
                ctx.with_fixture(|conn| conn.start_command_dispatch(&raw))
            else {
                panic!("gesture must run through the owned protocol workflow");
            };
            let running = tokio::task::spawn_local(pending.wait());
            ctx.process_async(json!({"id":35011,"method":"Runtime.evaluate",
            "params":{"expression":"firstRoundDone","awaitPromise":true}}))
                .await;
            let first_round = ctx
                .wait_for_scheduler_message("first wheel round completion", |message| {
                    message["id"] == 35011
                })
                .await;
            assert!(first_round.get("error").is_none(), "{first_round}");
            assert!(
                first_round["result"].get("exceptionDetails").is_none(),
                "{first_round}"
            );
            set_gesture_viewport(&mut ctx, 300).await;
            assert!(evaluate_bool(&mut ctx, "scrollTo(0,0);true").await);
            // Define the viewport input schedule: the 600ms repeat delay
            // exceeds Aura's 500ms transaction timeout even if the previous
            // producer End was clipped. Then clip 200ms of the new round.
            ctx.drive_owned_wait(tokio::time::sleep(std::time::Duration::from_millis(800)))
                .await;
            assert!(
                !running.is_finished(),
                "the repeat must still have input remaining"
            );
            assert!(evaluate_bool(&mut ctx, "window.restored=true;true").await);
            set_gesture_viewport(&mut ctx, 600).await;
            let completed = ctx.drive_owned_wait(running).await.unwrap();
            let step = ctx.with_fixture(|conn| {
                conn.complete_command_dispatch_with_context(
                    completed,
                    &mut CommandDispatchContext::default(),
                )
            });
            let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
            assert!(
                messages.contains(&json!({"id":35000,"result":{}})),
                "{messages:?}"
            );
            let value = snapshot(&mut ctx).await;
            let total = value["total"].as_f64().unwrap();
            assert!(
                (300.0..599.0).contains(&total),
                "must observe both clipped and restored repeat updates: {value}"
            );
            let resumed = value["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[3] == true)
                .collect::<Vec<_>>();
            assert!(total > 300.0 && !resumed.is_empty(), "{value}");
            for event in resumed {
                assert_eq!(
                    event[0], true,
                    "the first actual platform wheel begins a transaction: {value}"
                );
                assert_eq!(event[1], true, "{value}");
            }
            assert_eq!(
                value["page"], 0,
                "restored canceled wheels must not scroll: {value}"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_clipped_repeat_keeps_canceled_wheels_cancelable() {
    clipped_repeat(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_clipped_repeat_infers_began_after_prior_nonblocking_round() {
    clipped_repeat(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_clipped_queued_command_infers_began_after_previous_end() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut ctx = fixture(
                r#"
            window.firstRoundDone=new Promise(resolve=>window.finishFirstRound=resolve);
            window.firstFinished=false;window.restored=false;
            document.addEventListener('wheel',e=>{
                if(firstFinished)e.preventDefault();
                events.push([e.cancelable,e.deltaY,restored]);total+=e.deltaY;
                if(!firstFinished && total>=299.999){firstFinished=true;finishFirstRound();}
            },{passive:false});
        "#,
            )
            .await;
            set_gesture_viewport(&mut ctx, 600).await;
            let raw = |id| {
                json!({"id":id,"method":"Input.synthesizeScrollGesture",
            "params":{"x":500,"y":40,"yDistance":-300,"speed":300,"gestureSourceType":"mouse"}})
                .to_string()
            };
            let CdpCommandTaskStep::Pending(first) =
                ctx.with_fixture(|conn| conn.start_command_dispatch(&raw(35000)))
            else {
                panic!("pending first command")
            };
            let blocker = ctx.with_fixture(|conn| {
                let owner = CommandOwnerScope::capture(conn, None);
                conn.synthetic_scroll_admissions_for_owner(&owner)
                    .unwrap()
                    .reserve()
            });
            let CdpCommandTaskStep::Pending(second) =
                ctx.with_fixture(|conn| conn.start_command_dispatch(&raw(35020)))
            else {
                panic!("pending second command")
            };
            let first = tokio::task::spawn_local(first.wait());
            let second = tokio::task::spawn_local(second.wait());
            ctx.process_async(json!({"id":35011,"method":"Runtime.evaluate",
            "params":{"expression":"firstRoundDone","awaitPromise":true}}))
                .await;
            let response = ctx
                .wait_for_scheduler_message("first command wheels", |m| m["id"] == 35011)
                .await;
            assert!(response.get("error").is_none(), "{response}");
            let completed = ctx.drive_owned_wait(first).await.unwrap();
            let step = ctx.with_fixture(|conn| {
                conn.complete_command_dispatch_with_context(
                    completed,
                    &mut CommandDispatchContext::default(),
                )
            });
            let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
            assert!(
                messages.contains(&json!({"id":35000,"result":{}})),
                "{messages:?}"
            );
            set_gesture_viewport(&mut ctx, 300).await;
            assert!(evaluate_bool(&mut ctx, "scrollTo(0,0);scrollY===0").await);
            drop(blocker);
            // Dispatch the next command's Began while the widget is narrow.
            ctx.drive_owned_wait(tokio::time::sleep(std::time::Duration::from_millis(200)))
                .await;
            assert!(!second.is_finished());
            assert!(evaluate_bool(&mut ctx, "window.restored=true;true").await);
            set_gesture_viewport(&mut ctx, 600).await;
            let completed = ctx.drive_owned_wait(second).await.unwrap();
            let step = ctx.with_fixture(|conn| {
                conn.complete_command_dispatch_with_context(
                    completed,
                    &mut CommandDispatchContext::default(),
                )
            });
            let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
            assert!(
                messages.contains(&json!({"id":35020,"result":{}})),
                "{messages:?}"
            );
            let value = snapshot(&mut ctx).await;
            let total = value["total"].as_f64().unwrap();
            assert!(total > 300.0 && total < 599.0, "{value}");
            assert!(
                value["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| e[2] == true)
                    .all(|e| e[0] == true),
                "{value}"
            );
            assert_eq!(
                value["page"], 0,
                "the prior command's End permits a fresh platform Began: {value}"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_validates_anchor_before_waiting_in_page_queue() {
    for initially_valid in [true, false] {
        let mut ctx = fixture("document.addEventListener('wheel',e=>total+=e.deltaY);").await;
        set_gesture_viewport(&mut ctx, if initially_valid { 600 } else { 300 }).await;
        let blocker = ctx.with_fixture(|conn| {
            let owner = CommandOwnerScope::capture(conn, None);
            conn.synthetic_scroll_admissions_for_owner(&owner)
                .unwrap()
                .reserve()
        });
        let raw = json!({"id":35000,"method":"Input.synthesizeScrollGesture",
            "params":{"x":500,"y":40,"yDistance":-10,"speed":300,"gestureSourceType":"mouse"}})
        .to_string();
        let step = ctx.with_fixture(|conn| conn.start_command_dispatch(&raw));
        let messages = if initially_valid {
            // Validation is already enqueued ahead of this viewport change.
            set_gesture_viewport(&mut ctx, 300).await;
            drop(blocker);
            ctx.complete_command_task_step_for_test(step).await.0
        } else {
            // Invalid input must finish while its predecessor still owns FIFO.
            let messages = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                ctx.complete_command_task_step_for_test(step),
            )
            .await
            .expect("initial validation must not wait for the scroll queue")
            .0;
            set_gesture_viewport(&mut ctx, 600).await;
            drop(blocker);
            messages
        };
        if initially_valid {
            assert!(
                messages.contains(&json!({"id":35000,"result":{}})),
                "{messages:?}"
            );
        } else {
            let error = messages
                .iter()
                .find(|m| m["id"] == 35000)
                .expect("gesture reply");
            assert_eq!(
                error["error"]["message"], "Position out of bounds",
                "{messages:?}"
            );
        }
        assert!(evaluate_bool(&mut ctx, "total===0 && scrollY===0").await);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_scroll_gesture_binds_to_the_document_when_admitted() {
    let mut ctx = fixture("").await;
    let blocker = ctx.with_fixture(|conn| {
        let owner = CommandOwnerScope::capture(conn, None);
        conn.synthetic_scroll_admissions_for_owner(&owner)
            .unwrap()
            .reserve()
    });
    let raw = json!({"id":35000,"method":"Input.synthesizeScrollGesture",
        "params":{"x":20,"y":20,"yDistance":-40,"speed":800,"gestureSourceType":"mouse"}})
    .to_string();
    let step = ctx.with_fixture(|conn| conn.start_command_dispatch(&raw));

    // The bounds check was queued on the old Document, before this command.
    // The gesture itself is still waiting for the Page's FIFO admission.
    assert!(
        evaluate_bool(&mut ctx, r#"
            document.open();
            document.write('<body style="margin:0;height:2400px"><div id="newScroller" style="width:160px;height:120px;overflow:auto"><div style="height:1800px">new page</div></div>');
            document.close();
            window.delta = 0;
            newScroller.addEventListener('wheel', event => delta += event.deltaY);
            true
        "#).await
    );
    drop(blocker);

    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    assert!(
        messages.contains(&json!({"id":35000,"result":{}})),
        "{messages:?}"
    );
    assert!(
        evaluate_bool(
            &mut ctx,
            "Math.abs(delta - 40) < 0.001 && Math.abs(newScroller.scrollTop - 40) < 0.001"
        )
        .await
    );
}

async fn raw_wheel_during_synthetic(cancel_raw: bool) {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut ctx = fixture(&format!(
                r#"
            window.block={};window.marked=false;window.atRaw=0;
            window.firstWheel=new Promise(resolve=>window.notifyFirst=resolve);
            document.addEventListener('wheel',e=>{{
                if(e.deltaX===1){{block={cancel_raw};marked=true;atRaw=a.scrollTop;}}
                if(block)e.preventDefault();
                events.push([e.deltaX,e.deltaY,e.cancelable,e.defaultPrevented,marked]);
                total+=e.deltaY;
                if(!marked && e.deltaY>0)notifyFirst();
            }},{{passive:false}});
        "#,
                !cancel_raw
            ))
            .await;
            let raw = json!({"id":35000,"method":"Input.synthesizeScrollGesture",
            "params":{"x":20,"y":20,"yDistance":-300,"speed":300,"gestureSourceType":"mouse"}})
            .to_string();
            let CdpCommandTaskStep::Pending(pending) =
                ctx.with_fixture(|conn| conn.start_command_dispatch(&raw))
            else {
                panic!("pending gesture")
            };
            let running = tokio::task::spawn_local(pending.wait());
            ctx.process_async(json!({"id":35011,"method":"Runtime.evaluate",
            "params":{"expression":"firstWheel","awaitPromise":true}}))
                .await;
            let response = ctx
                .wait_for_scheduler_message("first actual synthetic wheel", |m| m["id"] == 35011)
                .await;
            assert!(response.get("error").is_none(), "{response}");
            assert!(
                response["result"].get("exceptionDetails").is_none(),
                "{response}"
            );
            ctx.process_async(json!({"id":35012,"method":"Input.dispatchMouseEvent",
            "params":{"type":"mouseWheel","x":20,"y":20,"deltaX":1,"deltaY":0}}))
                .await;
            ctx.expect_result(35012, json!({}), None);
            let completed = ctx.drive_owned_wait(running).await.unwrap();
            let step = ctx.with_fixture(|conn| {
                conn.complete_command_dispatch_with_context(
                    completed,
                    &mut CommandDispatchContext::default(),
                )
            });
            let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
            assert!(
                messages.contains(&json!({"id":35000,"result":{}})),
                "{messages:?}"
            );
            let value: Value = serde_json::from_str(
                &evaluate_string(
                    &mut ctx,
                    "JSON.stringify({events,atRaw,a:a.scrollTop,total})",
                )
                .await,
            )
            .unwrap();
            let events = value["events"].as_array().unwrap();
            let raw = events
                .iter()
                .find(|e| e[0] == 1)
                .expect("raw CDP wheel must reach the page");
            assert_eq!(raw[2], true, "{value}");
            assert_eq!(raw[3], cancel_raw, "{value}");
            let after = events
                .iter()
                .filter(|e| e[4] == true && e[1].as_f64().unwrap() > 0.0)
                .collect::<Vec<_>>();
            assert!(after.len() > 1, "{value}");
            for event in after {
                assert_eq!(
                    event[2], cancel_raw,
                    "raw Began ACK must update the common queue: {value}"
                );
                assert_eq!(event[3], cancel_raw, "{value}");
            }
            if cancel_raw {
                assert_eq!(value["a"], value["atRaw"], "{value}");
            } else {
                assert!(value["a"].as_f64().unwrap() > 100.0, "{value}");
            }
            assert!(
                (value["total"].as_f64().unwrap() - 300.0).abs() < 0.001,
                "{value}"
            );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_raw_canceled_began_restores_blocking_dispatch() {
    raw_wheel_during_synthetic(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_raw_uncanceled_began_enables_nonblocking_dispatch() {
    raw_wheel_during_synthetic(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_raw_began_hit_tests_without_replacing_the_scroll_latch() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut ctx = fixture(r#"
                b.style.left = '200px';
                b.style.display = 'block';
                window.firstScroll = new Promise(resolve =>
                    a.addEventListener('scroll', resolve, {once: true}));
                window.rawWheel = null;
                b.addEventListener('wheel', event => rawWheel = [event.deltaY, event.cancelable]);
            "#).await;
            let raw = json!({"id":35000,"method":"Input.synthesizeScrollGesture",
                "params":{"x":20,"y":20,"yDistance":-300,"speed":300,"gestureSourceType":"mouse"}})
            .to_string();
            let CdpCommandTaskStep::Pending(pending) =
                ctx.with_fixture(|conn| conn.start_command_dispatch(&raw))
            else {
                panic!("gesture must wait for its wheel events");
            };
            let running = tokio::task::spawn_local(pending.wait());
            ctx.process_async(json!({
                "id":35011,"method":"Runtime.evaluate",
                "params":{"expression":"firstScroll.then(() => true)","awaitPromise":true,"returnByValue":true}
            })).await;
            let response = ctx
                .wait_for_scheduler_message("first default scroll", |message| message["id"] == 35011)
                .await;
            assert_eq!(response["result"]["result"]["value"], true, "{response}");
            ctx.process_async(json!({
                "id":35012,"method":"Input.dispatchMouseEvent",
                "params":{"type":"mouseWheel","x":220,"y":20,"deltaX":0,"deltaY":100}
            })).await;
            ctx.expect_result(35012, json!({}), None);

            let completed = ctx.drive_owned_wait(running).await.unwrap();
            let step = ctx.with_fixture(|conn| {
                conn.complete_command_dispatch_with_context(
                    completed,
                    &mut CommandDispatchContext::default(),
                )
            });
            let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
            assert!(messages.contains(&json!({"id":35000,"result":{}})), "{messages:?}");
            assert_eq!(
                evaluate_string(&mut ctx, "JSON.stringify(rawWheel)").await,
                "[100,true]"
            );
            let value = snapshot(&mut ctx).await;
            assert!((value["a"].as_f64().unwrap() - 400.0).abs() < 0.001, "{value}");
            assert_eq!(value["b"], 0, "{value}");
        })
        .await;
}
