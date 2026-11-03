use super::*;

#[derive(Clone, Copy)]
enum BodyRead {
    Stream,
    Buffered,
}

#[tokio::test]
async fn superseded_fetch_request_does_not_block_cancelled_replacement() {
    assert_superseded_interception("Request", false, None).await;
}

#[tokio::test]
async fn superseded_fetch_response_does_not_block_cancelled_replacement() {
    assert_superseded_interception("Response", false, None).await;
}

#[tokio::test]
async fn superseded_fetch_request_preserves_the_cancelled_replacements_debugger_pause() {
    assert_superseded_interception("Request", true, None).await;
}

#[tokio::test]
async fn superseded_fetch_response_preserves_the_cancelled_replacements_debugger_pause() {
    assert_superseded_interception("Response", true, None).await;
}

#[tokio::test]
async fn superseded_response_cancels_an_inflight_body_read_without_resurrecting_the_request() {
    assert_superseded_interception("Response", false, Some(BodyRead::Stream)).await;
}

#[tokio::test]
async fn superseded_get_response_body_retires_navigation_without_resurrecting_the_request() {
    assert_superseded_interception("Response", false, Some(BodyRead::Buffered)).await;
}

#[tokio::test]
async fn superseded_get_response_body_preserves_the_cancelled_replacements_debugger_pause() {
    assert_superseded_interception("Response", true, Some(BodyRead::Buffered)).await;
}

async fn assert_superseded_interception(
    stage: &str,
    debugger_paused: bool,
    body_read: Option<BodyRead>,
) {
    use axum::response::IntoResponse;

    let (fixture_addr, fixture) = spawn_dedicated_fixture_server(
        Router::new().route(
            "/",
            get(move || async move {
                if body_read.is_some() {
                    // Send the response head, but never finish the body. Only
                    // navigation cancellation can release the pending read.
                    let stream = futures_util::stream::once(std::future::pending::<
                        Result<axum::body::Bytes, std::io::Error>,
                    >());
                    axum::body::Body::from_stream(stream).into_response()
                } else {
                    "<title>replacement</title>".into_response()
                }
            }),
        ),
        "superseded-interception",
    );
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(
        discover_browser_websocket_url(cdp_addr)
            .await
            .expect("discover browser endpoint"),
    )
    .await
    .unwrap();
    let session_id =
        cdp_create_default_session_and_navigate(&mut socket, "data:text/html,<title>old</title>")
            .await;
    send_cdp_command(
        &mut socket,
        6,
        "Debugger.enable",
        Some(&session_id),
        json!({}),
    )
    .await;
    send_cdp_command(
        &mut socket,
        7,
        "Fetch.enable",
        Some(&session_id),
        json!({"patterns": [{"urlPattern": "*", "resourceType": "Document", "requestStage": stage}]}),
    )
    .await;
    if debugger_paused {
        pause_in_timer(
            &mut socket,
            &session_id,
            8,
            "debugger; globalThis.resumedNormally = true;",
        )
        .await;
    }

    let mut messages = Vec::new();
    let mut requests = Vec::new();
    for id in [9, 10] {
        send_cdp_command_without_wait(
            &mut socket,
            id,
            "Page.navigate",
            Some(&session_id),
            json!({"url": format!("http://{fixture_addr}/?navigation={id}")}),
        )
        .await;
        let paused = recv_until_match(&mut socket, |message| {
            message["method"] == "Fetch.requestPaused"
        })
        .await;
        requests.push(paused.last().unwrap()["params"]["requestId"].clone());
        messages.extend(paused);
        if let Some(body_read) = body_read
            && id == 9
        {
            if matches!(body_read, BodyRead::Buffered) {
                send_cdp_command_without_wait(
                    &mut socket,
                    16,
                    "Fetch.getResponseBody",
                    Some(&session_id),
                    json!({"requestId": requests[0]}),
                )
                .await;
                continue;
            }
            let opened = send_cdp_command(
                &mut socket,
                15,
                "Fetch.takeResponseBodyAsStream",
                Some(&session_id),
                json!({"requestId": requests[0]}),
            )
            .await;
            let handle =
                opened.iter().find(|message| message["id"] == 15).unwrap()["result"]["stream"]
                    .clone();
            assert!(handle.is_string(), "body stream should open: {opened:#?}");
            messages.extend(opened);
            send_cdp_command_without_wait(
                &mut socket,
                16,
                "IO.read",
                Some(&session_id),
                json!({"handle": handle}),
            )
            .await;
        }
    }

    // Leave A entirely alone. Only B is cancelled, and the old document must
    // become usable without a client action on A (including Fetch.disable).
    messages.extend(
        send_cdp_command(
            &mut socket,
            11,
            "Fetch.failRequest",
            Some(&session_id),
            json!({"requestId": requests[1], "errorReason": "Aborted"}),
        )
        .await,
    );
    if debugger_paused {
        messages.extend(
            send_cdp_command(
                &mut socket,
                12,
                "Debugger.resume",
                Some(&session_id),
                json!({}),
            )
            .await,
        );
    }
    send_cdp_command_without_wait(
        &mut socket,
        13,
        "Runtime.evaluate",
        Some(&session_id),
        json!({"expression": "document.title + ':' + globalThis.resumedNormally", "returnByValue": true}),
    )
    .await;
    let completion = timeout(NAVIGATION_TIMEOUT, async {
        while ![9, 13]
            .into_iter()
            .all(|id| messages.iter().any(|message| message["id"] == id))
            || (body_read.is_some() && !messages.iter().any(|message| message["id"] == 16))
        {
            messages.push(recv_ws_json(&mut socket).await);
        }
    })
    .await;

    // Always clean up before asserting so a failure cannot strand the old
    // document's owner thread in a debugger pause or suspended channel.
    messages.extend(
        send_cdp_command(
            &mut socket,
            14,
            "Fetch.failRequest",
            Some(&session_id),
            json!({"requestId": requests[0], "errorReason": "Aborted"}),
        )
        .await,
    );
    let _ = socket.close(None).await;
    abort_test_cdp_server(protocol_server).await;
    drop(fixture);

    assert!(
        completion.is_ok(),
        "stale A must retire and leave the old document usable: {messages:#?}"
    );
    for id in [9, 10] {
        let replies = messages
            .iter()
            .filter(|message| message["id"] == id)
            .collect::<Vec<_>>();
        assert_eq!(
            replies.len(),
            1,
            "each navigation needs exactly one terminal reply: {messages:#?}"
        );
        if id == 9 {
            assert!(replies[0].get("error").is_none());
            assert_eq!(replies[0]["result"]["errorText"], "net::ERR_ABORTED");
        } else {
            assert_eq!(replies[0]["error"]["message"], "Aborted");
        }
    }
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == "Page.frameNavigated")
    );
    let value =
        &messages.iter().find(|message| message["id"] == 13).unwrap()["result"]["result"]["value"];
    assert_eq!(
        value,
        if debugger_paused {
            "old:true"
        } else {
            "old:undefined"
        }
    );
    if debugger_paused {
        assert!(
            messages
                .iter()
                .any(|message| message["id"] == 12 && message.get("error").is_none()),
            "supersession and cancellation must not terminate the original pause: {messages:#?}"
        );
    }
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == 14 && message.get("error").is_some()),
        "the retired interception must no longer accept client actions: {messages:#?}"
    );
    if let Some(body_read) = body_read {
        let expected_error = match body_read {
            BodyRead::Stream => "StreamHandleNotFound",
            BodyRead::Buffered => "RequestNotFound",
        };
        assert!(
            messages.iter().any(|message| message["id"] == 16
                && message["error"]["message"] == expected_error),
            "the cancelled read must complete without restoring the old response: {messages:#?}"
        );
    }
}
