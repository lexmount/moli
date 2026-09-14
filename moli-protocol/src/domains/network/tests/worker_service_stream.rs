use super::*;
use crate::testing::{
    enable_network_on_new_dedicated_worker, pause_new_dedicated_workers,
    wait_until_scheduler_message,
};

#[tokio::test(flavor = "multi_thread")]
async fn worker_service_stream_network_control() {
    response_stages(Case::Network).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_service_stream_controller_fallback() {
    response_stages(Case::Fallback).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_service_stream_response_before_body() {
    response_stages(Case::Stream).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_service_stream_failure_retains_prefix() {
    response_stages(Case::Partial).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_service_stream_abort_does_not_cancel_another_worker() {
    response_stages(Case::AbortSecond).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_service_stream_termination_closes_only_its_upstream() {
    response_stages(Case::TerminateSecond).await;
}

#[derive(Clone, Copy)]
enum Case {
    Network,
    Fallback,
    Stream,
    Partial,
    AbortSecond,
    TerminateSecond,
}

async fn evaluate(
    ctx: &mut TestContext,
    id: u64,
    session: &str,
    expression: &str,
) -> serde_json::Value {
    ctx.process_async(
        json!({"id":id,"method":"Runtime.evaluate","sessionId":session,
        "params":{"expression":expression,"awaitPromise":true,"returnByValue":true}}),
    )
    .await;
    wait_until_scheduler_message(ctx, expression, |m| {
        m["sessionId"] == session && m["id"] == id
    })
    .await;
    let result = ctx.take_response_by_id(id);
    assert!(result["result"]["exceptionDetails"].is_null(), "{result}");
    result["result"]["result"]["value"].clone()
}

async fn response_stages(case: Case) {
    let controlled = !matches!(case, Case::Network);
    let streamed = !matches!(case, Case::Network | Case::Fallback);
    let partial = matches!(case, Case::Partial);
    let count = if matches!(case, Case::AbortSecond | Case::TerminateSecond) {
        2
    } else {
        1
    };
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for _ in 0..count {
        let (prefix_tx, prefix_rx) = tokio::sync::oneshot::channel();
        let (tail_tx, tail_rx) = tokio::sync::oneshot::channel();
        senders.push((prefix_tx, tail_tx));
        receivers.push(Some((prefix_rx, tail_rx)));
    }
    let gates = Arc::new(Mutex::new(receivers));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (wire_tx, mut wire_rx) = tokio::sync::mpsc::unbounded_channel();
    let (closed_tx, mut closed_rx) = tokio::sync::mpsc::unbounded_channel();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        tokio::pin!(stopped);
        loop {
            let (mut socket, _) = tokio::select! {
                accepted = listener.accept() => accepted.unwrap(),
                finished = connections.join_next(), if !connections.is_empty() => {
                    finished.unwrap().unwrap();
                    continue;
                }
                _ = &mut stopped => break,
            };
            let gates = gates.clone();
            let wire_tx = wire_tx.clone();
            let closed_tx = closed_tx.clone();
            connections.spawn(async move {
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if socket.read(&mut byte).await.unwrap() == 0 { return; }
                    bytes.push(byte[0]);
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let (route, query) = path.split_once('?').unwrap_or((path, ""));
                let body = match route {
                    "/page" => "<!doctype html><title>worker response</title>".to_owned(),
                    "/worker.js" => "globalThis.controller=new AbortController();onmessage=async e=>{try{const r=await fetch('/probe?id='+e.data,{method:'POST',body:new Uint8Array([0,128,255,65]),signal:controller.signal});postMessage({stage:'head',status:r.status});postMessage({stage:'done',body:await r.text()});}catch(e){postMessage({stage:'error',name:e.name});}};postMessage({stage:'ready',controlled:!!navigator.serviceWorker.controller});".to_owned(),
                    "/sw.js" => format!("oninstall=e=>e.waitUntil(skipWaiting());onactivate=e=>e.waitUntil(clients.claim());onfetch=e=>{{{}}};",
                        if streamed { "if(new URL(e.request.url).pathname==='/probe')e.respondWith((async()=>{const body=await e.request.arrayBuffer();const r=await fetch('/upstream'+new URL(e.request.url).search,{method:'POST',body});return new Response(r.body,{status:202})})());" } else { "" }),
                    "/probe" | "/upstream" => {
                        assert_eq!(route, if streamed { "/upstream" } else { "/probe" });
                        assert!(request.starts_with("POST "));
                        let mut upload = [0; 4];
                        socket.read_exact(&mut upload).await.unwrap();
                        assert_eq!(upload, [0,128,255,65]);
                        let client: usize = query.strip_prefix("id=").unwrap().parse().unwrap();
                        wire_tx.send(client).unwrap();
                        let (prefix, tail) = gates.lock()[client].take().expect("one physical request per Worker");
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
                        if prefix.await.is_err() { return; }
                        socket.write_all(b"bo").await.unwrap();
                        let mut peer_byte = [0];
                        tokio::select! {
                            result = socket.read(&mut peer_byte) => {
                                match result {
                                    Ok(0) => {},
                                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {},
                                    other => panic!("unexpected client data while body is gated: {other:?}"),
                                }
                                closed_tx.send(client).unwrap();
                                return;
                            },
                            result = tail => if result.is_err() { return; },
                        }
                        if !partial { socket.write_all(b"dy").await.unwrap(); }
                        return;
                    }
                    other => panic!("unexpected HTTP request {other}"),
                };
                let mime = if route.ends_with(".js") { "text/javascript" } else { "text/html" };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            });
        }
        while let Some(finished) = connections.join_next().await {
            finished.unwrap();
        }
    });
    let mut ctx = TestContext::new();
    let mut browser_context = ctx.conn.new_browser_context_fixture_for_test("BID-1");
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    ctx.process_async(json!({"id":1,"method":"Network.enable","sessionId":"SID-1"}))
        .await;
    ctx.expect_result(1, json!({}), Some("SID-1"));
    ctx.process_async(json!({"id":2,"method":"Page.navigate","sessionId":"SID-1","params":{"url":format!("{origin}/page")}})).await;
    let _ = ctx.take_response_by_id(2);
    if controlled {
        assert_eq!(evaluate(&mut ctx,3,"SID-1","(async()=>{await navigator.serviceWorker.register('/sw.js');await navigator.serviceWorker.ready;if(!navigator.serviceWorker.controller)await new Promise(r=>navigator.serviceWorker.addEventListener('controllerchange',r,{once:true}));return true})()").await, true);
    }
    pause_new_dedicated_workers(&mut ctx, "SID-1", 4).await;
    assert_eq!(
        evaluate(&mut ctx, 5, "SID-1", "globalThis.runs=[];true").await,
        true
    );
    let mut workers = Vec::new();
    for (client, (prefix, tail)) in senders.into_iter().enumerate() {
        let command_id = 10 + client as u64 * 10;
        assert_eq!(evaluate(&mut ctx,command_id,"SID-1","{const run={};runs.push(run);run.ready=new Promise(ready=>{run.head=new Promise(head=>{run.done=new Promise(done=>{run.worker=new Worker('/worker.js');run.worker.onmessage=e=>{if(e.data.stage==='ready')ready(e.data.controlled);else if(e.data.stage==='head')head(e.data.status);else done(e.data);};});});});}true").await,true);
        let session =
            enable_network_on_new_dedicated_worker(&mut ctx, "SID-1", command_id + 1).await;
        assert_eq!(
            evaluate(
                &mut ctx,
                command_id + 3,
                "SID-1",
                &format!("runs[{client}].ready")
            )
            .await,
            controlled
        );
        assert_eq!(
            evaluate(
                &mut ctx,
                command_id + 4,
                "SID-1",
                &format!("runs[{client}].worker.postMessage({client});true")
            )
            .await,
            true
        );
        let request_url = format!("{origin}/probe?id={client}");
        wait_until_scheduler_message(&mut ctx, "Worker request admission", |m| {
            m["sessionId"] == session
                && m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == request_url
        })
        .await;
        let id = ctx
            .sent
            .iter()
            .find(|m| {
                m["sessionId"] == session
                    && m["method"] == "Network.requestWillBeSent"
                    && m["params"]["request"]["url"] == request_url
            })
            .unwrap()["params"]["requestId"]
            .clone();
        wait_until_scheduler_message(&mut ctx, "Worker response head before body", |m| {
            m["sessionId"] == session
                && m["method"] == "Network.responseReceived"
                && m["params"]["requestId"] == id
        })
        .await;
        let status = if streamed { 202 } else { 200 };
        let head = ctx
            .sent
            .iter()
            .find(|m| {
                m["sessionId"] == session
                    && m["method"] == "Network.responseReceived"
                    && m["params"]["requestId"] == id
            })
            .unwrap();
        assert_eq!(head["params"]["response"]["status"], status);
        assert_eq!(
            evaluate(
                &mut ctx,
                command_id + 5,
                "SID-1",
                &format!("runs[{client}].head")
            )
            .await,
            status
        );
        prefix.send(()).unwrap();
        wait_until_scheduler_message(&mut ctx, "Worker first data before tail", |m| {
            m["sessionId"] == session
                && m["method"] == "Network.dataReceived"
                && m["params"]["requestId"] == id
        })
        .await;
        let data = ctx
            .sent
            .iter()
            .find(|m| {
                m["sessionId"] == session
                    && m["method"] == "Network.dataReceived"
                    && m["params"]["requestId"] == id
            })
            .unwrap();
        assert_eq!(data["params"]["dataLength"], 2);
        assert!(!ctx.sent.iter().any(|m| m["sessionId"] == session
            && m["params"]["requestId"] == id
            && (m["method"] == "Network.loadingFinished"
                || m["method"] == "Network.loadingFailed")));
        workers.push((session, id, tail));
    }
    if matches!(case, Case::AbortSecond) {
        // Both VMs start with local fetch id 1. The original response, rather
        // than that local number, must select the ServiceWorker job to cancel.
        assert_eq!(
            evaluate(&mut ctx, 30, &workers[1].0, "controller.abort();true").await,
            true
        );
    }
    if matches!(case, Case::TerminateSecond) {
        assert_eq!(
            evaluate(&mut ctx, 30, "SID-1", "runs[1].worker.terminate();true").await,
            true
        );
        wait_until_scheduler_message(&mut ctx, "terminated Worker detach", |m| {
            m["sessionId"] == "SID-1"
                && m["method"] == "Target.detachedFromTarget"
                && m["params"]["sessionId"] == workers[1].0
        })
        .await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), closed_rx.recv())
                .await
                .expect("Worker termination must close its physical upstream before body release"),
            Some(1)
        );
    }
    for (client, (session, id, tail)) in workers.into_iter().enumerate() {
        if matches!(case, Case::TerminateSecond) && client == 1 {
            drop(tail);
            continue;
        }
        let aborted = matches!(case, Case::AbortSecond) && client == 1;
        if aborted {
            drop(tail);
        } else {
            tail.send(()).unwrap();
        }
        let failed = partial || aborted;
        let terminal = if failed {
            "Network.loadingFailed"
        } else {
            "Network.loadingFinished"
        };
        wait_until_scheduler_message(&mut ctx, "Worker response terminal", |m| {
            m["sessionId"] == session && m["method"] == terminal && m["params"]["requestId"] == id
        })
        .await;
        for methods in [
            &["Network.responseReceived"][..],
            &["Network.loadingFinished", "Network.loadingFailed"][..],
        ] {
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|m| m["sessionId"] == session
                        && m["params"]["requestId"] == id
                        && methods.contains(&m["method"].as_str().unwrap_or_default()))
                    .count(),
                1
            );
        }
        let command_id = 40 + client as u64 * 10;
        ctx.process_async(json!({"id":command_id,"method":"Network.getResponseBody","sessionId":session,"params":{"requestId":id}})).await;
        ctx.expect_result(
            command_id,
            json!({"body":if failed {"bo"}else{"body"},"base64Encoded":false}),
            Some(&session),
        );
        assert_eq!(
            evaluate(
                &mut ctx,
                command_id + 1,
                "SID-1",
                &format!("runs[{client}].done")
            )
            .await,
            if failed {
                json!({"stage":"error","name":if aborted {"AbortError"}else{"TypeError"}})
            } else {
                json!({"stage":"done","body":"body"})
            }
        );
    }
    let mut wire = Vec::new();
    while let Ok(client) = wire_rx.try_recv() {
        wire.push(client);
    }
    wire.sort();
    assert_eq!(wire, (0..count).collect::<Vec<_>>());
    stop.send(()).unwrap();
    server.await.unwrap();
}
