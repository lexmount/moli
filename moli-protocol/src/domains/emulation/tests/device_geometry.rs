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
