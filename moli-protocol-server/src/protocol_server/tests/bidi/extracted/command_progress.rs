use super::*;
use serde_json::Value;

async fn send_pending(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
    params: Value,
) {
    socket.send(WsMessage::Text(json!({"id": id, "method": method, "params": params, "goog:channel": format!("progress-{id}")}).to_string().into())).await.unwrap();
}

#[tokio::test]
async fn websocket_bidi_command_progress_keeps_two_runtime_receipts_and_channels() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, a) = bidi_session_with_context(addr).await;
    let b = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({"type":"tab", "background":true}),
    )
    .await["result"]["context"]
        .as_str()
        .unwrap()
        .to_owned();
    let subscribed = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({"events":["log.entryAdded"]}),
    )
    .await;
    assert_eq!(subscribed["type"], "success");
    let mut messages = Vec::new();
    for (id, context) in [(5, &a), (6, &b)] {
        send_pending(&mut socket, id, "script.evaluate", json!({"target":{"context":context}, "awaitPromise":true,
            "expression":format!("new Promise(resolve => {{ globalThis.progressResolve = resolve; console.log('waiting-{id}'); }})")})).await;
        messages = collect_bidi_messages_until(
            &mut socket,
            messages,
            |messages| {
                messages.iter().any(|message| {
                    message["method"] == "log.entryAdded"
                        && message["params"]["text"] == format!("waiting-{id}")
                })
            },
            "both original Promise bodies execute",
        )
        .await;
        assert!(
            !messages.iter().any(|message| message["id"] == id),
            "unresolved Promise: {messages:?}"
        );
    }
    send_pending(&mut socket, 7, "script.evaluate", json!({"target":{"context":b}, "expression":"progressResolve(29); true", "awaitPromise":true})).await;
    messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            [6, 7]
                .iter()
                .all(|id| messages.iter().any(|message| message["id"] == *id))
        },
        "B finishes before A",
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&messages, 6)["result"]["result"]["value"],
        29
    );
    assert!(!messages.iter().any(|message| message["id"] == 5));
    send_pending(
        &mut socket,
        8,
        "browsingContext.close",
        json!({"context":a}),
    )
    .await;
    messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            [5, 8]
                .iter()
                .all(|id| messages.iter().any(|message| message["id"] == *id))
        },
        "closing A settles only its remaining receipt",
    )
    .await;
    assert_eq!(bidi_message_by_id(&messages, 8)["type"], "success");
    send_pending(
        &mut socket,
        9,
        "script.evaluate",
        json!({"target":{"context":b}, "expression":"42", "awaitPromise":true}),
    )
    .await;
    messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| messages.iter().any(|message| message["id"] == 9),
        "surviving Page still executes",
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&messages, 9)["result"]["result"]["value"],
        42
    );
    for id in 5..=9 {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == id)
                .count(),
            1,
            "exactly one terminal for {id}: {messages:?}"
        );
        assert_eq!(
            bidi_message_by_id(&messages, id)["goog:channel"],
            format!("progress-{id}")
        );
    }
    socket.close(None).await.unwrap();
    server.abort();
}

async fn navigation_controls(wait: &str) {
    let arrived = Arc::new(tokio::sync::Notify::new());
    let (release, released) = tokio::sync::watch::channel(false);
    let fixture = Router::new().route(
        "/held",
        get({
            let arrived = Arc::clone(&arrived);
            move || {
                let arrived = Arc::clone(&arrived);
                let mut released = released.clone();
                async move {
                    arrived.notify_one();
                    released.wait_for(|released| *released).await.unwrap();
                    "<!doctype html><title>held</title>"
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/held", listener.local_addr().unwrap());
    let fixture = tokio::spawn(async move { axum::serve(listener, fixture).await });
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, a) = bidi_session_with_context(addr).await;
    send_pending(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({"context":a,"url":url,"wait":wait}),
    )
    .await;
    timeout(Duration::from_secs(2), arrived.notified())
        .await
        .expect("request is held on an actual HTTP response");
    send_pending(
        &mut socket,
        4,
        "browsingContext.close",
        json!({"context":a}),
    )
    .await;
    send_pending(&mut socket, 5, "browsingContext.getTree", json!({})).await;
    let messages = collect_bidi_messages_until(
        &mut socket,
        Vec::new(),
        |messages| {
            [3, 4, 5]
                .iter()
                .all(|id| messages.iter().any(|message| message["id"] == *id))
        },
        "navigation cancellation does not occupy the WebSocket handler",
    )
    .await;
    assert!(
        !*release.borrow(),
        "the HTTP gate must not rescue the socket"
    );
    for id in [4, 5] {
        assert_eq!(
            bidi_message_by_id(&messages, id)["type"],
            "success",
            "{messages:?}"
        );
    }
    assert!(
        bidi_message_by_id(&messages, 5)["result"]["contexts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|context| context["context"] != a)
    );
    for id in 3..=5 {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == id)
                .count(),
            1
        );
    }
    release.send_replace(true);
    socket.close(None).await.unwrap();
    server.abort();
    fixture.abort();
}

#[tokio::test]
async fn websocket_bidi_command_progress_complete_navigation_can_be_closed() {
    navigation_controls("complete").await;
}
#[tokio::test]
async fn websocket_bidi_command_progress_interactive_navigation_can_be_closed() {
    navigation_controls("interactive").await;
}

#[tokio::test]
async fn websocket_bidi_command_progress_release_interrupts_pause_without_future_key_state() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut socket, a) = bidi_session_with_context(addr).await;
    send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({"events":["log.entryAdded"],"contexts":[a]}),
    )
    .await;
    let setup=send_bidi_command(&mut socket,4,"script.evaluate",json!({"target":{"context":a},"awaitPromise":false,
        "expression":"document.body.innerHTML='<input>'; document.querySelector('input').focus(); globalThis.progressKeys=[]; for(const type of ['keydown','keyup']) addEventListener(type,e=>{progressKeys.push(type+':'+e.key);console.log(type+':'+e.key);}); true"})).await;
    assert_eq!(setup["type"], "success");
    send_pending(&mut socket,5,"input.performActions", json!({"context":a,"actions":[{"type":"key","id":"held","actions":[{"type":"keyDown","value":"\u{e008}"},{"type":"pause","duration":5000},{"type":"keyDown","value":"a"}]}]})).await;
    let mut messages = collect_bidi_messages_until(
        &mut socket,
        Vec::new(),
        |messages| {
            messages.iter().any(|message| {
                message["method"] == "log.entryAdded"
                    && message["params"]["text"] == "keydown:Shift"
            })
        },
        "the first tick has dispatched Shift",
    )
    .await;
    send_pending(&mut socket, 6, "input.releaseActions", json!({"context":a})).await;
    messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| messages.iter().any(|message| message["id"] == 6),
        "release completes while performActions stays paused",
    )
    .await;
    send_pending(&mut socket,7,"script.evaluate",json!({"target":{"context":a},"awaitPromise":false,"expression":"JSON.stringify(progressKeys)"})).await;
    messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            [6, 7]
                .iter()
                .all(|id| messages.iter().any(|message| message["id"] == *id))
        },
        "release and query complete during the real pause",
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&messages, 6)["type"],
        "success",
        "{messages:?}"
    );
    assert_eq!(
        bidi_message_by_id(&messages, 7)["result"]["result"]["value"],
        r#"["keydown:Shift","keyup:Shift"]"#
    );
    assert!(!messages.iter().any(|message| message["id"] == 5));
    send_pending(
        &mut socket,
        8,
        "browsingContext.close",
        json!({"context":a}),
    )
    .await;
    messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            [5, 8]
                .iter()
                .all(|id| messages.iter().any(|message| message["id"] == *id))
        },
        "closing cancels the remaining action ticks",
    )
    .await;
    assert_eq!(bidi_message_by_id(&messages, 8)["type"], "success");
    for id in 5..=8 {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == id)
                .count(),
            1
        );
    }
    socket.close(None).await.unwrap();
    server.abort();
}
