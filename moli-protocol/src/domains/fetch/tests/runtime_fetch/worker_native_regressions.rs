use super::fulfilled_redirects::command;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn worker_network_redirects_do_not_dispatch_another_service_worker_fetch_event() {
    let final_hits = Arc::new(AtomicUsize::new(0));
    let hits = final_hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/page", get(|| async { axum::response::Html("<!doctype html>worker SW redirects") }))
            .route("/sw.js", get(|| async { ([("content-type","text/javascript")], r#"
                oninstall = event => event.waitUntil(skipWaiting());
                onactivate = event => event.waitUntil(clients.claim());
                onfetch = event => {
                    const url = new URL(event.request.url);
                    if (url.pathname === '/controlled') event.respondWith(new Response('controlled'));
                    if (url.pathname === '/synthetic-start') event.respondWith(Response.redirect(url.origin + '/final', 302));
                    if (url.pathname === '/final') event.respondWith(new Response('SW final'));
                };
            "#) }))
            .route("/worker.js", get(|| async { ([("content-type","text/javascript")], r#"
                onmessage = async event => {
                    try { postMessage(await (await fetch(event.data)).text()); }
                    catch(error) { postMessage(error.name + ':' + error.message); }
                };
                postMessage('ready');
            "#) }))
            .route("/network-start",get(|| async { (axum::http::StatusCode::FOUND,[("location","/final")]) }))
            .route("/final",get(move || { let hits=hits.clone();async move {
                hits.fetch_add(1,Ordering::SeqCst);
                "HTTP final"
            } }))
        ).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
    ctx.observe_worker_network_for_test(None).await;
    enable_runtime_async(&mut ctx, "SID-1", 56_000).await;
    let controlled = command(&mut ctx,56_001,"Runtime.evaluate",json!({
        "expression":r#"(async()=>{
            await navigator.serviceWorker.register('/sw.js');
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller) await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange',resolve,{once:true}));
            return new Promise(resolve => {
                globalThis.nativeWorker = new Worker('/worker.js');
                nativeWorker.onmessage = event => resolve(event.data);
            });
        })()"#,
        "awaitPromise":true,"returnByValue":true
    })).await;
    assert_eq!(controlled["result"]["result"]["value"], "ready");
    for (index, (path, expected, hits)) in [
        ("/controlled", "controlled", 0),
        ("/network-start", "HTTP final", 1),
        ("/synthetic-start", "SW final", 1),
    ]
    .into_iter()
    .enumerate()
    {
        let response=command(&mut ctx,56_010+index as u64,"Runtime.evaluate",json!({
            "expression":format!("new Promise(resolve=>{{nativeWorker.onmessage=e=>resolve(e.data);nativeWorker.postMessage({path:?});}})"),
            "awaitPromise":true,"returnByValue":true
        })).await;
        assert_eq!(
            response["result"]["result"]["value"], expected,
            "{path}: {response}"
        );
        assert_eq!(final_hits.load(Ordering::SeqCst), hits, "{path}");
    }
    command(
        &mut ctx,
        56_020,
        "Runtime.evaluate",
        json!({"expression":"nativeWorker.terminate()"}),
    )
    .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_redirect_extra_info_reports_each_wire_request_and_sent_cookies() {
    check_worker_redirect_extra_info(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_worker_redirect_extra_info_reports_each_wire_request_and_sent_cookies() {
    check_worker_redirect_extra_info(true).await;
}

async fn check_worker_redirect_extra_info(shared: bool) {
    let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let capture = requests.clone();
    let final_capture = requests.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener,Router::new()
            .route("/page",get(||async{axum::response::Html("<!doctype html>worker ExtraInfo")}))
            .route("/worker.js",get(||async{([("content-type","text/javascript")],r#"
                async function receive(reply) {
                    const response = await fetch('/start',{method:'POST',body:'upload',headers:{'Content-Type':'text/plain'}});
                    reply(await response.text());
                }
                onmessage = () => receive(value=>postMessage(value));
                onconnect = event => {
                    const port = event.ports[0];
                    port.onmessage = () => receive(value=>port.postMessage(value));
                    port.start();port.postMessage('ready');
                };
                if(typeof SharedWorkerGlobalScope==='undefined'||!(self instanceof SharedWorkerGlobalScope)) postMessage('ready');
            "#)}))
            .route("/start",axum::routing::post(move |headers:axum::http::HeaderMap,body:bytes::Bytes|{
                let capture=capture.clone();async move{
                    capture.lock().await.push((headers,body));
                    (axum::http::StatusCode::SEE_OTHER,[("location","/final")])
                }
            }))
            .route("/final",get(move |headers:axum::http::HeaderMap,body:bytes::Bytes|{
                let capture=final_capture.clone();async move{
                    capture.lock().await.push((headers,body));
                    "done"
                }
            }))
        ).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
    ctx.observe_worker_network_for_test(None).await;
    command(&mut ctx, 56_098, "Network.enable", json!({})).await;
    enable_runtime_async(&mut ctx, "SID-1", 56_100).await;
    let ready=command(&mut ctx,56_101,"Runtime.evaluate",json!({
        "expression":format!("document.cookie='worker_probe=sent;Path=/';new Promise(resolve=>{{globalThis.nativeWorker=new {}('/worker.js');globalThis.nativePort={};nativePort.onmessage=e=>resolve(e.data);{};}})", if shared {"SharedWorker"} else {"Worker"}, if shared {"nativeWorker.port"} else {"nativeWorker"},if shared {"nativePort.start()"} else {"void 0"}),
        "awaitPromise":true,"returnByValue":true
    })).await;
    assert_eq!(ready["result"]["result"]["value"], "ready");
    let response=command(&mut ctx,56_102,"Runtime.evaluate",json!({
        "expression":"new Promise(resolve=>{nativePort.onmessage=e=>resolve(e.data);nativePort.postMessage('start');})",
        "awaitPromise":true,"returnByValue":true
    })).await;
    assert_eq!(response["result"]["result"]["value"], "done");
    let started = ctx
        .sent
        .iter()
        .find(|event| {
            event["method"] == "Network.requestWillBeSent"
                && event["params"]["request"]["url"] == format!("{origin}/start")
        })
        .unwrap();
    let extra_session = if shared {
        started["sessionId"].clone()
    } else {
        json!("SID-1")
    };
    let request_id = started["params"]["requestId"].clone();
    wait_until_scheduler_message(&mut ctx, "native redirect terminal", |event| {
        event["method"] == "Network.loadingFinished" && event["params"]["requestId"] == request_id
    })
    .await;
    let extras = ctx
        .sent
        .iter()
        .filter(|event| {
            event["method"] == "Network.requestWillBeSentExtraInfo"
                && event["params"]["requestId"] == request_id
        })
        .collect::<Vec<_>>();
    assert_eq!(extras.len(), 2, "{extras:?}");
    assert!(
        extras
            .iter()
            .all(|event| event["sessionId"] == extra_session),
        "request ExtraInfo must follow the loader owner: {extras:?}"
    );
    let response_extras = ctx
        .sent
        .iter()
        .filter(|event| {
            event["method"] == "Network.responseReceivedExtraInfo"
                && event["params"]["requestId"] == request_id
        })
        .collect::<Vec<_>>();
    assert_eq!(
        response_extras.len(),
        2,
        "response ExtraInfo for 303 and 200 without Set-Cookie: {response_extras:?}"
    );
    assert_eq!(response_extras[0]["params"]["statusCode"], 303);
    assert_eq!(response_extras[1]["params"]["statusCode"], 200);
    assert!(
        response_extras
            .iter()
            .all(|event| event["sessionId"] == extra_session)
    );
    let redirect = ctx
        .sent
        .iter()
        .find(|event| {
            event["method"] == "Network.requestWillBeSent"
                && event["params"]["requestId"] == request_id
                && event["params"].get("redirectResponse").is_some()
        })
        .expect("Worker redirect request");
    assert_eq!(redirect["params"]["redirectHasExtraInfo"], true);
    let header = |index: usize, name: &str| {
        extras[index]["params"]["headers"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    assert_eq!(header(0, "Content-Type"), Some(json!("text/plain")));
    assert_eq!(header(0, "Content-Length"), Some(json!("6")));
    assert_eq!(header(1, "Content-Type"), None);
    assert_eq!(header(1, "Content-Length"), None);
    for extra in extras {
        assert!(
            extra["params"]["associatedCookies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cookie| cookie["cookie"]["name"] == "worker_probe"
                    && cookie["blockedReasons"] == json!([])),
            "{extra}"
        );
    }
    let received = requests.lock().await;
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].1.as_ref(), b"upload");
    assert!(received[1].1.is_empty());
    assert!(received[0].0.contains_key("content-type"));
    assert!(!received[1].0.contains_key("content-type"));
    assert!(!received[1].0.contains_key("content-length"));
    command(
        &mut ctx,
        56_103,
        "Runtime.evaluate",
        json!({"expression":if shared {"nativeWorker.port.close()"} else {"nativeWorker.terminate()"}}),
    )
    .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_continue_response_preserves_wire_info_without_creating_a_redirect() {
    let next_hits = Arc::new(AtomicUsize::new(0));
    let hits = next_hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/page", get(|| async { axum::response::Html("<!doctype html>response provenance") }))
            .route("/worker.js", get(|| async { ([("content-type", "text/javascript")], r#"
                onmessage = async event => {
                    try {
                        const r = await fetch(event.data.url, {redirect: event.data.redirect || 'follow'});
                        postMessage({status:r.status, type:r.type, body:await r.text()});
                    } catch(error) { postMessage({error:error.name}); }
                };
                postMessage('ready');
            "#) }))
            .route("/probe", get(|| async { ([("content-type","text/plain"),("x-wire","original")], "network") }))
            .route("/empty", get(|| async { (axum::http::StatusCode::FOUND, [("location", "")], "empty-location-body") }))
            .route("/next", get(move || { let hits = hits.clone(); async move {
                hits.fetch_add(1, Ordering::SeqCst);
                "next"
            } }))
        ).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("{origin}/page"), "SID-1", "TID-1").await;
    ctx.observe_worker_network_for_test(None).await;
    enable_runtime_async(&mut ctx, "SID-1", 57_000).await;
    command(&mut ctx, 57_001, "Network.enable", json!({})).await;
    let ready = command(&mut ctx,57_002,"Runtime.evaluate",json!({
        "expression":"new Promise(resolve=>{globalThis.wireWorker=new Worker('/worker.js');wireWorker.onmessage=e=>resolve(e.data);})",
        "awaitPromise":true,"returnByValue":true
    })).await;
    assert_eq!(ready["result"]["result"]["value"], "ready");
    command(
        &mut ctx,
        57_003,
        "Fetch.enable",
        json!({"patterns":[{"urlPattern":"*/probe*","requestStage":"Response"}]}),
    )
    .await;
    for (index, fulfill) in [false, true].into_iter().enumerate() {
        let id = 57_010 + index as u64 * 10;
        let url = format!("{origin}/probe?fulfill={fulfill}");
        command(&mut ctx,id,"Runtime.evaluate",json!({"expression":format!(
            "globalThis.wireResult=new Promise(resolve=>{{wireWorker.onmessage=e=>resolve(e.data);wireWorker.postMessage({{url:{url:?}}});}});'started'"
        )})).await;
        let pause = wait_for_request_paused(&mut ctx, &url, "response provenance pause").await;
        let network_id = pause["params"]["networkId"].clone();
        wait_until_scheduler_message(
            &mut ctx,
            "wire ExtraInfo before continueResponse",
            |event| {
                event["method"] == "Network.responseReceivedExtraInfo"
                    && event["params"]["requestId"] == network_id
            },
        )
        .await;
        let raw = ctx
            .sent
            .iter()
            .find(|event| {
                event["method"] == "Network.responseReceivedExtraInfo"
                    && event["params"]["requestId"] == network_id
            })
            .unwrap();
        assert_eq!(raw["params"]["statusCode"], 200);
        assert_eq!(raw["params"]["headers"]["x-wire"], "original");
        let mut params = json!({"requestId":pause["params"]["requestId"],"responseCode":302,
            "responseHeaders":[{"name":"Location","value":"/next"},{"name":"X-Wire","value":"rewritten"}]
        });
        if fulfill {
            params["body"] = json!("c3ludGhldGlj");
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
                "expression":"wireResult","awaitPromise":true,"returnByValue":true
            }),
        )
        .await;
        assert_eq!(
            response["result"]["result"]["value"],
            if fulfill {
                json!({"status":200,"type":"basic","body":"next"})
            } else {
                json!({"status":302,"type":"basic","body":"network"})
            }
        );
        wait_until_scheduler_message(&mut ctx, "response provenance terminal", |event| {
            event["method"] == "Network.loadingFinished"
                && event["params"]["requestId"] == network_id
        })
        .await;
        assert_eq!(next_hits.load(Ordering::SeqCst), usize::from(fulfill));
        let extras = ctx
            .sent
            .iter()
            .filter(|e| {
                e["method"] == "Network.responseReceivedExtraInfo"
                    && e["params"]["requestId"] == network_id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            extras.len(),
            if fulfill { 2 } else { 1 },
            "one wire response fact per hop: {extras:?}"
        );
        assert!(
            extras.iter().all(|e| e["params"]["statusCode"] == 200),
            "CDP edits must never replace wire metadata: {extras:?}"
        );
    }
    command(&mut ctx, 57_040, "Fetch.disable", json!({})).await;
    for (index, redirect) in ["error", "follow", "manual"].into_iter().enumerate() {
        let response = command(&mut ctx,57_050+index as u64,"Runtime.evaluate",json!({
            "expression":format!("new Promise(resolve=>{{wireWorker.onmessage=e=>resolve(e.data);wireWorker.postMessage({{url:'/empty',redirect:{redirect:?}}});}})"),
            "awaitPromise":true,"returnByValue":true
        })).await;
        assert_eq!(
            response["result"]["result"]["value"],
            if redirect == "manual" {
                json!({"status":0,"type":"opaqueredirect","body":""})
            } else {
                json!({"status":302,"type":"basic","body":"empty-location-body"})
            }
        );
    }
    command(
        &mut ctx,
        57_060,
        "Runtime.evaluate",
        json!({"expression":"wireWorker.terminate()"}),
    )
    .await;
    server.abort();
}
