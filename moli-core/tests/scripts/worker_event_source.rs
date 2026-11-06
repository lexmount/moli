use super::*;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

const WORKER: &str = r#"
async function run(config, send) {
    try {
        const check = (value, label) => { if (!value) throw new Error(label); };
        const url = config.url;
        const scenario = config.scenario;
        if (scenario === 'after-close') {
            close();
            const source = new EventSource(url);
            check(source.readyState === EventSource.CONNECTING, 'closing Worker discards tasks without changing initial EventSource state');
            send({ok:true}); return;
        }
        if (scenario === 'constructor') {
            let missing = false, invalid = false, reads = 0;
            try { new EventSource(); } catch (e) { missing = e instanceof TypeError; }
            try { new EventSource('http://['); } catch (e) { invalid = e.name === 'SyntaxError'; }
            const source = new EventSource(url, {get withCredentials() { reads++; return true; }});
            check(missing && invalid && reads === 1, 'constructor conversion/errors');
            check(source.url === url && source.withCredentials && source.readyState === 0, 'initial attributes');
            check(source instanceof EventTarget && EventSource.CONNECTING === 0 &&
                EventSource.OPEN === 1 && EventSource.CLOSED === 2, 'interface/constants');
            source.close(); source.close();
            check(source.readyState === 2, 'idempotent immediate close');
            send({ok:true}); return;
        }
        // Browser algorithms must not call author replacements of timers.
        const cancelAuthorTimeout = globalThis.clearTimeout;
        globalThis.setTimeout = globalThis.clearTimeout = () => { throw new Error('author timer called'); };
        const source = new EventSource(url, {withCredentials:!!config.credentials});
        for (let id = 0; id < 16; id++) cancelAuthorTimeout(id);
        if (scenario === 'report-close') {
            addEventListener('securitypolicyviolation', event => {
                try {
                    check(event.disposition === 'report' && event.effectiveDirective === 'connect-src', 'report-only violation');
                    source.close();
                    check(source.readyState === 2, 'closed by policy callback');
                    send({ok:true});
                } catch (error) { send({error:String(error)}); }
            }, {once:true});
            return;
        }
        if (scenario === 'close-before') {
            source.close();
            check(source.readyState === 2, 'close before connection');
            send({ok:true}); return;
        }
        const result = await new Promise((resolve, reject) => {
            let opens = 0, errors = 0, messages = [], checkpoint = false;
            source.onopen = event => {
                try {
                    check(event.isTrusted && source.readyState === 1, 'open event/state');
                    opens++;
                } catch (error) { source.close(); reject(error); }
            };
            source.onerror = event => {
                try {
                    check(event.isTrusted && !('data' in event), 'error event shape');
                    errors++;
                    if (scenario === 'reject') {
                        check(source.readyState === 2 && opens === 0 && messages.length === 0, 'fatal response/policy failure');
                        resolve({ok:true});
                    } else if (scenario !== 'reconnect') {
                        source.close(); reject(new Error('unexpected EventSource error'));
                    } else {
                        check(source.readyState === 0, 'reconnecting state');
                    }
                } catch (error) { source.close(); reject(error); }
            };
            const receive = event => {
                try {
                    check(event.isTrusted && event.source === null && event.ports.length === 0, 'message shape');
                    check(event.origin === new URL(url).origin && source.readyState === 1, 'message origin/state');
                    messages.push([event.type, event.data, event.lastEventId]);
                    if (scenario === 'stream') {
                        if (messages.length === 1) { queueMicrotask(() => checkpoint = true); return; }
                        check(checkpoint, 'separate SSE networking tasks need a microtask checkpoint');
                        check(JSON.stringify(messages) === JSON.stringify([['message','hello 世界','42'],['special','line1\nline2','43']]), 'SSE decoding/type/id');
                    } else if (scenario === 'reconnect') {
                        if (messages.length !== 2) return;
                        check(messages[0][1] === 'first' && messages[1][1] === 'second' &&
                            messages[1][2] === '43' && opens === 2 && errors === 1, 'reconnect sequence');
                    } else if (scenario === 'hold') {
                        resolve({ok:true, open:true}); return;
                    } else if (scenario === 'close-messages') {
                        check(messages.length === 1 && event.data === 'first', 'closed source delivered queued event');
                    } else {
                        check(event.data === 'allowed', 'CORS allowed payload');
                    }
                    source.close();
                    check(source.readyState === 2, 'closed state');
                    resolve({ok:true, messages:messages.length});
                } catch (error) { source.close(); reject(error); }
            };
            source.onmessage = receive;
            source.addEventListener('special', receive);
        });
        send(result);
    } catch (error) { send({error:String(error)}); }
}
if ('onconnect' in globalThis) {
    onconnect = event => {
        const port = event.ports[0];
        port.onmessage = event => run(event.data, value => port.postMessage(value));
        port.start();
    };
} else {
    onmessage = event => run(event.data, value => postMessage(value));
}
"#;

struct EventSourceWorkerServer {
    origin: String,
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
    task: JoinHandle<()>,
    disconnected: Arc<tokio::sync::Notify>,
}

impl Drop for EventSourceWorkerServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl EventSourceWorkerServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let recorded = Arc::clone(&requests);
        let disconnected = Arc::new(tokio::sync::Notify::new());
        let closed = Arc::clone(&disconnected);
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let recorded = Arc::clone(&recorded);
                        let closed = Arc::clone(&closed);
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 16384 {
                                let Ok(byte) = stream.read_u8().await else { return };
                                bytes.push(byte);
                            }
                            let request = String::from_utf8_lossy(&bytes);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            let header = |name: &str| request.lines().filter_map(|line| line.split_once(':'))
                                .find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.trim());
                            let sequence = {
                                let mut requests = recorded.lock();
                                let sequence = requests.iter().filter(|r| r["path"] == path).count();
                                requests.push(serde_json::json!({"path":path,"accept":header("Accept"),
                                    "lastEventId":header("Last-Event-ID"),"origin":header("Origin"),"method":request.split_whitespace().next()}));
                                sequence
                            };
                            if path.starts_with("/hold") {
                                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nf\r\ndata: allowed\n\n\r\n").await;
                                let _ = stream.read_u8().await;
                                closed.notify_one();
                                return;
                            }
                            let (status, mime, body) = if path.starts_with("/page") {
                                ("200 OK", "text/html", "<!doctype html><body>Worker EventSource")
                            } else if path.starts_with("/sw.js") {
                                ("200 OK", "text/javascript", "oninstall = event => event.waitUntil(skipWaiting()); onactivate = event => event.waitUntil(clients.claim()); onfetch = event => { if (new URL(event.request.url).pathname === '/sw-events') event.respondWith(new Response('data: allowed\\n\\n', {headers:{'Content-Type':'text/event-stream'}})); };")
                            } else if path.starts_with("/worker.js") {
                                ("200 OK", "text/javascript", WORKER)
                            } else if path.starts_with("/no-content") {
                                ("204 No Content", "text/event-stream", "")
                            } else if path.starts_with("/wrong-mime") {
                                ("200 OK", "text/plain", "data: forbidden\n\n")
                            } else if path.starts_with("/reconnect") {
                                ("200 OK", "text/event-stream", if sequence == 0 {
                                    if path.starts_with("/reconnect-unicode") { "retry: 0\nid: 雪\ndata: first\n\n" } else { "retry: 0\nid: 42\ndata: first\n\n" }
                                } else { "id: 43\ndata: second\n\n" })
                            } else if path.starts_with("/stream") {
                                ("200 OK", "text/event-stream", "\u{feff}id: 42\r\ndata: hello 世界\r\n\r\nid: 43\nevent: special\ndata: line1\ndata: line2\n\n")
                            } else if path.starts_with("/close-messages") {
                                ("200 OK", "text/event-stream", "data: first\n\ndata: second\n\n")
                            } else {
                                ("200 OK", "text/event-stream", "data: allowed\n\n")
                            };
                            let policy = if path.starts_with("/worker.js?csp") {
                                "Content-Security-Policy: connect-src 'none'\r\n".to_owned()
                            } else if path.starts_with("/worker.js?report") {
                                "Content-Security-Policy-Report-Only: connect-src 'none'\r\n".to_owned()
                            } else if path.starts_with("/allow") || path.starts_with("/reconnect") {
                                format!("Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Credentials: true\r\n", header("Origin").unwrap_or("*"))
                            } else { String::new() };
                            let response = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{policy}Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            origin,
            requests,
            task,
            disconnected,
        })
    }
}

async fn run_worker_source_case(
    server: &EventSourceWorkerServer,
    target: &EventSourceWorkerServer,
    scenario: &str,
    path: &str,
    credentials: bool,
    csp: bool,
) -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    if scenario == "serviceworker" {
        let setup = page.evaluate_runtime_expression_with_await_async(r#"(async () => {
            await navigator.serviceWorker.register('/sw.js');
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller)
                await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
            return true;
        })()"#,true).await?;
        assert_eq!(setup["value"], true, "{setup}");
    }
    for shared in [false, true] {
        if scenario == "after-close" && shared {
            continue;
        }
        let config = serde_json::json!({"scenario":if scenario=="serviceworker" {"cors"} else {scenario},"url":format!("{}{path}?shared={shared}",target.origin),"credentials":credentials});
        let expression = format!(
            r#"new Promise((resolve, reject) => {{
            const worker = {shared} ? new SharedWorker({worker_url}) : new Worker({worker_url});
            const port = {shared} ? worker.port : worker;
            port.onmessage = event => {{
                if ({shared}) port.close(); else worker.terminate();
                resolve(JSON.stringify(event.data));
            }};
            worker.onerror = event => reject(new Error(event.message || 'worker bootstrap error'));
            if ({shared}) port.start();
            port.postMessage({config});
        }})"#,
            worker_url = serde_json::to_string(&format!(
                "/worker.js?{}shared={shared}",
                if csp {
                    "csp&"
                } else if scenario == "report-close" {
                    "report&"
                } else {
                    ""
                }
            ))?,
        );
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let value: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .unwrap_or_else(|| panic!("serialized worker result: {result}")),
        )?;
        assert_eq!(
            value["ok"], true,
            "scenario={scenario}, shared={shared}: {value}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_stream_decodes_messages_and_custom_events() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(&server, &server, "stream", "/stream", false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_reconnects_with_last_event_id() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(&server, &server, "reconnect", "/reconnect", false, false).await?;
    let requests = server.requests.lock();
    for shared in [false, true] {
        let path = format!("/reconnect?shared={shared}");
        let requests = requests
            .iter()
            .filter(|r| r["path"] == path)
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 2, "{requests:?}");
        assert_eq!(requests[0]["lastEventId"], serde_json::Value::Null);
        assert_eq!(requests[1]["lastEventId"], "42");
        assert!(requests.iter().all(|r| r["accept"] == "text/event-stream"));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_close_before_connect_cancels_startup() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(
        &server,
        &server,
        "close-before",
        "/closed-before",
        false,
        false,
    )
    .await?;
    assert!(
        !server
            .requests
            .lock()
            .iter()
            .any(|r| r["path"].as_str().unwrap().starts_with("/closed-before"))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_close_suppresses_remaining_messages() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(
        &server,
        &server,
        "close-messages",
        "/close-messages",
        false,
        false,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_invalid_response_closes_without_opening() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    for path in ["/no-content", "/wrong-mime"] {
        run_worker_source_case(&server, &server, "reject", path, false, false).await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_cors_checks_credentials_before_dispatch() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    let cross = EventSourceWorkerServer::spawn().await?;
    for credentials in [false, true] {
        run_worker_source_case(&server, &cross, "cors", "/allow", credentials, false).await?;
        run_worker_source_case(&server, &cross, "reject", "/deny", credentials, false).await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_csp_prevents_transport() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(&server, &server, "reject", "/csp-blocked", false, true).await?;
    assert!(
        !server
            .requests
            .lock()
            .iter()
            .any(|r| r["path"].as_str().unwrap().starts_with("/csp-blocked"))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_constructor_converts_arguments_and_closes() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(
        &server,
        &server,
        "constructor",
        "/constructor",
        false,
        false,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_cross_origin_reconnect_keeps_browser_headers() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    let cross = EventSourceWorkerServer::spawn().await?;
    for (credentials, path) in [
        (false, "/reconnect-anonymous"),
        (true, "/reconnect-credentials"),
    ] {
        run_worker_source_case(&server, &cross, "reconnect", path, credentials, false).await?;
    }
    let requests = cross.requests.lock();
    assert_eq!(
        requests.len(),
        8,
        "both Worker kinds reconnect once in each credentials mode: {requests:?}"
    );
    assert!(
        requests.iter().all(|r| r["method"] == "GET"),
        "browser Last-Event-ID must not trigger an author-header preflight: {requests:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_reconnect_preserves_utf8_event_ids() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(
        &server,
        &server,
        "reconnect",
        "/reconnect-unicode",
        false,
        false,
    )
    .await?;
    let requests = server.requests.lock();
    let ids = requests
        .iter()
        .filter(|r| {
            r["path"]
                .as_str()
                .unwrap()
                .starts_with("/reconnect-unicode")
        })
        .filter_map(|r| r["lastEventId"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec!["雪", "雪"],
        "Last-Event-ID transmits the event id as UTF-8 bytes"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn terminating_worker_cancels_open_event_source_transport() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/page", server.origin)).await?;
    let result=tokio::time::timeout(Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(r#"new Promise((resolve,reject) => {
            const worker = new Worker('/worker.js');
            worker.onmessage = event => { worker.terminate(); resolve(JSON.stringify(event.data)); };
            worker.onerror = event => reject(new Error(event.message || 'worker bootstrap error'));
            worker.postMessage({scenario:'hold', url:new URL('/hold',location.href).href});
        })"#,true)).await??;
    let value: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    assert_eq!(value, serde_json::json!({"ok":true,"open":true}));
    tokio::time::timeout(Duration::from_secs(5), server.disconnected.notified()).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_receives_service_worker_response() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(
        &server,
        &server,
        "serviceworker",
        "/sw-events",
        false,
        false,
    )
    .await?;
    assert!(
        !server
            .requests
            .lock()
            .iter()
            .any(|r| r["path"].as_str().unwrap().starts_with("/sw-events")),
        "the response must come from the controlling Service Worker"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_event_source_policy_callback_can_close_connection() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(&server, &server, "report-close", "/allow", false, false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_worker_event_source_remains_connecting_without_transport() -> Result<()> {
    let server = EventSourceWorkerServer::spawn().await?;
    run_worker_source_case(
        &server,
        &server,
        "after-close",
        "/must-not-start",
        false,
        false,
    )
    .await?;
    assert!(
        !server
            .requests
            .lock()
            .iter()
            .any(|r| r["path"].as_str().unwrap().starts_with("/must-not-start"))
    );
    Ok(())
}
