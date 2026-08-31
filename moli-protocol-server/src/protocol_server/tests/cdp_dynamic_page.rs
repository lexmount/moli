use super::*;

type TestCdpSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn create_dynamic_target(browser: &mut TestCdpSocket, command_id: u64) -> String {
    send_cdp_command(
        browser,
        command_id,
        "Target.createTarget",
        None,
        json!({ "url": "about:blank" }),
    )
    .await
    .iter()
    .find(|message| message["id"] == json!(command_id))
    .and_then(|message| message["result"]["targetId"].as_str())
    .expect("dynamic target id")
    .to_owned()
}

async fn connect_dynamic_page(addr: std::net::SocketAddr, target_id: &str) -> TestCdpSocket {
    connect_async(format!("ws://{addr}/devtools/page/{target_id}"))
        .await
        .expect("dynamic page websocket should connect")
        .0
}

async fn discovered_tab_websocket_url(addr: std::net::SocketAddr, tab_target_id: &str) -> String {
    let (status, body) = fetch_server_response(addr, "GET", "/json/list?for_tab").await;
    assert_eq!(status, 200);
    let targets: Vec<serde_json::Value> =
        serde_json::from_slice(&body).expect("parse tab target discovery response");
    let target = targets
        .iter()
        .find(|target| target["id"] == json!(tab_target_id))
        .expect("discovered tab target");
    assert_eq!(target["type"], json!("tab"));
    assert_eq!(
        target["devtoolsFrontendUrl"],
        json!(format!(
            "/devtools/inspector.html?ws={addr}/devtools/page/{tab_target_id}"
        )),
        "remote discovery uses the same inspector URL shape for Page and Tab hosts"
    );
    let websocket_url = target["webSocketDebuggerUrl"]
        .as_str()
        .expect("tab websocket URL");
    assert!(websocket_url.contains("/devtools/page/"));
    websocket_url.to_owned()
}

async fn connect_discovered_tab(addr: std::net::SocketAddr, tab_target_id: &str) -> TestCdpSocket {
    let websocket_url = discovered_tab_websocket_url(addr, tab_target_id).await;
    connect_async(websocket_url)
        .await
        .expect("tab websocket should connect")
        .0
}

async fn enable_tab_page(
    socket: &mut TestCdpSocket,
    command_id: u64,
    page_target_id: &str,
) -> String {
    send_cdp_command_without_wait(
        socket,
        command_id,
        "Target.setAutoAttach",
        None,
        json!({
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }),
    )
    .await;
    let mut saw_response = false;
    let mut attached_session_id = None;
    let messages = recv_until_match(socket, |message| {
        saw_response |= message["id"] == json!(command_id);
        if message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
        {
            attached_session_id = message["params"]["sessionId"].as_str().map(str::to_owned);
        }
        saw_response && attached_session_id.is_some()
    })
    .await;
    assert_eq!(response_by_id(&messages, command_id)["result"], json!({}));
    attached_session_id.expect("tab child page session")
}

async fn wait_for_websocket_close(socket: &mut TestCdpSocket, label: &str) {
    let wait_for_close = async {
        loop {
            match socket.next().await {
                None | Some(Ok(WsMessage::Close(_))) => break,
                Some(Ok(_)) => continue,
                Some(Err(error)) => panic!("{label} websocket close failed: {error}"),
            }
        }
    };
    timeout(Duration::from_secs(5), wait_for_close)
        .await
        .expect("websocket should close");
}

fn response_by_id(messages: &[serde_json::Value], id: u64) -> &serde_json::Value {
    messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing response id {id}: {messages:#?}"))
}

fn is_screencast_visibility(message: &serde_json::Value, visible: bool) -> bool {
    message["method"] == json!("Page.screencastVisibilityChanged")
        && message["params"]["visible"] == json!(visible)
}

fn is_target_created_for_url(message: &serde_json::Value, url: &str) -> bool {
    message["method"] == json!("Target.targetCreated")
        && message["params"]["targetInfo"]["url"] == json!(url)
}

async fn expect_screencast_visibility(socket: &mut TestCdpSocket, visible: bool) {
    let messages =
        recv_until_match(socket, |message| is_screencast_visibility(message, visible)).await;
    assert!(
        messages
            .iter()
            .any(|message| is_screencast_visibility(message, visible))
    );
}

async fn enable_runtime_and_expect_default_context(
    socket: &mut TestCdpSocket,
    command_id: u64,
    session_id: Option<&str>,
    label: &str,
) -> Vec<serde_json::Value> {
    let mut messages =
        send_cdp_command(socket, command_id, "Runtime.enable", session_id, json!({})).await;
    assert_eq!(response_by_id(&messages, command_id)["result"], json!({}));
    if !messages.iter().any(|message| {
        message.get("sessionId").and_then(serde_json::Value::as_str) == session_id
            && message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
    }) {
        messages.extend(
            send_cdp_command(
                socket,
                command_id + 1,
                "Runtime.evaluate",
                session_id,
                json!({ "expression": "void 0" }),
            )
            .await,
        );
    }
    assert!(
        messages.iter().any(|message| {
            message.get("sessionId").and_then(serde_json::Value::as_str) == session_id
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "{label} did not report the existing default context before the next Runtime response: {messages:#?}"
    );
    messages
}

async fn puppeteer_auto_attach_existing_page(
    browser: &mut TestCdpSocket,
    command_id: u64,
    page_target_id: &str,
    tab_target_id: &str,
) -> String {
    send_cdp_command_without_wait(
        browser,
        command_id,
        "Target.setAutoAttach",
        None,
        json!({
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true,
            "filter": [
                { "type": "page", "exclude": true },
                {}
            ]
        }),
    )
    .await;
    let mut saw_root_response = false;
    let mut saw_tab_attach = false;
    let root_auto_attach = recv_until_match(browser, |message| {
        saw_root_response |= message["id"] == json!(command_id);
        saw_tab_attach |= message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id);
        saw_root_response && saw_tab_attach
    })
    .await;
    assert_eq!(
        response_by_id(&root_auto_attach, command_id)["result"],
        json!({})
    );
    let tab_session_id = root_auto_attach
        .iter()
        .find(|message| {
            message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
        })
        .and_then(|message| message["params"]["sessionId"].as_str())
        .expect("auto-attached tab session")
        .to_owned();

    let child_command_id = command_id + 1;
    send_cdp_command_without_wait(
        browser,
        child_command_id,
        "Target.setAutoAttach",
        Some(&tab_session_id),
        json!({
            "autoAttach": true,
            "waitForDebuggerOnStart": false,
            "flatten": true,
            "filter": [{}]
        }),
    )
    .await;
    let mut saw_child_response = false;
    let mut saw_page_attach = false;
    let tab_auto_attach = recv_until_match(browser, |message| {
        saw_child_response |= message["id"] == json!(child_command_id);
        saw_page_attach |= message["sessionId"] == json!(tab_session_id)
            && message["method"] == json!("Target.attachedToTarget")
            && message["params"]["targetInfo"]["targetId"] == json!(page_target_id);
        saw_child_response && saw_page_attach
    })
    .await;
    assert_eq!(
        response_by_id(&tab_auto_attach, child_command_id)["result"],
        json!({})
    );
    tab_auto_attach
        .iter()
        .find(|message| {
            message["sessionId"] == json!(tab_session_id)
                && message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
        })
        .and_then(|message| message["params"]["sessionId"].as_str())
        .expect("auto-attached page session")
        .to_owned()
}

async fn fetch_server_json(addr: std::net::SocketAddr, path: &str) -> serde_json::Value {
    let (status, body) = fetch_server_response(addr, "GET", path).await;
    assert_eq!(status, 200, "unexpected HTTP status for {path}");
    serde_json::from_slice(&body).expect("protocol server JSON response")
}

async fn fetch_server_response(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
) -> (u16, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect protocol server HTTP route");
    stream
        .write_all(
            format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("write protocol server HTTP request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read protocol server HTTP response");
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response header terminator");
    let headers = std::str::from_utf8(&response[..header_end]).expect("HTTP response headers");
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .expect("HTTP response status");
    (status, response[header_end + 4..].to_vec())
}

async fn dispatch_primary_click(
    socket: &mut TestCdpSocket,
    command_id: u64,
    session_id: Option<&str>,
    x: u32,
    y: u32,
) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();
    for (offset, event_type) in [(0, "mousePressed"), (1, "mouseReleased")] {
        let response = send_cdp_command(
            socket,
            command_id + offset,
            "Input.dispatchMouseEvent",
            session_id,
            json!({
                "type": event_type,
                "x": x,
                "y": y,
                "button": "left",
                "buttons": if event_type == "mousePressed" { 1 } else { 0 },
                "clickCount": 1
            }),
        )
        .await;
        assert!(
            response_by_id(&response, command_id + offset)
                .get("result")
                .is_some()
        );
        messages.extend(response);
    }
    messages
}

async fn evaluate_page_surface(socket: &mut TestCdpSocket, command_id: u64) -> serde_json::Value {
    let response = send_cdp_command(
        socket,
        command_id,
        "Runtime.evaluate",
        None,
        json!({
            "expression": "({ href: location.href, hidden: document.hidden, visibility: document.visibilityState, focused: document.hasFocus() })",
            "returnByValue": true
        }),
    )
    .await;
    response_by_id(&response, command_id)["result"]["result"]["value"].clone()
}

fn assert_page_surface_active(surface: &serde_json::Value, expected_active: bool) {
    assert_eq!(surface["hidden"], json!(!expected_active));
    assert_eq!(
        surface["visibility"],
        json!(if expected_active { "visible" } else { "hidden" })
    );
    assert_eq!(surface["focused"], json!(expected_active));
}

async fn wait_for_target_list(
    addr: std::net::SocketAddr,
    label: &str,
    predicate: impl Fn(&[serde_json::Value]) -> bool,
) -> Vec<serde_json::Value> {
    timeout(Duration::from_secs(5), async {
        loop {
            let response = fetch_server_json(addr, "/json/list").await;
            if let Some(targets) = response.as_array()
                && predicate(targets)
            {
                return targets.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_mouse_wheel_scrolls_page_and_honors_prevent_default() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;

    let installed = send_cdp_command(
        &mut page,
        1,
        "Runtime.evaluate",
        None,
        json!({
            "expression": r#"
                (() => {
                  document.documentElement.style.margin = "0";
                  document.body.style.margin = "0";
                  document.body.innerHTML =
                    '<div style="height: 500px"></div>' +
                    '<div id="marker" style="height: 20px"></div>' +
                    '<div style="height: 2500px"></div>';
                  window.__wheelDeltas = [];
                  window.addEventListener("wheel", event => {
                    window.__wheelDeltas.push(event.deltaY);
                  }, { capture: true });
                  return document.getElementById("marker").getBoundingClientRect().top;
                })()
            "#,
            "returnByValue": true
        }),
    )
    .await;
    let marker_before = response_by_id(&installed, 1)["result"]["result"]["value"]
        .as_f64()
        .expect("initial marker top");

    let wheel = send_cdp_command(
        &mut page,
        2,
        "Input.dispatchMouseEvent",
        None,
        json!({
            "type": "mouseWheel",
            "x": 10,
            "y": 10,
            "deltaX": 0,
            "deltaY": 120
        }),
    )
    .await;
    assert!(response_by_id(&wheel, 2).get("result").is_some());

    let scrolled = send_cdp_command(
        &mut page,
        3,
        "Runtime.evaluate",
        None,
        json!({
            "expression": r#"
                ({
                  scrollY,
                  markerTop: document.getElementById("marker").getBoundingClientRect().top,
                  wheelDeltas: window.__wheelDeltas
                })
            "#,
            "returnByValue": true
        }),
    )
    .await;
    let value = &response_by_id(&scrolled, 3)["result"]["result"]["value"];
    assert_eq!(value["scrollY"], json!(120));
    assert_eq!(
        value["markerTop"].as_f64().expect("scrolled marker top"),
        marker_before - 120.0
    );
    assert_eq!(value["wheelDeltas"], json!([120]));

    let cancel = send_cdp_command(
        &mut page,
        4,
        "Runtime.evaluate",
        None,
        json!({
            "expression": r#"
                window.addEventListener("wheel", event => event.preventDefault(), {
                  capture: true,
                  passive: false
                })
            "#
        }),
    )
    .await;
    assert!(response_by_id(&cancel, 4).get("result").is_some());
    let canceled_wheel = send_cdp_command(
        &mut page,
        5,
        "Input.dispatchMouseEvent",
        None,
        json!({
            "type": "mouseWheel",
            "x": 10,
            "y": 10,
            "deltaX": 0,
            "deltaY": 80
        }),
    )
    .await;
    assert!(response_by_id(&canceled_wheel, 5).get("result").is_some());
    let final_scroll = send_cdp_command(
        &mut page,
        6,
        "Runtime.evaluate",
        None,
        json!({ "expression": "scrollY", "returnByValue": true }),
    )
    .await;
    assert_eq!(
        response_by_id(&final_scroll, 6)["result"]["result"]["value"],
        json!(120)
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_routes_to_existing_target_owner() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let target_list = fetch_server_json(addr, "/json/list").await;
    let listed_target = target_list
        .as_array()
        .expect("target list array")
        .iter()
        .find(|target| target["id"] == json!(target_id))
        .expect("dynamic target should be listed");
    assert_eq!(listed_target["url"], json!("about:blank"));
    assert_eq!(
        listed_target["webSocketDebuggerUrl"],
        json!(format!("ws://{addr}/devtools/page/{target_id}"))
    );
    assert_eq!(
        listed_target["devtoolsFrontendUrl"],
        json!(format!(
            "/devtools/inspector.html?ws={addr}/devtools/page/{target_id}"
        ))
    );
    let mut page = connect_dynamic_page(addr, &target_id).await;

    page.send(WsMessage::Text("{".into()))
        .await
        .expect("send malformed direct page command");
    let parse_error = timeout(Duration::from_secs(5), recv_ws_json(&mut page))
        .await
        .expect("direct page should receive its parse error");
    assert_eq!(parse_error["id"], serde_json::Value::Null);
    assert_eq!(parse_error["error"]["code"], json!(-32700));
    if let Ok(message) =
        tokio::time::timeout(Duration::from_millis(100), recv_ws_json(&mut browser)).await
    {
        panic!("browser frontend received direct-page parse output: {message:#?}");
    }

    let page_owner_info =
        send_cdp_command(&mut page, 7000, "Target.getTargetInfo", None, json!({})).await;
    assert_eq!(
        response_by_id(&page_owner_info, 7000)["result"]["targetInfo"]["targetId"],
        json!(target_id)
    );
    assert_eq!(
        response_by_id(&page_owner_info, 7000)["result"]["targetInfo"]["type"],
        json!("page")
    );

    let frame_tree = send_cdp_command(&mut page, 1, "Page.getFrameTree", None, json!({})).await;
    let frame_tree_response = response_by_id(&frame_tree, 1);
    assert_eq!(
        frame_tree_response["result"]["frameTree"]["frame"]["id"],
        json!(target_id)
    );
    assert!(
        frame_tree_response.get("sessionId").is_none(),
        "direct page response leaked its private flattened session: {frame_tree_response:#?}"
    );

    let page_set = send_cdp_command(
        &mut page,
        2,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_page_bridge = 'from-page'" }),
    )
    .await;
    assert_eq!(
        response_by_id(&page_set, 2)["result"]["result"]["value"],
        json!("from-page")
    );

    let browser_probe =
        send_cdp_command(&mut browser, 2, "Browser.getVersion", None, json!({})).await;
    assert!(
        browser_probe
            .iter()
            .all(|message| message["method"] != json!("Target.attachedToTarget")),
        "private page frontend attach leaked to browser frontend: {browser_probe:#?}"
    );
    let browser_page_command =
        send_cdp_command(&mut browser, 3, "Page.getFrameTree", None, json!({})).await;
    assert_eq!(
        response_by_id(&browser_page_command, 3)["error"],
        json!({
            "code": -32601,
            "message": "'Page.getFrameTree' wasn't found"
        }),
        "the Browser AgentHost must not proxy Page commands to the active target"
    );

    let attach = send_cdp_command(
        &mut browser,
        3,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id }),
    )
    .await;
    let browser_session_id = response_by_id(&attach, 3)["result"]["sessionId"]
        .as_str()
        .expect("browser target session")
        .to_owned();
    let browser_read = send_cdp_command(
        &mut browser,
        4,
        "Runtime.evaluate",
        Some(&browser_session_id),
        json!({ "expression": "globalThis.__moli_page_bridge" }),
    )
    .await;
    assert_eq!(
        response_by_id(&browser_read, 4)["result"]["result"]["value"],
        json!("from-page"),
        "browser and direct page frontends did not observe the same runtime"
    );

    browser
        .send(WsMessage::Text(
            json!({
                "id": 77_u64,
                "method": "Runtime.evaluate",
                "sessionId": browser_session_id,
                "params": { "expression": "'browser-response'" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browser command with colliding id");
    page.send(WsMessage::Text(
        json!({
            "id": 77_u64,
            "method": "Runtime.evaluate",
            "params": { "expression": "'page-response'" }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send page command with colliding id");
    let browser_collision = recv_until_id(&mut browser, 77).await;
    let page_collision = recv_until_id(&mut page, 77).await;
    assert_eq!(
        response_by_id(&browser_collision, 77)["result"]["result"]["value"],
        json!("browser-response")
    );
    assert_eq!(
        response_by_id(&page_collision, 77)["result"]["result"]["value"],
        json!("page-response")
    );
    assert!(
        response_by_id(&page_collision, 77)
            .get("sessionId")
            .is_none()
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_create_for_tab_exposes_distinct_chromium_agent_hosts() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");

    let discover = send_cdp_command(
        &mut browser,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({ "discover": true, "filter": [{}] }),
    )
    .await;
    assert_eq!(response_by_id(&discover, 1)["result"], json!({}));

    send_cdp_command_without_wait(
        &mut browser,
        2,
        "Target.createTarget",
        None,
        json!({ "url": "about:blank", "forTab": true }),
    )
    .await;
    let mut saw_response = false;
    let mut page_target_id = None;
    let mut tab_target_id = None;
    let created = recv_until_match(&mut browser, |message| {
        saw_response |= message["id"] == json!(2_u64);
        if message["method"] == json!("Target.targetCreated") {
            let info = &message["params"]["targetInfo"];
            let target_id = info["targetId"].as_str().map(str::to_owned);
            match info["type"].as_str() {
                Some("page") if target_id.as_deref() != Some(DEFAULT_TARGET_ID) => {
                    page_target_id = target_id;
                }
                Some("tab") if target_id.as_deref() != Some(DEFAULT_TAB_TARGET_ID) => {
                    tab_target_id = target_id;
                }
                _ => {}
            }
        }
        saw_response && page_target_id.is_some() && tab_target_id.is_some()
    })
    .await;
    let page_target_id = page_target_id.expect("created Page target id");
    let tab_target_id = tab_target_id.expect("created Tab target id");
    assert_ne!(page_target_id, tab_target_id);
    assert_eq!(
        response_by_id(&created, 2)["result"]["targetId"],
        json!(tab_target_id),
        "forTab=true must return the stable Tab AgentHost"
    );
    let tab_created = created
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] == json!(tab_target_id)
        })
        .expect("Tab targetCreated");
    let page_created = created
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] == json!(page_target_id)
        })
        .expect("Page targetCreated");
    assert!(
        tab_created < page_created,
        "Chromium publishes the WebContents Tab host before its primary Page host: {created:#?}"
    );

    let mut tab = connect_discovered_tab(addr, &tab_target_id).await;
    let tab_info = send_cdp_command(&mut tab, 1, "Target.getTargetInfo", None, json!({})).await;
    assert_eq!(
        response_by_id(&tab_info, 1)["result"]["targetInfo"]["targetId"],
        json!(tab_target_id)
    );
    assert_eq!(
        response_by_id(&tab_info, 1)["result"]["targetInfo"]["type"],
        json!("tab")
    );
    let tab_page_session = enable_tab_page(&mut tab, 2, &page_target_id).await;
    let child_info = send_cdp_command(
        &mut tab,
        4,
        "Target.getTargetInfo",
        Some(&tab_page_session),
        json!({}),
    )
    .await;
    assert_eq!(
        response_by_id(&child_info, 4)["result"]["targetInfo"]["targetId"],
        json!(page_target_id)
    );
    assert_eq!(
        response_by_id(&child_info, 4)["result"]["targetInfo"]["type"],
        json!("page")
    );

    send_cdp_command_without_wait(
        &mut browser,
        3,
        "Target.closeTarget",
        None,
        json!({ "targetId": tab_target_id }),
    )
    .await;
    let mut saw_close_response = false;
    let mut saw_page_destroyed = false;
    let mut saw_tab_destroyed = false;
    let closed = recv_until_match(&mut browser, |message| {
        saw_close_response |= message["id"] == json!(3_u64);
        if message["method"] == json!("Target.targetDestroyed") {
            saw_page_destroyed |= message["params"]["targetId"] == json!(page_target_id);
            saw_tab_destroyed |= message["params"]["targetId"] == json!(tab_target_id);
        }
        saw_close_response && saw_page_destroyed && saw_tab_destroyed
    })
    .await;
    assert_eq!(response_by_id(&closed, 3)["result"]["success"], json!(true));
    let page_destroyed = closed
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(page_target_id)
        })
        .expect("Page targetDestroyed");
    let tab_destroyed = closed
        .iter()
        .position(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(tab_target_id)
        })
        .expect("Tab targetDestroyed");
    assert!(
        page_destroyed < tab_destroyed,
        "Chromium destroys the Page host before its owning Tab host: {closed:#?}"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_agent_host_and_tab_host_survive_document_navigation() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let (mut page, _) = connect_async(format!("ws://{addr}/devtools/page/{DEFAULT_TARGET_ID}"))
        .await
        .expect("connect default Page websocket");
    let mut tab = connect_discovered_tab(addr, DEFAULT_TAB_TARGET_ID).await;

    let discover = send_cdp_command(
        &mut browser,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({ "discover": true, "filter": [{}] }),
    )
    .await;
    assert_eq!(response_by_id(&discover, 1)["result"], json!({}));

    let page_before = send_cdp_command(&mut page, 1, "Target.getTargetInfo", None, json!({})).await;
    let tab_before = send_cdp_command(&mut tab, 1, "Target.getTargetInfo", None, json!({})).await;
    assert_eq!(
        response_by_id(&page_before, 1)["result"]["targetInfo"]["targetId"],
        json!(DEFAULT_TARGET_ID)
    );
    assert_eq!(
        response_by_id(&tab_before, 1)["result"]["targetInfo"]["targetId"],
        json!(DEFAULT_TAB_TARGET_ID)
    );

    let destination = "data:text/html,<title>Stable Navigation Host</title>";
    let destination_title = "Stable Navigation Host";
    let navigation = send_cdp_command(
        &mut page,
        2,
        "Page.navigate",
        None,
        json!({ "url": destination }),
    )
    .await;
    assert_eq!(
        response_by_id(&navigation, 2)["result"]["frameId"],
        json!(DEFAULT_TARGET_ID),
        "the stable Page AgentHost id is also the main Frame id"
    );
    let title = send_cdp_command(
        &mut page,
        3,
        "Runtime.evaluate",
        None,
        json!({ "expression": "document.title", "returnByValue": true }),
    )
    .await;
    assert_eq!(
        response_by_id(&title, 3)["result"]["result"]["value"],
        json!(destination_title)
    );

    let mut target_events = recv_until_match(&mut browser, |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!(DEFAULT_TARGET_ID)
            && message["params"]["targetInfo"]["url"] == json!(destination)
    })
    .await;
    target_events
        .extend(send_cdp_command(&mut browser, 2, "Browser.getVersion", None, json!({})).await);
    target_events.extend(recv_cdp_messages_for(&mut browser, Duration::from_millis(100)).await);
    // The Tab frontend attaches on a separate socket, so its about:blank
    // attached-state delta may reach this discovery owner after navigation
    // starts. Only a Tab update carrying this document's metadata would mean
    // that the Page navigation was incorrectly mirrored onto the Tab host.
    let mirrored_tab_navigation = target_events.iter().find(|message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!(DEFAULT_TAB_TARGET_ID)
            && (message["params"]["targetInfo"]["url"] == json!(destination)
                || message["params"]["targetInfo"]["title"] == json!(destination_title))
    });
    assert!(
        mirrored_tab_navigation.is_none(),
        "document navigation must update the Page target without mirroring a Tab event: {target_events:#?}"
    );

    let descriptors = timeout(Duration::from_secs(5), async {
        loop {
            let descriptors = fetch_server_json(addr, "/json/list?for_tab").await;
            if descriptors.as_array().is_some_and(|targets| {
                targets.iter().any(|target| {
                    target["id"] == json!(DEFAULT_TAB_TARGET_ID)
                        && target["url"] == json!(destination)
                        && target["title"] == json!(destination_title)
                })
            }) {
                break descriptors;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Tab discovery metadata should follow its primary Page");
    assert!(descriptors.as_array().is_some_and(|targets| {
        targets.iter().any(|target| {
            target["id"] == json!(DEFAULT_TARGET_ID)
                && target["type"] == json!("page")
                && target["url"] == json!(destination)
        })
    }));

    let frame_tree = send_cdp_command(&mut page, 4, "Page.getFrameTree", None, json!({})).await;
    assert_eq!(
        response_by_id(&frame_tree, 4)["result"]["frameTree"]["frame"]["id"],
        json!(DEFAULT_TARGET_ID)
    );
    let page_after = send_cdp_command(&mut page, 5, "Target.getTargetInfo", None, json!({})).await;
    let tab_after = send_cdp_command(&mut tab, 2, "Target.getTargetInfo", None, json!({})).await;
    assert_eq!(
        response_by_id(&page_after, 5)["result"]["targetInfo"]["targetId"],
        json!(DEFAULT_TARGET_ID)
    );
    assert_eq!(
        response_by_id(&tab_after, 2)["result"]["targetInfo"]["targetId"],
        json!(DEFAULT_TAB_TARGET_ID)
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tab_websocket_stays_bound_to_its_page_when_a_popup_becomes_active() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut opener, _) = connect_async(format!("ws://{addr}/devtools/page/{DEFAULT_TARGET_ID}"))
        .await
        .expect("connect default page websocket");
    let opener_tab_websocket_url = discovered_tab_websocket_url(addr, DEFAULT_TAB_TARGET_ID).await;
    let mut opener_tab = connect_async(&opener_tab_websocket_url)
        .await
        .expect("connect opener tab websocket")
        .0;
    let tab_owner_info = send_cdp_command(
        &mut opener_tab,
        9001,
        "Target.getTargetInfo",
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        response_by_id(&tab_owner_info, 9001)["result"]["targetInfo"]["targetId"],
        json!(DEFAULT_TAB_TARGET_ID)
    );
    assert_eq!(
        response_by_id(&tab_owner_info, 9001)["result"]["targetInfo"]["type"],
        json!("tab")
    );
    let tab_page_command =
        send_cdp_command(&mut opener_tab, 9002, "Page.getFrameTree", None, json!({})).await;
    assert_eq!(
        response_by_id(&tab_page_command, 9002)["error"],
        json!({
            "code": -32601,
            "message": "'Page.getFrameTree' wasn't found"
        }),
        "the Tab AgentHost must expose its Page through Target auto-attach"
    );
    let opener_page_session = enable_tab_page(&mut opener_tab, 1, DEFAULT_TARGET_ID).await;
    let popup_url = "data:text/html,%3Ctitle%3EStable%20Popup%3C/title%3E";
    let popup_final_url = "data:text/html,<title>Stable Popup</title>";
    let opener_url = format!(
        "data:text/html,<title>Stable Opener</title>\
         <a id='popup' href='{popup_url}' \
         target='_blank' style='display:block;width:200px;height:100px'>open</a>"
    );

    let navigation = send_cdp_command(
        &mut opener,
        1,
        "Page.navigate",
        None,
        json!({ "url": &opener_url }),
    )
    .await;
    assert!(response_by_id(&navigation, 1).get("result").is_some());
    assert!(
        response_by_id(
            &send_cdp_command(&mut opener, 2, "Page.enable", None, json!({})).await,
            2,
        )
        .get("result")
        .is_some()
    );
    assert!(
        response_by_id(
            &send_cdp_command(
                &mut opener,
                3,
                "Target.setDiscoverTargets",
                None,
                json!({ "discover": true }),
            )
            .await,
            3,
        )
        .get("result")
        .is_some()
    );
    let screencast_started =
        send_cdp_command(&mut opener, 4, "Page.startScreencast", None, json!({})).await;
    assert_eq!(response_by_id(&screencast_started, 4)["result"], json!({}));
    assert!(
        screencast_started
            .iter()
            .any(|message| is_screencast_visibility(message, true))
    );

    let opener_link = send_cdp_command(
        &mut opener_tab,
        3,
        "Runtime.evaluate",
        Some(&opener_page_session),
        json!({
            "expression": "(() => { const link = document.getElementById('popup'); const rect = link?.getBoundingClientRect(); return { href: location.href, link: link?.href, x: rect?.x, y: rect?.y, width: rect?.width, height: rect?.height, hit: document.elementFromPoint(20, 20)?.id }; })()",
            "returnByValue": true
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&opener_link, 3)["result"]["result"]["value"]["link"],
        json!(popup_final_url)
    );

    dispatch_primary_click(&mut opener_tab, 5, Some(&opener_page_session), 20, 20).await;
    let mut activation_messages = recv_until_match(&mut opener, |message| {
        is_target_created_for_url(message, popup_final_url)
            || is_screencast_visibility(message, false)
    })
    .await;
    let mut saw_popup_created = activation_messages
        .iter()
        .any(|message| is_target_created_for_url(message, popup_final_url));
    let mut saw_opener_hidden = activation_messages
        .iter()
        .any(|message| is_screencast_visibility(message, false));
    if !saw_popup_created || !saw_opener_hidden {
        activation_messages.extend(
            recv_until_match(&mut opener, |message| {
                saw_popup_created |= is_target_created_for_url(message, popup_final_url);
                saw_opener_hidden |= is_screencast_visibility(message, false);
                saw_popup_created && saw_opener_hidden
            })
            .await,
        );
    }
    let popup_created_index = activation_messages
        .iter()
        .position(|message| is_target_created_for_url(message, popup_final_url))
        .expect("foreground click should create the popup target");
    let opener_hidden_index = activation_messages
        .iter()
        .position(|message| is_screencast_visibility(message, false))
        .expect("foreground click should hide the opener screencast");
    assert!(
        popup_created_index < opener_hidden_index,
        "Chromium reports popup creation before the opener becomes inactive: {activation_messages:#?}"
    );
    let popup_target_id = activation_messages
        .iter()
        .find(|message| is_target_created_for_url(message, popup_final_url))
        .and_then(|message| message["params"]["targetInfo"]["targetId"].as_str())
        .expect("foreground popup target id")
        .to_owned();

    let targets = wait_for_target_list(
        addr,
        "foreground popup should become the first browser surface",
        |targets| {
            targets.len() == 2
                && targets[0]["id"] != json!(DEFAULT_TARGET_ID)
                && targets[0]["title"] == json!("Stable Popup")
                && targets[1]["title"] == json!("Stable Opener")
        },
    )
    .await;
    assert_eq!(targets[0]["id"], json!(&popup_target_id));

    // A tab endpoint is a durable DevToolsAgentHost. Bringing another tab to
    // the foreground must not detach its child Page session or retarget the
    // socket to the new foreground Page.
    let opener_after_popup = send_cdp_command(
        &mut opener_tab,
        9,
        "Runtime.evaluate",
        Some(&opener_page_session),
        json!({
            "expression": "location.href",
            "returnByValue": true
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&opener_after_popup, 9)["result"]["result"]["value"],
        json!(&opener_url)
    );
    assert!(
        opener_after_popup.iter().all(|message| {
            let detached_from_opener = message["method"] == json!("Target.detachedFromTarget")
                && message["params"]["sessionId"] == json!(opener_page_session);
            let attached_to_popup = message["method"] == json!("Target.attachedToTarget")
                && message["params"]["targetInfo"]["targetId"] == json!(popup_target_id);
            !(detached_from_opener || attached_to_popup)
        }),
        "the opener tab socket migrated to the foreground popup: {opener_after_popup:#?}"
    );
    assert_eq!(targets[0]["title"], json!("Stable Popup"));
    assert_eq!(targets[1]["id"], json!(DEFAULT_TARGET_ID));
    assert_eq!(targets[1]["title"], json!("Stable Opener"));
    assert_eq!(targets[1]["url"], json!(&opener_url));

    let tab_targets = fetch_server_json(addr, "/json/list?for_tab").await;
    let tab_targets = tab_targets.as_array().expect("tab target list");
    let opener_tab_descriptor = tab_targets
        .iter()
        .find(|target| target["id"] == json!(DEFAULT_TAB_TARGET_ID))
        .expect("stable opener tab descriptor");
    assert_eq!(opener_tab_descriptor["title"], json!("Stable Opener"));
    assert_eq!(opener_tab_descriptor["url"], json!(&opener_url));
    let popup_tab_target_id = tab_targets
        .iter()
        .find(|target| target["type"] == json!("tab") && target["title"] == json!("Stable Popup"))
        .and_then(|target| target["id"].as_str())
        .expect("stable popup tab target id")
        .to_owned();
    assert_ne!(popup_tab_target_id, DEFAULT_TAB_TARGET_ID);

    // This URL was discovered before popup activation. It still identifies
    // the opener tab after the browser context's active page has changed.
    let mut late_opener_tab = connect_async(&opener_tab_websocket_url)
        .await
        .expect("connect opener tab from stale discovery descriptor")
        .0;
    let late_opener_page_session =
        enable_tab_page(&mut late_opener_tab, 1, DEFAULT_TARGET_ID).await;
    let late_opener_location = send_cdp_command(
        &mut late_opener_tab,
        3,
        "Runtime.evaluate",
        Some(&late_opener_page_session),
        json!({ "expression": "location.href", "returnByValue": true }),
    )
    .await;
    assert_eq!(
        response_by_id(&late_opener_location, 3)["result"]["result"]["value"],
        json!(&opener_url)
    );

    let mut popup_tab = connect_discovered_tab(addr, &popup_tab_target_id).await;
    let popup_page_session = enable_tab_page(&mut popup_tab, 1, &popup_target_id).await;
    let popup_location = send_cdp_command(
        &mut popup_tab,
        3,
        "Runtime.evaluate",
        Some(&popup_page_session),
        json!({ "expression": "location.href", "returnByValue": true }),
    )
    .await;
    assert_eq!(
        response_by_id(&popup_location, 3)["result"]["result"]["value"],
        json!(popup_final_url)
    );

    let opener_surface = evaluate_page_surface(&mut opener, 7).await;
    assert_eq!(opener_surface["href"], json!(&opener_url));
    assert_page_surface_active(&opener_surface, false);
    let parked_opener_screencast =
        send_cdp_command(&mut opener, 8, "Page.startScreencast", None, json!({})).await;
    assert_eq!(
        response_by_id(&parked_opener_screencast, 8)["result"],
        json!({})
    );
    assert!(
        parked_opener_screencast
            .iter()
            .any(|message| is_screencast_visibility(message, false))
    );

    let mut popup = connect_dynamic_page(addr, &popup_target_id).await;
    let popup_surface = evaluate_page_surface(&mut popup, 9).await;
    assert_page_surface_active(&popup_surface, true);
    let popup_screencast_started =
        send_cdp_command(&mut popup, 10, "Page.startScreencast", None, json!({})).await;
    assert_eq!(
        response_by_id(&popup_screencast_started, 10)["result"],
        json!({})
    );
    assert!(
        popup_screencast_started
            .iter()
            .any(|message| is_screencast_visibility(message, true))
    );

    let (activate_status, _) = fetch_server_response(
        addr,
        "GET",
        &format!("/json/activate/{DEFAULT_TAB_TARGET_ID}"),
    )
    .await;
    assert_eq!(activate_status, 200);
    wait_for_target_list(
        addr,
        "activating the opener should restore it as the browser surface",
        |targets| {
            targets
                .first()
                .is_some_and(|target| target["id"] == json!(DEFAULT_TARGET_ID))
        },
    )
    .await;

    expect_screencast_visibility(&mut opener, true).await;
    expect_screencast_visibility(&mut popup, false).await;

    let restored_opener = send_cdp_command(
        &mut opener_tab,
        10,
        "Runtime.evaluate",
        Some(&opener_page_session),
        json!({
            "expression": "location.href",
            "returnByValue": true
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&restored_opener, 10)["result"]["result"]["value"],
        json!(&opener_url)
    );
    let parked_popup = send_cdp_command(
        &mut popup_tab,
        4,
        "Runtime.evaluate",
        Some(&popup_page_session),
        json!({ "expression": "location.href", "returnByValue": true }),
    )
    .await;
    assert_eq!(
        response_by_id(&parked_popup, 4)["result"]["result"]["value"],
        json!(popup_final_url)
    );

    let restored_opener_surface = evaluate_page_surface(&mut opener, 11).await;
    assert_page_surface_active(&restored_opener_surface, true);
    let demoted_popup = evaluate_page_surface(&mut popup, 12).await;
    assert_page_surface_active(&demoted_popup, false);

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_parser_script_navigation_delivers_load_event() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            concat!(
                "<!doctype html><html><head>",
                "<meta charset=\"utf-8\"><title>CDP Core Fixture</title>",
                "<script src=\"/app.js\"></script></head><body>",
                "<h1 id=\"title\">CDP Core Fixture</h1>",
                "<script>window.__fixtureReady = true;</script>",
                "</body></html>"
            ),
        )
    }

    async fn app_script() -> impl IntoResponse {
        (
            [(
                axum::http::header::CONTENT_TYPE.as_str(),
                "application/javascript",
            )],
            "window.__scriptLoaded = true;",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let fixture_addr = fixture_listener.local_addr().expect("fixture addr");
    let fixture_server = tokio::spawn(async move {
        axum::serve(
            fixture_listener,
            Router::new()
                .route("/", get(page))
                .route("/app.js", get(app_script)),
        )
        .await
    });
    let fixture_url = format!("http://{fixture_addr}/");

    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 100).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;

    let enabled = send_cdp_command(&mut page, 1, "Page.enable", None, json!({})).await;
    assert_eq!(response_by_id(&enabled, 1)["result"], json!({}));

    page.send(WsMessage::Text(
        json!({
            "id": 2_u64,
            "method": "Page.navigate",
            "params": { "url": "about:blank" }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send first direct page navigation");

    let mut saw_first_response = false;
    let mut saw_first_load = false;
    let first_messages = recv_until_match(&mut page, |message| {
        saw_first_response |= message["id"] == json!(2_u64);
        saw_first_load |= message["method"] == json!("Page.loadEventFired");
        saw_first_response && saw_first_load
    })
    .await;
    assert!(
        response_by_id(&first_messages, 2).get("error").is_none(),
        "first direct page navigation failed: {first_messages:#?}"
    );

    let disabled = send_cdp_command(&mut page, 3, "Page.disable", None, json!({})).await;
    assert_eq!(response_by_id(&disabled, 3)["result"], json!({}));
    let reenabled = send_cdp_command(&mut page, 4, "Page.enable", None, json!({})).await;
    assert_eq!(response_by_id(&reenabled, 4)["result"], json!({}));

    page.send(WsMessage::Text(
        json!({
            "id": 5_u64,
            "method": "Page.navigate",
            "params": { "url": fixture_url }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send direct page navigation");

    let mut saw_second_response = false;
    let mut saw_second_load = false;
    let second_messages = recv_until_match(&mut page, |message| {
        saw_second_response |= message["id"] == json!(5_u64);
        saw_second_load |= message["method"] == json!("Page.loadEventFired");
        saw_second_response && saw_second_load
    })
    .await;
    assert!(
        second_messages
            .iter()
            .any(|message| message["method"] == json!("Page.loadEventFired")),
        "second direct page navigation did not receive loadEventFired: {second_messages:#?}"
    );
    assert!(
        response_by_id(&second_messages, 5).get("error").is_none(),
        "second direct page navigation failed: {second_messages:#?}"
    );

    let _ = page.close(None).await;
    let _ = browser.close(None).await;
    abort_test_cdp_server(server).await;
    fixture_server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_cdp_target_management_uses_live_agent_host_directory() {
    let (addr, server) = spawn_test_protocol_server().await;

    let (new_status, new_body) =
        fetch_server_response(addr, "PUT", "/json/new?about%3Ablank").await;
    assert_eq!(new_status, 200);
    let created: serde_json::Value =
        serde_json::from_slice(&new_body).expect("created target descriptor");
    let target_id = created["id"]
        .as_str()
        .expect("created target id")
        .to_owned();
    assert_ne!(target_id, DEFAULT_TARGET_ID);
    assert_eq!(created["url"], json!("about:blank"));

    let listed = fetch_server_json(addr, "/json/list").await;
    assert!(
        listed
            .as_array()
            .expect("target list")
            .iter()
            .any(|target| target["id"] == json!(target_id))
    );

    let mut page = connect_dynamic_page(addr, &target_id).await;
    let frame_tree = send_cdp_command(&mut page, 1, "Page.getFrameTree", None, json!({})).await;
    assert_eq!(
        response_by_id(&frame_tree, 1)["result"]["frameTree"]["frame"]["id"],
        json!(target_id)
    );

    let (activate_status, activate_body) =
        fetch_server_response(addr, "GET", &format!("/json/activate/{target_id}")).await;
    assert_eq!(activate_status, 200);
    assert_eq!(
        std::str::from_utf8(&activate_body).expect("activate response"),
        "Target activated"
    );

    let (close_status, close_body) =
        fetch_server_response(addr, "GET", &format!("/json/close/{target_id}")).await;
    assert_eq!(close_status, 200);
    assert_eq!(
        std::str::from_utf8(&close_body).expect("close response"),
        "Target is closing"
    );
    wait_for_websocket_close(&mut page, "HTTP-closed target page").await;

    let listed = fetch_server_json(addr, "/json/list").await;
    assert!(
        listed
            .as_array()
            .expect("target list")
            .iter()
            .all(|target| target["id"] != json!(target_id))
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_cdp_target_management_preserves_browser_discovery_events() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let discover = send_cdp_command(
        &mut browser,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({
            "discover": true,
            "filter": [{ "type": "page" }]
        }),
    )
    .await;
    assert_eq!(response_by_id(&discover, 1)["result"], json!({}));

    let (new_status, new_body) =
        fetch_server_response(addr, "PUT", "/json/new?about%3Ablank").await;
    assert_eq!(new_status, 200);
    let created: serde_json::Value =
        serde_json::from_slice(&new_body).expect("created target descriptor");
    let target_id = created["id"]
        .as_str()
        .expect("created target id")
        .to_owned();

    let created_events = recv_until_match(&mut browser, |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    })
    .await;
    assert!(
        created_events.iter().any(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["targetId"] == json!(target_id)
        }),
        "HTTP target creation did not reach the existing browser frontend: {created_events:#?}"
    );

    let (close_status, _) =
        fetch_server_response(addr, "GET", &format!("/json/close/{target_id}")).await;
    assert_eq!(close_status, 200);
    let destroyed_events = recv_until_match(&mut browser, |message| {
        message["method"] == json!("Target.targetDestroyed")
            && message["params"]["targetId"] == json!(target_id)
    })
    .await;
    assert!(
        destroyed_events.iter().any(|message| {
            message["method"] == json!("Target.targetDestroyed")
                && message["params"]["targetId"] == json!(target_id)
        }),
        "HTTP target destruction did not reach the existing browser frontend: {destroyed_events:#?}"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_child_session_routes_to_child_target() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let first_target_id = create_dynamic_target(&mut browser, 1).await;
    let second_target_id = create_dynamic_target(&mut browser, 2).await;
    let mut first_page = connect_dynamic_page(addr, &first_target_id).await;
    let mut second_page = connect_dynamic_page(addr, &second_target_id).await;

    let first_marker = send_cdp_command(
        &mut first_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_child_route = 'first-target'" }),
    )
    .await;
    assert_eq!(
        response_by_id(&first_marker, 1)["result"]["result"]["value"],
        json!("first-target")
    );
    let second_marker = send_cdp_command(
        &mut second_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_child_route = 'second-target'" }),
    )
    .await;
    assert_eq!(
        response_by_id(&second_marker, 1)["result"]["result"]["value"],
        json!("second-target")
    );

    let attach = send_cdp_command(
        &mut first_page,
        2,
        "Target.attachToTarget",
        None,
        json!({ "targetId": second_target_id, "flatten": true }),
    )
    .await;
    let child_session_id = response_by_id(&attach, 2)["result"]["sessionId"]
        .as_str()
        .expect("direct page child session")
        .to_owned();

    let child_read = send_cdp_command(
        &mut first_page,
        3,
        "Runtime.evaluate",
        Some(&child_session_id),
        json!({ "expression": "globalThis.__moli_child_route" }),
    )
    .await;
    let child_response = response_by_id(&child_read, 3);
    assert_eq!(
        child_response["result"]["result"]["value"],
        json!("second-target"),
        "child session command was executed against the direct page base target"
    );
    assert_eq!(
        child_response["sessionId"],
        json!(child_session_id),
        "flattened child session id was removed from the direct page wire response"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_rejects_unknown_child_session() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;

    let invalid = send_cdp_command(
        &mut page,
        1,
        "Runtime.evaluate",
        Some("SID-does-not-exist"),
        json!({ "expression": "globalThis.__moli_unknown_child_ran = true" }),
    )
    .await;
    let invalid_response = response_by_id(&invalid, 1);
    assert_eq!(invalid_response["error"]["code"], json!(-32001));
    assert_eq!(
        invalid_response["error"]["message"],
        json!("Unknown sessionId")
    );

    let probe = send_cdp_command(
        &mut page,
        2,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_unknown_child_ran" }),
    )
    .await;
    assert_eq!(
        response_by_id(&probe, 2)["result"]["result"]["type"],
        json!("undefined"),
        "unknown child session command executed against the base target"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_detach_reconnect_and_target_close_follow_host_lifecycle() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut first_page = connect_dynamic_page(addr, &target_id).await;
    let mut second_page = connect_dynamic_page(addr, &target_id).await;

    let first_set = send_cdp_command(
        &mut first_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_reconnect_state = 41" }),
    )
    .await;
    assert_eq!(
        response_by_id(&first_set, 1)["result"]["result"]["value"],
        json!(41)
    );
    let second_read = send_cdp_command(
        &mut second_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_reconnect_state + 1" }),
    )
    .await;
    assert_eq!(
        response_by_id(&second_read, 1)["result"]["result"]["value"],
        json!(42),
        "two direct page clients did not get independent sessions on the same target"
    );

    first_page
        .close(None)
        .await
        .expect("close first page socket");
    let mut reconnected_page = connect_dynamic_page(addr, &target_id).await;
    let reconnected_read = send_cdp_command(
        &mut reconnected_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_reconnect_state" }),
    )
    .await;
    assert_eq!(
        response_by_id(&reconnected_read, 1)["result"]["result"]["value"],
        json!(41),
        "page frontend disconnect destroyed or replaced the target runtime"
    );

    let close = send_cdp_command(
        &mut browser,
        2,
        "Target.closeTarget",
        None,
        json!({ "targetId": target_id }),
    )
    .await;
    assert_eq!(response_by_id(&close, 2)["result"]["success"], json!(true));
    assert!(
        close
            .iter()
            .all(|message| message["method"] != json!("Target.detachedFromTarget")),
        "browser frontend received a private direct-page detach event: {close:#?}"
    );
    if let Ok(message) =
        tokio::time::timeout(Duration::from_millis(100), recv_ws_json(&mut browser)).await
    {
        panic!("browser frontend received private direct-page output: {message:#?}");
    }

    wait_for_websocket_close(&mut reconnected_page, "closed target page").await;
    assert_eq!(
        rejected_websocket_status(format!("ws://{addr}/devtools/page/{target_id}")).await,
        404
    );
    let target_list = fetch_server_json(addr, "/json/list").await;
    assert!(
        target_list
            .as_array()
            .expect("target list array")
            .iter()
            .all(|target| target["id"] != json!(target_id)),
        "closed target remained in /json/list: {target_list:#?}"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_supports_concurrent_browser_frontends_with_isolated_sessions() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut first_browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect first browser websocket");
    let (mut second_browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect second browser websocket");

    for browser in [&mut first_browser, &mut second_browser] {
        browser
            .send(WsMessage::Text(
                json!({ "id": 1, "method": "Browser.getVersion", "params": {} })
                    .to_string()
                    .into(),
            ))
            .await
            .expect("send colliding browser command id");
    }
    let first_probe = recv_until_id(&mut first_browser, 1).await;
    let second_probe = recv_until_id(&mut second_browser, 1).await;
    assert!(
        response_by_id(&first_probe, 1)["result"]["product"]
            .as_str()
            .is_some(),
        "first browser frontend did not respond"
    );
    assert!(
        response_by_id(&second_probe, 1)["result"]["product"]
            .as_str()
            .is_some(),
        "second browser frontend did not respond"
    );
    assert!(
        first_probe
            .iter()
            .chain(&second_probe)
            .all(|message| message.get("sessionId").is_none()),
        "a hidden browser base session leaked onto the public wire"
    );

    let discover = send_cdp_command(
        &mut first_browser,
        2,
        "Target.setDiscoverTargets",
        None,
        json!({ "discover": true, "filter": [{ "type": "page" }] }),
    )
    .await;
    assert_eq!(response_by_id(&discover, 2)["result"], json!({}));
    let create = send_cdp_command(
        &mut second_browser,
        2,
        "Target.createTarget",
        None,
        json!({ "url": "about:blank" }),
    )
    .await;
    let target_id = response_by_id(&create, 2)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    assert!(
        create.iter().all(|message| {
            message["method"] != json!("Target.targetCreated")
                || message["params"]["targetInfo"]["targetId"] != json!(target_id)
        }),
        "second frontend inherited first frontend's discovery state: {create:#?}"
    );
    let discovered = recv_until_match(&mut first_browser, |message| {
        message["method"] == json!("Target.targetCreated")
            && message["params"]["targetInfo"]["targetId"] == json!(target_id)
    })
    .await;
    assert!(
        discovered
            .last()
            .is_some_and(|message| message.get("sessionId").is_none()),
        "discovery event leaked the first browser's hidden base session"
    );

    let first_attach = send_cdp_command(
        &mut first_browser,
        3,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id, "flatten": true }),
    )
    .await;
    let first_session_id = response_by_id(&first_attach, 3)["result"]["sessionId"]
        .as_str()
        .expect("first target session id")
        .to_owned();
    let second_attach = send_cdp_command(
        &mut second_browser,
        3,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id, "flatten": true }),
    )
    .await;
    let second_session_id = response_by_id(&second_attach, 3)["result"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    assert_ne!(first_session_id, second_session_id);

    let first_write = send_cdp_command(
        &mut first_browser,
        4,
        "Runtime.evaluate",
        Some(&first_session_id),
        json!({ "expression": "globalThis.__moli_multi_browser = 41" }),
    )
    .await;
    assert_eq!(
        response_by_id(&first_write, 4)["result"]["result"]["value"],
        json!(41)
    );
    let second_read = send_cdp_command(
        &mut second_browser,
        4,
        "Runtime.evaluate",
        Some(&second_session_id),
        json!({ "expression": "globalThis.__moli_multi_browser" }),
    )
    .await;
    assert_eq!(
        response_by_id(&second_read, 4)["result"]["result"]["value"],
        json!(41),
        "browser frontends did not share the target runtime"
    );

    let foreign_flat_session = send_cdp_command(
        &mut second_browser,
        5,
        "Runtime.evaluate",
        Some(&first_session_id),
        json!({ "expression": "1" }),
    )
    .await;
    assert_eq!(
        response_by_id(&foreign_flat_session, 5)["error"]["code"],
        json!(-32001)
    );
    let foreign_legacy_session = send_cdp_command(
        &mut second_browser,
        6,
        "Target.detachFromTarget",
        None,
        json!({ "sessionId": first_session_id }),
    )
    .await;
    assert_eq!(
        response_by_id(&foreign_legacy_session, 6)["error"]["code"],
        json!(-32602)
    );

    first_browser
        .close(None)
        .await
        .expect("close first browser websocket");
    wait_for_websocket_close(&mut first_browser, "first concurrent browser").await;
    let surviving_read = send_cdp_command(
        &mut second_browser,
        7,
        "Runtime.evaluate",
        Some(&second_session_id),
        json!({ "expression": "globalThis.__moli_multi_browser + 1" }),
    )
    .await;
    assert_eq!(
        response_by_id(&surviving_read, 7)["result"]["result"]["value"],
        json!(42),
        "disconnecting one browser detached another browser's target session"
    );

    let (mut third_browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect third browser while second remains connected");
    let third_probe =
        send_cdp_command(&mut third_browser, 1, "Browser.getVersion", None, json!({})).await;
    assert!(
        response_by_id(&third_probe, 1)["result"]["product"]
            .as_str()
            .is_some(),
        "replacement browser could not connect alongside surviving browser"
    );
    let third_attach = send_cdp_command(
        &mut third_browser,
        2,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id, "flatten": true }),
    )
    .await;
    let third_session_id = response_by_id(&third_attach, 2)["result"]["sessionId"]
        .as_str()
        .expect("third target session id")
        .to_owned();
    let third_read = send_cdp_command(
        &mut third_browser,
        3,
        "Runtime.evaluate",
        Some(&third_session_id),
        json!({ "expression": "globalThis.__moli_multi_browser" }),
    )
    .await;
    assert_eq!(
        response_by_id(&third_read, 3)["result"]["result"]["value"],
        json!(41)
    );

    let close = send_cdp_command(
        &mut third_browser,
        4,
        "Target.closeTarget",
        None,
        json!({ "targetId": target_id }),
    )
    .await;
    assert_eq!(response_by_id(&close, 4)["result"]["success"], json!(true));
    let second_detached = recv_until_match(&mut second_browser, |message| {
        message["method"] == json!("Target.detachedFromTarget")
            && message["params"]["sessionId"] == json!(second_session_id)
    })
    .await;
    assert!(
        second_detached
            .last()
            .is_some_and(|message| message.get("sessionId").is_none()),
        "target-close detach leaked the second browser's hidden base session"
    );
    let stale_session = send_cdp_command(
        &mut second_browser,
        8,
        "Runtime.evaluate",
        Some(&second_session_id),
        json!({ "expression": "1" }),
    )
    .await;
    assert_eq!(
        response_by_id(&stale_session, 8)["error"]["code"],
        json!(-32001),
        "closed target session remained routable"
    );
    let final_second_probe = send_cdp_command(
        &mut second_browser,
        9,
        "Browser.getVersion",
        None,
        json!({}),
    )
    .await;
    assert!(
        response_by_id(&final_second_probe, 9)["result"]["product"]
            .as_str()
            .is_some(),
        "closing a shared target disrupted the surviving browser frontend"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_browser_reconnect_resets_discovery_state() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut discovering_browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect discovering browser websocket");

    let discover = send_cdp_command(
        &mut discovering_browser,
        1,
        "Target.setDiscoverTargets",
        None,
        json!({
            "discover": true,
            "filter": [{ "type": "page" }]
        }),
    )
    .await;
    assert_eq!(response_by_id(&discover, 1)["result"], json!({}));

    discovering_browser
        .close(None)
        .await
        .expect("close discovering browser websocket");
    wait_for_websocket_close(&mut discovering_browser, "discovering browser").await;

    let (mut replacement_browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect replacement browser websocket");
    let create = send_cdp_command(
        &mut replacement_browser,
        1,
        "Target.createTarget",
        None,
        json!({ "url": "about:blank" }),
    )
    .await;
    let target_id = response_by_id(&create, 1)["result"]["targetId"]
        .as_str()
        .expect("created target id")
        .to_owned();
    assert!(
        create.iter().all(|message| {
            message["method"] != json!("Target.targetCreated")
                || message["params"]["targetInfo"]["targetId"] != json!(target_id)
        }),
        "target discovery state leaked into a replacement browser frontend: {create:#?}"
    );

    let replacement_probe = send_cdp_command(
        &mut replacement_browser,
        2,
        "Target.getTargets",
        None,
        json!({}),
    )
    .await;
    assert!(
        replacement_probe.iter().all(|message| {
            message["method"] != json!("Target.targetCreated")
                || message["params"]["targetInfo"]["targetId"] != json!(target_id)
        }),
        "late discovery output leaked into the replacement browser: {replacement_probe:#?}"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_browser_reconnect_can_control_existing_target() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut first_browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect first browser websocket");
    let target_id = create_dynamic_target(&mut first_browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;
    let marker = send_cdp_command(
        &mut page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_browser_reconnect = 73" }),
    )
    .await;
    assert_eq!(
        response_by_id(&marker, 1)["result"]["result"]["value"],
        json!(73)
    );

    first_browser
        .close(None)
        .await
        .expect("close first browser websocket");
    wait_for_websocket_close(&mut first_browser, "first browser frontend").await;

    let (mut second_browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect replacement browser websocket");
    let targets =
        send_cdp_command(&mut second_browser, 1, "Target.getTargets", None, json!({})).await;
    assert!(
        response_by_id(&targets, 1)["result"]["targetInfos"]
            .as_array()
            .expect("replacement browser targetInfos")
            .iter()
            .any(|target| target["targetId"] == json!(target_id)),
        "replacement browser did not discover the existing target"
    );

    let attach = send_cdp_command(
        &mut second_browser,
        2,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id, "flatten": true }),
    )
    .await;
    let session_id = response_by_id(&attach, 2)["result"]["sessionId"]
        .as_str()
        .expect("replacement browser target session")
        .to_owned();
    let read = send_cdp_command(
        &mut second_browser,
        3,
        "Runtime.evaluate",
        Some(&session_id),
        json!({ "expression": "globalThis.__moli_browser_reconnect" }),
    )
    .await;
    assert_eq!(
        response_by_id(&read, 3)["result"]["result"]["value"],
        json!(73)
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_default_page_created_target_has_global_identity_and_route() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let browser_target_id = create_dynamic_target(&mut browser, 1).await;
    let mut browser_page = connect_dynamic_page(addr, &browser_target_id).await;
    let browser_marker = send_cdp_command(
        &mut browser_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_owner_identity = 'browser-owner'" }),
    )
    .await;
    assert_eq!(
        response_by_id(&browser_marker, 1)["result"]["result"]["value"],
        json!("browser-owner")
    );

    let (mut default_page, _) =
        connect_async(format!("ws://{addr}/devtools/page/{DEFAULT_TARGET_ID}"))
            .await
            .expect("connect default page websocket");
    let default_page_target_id = create_dynamic_target(&mut default_page, 2).await;
    assert_ne!(
        browser_target_id, default_page_target_id,
        "all protocol-server owners must allocate target ids from one namespace"
    );

    let attach = send_cdp_command(
        &mut default_page,
        3,
        "Target.attachToTarget",
        None,
        json!({ "targetId": default_page_target_id, "flatten": true }),
    )
    .await;
    let default_page_session_id = response_by_id(&attach, 3)["result"]["sessionId"]
        .as_str()
        .expect("default page target session")
        .to_owned();
    let default_page_marker = send_cdp_command(
        &mut default_page,
        4,
        "Runtime.evaluate",
        Some(&default_page_session_id),
        json!({ "expression": "globalThis.__moli_owner_identity = 'default-page-owner'" }),
    )
    .await;
    assert_eq!(
        response_by_id(&default_page_marker, 4)["result"]["result"]["value"],
        json!("default-page-owner")
    );

    default_page
        .close(None)
        .await
        .expect("close default page frontend");
    wait_for_websocket_close(&mut default_page, "default page frontend").await;

    let mut routed_page = connect_dynamic_page(addr, &default_page_target_id).await;
    let routed_marker = send_cdp_command(
        &mut routed_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_owner_identity" }),
    )
    .await;
    assert_eq!(
        response_by_id(&routed_marker, 1)["result"]["result"]["value"],
        json!("default-page-owner"),
        "the published route did not resolve to the target that returned its id"
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_pending_runtime_command_does_not_block_later_command() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;

    page.send(WsMessage::Text(
        json!({
            "id": 1_u64,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => { globalThis.__moli_resolve_page = resolve; })",
                "awaitPromise": true,
                "returnByValue": true
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send pending direct page command");
    page.send(WsMessage::Text(
        json!({
            "id": 2_u64,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__moli_resolve_page('resolved'); 'released'",
                "returnByValue": true
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send command that resolves pending direct page command");

    let mut saw_pending_response = false;
    let mut saw_release_response = false;
    let messages = recv_until_match(&mut page, |message| {
        saw_pending_response |= message["id"] == json!(1_u64);
        saw_release_response |= message["id"] == json!(2_u64);
        saw_pending_response && saw_release_response
    })
    .await;
    assert_eq!(
        response_by_id(&messages, 1)["result"]["result"]["value"],
        json!("resolved")
    );
    assert_eq!(
        response_by_id(&messages, 2)["result"]["result"]["value"],
        json!("released")
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_matches_chromium_browser_attach_access_modes() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;

    let specialized = send_cdp_command(
        &mut page,
        1,
        "Target.attachToBrowserTarget",
        None,
        json!({}),
    )
    .await;
    let specialized_response = response_by_id(&specialized, 1);
    assert_eq!(specialized_response["error"]["code"], json!(-32000));
    assert_eq!(
        specialized_response["error"]["message"],
        json!("Not allowed")
    );
    assert!(specialized_response.get("sessionId").is_none());

    let discovery = send_cdp_command(
        &mut page,
        2,
        "Target.setDiscoverTargets",
        None,
        json!({ "discover": true, "filter": [{}] }),
    )
    .await;
    let browser_target_id = discovery
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetCreated")
                && message["params"]["targetInfo"]["type"] == json!("browser")
        })
        .and_then(|message| message["params"]["targetInfo"]["targetId"].as_str())
        .expect("direct page discovery should report the browser target")
        .to_owned();

    let attach = send_cdp_command(
        &mut page,
        3,
        "Target.attachToTarget",
        None,
        json!({ "targetId": browser_target_id, "flatten": true }),
    )
    .await;
    let attach_response = response_by_id(&attach, 3);
    assert!(attach_response.get("sessionId").is_none());
    let browser_session_id = attach_response["result"]["sessionId"]
        .as_str()
        .expect("generic browser target session")
        .to_owned();
    assert!(attach.iter().any(|message| {
        message["method"] == json!("Target.attachedToTarget")
            && message.get("sessionId").is_none()
            && message["params"]["sessionId"] == json!(browser_session_id)
            && message["params"]["targetInfo"]["type"] == json!("browser")
    }));

    let version = send_cdp_command(
        &mut page,
        4,
        "Browser.getVersion",
        Some(&browser_session_id),
        json!({}),
    )
    .await;
    let version_response = response_by_id(&version, 4);
    assert_eq!(version_response["sessionId"], json!(browser_session_id));
    assert!(version_response["result"]["product"].is_string());

    page.close(None).await.expect("close page websocket");
    browser.close(None).await.expect("close browser websocket");
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_dynamic_page_survives_browser_frontend_disconnect_until_target_close() {
    let (addr, server, owner_registry) = spawn_test_protocol_server_with_owner_registry().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;
    let initial = send_cdp_command(
        &mut page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_stage3_state = 73" }),
    )
    .await;
    assert_eq!(
        response_by_id(&initial, 1)["result"]["result"]["value"],
        json!(73)
    );
    let runtime_enabled = send_cdp_command(&mut page, 10, "Runtime.enable", None, json!({})).await;
    assert_eq!(response_by_id(&runtime_enabled, 10)["result"], json!({}));
    let binding_added = send_cdp_command(
        &mut page,
        11,
        "Runtime.addBinding",
        None,
        json!({ "name": "__moli_stage3_binding" }),
    )
    .await;
    assert_eq!(response_by_id(&binding_added, 11)["result"], json!({}));
    let browser_attach = send_cdp_command(
        &mut browser,
        2,
        "Target.attachToTarget",
        None,
        json!({ "targetId": target_id.clone(), "flatten": true }),
    )
    .await;
    assert!(
        response_by_id(&browser_attach, 2)["result"]["sessionId"]
            .as_str()
            .is_some()
    );
    assert_eq!(owner_registry.owner_count(), 1);

    browser
        .close(None)
        .await
        .expect("close browser frontend socket");
    wait_for_websocket_close(&mut browser, "client-closed browser").await;
    assert_eq!(
        owner_registry.owner_count(),
        1,
        "browser detach destroyed an owner that still had a live dynamic target"
    );
    let target_list = fetch_server_json(addr, "/json/list").await;
    assert!(
        target_list
            .as_array()
            .expect("target list array")
            .iter()
            .any(|target| target["id"] == json!(target_id)),
        "browser detach removed a live target from /json/list: {target_list:#?}"
    );

    let after_browser_close = send_cdp_command(
        &mut page,
        2,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_stage3_state + 1" }),
    )
    .await;
    assert_eq!(
        response_by_id(&after_browser_close, 2)["result"]["result"]["value"],
        json!(74)
    );
    page.send(WsMessage::Text(
        json!({
            "id": 12_u64,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "__moli_stage3_binding('after-browser-close')"
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("invoke direct page binding after browser detach");
    let mut saw_binding_response = false;
    let mut saw_binding_event = false;
    let binding_messages = recv_until_match(&mut page, |message| {
        saw_binding_response |= message["id"] == json!(12_u64);
        saw_binding_event |= message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("__moli_stage3_binding")
            && message["params"]["payload"] == json!("after-browser-close");
        saw_binding_response && saw_binding_event
    })
    .await;
    assert!(
        binding_messages
            .iter()
            .any(|message| message["method"] == json!("Runtime.bindingCalled")),
        "browser detach removed the direct page frontend Runtime binding state"
    );

    page.close(None).await.expect("close first page frontend");
    let mut reconnected_page = connect_dynamic_page(addr, &target_id).await;
    let after_reconnect = send_cdp_command(
        &mut reconnected_page,
        1,
        "Runtime.evaluate",
        None,
        json!({ "expression": "globalThis.__moli_stage3_state" }),
    )
    .await;
    assert_eq!(
        response_by_id(&after_reconnect, 1)["result"]["result"]["value"],
        json!(73)
    );

    let close = send_cdp_command(
        &mut reconnected_page,
        2,
        "Target.closeTarget",
        None,
        json!({ "targetId": target_id }),
    )
    .await;
    assert_eq!(response_by_id(&close, 2)["result"]["success"], json!(true));
    wait_for_websocket_close(&mut reconnected_page, "last closed target page").await;
    assert_eq!(
        owner_registry.owner_count(),
        1,
        "the server-level CDP control plane must survive after its last dynamic target closes"
    );
    assert_eq!(
        rejected_websocket_status(format!("ws://{addr}/devtools/page/{target_id}")).await,
        404
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_owner_registry_shutdown_closes_frontends_and_joins_owner() {
    let (addr, server, owner_registry) = spawn_test_protocol_server_with_owner_registry().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let target_id = create_dynamic_target(&mut browser, 1).await;
    let mut page = connect_dynamic_page(addr, &target_id).await;
    assert_eq!(owner_registry.owner_count(), 1);

    timeout(Duration::from_secs(5), owner_registry.shutdown())
        .await
        .expect("owner registry shutdown should join all owner threads");
    assert_eq!(owner_registry.owner_count(), 0);
    wait_for_websocket_close(&mut browser, "registry-shutdown browser").await;
    wait_for_websocket_close(&mut page, "registry-shutdown page").await;
    assert_eq!(
        rejected_websocket_status(format!("ws://{addr}/devtools/page/{target_id}")).await,
        404
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_shared_owner_survives_idle_browser_reconnect() {
    let (addr, server, owner_registry) = spawn_test_protocol_server_with_owner_registry().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let attach = send_cdp_command(
        &mut browser,
        1,
        "Target.attachToTarget",
        None,
        json!({ "targetId": DEFAULT_TARGET_ID, "flatten": true }),
    )
    .await;
    let session_id = response_by_id(&attach, 1)["result"]["sessionId"]
        .as_str()
        .expect("default target session")
        .to_owned();
    let marker = send_cdp_command(
        &mut browser,
        2,
        "Runtime.evaluate",
        Some(&session_id),
        json!({ "expression": "globalThis.__moli_idle_reconnect = 29" }),
    )
    .await;
    assert_eq!(
        response_by_id(&marker, 2)["result"]["result"]["value"],
        json!(29)
    );
    assert_eq!(owner_registry.owner_count(), 1);

    browser
        .close(None)
        .await
        .expect("close browser frontend socket");
    wait_for_websocket_close(&mut browser, "detached empty browser").await;
    assert_eq!(owner_registry.owner_count(), 1);

    let (mut replacement, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("reconnect browser websocket");
    let attach = send_cdp_command(
        &mut replacement,
        1,
        "Target.attachToTarget",
        None,
        json!({ "targetId": DEFAULT_TARGET_ID, "flatten": true }),
    )
    .await;
    let replacement_session_id = response_by_id(&attach, 1)["result"]["sessionId"]
        .as_str()
        .expect("replacement default target session")
        .to_owned();
    let marker = send_cdp_command(
        &mut replacement,
        2,
        "Runtime.evaluate",
        Some(&replacement_session_id),
        json!({ "expression": "globalThis.__moli_idle_reconnect" }),
    )
    .await;
    assert_eq!(
        response_by_id(&marker, 2)["result"]["result"]["value"],
        json!(29),
        "idle browser reconnect did not reuse the server-level target control plane"
    );
    assert_eq!(owner_registry.owner_count(), 1);

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_puppeteer_reconnect_recreates_session_owned_runtime_context() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (status, body) = fetch_server_response(addr, "PUT", "/json/new?about%3Ablank").await;
    assert_eq!(status, 200);
    let created_target: serde_json::Value =
        serde_json::from_slice(&body).expect("created target descriptor");
    let created_target_id = created_target["id"]
        .as_str()
        .expect("created target id")
        .to_owned();
    assert_ne!(created_target_id, DEFAULT_TARGET_ID);
    // Materializing a second Page parks the old default Page. Puppeteer's
    // first Page command promotes that existing target back to the active
    // slot, which is the reconnect lifecycle that regressed.
    let target_id = DEFAULT_TARGET_ID.to_owned();

    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect first browser websocket");
    let page_session_id =
        puppeteer_auto_attach_existing_page(&mut browser, 1, &target_id, DEFAULT_TAB_TARGET_ID)
            .await;
    let page_enabled = send_cdp_command(
        &mut browser,
        3,
        "Page.enable",
        Some(&page_session_id),
        json!({}),
    )
    .await;
    assert_eq!(response_by_id(&page_enabled, 3)["result"], json!({}));
    let _ = enable_runtime_and_expect_default_context(
        &mut browser,
        4,
        Some(&page_session_id),
        "first Runtime.enable",
    )
    .await;
    let utility_world = send_cdp_command(
        &mut browser,
        6,
        "Page.createIsolatedWorld",
        Some(&page_session_id),
        json!({
            "frameId": target_id,
            "worldName": "__puppeteer_utility_world__moli_reconnect",
            "grantUniveralAccess": true
        }),
    )
    .await;
    let first_utility_context_id =
        response_by_id(&utility_world, 6)["result"]["executionContextId"]
            .as_i64()
            .unwrap_or_else(|| panic!("failed to create the utility world: {utility_world:#?}"));

    browser
        .close(None)
        .await
        .expect("close first browser frontend");
    wait_for_websocket_close(&mut browser, "first browser frontend").await;

    let (mut replacement, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect replacement browser websocket");
    let replacement_page_session_id =
        puppeteer_auto_attach_existing_page(&mut replacement, 1, &target_id, DEFAULT_TAB_TARGET_ID)
            .await;
    assert_ne!(replacement_page_session_id, page_session_id);
    let page_enabled = send_cdp_command(
        &mut replacement,
        3,
        "Page.enable",
        Some(&replacement_page_session_id),
        json!({}),
    )
    .await;
    assert_eq!(response_by_id(&page_enabled, 3)["result"], json!({}));
    let replay = enable_runtime_and_expect_default_context(
        &mut replacement,
        4,
        Some(&replacement_page_session_id),
        "replacement Runtime.enable",
    )
    .await;
    assert!(
        replay.iter().all(|message| {
            message["sessionId"] != json!(replacement_page_session_id)
                || message["method"] != json!("Runtime.executionContextCreated")
                || message["params"]["context"]["name"]
                    != json!("__puppeteer_utility_world__moli_reconnect")
        }),
        "replacement Runtime.enable inherited a detached session's utility world: {replay:#?}"
    );

    let replacement_utility_world = send_cdp_command(
        &mut replacement,
        6,
        "Page.createIsolatedWorld",
        Some(&replacement_page_session_id),
        json!({
            "frameId": target_id,
            "worldName": "__puppeteer_utility_world__moli_reconnect",
            "grantUniveralAccess": true
        }),
    )
    .await;
    let replacement_utility_context_id =
        response_by_id(&replacement_utility_world, 6)["result"]["executionContextId"]
            .as_i64()
            .unwrap_or_else(|| {
                panic!(
                    "replacement session failed to recreate its utility world: \
                 {replacement_utility_world:#?}"
                )
            });
    assert_ne!(replacement_utility_context_id, first_utility_context_id);

    let evaluated = send_cdp_command(
        &mut replacement,
        7,
        "Runtime.evaluate",
        Some(&replacement_page_session_id),
        json!({ "expression": "6 * 7", "returnByValue": true }),
    )
    .await;
    assert_eq!(
        response_by_id(&evaluated, 7)["result"]["result"]["value"],
        json!(42)
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_browser_reconnect_clears_detached_session_emulated_media() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("connect browser websocket");
    let attach = send_cdp_command(
        &mut browser,
        1,
        "Target.attachToTarget",
        None,
        json!({ "targetId": DEFAULT_TARGET_ID, "flatten": true }),
    )
    .await;
    let session_id = response_by_id(&attach, 1)["result"]["sessionId"]
        .as_str()
        .expect("default target session")
        .to_owned();
    let media = json!({
        "features": [
            { "name": "prefers-color-scheme", "value": "dark" }
        ]
    });
    let set_dark = send_cdp_command(
        &mut browser,
        2,
        "Emulation.setEmulatedMedia",
        Some(&session_id),
        media.clone(),
    )
    .await;
    assert!(response_by_id(&set_dark, 2).get("result").is_some());
    let dark = send_cdp_command(
        &mut browser,
        3,
        "Runtime.evaluate",
        Some(&session_id),
        json!({
            "expression": "matchMedia('(prefers-color-scheme: dark)').matches",
            "returnByValue": true
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&dark, 3)["result"]["result"]["value"],
        json!(true)
    );

    browser
        .close(None)
        .await
        .expect("close first browser frontend");
    wait_for_websocket_close(&mut browser, "detached browser frontend").await;

    let (mut replacement, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("reconnect browser websocket");
    let attach = send_cdp_command(
        &mut replacement,
        1,
        "Target.attachToTarget",
        None,
        json!({ "targetId": DEFAULT_TARGET_ID, "flatten": true }),
    )
    .await;
    let replacement_session_id = response_by_id(&attach, 1)["result"]["sessionId"]
        .as_str()
        .expect("replacement default target session")
        .to_owned();
    let reset = send_cdp_command(
        &mut replacement,
        2,
        "Runtime.evaluate",
        Some(&replacement_session_id),
        json!({
            "expression": "matchMedia('(prefers-color-scheme: dark)').matches",
            "returnByValue": true
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&reset, 2)["result"]["result"]["value"],
        json!(false),
        "a detached CDP session leaked its media override into the replacement frontend"
    );

    let set_dark = send_cdp_command(
        &mut replacement,
        3,
        "Emulation.setEmulatedMedia",
        Some(&replacement_session_id),
        media,
    )
    .await;
    assert!(response_by_id(&set_dark, 3).get("result").is_some());
    let dark = send_cdp_command(
        &mut replacement,
        4,
        "Runtime.evaluate",
        Some(&replacement_session_id),
        json!({
            "expression": "matchMedia('(prefers-color-scheme: dark)').matches",
            "returnByValue": true
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&dark, 4)["result"]["result"]["value"],
        json!(true)
    );

    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_cdp_owner_registry_shutdown_joins_shared_default_page_owner() {
    let (addr, server, owner_registry) = spawn_test_protocol_server_with_owner_registry().await;
    let (mut page, _) = connect_async(format!("ws://{addr}/devtools/page/{DEFAULT_TARGET_ID}"))
        .await
        .expect("connect default page websocket");
    let frame_tree = send_cdp_command(&mut page, 1, "Page.getFrameTree", None, json!({})).await;
    assert!(response_by_id(&frame_tree, 1).get("result").is_some());
    assert_eq!(owner_registry.owner_count(), 1);

    timeout(Duration::from_secs(5), owner_registry.shutdown())
        .await
        .expect("registry shutdown should join shared default page owner");
    assert_eq!(owner_registry.owner_count(), 0);
    wait_for_websocket_close(&mut page, "shared owner default page").await;

    abort_test_cdp_server(server).await;
}
