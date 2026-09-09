use super::*;

const SNAPSHOT: &str = r#"function(w) {
    const names = ['ontouchstart', 'ontouchend', 'ontouchmove', 'ontouchcancel'];
    const owners = [w, w.Document.prototype, w.HTMLElement.prototype, w.SVGElement.prototype, w.MathMLElement.prototype];
    const present = names.every(name => owners.every(owner => {
        const d = Object.getOwnPropertyDescriptor(owner, name);
        return d && typeof d.get === 'function' && typeof d.set === 'function' && d.enumerable && d.configurable;
    }));
    let legacy;
    try {
        const e = w.document.createEvent('TouchEvent');
        let beforeInit;
        try { w.document.dispatchEvent(e); } catch (error) { beforeInit = error.name; }
        legacy = [e instanceof w.TouchEvent, e.type, e.touches, e.targetTouches, e.changedTouches, beforeInit];
        e.initEvent('touchstart', false, false);
        if (!w.document.dispatchEvent(e)) throw new Error('initialized event should dispatch');
    } catch (error) { legacy = error.name; }
    return {points: w.navigator.maxTouchPoints, present,
        absent: names.every(name => owners.every(owner => !(name in owner))),
        constructor: new w.TouchEvent('touchstart').touches.length === 0 && typeof w.Touch === 'function', legacy};
}"#;

fn expected(points: u32, enabled: bool) -> serde_json::Value {
    json!({"points": points, "present": enabled, "absent": !enabled, "constructor": true,
        "legacy": if enabled { json!([true, "", null, null, null, "InvalidStateError"]) } else { json!("NotSupportedError") }})
}

async fn command(
    ctx: &mut TestContext,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    ctx.process_async(
        json!({"id": 88010, "sessionId": "SID-1", "method": method, "params": params}),
    )
    .await;
    crate::testing::wait_until_scheduler_message(ctx, "touch capability command", |message| {
        message["id"] == json!(88010)
    })
    .await;
    let response = ctx.take_response_by_id(88010);
    assert!(response["error"].is_null(), "{response}");
    response["result"].clone()
}

async fn isolated_snapshot(
    ctx: &mut TestContext,
    frame: &serde_json::Value,
    name: &str,
) -> serde_json::Value {
    let world = command(
        ctx,
        "Page.createIsolatedWorld",
        json!({"frameId": frame, "worldName": name}),
    )
    .await;
    let result = command(ctx, "Runtime.evaluate", json!({
        "contextId": world["executionContextId"], "expression": format!("({SNAPSHOT})(globalThis)"), "returnByValue": true
    })).await;
    assert!(result["exceptionDetails"].is_null(), "{result}");
    result["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_feature_detection_follows_document_lifetime_not_live_emulation() {
    let mut ctx = setup().await;
    for (stage, points, reload, existing_enabled, new_enabled) in [
        ("desktop", 0, false, false, false),
        ("enable", 5, false, false, true),
        ("reload-enabled", 5, true, true, true),
        ("disable", 0, false, true, false),
        ("reload-disabled", 0, true, false, false),
    ] {
        if stage == "enable" || stage == "disable" {
            command(
                &mut ctx,
                "Emulation.setTouchEmulationEnabled",
                json!({"enabled": points > 0, "maxTouchPoints": 5}),
            )
            .await;
        }
        if reload {
            ctx.install_buffered_navigation_fixture_for_session_owner(
                url::Url::parse(&format!("https://geolocation.example/{stage}")).unwrap(),
                "<!doctype html><body><script>globalThis.initialTouchHandler = 'ontouchstart' in window;</script>".into(),
                Some("SID-1"),
            ).await;
            assert_eq!(
                evaluate(&mut ctx, "initialTouchHandler").await,
                json!(existing_enabled)
            );
        }
        assert_eq!(
            evaluate(&mut ctx, &format!("({SNAPSHOT})(globalThis)")).await,
            expected(points, existing_enabled),
            "main: {stage}"
        );
        let tree = command(&mut ctx, "Page.getFrameTree", json!({})).await;
        assert_eq!(
            isolated_snapshot(&mut ctx, &tree["frameTree"]["frame"]["id"], stage).await,
            expected(points, existing_enabled),
            "isolated: {stage}"
        );
        let child = evaluate(&mut ctx, &format!(r#"(async () => {{
            const frame = document.createElement('iframe');
            frame.id = 'touch-child'; frame.srcdoc = '<!doctype html><body>child</body>';
            await new Promise(resolve => {{ frame.onload = resolve; document.body.appendChild(frame); }});
            return ({SNAPSHOT})(frame.contentWindow);
        }})()"#)).await;
        assert_eq!(child, expected(points, new_enabled), "child: {stage}");
        let tree = command(&mut ctx, "Page.getFrameTree", json!({})).await;
        let child_frame = &tree["frameTree"]["childFrames"][0]["frame"]["id"];
        assert!(child_frame.is_string(), "{tree}");
        assert_eq!(
            isolated_snapshot(&mut ctx, child_frame, stage).await,
            expected(points, new_enabled),
            "child isolated: {stage}"
        );
        let borrowed = evaluate(
            &mut ctx,
            r#"(() => {
            const child = document.getElementById('touch-child').contentWindow;
            const attempt = (method, receiver, realm) => {
                try { return method.call(receiver, 'TouchEvent') instanceof realm.TouchEvent; }
                catch (error) { return error.name; }
            };
            return [attempt(Document.prototype.createEvent, child.document, child),
                attempt(child.Document.prototype.createEvent, document, window)];
        })()"#,
        )
        .await;
        assert_eq!(
            borrowed,
            json!([
                if new_enabled {
                    json!(true)
                } else {
                    json!("NotSupportedError")
                },
                if existing_enabled {
                    json!(true)
                } else {
                    json!("NotSupportedError")
                }
            ]),
            "receiver realm: {stage}"
        );
        evaluate(&mut ctx, "document.getElementById('touch-child').remove()").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_feature_detection_is_shared_when_child_isolated_world_is_created_first() {
    let mut ctx = setup().await;
    command(
        &mut ctx,
        "Emulation.setTouchEmulationEnabled",
        json!({"enabled": true, "maxTouchPoints": 5}),
    )
    .await;
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://geolocation.example/touch-isolated-first").unwrap(),
        "<!doctype html><body><iframe id='touch-child'></iframe></body>".into(),
        Some("SID-1"),
    )
    .await;
    let tree = command(&mut ctx, "Page.getFrameTree", json!({})).await;
    let child = &tree["frameTree"]["childFrames"][0]["frame"]["id"];
    assert!(child.is_string(), "{tree}");
    assert_eq!(
        isolated_snapshot(&mut ctx, child, "first").await,
        expected(5, true)
    );
    command(
        &mut ctx,
        "Emulation.setTouchEmulationEnabled",
        json!({"enabled": false}),
    )
    .await;
    assert_eq!(
        evaluate(
            &mut ctx,
            &format!("({SNAPSHOT})(document.getElementById('touch-child').contentWindow)")
        )
        .await,
        expected(0, true)
    );
    assert_eq!(
        isolated_snapshot(&mut ctx, child, "second").await,
        expected(0, true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_feature_detection_keeps_author_properties_and_native_event_factory() {
    let mut ctx = setup().await;
    evaluate(&mut ctx, r#"
        window.ontouchstart = 'author window';
        Object.defineProperty(SVGElement.prototype, 'ontouchstart', {value: 'author SVG', configurable: false});
        void 0;
    "#).await;
    for enabled in [true, false, true] {
        command(
            &mut ctx,
            "Emulation.setTouchEmulationEnabled",
            json!({"enabled": enabled}),
        )
        .await;
        assert_eq!(
            evaluate(
                &mut ctx,
                "[window.ontouchstart, SVGElement.prototype.ontouchstart]"
            )
            .await,
            json!(["author window", "author SVG"])
        );
    }
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://geolocation.example/touch-factory").unwrap(),
        "<!doctype html><body>touch</body>".into(),
        Some("SID-1"),
    )
    .await;
    assert_eq!(evaluate(&mut ctx, r#"(() => {
        const OriginalTouchEvent = TouchEvent;
        window.TouchEvent = function() { throw new Error('author constructor must not run'); };
        const e = document.createEvent('tOuChEvEnT');
        let invalidReceiver;
        try { Document.prototype.createEvent.call(Object.create(Document.prototype), 'TouchEvent'); }
        catch (error) { invalidReceiver = error.name; }
        return [e instanceof OriginalTouchEvent, e.touches, e.type, invalidReceiver];
    })()"#).await, json!([true, null, "", "TypeError"]));
}
