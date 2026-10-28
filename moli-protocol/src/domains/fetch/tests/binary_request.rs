use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

const BODY: &[u8] = &[0, 255, 195, 40, 128, 65];

enum RequestScope {
    Window,
    Worker,
    SharedWorker,
}

async fn binary_request_round_trip(
    scope: RequestScope,
    xhr: bool,
    chained: bool,
    replacement: Option<&[u8]>,
    inspect_paused_body: bool,
) {
    let (sender, mut received) = tokio::sync::mpsc::unbounded_channel();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { "<!doctype html><title>binary request</title>" }),
                )
                .route(
                    "/echo",
                    any(move |body: axum::body::Bytes| {
                        let sender = sender.clone();
                        async move {
                            sender.send(body.to_vec()).unwrap();
                            axum::Json(body.to_vec())
                        }
                    }),
                ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    let page_url = format!("http://{addr}/page");
    let api_url = format!("http://{addr}/echo");
    with_loaded_http_document(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let mut sessions = vec!["SID-1"];
    if chained {
        ctx.with_fixture(|conn| {
            assert!({
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .assign_attached_session_to_target("TID-1", "SID-2".to_owned())
            })
        });
        sessions.push("SID-2");
    }
    ctx.sent.clear();
    let mut id = 1;
    for session in &sessions {
        for method in ["Network.enable", "Fetch.enable"] {
            ctx.process_async(json!({"id":id,"method":method,"sessionId":session,
                "params":{"patterns":[{"urlPattern":"*/echo","requestStage":"Request"}]}}))
                .await;
            ctx.expect_result(id, json!({}), Some(session));
            id += 1;
        }
    }
    ctx.process_async(json!({"id":id,"method":"Runtime.enable","sessionId":"SID-1"}))
        .await;
    ctx.expect_result(id, json!({}), Some("SID-1"));
    id += 1;
    let api = serde_json::to_string(&api_url).unwrap();
    let send = if xhr {
        format!(
            "new Promise((resolve,reject)=>{{const x=new XMLHttpRequest();x.open('POST',{api});x.onload=()=>resolve(JSON.parse(x.responseText));x.onerror=reject;x.send(bytes);}})"
        )
    } else {
        format!("fetch({api},{{method:'POST',body:bytes}}).then(r=>r.json())")
    };
    let bytes = format!("const bytes=new Uint8Array({});", json!(BODY));
    let on_message =
        "event => { globalThis.binaryResult=event.data; console.log('binary upload completed'); }";
    let script = match scope {
        RequestScope::Window => format!(
            "{bytes} {send}.then(value=>{{globalThis.binaryResult=value;console.log('binary upload completed');}});"
        ),
        RequestScope::Worker => {
            let source =
                serde_json::to_string(&format!("{bytes} {send}.then(value=>postMessage(value));"))
                    .unwrap();
            format!(
                "globalThis.binaryWorker=new Worker(URL.createObjectURL(new Blob([{source}],{{type:'text/javascript'}}))); binaryWorker.onmessage={on_message};"
            )
        }
        RequestScope::SharedWorker => {
            let source = serde_json::to_string(&format!(
                "onconnect=event=>{{const port=event.ports[0];{bytes} {send}.then(value=>port.postMessage(value));}};"
            ))
            .unwrap();
            format!(
                "globalThis.binaryWorker=new SharedWorker(URL.createObjectURL(new Blob([{source}],{{type:'text/javascript'}}))); binaryWorker.port.onmessage={on_message}; binaryWorker.port.start();"
            )
        }
    };
    ctx.process_async(
        json!({"id":id,"method":"Runtime.evaluate","sessionId":"SID-1",
        "params":{"expression":format!("globalThis.binaryResult=null; {script} 'scheduled'"),
        "returnByValue":true}}),
    )
    .await;
    let response = take_response_by_id(&mut ctx, id);
    assert_eq!(response["result"]["result"]["value"], "scheduled");
    id += 1;
    let mut network_id = Value::Null;
    for (index, session) in sessions.iter().enumerate() {
        let paused = runtime_fetch::wait_for_request_paused_on_session(
            &mut ctx,
            session,
            &api_url,
            None,
            "binary request pause",
        )
        .await;
        network_id = paused["params"]["networkId"].clone();
        assert!(
            received.try_recv().is_err(),
            "paused request must not reach the server"
        );
        if inspect_paused_body {
            ctx.process_async(json!({"id":id,"method":"Network.getRequestPostData",
                "sessionId":session,"params":{"requestId":network_id}}))
                .await;
            ctx.expect_result(
                id,
                json!({"postData":BASE64_STANDARD.encode(BODY),"base64Encoded":true}),
                Some(session),
            );
            id += 1;
        }
        let mut params = json!({"requestId":paused["params"]["requestId"]});
        if index == 0
            && let Some(replacement) = replacement
        {
            params["postData"] = json!(BASE64_STANDARD.encode(replacement));
        }
        ctx.process_async(json!({"id":id,"method":"Fetch.continueRequest",
            "sessionId":session,"params":params}))
            .await;
        ctx.expect_result(id, json!({}), Some(session));
        id += 1;
    }
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "binary upload completion",
        |messages| {
            messages.iter().any(|m| {
                m["method"] == "Network.loadingFinished" && m["params"]["requestId"] == network_id
            }) && messages.iter().any(|m| {
                m["method"] == "Runtime.consoleAPICalled"
                    && m["sessionId"] == "SID-1"
                    && m["params"]["args"][0]["value"] == "binary upload completed"
            })
        },
    )
    .await;
    let actual = received
        .try_recv()
        .expect("server must have consumed the body before completion");
    let expected = replacement.unwrap_or(BODY);
    assert_eq!(
        actual, expected,
        "Fetch.continueRequest must preserve the original bytes unless overridden"
    );
    assert!(
        received.try_recv().is_err(),
        "one continuation must send one POST"
    );
    ctx.process_async(
        json!({"id":id,"method":"Runtime.evaluate","sessionId":"SID-1",
        "params":{"expression":"globalThis.binaryResult","returnByValue":true}}),
    )
    .await;
    let response = take_response_by_id(&mut ctx, id);
    assert_eq!(response["result"]["result"]["value"], json!(expected));
    id += 1;
    ctx.process_async(
        json!({"id":id,"method":"Network.getRequestPostData","sessionId":"SID-1",
        "params":{"requestId":network_id}}),
    )
    .await;
    let (body, base64_encoded) = match std::str::from_utf8(expected) {
        Ok(body) => (body.to_owned(), false),
        Err(_) => (BASE64_STANDARD.encode(expected), true),
    };
    if expected.is_empty() {
        // Keep the existing Network API contract for an empty upload; the
        // server and JavaScript assertions above still require zero bytes.
        ctx.expect_error(id, -32000, "No post data available for the request");
    } else {
        ctx.expect_result(
            id,
            json!({"postData":body,"base64Encoded":base64_encoded}),
            Some("SID-1"),
        );
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::Window, false, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::Window, true, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_fetch_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::Window, false, true, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_fetch_text_override_replaces_binary_request_body() {
    binary_request_round_trip(
        RequestScope::Window,
        false,
        true,
        Some(b"replacement"),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn paused_fetch_exposes_binary_request_body() {
    binary_request_round_trip(RequestScope::Window, false, false, None, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, false, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_xhr_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, true, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_fetch_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, false, true, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_xhr_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, true, true, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_fetch_empty_override_replaces_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, false, true, Some(b""), false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_xhr_empty_override_replaces_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, true, true, Some(b""), false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_fetch_text_override_replaces_binary_request_body() {
    binary_request_round_trip(
        RequestScope::Worker,
        false,
        true,
        Some(b"replacement"),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_xhr_text_override_replaces_binary_request_body() {
    binary_request_round_trip(
        RequestScope::Worker,
        true,
        true,
        Some(b"replacement"),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn paused_worker_fetch_exposes_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, false, false, None, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn paused_worker_xhr_exposes_binary_request_body() {
    binary_request_round_trip(RequestScope::Worker, true, false, None, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_fetch_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::SharedWorker, false, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_xhr_noop_preserves_binary_request_body() {
    binary_request_round_trip(RequestScope::SharedWorker, true, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_fetch_binary_override_preserves_bytes() {
    binary_request_round_trip(
        RequestScope::Window,
        false,
        true,
        Some(&[0, 254, 192, 128, 65, 66]),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_xhr_binary_override_preserves_bytes() {
    binary_request_round_trip(
        RequestScope::Window,
        true,
        true,
        Some(&[0, 254, 192, 128, 65, 66]),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_fetch_binary_override_preserves_bytes() {
    binary_request_round_trip(
        RequestScope::Worker,
        false,
        true,
        Some(&[0, 254, 192, 128, 65, 66]),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_worker_xhr_binary_override_preserves_bytes() {
    binary_request_round_trip(
        RequestScope::Worker,
        true,
        true,
        Some(&[0, 254, 192, 128, 65, 66]),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_shared_worker_fetch_binary_override_preserves_bytes() {
    binary_request_round_trip(
        RequestScope::SharedWorker,
        false,
        true,
        Some(&[0, 254, 192, 128, 65, 66]),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_shared_worker_xhr_binary_override_preserves_bytes() {
    binary_request_round_trip(
        RequestScope::SharedWorker,
        true,
        true,
        Some(&[0, 254, 192, 128, 65, 66]),
        false,
    )
    .await;
}
