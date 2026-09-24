use super::*;

fn metrics(extra: serde_json::Value) -> serde_json::Value {
    let mut params = json!({
        "width": 200,
        "height": 100,
        "deviceScaleFactor": 1,
        "mobile": false,
        "screenWidth": 900,
        "screenHeight": 800
    });
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    params
}

async fn set_metrics(ctx: &mut TestContext, extra: serde_json::Value) {
    expect_session_command_result(
        ctx,
        88400,
        "SID-1",
        "Emulation.setDeviceMetricsOverride",
        metrics(extra),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn device_metrics_screen_changes_take_effect_after_explicit_capture() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    evaluate(
        &mut ctx,
        r#"
        document.head.innerHTML = `<style>
            body { margin: 0 }
            #target { width: 100px; height: 100px }
            @media (device-width: 1280px), (device-height: 720px) {
                #target { width: 200px }
            }
        </style>`;
        document.body.innerHTML = '<div id=target></div>';
        undefined
        "#,
    )
    .await;

    for (screen_width, screen_height, expected_width) in [
        (1920, 1080, 100),
        (1280, 1080, 200),
        (1280, 1080, 200),
        (1920, 1080, 100),
        (1920, 720, 200),
        (1920, 1080, 100),
    ] {
        set_metrics(
            &mut ctx,
            json!({
                "width": 800, "height": 600,
                "screenWidth": screen_width, "screenHeight": screen_height
            }),
        )
        .await;
        ctx.capture_fixture_layout(Some("SID-1")).await;
        assert_eq!(
            evaluate(
                &mut ctx,
                r#"[
                    innerWidth, innerHeight, devicePixelRatio, screen.width, screen.height,
                    document.getElementById('target').getBoundingClientRect().width,
                    document.elementFromPoint(150, 20)?.id === 'target',
                    document.elementsFromPoint(150, 20).some(element => element.id === 'target')
                ]"#
            )
            .await,
            json!([
                800,
                600,
                1,
                screen_width,
                screen_height,
                expected_width,
                expected_width == 200,
                expected_width == 200
            ]),
        );
    }
}

fn unsupported_modes() -> [(&'static str, serde_json::Value); 7] {
    [
        ("mobile=true", json!({"mobile": true})),
        (
            "displayFeature",
            json!({"displayFeature": {
                "orientation": "vertical", "offset": 200, "maskLength": 0
            }}),
        ),
        (
            "devicePosture",
            json!({"devicePosture": {"type": "folded"}}),
        ),
        (
            "devicePosture",
            json!({"devicePosture": {"type": "continuous"}}),
        ),
        ("scrollbarType=overlay", json!({"scrollbarType": "overlay"})),
        (
            "screenOrientationLockEmulation=true",
            json!({"screenOrientationLockEmulation": true}),
        ),
        ("viewportMeta=enable", json!({"viewportMeta": "enable"})),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn device_metrics_unsupported_modes_preserve_live_and_saved_state() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    set_metrics(&mut ctx, json!({})).await;
    let snapshot = "[innerWidth, innerHeight, devicePixelRatio, screenX, screenY, screen.orientation.type, screen.orientation.angle]";
    let before = evaluate(&mut ctx, snapshot).await;
    let owner = crate::conn::CommandOwnerScope::for_session("SID-1");
    let saved = ctx
        .conn
        .target_session_owner_emulated_device_metrics_for_owner(&owner);

    for (setting, extra) in unsupported_modes() {
        let mut params = metrics(extra);
        params["width"] = json!(390);
        params["height"] = json!(844);
        params["deviceScaleFactor"] = json!(3);
        params["screenOrientation"] = json!({"type": "portraitPrimary", "angle": 90});
        ctx.process_async(json!({
            "id": 88410, "sessionId": "SID-1",
            "method": "Emulation.setDeviceMetricsOverride", "params": params
        }))
        .await;
        ctx.expect_error(
            88410,
            -32000,
            &format!("Emulation.setDeviceMetricsOverride does not support {setting}."),
        );
        assert_eq!(
            ctx.conn
                .target_session_owner_emulated_device_metrics_for_owner(&owner),
            saved
        );
        assert_eq!(evaluate(&mut ctx, snapshot).await, before, "{setting}");
    }

    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<meta name=viewport content='width=device-width'><body>next",
        Some("SID-1"),
    )
    .await;
    assert_eq!(
        evaluate(&mut ctx, snapshot).await,
        before,
        "rejected settings must not become the next document's bootstrap state"
    );

    set_metrics(
        &mut ctx,
        json!({
            "width": 300, "height": 250, "scrollbarType": "default",
            "screenOrientationLockEmulation": false, "viewportMeta": "default",
            "unknownClientExtension": {"value": 1}
        }),
    )
    .await;
    assert_eq!(
        evaluate(&mut ctx, "[innerWidth, innerHeight]").await,
        json!([300, 250])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn device_metrics_validate_known_options_even_without_a_target() {
    let mut ctx = TestContext::new();
    assert!(ctx.conn.browser_context.is_none());
    for (setting, extra) in unsupported_modes() {
        ctx.process_async(json!({
            "id": 88411, "method": "Emulation.setDeviceMetricsOverride", "params": metrics(extra)
        }))
        .await;
        ctx.expect_error(
            88411,
            -32000,
            &format!("Emulation.setDeviceMetricsOverride does not support {setting}."),
        );
    }
    for invalid in [
        json!({"mobile": "true"}),
        json!({"displayFeature": {"orientation": "vertical", "offset": 200}}),
        json!({"displayFeature": {"orientation": "diagonal", "offset": 200, "maskLength": 0}}),
        json!({"devicePosture": {"type": "unknown"}}),
        json!({"scrollbarType": "unknown"}),
        json!({"screenOrientationLockEmulation": "true"}),
        json!({"viewportMeta": "unknown"}),
        json!({"width": -1}),
    ] {
        ctx.process_async(json!({
            "id": 88412, "method": "Emulation.setDeviceMetricsOverride", "params": metrics(invalid)
        }))
        .await;
        ctx.expect_error(88412, -32602, "InvalidParams");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn screen_orientation_and_position_update_held_objects_and_clear() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    set_metrics(&mut ctx, json!({})).await;
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe></iframe>",
        Some("SID-1"),
    )
    .await;
    evaluate(&mut ctx, r#"
        globalThis.heldOrientation = screen.orientation;
        globalThis.orientationCount = 0;
        globalThis.resizeCount = 0;
        heldOrientation.addEventListener('change', () => orientationCount++);
        addEventListener('resize', () => resizeCount++);
        globalThis.armOrientation = () => {
            globalThis.orientationChanged = new Promise(resolve => heldOrientation.addEventListener('change', e => resolve([
                e.isTrusted, e.target === heldOrientation, screen.orientation === heldOrientation,
                heldOrientation.type, heldOrientation.angle, screenX, screenY,
                frames[0].screen.orientation.type, frames[0].screenX
            ]), {once:true}));
        };
        armOrientation(); undefined
    "#).await;
    let override_params = json!({
        "positionX": 20,
        "positionY": 30,
        "screenOrientation": {"type": "portraitPrimary", "angle": 45}
    });
    set_metrics(&mut ctx, override_params.clone()).await;
    assert_eq!(
        evaluate(&mut ctx, "orientationChanged").await,
        json!([
            true,
            true,
            true,
            "portrait-primary",
            45,
            20,
            30,
            "portrait-primary",
            20
        ])
    );
    assert_eq!(
        evaluate(
            &mut ctx,
            r#"[
            screenLeft, screenTop, innerWidth, innerHeight,
            matchMedia('(orientation:landscape)').matches, resizeCount
        ]"#
        )
        .await,
        json!([20, 30, 200, 100, true, 0])
    );
    set_metrics(&mut ctx, override_params).await;
    assert_eq!(
        evaluate(
            &mut ctx,
            "new Promise(requestAnimationFrame).then(() => orientationCount)"
        )
        .await,
        json!(1)
    );
    for invalid in [
        json!({"positionX": -1}),
        json!({"positionX": 901}),
        json!({"screenOrientation": {"type": "portraitPrimary", "angle": 360}}),
        json!({"screenOrientation": {"type": "portraitPrimary", "angle": -1}}),
    ] {
        ctx.process_async(json!({
            "id": 88401,
            "sessionId": "SID-1",
            "method": "Emulation.setDeviceMetricsOverride",
            "params": metrics(invalid)
        }))
        .await;
        assert_eq!(ctx.take_response_by_id(88401)["error"]["code"], -32602);
    }
    assert_eq!(
        evaluate(&mut ctx, "[heldOrientation.angle,screenX,screenY]").await,
        json!([45, 20, 30])
    );
    evaluate(&mut ctx, "armOrientation(); undefined").await;
    expect_session_command_result(
        &mut ctx,
        88402,
        "SID-1",
        "Emulation.clearDeviceMetricsOverride",
        json!({}),
    )
    .await;
    assert_eq!(
        evaluate(&mut ctx, "orientationChanged").await,
        json!([
            true,
            true,
            true,
            "landscape-primary",
            0,
            0,
            0,
            "landscape-primary",
            0
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn device_pixel_ratio_updates_css_and_existing_media_queries_without_resizing() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    set_metrics(&mut ctx, json!({})).await;
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<style>body{color:red}@media(min-resolution:2dppx){body{color:green}}</style><body><iframe></iframe>",
        Some("SID-1"),
    ).await;
    evaluate(
        &mut ctx,
        r#"
        globalThis.heldStyle = getComputedStyle(document.body);
        globalThis.query = matchMedia('(resolution:2dppx)');
        globalThis.childQuery = frames[0].matchMedia('(resolution:2dppx)');
        globalThis.resizeCount = 0;
        addEventListener('resize', () => resizeCount++);
        visualViewport.addEventListener('resize', () => resizeCount++);
        globalThis.armMedia = () => {
            globalThis.mediaChanged = Promise.all([query, childQuery].map(q =>
                new Promise(resolve => q.addEventListener('change', e =>
                    resolve([e.matches, e.isTrusted]), {once: true}))));
        };
        undefined
    "#,
    )
    .await;
    for (ratio, matches, color) in [
        (2, true, "rgb(0, 128, 0)"),
        (0, false, "rgb(255, 0, 0)"),
        (2, true, "rgb(0, 128, 0)"),
        (3, false, "rgb(0, 128, 0)"),
    ] {
        evaluate(&mut ctx, "armMedia(); undefined").await;
        set_metrics(&mut ctx, json!({"deviceScaleFactor": ratio})).await;
        assert_eq!(
            evaluate(&mut ctx, "mediaChanged").await,
            json!([[matches, true], [matches, true]])
        );
        let expected_ratio = if ratio == 0 { 1 } else { ratio };
        assert_eq!(
            evaluate(
                &mut ctx,
                r#"[
                devicePixelRatio, frames[0].devicePixelRatio, heldStyle.color,
                matchMedia('(device-width:900px)').matches,
                frames[0].matchMedia('(device-width:900px)').matches, resizeCount
            ]"#
            )
            .await,
            json!([expected_ratio, expected_ratio, color, true, true, 0])
        );
    }
}
