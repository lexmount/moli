use super::*;
use std::{collections::HashSet, io::Cursor};

const SCREENSHOT_FIXTURE: &str =
    include_str!("../../../../moli-renderer-v8/tests/fixtures/layout-screenshot-poc.html");
const CANVAS_SCREENSHOT_FIXTURE: &str = r#"<!doctype html>
<style>html,body{margin:0;padding:0;background:white}canvas{display:block;width:40px;height:20px;image-rendering:pixelated}</style>
<canvas id="canvas" width="4" height="2"></canvas>
<script>
const context = document.getElementById('canvas').getContext('2d');
context.fillStyle = '#ff0000';
context.fillRect(0, 0, 2, 2);
context.fillStyle = '#0000ff';
context.fillRect(2, 0, 2, 2);
</script>"#;

fn screenshot_fixture_app() -> Router {
    Router::new()
        .route(
            "/layout-screenshot-poc",
            get(|| async {
                (
                    [(
                        axum::http::header::CONTENT_TYPE.as_str(),
                        "text/html; charset=utf-8",
                    )],
                    SCREENSHOT_FIXTURE,
                )
            }),
        )
        .route(
            "/canvas-screenshot",
            get(|| async {
                (
                    [(
                        axum::http::header::CONTENT_TYPE.as_str(),
                        "text/html; charset=utf-8",
                    )],
                    CANVAS_SCREENSHOT_FIXTURE,
                )
            }),
        )
}

async fn open_screenshot_target(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    fixture_url: &str,
) -> TestCdpTargetSession {
    let browser_context_id = cdp_create_browser_context(socket, 1).await;
    let target = cdp_create_attached_target(socket, 2, &browser_context_id).await;
    assert_cdp_success(
        &send_cdp_command(
            socket,
            4,
            "Page.enable",
            Some(&target.session_id),
            json!({}),
        )
        .await,
        4,
    );
    assert_cdp_success(
        &send_cdp_command(
            socket,
            5,
            "Emulation.setDeviceMetricsOverride",
            Some(&target.session_id),
            json!({
                "width": 800,
                "height": 600,
                "deviceScaleFactor": 1,
                "mobile": false,
            }),
        )
        .await,
        5,
    );
    let navigation =
        cdp_navigate_and_wait_for_load(socket, 6, &target.session_id, fixture_url).await;
    assert_cdp_success(&navigation, 6);
    target
}

async fn capture_png(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    session_id: &str,
    id: u64,
) -> Vec<u8> {
    let messages = send_cdp_command(
        socket,
        id,
        "Page.captureScreenshot",
        Some(session_id),
        json!({
            "format": "png",
            "quality": 100,
            "fromSurface": true,
            "captureBeyondViewport": false,
        }),
    )
    .await;
    let response = response_by_id(&messages, id);
    assert_eq!(response["sessionId"], json!(session_id));
    assert!(
        response.get("error").is_none(),
        "capture failed: {response:#?}"
    );
    let data = response["result"]["data"]
        .as_str()
        .expect("captureScreenshot base64 data");
    BASE64_STANDARD
        .decode(data)
        .expect("captureScreenshot should return valid base64")
}

fn response_by_id(messages: &[serde_json::Value], id: u64) -> &serde_json::Value {
    messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing response id {id}: {messages:#?}"))
}

fn assert_cdp_success(messages: &[serde_json::Value], id: u64) {
    let response = response_by_id(messages, id);
    assert!(
        response.get("error").is_none(),
        "command failed: {response:#?}"
    );
}

fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("valid PNG header");
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("decoded PNG buffer size fits in memory")
    ];
    let output = reader.next_frame(&mut buffer).expect("valid PNG data");
    assert_eq!(output.color_type, png::ColorType::Rgba);
    assert_eq!(output.bit_depth, png::BitDepth::Eight);
    buffer.truncate(output.buffer_size());
    (output.width, output.height, buffer)
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;
    pixels[offset..offset + 4]
        .try_into()
        .expect("one RGBA pixel")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_hidden_scrollbars_follow_targets_navigation_and_resize() {
    let config = protocol_server_test_runtime_config(
        protocol_server_test_fetch_config(FetchConfig::default()),
        OptionalResourceFetchMask::NONE,
    )
    .with_scrollbars_hidden(true);
    let (addr, server) = spawn_test_protocol_server_with_runtime_config(config).await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("CDP socket");
    let url = "data:text/html,<!doctype html><body style='margin:0'><div style='width:100vw;height:2000px;background:rgb(0,255,0)'></div>";
    for _ in 0..2 {
        let target = open_screenshot_target(&mut socket, url).await;
        for width in [1280, 1000] {
            assert_cdp_success(
                &send_cdp_command(
                    &mut socket,
                    20,
                    "Emulation.setDeviceMetricsOverride",
                    Some(&target.session_id),
                    json!({"width":width,"height":720,"deviceScaleFactor":1,"mobile":false}),
                )
                .await,
                20,
            );
            let png = capture_png(&mut socket, &target.session_id, 21).await;
            let (image_width, _, pixels) = decode_png(&png);
            assert_eq!(image_width, width);
            assert_eq!(pixel(&pixels, width, width - 8, 300), [0, 255, 0, 255]);
            let result = send_cdp_command(
                &mut socket, 22, "Runtime.evaluate", Some(&target.session_id),
                json!({"expression":"JSON.stringify([innerWidth,document.documentElement.clientWidth,document.documentElement.scrollWidth])","returnByValue":true})
            ).await;
            assert_cdp_success(&result, 22);
            assert_eq!(
                response_by_id(&result, 22)["result"]["result"]["value"],
                json!(format!("[{width},{width},{width}]"))
            );
        }
        assert_cdp_success(
            &send_cdp_command(
                &mut socket,
                23,
                "Input.dispatchMouseEvent",
                Some(&target.session_id),
                json!({"type":"mouseWheel","x":100,"y":300,"deltaX":0,"deltaY":100}),
            )
            .await,
            23,
        );
        let scroll = send_cdp_command(
            &mut socket,
            24,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({"expression":"scrollY > 0","returnByValue":true}),
        )
        .await;
        assert_cdp_success(&scroll, 24);
        assert_eq!(
            response_by_id(&scroll, 24)["result"]["result"]["value"],
            true
        );
        assert_cdp_success(
            &cdp_navigate_and_wait_for_load(&mut socket, 25, &target.session_id, url).await,
            25,
        );
        capture_png(&mut socket, &target.session_id, 26).await;
        let result = send_cdp_command(
            &mut socket, 27, "Runtime.evaluate", Some(&target.session_id),
            json!({"expression":"innerWidth === document.documentElement.clientWidth","returnByValue":true})
        ).await;
        assert_cdp_success(&result, 27);
        assert_eq!(
            response_by_id(&result, 27)["result"]["result"]["value"],
            true
        );
        send_cdp_command(
            &mut socket,
            28,
            "Target.closeTarget",
            None,
            json!({"targetId":target.target_id}),
        )
        .await;
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_capture_screenshot_tracks_paint_and_layout_mutations() {
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(screenshot_fixture_app(), "layout-screenshot");
    let fixture_url = format!("http://{fixture_addr}/layout-screenshot-poc");
    let (cdp_addr, protocol_server) =
        spawn_test_protocol_server_with_layout_policy(LayoutPolicy::OnDemand).await;
    let (mut browser, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect screenshot CDP websocket");
    let target = open_screenshot_target(&mut browser, &fixture_url).await;

    let initial_png = capture_png(&mut browser, &target.session_id, 7).await;
    let (width, height, initial_pixels) = decode_png(&initial_png);
    assert_eq!((width, height), (800, 600));
    assert_eq!(pixel(&initial_pixels, width, 50, 20), [240, 40, 40, 255]);
    assert_eq!(pixel(&initial_pixels, width, 150, 20), [40, 200, 80, 255]);
    assert_eq!(pixel(&initial_pixels, width, 250, 20), [40, 100, 240, 255]);
    assert_eq!(
        pixel(&initial_pixels, width, 700, 500),
        [255, 255, 255, 255]
    );
    assert!(
        initial_pixels
            .chunks_exact(4)
            .map(|rgba| [rgba[0], rgba[1], rgba[2], rgba[3]])
            .collect::<HashSet<_>>()
            .len()
            > 8,
        "fixture screenshot should contain multiple painted colors"
    );

    assert_cdp_success(
        &send_cdp_command(
            &mut browser,
            8,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": "document.documentElement.style.backgroundColor = 'rgb(20, 30, 40)'"
            }),
        )
        .await,
        8,
    );
    let paint_png = capture_png(&mut browser, &target.session_id, 9).await;
    let (_, _, paint_pixels) = decode_png(&paint_png);
    assert_eq!(pixel(&paint_pixels, width, 700, 500), [20, 30, 40, 255]);
    assert_ne!(
        paint_png, initial_png,
        "paint mutation should change the PNG"
    );

    assert_cdp_success(
        &send_cdp_command(
            &mut browser,
            10,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": "document.querySelector('#cards').style.flexDirection = 'column'"
            }),
        )
        .await,
        10,
    );
    let layout_png = capture_png(&mut browser, &target.session_id, 11).await;
    let (_, _, layout_pixels) = decode_png(&layout_png);
    assert_eq!(pixel(&layout_pixels, width, 150, 20), [255, 255, 255, 255]);
    assert_eq!(pixel(&layout_pixels, width, 50, 60), [40, 200, 80, 255]);
    assert_ne!(
        layout_png, paint_png,
        "layout mutation should change geometry"
    );

    let _ = send_cdp_command(
        &mut browser,
        12,
        "Target.closeTarget",
        None,
        json!({ "targetId": target.target_id }),
    )
    .await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_capture_screenshot_paints_fresh_canvas_pixels() {
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(screenshot_fixture_app(), "canvas-screenshot");
    let fixture_url = format!("http://{fixture_addr}/canvas-screenshot");
    let (cdp_addr, protocol_server) =
        spawn_test_protocol_server_with_layout_policy(LayoutPolicy::OnDemand).await;
    let (mut browser, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect canvas screenshot CDP websocket");
    let target = open_screenshot_target(&mut browser, &fixture_url).await;

    let initial_png = capture_png(&mut browser, &target.session_id, 7).await;
    let (width, _, initial_pixels) = decode_png(&initial_png);
    assert_eq!(pixel(&initial_pixels, width, 5, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(&initial_pixels, width, 35, 5), [0, 0, 255, 255]);

    assert_cdp_success(
        &send_cdp_command(
            &mut browser,
            8,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": "document.getElementById('canvas').setAttribute('width', '4')"
            }),
        )
        .await,
        8,
    );
    let reset_png = capture_png(&mut browser, &target.session_id, 9).await;
    let (_, _, reset_pixels) = decode_png(&reset_png);
    assert_eq!(pixel(&reset_pixels, width, 5, 5), [255, 255, 255, 255]);
    assert_ne!(reset_png, initial_png);

    assert_cdp_success(
        &send_cdp_command(
            &mut browser,
            10,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({
                "expression": "(() => { const next = document.getElementById('canvas').getContext('2d'); next.fillStyle = '#008000'; next.fillRect(0, 0, 4, 2); })()"
            }),
        )
        .await,
        10,
    );
    let mutated_png = capture_png(&mut browser, &target.session_id, 11).await;
    let (_, _, mutated_pixels) = decode_png(&mutated_png);
    assert_eq!(pixel(&mutated_pixels, width, 5, 5), [0, 128, 0, 255]);
    assert_ne!(mutated_png, reset_png);

    let _ = send_cdp_command(
        &mut browser,
        12,
        "Target.closeTarget",
        None,
        json!({ "targetId": target.target_id }),
    )
    .await;
    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_capture_screenshot_preserves_default_mock_boundary() {
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(screenshot_fixture_app(), "no-layout-screenshot");
    let fixture_url = format!("http://{fixture_addr}/layout-screenshot-poc");
    let (cdp_addr, protocol_server) =
        spawn_test_protocol_server_with_layout_policy(LayoutPolicy::Mock).await;
    let (mut browser, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect no-layout CDP websocket");
    let target = open_screenshot_target(&mut browser, &fixture_url).await;

    let messages = send_cdp_command(
        &mut browser,
        7,
        "Page.captureScreenshot",
        Some(&target.session_id),
        json!({}),
    )
    .await;
    let response = response_by_id(&messages, 7);
    assert_eq!(response["sessionId"], json!(target.session_id));
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!(
            "Page.captureScreenshot is not supported: renderer layout is disabled; start Moli with --layout."
        )
    );
    assert!(response.get("result").is_none());

    let _ = send_cdp_command(
        &mut browser,
        8,
        "Target.closeTarget",
        None,
        json!({ "targetId": target.target_id }),
    )
    .await;
    abort_test_cdp_server(protocol_server).await;
}
