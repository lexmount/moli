use super::*;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type Fields = BTreeMap<String, Box<RawValue>>;

fn fields(text: &str) -> Fields {
    serde_json::from_str(text).expect("valid raw Inspector envelope")
}

async fn receive_raw_until(
    socket: &mut Socket,
    mut matches: impl FnMut(&str) -> bool,
) -> Vec<String> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut messages = Vec::new();
        while let Some(message) = socket.next().await {
            if let WsMessage::Text(text) = message.expect("CDP frame") {
                let text = text.to_string();
                let matched = matches(&text);
                messages.push(text);
                if matched {
                    return messages;
                }
            }
        }
        panic!("CDP closed before matching response: {messages:?}");
    })
    .await
    .expect("raw Inspector output must complete")
}

async fn evaluate_raw(socket: &mut Socket, session: &str, id: u64, expression: &str) -> String {
    socket.send(WsMessage::Text(json!({"id":id,"method":"Runtime.evaluate","sessionId":session,"params":{"expression":expression,"returnByValue":true,"awaitPromise":true}}).to_string().into())).await.unwrap();
    receive_raw_until(socket, |text| {
        fields(text)
            .get("id")
            .is_some_and(|value| value.get() == id.to_string())
    })
    .await
    .pop()
    .unwrap()
}

#[tokio::test]
async fn websocket_cdp_raw_inspector_evaluation_preserves_lone_utf16_and_literals() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let session =
        cdp_create_default_session_and_navigate(&mut socket, "data:text/html,<p>ready</p>").await;
    for (index, (expression, expected)) in [
        ("String.fromCharCode(0xD800)", r#""\ud800""#),
        (
            "Promise.resolve(String.fromCharCode(0xDFFF))",
            r#""\udfff""#,
        ),
        (
            "new Promise(resolve => setTimeout(() => resolve(String.fromCharCode(0xD800)), 0))",
            r#""\ud800""#,
        ),
        (r"'\\ud800'", r#""\\ud800""#),
        (
            "JSON.stringify(String.fromCharCode(0xD800))",
            r#""\"\\ud800\"""#,
        ),
    ]
    .iter()
    .enumerate()
    {
        let response = evaluate_raw(&mut socket, &session, 100 + index as u64, expression).await;
        let envelope = fields(&response);
        assert_eq!(
            serde_json::from_str::<String>(envelope["sessionId"].get()).unwrap(),
            session
        );
        let result = fields(envelope["result"].get());
        let remote = fields(result["result"].get());
        assert_eq!(remote["value"].get(), *expected, "{expression}: {response}");
    }
    let response = evaluate_raw(&mut socket, &session, 200, "({high:String.fromCharCode(0xD800),low:String.fromCharCode(0xDFFF),pair:String.fromCharCode(0xD83D,0xDE00)})").await;
    let result = fields(fields(&response)["result"].get());
    let remote = fields(result["result"].get());
    let value = fields(remote["value"].get());
    assert_eq!(value["high"].get(), r#""\ud800""#);
    assert_eq!(value["low"].get(), r#""\udfff""#);
    assert_eq!(
        serde_json::from_str::<String>(value["pair"].get()).unwrap(),
        "😀"
    );
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_raw_inspector_console_preserves_utf16_and_event_order() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let session =
        cdp_create_default_session_and_navigate(&mut socket, "data:text/html,<p>ready</p>").await;
    send_cdp_command(&mut socket, 90, "Runtime.enable", Some(&session), json!({})).await;
    socket.send(WsMessage::Text(json!({"id":100,"method":"Runtime.evaluate","sessionId":session,"params":{"expression":"console.log(String.fromCharCode(0xD800)); 'done'","returnByValue":true}}).to_string().into())).await.unwrap();
    let messages = receive_raw_until(&mut socket, |text| {
        fields(text)
            .get("id")
            .is_some_and(|value| value.get() == "100")
    })
    .await;
    let console = messages
        .iter()
        .position(|text| {
            fields(text)
                .get("method")
                .is_some_and(|value| value.get() == r#""Runtime.consoleAPICalled""#)
        })
        .expect("console event before response");
    assert!(console < messages.len() - 1);
    let event = fields(&messages[console]);
    let params = fields(event["params"].get());
    let args: Vec<Box<RawValue>> = serde_json::from_str(params["args"].get()).unwrap();
    assert_eq!(fields(args[0].get())["value"].get(), r#""\ud800""#);
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_raw_inspector_nonflat_response_preserves_utf16() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let target = cdp_create_target_with_transport(&mut socket, 2, &context, false).await;
    let nested = json!({"id":101,"method":"Runtime.evaluate","params":{"expression":"Promise.resolve(String.fromCharCode(0xD800))","returnByValue":true,"awaitPromise":true}}).to_string();
    socket.send(WsMessage::Text(json!({"id":100,"method":"Target.sendMessageToTarget","params":{"sessionId":target.session_id,"message":nested}}).to_string().into())).await.unwrap();
    let mut acknowledged = false;
    let mut reply = None;
    receive_raw_until(&mut socket, |text| {
        let envelope = fields(text);
        acknowledged |= envelope.get("id").is_some_and(|id| id.get() == "100");
        if envelope
            .get("method")
            .is_some_and(|method| method.get() == r#""Target.receivedMessageFromTarget""#)
        {
            let outer: serde_json::Value = serde_json::from_str(text).unwrap();
            if outer["params"]["sessionId"] == target.session_id {
                let inner = outer["params"]["message"].as_str().unwrap();
                if fields(inner).get("id").is_some_and(|id| id.get() == "101") {
                    reply = Some(inner.to_owned());
                }
            }
        }
        acknowledged && reply.is_some()
    })
    .await;
    let envelope = fields(&reply.unwrap());
    assert!(!envelope.contains_key("sessionId"));
    let result = fields(envelope["result"].get());
    assert_eq!(fields(result["result"].get())["value"].get(), r#""\ud800""#);
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn websocket_cdp_raw_inspector_worker_response_preserves_utf16() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let session =
        cdp_create_default_session_and_navigate(&mut socket, "data:text/html,<p>ready</p>").await;
    send_cdp_command(
        &mut socket,
        90,
        "Target.setAutoAttach",
        Some(&session),
        json!({"autoAttach":true,"waitForDebuggerOnStart":false,"flatten":true}),
    )
    .await;
    let mut messages = send_cdp_command(&mut socket, 91, "Runtime.evaluate", Some(&session), json!({"expression":"globalThis.rawWorker = new Worker(URL.createObjectURL(new Blob(['onmessage=()=>postMessage(1)'],{type:'text/javascript'}))); 1","returnByValue":true})).await;
    let is_worker = |message: &serde_json::Value| {
        message["method"] == "Target.attachedToTarget"
            && message["params"]["targetInfo"]["type"] == "worker"
    };
    if !messages.iter().any(is_worker) {
        messages.extend(recv_until_match(&mut socket, is_worker).await);
    }
    let worker = messages.iter().find(|message| is_worker(message)).unwrap()["params"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    send_cdp_command(&mut socket, 92, "Runtime.enable", Some(&worker), json!({})).await;
    let response = evaluate_raw(&mut socket, &worker, 100, "String.fromCharCode(0xD800)").await;
    let envelope = fields(&response);
    assert_eq!(
        serde_json::from_str::<String>(envelope["sessionId"].get()).unwrap(),
        worker
    );
    let result = fields(envelope["result"].get());
    assert_eq!(fields(result["result"].get())["value"].get(), r#""\ud800""#);
    send_cdp_command(
        &mut socket,
        101,
        "Runtime.evaluate",
        Some(&session),
        json!({"expression":"rawWorker.terminate(); 1","returnByValue":true}),
    )
    .await;
    socket.close(None).await.unwrap();
    abort_test_cdp_server(server).await;
}
