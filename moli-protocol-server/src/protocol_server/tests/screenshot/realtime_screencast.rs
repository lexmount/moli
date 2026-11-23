use super::*;

type CastSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const CAST_INTERVAL_MS: u32 = 17;
const CAST_BINDING: &str = "__moliScreencastLifecycleDone";
const REALTIME_CAST_HTML: &str = r#"<!doctype html>
<style>
html,body{margin:0;padding:0;width:64px;height:64px;background:white;overflow:hidden}
#target{position:absolute;left:0;top:0;width:20px;height:20px;background:red}
#probe{position:absolute;left:62px;top:1000px;width:1px;height:1px}
</style><div id="target"></div><div id="probe"></div>
<script>
window.runCastLifecycle = (phase, finalWidth, finalColor, minimumElapsed) => {
  const target = document.getElementById('target');
  const probe = document.getElementById('probe');
  const started = performance.now();
  const result = {phase, raf:0, widths:[], intersections:[], settleFrames:0};
  let stage = 0;
  const ro = new ResizeObserver(entries => {
    for (const entry of entries) {
      const width = entry.contentRect.width;
      if (stage === 0 && width === 40) {
        result.widths.push(width);
        stage = 1;
        probe.style.top = '0px';
      } else if (stage === 2 && width === finalWidth) {
        result.widths.push(width);
        stage = 3;
        probe.style.top = '1000px';
      }
    }
  });
  const io = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (stage === 1 && entry.isIntersecting) {
        result.intersections.push(true);
        stage = 2;
        requestAnimationFrame(() => {
          result.raf++;
          target.style.width = finalWidth + 'px';
          target.style.backgroundColor = finalColor;
        });
      } else if (stage === 3 && !entry.isIntersecting) {
        result.intersections.push(false);
        stage = 4;
        ro.disconnect();
        io.disconnect();
        // A finite rAF-driven observation window crosses several configured
        // cast intervals. It never measures geometry or requests pixels.
        const finish = () => {
          result.settleFrames++;
          result.elapsedMs = performance.now() - started;
          if (result.elapsedMs < minimumElapsed) {
            requestAnimationFrame(finish);
          } else {
            window.__castLifecycleResult = result;
            __moliScreencastLifecycleDone(JSON.stringify(result));
          }
        };
        requestAnimationFrame(finish);
      }
    }
  });
  window.__castLifecycleObservers = {ro, io};
  ro.observe(target);
  io.observe(probe);
  requestAnimationFrame(() => {
    result.raf++;
    target.style.width = '40px';
    target.style.backgroundColor = 'green';
  });
  return 'scheduled';
};
</script>"#;

async fn spawn_realtime_cast_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = NavigationRuntimeConfig::new(
        protocol_server_test_fetch_config(FetchConfig::default()),
        OptionalResourceFetchMask::NONE,
        true,
        LayoutPolicy::Realtime,
    )
    .with_scrollbars_hidden(true);
    let storage = Arc::new(StoragePartitionState::open(None).unwrap());
    let state = AppState::new_with_storage_partition_and_runtime_config(
        addr,
        storage,
        config,
        CAST_INTERVAL_MS,
        BrowserInstanceControl::new(),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let service = axum::ServiceExt::<Request<Body>>::into_make_service(build_router(state));
        axum::serve(listener, service).await.unwrap();
    });
    (addr, server)
}

fn is_cast_frame(message: &serde_json::Value, session: &str) -> bool {
    message["sessionId"] == session && message["method"] == "Page.screencastFrame"
}

async fn receive_cast(
    socket: &mut CastSocket,
    session: &str,
    mut messages: Vec<serde_json::Value>,
) -> (serde_json::Value, Vec<serde_json::Value>) {
    if !messages
        .iter()
        .any(|message| is_cast_frame(message, session))
    {
        messages.extend(recv_until_match(socket, |message| is_cast_frame(message, session)).await);
    }
    let frames = messages
        .iter()
        .filter(|message| is_cast_frame(message, session))
        .collect::<Vec<_>>();
    assert_eq!(
        frames.len(),
        1,
        "only one unacknowledged frame may be emitted: {messages:#?}"
    );
    ((*frames[0]).clone(), messages)
}

async fn run_lifecycle_without_followup_commands(
    socket: &mut CastSocket,
    session: &str,
    command_id: u64,
    phase: &str,
    final_width: u32,
    final_color: &str,
) -> Vec<serde_json::Value> {
    let expression = format!(
        "runCastLifecycle({}, {}, {}, {})",
        serde_json::to_string(phase).unwrap(),
        final_width,
        serde_json::to_string(final_color).unwrap(),
        CAST_INTERVAL_MS * 3,
    );
    send_cdp_command_without_wait(
        socket,
        command_id,
        "Runtime.evaluate",
        Some(session),
        json!({
            "expression": expression, "returnByValue": true,
        }),
    )
    .await;
    let mut response_seen = false;
    let mut terminal = None;
    // This is the only wait: the renderer/normal protocol actor must publish
    // the callback's binding. No polling evaluation, geometry read, capture,
    // screencast ACK or task-driving helper occurs before the terminal.
    let messages = recv_until_match(socket, |message| {
        response_seen |= message["id"] == command_id;
        if message["sessionId"] == session
            && message["method"] == "Runtime.bindingCalled"
            && message["params"]["name"] == CAST_BINDING
        {
            let payload: serde_json::Value = serde_json::from_str(
                message["params"]["payload"]
                    .as_str()
                    .expect("binding payload"),
            )
            .unwrap();
            if payload["phase"] == phase {
                terminal = Some(payload);
            }
        }
        response_seen && terminal.is_some()
    })
    .await;
    assert_cdp_success(&messages, command_id);
    let response = response_by_id(&messages, command_id);
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response:#?}"
    );
    assert_eq!(response["result"]["result"]["value"], "scheduled");
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == "Runtime.exceptionThrown"),
        "{messages:#?}"
    );
    let terminal = terminal.unwrap();
    assert_eq!(terminal["raf"], 2);
    assert_eq!(terminal["widths"], json!([40, final_width]));
    assert_eq!(terminal["intersections"], json!([true, false]));
    assert!(
        terminal["settleFrames"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        terminal["elapsedMs"]
            .as_f64()
            .is_some_and(|ms| ms >= f64::from(CAST_INTERVAL_MS * 3))
    );
    messages
}

fn cast_pixels(frame: &serde_json::Value, inside_x: u32, outside_x: u32) -> ([u8; 4], [u8; 4]) {
    let bytes = BASE64_STANDARD
        .decode(frame["params"]["data"].as_str().unwrap())
        .unwrap();
    let (width, height, pixels) = decode_png(&bytes);
    assert_eq!((width, height), (64, 64));
    (
        pixel(&pixels, width, inside_x, 10),
        pixel(&pixels, width, outside_x, 10),
    )
}

fn assert_cast_pixels(frame: &serde_json::Value, color: [u8; 4], inside_x: u32, outside_x: u32) {
    assert_eq!(cast_pixels(frame, inside_x, outside_x), (color, [255; 4]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_realtime_screencast_ack_backpressure_does_not_stop_page_lifecycle() {
    let (addr, server) = spawn_realtime_cast_server().await;
    let (mut socket, _) = connect_async(
        discover_browser_websocket_url(addr)
            .await
            .expect("browser discovery"),
    )
    .await
    .unwrap();
    let url = format!(
        "data:text/html,{}",
        percent_encoding::utf8_percent_encode(
            REALTIME_CAST_HTML,
            percent_encoding::NON_ALPHANUMERIC,
        )
    );
    let target = open_screenshot_target(&mut socket, &url).await;
    let session = target.session_id.as_str();
    for (id, method, params) in [
        (
            7,
            "Emulation.setDeviceMetricsOverride",
            json!({"width":64,"height":64,"deviceScaleFactor":1,"mobile":false}),
        ),
        (8, "Runtime.enable", json!({})),
        (9, "Runtime.addBinding", json!({"name":CAST_BINDING})),
    ] {
        assert_cdp_success(
            &send_cdp_command(&mut socket, id, method, Some(session), params).await,
            id,
        );
    }
    let started = send_cdp_command(
        &mut socket,
        10,
        "Page.startScreencast",
        Some(session),
        json!({
            "format":"png", "maxWidth":64, "maxHeight":64, "everyNthFrame":1,
        }),
    )
    .await;
    assert_cdp_success(&started, 10);
    let (first, _) = receive_cast(&mut socket, session, started).await;
    assert_cast_pixels(&first, [255, 0, 0, 255], 10, 30);
    let generation = first["params"]["sessionId"].as_i64().unwrap();
    assert!(generation > 0);

    // Receiving this first frame without sending ACK establishes AwaitingAck.
    // Both rAF callbacks, both RO size observations and both IO transitions
    // must now finish on normal owner work before any further pixel request.
    let blocked = run_lifecycle_without_followup_commands(
        &mut socket,
        session,
        11,
        "awaiting-ack",
        60,
        "blue",
    )
    .await;
    assert!(
        !blocked
            .iter()
            .any(|message| is_cast_frame(message, session)),
        "{blocked:#?}"
    );

    let acknowledged = send_cdp_command(
        &mut socket,
        12,
        "Page.screencastFrameAck",
        Some(session),
        json!({
            "sessionId": generation,
        }),
    )
    .await;
    assert_cdp_success(&acknowledged, 12);
    let (current, _) = receive_cast(&mut socket, session, acknowledged).await;
    assert_eq!(current["params"]["sessionId"], generation);
    // x=50 is outside the intermediate width40 and inside the final width60.
    assert_cast_pixels(&current, [0, 0, 255, 255], 50, 62);

    let stopped = send_cdp_command(
        &mut socket,
        13,
        "Page.stopScreencast",
        Some(session),
        json!({}),
    )
    .await;
    assert_cdp_success(&stopped, 13);
    assert!(
        !stopped
            .iter()
            .any(|message| is_cast_frame(message, session)),
        "{stopped:#?}"
    );
    // A late ACK must not resurrect the stopped generation. Without it a
    // broken stop could be masked by the second frame's still-pending ACK.
    let late_ack = send_cdp_command(
        &mut socket,
        14,
        "Page.screencastFrameAck",
        Some(session),
        json!({
            "sessionId": generation,
        }),
    )
    .await;
    assert_cdp_success(&late_ack, 14);
    assert!(
        !late_ack
            .iter()
            .any(|message| is_cast_frame(message, session)),
        "{late_ack:#?}"
    );
    let after_stop = run_lifecycle_without_followup_commands(
        &mut socket,
        session,
        15,
        "after-stop",
        24,
        "purple",
    )
    .await;
    assert!(
        !after_stop
            .iter()
            .any(|message| is_cast_frame(message, session)),
        "{after_stop:#?}"
    );

    let closed = send_cdp_command(
        &mut socket,
        16,
        "Target.closeTarget",
        None,
        json!({
            "targetId": target.target_id,
        }),
    )
    .await;
    assert_cdp_success(&closed, 16);
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_realtime_screencast_updates_for_overlay_only_changes() {
    let (addr, server) = spawn_realtime_cast_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let target = open_screenshot_target(
        &mut socket,
        "data:text/html,%3C!doctype%20html%3E%3Cbody%3E",
    )
    .await;
    let session = target.session_id.as_str();
    for (id, method, params) in [
        (
            7,
            "Emulation.setDeviceMetricsOverride",
            json!({"width":64,"height":64,"deviceScaleFactor":1,"mobile":false}),
        ),
        (8, "DOM.enable", json!({})),
        (9, "Overlay.enable", json!({})),
    ] {
        assert_cdp_success(
            &send_cdp_command(&mut socket, id, method, Some(session), params).await,
            id,
        );
    }
    let started = send_cdp_command(
        &mut socket,
        10,
        "Page.startScreencast",
        Some(session),
        json!({"format":"png","maxWidth":64,"maxHeight":64,"everyNthFrame":1}),
    )
    .await;
    assert_cdp_success(&started, 10);
    let (mut frame, _) = receive_cast(&mut socket, session, started).await;
    assert_cast_pixels(&frame, [255; 4], 10, 30);

    for (id, method, params, color) in [
        (
            12,
            "Overlay.highlightRect",
            json!({"x":2,"y":2,"width":16,"height":16,"color":{"r":255,"g":0,"b":0}}),
            [255, 0, 0, 255],
        ),
        (14, "Overlay.hideHighlight", json!({}), [255; 4]),
    ] {
        let ack = send_cdp_command(
            &mut socket,
            id - 1,
            "Page.screencastFrameAck",
            Some(session),
            json!({"sessionId":frame["params"]["sessionId"]}),
        )
        .await;
        assert_cdp_success(&ack, id - 1);
        let changed = send_cdp_command(&mut socket, id, method, Some(session), params).await;
        assert_cdp_success(&changed, id);
        (frame, _) = receive_cast(&mut socket, session, changed).await;
        assert_cast_pixels(&frame, color, 10, 30);
    }

    assert_cdp_success(
        &send_cdp_command(
            &mut socket,
            15,
            "Page.stopScreencast",
            Some(session),
            json!({}),
        )
        .await,
        15,
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_screencast_discovers_css_image_after_debugger_resume() {
    const WHITE: [u8; 4] = [255; 4];
    const RED: [u8; 4] = [255, 0, 0, 255];
    let mut red_image = Vec::new();
    let mut encoder = png::Encoder::new(&mut red_image, 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&RED)
        .unwrap();

    let (image_requests, mut requested) = tokio::sync::mpsc::unbounded_channel();
    let fixture = Router::new()
        .route(
            "/",
            get(|| async {
                axum::response::Html(
                    "<!doctype html><style>body{margin:0;background:white} #target{width:20px;height:20px}</style><div id=target></div>",
                )
            }),
        )
        .route(
            "/red.png",
            get(move || {
                let image_requests = image_requests.clone();
                let red_image = red_image.clone();
                async move {
                    let _ = image_requests.send(());
                    ([(header::CONTENT_TYPE, "image/png")], red_image)
                }
            }),
        );
    let (fixture_addr, _fixture_server) = spawn_dedicated_fixture_server(fixture, "paused-cast");
    let (addr, server) = spawn_test_protocol_server_with_image_fetch_enabled(true).await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let target = open_screenshot_target(&mut socket, &format!("http://{fixture_addr}/")).await;
    let session = target.session_id.as_str();
    for (id, method, params) in [
        (
            7,
            "Emulation.setDeviceMetricsOverride",
            json!({"width":64,"height":64,"deviceScaleFactor":1,"mobile":false}),
        ),
        (8, "Debugger.enable", json!({})),
    ] {
        assert_cdp_success(
            &send_cdp_command(&mut socket, id, method, Some(session), params).await,
            id,
        );
    }
    let started = send_cdp_command(
        &mut socket,
        9,
        "Page.startScreencast",
        Some(session),
        json!({"format":"png","maxWidth":64,"maxHeight":64,"everyNthFrame":1}),
    )
    .await;
    assert_cdp_success(&started, 9);
    let (frame, _) = receive_cast(&mut socket, session, started).await;
    assert_cast_pixels(&frame, WHITE, 10, 30);
    let ack = send_cdp_command(
        &mut socket,
        10,
        "Page.screencastFrameAck",
        Some(session),
        json!({"sessionId":frame["params"]["sessionId"]}),
    )
    .await;
    assert_cdp_success(&ack, 10);

    send_cdp_command_without_wait(
        &mut socket,
        11,
        "Runtime.evaluate",
        Some(session),
        json!({"expression":"document.getElementById('target').style.backgroundImage = 'url(/red.png)'; debugger;"}),
    )
    .await;
    let paused = recv_until_match(&mut socket, |message| {
        message["sessionId"] == session && message["method"] == "Debugger.paused"
    })
    .await;
    let (frame, paused) = receive_cast(&mut socket, session, paused).await;
    assert_cast_pixels(&frame, WHITE, 10, 30);
    assert!(!paused.iter().any(|message| message["id"] == 11));
    let mut messages = send_cdp_command(
        &mut socket,
        12,
        "Page.screencastFrameAck",
        Some(session),
        json!({"sessionId":frame["params"]["sessionId"]}),
    )
    .await;
    assert_cdp_success(&messages, 12);
    assert!(
        matches!(
            requested.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ),
        "paused screencast must defer the image request"
    );

    let resumed =
        send_cdp_command(&mut socket, 13, "Debugger.resume", Some(session), json!({})).await;
    assert_cdp_success(&resumed, 13);
    messages.extend(resumed);
    // Keep the page static. The scheduled poll must get past the visual-state
    // check and discover the image without an explicit capture.
    timeout(Duration::from_secs(10), requested.recv())
        .await
        .expect("screencast should request the CSS image after resuming")
        .expect("image fixture is still running");
    timeout(Duration::from_secs(10), async {
        let mut ack_id = 14;
        loop {
            let (frame, _) = receive_cast(&mut socket, session, messages).await;
            let (inside, outside) = cast_pixels(&frame, 10, 30);
            assert_eq!(outside, WHITE);
            if inside == RED {
                break;
            }
            assert_eq!(inside, WHITE);
            messages = send_cdp_command(
                &mut socket,
                ack_id,
                "Page.screencastFrameAck",
                Some(session),
                json!({"sessionId":frame["params"]["sessionId"]}),
            )
            .await;
            assert_cdp_success(&messages, ack_id);
            ack_id += 1;
        }
    })
    .await
    .expect("screencast should paint the CSS image");

    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_realtime_mobile_meta_resamples_existing_media_queries_and_resize() {
    let (addr, server) = spawn_realtime_cast_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let target = open_screenshot_target(
        &mut socket,
        "data:text/html,%3C!doctype%20html%3E%3Cbody%3E",
    )
    .await;
    let session = target.session_id.as_str();
    let applied = send_cdp_command(
        &mut socket,
        10,
        "Emulation.setDeviceMetricsOverride",
        Some(session),
        json!({"width":375,"height":667,"deviceScaleFactor":1,"mobile":true}),
    )
    .await;
    assert_cdp_success(&applied, 10);
    let initialize = send_cdp_command(
        &mut socket, 11, "Runtime.evaluate", Some(session),
        json!({"expression":r#"
            new Promise(resolve => requestAnimationFrame(() => {
                window.query = matchMedia('(max-width: 500px)');
                window.notifications = [];
                addEventListener('resize', () => notifications.push('window'));
                visualViewport.addEventListener('resize', () => notifications.push('visual'));
                query.addEventListener('change', event => notifications.push('matches:' + event.matches));
                resolve([innerWidth, query.matches]);
            }))
        "#,"awaitPromise":true,"returnByValue":true}),
    ).await;
    assert_eq!(
        response_by_id(&initialize, 11)["result"]["result"]["value"],
        json!([981, false])
    );

    for (id, update, geometry, events) in [
        (
            12,
            r#"document.head.insertAdjacentHTML('beforeend', '<meta name=viewport content="width=device-width, initial-scale=1">')"#,
            json!([375, 375, true, true]),
            json!(["window", "visual", "matches:true"]),
        ),
        (
            14,
            r#"document.querySelector('meta[name=viewport]').content='width=device-width, initial-scale=2'"#,
            json!([375, 187.5, true, true]),
            json!(["visual"]),
        ),
    ] {
        let changed = send_cdp_command(
            &mut socket, id, "Runtime.evaluate", Some(session),
            json!({"expression":format!("notifications.length = 0; {update}; [innerWidth, visualViewport.width, query.matches, matchMedia(query.media).matches]"),"returnByValue":true}),
        ).await;
        assert_eq!(
            response_by_id(&changed, id)["result"]["result"]["value"],
            geometry
        );
        let observed = send_cdp_command(
            &mut socket, id + 1, "Runtime.evaluate", Some(session),
            json!({
                "expression":"new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(notifications))))",
                "awaitPromise":true, "returnByValue":true,
            }),
        ).await;
        assert_eq!(
            response_by_id(&observed, id + 1)["result"]["result"]["value"],
            events
        );
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
