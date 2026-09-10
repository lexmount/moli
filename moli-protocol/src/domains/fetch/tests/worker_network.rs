use super::*;
use moli_core::browser::{BrowserEvent, NetworkOwner, NetworkRequestState, WorkerHandle};
use moli_core::page::{RendererNetworkOutputItem, ScriptNetworkOutputItem};

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_fetch_without_worker_attachment() {
    worker_fetch_without_attachment(false, false, UnattachedWorkerFinish::Fulfill).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_fetch_without_worker_attachment() {
    worker_fetch_without_attachment(true, false, UnattachedWorkerFinish::Fulfill).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_fetch_retirement_without_worker_attachment() {
    worker_fetch_without_attachment(false, false, UnattachedWorkerFinish::Retire).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_fetch_retirement_without_worker_attachment() {
    worker_fetch_without_attachment(true, false, UnattachedWorkerFinish::Retire).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_fetch_retirement_snapshot_without_worker_attachment() {
    worker_fetch_without_attachment(false, false, UnattachedWorkerFinish::RetireSnapshot).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_fetch_retirement_snapshot_without_worker_attachment() {
    worker_fetch_without_attachment(true, false, UnattachedWorkerFinish::RetireSnapshot).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_dedicated_fetch_without_worker_attachment() {
    worker_fetch_without_attachment(false, true, UnattachedWorkerFinish::Fulfill).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_ancestor_fetch_without_worker_attachment() {
    worker_fetch_without_attachment(true, true, UnattachedWorkerFinish::Fulfill).await;
}

enum UnattachedWorkerFinish {
    Fulfill,
    Retire,
    RetireSnapshot,
}

async fn worker_fetch_without_attachment(
    shared: bool,
    nested: bool,
    finish: UnattachedWorkerFinish,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/page", get(|| async { ([(CONTENT_TYPE.as_str(), "text/html")], "<!doctype html>") }))
            .route("/worker.js", get(|| async { ([(CONTENT_TYPE.as_str(), "text/javascript")],
                "async function run(reply){const r=await fetch('/body');reply(await r.text());}onmessage=()=>run(postMessage);onconnect=e=>{const p=e.ports[0];p.onmessage=()=>run(v=>p.postMessage(v));p.start();p.postMessage('ready');};if(typeof postMessage==='function')postMessage('ready');") }))
        ).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(
        &mut ctx,
        &format!("http://{address}/page"),
        "SID-1",
        "TID-1",
    )
    .await;
    enable_runtime_async(&mut ctx, "SID-1", 1).await;
    ctx.process_async(json!({"id":2,"method":"Fetch.enable","sessionId":"SID-1","params":{"patterns":[{"urlPattern":"*/body"}]}})).await;
    ctx.expect_result(2, json!({}), Some("SID-1"));
    let (_, mut native) = ctx.conn.subscribe_browser_events().unwrap();
    let worker_script = if nested {
        let child = format!(
            "data:text/javascript,onmessage=()=>fetch('http://{address}/body').then(r=>r.text()).then(postMessage);postMessage('ready');"
        );
        format!(
            "data:text/javascript,function bridge(port){{const child=new Worker({child:?});child.onmessage=e=>port.postMessage(e.data);port.onmessage=e=>child.postMessage(e.data);if(port.start)port.start();}};onconnect=e=>bridge(e.ports[0]);if(typeof postMessage==='function')bridge(globalThis);"
        )
    } else {
        "/worker.js".into()
    };
    // Deliberately do not discover/attach Worker targets or enable their
    // Network domains. The Page supplies policy, not the Worker executor.
    ctx.process_and_wait_for_response_async(json!({"id":3,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":format!("globalThis.result=new Promise(done=>{{const worker=new {}({});const port=worker.port||worker;port.onmessage=e=>{{if(e.data==='ready')port.postMessage('fetch');else done(e.data);}};if(worker.port)port.start();}});'started'", if shared { "SharedWorker" } else { "Worker" }, json!(worker_script)),
        "returnByValue":true,
    }})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 3)["result"]["result"]["value"],
        "started"
    );
    let url = format!("http://{address}/body");
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "Worker pause without attachment",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == "Fetch.requestPaused"
                    && message["params"]["request"]["url"] == url
            })
        },
    )
    .await;
    let paused = ctx.take_first_matching("unattached Worker pause", |message| {
        message["method"] == "Fetch.requestPaused" && message["params"]["request"]["url"] == url
    });
    assert_eq!(paused["sessionId"], "SID-1");
    if !matches!(finish, UnattachedWorkerFinish::Fulfill) {
        let pause = ctx.conn.subscribe_browser_events().unwrap().0.worker_fetch_pauses.into_iter().find(|pause| {
            matches!(pause.pause.stage(), moli_core::page::RendererWorkerFetchStage::Request(info) if info.url.as_str() == url)
        }).expect("the observed stage must still belong to its physical Worker");
        assert!(ctx.conn.observes_worker_fetch_pause(&pause));
        let context_id = ctx
            .conn
            .browser_context_by_browser_id(pause.document.web_contents().context())
            .unwrap()
            .id
            .clone();
        assert!(
            ctx.conn
                .native_worker_network_owner(&context_id, pause.pause.worker())
                .is_some()
        );
        assert!(match pause.worker {
            WorkerHandle::Dedicated { context, instance } => ctx
                .conn
                .close_browser_dedicated_worker(context, instance)
                .unwrap(),
            WorkerHandle::Shared { context, instance } => ctx
                .conn
                .close_browser_shared_worker(context, instance)
                .unwrap(),
            WorkerHandle::Service { .. } => unreachable!(),
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if matches!(native.recv().await.unwrap().event, BrowserEvent::WorkerDestroyed(worker) if worker == pause.worker) {
                    break;
                }
            }
        }).await.expect("Worker retirement must not need a Protocol consumer");
        assert!(!pause.pause.is_available());
        if matches!(finish, UnattachedWorkerFinish::RetireSnapshot) {
            let snapshot = ctx.conn.subscribe_browser_events().unwrap().0;
            assert!(snapshot.worker_fetch_pauses.is_empty());
            ctx.conn.project_browser_snapshot(snapshot).await;
        } else {
            ctx.wait_until_scheduler_state("the exact Worker retirement", |conn| {
                conn.native_worker_network_owner(&context_id, pause.pause.worker())
                    .is_none()
            })
            .await;
        }
        assert!(
            !ctx.conn.observes_worker_fetch_pause(&pause),
            "retiring a Worker must remove Page observer correlation without a Network terminal"
        );
        assert!(
            !ctx.conn
                .browser_context_by_id(&context_id)
                .unwrap()
                .page_targets
                .get("TID-1")
                .unwrap()
                .fetch_owner
                .has_pending_fetch_state_for_test()
        );
        // Consume a late FIFO in the snapshot case too: neither a late
        // projection nor the old wire request id may revive retired work.
        ctx.process_and_wait_for_response_async(json!({"id":4,"method":"Fetch.continueRequest","sessionId":"SID-1","params":{"requestId":paused["params"]["requestId"]}})).await;
        assert_eq!(take_response_by_id(&mut ctx, 4)["error"]["code"], -32000);
        assert!(!ctx.conn.observes_worker_fetch_pause(&pause));
        server.abort();
        return;
    }
    ctx.process_and_wait_for_response_async(json!({"id":4,"method":"Fetch.fulfillRequest","sessionId":"SID-1","params":{
        "requestId":paused["params"]["requestId"],"responseCode":200,
        "responseHeaders":[{"name":"Access-Control-Allow-Origin","value":"*"}],
        "body":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "worker-owned"),
    }})).await;
    ctx.expect_result(4, json!({}), Some("SID-1"));
    ctx.process_and_wait_for_response_async(
        json!({"id":5,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
            "expression":"result","awaitPromise":true,"returnByValue":true,
        }}),
    )
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 5)["result"]["result"]["value"],
        "worker-owned"
    );
    let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::NetworkRequestCompleted(occurrence) =
                native.recv().await.unwrap().event
                && let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
                && let ScriptNetworkOutputItem::SubresourceNetworkRecord(record) = item.as_ref()
                && record.url().as_str() == url
            {
                break occurrence;
            }
        }
    })
    .await
    .expect("unattached Worker must publish its own terminal fact");
    assert!(matches!(completed.owner, NetworkOwner::Worker(_)));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == "Target.attachedToTarget")
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["params"]["requestId"] == paused["params"]["networkId"]
                && message["method"]
                    .as_str()
                    .is_some_and(|method| method.starts_with("Network."))
        }),
        "Page must not inherit unattached Worker Network events: {:?}",
        ctx.sent
    );
    server.abort();
}

#[derive(Clone, Copy, Debug)]
enum Resource {
    Fetch,
    Xhr,
    CspReport,
}

#[derive(Clone, Copy, Debug)]
enum Decision {
    RequestFail,
    RequestAbort,
    ResponseAbort,
    RequestFulfill,
    RequestFulfillBinary,
    RequestFulfillCorsBlocked,
    AuthCancel,
    AuthProvide,
    ResponseContinue,
    ResponseFail,
    ResponseFulfill,
}

macro_rules! worker_network_cases {
    ($shared:literal, $resource:ident, $($name:ident: $decision:ident),+ $(,)?) => {
        $(#[tokio::test(flavor = "multi_thread")]
        async fn $name() {
            assert_worker_network_owner($shared, Resource::$resource, Decision::$decision).await;
        })+
    };
}

worker_network_cases!(false, Fetch,
    dedicated_fetch_request_fail: RequestFail,
    dedicated_fetch_request_abort: RequestAbort,
    dedicated_fetch_response_abort: ResponseAbort,
    dedicated_fetch_request_fulfill: RequestFulfill,
    dedicated_fetch_binary_fulfill: RequestFulfillBinary,
    dedicated_fetch_cors_fulfill: RequestFulfillCorsBlocked,
    dedicated_fetch_auth_cancel: AuthCancel,
    dedicated_fetch_auth_provide: AuthProvide,
    dedicated_fetch_response_continue: ResponseContinue,
    dedicated_fetch_response_fail: ResponseFail,
    dedicated_fetch_response_fulfill: ResponseFulfill,
);
worker_network_cases!(false, Xhr,
    dedicated_xhr_request_fail: RequestFail,
    dedicated_xhr_request_abort: RequestAbort,
    dedicated_xhr_response_abort: ResponseAbort,
    dedicated_xhr_request_fulfill: RequestFulfill,
    dedicated_xhr_binary_fulfill: RequestFulfillBinary,
    dedicated_xhr_cors_fulfill: RequestFulfillCorsBlocked,
    dedicated_xhr_auth_cancel: AuthCancel,
    dedicated_xhr_auth_provide: AuthProvide,
    dedicated_xhr_response_continue: ResponseContinue,
    dedicated_xhr_response_fail: ResponseFail,
    dedicated_xhr_response_fulfill: ResponseFulfill,
);
worker_network_cases!(true, Fetch,
    shared_fetch_request_fail: RequestFail,
    shared_fetch_request_abort: RequestAbort,
    shared_fetch_response_abort: ResponseAbort,
    shared_fetch_request_fulfill: RequestFulfill,
    shared_fetch_binary_fulfill: RequestFulfillBinary,
    shared_fetch_cors_fulfill: RequestFulfillCorsBlocked,
    shared_fetch_auth_cancel: AuthCancel,
    shared_fetch_auth_provide: AuthProvide,
    shared_fetch_response_continue: ResponseContinue,
    shared_fetch_response_fail: ResponseFail,
    shared_fetch_response_fulfill: ResponseFulfill,
);
worker_network_cases!(true, Xhr,
    shared_xhr_request_fail: RequestFail,
    shared_xhr_request_abort: RequestAbort,
    shared_xhr_response_abort: ResponseAbort,
    shared_xhr_request_fulfill: RequestFulfill,
    shared_xhr_binary_fulfill: RequestFulfillBinary,
    shared_xhr_cors_fulfill: RequestFulfillCorsBlocked,
    shared_xhr_auth_cancel: AuthCancel,
    shared_xhr_auth_provide: AuthProvide,
    shared_xhr_response_continue: ResponseContinue,
    shared_xhr_response_fail: ResponseFail,
    shared_xhr_response_fulfill: ResponseFulfill,
);

worker_network_cases!(false, CspReport,
    dedicated_csp_request_fail: RequestFail,
    dedicated_csp_request_fulfill: RequestFulfill,
);
worker_network_cases!(true, CspReport,
    shared_csp_request_fail: RequestFail,
    shared_csp_request_fulfill: RequestFulfill,
);

async fn assert_worker_network_owner(shared: bool, resource_kind: Resource, decision: Decision) {
    let csp_report = matches!(resource_kind, Resource::CspReport);
    let binary = matches!(decision, Decision::RequestFulfillBinary);
    let request_stage = matches!(
        decision,
        Decision::RequestFail
            | Decision::RequestFulfill
            | Decision::RequestFulfillBinary
            | Decision::RequestFulfillCorsBlocked
            | Decision::RequestAbort
    );
    async fn worker(uri: axum::http::Uri) -> impl IntoResponse {
        (
            [
                (CONTENT_TYPE.as_str(), "text/javascript"),
                (
                    "content-security-policy",
                    if uri.query() == Some("csp") {
                        "connect-src 'none'; report-uri /worker-owned"
                    } else {
                        "script-src 'self'"
                    },
                ),
            ],
            r#"
async function run(data, reply) {
  if (data.abort) { globalThis.abortRequest(); return; }
  if (data.resource === 'csp') {
    await fetch('/blocked-data').catch(() => {});
    reply({reported:true});
  } else if (data.resource === 'xhr') {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', data.url);
    xhr.setRequestHeader('x-worker-request', 'original');
    xhr.onload = () => reply({status:xhr.status, text:xhr.responseText, url:xhr.responseURL});
    xhr.onerror = () => reply({error:'xhr failed'});
    xhr.onabort = () => reply({error:'xhr aborted'});
    globalThis.abortRequest = () => xhr.abort();
    xhr.send(data.binary ? new Uint8Array([0, 128, 255]) : 'worker-payload');
  } else {
    const controller = new AbortController();
    globalThis.abortRequest = () => controller.abort();
    try {
      const response = await fetch(data.url, {method:'POST', body:data.binary ? new Uint8Array([0, 128, 255]) : 'worker-payload', signal:controller.signal,
        headers:{'x-worker-request':'original'}});
      reply({status:response.status, text:await response.text(), url:response.url});
    } catch (error) { reply({error:String(error)}); }
  }
}
onmessage = event => run(event.data, value => postMessage(value));
onconnect = event => {
  const port = event.ports[0];
  port.onmessage = event => run(event.data, value => port.postMessage(value));
  port.start();
  port.postMessage('ready');
};
if (typeof postMessage === 'function') postMessage('ready');
"#,
        )
    }
    async fn resource(
        uri: axum::http::Uri,
        headers: HeaderMap,
        body: String,
    ) -> axum::response::Response {
        match headers
            .get("x-worker-request")
            .and_then(|header| header.to_str().ok())
        {
            Some("original") => assert_eq!(body, "worker-payload"),
            Some("intercepted") => assert_eq!(body, "intercepted-payload"),
            _ => assert!(serde_json::from_str::<Value>(&body).unwrap()["csp-report"].is_object()),
        }
        if uri.query() == Some("auth") && !headers.contains_key("authorization") {
            (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE.as_str(), "Basic realm=\"worker-owned\"")],
                "auth-required",
            )
                .into_response()
        } else {
            if uri.query() == Some("auth") {
                assert_eq!(headers["authorization"], "Basic d29ya2VyOnNlY3JldA==");
            }
            ([(CONTENT_TYPE.as_str(), "text/plain")], "server-body").into_response()
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/page",
                    get(|| async { ([(CONTENT_TYPE.as_str(), "text/html")], "<!doctype html>") }),
                )
                .route("/worker.js", get(worker))
                .route("/worker-owned", any(resource)),
        )
        .await
        .unwrap();
    });
    let worker_path = if csp_report {
        "/worker.js?csp"
    } else {
        "/worker.js"
    };
    let worker_url = format!("http://{addr}{worker_path}");
    let mut url = format!(
        "http://{addr}/worker-owned{}",
        if matches!(decision, Decision::AuthCancel | Decision::AuthProvide) {
            "?auth"
        } else {
            ""
        }
    );
    if matches!(decision, Decision::RequestFulfillCorsBlocked) {
        url = url.replace("127.0.0.1", "localhost");
    }
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 1).await;
    if !shared {
        // A new Document restarts its local wrapper IDs, not the Context's
        // physical Worker IDs. Never let equality in the first Document mask
        // an incorrect Fetch-to-Network identity conversion.
        ctx.process_and_wait_for_response_async(json!({"id":900,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
            "expression":"new Promise(resolve => { const worker = new Worker('/worker.js?warmup'); worker.onmessage = () => { worker.terminate(); resolve(true); }; })",
            "awaitPromise":true,"returnByValue":true,
        }})).await;
        let warmup = take_response_by_id(&mut ctx, 900);
        assert_eq!(warmup["result"]["result"]["value"], true, "{warmup:?}");
        ctx.process_async(json!({"id":901,"method":"Page.enable","sessionId":"SID-1"}))
            .await;
        ctx.expect_result(901, json!({}), Some("SID-1"));
        ctx.sent.clear();
        ctx.process_async(json!({"id":902,"method":"Page.navigate","sessionId":"SID-1","params":{"url":format!("http://{addr}/page?replacement")}})).await;
        let navigation = take_response_by_id(&mut ctx, 902);
        assert_eq!(navigation["result"]["frameId"], "TID-1", "{navigation:?}");
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    }
    for (id, method, params, session) in [
        (
            2,
            "Target.setDiscoverTargets",
            json!({"discover":true}),
            None,
        ),
        (3, "Network.enable", json!({}), Some("SID-1")),
        (
            4,
            "Fetch.enable",
            json!({"handleAuthRequests":true,
            "patterns":[{"urlPattern":"*/worker-owned*","requestStage":"Request"}]}),
            Some("SID-1"),
        ),
    ] {
        ctx.process_async(json!({"id":id,"method":method,"params":params,"sessionId":session}))
            .await;
        ctx.expect_result(id, json!({}), session);
    }
    ctx.process_and_wait_for_response_async(json!({"id":5,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":format!(r#"new Promise(resolve => {{
          const worker = new {}({});
          globalThis.worker = worker;
          globalThis.workerResult = new Promise(done => {{ globalThis.workerDone = done; }});
          const channel = worker.port || worker;
          channel.onmessage = event => {{ if (event.data === 'ready') resolve(true); else workerDone(event.data); }};
          if (worker.port) worker.port.start();
        }})"#, if shared { "SharedWorker" } else { "Worker" }, json!(worker_path)),
        "awaitPromise":true,"returnByValue":true,
    }})).await;
    let ready = take_response_by_id(&mut ctx, 5);
    assert_eq!(ready["result"]["result"]["value"], true, "{ready:?}");
    let target_type = if shared { "shared_worker" } else { "worker" };
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "exact Worker script ready",
        |messages| {
            messages.iter().any(|message| {
                message["params"]["targetInfo"]["url"] == worker_url
                    && message["params"]["targetInfo"]["type"] == target_type
            })
        },
    )
    .await;
    let target = ctx
        .sent
        .iter()
        .find(|message| {
            message["params"]["targetInfo"]["url"] == worker_url
                && message["params"]["targetInfo"]["type"] == target_type
        })
        .unwrap()["params"]["targetInfo"]["targetId"]
        .clone();
    ctx.process_async(json!({"id":6,"method":"Target.attachToTarget","params":{"targetId":target,"flatten":true}})).await;
    let worker_session = take_response_by_id(&mut ctx, 6)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    ctx.process_async(json!({"id":7,"method":"Network.enable","sessionId":worker_session}))
        .await;
    ctx.expect_result(7, json!({}), Some(&worker_session));
    ctx.sent.clear();
    let (_, mut native) = ctx.conn.subscribe_browser_events().unwrap();
    ctx.process_async(json!({"id":8,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
        "expression":format!("(worker.port || worker).postMessage({})", json!({"url":url,"binary":binary,"resource":match resource_kind {Resource::Fetch=>"fetch",Resource::Xhr=>"xhr",Resource::CspReport=>"csp"}})),
    }})).await;
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "Worker request pause",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == "Fetch.requestPaused"
                    && message["params"]["request"]["url"] == url
            })
        },
    )
    .await;
    let paused = ctx.take_first_matching("Worker request pause", |message| {
        message["method"] == "Fetch.requestPaused" && message["params"]["request"]["url"] == url
    });
    let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();
    let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();
    if !request_stage {
        ctx.process_async(json!({"id":9,"method":"Fetch.continueRequest","sessionId":"SID-1","params":{
            "requestId":request_id,"interceptResponse":!matches!(decision, Decision::AuthCancel | Decision::AuthProvide),
            "method":"PATCH","headers":[{"name":"x-worker-request","value":"intercepted"},{"name":"content-type","value":"text/plain"}],
            "postData":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "intercepted-payload"),
        }})).await;
        ctx.expect_result(9, json!({}), Some("SID-1"));
        let method = if matches!(decision, Decision::AuthCancel | Decision::AuthProvide) {
            "Fetch.authRequired"
        } else {
            "Fetch.requestPaused"
        };
        wait_until_messages(
            &mut ctx,
            Some("SID-1"),
            "Worker auth/response pause",
            |messages| {
                messages.iter().any(|message| {
                    message["method"] == method && message["params"]["requestId"] == request_id
                })
            },
        )
        .await;
    }
    let (method, params) = match decision {
        Decision::RequestFail | Decision::ResponseFail => (
            "Fetch.failRequest",
            json!({"requestId":request_id,"errorReason":"Failed"}),
        ),
        Decision::RequestFulfill
        | Decision::RequestFulfillBinary
        | Decision::RequestFulfillCorsBlocked
        | Decision::ResponseFulfill => (
            "Fetch.fulfillRequest",
            json!({"requestId":request_id,
            "responseCode":201,"responseHeaders":[{"name":"content-type","value":"text/plain"}],
            "body":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "synthetic-body")}),
        ),
        Decision::AuthCancel => (
            "Fetch.continueWithAuth",
            json!({"requestId":request_id,"authChallengeResponse":{"response":"CancelAuth"}}),
        ),
        Decision::AuthProvide => (
            "Fetch.continueWithAuth",
            json!({"requestId":request_id,"authChallengeResponse":{"response":"ProvideCredentials","username":"worker","password":"secret"}}),
        ),
        Decision::ResponseContinue => ("Fetch.continueResponse", json!({"requestId":request_id})),
        Decision::RequestAbort | Decision::ResponseAbort => (
            "Runtime.evaluate",
            json!({"expression":"(worker.port || worker).postMessage({abort:true})"}),
        ),
    };
    ctx.process_async(json!({"id":10,"method":method,"sessionId":"SID-1","params":params}))
        .await;
    if matches!(decision, Decision::RequestAbort | Decision::ResponseAbort) {
        let response = take_response_by_id(&mut ctx, 10);
        assert!(response.get("error").is_none(), "{response:?}");
    } else {
        ctx.expect_result(10, json!({}), Some("SID-1"));
    }
    ctx.process_and_wait_for_response_async(
        json!({"id":11,"method":"Runtime.evaluate","sessionId":"SID-1","params":{
            "expression":"workerResult","awaitPromise":true,"returnByValue":true,
        }}),
    )
    .await;
    let result = take_response_by_id(&mut ctx, 11);
    let value = &result["result"]["result"]["value"];
    let expected_body = match decision {
        Decision::RequestFail
        | Decision::ResponseFail
        | Decision::RequestAbort
        | Decision::ResponseAbort
        | Decision::RequestFulfillCorsBlocked => {
            if !csp_report {
                assert!(value["error"].is_string(), "{result:?}");
            }
            None
        }
        Decision::RequestFulfill | Decision::RequestFulfillBinary | Decision::ResponseFulfill => {
            if !csp_report {
                assert_eq!(value["status"], 201, "{result:?}");
            }
            Some("synthetic-body")
        }
        Decision::AuthCancel => {
            assert_eq!(value["status"], 401, "{result:?}");
            Some("auth-required")
        }
        Decision::AuthProvide | Decision::ResponseContinue => {
            assert_eq!(value["status"], 200, "{result:?}");
            Some("server-body")
        }
    };
    if csp_report {
        assert_eq!(value["reported"], true, "{result:?}");
    }
    if let Some(body) = expected_body
        && !csp_report
    {
        assert_eq!(value["text"], body);
    }
    let owner = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::NetworkRequestCompleted(occurrence) = native.recv().await.unwrap().event
                && matches!(occurrence.owner, NetworkOwner::Worker(_))
                && matches!(&occurrence.renderer.item, RendererNetworkOutputItem::Resource(item)
                    if matches!(item.as_ref(), ScriptNetworkOutputItem::SubresourceNetworkRecord(record) if record.url().as_str() == url)) {
                break occurrence.owner;
            }
        }
    }).await.expect("the exact Worker must commit its terminal fact");
    assert!(if shared {
        matches!(owner, NetworkOwner::Worker(WorkerHandle::Shared { .. }))
    } else {
        matches!(owner, NetworkOwner::Worker(WorkerHandle::Dedicated { .. }))
    });
    let snapshot = ctx.conn.subscribe_browser_events().unwrap().0;
    let requests = snapshot.network_requests.iter().filter(|request| match &request.state {
        NetworkRequestState::Recorded(record) => record.url().as_str() == url,
        _ => request.output_items().iter().any(|item| matches!(item, RendererNetworkOutputItem::Resource(item)
            if matches!(item.as_ref(), ScriptNetworkOutputItem::SubresourceRequestStarted(request) if request.url().as_str() == url))),
    }).collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        1,
        "Page must not keep a second Worker fact: {requests:?}"
    );
    assert_eq!(requests[0].owner, owner);
    let NetworkRequestState::Recorded(record) = &requests[0].state else {
        panic!("terminal Worker record");
    };
    if !csp_report {
        let (method, header, body) = if request_stage {
            ("POST", "original", "worker-payload")
        } else {
            ("PATCH", "intercepted", "intercepted-payload")
        };
        assert_eq!(record.method(), method);
        assert!(
            record.request_headers().iter().any(|(name, value)| name
                .eq_ignore_ascii_case("x-worker-request")
                && value == header)
        );
        if binary {
            assert_eq!(record.request_body_bytes(), Some(&[0, 128, 255][..]));
        } else {
            assert_eq!(record.request_body(), Some(body));
            assert_eq!(record.request_body_bytes(), Some(body.as_bytes()));
        }
    }
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "Worker terminal projection",
        |messages| {
            messages.iter().any(|message| {
                message["sessionId"] == worker_session
                    && message["params"]["requestId"] == network_id
                    && matches!(
                        message["method"].as_str(),
                        Some("Network.loadingFinished" | "Network.loadingFailed")
                    )
            })
        },
    )
    .await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["sessionId"] == "SID-1"
                && message["params"]["requestId"] == network_id
                && message["method"]
                    .as_str()
                    .is_some_and(|method| method.starts_with("Network."))),
        "Page must not publish Worker Network: {:?}",
        ctx.sent
    );
    let worker_requests = ctx
        .sent
        .iter()
        .filter(|message| {
            message["sessionId"] == worker_session
                && message["method"] == "Network.requestWillBeSent"
                && message["params"]["requestId"] == network_id
        })
        .collect::<Vec<_>>();
    assert_eq!(
        worker_requests.len(),
        1,
        "one Worker request announcement: {:?}",
        ctx.sent
    );
    assert!(worker_requests[0]["params"].get("frameId").is_none());
    assert_eq!(worker_requests[0]["params"]["loaderId"], "");
    if matches!(decision, Decision::AuthCancel | Decision::AuthProvide) {
        let extra_info = |method| {
            let events = ctx
                .sent
                .iter()
                .filter(|message| {
                    message["sessionId"] == worker_session
                        && message["method"] == method
                        && message["params"]["requestId"] == network_id
                })
                .collect::<Vec<_>>();
            assert_eq!(events.len(), 1, "one Worker {method}: {:?}", ctx.sent);
            &events[0]["params"]
        };
        let request_extra = extra_info("Network.requestWillBeSentExtraInfo");
        assert!(
            request_extra["headers"]
                .as_object()
                .unwrap()
                .iter()
                .any(
                    |(name, value)| name.eq_ignore_ascii_case("x-worker-request")
                        && value == "intercepted"
                )
        );
        assert!(
            !request_extra["headers"]
                .as_object()
                .unwrap()
                .keys()
                .any(|name| name.eq_ignore_ascii_case("authorization"))
        );
        assert_eq!(
            extra_info("Network.responseReceivedExtraInfo")["statusCode"],
            if matches!(decision, Decision::AuthCancel) {
                401
            } else {
                200
            }
        );
    }
    if let Some(body) = expected_body {
        ctx.process_async(json!({"id":12,"method":"Network.getResponseBody","sessionId":worker_session,"params":{"requestId":network_id}})).await;
        ctx.expect_result(
            12,
            json!({"body":body,"base64Encoded":false}),
            Some(&worker_session),
        );
        ctx.process_async(json!({"id":13,"method":"Network.getResponseBody","sessionId":"SID-1","params":{"requestId":network_id}})).await;
        let page_body = take_response_by_id(&mut ctx, 13);
        assert!(
            page_body.get("error").is_some(),
            "Page must not own Worker response bytes: {page_body:?}"
        );
    }
    server.abort();
}
