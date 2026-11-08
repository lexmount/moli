use super::*;
use moli_protocol::version;
use serde_json::Value;

fn command_result(messages: &[Value], id: u64) -> Value {
    let response = messages.iter().find(|message| message["id"] == id).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    response["result"].clone()
}

async fn discovery_version(addr: std::net::SocketAddr) -> Value {
    let (status, body) = fetch_server_response(addr, "GET", "/json/version").await;
    assert_eq!(status, 200);
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn version_metadata_matches_discovery_and_cdp() {
    for user_agent in [FetchConfig::DEFAULT_USER_AGENT, "MoliStartup/2.0"] {
        let mut fetch = FetchConfig::default();
        fetch.set_user_agent(user_agent);
        let (addr, server) = spawn_test_protocol_server_with_fetch_config(fetch).await;
        let discovery = discovery_version(addr).await;
        let (mut socket, _) = connect_async(discovery["webSocketDebuggerUrl"].as_str().unwrap())
            .await
            .unwrap();
        let messages =
            send_cdp_command(&mut socket, 1, "Browser.getVersion", None, json!({})).await;
        let browser = command_result(&messages, 1);
        assert_eq!(
            browser,
            json!({
                "protocolVersion": version::PROTOCOL_VERSION,
                "product": version::PRODUCT,
                "moliVersion": version::MOLI_VERSION,
                "revision": version::REVISION,
                "userAgent": user_agent,
                "jsVersion": version::js_version(),
            }),
        );
        assert_eq!(
            discovery,
            json!({
                "Browser": browser["product"],
                "Protocol-Version": browser["protocolVersion"],
                "User-Agent": browser["userAgent"],
                "V8-Version": browser["jsVersion"],
                "WebKit-Version": format!("537.36 ({})", browser["revision"].as_str().unwrap()),
                "File-Path-Style": if std::path::MAIN_SEPARATOR == '\\' { "windows" } else { "posix" },
                "Moli-Version": browser["moliVersion"],
                "webSocketDebuggerUrl": discovery["webSocketDebuggerUrl"],
            }),
        );
        socket.close(None).await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn version_metadata_ignores_page_overrides_and_target_switches() {
    let mut fetch = FetchConfig::default();
    fetch.set_user_agent("BrowserStartup/3.0");
    let (addr, server) = spawn_test_protocol_server_with_fetch_config(fetch).await;
    let discovery = discovery_version(addr).await;
    let (mut socket, _) = connect_async(discovery["webSocketDebuggerUrl"].as_str().unwrap())
        .await
        .unwrap();
    let messages = send_cdp_command(&mut socket, 1, "Browser.getVersion", None, json!({})).await;
    let browser = command_result(&messages, 1);
    assert_eq!(browser["userAgent"], "BrowserStartup/3.0");

    let mut targets = Vec::new();
    for (index, user_agent) in ["PageA/4.0", "PageB/5.0"].into_iter().enumerate() {
        let id = 10 + index as u64 * 10;
        let context = cdp_create_browser_context(&mut socket, id).await;
        let target = cdp_create_attached_target(&mut socket, id + 1, &context).await;
        let messages = send_cdp_command(
            &mut socket,
            id + 3,
            "Network.setUserAgentOverride",
            Some(&target.session_id),
            json!({ "userAgent": user_agent }),
        )
        .await;
        assert_eq!(command_result(&messages, id + 3), json!({}));
        let messages = send_cdp_command(
            &mut socket,
            id + 4,
            "Runtime.evaluate",
            Some(&target.session_id),
            json!({ "expression": "navigator.userAgent", "returnByValue": true }),
        )
        .await;
        assert_eq!(
            command_result(&messages, id + 4)["result"]["value"],
            user_agent
        );
        targets.push(target);
    }

    for (index, target) in [&targets[0], &targets[1], &targets[0]]
        .into_iter()
        .enumerate()
    {
        let id = 40 + index as u64 * 10;
        let messages = send_cdp_command(
            &mut socket,
            id,
            "Target.activateTarget",
            None,
            json!({ "targetId": target.target_id }),
        )
        .await;
        assert_eq!(command_result(&messages, id), json!({}));
        for (offset, session) in [(1, None), (2, Some(target.session_id.as_str()))] {
            let messages = send_cdp_command(
                &mut socket,
                id + offset,
                "Browser.getVersion",
                session,
                json!({}),
            )
            .await;
            assert_eq!(command_result(&messages, id + offset), browser);
        }
        assert_eq!(discovery_version(addr).await, discovery);
    }
    socket.close(None).await.unwrap();
    server.abort();
}
