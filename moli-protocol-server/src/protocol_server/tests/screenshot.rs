use super::*;
use std::{collections::HashSet, io::Cursor};

mod realtime_screencast;

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
    let (mut socket, _) = connect_async(
        discover_browser_websocket_url(addr)
            .await
            .expect("discover browser endpoint"),
    )
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
        spawn_test_protocol_server_with_layout_policy(LayoutPolicy::Lazy).await;
    let (mut browser, _) = connect_async(
        discover_browser_websocket_url(cdp_addr)
            .await
            .expect("discover browser endpoint"),
    )
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
            .as_chunks::<4>()
            .0
            .iter()
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
async fn websocket_cdp_box_edges_share_the_device_pixel_grid() {
    let html = r#"<!doctype html>
<style>
body { margin: 0; background: white }
.frame {
    position: absolute; left: 10.5px; top: 12.25px;
    box-sizing: border-box; width: 120px; height: 42px;
    border: 1.5px solid transparent; border-radius: 12px;
    background: linear-gradient(white, white) padding-box,
                linear-gradient(160deg, #2955ff, #ab50ff) border-box;
}
.border-origin { top: 64.25px; background-origin: border-box }
.solid { top: 116.25px; background: #2955ff padding-box }
.image {
    top: 168.25px;
    background: url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mPQDP0PAAIoAX46fBFaAAAAAElFTkSuQmCC)
        0.5px 1.25px / 7.25px 6.5px border-box padding-box;
}
.single-image {
    top: 220.25px; background-repeat: no-repeat;
    background-size: 100% 100%; background-position: 0 0;
}
.bordered { top: 272.25px; background: #2955ff; border-color: #dc2800 }
.overflow { top: 324.25px; background: none; overflow: hidden }
.overflow > div { position: absolute; inset: -4px; background: #2955ff }
.full-image {
    top: 376.25px; border: 0; border-radius: 0;
    background-color: #dc2800; background-clip: border-box;
}
.full-svg {
    top: 428.25px;
    background-image: url(data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxIiBoZWlnaHQ9IjEiPjxyZWN0IHdpZHRoPSIxIiBoZWlnaHQ9IjEiIGZpbGw9IiMyOTU1ZmYiLz48L3N2Zz4=);
}
.border-image {
    top: 480.25px; background: #dc2800; border-radius: 0;
    border-image: linear-gradient(#2955ff, #2955ff) 2 fill / 1.5px stretch;
}
.gradient-mask {
    top: 532.25px; border: 0; border-radius: 0; background: #2955ff;
    mask-image: linear-gradient(black, black);
}
.image-mask {
    top: 584.25px; border: 0; border-radius: 0; background: #2955ff;
    mask: url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mPQDP0PAAIoAX46fBFaAAAAAElFTkSuQmCC)
        0 0 / 100% 100% no-repeat border-box;
}
.embedded { top: 636.25px; border: 0; border-radius: 0; background: #dc2800 }
</style>
<div class="frame"></div><div class="frame border-origin"></div>
<div class="frame solid"></div><div class="frame image"></div>
<div class="frame image single-image"></div>
<div class="frame bordered"></div><div class="frame overflow"><div></div></div>
<div class="frame image single-image full-image"></div>
<div class="frame image single-image full-image full-svg"></div>
<div class="frame border-image"></div>
<div class="frame gradient-mask"></div><div class="frame image-mask"></div>
<iframe class="frame embedded" srcdoc="<style>body { margin: 0; background: #2955ff }</style>"></iframe>
<img hidden src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mPQDP0PAAIoAX46fBFaAAAAAElFTkSuQmCC">
<img hidden src="data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxIiBoZWlnaHQ9IjEiPjxyZWN0IHdpZHRoPSIxIiBoZWlnaHQ9IjEiIGZpbGw9IiMyOTU1ZmYiLz48L3N2Zz4=">"#;
    let fixture_url = format!(
        "data:text/html;charset=utf-8,{}",
        url::form_urlencoded::byte_serialize(html.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    );
    let (cdp_addr, protocol_server) =
        spawn_test_protocol_server_with_runtime_config(NavigationRuntimeConfig::new(
            protocol_server_test_fetch_config(FetchConfig::default()),
            OptionalResourceFetchMask::IMAGE,
            true,
            LayoutPolicy::Realtime,
        ))
        .await;
    let (mut browser, _) = connect_async(
        discover_browser_websocket_url(cdp_addr)
            .await
            .expect("discover browser endpoint"),
    )
    .await
    .expect("connect screenshot CDP websocket");
    let target = open_screenshot_target(&mut browser, &fixture_url).await;
    let decoded = send_cdp_command(
        &mut browser,
        7,
        "Runtime.evaluate",
        Some(&target.session_id),
        json!({
            "expression": "Promise.all(Array.from(document.images, image => image.decode())).then(() => true)",
            "awaitPromise": true,
            "returnByValue": true,
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&decoded, 7)["result"]["result"]["value"],
        true
    );

    for (index, (scale, border_width)) in [(1.0_f32, 1), (1.25, 1), (2.0, 3), (1.0, 1)]
        .into_iter()
        .enumerate()
    {
        let id = 8 + 2 * index as u64;
        let metrics = send_cdp_command(
            &mut browser,
            id,
            "Emulation.setDeviceMetricsOverride",
            Some(&target.session_id),
            json!({
                "width": 160,
                "height": 704,
                "deviceScaleFactor": scale,
                "mobile": false,
            }),
        )
        .await;
        assert_cdp_success(&metrics, id);
        let png = capture_png(&mut browser, &target.session_id, id + 1).await;
        let (width, height, pixels) = decode_png(&png);
        assert_eq!(
            (width, height),
            ((160.0 * scale) as u32, (704.0 * scale) as u32)
        );

        let left = (10.5 * scale).round() as i32;
        let right = (130.5 * scale).round() as i32;
        let center_x = (left + right) / 2;
        let white = [255; 4];
        let blue = [41, 85, 255, 255];
        for (border_top, border_width, border_color, inside_color) in [
            (12.25, border_width, None, white),
            (64.25, border_width, None, white),
            (116.25, border_width, Some(white), blue),
            (168.25, border_width, Some(white), blue),
            (220.25, border_width, Some(white), blue),
            (272.25, border_width, Some([220, 40, 0, 255]), blue),
            (324.25, border_width, Some(white), blue),
            (376.25, 0, Some(white), blue),
            (428.25, 0, Some(white), blue),
            (480.25, 0, Some(white), blue),
            (532.25, 0, Some(white), blue),
            (584.25, 0, Some(white), blue),
            (636.25, 0, Some(white), blue),
        ] {
            let top = (border_top * scale).round() as i32;
            let bottom = ((border_top + 42.0) * scale).round() as i32;
            let center_y = (top + bottom) / 2;
            for (x, y, dx, dy) in [
                (left, center_y, 1, 0),
                (right - 1, center_y, -1, 0),
                (center_x, top, 0, 1),
                (center_x, bottom - 1, 0, -1),
            ] {
                for inset in -1..=border_width {
                    let color = pixel(
                        &pixels,
                        width,
                        (x + dx * inset) as u32,
                        (y + dy * inset) as u32,
                    );
                    let expected = if inset < 0 {
                        Some(white)
                    } else if inset < border_width {
                        border_color
                    } else {
                        Some(inside_color)
                    };
                    if let Some(expected) = expected {
                        assert_eq!(color, expected, "({x}, {y}) at inset {inset}, DPR {scale}");
                    } else {
                        assert!(
                            color[1] < 100 && color[2] >= 254,
                            "gradient border ({x}, {y}) at inset {inset}, DPR {scale}: {color:?}"
                        );
                    }
                }
            }
        }
    }

    abort_test_cdp_server(protocol_server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_capture_screenshot_paints_fresh_canvas_pixels() {
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(screenshot_fixture_app(), "canvas-screenshot");
    let fixture_url = format!("http://{fixture_addr}/canvas-screenshot");
    let (cdp_addr, protocol_server) =
        spawn_test_protocol_server_with_layout_policy(LayoutPolicy::Lazy).await;
    let (mut browser, _) = connect_async(
        discover_browser_websocket_url(cdp_addr)
            .await
            .expect("discover browser endpoint"),
    )
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
    let (mut browser, _) = connect_async(
        discover_browser_websocket_url(cdp_addr)
            .await
            .expect("discover browser endpoint"),
    )
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_mobile_root_scrolls_using_frame_size() {
    let html = r#"<!doctype html>
        <meta name="viewport" content="width=980, minimum-scale=1">
        <style>
        html { overflow-y: hidden; background: white; }
        body { margin: 0; }
        #wide { width: 100vw; height: 100px; position: relative; }
        #right { position: absolute; right: 40px; top: 0; width: 20px; height: 20px; background: red; }
        </style>
        <div id="wide"><div id="right"></div></div>"#;
    let url = format!(
        "data:text/html,{}",
        percent_encoding::utf8_percent_encode(html, percent_encoding::NON_ALPHANUMERIC),
    );
    for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
        let (addr, server) = spawn_test_protocol_server_with_layout_policy(policy).await;
        let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
            .await
            .unwrap();
        let target = open_screenshot_target(&mut socket, &url).await;
        let session = target.session_id.as_str();
        let applied = send_cdp_command(
            &mut socket,
            7,
            "Emulation.setDeviceMetricsOverride",
            Some(session),
            json!({"width":375,"height":667,"deviceScaleFactor":1,"mobile":true}),
        )
        .await;
        assert_cdp_success(&applied, 7);

        // Lazy getters and setters consume the last published layout.
        let before = capture_png(&mut socket, session, 8).await;
        let (width, height, pixels) = decode_png(&before);
        assert_eq!((width, height), (375, 667), "{policy:?}");
        assert_eq!(pixel(&pixels, width, 365, 10), [255; 4], "{policy:?}");

        let evaluated = send_cdp_command(
            &mut socket,
            9,
            "Runtime.evaluate",
            Some(session),
            json!({
                "returnByValue": true,
                "expression": r#"(() => {
                    const root = document.scrollingElement;
                    root.scrollLeft = 605;
                    const requestedLeft = root.scrollLeft;
                    root.scrollLeft = 10000;
                    root.scrollTop = 10000;
                    const limits = [root.scrollLeft, root.scrollTop];
                    root.scrollLeft = 0;
                    root.scrollTop = 0;
                    document.getElementById('right').scrollIntoView({block: 'nearest', inline: 'end'});
                    return {
                        frame: [innerWidth, innerHeight],
                        css: [root.clientWidth, root.clientHeight],
                        wide: document.getElementById('wide').getBoundingClientRect().width,
                        media: matchMedia('(min-width: 600px)').matches,
                        requestedLeft, limits,
                        scrolledIntoView: [scrollX, scrollY],
                        outside: document.elementFromPoint(375, 10) === null,
                    };
                })()"#,
            }),
        )
        .await;
        assert_cdp_success(&evaluated, 9);
        assert_eq!(
            response_by_id(&evaluated, 9)["result"]["result"]["value"],
            json!({
                "frame": [375, 667],
                "css": [980, 1743],
                "wide": 980,
                "media": true,
                "requestedLeft": 605,
                "limits": [605, 1076],
                "scrolledIntoView": [565, 0],
                "outside": true,
            }),
            "{policy:?}",
        );

        let after = capture_png(&mut socket, session, 10).await;
        let (width, height, pixels) = decode_png(&after);
        assert_eq!((width, height), (375, 667), "{policy:?}");
        assert_eq!(
            pixel(&pixels, width, 365, 10),
            [255, 0, 0, 255],
            "{policy:?}",
        );
        let hit = send_cdp_command(
            &mut socket,
            11,
            "Runtime.evaluate",
            Some(session),
            json!({"expression": r#"
                (() => {
                    const hit = document.elementFromPoint(365, 10)?.id;
                    document.documentElement.style.direction = 'rtl';
                    document.getElementById('wide').style.width = '1180px';
                    return hit;
                })()
            "#}),
        )
        .await;
        assert_cdp_success(&hit, 11);
        assert_eq!(
            response_by_id(&hit, 11)["result"]["result"]["value"],
            "right",
            "{policy:?}"
        );

        capture_png(&mut socket, session, 12).await;
        let rtl = send_cdp_command(
            &mut socket,
            13,
            "Runtime.evaluate",
            Some(session),
            json!({"returnByValue": true, "expression": r#"
                (() => {
                    const root = document.scrollingElement;
                    root.scrollLeft = -10000;
                    const minimum = [root.scrollLeft, scrollX];
                    root.scrollLeft = 10000;
                    return [minimum, root.scrollLeft];
                })()
            "#}),
        )
        .await;
        assert_cdp_success(&rtl, 13);
        assert_eq!(
            response_by_id(&rtl, 13)["result"]["result"]["value"],
            json!([[-200, -200], 605]),
            "{policy:?}",
        );
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_mobile_fixed_layout_and_intersection_use_the_frame() {
    let html = r#"<!doctype html>
        <meta name="viewport" content="width=980, minimum-scale=1">
        <style>
        html,body { margin: 0; background: white; }
        #flow { width: 100vw; height: 10px; }
        #fixed { position: fixed; right: 0; bottom: 0; width: 20px; height: 20px; background: red; z-index: 1; }
        #percent { position: fixed; right: 20%; width: 40%; height: 25%; }
        #container { position: absolute; left: 50px; top: 120px; width: 200px; height: 100px; transform: translate(0); }
        #nested { position: fixed; right: 0; bottom: 0; width: 20px; height: 20px; }
        #absolute { position: absolute; right: 0; bottom: 0; width: 20px; height: 20px; }
        #target { position: absolute; left: 500px; top: 10px; width: 50px; height: 20px; }
        </style>
        <div id="flow"></div><div id="percent"></div><div id="fixed"></div>
        <div id="container"><div id="nested"></div></div>
        <div id="absolute"></div><div id="target"></div>"#;
    let url = format!(
        "data:text/html,{}",
        percent_encoding::utf8_percent_encode(html, percent_encoding::NON_ALPHANUMERIC),
    );
    for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
        let (addr, server) = spawn_test_protocol_server_with_layout_policy(policy).await;
        let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
            .await
            .unwrap();
        let target = open_screenshot_target(&mut socket, &url).await;
        let session = target.session_id.as_str();
        let applied = send_cdp_command(
            &mut socket,
            7,
            "Emulation.setDeviceMetricsOverride",
            Some(session),
            json!({"width":375,"height":667,"deviceScaleFactor":1,"mobile":true}),
        )
        .await;
        assert_cdp_success(&applied, 7);
        let before = capture_png(&mut socket, session, 8).await;
        let (width, height, pixels) = decode_png(&before);
        assert_eq!((width, height), (375, 667), "{policy:?}");
        assert_eq!(
            pixel(&pixels, width, 365, 657),
            [255, 0, 0, 255],
            "{policy:?}"
        );

        let measured = send_cdp_command(
            &mut socket,
            9,
            "Runtime.evaluate",
            Some(session),
            json!({"returnByValue":true,"awaitPromise":true,"expression":r#"
                (async () => {
                    const rects = ['fixed', 'percent', 'nested', 'absolute', 'flow'].map(id => {
                        const {x, y, width, height} = document.getElementById(id).getBoundingClientRect();
                        return [x, y, width, height];
                    });
                    const intersections = await Promise.all(['0px', '0px 40% 0px 0px'].map(rootMargin =>
                        new Promise(resolve => {
                            new IntersectionObserver(([entry], observer) => {
                                observer.disconnect();
                                resolve([entry.rootBounds.width, entry.rootBounds.height,
                                    entry.isIntersecting, entry.intersectionRatio]);
                            }, {rootMargin}).observe(document.getElementById('target'));
                        })
                    ));
                    return [rects, intersections];
                })()
            "#}),
        )
        .await;
        assert_cdp_success(&measured, 9);
        assert_eq!(
            response_by_id(&measured, 9)["result"]["result"]["value"],
            json!([
                [
                    [355, 647, 20, 20],
                    [150, 10, 150, 166.75],
                    [230, 200, 20, 20],
                    [960, 1723, 20, 20],
                    [0, 0, 980, 10]
                ],
                [[375, 667, false, 0], [525, 667, true, 0.5]],
            ]),
            "{policy:?}",
        );

        let scrolled = send_cdp_command(
            &mut socket,
            10,
            "Runtime.evaluate",
            Some(session),
            json!({"expression":"window.scrollTo(605,1076)"}),
        )
        .await;
        assert_cdp_success(&scrolled, 10);
        let after = capture_png(&mut socket, session, 11).await;
        let (width, height, pixels) = decode_png(&after);
        assert_eq!((width, height), (375, 667), "{policy:?}");
        assert_eq!(
            pixel(&pixels, width, 365, 657),
            [255, 0, 0, 255],
            "{policy:?}"
        );
        let _ = socket.close(None).await;
        abort_test_cdp_server(server).await;
    }
}

#[tokio::test]
async fn websocket_cdp_highlights_and_following_queries_complete_while_paused() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let html = "<!doctype html><style>body{margin:0;background:white} #target{position:absolute;left:8px;top:8px;width:12px;height:12px}</style><div id=target></div>";
    let url = format!(
        "data:text/html,{}",
        percent_encoding::utf8_percent_encode(html, percent_encoding::NON_ALPHANUMERIC),
    );
    let target = open_screenshot_target(&mut socket, &url).await;
    let session = target.session_id.as_str();
    for (id, method) in [
        (7, "Runtime.enable"),
        (8, "Debugger.enable"),
        (9, "DOM.enable"),
        (10, "Overlay.enable"),
    ] {
        assert_cdp_success(
            &send_cdp_command(&mut socket, id, method, Some(session), json!({})).await,
            id,
        );
    }
    let evaluated = send_cdp_command(
        &mut socket,
        11,
        "Runtime.evaluate",
        Some(session),
        json!({"expression":"document.getElementById('target')"}),
    )
    .await;
    let object_id = response_by_id(&evaluated, 11)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime object id");
    let described = send_cdp_command(
        &mut socket,
        12,
        "DOM.describeNode",
        Some(session),
        json!({"objectId":object_id}),
    )
    .await;
    let backend_node_id = response_by_id(&described, 12)["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("backend node id");
    send_cdp_command_without_wait(
        &mut socket,
        13,
        "Runtime.evaluate",
        Some(session),
        json!({"expression":"debugger; 42","returnByValue":true}),
    )
    .await;
    recv_until_match(&mut socket, |message| {
        message["sessionId"] == session && message["method"] == "Debugger.paused"
    })
    .await;

    let clip = json!({"clip":{"x":0,"y":0,"width":32,"height":32,"scale":1}});
    let node_colors = json!({"contentColor":{"r":0,"g":0,"b":255}});
    let commands = [
        (
            14,
            "Overlay.highlightRect",
            json!({"x":2,"y":2,"width":4,"height":4,"color":{"r":255,"g":0,"b":0}}),
        ),
        (
            15,
            "DOM.getBoxModel",
            json!({"backendNodeId":backend_node_id}),
        ),
        (
            16,
            "Overlay.highlightNode",
            json!({"backendNodeId":backend_node_id,"highlightConfig":node_colors}),
        ),
        (
            17,
            "Overlay.highlightNode",
            json!({"objectId":object_id,"highlightConfig":node_colors}),
        ),
        (
            18,
            "Overlay.highlightNode",
            json!({"objectId":"invalid","highlightConfig":node_colors}),
        ),
        (19, "Page.captureScreenshot", clip.clone()),
        (
            20,
            "Runtime.getProperties",
            json!({"objectId":object_id,"ownProperties":true}),
        ),
        (21, "Overlay.hideHighlight", json!({})),
        (22, "Page.captureScreenshot", clip),
        (23, "Overlay.disable", json!({})),
        (
            24,
            "DOM.getBoxModel",
            json!({"backendNodeId":backend_node_id}),
        ),
    ];
    let paused = timeout(Duration::from_secs(10), async {
        let mut remaining = commands
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<std::collections::BTreeSet<_>>();
        // Pipeline the requests to exercise the same-session Main queue.
        for (id, method, params) in commands {
            send_cdp_command_without_wait(&mut socket, id, method, Some(session), params).await;
        }
        recv_until_match(&mut socket, |message| {
            if message["sessionId"] == session
                && let Some(id) = message["id"].as_u64()
            {
                remaining.remove(&id);
            }
            remaining.is_empty()
        })
        .await
    })
    .await;

    let mut resumed =
        send_cdp_command(&mut socket, 25, "Debugger.resume", Some(session), json!({})).await;
    if !resumed.iter().any(|message| message["id"] == 13) {
        resumed.extend(recv_until_id(&mut socket, 13).await);
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;

    let paused = paused.expect("Overlay commands and queries must finish before resuming");
    assert!(
        !paused
            .iter()
            .any(|message| message["id"] == 13 || message["method"] == "Debugger.resumed")
    );
    for id in (14..=24).filter(|id| *id != 18) {
        assert_cdp_success(&paused, id);
    }
    assert_eq!(response_by_id(&paused, 18)["error"]["code"], -32000);
    assert_eq!(response_by_id(&paused, 15)["result"]["model"]["width"], 12);
    assert_eq!(response_by_id(&paused, 24)["result"]["model"]["width"], 12);
    assert!(response_by_id(&paused, 20)["result"]["result"].is_array());
    for (id, color) in [(19, [0, 0, 255, 255]), (22, [255; 4])] {
        let result = response_by_id(&paused, id);
        let bytes = BASE64_STANDARD
            .decode(result["result"]["data"].as_str().unwrap())
            .unwrap();
        let (width, _, pixels) = decode_png(&bytes);
        assert_eq!(pixel(&pixels, width, 10, 10), color);
    }
    assert_cdp_success(&resumed, 25);
    assert_eq!(
        response_by_id(&resumed, 13)["result"]["result"]["value"],
        42
    );
}
