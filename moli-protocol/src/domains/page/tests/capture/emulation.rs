use super::*;
use crate::domains::page::{
    PageScreencastCaptureCompletion, PageScreencastCaptureStart, PageScreencastRegistration,
};
use moli_core::page::RendererVisualStateToken;

const SESSION: &str = "SID-VIEW-CAPTURE";
const HTML: &str = r#"<!doctype html><style>
    html { background: blue; scrollbar-width: none }
    body { margin: 0 }
    #stripe { width: 40px; height: 100px; background: red }
</style><div id="stripe"></div>"#;

async fn command(ctx: &mut TestContext, method: &str, params: Value) -> Value {
    ctx.process_async(json!({
        "id": 88500, "sessionId": SESSION, "method": method, "params": params
    }))
    .await;
    let response = ctx
        .wait_for_scheduler_message(method, |message| message["id"] == 88500)
        .await;
    assert!(response["error"].is_null(), "{method}: {response}");
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{method}: {response}"
    );
    response["result"].clone()
}

async fn set_metrics(ctx: &mut TestContext, extra: Value) {
    let mut params = json!({
        "width": 200, "height": 100, "deviceScaleFactor": 1, "mobile": false
    });
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    command(ctx, "Emulation.setDeviceMetricsOverride", params).await;
}

async fn install_page(ctx: &mut TestContext) {
    install_active_screenshot_page(
        ctx,
        "BID-VIEW-CAPTURE",
        "TID-VIEW-CAPTURE",
        SESSION,
        &screenshot_data_url(HTML),
    )
    .await;
    set_metrics(ctx, json!({})).await;
}

fn start_screencast(ctx: &mut TestContext, extra: Value) -> PageScreencastRegistration {
    let mut params = json!({"format": "png"});
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let raw = json!({
        "id": 88501, "sessionId": SESSION, "method": "Page.startScreencast", "params": params
    })
    .to_string();
    let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("screencast registration should complete synchronously");
    };
    let (messages, events) = outcome.into_parts();
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == 88501 && message["result"] == json!({}))
    );
    events
        .into_iter()
        .find_map(|event| match event {
            CdpSchedulerEvent::PageScreencastStarted { registration } => Some(registration),
            _ => None,
        })
        .expect("screencast registration")
}

async fn poll_screencast(
    ctx: &mut TestContext,
    registration: &PageScreencastRegistration,
    previous: Option<RendererVisualStateToken>,
) -> PageScreencastCaptureCompletion {
    let PageScreencastCaptureStart::Pending(capture) = ctx
        .conn
        .start_page_screencast_frame_capture(registration, previous)
    else {
        panic!("registered screencast should start a renderer capture");
    };
    ctx.conn
        .complete_page_screencast_frame_capture(capture.wait().await)
}

fn take_frame(
    ctx: &mut TestContext,
    registration: &PageScreencastRegistration,
    completion: PageScreencastCaptureCompletion,
) -> (Value, RendererVisualStateToken) {
    let PageScreencastCaptureCompletion::Frame {
        event,
        visual_state,
    } = completion
    else {
        panic!("changed presentation must produce a frame");
    };
    let (event, _) = event.into_parts();
    assert_eq!(
        ctx.conn
            .acknowledge_page_screencast_frame_for_session_owner(
                Some(SESSION),
                registration.generation(),
            ),
        Some(true)
    );
    (event["params"].clone(), visual_state)
}

fn png(data: &Value) -> Vec<u8> {
    BASE64_STANDARD
        .decode(data.as_str().expect("encoded PNG"))
        .unwrap()
}

#[track_caller]
fn assert_blue_boundary(bytes: &[u8], dimensions: (u32, u32), boundary: Option<u32>) {
    assert_png_dimensions(bytes, dimensions.0, dimensions.1);
    let image = moli_image::decode_png(bytes).unwrap();
    let row = 10 * image.width as usize * 4;
    let first_blue = image.rgba[row..row + image.width as usize * 4]
        .chunks_exact(4)
        .position(|pixel| pixel == [0, 0, 255, 255])
        .map(|x| x as u32);
    assert_eq!(first_blue, boundary);
}

#[tokio::test(flavor = "multi_thread")]
async fn emulated_view_capture_distinguishes_layout_widget_and_explicit_clip() {
    let mut ctx = TestContext::new();
    install_page(&mut ctx).await;

    // Expected pixels were checked against Chromium. In particular, an
    // ordinary surface screenshot resets preview scale but keeps a forced
    // viewport; a clip supplied on captureScreenshot replaces that viewport.
    let viewport = json!({"x": 30, "y": 0, "width": 60, "height": 50, "scale": 2});
    let cases = [
        (
            json!({"scale": 0.5}),
            (200, 100),
            Some(20),
            (200, 100),
            Some(40),
            1,
        ),
        (
            json!({"scale": 2}),
            (200, 100),
            Some(80),
            (200, 100),
            Some(40),
            1,
        ),
        (
            json!({"viewport": viewport}),
            (120, 100),
            Some(20),
            (200, 100),
            Some(20),
            1,
        ),
        (
            json!({"deviceScaleFactor": 2, "viewport": viewport}),
            (240, 200),
            Some(40),
            (400, 200),
            Some(200),
            2,
        ),
        (
            json!({"scale": 0.5, "viewport": viewport}),
            (120, 100),
            Some(0),
            (200, 100),
            Some(20),
            1,
        ),
        (
            json!({"dontSetVisibleSize": true, "viewport": viewport}),
            (200, 100),
            Some(20),
            (200, 100),
            Some(20),
            1,
        ),
        (
            json!({"viewport": {"x": 30, "y": 0, "width": 0, "height": 0, "scale": 0}}),
            (200, 100),
            Some(0),
            (200, 100),
            Some(0),
            1,
        ),
        (
            json!({"viewport": {"x": 30, "y": 0, "width": 60, "height": 50, "scale": -1}}),
            (200, 100),
            Some(0),
            (200, 100),
            Some(0),
            1,
        ),
        (
            json!({"viewport": {"x": -1, "y": 0, "width": 60, "height": 50, "scale": 2}}),
            (120, 100),
            Some(40),
            (200, 100),
            Some(40),
            1,
        ),
    ];
    for (params, widget_size, widget_boundary, surface_size, surface_boundary, dpr) in cases {
        eprintln!("capture presentation: {params}");
        set_metrics(&mut ctx, json!({})).await;
        set_metrics(&mut ctx, params.clone()).await;
        let registration = start_screencast(&mut ctx, json!({}));
        let completion = poll_screencast(&mut ctx, &registration, None).await;
        let (frame, _) = take_frame(&mut ctx, &registration, completion);
        let image = png(&frame["data"]);
        assert_blue_boundary(&image, widget_size, widget_boundary);
        assert_eq!(
            frame["metadata"]["deviceWidth"].as_f64(),
            Some(f64::from(widget_size.0))
        );
        assert_eq!(
            frame["metadata"]["deviceHeight"].as_f64(),
            Some(f64::from(widget_size.1))
        );
        if params == json!({"scale": 0.5}) {
            assert_eq!(
                decode_png_pixel(&image, 150, 80),
                [0, 0, 255, 255],
                "the propagated solid canvas color fills the compositor surface"
            );
        }
        command(&mut ctx, "Page.stopScreencast", json!({})).await;

        let screenshot = command(&mut ctx, "Page.captureScreenshot", json!({})).await;
        assert_blue_boundary(&png(&screenshot["data"]), surface_size, surface_boundary);
        let clipped = command(
            &mut ctx,
            "Page.captureScreenshot",
            json!({
                "clip": {"x": 10, "y": 0, "width": 60, "height": 30, "scale": 1}
            }),
        )
        .await;
        assert_blue_boundary(&png(&clipped["data"]), (60 * dpr, 30 * dpr), Some(30 * dpr));
        let layout = command(
            &mut ctx,
            "Runtime.evaluate",
            json!({
                "expression": "[innerWidth, innerHeight, devicePixelRatio, visualViewport.scale]",
                "returnByValue": true
            }),
        )
        .await;
        assert_eq!(
            layout["result"]["value"],
            json!([200, 100, dpr, 1]),
            "{params}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn emulated_view_capture_invalidates_frames_survives_navigation_and_clears() {
    let mut ctx = TestContext::new();
    install_page(&mut ctx).await;
    let registration = start_screencast(&mut ctx, json!({"maxWidth": 100, "maxHeight": 50}));
    let completion = poll_screencast(&mut ctx, &registration, None).await;
    let (_, original) = take_frame(&mut ctx, &registration, completion);

    set_metrics(&mut ctx, json!({"scale": 0.5})).await;
    let completion = poll_screencast(&mut ctx, &registration, Some(original.clone())).await;
    let (frame, scaled) = take_frame(&mut ctx, &registration, completion);
    assert_ne!(original, scaled);
    assert_blue_boundary(&png(&frame["data"]), (100, 50), Some(10));
    assert_eq!(frame["metadata"]["deviceWidth"].as_f64(), Some(200.0));
    assert_eq!(frame["metadata"]["deviceHeight"].as_f64(), Some(100.0));
    assert!(matches!(
        poll_screencast(&mut ctx, &registration, Some(scaled)).await,
        PageScreencastCaptureCompletion::Unchanged
    ));

    ctx.install_navigation_fixture_for_session_owner(&screenshot_data_url(HTML), Some(SESSION))
        .await;
    let completion = poll_screencast(&mut ctx, &registration, None).await;
    let (frame, _) = take_frame(&mut ctx, &registration, completion);
    assert_blue_boundary(&png(&frame["data"]), (100, 50), Some(10));

    command(&mut ctx, "Emulation.clearDeviceMetricsOverride", json!({})).await;
    let layout = command(
        &mut ctx,
        "Runtime.evaluate",
        json!({
            "expression": "[innerWidth, innerHeight]", "returnByValue": true
        }),
    )
    .await;
    let completion = poll_screencast(&mut ctx, &registration, None).await;
    let (frame, _) = take_frame(&mut ctx, &registration, completion);
    assert_eq!(
        frame["metadata"]["deviceWidth"].as_f64(),
        layout["result"]["value"][0].as_f64()
    );
    assert_eq!(
        frame["metadata"]["deviceHeight"].as_f64(),
        layout["result"]["value"][1].as_f64()
    );
    let image = moli_image::decode_png(&png(&frame["data"])).unwrap();
    assert!(image.width <= 100 && image.height <= 50);
}

#[tokio::test(flavor = "multi_thread")]
async fn emulated_view_capture_resolves_zero_dimensions_and_scroll_at_layout() {
    let mut ctx = TestContext::new();
    install_page(&mut ctx).await;
    set_metrics(
        &mut ctx,
        json!({
            "width": 0, "height": 0, "scale": 2,
            "viewport": {"x": 30, "y": 0, "width": 60, "height": 50, "scale": 2}
        }),
    )
    .await;
    let layout = command(
        &mut ctx,
        "Runtime.evaluate",
        json!({
            "expression": "[innerWidth, innerHeight]", "returnByValue": true
        }),
    )
    .await;
    assert_eq!(layout["result"]["value"], json!([60, 50]));
    let screenshot = command(&mut ctx, "Page.captureScreenshot", json!({})).await;
    assert_blue_boundary(&png(&screenshot["data"]), (60, 50), Some(20));

    ctx.install_navigation_fixture_for_session_owner(
        &screenshot_data_url(&format!("{HTML}<style>body {{ height: 500px }}</style>")),
        Some(SESSION),
    )
    .await;
    set_metrics(
        &mut ctx,
        json!({
            "viewport": {"x": 0, "y": 30, "width": 60, "height": 50, "scale": 1}
        }),
    )
    .await;
    command(&mut ctx, "Page.captureScreenshot", json!({})).await;
    let scroll = command(
        &mut ctx,
        "Runtime.evaluate",
        json!({
            "expression": "scrollTo(0, 50); new Promise(requestAnimationFrame).then(() => scrollY)",
            "returnByValue": true, "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(scroll["result"]["value"], 50);
    command(
        &mut ctx,
        "Emulation.setDefaultBackgroundColorOverride",
        json!({
            "color": {"r": 0, "g": 0, "b": 0, "a": 0}
        }),
    )
    .await;
    let registration = start_screencast(&mut ctx, json!({}));
    let completion = poll_screencast(&mut ctx, &registration, None).await;
    let (frame, _) = take_frame(&mut ctx, &registration, completion);
    let image = png(&frame["data"]);
    assert_png_dimensions(&image, 60, 50);
    assert_eq!(decode_png_pixel(&image, 10, 0), [0, 0, 255, 255]);
    assert_eq!(decode_png_pixel(&image, 10, 25), [255, 0, 0, 255]);
    assert_eq!(decode_png_pixel(&image, 50, 25), [0, 0, 255, 255]);
}

#[tokio::test(flavor = "multi_thread")]
async fn emulated_view_capture_clips_background_images_over_a_transparent_base() {
    let mut ctx = TestContext::new();
    install_page(&mut ctx).await;
    let gradient =
        screenshot_data_url("<style>html{background:linear-gradient(blue,blue)}</style>");
    ctx.install_navigation_fixture_for_session_owner(&gradient, Some(SESSION))
        .await;
    command(
        &mut ctx,
        "Emulation.setDefaultBackgroundColorOverride",
        json!({
            "color": {"r": 0, "g": 0, "b": 0, "a": 0}
        }),
    )
    .await;
    set_metrics(&mut ctx, json!({"scale": 0.5})).await;
    let registration = start_screencast(&mut ctx, json!({}));
    let completion = poll_screencast(&mut ctx, &registration, None).await;
    let (frame, _) = take_frame(&mut ctx, &registration, completion);
    let image = png(&frame["data"]);
    assert_png_dimensions(&image, 200, 100);
    assert_eq!(decode_png_pixel(&image, 10, 10), [0, 0, 255, 255]);
    assert_eq!(decode_png_pixel(&image, 150, 80), [0; 4]);

    set_metrics(
        &mut ctx,
        json!({
            "viewport": {"x": 0, "y": 0, "width": 0, "height": 0, "scale": 0}
        }),
    )
    .await;
    let completion = poll_screencast(&mut ctx, &registration, None).await;
    let (frame, _) = take_frame(&mut ctx, &registration, completion);
    let image = moli_image::decode_png(&png(&frame["data"])).unwrap();
    assert!(image.rgba.iter().all(|byte| *byte == 0));
}
