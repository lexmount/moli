use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

mod http_errors;
mod stylesheets;

async fn intercepted_parser_script_has_one_network_terminal(
    action: &str,
    failed: bool,
    count: usize,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(move || async move {
                        (
                            [("content-type", "text/html")],
                            format!(
                                "<!doctype html>{}<p>loaded</p>",
                                "<script src='/script.js'></script>".repeat(count)
                            ),
                        )
                    }),
                )
                .route(
                    "/script.js",
                    get(|| async {
                        (
                            [("content-type", "text/javascript")],
                            "globalThis.scriptLoaded = true;",
                        )
                    }),
                ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    ctx.with_fixture(|conn| {
        conn.install_browser_context_fixture_for_test(attached_browser_context());
    });
    ctx.process_async(json!({
        "id": 1, "sessionId": "SID-1", "method": "Network.enable",
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 2, "sessionId": "SID-1", "method": "Fetch.enable",
        "params": {"patterns": [{"resourceType": "Script", "urlPattern": "*"}]},
    }))
    .await;
    ctx.expect_result(2, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 3, "sessionId": "SID-1", "method": "Page.navigate",
        "params": {"url": format!("http://{addr}/page")},
    }))
    .await;

    let script_url = format!("http://{addr}/script.js");
    let mut request_ids = Vec::new();
    for _ in 0..count {
        wait_until_scheduler_message(&mut ctx, "parser script Fetch pause", |message| {
            message["method"] == "Fetch.requestPaused"
                && message["params"]["request"]["url"] == script_url
                && !request_ids.contains(&message["params"]["networkId"])
        })
        .await;
        let paused = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == "Fetch.requestPaused"
                    && message["params"]["request"]["url"] == script_url
                    && !request_ids.contains(&message["params"]["networkId"])
            })
            .unwrap()
            .clone();
        network_request_announced_before_fetch_pause(&ctx, &paused, Some("Script"));
        request_ids.push(paused["params"]["networkId"].clone());
        let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();
        let mut params = json!({"requestId": paused["params"]["requestId"]});
        match action {
            "Fetch.fulfillRequest" => {
                params["responseCode"] = json!(200);
                params["responseHeaders"] =
                    json!([{"name": "Content-Type", "value": "text/javascript"}]);
                params["body"] = json!(STANDARD.encode(b"globalThis.scriptLoaded = true;"));
            }
            "Fetch.failRequest" => params["errorReason"] = json!("BlockedByClient"),
            "Fetch.disable" => params = json!({}),
            "Fetch.continueRequest" if failed => {
                // Stop the listener before resuming so transport failure is deterministic.
                server.abort();
                let _ = (&mut server).await;
            }
            "Fetch.continueRequest" => {}
            _ => panic!("unsupported test action"),
        }
        ctx.process_async(json!({
            "id": 4, "sessionId": "SID-1", "method": action, "params": params,
        }))
        .await;
        ctx.expect_result(4, json!({}), Some("SID-1"));

        // Only drive real completion inputs: an extra Runtime.evaluate could mask
        // missing autonomous delivery, and a different request's terminal cannot
        // remove this request from Playwright's inflight set.
        let terminal = if failed {
            "Network.loadingFailed"
        } else {
            "Network.loadingFinished"
        };
        wait_until_scheduler_message(&mut ctx, terminal, |message| {
            message["method"] == terminal && message["params"]["requestId"] == network_id
        })
        .await;
    }
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["url"] == script_url
        })
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        count,
        "completion must reuse the paused request: {requests:?}"
    );
    for network_id in request_ids {
        let terminals = ctx
            .sent
            .iter()
            .filter(|message| {
                matches!(
                    message["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && message["params"]["requestId"] == network_id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            terminals.len(),
            1,
            "one terminal per paused request: {terminals:?}"
        );
        if action == "Fetch.failRequest" {
            assert_eq!(
                terminals[0]["params"]["errorText"],
                "net::ERR_BLOCKED_BY_CLIENT"
            );
        }
        if !failed {
            let responses = ctx
                .sent
                .iter()
                .filter(|message| {
                    message["method"] == "Network.responseReceived"
                        && message["params"]["requestId"] == network_id
                })
                .collect::<Vec<_>>();
            assert_eq!(
                responses.len(),
                1,
                "one response per paused request: {responses:?}"
            );
            let response_index = ctx
                .sent
                .iter()
                .position(|message| *message == *responses[0])
                .unwrap();
            let terminal_index = ctx
                .sent
                .iter()
                .position(|message| *message == *terminals[0])
                .unwrap();
            assert!(
                response_index < terminal_index,
                "response must precede its terminal"
            );
        }
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_parser_script_finishes_the_paused_network_request() {
    intercepted_parser_script_has_one_network_terminal("Fetch.fulfillRequest", false, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn continued_parser_script_finishes_the_paused_network_request() {
    intercepted_parser_script_has_one_network_terminal("Fetch.continueRequest", false, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_parser_script_finishes_the_paused_network_request() {
    intercepted_parser_script_has_one_network_terminal("Fetch.failRequest", true, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn continued_parser_script_transport_failure_finishes_the_paused_network_request() {
    intercepted_parser_script_has_one_network_terminal("Fetch.continueRequest", true, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_fetch_finishes_the_paused_parser_script_network_request() {
    intercepted_parser_script_has_one_network_terminal("Fetch.disable", false, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_url_parser_scripts_finish_distinct_paused_network_requests() {
    intercepted_parser_script_has_one_network_terminal("Fetch.fulfillRequest", false, 2).await;
}
