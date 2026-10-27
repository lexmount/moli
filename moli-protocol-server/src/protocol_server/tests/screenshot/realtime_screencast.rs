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

fn assert_cast_pixels(frame: &serde_json::Value, color: [u8; 4], inside_x: u32, outside_x: u32) {
    let bytes = BASE64_STANDARD
        .decode(frame["params"]["data"].as_str().unwrap())
        .unwrap();
    let (width, height, pixels) = decode_png(&bytes);
    assert_eq!((width, height), (64, 64));
    assert_eq!(pixel(&pixels, width, inside_x, 10), color);
    assert_eq!(pixel(&pixels, width, outside_x, 10), [255, 255, 255, 255]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_realtime_screencast_ack_backpressure_does_not_stop_page_lifecycle() {
    let (addr, server) = spawn_realtime_cast_server().await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
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
