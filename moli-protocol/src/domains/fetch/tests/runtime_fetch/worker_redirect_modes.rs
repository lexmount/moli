use super::fulfilled_redirects::command;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn fulfilled_worker_redirect_modes_are_checked_after_response_security() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { axum::response::Html("<!doctype html>worker redirects") }),
                )
                .route(
                    "/worker.js",
                    get(|| async {
                        (
                            [("content-type", "text/javascript")],
                            r#"
                onmessage = async event => {
                    const {url, redirect, mode} = event.data;
                    try {
                        const response = await fetch(url, {redirect, mode});
                        const clone = response.clone();
                        postMessage({type:response.type,status:response.status,url:response.url,
                            redirected:response.redirected,bodyNull:response.body===null,
                            header:response.headers.get('x-probe'),body:await response.text(),
                            cloneType:clone.type,cloneBody:await clone.text()});
                    } catch(error) { postMessage({error:error.name,message:error.message}); }
                };
                postMessage('ready');
            "#,
                        )
                    }),
                )
                .fallback(get(|| async {
                    ([("access-control-allow-origin", "*")], "network")
                })),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
    ctx.observe_worker_network_for_test(None).await;
    enable_runtime_async(&mut ctx, "SID-1", 53_000).await;
    command(&mut ctx, 53_001, "Network.enable", json!({})).await;
    let ready = command(&mut ctx, 53_002, "Runtime.evaluate", json!({
        "expression":"new Promise(resolve=>{globalThis.redirectWorker=new Worker('/worker.js');redirectWorker.onmessage=event=>resolve(event.data);})",
        "awaitPromise":true,"returnByValue":true
    })).await;
    assert_eq!(ready["result"]["result"]["value"], "ready");
    let mut cases = Vec::new();
    for status in [200, 301, 302, 303, 307, 308] {
        for mode in ["error", "manual", "follow"] {
            cases.push((status, mode, "cors", false));
        }
    }
    cases.extend([
        (302, "error", "cors", true),
        (302, "manual", "cors", true),
        (302, "error", "no-cors", true),
    ]);
    let mut index = 0;
    for (stage, fulfill) in [("Request", true), ("Response", true), ("Response", false)] {
        command(
            &mut ctx,
            53_003,
            "Fetch.enable",
            json!({"patterns":[{
                "urlPattern":"*worker-probe*","requestStage":stage
            }]}),
        )
        .await;
        for ((status, redirect, mode, foreign), empty_location) in cases
            .iter()
            .flat_map(|case| [(*case, false), (*case, true)])
        {
            let id = 53_010 + index * 4;
            index += 1;
            let base = if foreign {
                origin.replace("127.0.0.1", "localhost")
            } else {
                origin.clone()
            };
            let url = format!("{base}/worker-probe?case={index}");
            command(&mut ctx, id, "Runtime.evaluate", json!({"expression":format!(
                "globalThis.workerRedirectResult=new Promise(resolve=>{{redirectWorker.onmessage=event=>resolve(event.data);redirectWorker.postMessage({{url:{url:?},redirect:{redirect:?},mode:{mode:?}}});}});'started'"
            )})).await;
            if foreign && mode == "no-cors" {
                let response = command(
                    &mut ctx,
                    id + 2,
                    "Runtime.evaluate",
                    json!({
                        "expression":"workerRedirectResult","awaitPromise":true,"returnByValue":true
                    }),
                )
                .await;
                assert_eq!(response["result"]["result"]["value"]["error"], "TypeError");
                assert!(
                    !ctx.sent.iter().any(|event| {
                        event["method"] == "Fetch.requestPaused"
                            && event["params"]["request"]["url"] == url
                    }),
                    "invalid cross-origin no-cors/error request must fail before interception"
                );
                continue;
            }
            let pause = wait_for_request_paused(&mut ctx, &url, "worker redirect pause").await;
            let mut params = json!({
                "requestId":pause["params"]["requestId"],"responseCode":status,
                "responseHeaders":[{"name":"Content-Type","value":if mode=="no-cors" {"application/json"} else {"text/plain"}},{"name":"X-Probe","value":"visible"}]
            });
            if empty_location {
                params["responseHeaders"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name":"Location","value":""}));
            }
            if fulfill {
                params["body"] = json!("Ym9keQ==");
            }
            command(
                &mut ctx,
                id + 1,
                if fulfill {
                    "Fetch.fulfillRequest"
                } else {
                    "Fetch.continueResponse"
                },
                params,
            )
            .await;
            let response = command(
                &mut ctx,
                id + 2,
                "Runtime.evaluate",
                json!({
                    "expression":"workerRedirectResult","awaitPromise":true,"returnByValue":true
                }),
            )
            .await;
            let value = &response["result"]["result"]["value"];
            let failure = foreign && mode == "cors";
            if failure {
                assert_eq!(
                    value["error"], "TypeError",
                    "{stage}/{status}/{redirect}/{mode}: {response}"
                );
                if foreign && mode == "cors" {
                    assert!(
                        value["message"].as_str().unwrap().contains("CORS"),
                        "{value}"
                    );
                }
            } else if status != 200 && redirect == "manual" {
                assert_eq!(
                    value,
                    &json!({"type":"opaqueredirect","status":0,"url":url,"redirected":false,
                    "bodyNull":true,"header":null,"body":"","cloneType":"opaqueredirect","cloneBody":""})
                );
            } else {
                let body = if fulfill { "body" } else { "network" };
                assert_eq!(
                    value,
                    &json!({"type":"basic","status":status,"url":url,"redirected":false,
                        "bodyNull":false,"header":"visible","body":body,"cloneType":"basic","cloneBody":body})
                );
            }
            let network_id = &pause["params"]["networkId"];
            wait_until_scheduler_message(&mut ctx, "worker redirect terminal", |event| {
                matches!(
                    event["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && &event["params"]["requestId"] == network_id
            })
            .await;
            let terminals = ctx
                .sent
                .iter()
                .filter(|event| {
                    matches!(
                        event["method"].as_str(),
                        Some("Network.loadingFinished" | "Network.loadingFailed")
                    ) && &event["params"]["requestId"] == network_id
                })
                .collect::<Vec<_>>();
            assert_eq!(terminals.len(), 1, "{terminals:?}");
            assert_eq!(
                terminals[0]["method"],
                if failure {
                    "Network.loadingFailed"
                } else {
                    "Network.loadingFinished"
                },
                "{stage}/fulfill={fulfill}/{status}/{redirect}/{mode}"
            );
            if foreign && mode == "cors" {
                assert_eq!(terminals[0]["params"]["errorText"], "net::ERR_FAILED");
            }
        }
    }
    command(
        &mut ctx,
        53_500,
        "Runtime.evaluate",
        json!({"expression":"redirectWorker.terminate();'terminated'"}),
    )
    .await;
    server.abort();
}
