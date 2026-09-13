use super::*;
use crate::devtools_runtime::{
    DevToolsDevicePixelRatioSetting, DevToolsSetViewportCommand, DevToolsViewportSetting,
};

async fn evaluate(ctx: &mut TestContext, expression: &str) -> serde_json::Value {
    ctx.process_async(json!({
        "id": 88000, "sessionId": "SID-1", "method": "Runtime.evaluate",
        "params": {"expression": expression, "returnByValue": true, "awaitPromise": true}
    }))
    .await;
    crate::testing::wait_until_scheduler_message(ctx, "native Navigator evaluation", |message| {
        message["id"] == json!(88000)
    })
    .await;
    let response = ctx.take_response_by_id(88000);
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    assert!(response["error"].is_null(), "{response}");
    response["result"]["result"]["value"].clone()
}

async fn setup() -> TestContext {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    install_geolocation_page_for_test(&mut ctx, bc).await;
    ctx
}

async fn set_position(ctx: &mut TestContext, latitude: f64) {
    expect_session_command_result(ctx, 88001, "SID-1", "Emulation.setGeolocationOverride",
        json!({"latitude": latitude, "longitude": 2, "accuracy": 3, "altitude": 4, "altitudeAccuracy": 5, "heading": 6, "speed": 7})).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_navigator_descriptors_survive_cdp_override_and_clear() {
    let mut ctx = setup().await;
    assert_eq!(evaluate(&mut ctx, r#"
        globalThis.navKeys = ['onLine', 'maxTouchPoints', 'geolocation'];
        globalThis.navGetters = navKeys.map(k => Object.getOwnPropertyDescriptor(Navigator.prototype, k).get);
        globalThis.geo = navigator.geolocation;
        globalThis.checkDescriptors = () => navKeys.every((k, i) =>
            !Object.hasOwn(navigator, k) &&
            navGetters[i] === Object.getOwnPropertyDescriptor(Navigator.prototype, k).get &&
            Function.prototype.toString.call(navGetters[i]).includes('[native code]')) &&
            navigator.geolocation === geo && geo instanceof Geolocation &&
            !('__moliGeolocationState' in globalThis) && !('__moliNavigatorOnline' in globalThis);
        checkDescriptors()
    "#).await, json!(true));
    set_position(&mut ctx, 1.0).await;
    expect_session_command_result(
        &mut ctx,
        88002,
        "SID-1",
        "Emulation.setTouchEmulationEnabled",
        json!({"enabled": true}),
    )
    .await;
    expect_session_command_result(
        &mut ctx,
        88003,
        "SID-1",
        "Network.emulateNetworkConditions",
        json!({"offline": true, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1}),
    )
    .await;
    assert_eq!(
        evaluate(
            &mut ctx,
            "[checkDescriptors(), navigator.onLine, navigator.maxTouchPoints]"
        )
        .await,
        json!([true, false, 1])
    );
    expect_session_command_result(
        &mut ctx,
        88004,
        "SID-1",
        "Network.emulateNetworkConditions",
        json!({"offline": false, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1}),
    )
    .await;
    expect_session_command_result(
        &mut ctx,
        88005,
        "SID-1",
        "Emulation.setTouchEmulationEnabled",
        json!({"enabled": false}),
    )
    .await;
    expect_session_command_result(
        &mut ctx,
        88006,
        "SID-1",
        "Emulation.clearGeolocationOverride",
        json!({}),
    )
    .await;
    assert_eq!(
        evaluate(
            &mut ctx,
            "[checkDescriptors(), navigator.onLine, navigator.maxTouchPoints]"
        )
        .await,
        json!([true, true, 0])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_geolocation_position_has_branded_prototype_attributes() {
    let mut ctx = setup().await;
    set_position(&mut ctx, 1.0).await;
    assert_eq!(evaluate(&mut ctx, r#"
        new Promise((resolve, reject) => navigator.geolocation.getCurrentPosition(function(p) {
            'use strict';
            const getters = [
                [GeolocationPosition.prototype, 'coords'], [GeolocationPosition.prototype, 'timestamp'],
                ...['latitude','longitude','altitude','accuracy','altitudeAccuracy','heading','speed'].map(k => [GeolocationCoordinates.prototype, k])
            ];
            const branded = getters.every(([prototype, key]) => {
                try { Object.getOwnPropertyDescriptor(prototype, key).get.call(Object.create(prototype)); return false; }
                catch (e) { return e instanceof TypeError; }
            });
            resolve([
                this === undefined, p instanceof GeolocationPosition, p.coords instanceof GeolocationCoordinates,
                Object.keys(p).length, Object.keys(p.coords).length, branded,
                p.coords.toJSON(), Number.isInteger(p.timestamp), p.toJSON().coords.latitude
            ]);
        }, reject))
    "#).await, json!([true, true, true, 0, 0, true,
        {"latitude":1,"longitude":2,"altitude":4,"accuracy":3,"altitudeAccuracy":5,"heading":6,"speed":7},true,1]));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_geolocation_watch_updates_and_clear_cancels_delivery() {
    let mut ctx = setup().await;
    set_position(&mut ctx, 1.0).await;
    assert_eq!(
        evaluate(
            &mut ctx,
            r#"
        globalThis.seen = [];
        globalThis.nextPosition = new Promise(resolve => globalThis.nextResolve = resolve);
        globalThis.watchId = navigator.geolocation.watchPosition(p => {
            seen.push(p.coords.latitude); nextResolve(p.coords.latitude);
        });
        nextPosition
    "#
        )
        .await,
        json!(1)
    );
    evaluate(&mut ctx, "globalThis.nextPosition = new Promise(resolve => globalThis.nextResolve = resolve); undefined").await;
    set_position(&mut ctx, 8.0).await;
    assert_eq!(evaluate(&mut ctx, "nextPosition").await, json!(8));
    evaluate(&mut ctx, "navigator.geolocation.clearWatch(watchId)").await;
    set_position(&mut ctx, 9.0).await;
    // A task checkpoint after the update is a deterministic cancellation fence.
    assert_eq!(
        evaluate(
            &mut ctx,
            "new Promise(resolve => setTimeout(() => resolve(seen), 0))"
        )
        .await,
        json!([1, 8])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_navigator_overrides_are_installed_before_author_script() {
    let mut ctx = setup().await;
    set_position(&mut ctx, 1.0).await;
    expect_session_command_result(
        &mut ctx,
        88002,
        "SID-1",
        "Emulation.setTouchEmulationEnabled",
        json!({"enabled": true}),
    )
    .await;
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://geolocation.example/next").unwrap(),
        r#"<!doctype html><script>
            globalThis.initialTouch = navigator.maxTouchPoints;
            globalThis.initialPosition = new Promise((resolve, reject) =>
                navigator.geolocation.getCurrentPosition(p => resolve(p.coords.latitude), reject));
        </script>"#
            .into(),
        Some("SID-1"),
    )
    .await;
    assert_eq!(evaluate(&mut ctx, "initialTouch").await, json!(1));
    assert_eq!(evaluate(&mut ctx, "initialPosition").await, json!(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_geolocation_override_does_not_bypass_insecure_context() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    set_position(&mut ctx, 1.0).await;
    assert_eq!(evaluate(&mut ctx, "new Promise(resolve => navigator.geolocation.getCurrentPosition(() => resolve('unexpected position'), e => resolve(e.code)))").await, json!(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_geolocation_result_uses_receiver_realm() {
    let mut ctx = setup().await;
    set_position(&mut ctx, 1.0).await;
    assert_eq!(evaluate(&mut ctx, r#"
        (async () => {
            const frame = document.createElement('iframe');
            frame.srcdoc = '<body>child</body>';
            await new Promise(resolve => { frame.onload = resolve; document.body.append(frame); });
            const child = frame.contentWindow;
            return new Promise((resolve, reject) => Geolocation.prototype.getCurrentPosition.call(
                child.navigator.geolocation,
                p => resolve([p instanceof child.GeolocationPosition, p instanceof GeolocationPosition,
                    p.coords instanceof child.GeolocationCoordinates, p.coords.latitude]), reject));
        })()
    "#).await, json!([true, false, true, 1]));
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_override_updates_loaded_background_page_and_preserves_real_focus() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active");
    bc.attach_active_session("SID-active");
    bc.insert_page_target_host(PageTargetHost::new(
        "TID-1".into(),
        Some("SID-1".into()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    install_geolocation_page_for_test(&mut ctx, bc).await;
    let snapshot = "[document.hasFocus(), document.hidden, document.visibilityState]";
    assert_eq!(
        evaluate(&mut ctx, snapshot).await,
        json!([false, true, "hidden"])
    );
    for enabled in [true, true, false] {
        expect_session_command_result(
            &mut ctx,
            88001,
            "SID-1",
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": enabled}),
        )
        .await;
        assert_eq!(
            evaluate(&mut ctx, snapshot).await,
            json!([
                enabled,
                !enabled,
                if enabled { "visible" } else { "hidden" }
            ])
        );
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .active_target_id(),
            Some("TID-active")
        );
    }
    // Removing an override does not remove the real foreground target's focus.
    let mut foreground = setup().await;
    expect_session_command_result(
        &mut foreground,
        88001,
        "SID-1",
        "Emulation.setFocusEmulationEnabled",
        json!({"enabled": false}),
    )
    .await;
    assert_eq!(
        evaluate(&mut foreground, snapshot).await,
        json!([true, false, "visible"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn max_touch_points_updates_native_getters_and_rejects_invalid_counts_atomically() {
    let mut ctx = setup().await;
    evaluate(&mut ctx, "globalThis.touchGetter = Object.getOwnPropertyDescriptor(Navigator.prototype, 'maxTouchPoints').get").await;
    for (params, expected) in [
        (json!({"enabled": true}), 1),
        (json!({"enabled": true, "maxTouchPoints": 5}), 5),
        (json!({"enabled": true, "maxTouchPoints": 16}), 16),
        (json!({"enabled": false, "maxTouchPoints": 5}), 0),
        (json!({"enabled": true, "maxTouchPoints": 5}), 5),
    ] {
        expect_session_command_result(
            &mut ctx,
            88001,
            "SID-1",
            "Emulation.setTouchEmulationEnabled",
            params,
        )
        .await;
        assert_eq!(evaluate(&mut ctx, "[navigator.maxTouchPoints, touchGetter.call(navigator), touchGetter === Object.getOwnPropertyDescriptor(Navigator.prototype, 'maxTouchPoints').get]").await, json!([expected, expected, true]));
    }
    for enabled in [true, false] {
        for max_touch_points in [0, 17, -1] {
            ctx.process_async(json!({"id": 88001,"sessionId": "SID-1", "method": "Emulation.setTouchEmulationEnabled", "params": {"enabled": enabled,"maxTouchPoints": max_touch_points}})).await;
            ctx.expect_error(88001, -32602, "Touch points must be between 1 and 16");
            assert_eq!(
                evaluate(&mut ctx, "navigator.maxTouchPoints").await,
                json!(5)
            );
        }
    }
    assert_eq!(
        evaluate(
            &mut ctx,
            r#"(async () => {
        const frame = document.createElement('iframe');
        frame.srcdoc = '<body>child</body>';
        await new Promise(resolve => { frame.onload = resolve; document.body.append(frame); });
        return frame.contentWindow.navigator.maxTouchPoints;
    })()"#
        )
        .await,
        json!(5)
    );
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://geolocation.example/next-touch").unwrap(),
        "<!doctype html><script>globalThis.initialTouch = navigator.maxTouchPoints;</script>"
            .into(),
        Some("SID-1"),
    )
    .await;
    assert_eq!(evaluate(&mut ctx, "initialTouch").await, json!(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn device_metrics_zero_axes_use_visible_size_and_clear_restores_native_defaults() {
    let mut ctx = setup().await;
    let snapshot = "[innerWidth, innerHeight, devicePixelRatio, outerWidth, outerHeight, screen.width, screen.height]";
    let baseline = evaluate(&mut ctx, snapshot).await;
    let base = baseline.as_array().unwrap();
    for (width, height, dpr, expected_width, expected_height) in [
        (640, 480, 2, 640, 480),
        (1000, 0, 2, 1000, 480),
        (0, 0, 0, 640, 480),
        (0, 200, 0, 640, 200),
        (0, 0, 0, 640, 480),
    ] {
        expect_session_command_result(
            &mut ctx,
            88001,
            "SID-1",
            "Emulation.setDeviceMetricsOverride",
            json!({"width": width,"height": height,"deviceScaleFactor": dpr,"mobile": false}),
        )
        .await;
        assert_eq!(
            evaluate(&mut ctx, snapshot).await,
            json!([
                expected_width,
                expected_height,
                if dpr == 0 {
                    base[2].clone()
                } else {
                    json!(dpr)
                },
                base[3],
                base[4],
                base[5],
                base[6]
            ])
        );
    }
    let previous = evaluate(&mut ctx, snapshot).await;
    for invalid in [
        json!({"width": -1}),
        json!({"height": 10_000_001}),
        json!({"deviceScaleFactor": -1}),
        json!({"screenWidth": -1}),
    ] {
        let mut params = json!({"width":640,"height":480,"deviceScaleFactor":1,"mobile":false});
        params
            .as_object_mut()
            .unwrap()
            .extend(invalid.as_object().unwrap().clone());
        ctx.process_async(json!({"id":88001,"sessionId":"SID-1", "method":"Emulation.setDeviceMetricsOverride","params":params})).await;
        ctx.expect_error(88001, -32602, "InvalidParams");
        assert_eq!(evaluate(&mut ctx, snapshot).await, previous);
    }
    expect_session_command_result(&mut ctx, 88001, "SID-1", "Emulation.setDeviceMetricsOverride",
        json!({"width":640,"height":480,"deviceScaleFactor":1,"mobile":false,"screenWidth":1500,"screenHeight":1200})).await;
    assert_eq!(
        evaluate(&mut ctx, "[screen.width,screen.height,screen.availHeight]").await,
        json!([1500, 1200, 1200])
    );
    expect_session_command_result(&mut ctx, 88001, "SID-1", "Emulation.setDeviceMetricsOverride",
        json!({"width":640,"height":480,"deviceScaleFactor":1,"mobile":false,"screenWidth":1500,"screenHeight":0})).await;
    assert_eq!(
        evaluate(&mut ctx, "[screen.width,screen.height]").await,
        json!([base[5], base[6]])
    );
    expect_session_command_result(
        &mut ctx,
        88001,
        "SID-1",
        "Emulation.clearDeviceMetricsOverride",
        json!({}),
    )
    .await;
    assert_eq!(evaluate(&mut ctx, snapshot).await, baseline);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsupported_throttling_rejects_before_changing_live_offline_state() {
    let mut ctx = setup().await;
    for offline in [false, true] {
        expect_session_command_result(
            &mut ctx,
            88001,
            "SID-1",
            "Network.emulateNetworkConditions",
            json!({"offline":offline,"latency":0,"downloadThroughput":-1,"uploadThroughput":0}),
        )
        .await;
        for unsupported in [
            json!({"latency":100}),
            json!({"downloadThroughput":1024}),
            json!({"uploadThroughput":1024}),
            json!({"connectionType":"cellular3g"}),
            json!({"packetLoss":1}),
            json!({"packetQueueLength":1}),
            json!({"packetReordering":true}),
        ] {
            let mut params = json!({"offline": !offline,"latency":0,"downloadThroughput":-1,"uploadThroughput":-1});
            params
                .as_object_mut()
                .unwrap()
                .extend(unsupported.as_object().unwrap().clone());
            ctx.process_async(json!({"id":88001,"sessionId":"SID-1","method":"Network.emulateNetworkConditions","params":params})).await;
            ctx.expect_error(
                88001,
                -32000,
                "Network throttling and connection type overrides are not supported",
            );
            assert_eq!(
                evaluate(&mut ctx, "navigator.onLine").await,
                json!(!offline)
            );
        }
    }
    for rate in [-1.0, 0.0, 0.5, 1.0] {
        expect_session_command_result(
            &mut ctx,
            88001,
            "SID-1",
            "Emulation.setCPUThrottlingRate",
            json!({"rate":rate}),
        )
        .await;
    }
    ctx.process_async(json!({"id":88001,"sessionId":"SID-1","method":"Emulation.setCPUThrottlingRate","params":{"rate":4}})).await;
    ctx.expect_error(88001, -32000, "CPU throttling is not supported");
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .effective_emulation_state
            .cpu_throttling_rate,
        1.0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_viewport_overrides_preserve_getters_and_ignore_page_property_hooks() {
    let mut ctx = setup().await;
    assert_eq!(evaluate(&mut ctx,r#"
        globalThis.originalInner = Object.getOwnPropertyDescriptor(globalThis, 'innerWidth').get;
        globalThis.originalDpr = Object.getOwnPropertyDescriptor(globalThis, 'devicePixelRatio').get;
        globalThis.originalScreenWidth = Object.getOwnPropertyDescriptor(Screen.prototype, 'width').get;
        globalThis.heldScreen = screen;
        globalThis.heldVisual = visualViewport;
        globalThis.originalDefine = Object.defineProperty;
        Object.defineProperty(globalThis, '__moliDeviceMetricsOriginalDescriptors', {
            configurable: true, get() { throw new Error('page-owned property'); }
        });
        Object.defineProperty = () => { throw new Error('page-owned defineProperty'); };
        true
    "#).await,json!(true));
    for width in [640, 800] {
        expect_session_command_result(&mut ctx,88001,"SID-1","Emulation.setDeviceMetricsOverride",
            json!({"width":width,"height":480,"deviceScaleFactor":2,"mobile":false,"screenWidth":1000,"screenHeight":700})).await;
        assert_eq!(
            evaluate(
                &mut ctx,
                r#"[
            innerWidth, originalInner.call(globalThis), heldVisual.width,
            devicePixelRatio, originalDpr.call(globalThis),
            heldScreen.width, originalScreenWidth.call(heldScreen),
            originalInner === Object.getOwnPropertyDescriptor(globalThis,'innerWidth').get,
            originalScreenWidth === Object.getOwnPropertyDescriptor(Screen.prototype,'width').get,
            Function.prototype.toString.call(originalInner).includes('[native code]')
        ]"#
            )
            .await,
            json!([width, width, width, 2, 2, 1000, 1000, true, true, true])
        );
    }
    expect_session_command_result(
        &mut ctx,
        88001,
        "SID-1",
        "Emulation.clearDeviceMetricsOverride",
        json!({}),
    )
    .await;
    assert_eq!(evaluate(&mut ctx,"[innerWidth === originalInner.call(globalThis), originalInner === Object.getOwnPropertyDescriptor(globalThis,'innerWidth').get, !Object.hasOwn(screen,'width')]").await,json!([true,true,true]));
    evaluate(&mut ctx,"Object.defineProperty = originalDefine; delete globalThis.__moliDeviceMetricsOriginalDescriptors").await;
    expect_session_command_result(&mut ctx,88001,"SID-1","Emulation.setDeviceMetricsOverride",
        json!({"width":800,"height":600,"deviceScaleFactor":2,"mobile":false,"screenWidth":1000,"screenHeight":700})).await;
    assert_eq!(evaluate(&mut ctx,r#"(async () => {
        const frame = document.createElement('iframe');
        frame.style.width = '321px'; frame.style.height = '123px'; frame.srcdoc = '<body>child</body>';
        await new Promise(resolve => { frame.onload = resolve; document.body.append(frame); });
        const child = frame.contentWindow;
        return [child.innerWidth, child.visualViewport.width, child.devicePixelRatio, child.screen.width];
    })()"#).await,json!([321,321,2,1000]));
}

#[tokio::test(flavor = "multi_thread")]
async fn clearing_target_metrics_restores_browser_context_viewport_defaults() {
    let mut ctx = setup().await;
    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::SetViewport(DevToolsSetViewportCommand {
            context: bidi_command_context(),
            browser_context_ids: vec!["BID-1".into()],
            viewport: DevToolsViewportSetting::Dimensions {
                width: 900,
                height: 700,
            },
            device_pixel_ratio: DevToolsDevicePixelRatioSetting::Scale(3.0),
            screen_width: None,
            screen_height: None,
        }))
        .await
        .into_parts();
    assert_eq!(
        result.expect("context viewport default"),
        DevToolsCommandResult::Empty
    );
    assert_eq!(
        evaluate(&mut ctx, "[innerWidth, innerHeight, devicePixelRatio]").await,
        json!([900, 700, 3])
    );
    expect_session_command_result(
        &mut ctx,
        88001,
        "SID-1",
        "Emulation.setDeviceMetricsOverride",
        json!({"width":640,"height":480,"deviceScaleFactor":0,"mobile":false}),
    )
    .await;
    assert_eq!(
        evaluate(&mut ctx, "[innerWidth, innerHeight, devicePixelRatio]").await,
        json!([640, 480, 3])
    );
    expect_session_command_result(
        &mut ctx,
        88001,
        "SID-1",
        "Emulation.clearDeviceMetricsOverride",
        json!({}),
    )
    .await;
    assert_eq!(
        evaluate(&mut ctx, "[innerWidth, innerHeight, devicePixelRatio]").await,
        json!([900, 700, 3])
    );
}
