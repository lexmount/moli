use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn xhr_cors_decision_and_network_terminal_agree_in_every_delivery_mode() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { axum::response::Html("<!doctype html>ready") }),
                )
                .route(
                    "/cors",
                    get(|| async { ([("access-control-allow-origin", "*")], "body") }),
                ),
        )
        .await
        .unwrap();
    });
    for worker in [false, true] {
        for synchronous in [false, true] {
            for allowed in [false, true] {
                for decision in ["continue", "fulfill-request", "fulfill-response"] {
                    let mut ctx = TestContext::new();
                    with_loaded_http_document(
                        &mut ctx,
                        &format!("http://{addr}/page"),
                        "SID-1",
                        "TID-1",
                    )
                    .await;
                    enable_runtime_async(&mut ctx, "SID-1", 41400).await;
                    ctx.process_async(
                        json!({"id":41401,"method":"Network.enable","sessionId":"SID-1"}),
                    )
                    .await;
                    ctx.expect_result(41401, json!({}), Some("SID-1"));
                    let stage = if decision == "fulfill-request" {
                        "Request"
                    } else {
                        "Response"
                    };
                    ctx.process_async(json!({"id":41402,"method":"Fetch.enable","sessionId":"SID-1","params":{
                    "patterns":[{"urlPattern":"*/cors","resourceType":"XHR","requestStage":stage}]
                }})).await;
                    ctx.expect_result(41402, json!({}), Some("SID-1"));
                    let pauses = run_cdp_command_with_network_decisions(&mut ctx, json!({
                    "id":41403,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
                        "expression":xhr_expression(&format!("http://localhost:{}/cors",addr.port()),worker,synchronous),
                        "awaitPromise":true,"returnByValue":true
                    }
                }), |pause| {
                    let mut headers = vec![json!({"name":"content-type","value":"text/plain"})];
                    if allowed {
                        headers.push(json!({"name":"access-control-allow-origin","value":"*"}));
                    }
                    let method = if decision == "continue" { "Fetch.continueResponse" } else { "Fetch.fulfillRequest" };
                    let mut params = json!({
                        "requestId":pause["params"]["requestId"],"responseCode":200,"responseHeaders":headers
                    });
                    if decision != "continue" {
                        params["body"] = json!(BASE64_STANDARD.encode("body"));
                    }
                    json!({"id":41404,"method":method,"sessionId":"SID-1","params":params})
                }).await;
                    assert_eq!(pauses.len(), 1, "{pauses:?}");
                    let result = take_response_by_id(&mut ctx, 41403);
                    assert_eq!(
                        result["result"]["result"]["value"],
                        if allowed { "200:body" } else { "NetworkError" },
                        "worker={worker} synchronous={synchronous} allowed={allowed} decision={decision}: {result}"
                    );
                    let network_id = pauses[0]["params"]["networkId"].clone();
                    assert!(network_id.is_string(), "{pauses:?}");
                    let terminal = ctx
                        .wait_for_scheduler_message("XHR network terminal", |message| {
                            matches!(
                                message["method"].as_str(),
                                Some("Network.loadingFinished" | "Network.loadingFailed")
                            ) && message["params"]["requestId"] == network_id
                        })
                        .await;
                    assert_eq!(
                        terminal["method"],
                        if allowed {
                            "Network.loadingFinished"
                        } else {
                            "Network.loadingFailed"
                        },
                        "{terminal}"
                    );
                    ctx.process_async(json!({"id":41405,"method":"Runtime.evaluate","sessionId":"SID-1","params":{"expression":"0"}})).await;
                    assert!(
                        !ctx.sent.iter().any(|message| {
                            matches!(
                                message["method"].as_str(),
                                Some("Network.loadingFinished" | "Network.loadingFailed")
                            ) && message["params"]["requestId"] == network_id
                        }),
                        "duplicate or contradictory terminal: {:?}",
                        ctx.sent
                    );
                }
            }
        }
    }
    server.abort();
}
