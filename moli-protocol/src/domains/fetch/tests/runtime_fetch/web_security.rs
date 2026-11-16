use super::fulfilled_redirects::{command, session_command};
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn worker_fetch_and_xhr_settle_intercepted_responses_with_their_instance_policy() {
    check_worker_intercepted_policy(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_fetch_and_xhr_settle_intercepted_responses_with_their_instance_policy() {
    check_worker_intercepted_policy(true).await;
}

async fn check_worker_intercepted_policy(shared: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/page", get(|| async { axum::response::Html("<!doctype html>worker security") }))
            .route("/worker.js", get(|| async { ([("content-type", "text/javascript")], r#"
                async function receive({data:{url,api}}, reply) {
                    if (api === 'xhr') {
                        const xhr = new XMLHttpRequest(); xhr.open('GET',url);
                        xhr.onload = () => reply({body:xhr.responseText,header:xhr.getResponseHeader('x-probe')});
                        xhr.onerror = () => reply({error:'NetworkError'}); xhr.send();
                    } else {
                        try { const response = await fetch(url); const copy=response.clone();
                            reply({body:await response.text(),header:response.headers.get('x-probe'),
                                type:response.type,clone:await copy.text()});
                        } catch(error) { reply({error:error.name}); }
                    }
                }
                onmessage = event => receive(event, value => postMessage(value));
                onconnect = ({ports:[port]}) => {
                    port.onmessage = event => receive(event, value => port.postMessage(value));
                    port.start(); port.postMessage('ready');
                };
                if (typeof SharedWorkerGlobalScope === 'undefined' || !(self instanceof SharedWorkerGlobalScope)) postMessage('ready');
            "#) }))
            .fallback(get(|| async { ([("x-probe","visible")],"network") })))
            .await.unwrap();
    });
    for disabled in [false, true] {
        let mut config = moli_fetch::FetchConfig::default();
        config.set_web_security_disabled(disabled);
        let mut ctx = TestContext::from_conn(crate::CdpConnection::new_with_fetch_config(config));
        with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
        ctx.observe_worker_network_for_test(None).await;
        enable_runtime_async(&mut ctx, "SID-1", 54_000).await;
        command(&mut ctx, 54_001, "Network.enable", json!({})).await;
        let ready = command(&mut ctx,54_002,"Runtime.evaluate",json!({
            "expression":format!("new Promise(resolve=>{{globalThis.policyWorker=new {}('/worker.js');globalThis.policyPort={};policyPort.onmessage=e=>resolve(e.data);{};}})",
                if shared {"SharedWorker"} else {"Worker"},
                if shared {"policyWorker.port"} else {"policyWorker"},
                if shared {"policyPort.start()"} else {"void 0"}),
            "awaitPromise":true,"returnByValue":true
        })).await;
        assert_eq!(ready["result"]["result"]["value"], "ready");
        let fetch_session = if shared {
            ctx.shared_worker_fetch_sessions_for_test().await[0].clone()
        } else {
            "SID-1".to_owned()
        };
        let mut index = 0;
        for api in ["fetch", "xhr"] {
            for (stage, action) in [
                ("Request", "continue"),
                ("Request", "fulfill"),
                ("Response", "continue"),
                ("Response", "fulfill"),
                ("Response", "fail"),
            ] {
                let fulfill = action == "fulfill";
                let failed = action == "fail";
                index += 1;
                let id = 54_010 + index * 5;
                let url = format!(
                    "{}/policy-probe?case={index}",
                    origin.replace("127.0.0.1", "localhost")
                );
                session_command(
                    &mut ctx,
                    &fetch_session,
                    id,
                    "Fetch.enable",
                    json!({"patterns":[{"urlPattern":"*policy-probe*","requestStage":stage}]}),
                )
                .await;
                command(&mut ctx,id+1,"Runtime.evaluate",json!({"expression":format!(
                    "globalThis.policyResult=new Promise(resolve=>{{policyPort.onmessage=e=>resolve(e.data);policyPort.postMessage({{url:{url:?},api:{api:?}}});}});'started'"
                )})).await;
                let pause = wait_for_request_paused_on_session(
                    &mut ctx,
                    &fetch_session,
                    &url,
                    None,
                    "worker policy pause",
                )
                .await;
                let mut params = json!({"requestId":pause["params"]["requestId"]});
                let method = if failed {
                    params["errorReason"] = json!("Aborted");
                    "Fetch.failRequest"
                } else if fulfill {
                    params["responseCode"] = json!(200);
                    params["responseHeaders"] = json!([{"name":"Content-Type","value":"text/plain"},{"name":"X-Probe","value":"replacement"}]);
                    params["body"] = json!("cmVwbGFjZW1lbnQ=");
                    "Fetch.fulfillRequest"
                } else if stage == "Request" {
                    "Fetch.continueRequest"
                } else {
                    "Fetch.continueResponse"
                };
                session_command(&mut ctx, &fetch_session, id + 2, method, params).await;
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
                if disabled && !failed {
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
                    terminals[0]["sessionId"],
                    ctx.worker_network_session_for_request(network_id)
                );
                assert_eq!(
                    terminals[0]["method"],
                    if disabled && !failed {
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
            json!({"expression":if shared {"policyPort.close()"} else {"policyWorker.terminate()"}}),
        )
        .await;
    }
    server.abort();
}
