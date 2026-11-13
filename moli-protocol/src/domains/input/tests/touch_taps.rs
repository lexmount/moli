use super::*;
use serde_json::Value;

async fn load_tap_fixture(ctx: &mut TestContext, script: &str) {
    with_loaded_document(ctx, &format!(r#"<html><body style='margin:0'>
        <input id='tap' type='checkbox' style='position:absolute;left:0;top:0;width:80px;height:80px;margin:0'>
        <input id='before' style='position:absolute;left:100px;top:0'>
        <script>
        window.log=[]; window.clicks=0; window.pointerId=null;
        const tap=document.getElementById('tap');
        for(const type of ['pointerdown','touchstart','pointerup','touchend','mousedown','mouseup','click','input','change'])
            tap.addEventListener(type,e=>{{log.push(type);if(type==='pointerdown')pointerId=e.pointerId;if(type==='click'){{clicks++;window.clickState=[e instanceof PointerEvent,e.pointerType,e.pointerId===pointerId,e.detail,e.isTrusted,e.button,e.buttons];}}}});
        {script}
        </script></body></html>"#)).await;
    ctx.capture_fixture_layout(None).await;
}

async fn contact(ctx: &mut TestContext, id: u64, kind: &str, points: Value) {
    ctx.process_async(json!({"id":id,"method":"Input.dispatchTouchEvent","params":{"type":kind,"touchPoints":points}})).await;
    ctx.expect_result(id, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_touch_taps_dispatch_one_trusted_pointer_click_and_activate() {
    let mut ctx = TestContext::new();
    load_tap_fixture(&mut ctx, "").await;
    for tap in 0..40_u64 {
        contact(
            &mut ctx,
            20000 + tap * 2,
            "touchStart",
            json!([{"x":20,"y":20}]),
        )
        .await;
        contact(&mut ctx, 20001 + tap * 2, "touchEnd", json!([])).await;
    }
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify({clicks,checked:tap.checked,clickState})"
        )
        .await,
        r#"{"clicks":40,"checked":false,"clickState":[true,"touch",true,1,true,0,0]}"#
    );
    assert_eq!(
        evaluate_string(&mut ctx, "log.slice(0,9).join('|')").await,
        "pointerdown|touchstart|pointerup|touchend|mousedown|mouseup|click|input|change"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_tap_cancellation_separates_pointer_mouse_and_click_defaults() {
    for (event, clicks, checked, mouse_down, focused) in [
        ("touchstart", 0, false, false, false),
        ("touchend", 0, false, false, false),
        ("pointerdown", 1, true, false, false),
        ("pointerup", 1, true, true, true),
        ("mousedown", 1, true, true, false),
        ("click", 1, false, true, true),
    ] {
        let mut ctx = TestContext::new();
        load_tap_fixture(&mut ctx,&format!("document.getElementById('before').focus();tap.addEventListener('{event}',e=>e.preventDefault(),{{passive:false}});" )).await;
        contact(&mut ctx, 21000, "touchStart", json!([{"x":20,"y":20}])).await;
        contact(&mut ctx, 21001, "touchEnd", json!([])).await;
        let expected = json!([clicks, checked, mouse_down, focused]).to_string();
        assert_eq!(evaluate_string(&mut ctx,"JSON.stringify([clicks,tap.checked,log.includes('mousedown'),document.activeElement===tap])").await,expected,"canceling {event}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_tap_rejects_drags_cancellation_and_multi_contact_gestures() {
    for gesture in ["small", "drag", "return", "cancel", "multi", "removed"] {
        let mut ctx = TestContext::new();
        load_tap_fixture(
            &mut ctx,
            if gesture == "removed" {
                "tap.addEventListener('touchend',()=>tap.remove());"
            } else {
                ""
            },
        )
        .await;
        contact(
            &mut ctx,
            22000,
            "touchStart",
            json!([{"x":20,"y":20,"id":7}]),
        )
        .await;
        if gesture == "multi" {
            contact(
                &mut ctx,
                22001,
                "touchStart",
                json!([{"x":20,"y":20,"id":7},{"x":30,"y":30,"id":3}]),
            )
            .await;
        }
        if matches!(gesture, "small" | "drag" | "return") {
            contact(
                &mut ctx,
                22002,
                "touchMove",
                json!([{"x":if gesture=="small" {21}else{60},"y":20,"id":7}]),
            )
            .await;
            if gesture == "return" {
                contact(
                    &mut ctx,
                    22003,
                    "touchMove",
                    json!([{"x":20,"y":20,"id":7}]),
                )
                .await;
            }
        }
        contact(
            &mut ctx,
            22004,
            if gesture == "cancel" {
                "touchCancel"
            } else {
                "touchEnd"
            },
            json!([]),
        )
        .await;
        assert_eq!(
            evaluate_string(&mut ctx, "String(clicks)").await,
            if gesture == "small" { "1" } else { "0" },
            "gesture {gesture}"
        );
        if gesture != "removed" {
            contact(
                &mut ctx,
                22005,
                "touchStart",
                json!([{"x":20,"y":20,"id":7}]),
            )
            .await;
            contact(&mut ctx, 22006, "touchEnd", json!([])).await;
            assert_eq!(
                evaluate_string(&mut ctx, "String(clicks)").await,
                if gesture == "small" { "2" } else { "1" },
                "gesture {gesture} must release all contacts"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_end_without_a_contact_does_not_click() {
    let mut ctx = TestContext::new();
    load_tap_fixture(&mut ctx, "").await;
    contact(&mut ctx, 23000, "touchEnd", json!([])).await;
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify([clicks,log])").await,
        "[0,[]]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_tap_respects_disabled_and_removed_targets() {
    for script in [
        "tap.disabled=true;",
        "tap.addEventListener('touchstart',()=>tap.remove());",
        "tap.addEventListener('mousedown',()=>tap.remove());",
    ] {
        let mut ctx = TestContext::new();
        load_tap_fixture(&mut ctx, script).await;
        contact(&mut ctx, 24000, "touchStart", json!([{"x":20,"y":20}])).await;
        contact(&mut ctx, 24001, "touchEnd", json!([])).await;
        assert_eq!(
            evaluate_string(&mut ctx, "String(clicks)").await,
            "0",
            "{script}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_tap_runs_submit_default_action() {
    let mut ctx = TestContext::new();
    load_tap_fixture(
        &mut ctx,
        r#"
        const form=document.createElement('form'); document.body.appendChild(form);
        tap.type='submit'; form.appendChild(tap); window.submits=0;
        form.addEventListener('submit',e=>{submits++;e.preventDefault();});
    "#,
    )
    .await;
    contact(&mut ctx, 25000, "touchStart", json!([{"x":20,"y":20}])).await;
    contact(&mut ctx, 25001, "touchEnd", json!([])).await;
    assert_eq!(
        evaluate_string(&mut ctx, "JSON.stringify([clicks,submits])").await,
        "[1,1]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_tap_click_keeps_touch_target_after_pointer_capture() {
    let mut ctx = TestContext::new();
    load_tap_fixture(
        &mut ctx,
        r#"
        window.captureClicks=0;
        const capture=document.getElementById('before');
        tap.addEventListener('pointerdown',e=>capture.setPointerCapture(e.pointerId));
        capture.addEventListener('click',()=>captureClicks++);
    "#,
    )
    .await;
    contact(&mut ctx, 26000, "touchStart", json!([{"x":20,"y":20}])).await;
    contact(&mut ctx, 26001, "touchEnd", json!([])).await;
    assert_eq!(
        evaluate_string(
            &mut ctx,
            "JSON.stringify([clicks,captureClicks,capture.hasPointerCapture(pointerId),tap.checked])"
        )
        .await,
        "[1,0,false,true]"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_compatibility_boundaries_respect_ancestor_and_sibling_relationships() {
    for previous in ["mouse-parent", "tap-parent", "child", "sibling"] {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx,r#"<style>body{margin:0}#menu{position:absolute;width:200px;height:100px}button{position:absolute;top:10px;width:60px;height:40px}#item{left:10px}#other{left:80px}</style><div id=menu><button id=item>Item</button><button id=other>Other</button></div><script>window.boundaries=[];window.menuClosed=false;window.itemClicks=0;const menuNode=document.getElementById('menu');document.getElementById('item').onclick=()=>itemClicks++;for(const type of ['mouseout','mouseleave','mouseover','mouseenter'])for(const node of [menuNode,...menuNode.children])node.addEventListener(type,e=>{if(e.target===node)boundaries.push(type+'@'+node.id)});</script>"#).await;
        ctx.capture_fixture_layout(None).await;
        let initial = if previous.ends_with("parent") {
            json!([{"x":190,"y":90}])
        } else {
            json!([{"x":20,"y":20}])
        };
        if previous == "mouse-parent" {
            ctx.process_async(json!({"id":27000,"method":"Input.dispatchMouseEvent","params":{"type":"mouseMoved","x":190,"y":90}})).await;
            ctx.expect_result(27000, json!({}), None);
        } else {
            contact(&mut ctx, 27000, "touchStart", initial).await;
            contact(&mut ctx, 27001, "touchEnd", json!([])).await;
        }
        let close = previous.ends_with("parent");
        assert_eq!(evaluate_string(&mut ctx,&format!(r#"(() => {{boundaries=[];if({close})document.getElementById('menu').onmouseleave=()=>{{menuClosed=true;document.getElementById('menu').remove();}};return 'ready';}})()"#)).await,"ready");
        let next = match previous {
            "child" => json!([{"x":190,"y":90}]),
            "sibling" => json!([{"x":100,"y":20}]),
            _ => json!([{"x":20,"y":20}]),
        };
        contact(&mut ctx, 27002, "touchStart", next).await;
        contact(&mut ctx, 27003, "touchEnd", json!([])).await;
        let observed: serde_json::Value = serde_json::from_str(
            &evaluate_string(
                &mut ctx,
                "JSON.stringify([menuClosed,itemClicks,boundaries])",
            )
            .await,
        )
        .unwrap();
        let expected = match previous {
            "child" => json!([
                false,
                1,
                ["mouseout@item", "mouseleave@item", "mouseover@menu"]
            ]),
            "sibling" => json!([
                false,
                1,
                [
                    "mouseout@item",
                    "mouseleave@item",
                    "mouseover@other",
                    "mouseenter@other"
                ]
            ]),
            _ => json!([
                false,
                1,
                ["mouseout@menu", "mouseover@item", "mouseenter@item"]
            ]),
        };
        assert_eq!(observed, expected, "{previous}");
    }
}
