use super::fulfilled_redirects::command;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_and_xhr_settle_intercepted_responses_with_their_instance_policy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/page", get(|| async { axum::response::Html("<!doctype html>worker security") }))
            .route("/worker.js", get(|| async { ([("content-type", "text/javascript")], r#"
                onmessage = async ({data:{url,api}}) => {
                    if (api === 'xhr') {
                        const xhr = new XMLHttpRequest(); xhr.open('GET',url);
                        xhr.onload = () => postMessage({body:xhr.responseText,header:xhr.getResponseHeader('x-probe')});
                        xhr.onerror = () => postMessage({error:'NetworkError'}); xhr.send();
                    } else {
                        try { const response = await fetch(url); const copy=response.clone();
                            postMessage({body:await response.text(),header:response.headers.get('x-probe'),
                                type:response.type,clone:await copy.text()});
                        } catch(error) { postMessage({error:error.name}); }
                    }
                }; postMessage('ready');
            "#) }))
            .fallback(get(|| async { ([("x-probe","visible")],"network") })))
            .await.unwrap();
    });
    for disabled in [false, true] {
        let mut config = moli_fetch::FetchConfig::default();
        config.set_web_security_disabled(disabled);
        let mut ctx = TestContext::from_conn(crate::CdpConnection::new_with_fetch_config(config));
        with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
        enable_runtime_async(&mut ctx, "SID-1", 54_000).await;
        command(&mut ctx, 54_001, "Network.enable", json!({})).await;
        let ready = command(&mut ctx,54_002,"Runtime.evaluate",json!({
            "expression":"new Promise(resolve=>{globalThis.policyWorker=new Worker('/worker.js');policyWorker.onmessage=e=>resolve(e.data);})",
            "awaitPromise":true,"returnByValue":true
        })).await;
        assert_eq!(ready["result"]["result"]["value"], "ready");
        let mut index = 0;
        for api in ["fetch", "xhr"] {
            for (stage, fulfill) in [
                ("Request", false),
                ("Request", true),
                ("Response", false),
                ("Response", true),
            ] {
                index += 1;
                let id = 54_010 + index * 5;
                let url = format!(
                    "{}/policy-probe?case={index}",
                    origin.replace("127.0.0.1", "localhost")
                );
                command(
                    &mut ctx,
                    id,
                    "Fetch.enable",
                    json!({"patterns":[{"urlPattern":"*policy-probe*","requestStage":stage}]}),
                )
                .await;
                command(&mut ctx,id+1,"Runtime.evaluate",json!({"expression":format!(
                    "globalThis.policyResult=new Promise(resolve=>{{policyWorker.onmessage=e=>resolve(e.data);policyWorker.postMessage({{url:{url:?},api:{api:?}}});}});'started'"
                )})).await;
                let pause = wait_for_request_paused(&mut ctx, &url, "worker policy pause").await;
                let mut params = json!({"requestId":pause["params"]["requestId"]});
                let method = if fulfill {
                    params["responseCode"] = json!(200);
                    params["responseHeaders"] = json!([{"name":"Content-Type","value":"text/plain"},{"name":"X-Probe","value":"replacement"}]);
                    params["body"] = json!("cmVwbGFjZW1lbnQ=");
                    "Fetch.fulfillRequest"
                } else if stage == "Request" {
                    "Fetch.continueRequest"
                } else {
                    "Fetch.continueResponse"
                };
                command(&mut ctx, id + 2, method, params).await;
                let response = command(
                    &mut ctx,
                    id + 3,
                    "Runtime.evaluate",
                    json!({
                        "expression":"policyResult","awaitPromise":true,"returnByValue":true
                    }),
                )
                .await;
                let value = &response["result"]["result"]["value"];
                if disabled {
                    assert_eq!(
                        value["body"],
                        if fulfill { "replacement" } else { "network" },
                        "{api}/{stage}: {response}"
                    );
                    assert_eq!(
                        value["header"],
                        if fulfill { "replacement" } else { "visible" }
                    );
                    if api == "fetch" {
                        assert_eq!(value["type"], "basic");
                        assert_eq!(value["clone"], value["body"]);
                    }
                } else {
                    assert_eq!(
                        value["error"],
                        if api == "fetch" {
                            "TypeError"
                        } else {
                            "NetworkError"
                        },
                        "{response}"
                    );
                }
                let network_id = &pause["params"]["networkId"];
                wait_until_scheduler_message(&mut ctx, "worker policy terminal", |event| {
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
                assert_eq!(terminals.len(), 1, "{api}/{stage}: {terminals:?}");
                assert_eq!(
                    terminals[0]["method"],
                    if disabled {
                        "Network.loadingFinished"
                    } else {
                        "Network.loadingFailed"
                    }
                );
            }
        }
        command(
            &mut ctx,
            54_100,
            "Runtime.evaluate",
            json!({"expression":"policyWorker.terminate()"}),
        )
        .await;
    }
    server.abort();
}
