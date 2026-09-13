use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

const BODY: &[u8] = &[0, 255, 195, 40, 128, 65];

async fn binary_request_round_trip(
    xhr: bool,
    chained: bool,
    replacement: Option<&str>,
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
        assert!(
            ctx.conn
                .browser_context
                .as_mut()
                .unwrap()
                .assign_attached_session_to_target("TID-1", "SID-2".to_owned())
        );
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
    let send = if xhr {
        "new Promise((resolve,reject)=>{const x=new XMLHttpRequest();x.open('POST','/echo');x.onload=()=>resolve(JSON.parse(x.responseText));x.onerror=reject;x.send(bytes);})"
    } else {
        "fetch('/echo',{method:'POST',body:bytes}).then(r=>r.json())"
    };
    ctx.process_async(json!({"id":id,"method":"Runtime.evaluate","sessionId":"SID-1",
        "params":{"expression":format!(
            "globalThis.binaryResult=null; const bytes=new Uint8Array({}); {send}.then(v=>globalThis.binaryResult=v); 'scheduled'",
            json!(BODY)),"returnByValue":true}})).await;
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
            })
        },
    )
    .await;
    let actual = received
        .try_recv()
        .expect("server must have consumed the body before completion");
    let expected = replacement.map(str::as_bytes).unwrap_or(BODY);
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
    let body = replacement.map_or_else(|| BASE64_STANDARD.encode(BODY), str::to_owned);
    ctx.expect_result(
        id,
        json!({"postData":body,"base64Encoded":replacement.is_none()}),
        Some("SID-1"),
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_noop_preserves_binary_request_body() {
    binary_request_round_trip(false, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn xhr_noop_preserves_binary_request_body() {
    binary_request_round_trip(true, false, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_fetch_noop_preserves_binary_request_body() {
    binary_request_round_trip(false, true, None, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn chained_fetch_text_override_replaces_binary_request_body() {
    binary_request_round_trip(false, true, Some("replacement"), false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn paused_fetch_exposes_binary_request_body() {
    binary_request_round_trip(false, false, None, true).await;
}
