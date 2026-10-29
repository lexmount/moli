use super::*;
use serde_json::Value;

type CdpTestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn wait_for_session_navigation_response_and_load(
    socket: &mut CdpTestSocket,
    command_id: u64,
    session_id: &str,
) {
    let mut saw_navigate_response = false;
    let mut saw_load_event = false;
    while !(saw_navigate_response && saw_load_event) {
        let message = recv_ws_json(socket).await;
        if message["id"] == json!(command_id) {
            saw_navigate_response = true;
        }
        if message["sessionId"] == json!(session_id)
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_load_event = true;
        }
    }
}

async fn runtime_click_download_link(
    socket: &mut CdpTestSocket,
    command_id: u64,
    session_id: &str,
) -> Vec<Value> {
    socket
        .send(WsMessage::Text(
            json!({
                "id": command_id,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "document.getElementById('dl').click(); 'clicked'",
                    "userGesture": true,
                    "returnByValue": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate click download link");
    recv_until_id(socket, command_id).await
}

fn runtime_click_response_value(messages: &[Value], command_id: u64) -> Option<&str> {
    messages
        .iter()
        .find(|message| message["id"] == json!(command_id))
        .and_then(|message| message["result"]["result"]["value"].as_str())
}

#[tokio::test]
async fn websocket_cdp_download_events_are_emitted_after_input_response() {
    let (fixture_addr, fixture_server) =
        spawn_delayed_download_fixture_server("download-body", Duration::from_millis(100)).await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-download-async-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    wait_for_session_navigation_response_and_load(&mut socket, 6, &session_id).await;

    let first_after_release = runtime_click_download_link(&mut socket, 7, &session_id).await;
    assert_eq!(
        runtime_click_response_value(&first_after_release, 7),
        Some("clicked"),
        "download activation should return the Runtime.evaluate response before outbound Browser download events: {first_after_release:?}"
    );

    let (page_will_begin, browser_will_begin) = timeout(Duration::from_secs(2), async {
        let mut page_will_begin = None;
        let mut browser_will_begin = None;
        loop {
            let message = recv_ws_json(&mut socket).await;
            match message["method"].as_str() {
                Some("Page.downloadWillBegin") => page_will_begin = Some(message),
                Some("Browser.downloadWillBegin") => browser_will_begin = Some(message),
                _ => {}
            }
            if page_will_begin.is_some() && browser_will_begin.is_some() {
                return (
                    page_will_begin.take().expect("Page event checked above"),
                    browser_will_begin
                        .take()
                        .expect("Browser event checked above"),
                );
            }
        }
    })
    .await
    .expect("receive Page.downloadWillBegin and Browser.downloadWillBegin");
    assert_eq!(page_will_begin["sessionId"], json!(session_id));
    assert_eq!(page_will_begin["params"]["frameId"], json!(target_id));
    assert_eq!(
        page_will_begin["params"]["guid"],
        browser_will_begin["params"]["guid"]
    );
    assert_eq!(
        page_will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );
    assert_eq!(browser_will_begin["params"]["frameId"], json!(target_id));
    assert_eq!(
        browser_will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );

    let (page_in_progress, browser_in_progress) = timeout(Duration::from_secs(2), async {
        let mut page_in_progress = None;
        let mut browser_in_progress = None;
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["params"]["state"] == json!("inProgress") {
                match message["method"].as_str() {
                    Some("Page.downloadProgress") => page_in_progress = Some(message),
                    Some("Browser.downloadProgress") => browser_in_progress = Some(message),
                    _ => {}
                }
            }
            if page_in_progress.is_some() && browser_in_progress.is_some() {
                return (
                    page_in_progress.take().expect("Page event checked above"),
                    browser_in_progress
                        .take()
                        .expect("Browser event checked above"),
                );
            }
        }
    })
    .await
    .expect("receive Page.downloadProgress and Browser.downloadProgress inProgress");
    assert_eq!(page_in_progress["sessionId"], json!(session_id));
    assert_eq!(
        page_in_progress["params"]["guid"],
        page_will_begin["params"]["guid"]
    );
    assert_eq!(
        browser_in_progress["params"]["guid"],
        browser_will_begin["params"]["guid"]
    );

    let (page_completed, browser_completed) = timeout(Duration::from_secs(2), async {
        let mut page_completed = None;
        let mut browser_completed = None;
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["params"]["state"] == json!("completed") {
                match message["method"].as_str() {
                    Some("Page.downloadProgress") => page_completed = Some(message),
                    Some("Browser.downloadProgress") => browser_completed = Some(message),
                    _ => {}
                }
            }
            if page_completed.is_some() && browser_completed.is_some() {
                return (
                    page_completed.take().expect("Page event checked above"),
                    browser_completed
                        .take()
                        .expect("Browser event checked above"),
                );
            }
        }
    })
    .await
    .expect("receive Page.downloadProgress and Browser.downloadProgress completed");
    assert_eq!(page_completed["sessionId"], json!(session_id));
    assert_eq!(
        page_completed["params"]["guid"],
        browser_completed["params"]["guid"]
    );
    assert!(
        page_completed["params"].get("filePath").is_none(),
        "Page.downloadProgress must not expose Browser.downloadProgress.filePath"
    );
    assert!(browser_completed["params"]["filePath"].is_string());
    let guid = browser_completed["params"]["guid"]
        .as_str()
        .expect("download guid should be present");
    let artifact_path = download_root.join(guid);
    let body =
        std::fs::read_to_string(&artifact_path).expect("download artifact should be written");
    assert_eq!(body, "download-body");

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_download_events_work_with_playwright_context_scoped_behavior() {
    let (fixture_addr, fixture_server) =
        spawn_delayed_download_fixture_server("download-body", Duration::from_millis(100)).await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-download-playwright-context-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "Target.setAutoAttach",
                "params": {
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send root Target.setAutoAttach");
    let _ = recv_until_id(&mut socket, 1).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send root Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 2).await;

    socket
        .send(WsMessage::Text(
            json!({ "id": 3_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 3).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "browserContextId": browser_context_id,
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send context Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target_messages = recv_until_id(&mut socket, 5).await;
    let target_id = create_target_messages
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();
    let session_id = create_target_messages
        .iter()
        .find(|message| message["method"] == json!("Target.attachedToTarget"))
        .and_then(|message| message["params"]["sessionId"].as_str())
        .expect("auto-attached sessionId")
        .to_owned();

    for (id, method) in [
        (6_u64, "Page.enable"),
        (7_u64, "Runtime.enable"),
        (8_u64, "Network.enable"),
        (9_u64, "Runtime.runIfWaitingForDebugger"),
    ] {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": session_id
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Playwright-style target init command");
        let _ = recv_until_id(&mut socket, id).await;
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    wait_for_session_navigation_response_and_load(&mut socket, 10, &session_id).await;
    let click_response = runtime_click_download_link(&mut socket, 11, &session_id).await;
    assert_eq!(
        runtime_click_response_value(&click_response, 11),
        Some("clicked")
    );

    let will_begin = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadWillBegin") {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadWillBegin");
    assert_eq!(will_begin["params"]["frameId"], json!(target_id));
    assert_eq!(
        will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress completed");
    let guid = completed["params"]["guid"]
        .as_str()
        .expect("download guid should be present");
    let artifact_path = download_root.join(guid);
    let body =
        std::fs::read_to_string(&artifact_path).expect("download artifact should be written");
    assert_eq!(body, "download-body");

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_page_download_events_do_not_require_browser_events_enabled() {
    let (fixture_addr, fixture_server) = spawn_delayed_content_disposition_download_fixture_server(
        "download-body",
        Duration::from_millis(100),
        "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.txt",
    )
    .await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-download-cd-async-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allow",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    wait_for_session_navigation_response_and_load(&mut socket, 6, &session_id).await;
    let click_response = runtime_click_download_link(&mut socket, 7, &session_id).await;
    assert_eq!(
        runtime_click_response_value(&click_response, 7),
        Some("clicked")
    );

    let will_begin = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            assert_ne!(
                message["method"],
                json!("Browser.downloadWillBegin"),
                "Browser events must remain disabled"
            );
            if message["method"] == json!("Page.downloadWillBegin") {
                return message;
            }
        }
    })
    .await
    .expect("receive Page.downloadWillBegin");
    assert_eq!(will_begin["sessionId"], json!(session_id));
    assert_eq!(will_begin["params"]["frameId"], json!(target_id));
    assert_eq!(will_begin["params"]["suggestedFilename"], json!("中文.txt"));

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            assert_ne!(
                message["method"],
                json!("Browser.downloadProgress"),
                "Browser events must remain disabled"
            );
            if message["method"] == json!("Page.downloadProgress")
                && message["params"]["state"] == json!("completed")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Page.downloadProgress completed");
    assert!(
        completed["params"].get("filePath").is_none(),
        "Page.downloadProgress must not expose Browser.downloadProgress.filePath"
    );
    let artifact_path = download_root.join("中文.txt");
    let body =
        std::fs::read_to_string(&artifact_path).expect("download artifact should be written");
    assert_eq!(body, "download-body");

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_plain_anchor_attachment_download_keeps_document() {
    let (fixture_addr, fixture_server) =
        spawn_delayed_plain_attachment_fixture_server("download-body", Duration::from_millis(100))
            .await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-plain-anchor-download-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Runtime.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.enable");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 6).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    wait_for_session_navigation_response_and_load(&mut socket, 7, &session_id).await;
    let click_response = runtime_click_download_link(&mut socket, 8, &session_id).await;
    assert_eq!(
        runtime_click_response_value(&click_response, 8),
        Some("clicked")
    );

    let will_begin = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadWillBegin") {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadWillBegin");
    assert_eq!(will_begin["params"]["frameId"], json!(target_id));
    assert_eq!(
        will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress completed");
    let guid = completed["params"]["guid"]
        .as_str()
        .expect("download guid should be present");
    let artifact_path = download_root.join(guid);
    let body =
        std::fs::read_to_string(&artifact_path).expect("download artifact should be written");
    assert_eq!(body, "download-body");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "location.pathname + '|' + document.getElementById('dl').id"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate = recv_until_id(&mut socket, 10).await;
    let evaluated_value = evaluate
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .and_then(|message| message["result"]["result"]["value"].as_str())
        .expect("Runtime.evaluate string result");
    assert_eq!(evaluated_value, "/page|dl");

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_page_navigate_direct_attachment_returns_is_download_and_keeps_document() {
    let (fixture_addr, fixture_server) =
        spawn_delayed_download_fixture_server("download-body", Duration::from_millis(100)).await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-direct-download-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Runtime.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.enable");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 6).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send initial Page.navigate");
    let mut saw_navigate_response = false;
    let mut saw_load_event = false;
    while !(saw_navigate_response && saw_load_event) {
        let message = recv_ws_json(&mut socket).await;
        if message["id"] == json!(7_u64) {
            saw_navigate_response = true;
        }
        if message["sessionId"] == json!(session_id)
            && message["method"] == json!("Page.loadEventFired")
        {
            saw_load_event = true;
        }
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/download")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send direct attachment Page.navigate");
    let direct_navigation_messages = recv_until_id(&mut socket, 8).await;
    let direct_navigation = direct_navigation_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("direct attachment Page.navigate response");
    assert_eq!(direct_navigation["result"]["frameId"], json!(target_id));
    assert_eq!(direct_navigation["result"]["isDownload"], json!(true));
    assert_eq!(
        direct_navigation["result"]["errorText"],
        json!("net::ERR_ABORTED")
    );
    assert!(
        direct_navigation["result"].get("loaderId").is_none(),
        "download navigate result should not expose loaderId: {direct_navigation:?}"
    );
    assert!(
        !direct_navigation_messages.iter().any(|message| {
            message["sessionId"] == json!(session_id)
                && message["method"] == json!("Page.loadEventFired")
        }),
        "download navigate should not fire Page.loadEventFired before returning the navigate result: {direct_navigation_messages:?}"
    );
    assert!(
        !direct_navigation_messages
            .iter()
            .any(|message| { message["method"] == json!("Browser.downloadWillBegin") }),
        "download events should be emitted after the navigate response: {direct_navigation_messages:?}"
    );

    let will_begin = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadWillBegin") {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadWillBegin");
    assert_eq!(will_begin["params"]["frameId"], json!(target_id));
    assert_eq!(
        will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress completed");
    let guid = completed["params"]["guid"]
        .as_str()
        .expect("download guid should be present");
    let artifact_path = download_root.join(guid);
    let body =
        std::fs::read_to_string(&artifact_path).expect("download artifact should be written");
    assert_eq!(body, "download-body");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "location.pathname + '|' + document.getElementById('dl').id"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate = recv_until_id(&mut socket, 9).await;
    let evaluated_value = evaluate
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .and_then(|message| message["result"]["result"]["value"].as_str())
        .expect("Runtime.evaluate string result");
    assert_eq!(evaluated_value, "/page|dl");

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_page_navigate_post_parse_location_download_keeps_document_and_emits_download_later()
 {
    let (fixture_addr, fixture_server) = spawn_post_parse_location_download_fixture_server(
        "download-body",
        Duration::from_millis(100),
    )
    .await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-post-parse-location-download-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Runtime.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.enable");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 6).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-parse download Page.navigate");
    let navigation_messages = recv_until_id(&mut socket, 7).await;
    let navigation = navigation_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("Page.navigate response");
    assert_eq!(navigation["result"]["frameId"], json!(target_id));
    assert!(
        navigation["result"].get("isDownload").is_none(),
        "post-parse location download should still return a loaded navigation result: {navigation:?}"
    );
    assert!(
        navigation["result"].get("loaderId").is_some(),
        "loaded navigation should still expose loaderId: {navigation:?}"
    );
    assert!(
        !navigation_messages
            .iter()
            .any(|message| { message["method"] == json!("Browser.downloadWillBegin") }),
        "download events should be emitted after the navigate response: {navigation_messages:?}"
    );
    let mut source_load_seen = navigation_messages.iter().any(|message| {
        message["sessionId"] == json!(session_id)
            && message["method"] == json!("Page.loadEventFired")
    });

    if !source_load_seen {
        let load_event = timeout(Duration::from_secs(2), async {
            loop {
                let message = recv_ws_json(&mut socket).await;
                assert!(
                    message["method"] != json!("Browser.downloadWillBegin"),
                    "download events must not overtake the source page load boundary: {message:?}; navigation_messages={navigation_messages:?}"
                );
                if message["sessionId"] == json!(session_id)
                    && message["method"] == json!("Page.loadEventFired")
                {
                    return message;
                }
            }
        })
        .await
        .expect("receive Page.loadEventFired before Browser.downloadWillBegin");
        assert_eq!(load_event["sessionId"], json!(session_id));
        source_load_seen = true;
    }
    assert!(
        source_load_seen,
        "source page load boundary should be observed"
    );

    let will_begin = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadWillBegin") {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadWillBegin");
    assert_eq!(will_begin["params"]["frameId"], json!(target_id));
    assert_eq!(
        will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress completed");
    let guid = completed["params"]["guid"]
        .as_str()
        .expect("download guid should be present");
    let artifact_path = download_root.join(guid);
    let body =
        std::fs::read_to_string(&artifact_path).expect("download artifact should be written");
    assert_eq!(body, "download-body");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Runtime.evaluate",
                "sessionId": session_id,
                "params": {
                    "expression": "location.pathname + '|' + document.getElementById('source').id"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Runtime.evaluate");
    let evaluate = recv_until_id(&mut socket, 8).await;
    let evaluated_value = evaluate
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .and_then(|message| message["result"]["result"]["value"].as_str())
        .expect("Runtime.evaluate string result");
    assert_eq!(evaluated_value, "/page|source");

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_cancel_download_does_not_resume_stale_page_observer() {
    let (fixture_addr, fixture_server) =
        spawn_delayed_download_fixture_server("download-body", Duration::from_secs(5)).await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-download-cancel-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createBrowserContext" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createBrowserContext");
    let create_browser_context = recv_until_id(&mut socket, 1).await;
    let browser_context_id = create_browser_context
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["browserContextId"].as_str())
        .expect("browserContextId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": browser_context_id,
                    "url": "about:blank"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 2).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 3).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(3_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Page.enable",
                "sessionId": session_id
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 5).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    wait_for_session_navigation_response_and_load(&mut socket, 6, &session_id).await;
    let click_response = runtime_click_download_link(&mut socket, 7, &session_id).await;
    assert_eq!(
        runtime_click_response_value(&click_response, 7),
        Some("clicked")
    );

    let will_begin = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadWillBegin") {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadWillBegin");
    assert_eq!(will_begin["params"]["frameId"], json!(target_id));
    let guid = will_begin["params"]["guid"]
        .as_str()
        .expect("download guid should be present")
        .to_owned();

    let in_progress = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("inProgress")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress inProgress");
    assert_eq!(in_progress["params"]["guid"], json!(guid.clone()));

    for (id, method) in [(8_u64, "Page.disable"), (9_u64, "Page.enable")] {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": method,
                    "sessionId": session_id
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send Page domain lifecycle command");
        let messages = recv_until_id(&mut socket, id).await;
        let response = messages
            .iter()
            .find(|message| message["id"] == json!(id))
            .expect("Page domain lifecycle response");
        assert_eq!(response["result"], json!({}));
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "Browser.cancelDownload",
                "params": { "guid": guid }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.cancelDownload");
    let cancel = recv_until_id(&mut socket, 10).await;
    let cancel_response = cancel
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .expect("cancel response");
    assert_eq!(cancel_response["result"], json!({}));

    let canceled = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            assert_ne!(
                message["method"],
                json!("Page.downloadProgress"),
                "re-enabled Page domain must not resume an old download observer"
            );
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("canceled")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress canceled");
    assert_eq!(canceled["params"]["guid"], json!(guid.clone()));
    assert!(
        !download_root.join(&guid).exists(),
        "canceled download should not write artifact"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "Browser.cancelDownload",
                "params": { "guid": guid }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.cancelDownload on terminated guid");
    let cancel_terminated = recv_until_id(&mut socket, 11).await;
    let cancel_terminated_response = cancel_terminated
        .iter()
        .find(|message| message["id"] == json!(11_u64))
        .expect("cancel terminated response");
    assert_eq!(cancel_terminated_response["error"]["code"], json!(-32602));
    assert_eq!(
        cancel_terminated_response["error"]["message"],
        json!("Download item is no longer active")
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "Browser.cancelDownload",
                "params": { "guid": "foo-no-such-guid" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.cancelDownload on invalid guid");
    let invalid = recv_until_id(&mut socket, 12).await;
    let invalid_response = invalid
        .iter()
        .find(|message| message["id"] == json!(12_u64))
        .expect("invalid guid response");
    assert_eq!(invalid_response["error"]["code"], json!(-32602));
    assert_eq!(
        invalid_response["error"]["message"],
        json!("No download item found for the given GUID")
    );

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_cdp_completed_download_can_be_opened_as_io_stream() {
    let (fixture_addr, fixture_server) =
        spawn_delayed_download_fixture_server("download-body", Duration::from_millis(100)).await;
    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-download-stream-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .expect("connect to cdp websocket");

    socket
        .send(WsMessage::Text(
            json!({ "id": 1_u64, "method": "Target.createTarget", "params": { "url": "about:blank" } })
                .to_string()
                .into(),
        ))
        .await
        .expect("send createTarget");
    let create_target = recv_until_id(&mut socket, 1).await;
    let target_id = create_target
        .iter()
        .find(|message| message["id"] == json!(1_u64))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("targetId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "Target.attachToTarget",
                "params": { "targetId": target_id, "flatten": true }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attachToTarget");
    let attach = recv_until_id(&mut socket, 2).await;
    let session_id = attach
        .iter()
        .find(|message| message["id"] == json!(2_u64))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .expect("sessionId")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({ "id": 3_u64, "method": "Page.enable", "sessionId": session_id })
                .to_string()
                .into(),
        ))
        .await
        .expect("send Page.enable");
    let _ = recv_until_id(&mut socket, 3).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "Browser.setDownloadBehavior",
                "params": {
                    "behavior": "allowAndName",
                    "downloadPath": download_root.to_string_lossy(),
                    "eventsEnabled": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.setDownloadBehavior");
    let _ = recv_until_id(&mut socket, 4).await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "Page.navigate",
                "sessionId": session_id,
                "params": {
                    "url": format!("http://{fixture_addr}/page")
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Page.navigate");
    wait_for_session_navigation_response_and_load(&mut socket, 5, &session_id).await;
    let click_response = runtime_click_download_link(&mut socket, 6, &session_id).await;
    assert_eq!(
        runtime_click_response_value(&click_response, 6),
        Some("clicked")
    );

    let will_begin = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadWillBegin") {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadWillBegin");
    let guid = will_begin["params"]["guid"]
        .as_str()
        .expect("download guid should be present")
        .to_owned();

    let _in_progress = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("inProgress")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress inProgress");

    let _completed = timeout(Duration::from_secs(2), async {
        loop {
            let message = recv_ws_json(&mut socket).await;
            if message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
            {
                return message;
            }
        }
    })
    .await
    .expect("receive Browser.downloadProgress completed");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "Browser.openDownloadAsStream",
                "params": { "guid": guid }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send Browser.openDownloadAsStream");
    let open_stream = recv_until_id(&mut socket, 8).await;
    let stream = open_stream
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .and_then(|message| message["result"]["stream"].as_str())
        .expect("stream handle should be returned")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "IO.read",
                "params": { "handle": stream, "size": 5 }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send IO.read first chunk");
    let first_chunk = recv_until_id(&mut socket, 9).await;
    let first_chunk_response = first_chunk
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("IO.read first chunk response");
    assert_eq!(
        first_chunk_response["result"]["base64Encoded"],
        json!(false)
    );
    assert_eq!(first_chunk_response["result"]["data"], json!("downl"));
    assert_eq!(first_chunk_response["result"]["eof"], json!(false));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "IO.read",
                "params": { "handle": stream }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send IO.read second chunk");
    let second_chunk = recv_until_id(&mut socket, 10).await;
    let second_chunk_response = second_chunk
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .expect("IO.read second chunk response");
    assert_eq!(
        second_chunk_response["result"]["base64Encoded"],
        json!(false)
    );
    assert_eq!(second_chunk_response["result"]["data"], json!("oad-body"));
    assert_eq!(second_chunk_response["result"]["eof"], json!(true));

    let _ = std::fs::remove_dir_all(&download_root);
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
