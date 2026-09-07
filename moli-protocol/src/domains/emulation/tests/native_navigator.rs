use super::*;

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
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_navigator_overrides(),
        moli_page_types::NavigatorOverrides::default()
    );
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
    let bc = ctx.conn.browser_context.as_ref().unwrap();
    let overrides = bc.active_navigator_overrides();
    // This CDP command updates the native network source, so onLine must still
    // report offline without requiring a separate Navigator override.
    assert!(bc.active_page_target().network_policy.network_offline());
    assert_eq!(overrides.online, None);
    assert_eq!(overrides.max_touch_points, Some(1));
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
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_navigator_overrides(),
        moli_page_types::NavigatorOverrides::default()
    );
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
async fn native_geolocation_watch_tracks_unavailable_and_cleared_overrides() {
    use moli_page_types::{GeolocationOverride, GeolocationPositionOverride};

    let mut ctx = setup().await;
    assert_eq!(
        evaluate(
            &mut ctx,
            r#"
        globalThis.nextResult = new Promise(resolve => globalThis.nextResolve = resolve);
        globalThis.watchId = navigator.geolocation.watchPosition(
            p => nextResolve(['position', p.coords.latitude]),
            e => nextResolve(['error', e.code]));
        nextResult
    "#
        )
        .await,
        json!(["error", 2])
    );

    for (method, params, expected_override, expected_result) in [
        (
            "Emulation.setGeolocationOverride",
            json!({}),
            Some(GeolocationOverride::PositionUnavailable),
            json!(["error", 2]),
        ),
        (
            "Emulation.setGeolocationOverride",
            json!({"latitude": 1, "longitude": 2, "accuracy": 3}),
            Some(GeolocationOverride::Position(GeolocationPositionOverride {
                latitude: 1.0,
                longitude: 2.0,
                accuracy: 3.0,
                altitude: None,
                altitude_accuracy: None,
                heading: None,
                speed: None,
            })),
            json!(["position", 1]),
        ),
        (
            "Emulation.setGeolocationOverride",
            json!({}),
            Some(GeolocationOverride::PositionUnavailable),
            json!(["error", 2]),
        ),
        (
            "Emulation.clearGeolocationOverride",
            json!({}),
            None,
            json!(["error", 2]),
        ),
    ] {
        evaluate(&mut ctx, "globalThis.nextResult = new Promise(resolve => globalThis.nextResolve = resolve); undefined").await;
        expect_session_command_result(&mut ctx, 88002, "SID-1", method, params).await;
        assert_eq!(
            ctx.conn
                .browser_context
                .as_ref()
                .unwrap()
                .active_navigator_overrides()
                .geolocation,
            expected_override,
            "{method} must preserve the source state"
        );
        assert_eq!(
            evaluate(&mut ctx, "nextResult").await,
            expected_result,
            "{method}"
        );
    }
    evaluate(&mut ctx, "navigator.geolocation.clearWatch(watchId)").await;
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
