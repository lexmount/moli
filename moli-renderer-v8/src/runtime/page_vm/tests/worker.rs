use super::*;
use crate::{RendererOutputItem, RendererOwnerAction};

const SHARED_WORKER_CONSOLE_MESSAGE: &str = "log: shared-console console-probe 7";

const SHARED_WORKER_CONNECTION_COUNT_SOURCE: &str = r#"
let connections = 0;
onconnect = (event) => {
    connections++;
    const port = event.ports[0];
    port.onmessage = () => {
        port.postMessage(String(connections));
    };
    port.postMessage(String(connections));
};
"#;

async fn spawn_shared_worker_script_capture_http_server(
    script_body: &'static str,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shared worker script capture server");
    let addr = listener
        .local_addr()
        .expect("shared worker script capture server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept shared worker script capture request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read shared worker script capture request");
        let _ = request_tx.send(request);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            script_body.len(),
            script_body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write shared worker script capture response");
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn spawn_worker_script_then_api_capture_http_server(
    worker_response_headers: &'static str,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker script/api capture server");
    let addr = listener
        .local_addr()
        .expect("worker script/api capture server addr");
    let (worker_request_tx, worker_request_rx) = tokio::sync::oneshot::channel();
    let (api_request_tx, api_request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker script request");
        let worker_request = read_http_request_head(&mut stream)
            .await
            .expect("read worker script request");
        assert!(
            worker_request.starts_with("GET /worker.js HTTP/1.1\r\n"),
            "unexpected worker script request:\n{worker_request}"
        );
        let _ = worker_request_tx.send(worker_request);
        let worker_body = r#"
fetch("/api")
  .then(response => response.text())
  .then(() => postMessage("worker-fetch-ok"))
  .catch(error => postMessage("worker-fetch-error:" + error.name + ":" + error.message));
"#;
        let worker_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\n{worker_response_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            worker_body.len(),
            worker_body
        );
        stream
            .write_all(worker_response.as_bytes())
            .await
            .expect("write worker script response");

        let (mut stream, _) = listener.accept().await.expect("accept worker api request");
        let api_request = read_http_request_head(&mut stream)
            .await
            .expect("read worker api request");
        assert!(
            api_request.starts_with("GET /api HTTP/1.1\r\n"),
            "unexpected worker api request:\n{api_request}"
        );
        let _ = api_request_tx.send(api_request);
        let body = "ok";
        let api_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(api_response.as_bytes())
            .await
            .expect("write worker api response");
    });
    (
        format!("http://{addr}"),
        worker_request_rx,
        api_request_rx,
        server,
    )
}

async fn spawn_child_document_referrer_policy_shared_worker_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind child referrer policy shared worker server");
    let addr = listener
        .local_addr()
        .expect("child referrer policy shared worker server addr");
    let base_url = format!("http://{addr}");
    let (worker_request_tx, worker_request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut worker_request_tx = Some(worker_request_tx);
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept child referrer policy request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read child referrer policy request");
            if request.starts_with("GET /child.html ") {
                let body = "<!doctype html><body>child</body>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write child referrer policy document response");
            } else if request.starts_with("GET /worker.js ") {
                if let Some(tx) = worker_request_tx.take() {
                    let _ = tx.send(request);
                }
                let body = r#"onconnect = event => event.ports[0].postMessage("ready");"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write child shared worker response");
            } else {
                panic!("unexpected child referrer policy request:\n{request}");
            }
        }
    });
    (base_url, worker_request_rx, server)
}

async fn spawn_shared_worker_self_csp_websocket_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shared worker self CSP websocket server");
    let addr = listener
        .local_addr()
        .expect("shared worker self CSP websocket addr");
    let base_url = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/socket");
    let script_body = format!(
        r#"
        onconnect = (event) => {{
            const port = event.ports[0];
            const socket = new WebSocket({ws_url:?});
            socket.onopen = () => {{
                socket.send("shared-worker-self-csp");
            }};
            socket.onmessage = (event) => {{
                port.postMessage(event.data);
                socket.close(1000, "done");
                close();
            }};
            socket.onerror = () => {{
                port.postMessage("error");
                close();
            }};
        }};
        "#
    );
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept shared worker self CSP request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read shared worker self CSP request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("shared worker self CSP request path");
            match path {
                "/sw.js" => {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Security-Policy: connect-src 'self'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        script_body.len(),
                        script_body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write shared worker self CSP script response");
                }
                "/socket" => {
                    write_websocket_handshake_response(&mut stream, &request).await;
                    let payload = read_masked_websocket_text_frame(&mut stream).await;
                    write_websocket_text_frame(&mut stream, &payload).await;
                }
                other => panic!("unexpected shared worker self CSP request path: {other}"),
            }
        }
    });
    (base_url, server)
}

async fn spawn_service_worker_execution_capture_http_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker execution capture server");
    let addr = listener
        .local_addr()
        .expect("service worker execution capture server addr");
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut finished_tx = Some(finished_tx);
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept service worker execution capture request");
            let request = read_http_request_with_body(&mut stream)
                .await
                .expect("read service worker execution capture request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker execution capture request path")
                .to_owned();
            match path.as_str() {
                "/sw.js" => {
                    let body = r#"
                        const probe = [
                            Object.prototype.toString.call(self),
                            self instanceof ServiceWorkerGlobalScope,
                            self.registration && self.registration.scope,
                            typeof self.clients.claim,
                            typeof self.skipWaiting
                        ].join("|");
                        fetch("/finished", { method: "POST", body: probe });
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker script response");
                }
                "/finished" => {
                    if let Some(sender) = finished_tx.take() {
                        let body = request
                            .split_once("\r\n\r\n")
                            .map(|(_, body)| body.to_owned())
                            .unwrap_or_default();
                        let _ = sender.send(body);
                    }
                    let response =
                        "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker finished response");
                }
                other => panic!("unexpected service worker execution request path: {other}"),
            }
        }
    });
    (format!("http://{addr}"), finished_rx, server)
}

async fn spawn_service_worker_worker_main_script_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker worker main script server");
    let addr = listener
        .local_addr()
        .expect("service worker worker main script server addr");
    let (worker_request_tx, worker_request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut worker_request_tx = Some(worker_request_tx);
        for request_index in 0..2 {
            let accepted = if request_index == 0 {
                Some(listener.accept().await)
            } else {
                tokio::time::timeout(Duration::from_millis(500), listener.accept())
                    .await
                    .ok()
            };
            let Some(accepted) = accepted else {
                break;
            };
            let (mut stream, _) = accepted.expect("accept service worker worker main request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read service worker worker main request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker worker main request path");
            match path {
                "/app/sw.js" => {
                    let body = r#"
                        self.addEventListener("install", event => {
                            event.waitUntil(self.skipWaiting());
                        });
                        self.addEventListener("activate", event => {
                            event.waitUntil(self.clients.claim());
                        });
                        self.addEventListener("fetch", event => {
                            const url = new URL(event.request.url);
                            if (url.pathname === "/app/worker.js") {
                                const source = `
                                    const container = navigator.serviceWorker;
                                    const controller = container && container.controller;
                                    postMessage(JSON.stringify({
                                        main: 'sw-main:' + self.location.href,
                                        serviceWorkerType: typeof container,
                                        controllerScriptURL: controller && controller.scriptURL,
                                        controllerState: controller && controller.state,
                                        oncontrollerchangeIsNull:
                                            container.oncontrollerchange === null,
                                        addEventListenerType:
                                            typeof container.addEventListener
                                    }));
                                `;
                                event.respondWith(new Response(source, {
                                    headers: { "Content-Type": "application/javascript" }
                                }));
                            }
                        });
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker worker main sw response");
                }
                "/app/worker.js" => {
                    if let Some(sender) = worker_request_tx.take() {
                        let _ = sender.send(request);
                    }
                    let body = r#"postMessage("network-main");"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker worker main fallback response");
                }
                other => panic!("unexpected service worker worker main request path: {other}"),
            }
        }
    });
    (format!("http://{addr}"), worker_request_rx, server)
}

async fn spawn_service_worker_worker_controllerchange_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker worker controllerchange server");
    let addr = listener
        .local_addr()
        .expect("service worker worker controllerchange server addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept service worker worker controllerchange request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read service worker worker controllerchange request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker worker controllerchange request path");
            match path {
                "/app/worker.js" => {
                    let body = r#"
                        const container = navigator.serviceWorker;
                        const initialControllerIsNull = container.controller === null;
                        const events = [];
                        function postControllerChange(label, event) {
                            events.push(label + ":" + event.type);
                            if (events.length < 2) {
                                return;
                            }
                            const controller = container.controller;
                            postMessage(JSON.stringify({
                                initialControllerIsNull,
                                events,
                                controllerScriptURL: controller && controller.scriptURL,
                                controllerState: controller && controller.state
                            }));
                        }
                        container.addEventListener("controllerchange", event => {
                            postControllerChange("listener", event);
                        });
                        container.oncontrollerchange = event => {
                            postControllerChange("handler", event);
                        };
                        postMessage(JSON.stringify({
                            ready: true,
                            initialControllerIsNull
                        }));
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker worker controllerchange worker response");
                }
                "/app/sw.js" => {
                    let body = r#"
                        self.addEventListener("install", event => {
                            event.waitUntil(self.skipWaiting());
                        });
                        self.addEventListener("activate", event => {
                            event.waitUntil(self.clients.claim());
                        });
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker worker controllerchange sw response");
                }
                other => {
                    panic!(
                        "unexpected service worker worker controllerchange request path: {other}"
                    )
                }
            }
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_service_worker_abort_fetch_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker abort fetch server");
    let addr = listener
        .local_addr()
        .expect("service worker abort fetch server addr");
    let server = tokio::spawn(async move {
        for request_index in 0..2 {
            let accepted = if request_index == 0 {
                Some(listener.accept().await)
            } else {
                tokio::time::timeout(Duration::from_millis(500), listener.accept())
                    .await
                    .ok()
            };
            let Some(accepted) = accepted else {
                break;
            };
            let (mut stream, _) = accepted.expect("accept service worker abort fetch request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read service worker abort fetch request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker abort fetch request path");
            match path {
                "/app/sw.js" => {
                    let body = r#"
                        self.addEventListener("install", event => {
                            event.waitUntil(self.skipWaiting());
                        });
                        self.addEventListener("activate", event => {
                            event.waitUntil(self.clients.claim());
                        });
                        self.addEventListener("fetch", event => {
                            const url = new URL(event.request.url);
                            if (url.pathname === "/app/slow.txt") {
                                event.respondWith(new Promise(() => {}));
                            }
                        });
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker abort fetch sw response");
                }
                "/app/slow.txt" => {
                    let body = "network-slow";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker abort fetch fallback response");
                }
                other => panic!("unexpected service worker abort fetch request path: {other}"),
            }
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_service_worker_shared_worker_main_script_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker shared worker main script server");
    let addr = listener
        .local_addr()
        .expect("service worker shared worker main script server addr");
    let (worker_request_tx, worker_request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut worker_request_tx = Some(worker_request_tx);
        for request_index in 0..2 {
            let accepted = if request_index == 0 {
                Some(listener.accept().await)
            } else {
                tokio::time::timeout(Duration::from_millis(500), listener.accept())
                    .await
                    .ok()
            };
            let Some(accepted) = accepted else {
                break;
            };
            let (mut stream, _) =
                accepted.expect("accept service worker shared worker main request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read service worker shared worker main request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker shared worker main request path");
            match path {
                "/app/sw.js" => {
                    let body = r#"
                        self.addEventListener("install", event => {
                            event.waitUntil(self.skipWaiting());
                        });
                        self.addEventListener("activate", event => {
                            event.waitUntil(self.clients.claim());
                        });
                        self.addEventListener("fetch", event => {
                            const url = new URL(event.request.url);
                            if (url.pathname === "/app/shared-worker.js") {
                                const source = `
                                    onconnect = event => {
                                        event.ports[0].postMessage('sw-main:' + self.location.href);
                                    };
                                `;
                                event.respondWith(new Response(source, {
                                    headers: { "Content-Type": "application/javascript" }
                                }));
                            }
                        });
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker shared worker main sw response");
                }
                "/app/shared-worker.js" => {
                    if let Some(sender) = worker_request_tx.take() {
                        let _ = sender.send(request);
                    }
                    let body = r#"
                        onconnect = event => {
                            event.ports[0].postMessage("network-main");
                        };
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker shared worker main fallback response");
                }
                other => {
                    panic!("unexpected service worker shared worker main request path: {other}")
                }
            }
        }
    });
    (format!("http://{addr}"), worker_request_rx, server)
}

async fn spawn_service_worker_shared_worker_port_messageerror_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker shared worker port messageerror server");
    let addr = listener
        .local_addr()
        .expect("service worker shared worker port messageerror server addr");
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept service worker shared worker port messageerror request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read service worker shared worker port messageerror request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker shared worker port messageerror request path");
            let body = match path {
                "/app/sw.js" => {
                    r#"
                        const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
                        self.addEventListener("install", event => {
                            event.waitUntil(self.skipWaiting());
                        });
                        self.addEventListener("activate", event => {
                            event.waitUntil(self.clients.claim());
                        });
                        self.addEventListener("message", event => {
                            event.waitUntil(Promise.resolve().then(() => {
                                if (event.data !== "send-wasm-over-port") {
                                    event.source.postMessage(
                                        "unexpected-worker-message:" + event.data
                                    );
                                    return;
                                }
                                const port = event.ports[0];
                                const module = new WebAssembly.Module(bytes);
                                port.postMessage({ kind: "worker-module", module });
                                event.source.postMessage("worker-sent-module");
                            }));
                        });
                    "#
                }
                "/app/shared-worker.js" => {
                    r#"
                        onconnect = event => {
                            const controlPort = event.ports[0];
                            controlPort.onmessage = event => {
                                if (event.data !== "bind-port") {
                                    controlPort.postMessage("unexpected-control:" + event.data);
                                    return;
                                }
                                const port = event.ports[0];
                                port.onmessage = event => {
                                    controlPort.postMessage(JSON.stringify({
                                        kind: "unexpected-port-message",
                                        module: event.data &&
                                            event.data.module instanceof WebAssembly.Module
                                    }));
                                };
                                port.onmessageerror = event => {
                                    controlPort.postMessage(JSON.stringify({
                                        kind: "port-messageerror",
                                        data: event.data,
                                        origin: event.origin,
                                        source: event.source,
                                        ports: event.ports.length
                                    }));
                                };
                                port.start();
                                controlPort.postMessage("port-ready");
                            };
                            controlPort.start();
                            controlPort.postMessage("shared-ready");
                        };
                    "#
                }
                other => {
                    panic!(
                        "unexpected service worker shared worker port messageerror request path: {other}"
                    )
                }
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write service worker shared worker port messageerror response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_service_worker_blob_worker_fetch_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind service worker blob worker fetch server");
    let addr = listener
        .local_addr()
        .expect("service worker blob worker fetch server addr");
    let (sample_request_tx, sample_request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut sample_request_tx = Some(sample_request_tx);
        for request_index in 0..2 {
            let accepted = if request_index == 0 {
                Some(listener.accept().await)
            } else {
                tokio::time::timeout(Duration::from_millis(500), listener.accept())
                    .await
                    .ok()
            };
            let Some(accepted) = accepted else {
                break;
            };
            let (mut stream, _) = accepted.expect("accept service worker blob worker request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read service worker blob worker request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("service worker blob worker request path");
            match path {
                "/app/sw.js" => {
                    let body = r#"
                        self.addEventListener("install", event => {
                            event.waitUntil(self.skipWaiting());
                        });
                        self.addEventListener("activate", event => {
                            event.waitUntil(self.clients.claim());
                        });
                        self.addEventListener("fetch", event => {
                            const url = new URL(event.request.url);
                            if (url.pathname === "/app/sample.txt") {
                                event.respondWith(new Response("sw-sample"));
                            }
                        });
                    "#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker blob worker sw response");
                }
                "/app/sample.txt" => {
                    if let Some(sender) = sample_request_tx.take() {
                        let _ = sender.send(request);
                    }
                    let body = "network-sample";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write service worker blob worker sample fallback");
                }
                other => panic!("unexpected service worker blob worker request path: {other}"),
            }
        }
    });
    (format!("http://{addr}"), sample_request_rx, server)
}

async fn drive_service_worker_page_vm_until_done_with_explicit_producer_admission(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
    mut admit_additional_producer_work: impl FnMut(&PageVm),
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        page_vm
            .runtime_hooks
            .browser_context_runtime
            .drain_service_worker_service_lane();
        admit_additional_producer_work(page_vm);
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {
            page_vm
                .runtime_hooks
                .browser_context_runtime
                .drain_service_worker_service_lane();
            admit_additional_producer_work(page_vm);
        }
        let loader = page_vm.main_document_resource_loader();
        while page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {
            page_vm
                .runtime_hooks
                .browser_context_runtime
                .drain_service_worker_service_lane();
            admit_additional_producer_work(page_vm);
        }
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        if page_vm.vm_mut().eval(done_expression)? == "true" {
            return Ok(());
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
    }
    admit_additional_producer_work(page_vm);
    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let value = page_vm
        .vm_mut()
        .eval(done_expression)
        .unwrap_or_else(|error| format!("<failed to read done expression: {error}>"));
    anyhow::bail!("{context}; done={value}");
}

async fn drive_service_worker_page_vm_until_done(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    drive_service_worker_page_vm_until_done_with_explicit_producer_admission(
        page_vm,
        done_expression,
        context,
        |_| {},
    )
    .await
}

async fn drive_service_worker_and_shared_worker_page_vm_until_done(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    drive_service_worker_page_vm_until_done_with_explicit_producer_admission(
        page_vm,
        done_expression,
        context,
        |page_vm| {
            page_vm
                .runtime_hooks
                .browser_context_runtime
                .drain_shared_worker_service_lane();
        },
    )
    .await
}

async fn write_websocket_handshake_response(stream: &mut tokio::net::TcpStream, request: &str) {
    let key = http_request_header(request, "sec-websocket-key")
        .expect("shared worker websocket request should carry key");
    let accept = websocket_accept_key(key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write shared worker websocket handshake response");
}

fn websocket_accept_key(key: &str) -> String {
    use base64::Engine as _;

    let source = format!("{}258EAFA5-E914-47DA-95CA-C5AB0DC85B11", key.trim());
    base64::engine::general_purpose::STANDARD.encode(moli_crypto::sha1_digest(source.as_bytes()))
}

fn http_request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (header_name, value) = line.split_once(':')?;
        header_name
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

async fn read_masked_websocket_text_frame(stream: &mut tokio::net::TcpStream) -> String {
    let mut header = [0_u8; 2];
    stream
        .read_exact(&mut header)
        .await
        .expect("read websocket frame header");
    assert_eq!(header[0] & 0x0f, 0x1, "expected text websocket frame");
    assert_ne!(
        header[1] & 0x80,
        0,
        "client websocket frames must be masked"
    );
    let mut len = u64::from(header[1] & 0x7f);
    if len == 126 {
        let mut extended = [0_u8; 2];
        stream
            .read_exact(&mut extended)
            .await
            .expect("read websocket frame u16 length");
        len = u64::from(u16::from_be_bytes(extended));
    } else if len == 127 {
        let mut extended = [0_u8; 8];
        stream
            .read_exact(&mut extended)
            .await
            .expect("read websocket frame u64 length");
        len = u64::from_be_bytes(extended);
    }
    assert!(len < 8192, "test websocket payload is unexpectedly large");
    let mut mask = [0_u8; 4];
    stream
        .read_exact(&mut mask)
        .await
        .expect("read websocket frame mask");
    let mut payload = vec![0_u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .expect("read websocket frame payload");
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    String::from_utf8(payload).expect("test websocket payload should be utf8")
}

async fn write_websocket_text_frame(stream: &mut tokio::net::TcpStream, payload: &str) {
    let bytes = payload.as_bytes();
    let mut frame = Vec::with_capacity(bytes.len() + 10);
    frame.push(0x81);
    match bytes.len() {
        len @ 0..=125 => frame.push(len as u8),
        len @ 126..=65535 => {
            frame.push(126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            frame.push(127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(bytes);
    stream
        .write_all(&frame)
        .await
        .expect("write websocket text frame");
}

async fn spawn_shared_worker_script_capture_http_server_for_request_count(
    script_body: &'static str,
    request_count: usize,
) -> (
    String,
    tokio::sync::oneshot::Receiver<Vec<String>>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shared worker multi-request capture server");
    let addr = listener
        .local_addr()
        .expect("shared worker multi-request capture server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..request_count {
            let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(Duration::from_secs(5), listener.accept()).await
            else {
                break;
            };
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read shared worker multi-request capture request");
            requests.push(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                script_body.len(),
                script_body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write shared worker multi-request capture response");
        }
        let _ = request_tx.send(requests);
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn spawn_cacheable_worker_partition_server(
    worker_kind: &'static str,
) -> (
    String,
    tokio::sync::oneshot::Receiver<usize>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind credentialless worker partition server");
    let addr = listener
        .local_addr()
        .expect("credentialless worker partition server addr");
    let (request_count_tx, request_count_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut worker_requests = 0usize;
        let mut request_count_tx = Some(request_count_tx);
        while worker_requests < 2 {
            let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(Duration::from_secs(5), listener.accept()).await
            else {
                break;
            };
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read credentialless worker partition request");
            if request.starts_with("GET /child.html ") {
                let body = "<!doctype html><body>child</body>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write credentialless worker child document response");
                continue;
            }
            assert!(
                request.starts_with("GET /worker.js "),
                "unexpected credentialless worker partition request:\n{request}"
            );
            worker_requests += 1;
            let label = if worker_requests == 1 {
                "credentialless"
            } else {
                "normal"
            };
            let body = if worker_kind == "shared" {
                format!(r#"onconnect = event => event.ports[0].postMessage("{label}");"#)
            } else {
                format!(r#"postMessage("{label}");"#)
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: null\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write credentialless worker partition response");
        }
        if let Some(tx) = request_count_tx.take() {
            let _ = tx.send(worker_requests);
        }
    });
    (format!("http://{addr}"), request_count_rx, server)
}

async fn spawn_third_party_shared_worker_same_site_http_server(
    same_site_option: &'static str,
) -> (
    String,
    tokio::sync::oneshot::Receiver<Vec<String>>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind third-party shared worker sameSite server");
    let addr = listener
        .local_addr()
        .expect("third-party shared worker sameSite server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for index in 0..2 {
            let accept_timeout = if index == 0 {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(500)
            };
            let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(accept_timeout, listener.accept()).await
            else {
                break;
            };
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read third-party shared worker sameSite request");
            let request_path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let response_body = if request_path.starts_with("/child.html") {
                let options_source = match same_site_option {
                    "default" => "const options = {};",
                    "none" => r#"const options = { sameSiteCookies: "none" };"#,
                    "all" => r#"const options = { sameSiteCookies: "all" };"#,
                    _ => "const options = {};",
                };
                format!(
                    r#"<!doctype html>
<meta charset="utf-8">
<script>
document.cookie = "sw_lax_cookie=sent; Path=/; SameSite=Lax";
try {{
    {options_source}
    const worker = new SharedWorker("/sw.js?mode={same_site_option}", options);
    worker.onerror = (event) => {{
        window.top.postMessage("error:" + event.message, "*");
    }};
    worker.port.onmessage = (event) => {{
        window.top.postMessage("message:" + event.data, "*");
    }};
    worker.port.start();
}} catch (error) {{
    window.top.postMessage("throw:" + error.name + ":" + error.message, "*");
}}
</script>
"#
                )
            } else if request_path.starts_with("/sw.js") {
                format!(
                    r#"onconnect = (event) => event.ports[0].postMessage("ok:{same_site_option}");"#
                )
            } else {
                "not found".to_owned()
            };
            let status =
                if request_path.starts_with("/child.html") || request_path.starts_with("/sw.js") {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
            let content_type = if request_path.starts_with("/child.html") {
                "text/html"
            } else {
                "application/javascript"
            };
            requests.push(request);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write third-party shared worker sameSite response");
        }
        let _ = request_tx.send(requests);
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn spawn_cross_origin_redirecting_shared_worker_script_servers()
-> (String, JoinHandle<()>, JoinHandle<()>) {
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shared worker redirect target server");
    let target_addr = target_listener
        .local_addr()
        .expect("shared worker redirect target addr");
    let target_url = format!("http://{target_addr}/redirect-target.js");
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target_listener
            .accept()
            .await
            .expect("accept shared worker redirect target request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read shared worker redirect target request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("shared worker redirect target request path");
        assert_eq!(request_path, "/redirect-target.js");
        let body = r#"onconnect = (event) => event.ports[0].postMessage("executed-cross-origin");"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write shared worker redirect target response");
    });

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shared worker redirect source server");
    let source_addr = source_listener
        .local_addr()
        .expect("shared worker redirect source addr");
    let source_base_url = format!("http://{source_addr}");
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept shared worker redirect source request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read shared worker redirect source request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("shared worker redirect source request path");
        assert_eq!(request_path, "/redirect-source.js");
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write shared worker redirect source response");
    });

    (source_base_url, source_server, target_server)
}

async fn spawn_sw_return_redirect_servers() -> (String, JoinHandle<()>, JoinHandle<()>) {
    let cross_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shared worker intermediate redirect server");
    let cross_addr = cross_listener
        .local_addr()
        .expect("shared worker intermediate redirect addr");

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shared worker returning redirect source server");
    let source_addr = source_listener
        .local_addr()
        .expect("shared worker returning redirect source addr");
    let source_base_url = format!("http://{source_addr}");
    let final_url = format!("{source_base_url}/redirect-target.js");
    let cross_url = format!("http://{cross_addr}/redirect-middle.js");

    // All network responses permit CORS: rejection must still account for
    // the cross-origin URL in the classic worker script's redirect history.
    let cross_server = tokio::spawn(async move {
        let (mut stream, _) = cross_listener
            .accept()
            .await
            .expect("accept shared worker intermediate redirect request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read shared worker intermediate redirect request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("shared worker intermediate redirect request path");
        assert_eq!(request_path, "/redirect-middle.js");
        let response = format!(
            "HTTP/1.1 302 Found\r\nAccess-Control-Allow-Origin: http://{source_addr}\r\nAccess-Control-Allow-Credentials: true\r\nLocation: {final_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write shared worker intermediate redirect response");
    });

    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept shared worker initial redirect request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read shared worker initial redirect request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("shared worker initial redirect request path");
        assert_eq!(request_path, "/redirect-source.js");
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {cross_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write shared worker initial redirect response");

        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept shared worker returning redirect target request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read shared worker returning redirect target request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("shared worker returning redirect target request path");
        assert_eq!(request_path, "/redirect-target.js");
        let body = r#"onconnect = (event) => event.ports[0].postMessage("executed-returned-same-origin");"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: null\r\nAccess-Control-Allow-Credentials: true\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write shared worker returning redirect target response");
    });

    (source_base_url, source_server, cross_server)
}

async fn drive_shared_worker_probe(page_vm: &mut PageVm, context: &str) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        page_vm
            .runtime_hooks
            .browser_context_runtime
            .drain_shared_worker_service_lane();
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {}
        let loader = page_vm.main_document_resource_loader();
        while page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {}
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        if page_vm
            .vm_mut()
            .eval("String(globalThis.__sharedWorkerDone === true)")?
            == "true"
        {
            return Ok(());
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
    }
    page_vm
        .runtime_hooks
        .browser_context_runtime
        .drain_shared_worker_service_lane();
    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let messages = page_vm
        .vm_mut()
        .eval("JSON.stringify(globalThis.__sharedWorkerMessages || null)")
        .unwrap_or_else(|error| format!("<failed to read messages: {error}>"));
    let done = page_vm
        .vm_mut()
        .eval("String(globalThis.__sharedWorkerDone)")
        .unwrap_or_else(|error| format!("<failed to read done: {error}>"));
    panic!("{context}; sharedWorkerMessages={messages}; sharedWorkerDone={done}");
}

async fn wait_for_shared_worker_client_count(
    page_vm: &mut PageVm,
    expected: usize,
    context: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        page_vm
            .runtime_hooks
            .browser_context_runtime
            .drain_shared_worker_service_lane();
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {}
        let loader = page_vm.main_document_resource_loader();
        while page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {}
        let actual = page_vm.vm().shared_worker_client_count_for_test();
        if actual == expected {
            return Ok(());
        }
        let loader = page_vm.main_document_resource_loader();
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
    }
    page_vm
        .runtime_hooks
        .browser_context_runtime
        .drain_shared_worker_service_lane();
    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let loader = page_vm.main_document_resource_loader();
    while page_vm
        .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
        .await?
    {}
    let actual = page_vm.vm().shared_worker_client_count_for_test();
    if actual == expected {
        return Ok(());
    }
    anyhow::bail!("{context}; expected shared worker client count {expected}, got {actual}");
}

async fn wait_for_child_shared_worker_owner_probe(
    page_vm: &mut PageVm,
    page_resource_source: &mut crate::page_task_queue::RendererPageResourceCompletionTestSource,
    shared_worker_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::shared_worker_runtime::SharedWorkerRuntimeOwnerWake,
    >,
    owner_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::page_task_queue::RendererOwnerWake,
    >,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if page_vm.vm_mut().eval(done_expression)? == "true" {
            return Ok(());
        }
        page_vm
            .runtime_hooks
            .browser_context_runtime
            .drain_shared_worker_service_lane();
        if page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(page_resource_source)?
            .is_some()
        {
            continue;
        }
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {
            if page_vm.vm_mut().eval(done_expression)? == "true" {
                return Ok(());
            }
        }
        let loader = page_vm.main_document_resource_loader();
        if page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {
            continue;
        }
        if page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
            .is_some()
        {
            continue;
        }
        let loader = page_vm.main_document_resource_loader();
        if page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::WindowMessage,
                loader.request_client(),
            )
            .await?
        {
            if page_vm.vm_mut().eval(done_expression)? == "true" {
                return Ok(());
            }
            continue;
        }
        let arrived = tokio::time::timeout_at(deadline, async {
            tokio::select! {
                wake = shared_worker_wake_rx.recv() => wake.is_some(),
                wake = owner_wake_rx.recv() => wake.is_some(),
                arrived = page_vm.wait_for_page_work_arrival_without_timeout(false) => arrived,
            }
        })
        .await
        .unwrap_or(false);
        if !arrived {
            break;
        }
    }

    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let loader = page_vm.main_document_resource_loader();
    while page_vm
        .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
        .await?
    {}
    let diagnostics = page_vm
        .vm_mut()
        .eval(
            r#"JSON.stringify({
                messages: globalThis.__childSharedWorkerOwnerMessages ?? null,
                done: globalThis.__childSharedWorkerOwnerDone,
                constructError: globalThis.__childSharedWorkerConstructError ?? null,
                handlerError: globalThis.__childSharedWorkerHandlerError ?? null
            })"#,
        )
        .unwrap_or_else(|error| format!("<failed to read owner probe diagnostics: {error}>"));
    let client_count = page_vm.vm().shared_worker_client_count_for_test();
    let ready_window_message_task = page_vm.vm().has_ready_window_message_task();
    let pending_activity = page_vm
        .vm_mut()
        .page_diagnostics_snapshot()
        .map(|snapshot| format!("{snapshot:?}"))
        .unwrap_or_else(|error| format!("<failed to read pending activity: {error}>"));
    panic!(
        "{context}; diagnostics={diagnostics}; shared_worker_clients={client_count}; ready_window_message_task={ready_window_message_task}; pending_activity={pending_activity}"
    );
}

fn shared_worker_probe_messages(page_vm: &mut PageVm) -> anyhow::Result<String> {
    page_vm
        .vm_mut()
        .eval("globalThis.__sharedWorkerMessages.join('|')")
}

fn shared_worker_console_entry(
    snapshot: &crate::runtime::RendererPageDiagnosticsSnapshot,
) -> Option<&crate::runtime::RuntimeConsoleMessageSnapshot> {
    snapshot
        .runtime_observable_source()?
        .source_items()
        .iter()
        .find_map(|item| match item {
            RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                if message.message == SHARED_WORKER_CONSOLE_MESSAGE =>
            {
                Some(message)
            }
            _ => None,
        })
}

fn has_console_probe_created_event(events: &[RendererSharedWorkerTargetEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            RendererSharedWorkerTargetEvent::Created(info)
                if info.name == "console-probe"
                    && info.url.starts_with("data:text/javascript,")
        )
    })
}

fn has_console_probe_target_console_event(events: &[RendererSharedWorkerTargetEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            RendererSharedWorkerTargetEvent::Console { message, .. }
                if message.message == SHARED_WORKER_CONSOLE_MESSAGE
        )
    })
}

async fn drain_until_shared_worker_console_activity(
    page_vm: &mut PageVm,
    output_rx: &mut crate::runtime::RendererOutputTransportReceiver,
) -> anyhow::Result<(
    crate::runtime::RendererPageDiagnosticsSnapshot,
    Vec<RendererSharedWorkerTargetEvent>,
)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut target_events = Vec::new();
    let mut last_snapshot = None;

    while Instant::now() < deadline {
        let loader = page_vm
            .main_document_resource_loader()
            .request_client()
            .clone();
        while page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
            .await?
        {}
        let snapshot = page_vm.page_diagnostics_snapshot()?;
        while let Ok(message) = output_rx.try_recv() {
            let crate::runtime::RendererOutputTransportMessage::Publication(output) = message
            else {
                continue;
            };
            target_events.extend(output.records().iter().filter_map(
                |record| match record.item() {
                    RendererOutputItem::OwnerAction(
                        RendererOwnerAction::SharedWorkerTargetLifecycle(event),
                    ) => Some(event.clone()),
                    _ => None,
                },
            ));
        }

        if has_console_probe_created_event(&target_events)
            && has_console_probe_target_console_event(&target_events)
            && shared_worker_console_entry(&snapshot).is_some()
        {
            return Ok((snapshot, target_events));
        }
        last_snapshot = Some(snapshot);

        let loader = page_vm.main_document_resource_loader();
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
    }

    anyhow::bail!(
        "timed out waiting for SharedWorker console activity; \
         target_events={target_events:?}; last_snapshot={last_snapshot:?}"
    );
}

async fn drain_until_websocket_trace_output(
    page_vm: &mut PageVm,
    url: &str,
) -> anyhow::Result<ScriptNetworkOutput> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut items = Vec::new();
    loop {
        let loader = page_vm
            .main_document_resource_loader()
            .request_client()
            .clone();
        while page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
            .await?
        {}
        items.extend(page_vm.vm_mut().take_network_output().into_items());
        if websocket_trace_output_is_complete(&items, url) || Instant::now() >= deadline {
            return Ok(ScriptNetworkOutput::from_items(items));
        }
        let arrived = tokio::time::timeout(
            Duration::from_millis(250),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await
        .unwrap_or(false);
        if !arrived {
            let loader = page_vm.main_document_resource_loader();
            page_vm
                .advance_timers_until_deadline_for_test(loader.request_client())
                .await?;
        }
    }
}

fn websocket_trace_output_is_complete(items: &[ScriptNetworkOutputItem], url: &str) -> bool {
    let Some(socket_id) = items.iter().find_map(|item| match item {
        ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
            if record.url().as_str() == url =>
        {
            record.websocket_socket_id()
        }
        _ => None,
    }) else {
        return false;
    };
    let frame_events = items
        .iter()
        .filter(|item| {
            matches!(
                item,
                ScriptNetworkOutputItem::WebSocketNetworkEvent(event)
                    if event.socket_id() == socket_id
            )
        })
        .count();
    let lifecycle_events = items
        .iter()
        .filter(|item| {
            matches!(
                item,
                ScriptNetworkOutputItem::WebSocketLifecycleEvent(event)
                    if event.socket_id() == socket_id
            )
        })
        .count();
    frame_events >= 2 && lifecycle_events >= 3
}

async fn drive_until_worker_completion_observed(
    page_vm: &mut PageVm,
    context: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let loader = page_vm.main_document_resource_loader();
    let mut progress_sources = Vec::new();

    while Instant::now() < deadline {
        if let Some(source) = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
        {
            progress_sources.push(format!("child:{source:?}"));
            continue;
        }
        while let Some(claimed) = page_vm.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
        ) {
            let event_kind = claimed
                .dedicated_worker_owner_and_event_kind()
                .map(|(_, event_kind)| event_kind)
                .expect("DedicatedWorker selector must retain its event kind");
            page_vm
                .run_claimed_selected_page_task_for_test(claimed, loader.request_client())
                .await?;
            progress_sources.push(format!("typed:{event_kind:?}"));
            if event_kind == crate::page_task_queue::RendererDedicatedWorkerClientEventKind::Message
            {
                return Ok(());
            }
        }
        while let Some(source) = page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
        {
            progress_sources.push(format!("websocket:{source:?}"));
        }

        let arrived = tokio::time::timeout(
            Duration::from_millis(250),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await
        .unwrap_or(false);
        if !arrived {
            progress_sources.push("wait:no-arrival".to_owned());
        }
    }

    anyhow::bail!(
        "{context}; timed out before observing worker completion; progress_sources={progress_sources:?}"
    );
}

async fn drive_window_message_until(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let loader = page_vm.main_document_resource_loader();
    let mut progress_sources = Vec::new();

    while Instant::now() < deadline {
        while let Some(source) = page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
        {
            progress_sources.push(format!("websocket:{source:?}"));
            if page_vm.vm_mut().eval(done_expression)? == "true" {
                return Ok(());
            }
        }
        if page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {
            progress_sources.push("typed:PageEvent".to_owned());
            if page_vm.vm_mut().eval(done_expression)? == "true" {
                return Ok(());
            }
            continue;
        }
        if page_vm.vm_mut().eval(done_expression)? == "true" {
            return Ok(());
        }
        let arrived = tokio::time::timeout(
            Duration::from_millis(100),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await
        .unwrap_or(false);
        if !arrived {
            progress_sources.push("wait:no-arrival".to_owned());
        }
    }

    let diagnostics = page_vm
        .vm_mut()
        .eval(
            r#"JSON.stringify({
                childWorkerMessages: globalThis.__childWorkerMessages ?? null,
                childWorkerDone: globalThis.__childWorkerDone ?? null,
                topBroadcastChannelMessages: globalThis.__topBroadcastChannelMessages ?? null,
                topBroadcastChannelDone: globalThis.__topBroadcastChannelDone ?? null
            })"#,
        )
        .unwrap_or_else(|error| format!("<failed to read diagnostics: {error}>"));
    anyhow::bail!("{context}; diagnostics={diagnostics}; progress_sources={progress_sources:?}");
}

async fn spawn_worker_script_path_capture_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local worker query encoding test server");
    let addr = listener.local_addr().expect("server local addr");
    let (path_tx, path_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker query encoding request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker query encoding request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker query encoding request path")
            .to_owned();
        let _ = path_tx.send(path);
        let body = "postMessage('ready');";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker query encoding response");
    });
    (format!("http://{addr}"), path_rx, server)
}

async fn spawn_audio_worklet_dynamic_descendant_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind AudioWorklet descendant test server");
    let addr = listener
        .local_addr()
        .expect("AudioWorklet descendant server addr");
    let server = tokio::spawn(async move {
        let (mut entry_stream, _) = listener
            .accept()
            .await
            .expect("accept AudioWorklet entry request");
        let (entry_request, entry_path) =
            read_request_head_and_path(&mut entry_stream, "AudioWorklet entry").await;
        assert_eq!(entry_path, "/worklet/entry.js");
        assert_audio_worklet_fetch_destination(&entry_request, "AudioWorklet entry");
        write_script_response(
            &mut entry_stream,
            [
                "import { a } from './a.js';",
                "import { b } from './b.js';",
                "globalThis.__moliAudioWorkletValue = `${a}${b}`;",
            ]
            .join("\n"),
            "AudioWorklet entry",
        )
        .await;

        let mut first_stream = None;
        let mut first_path = String::new();
        let mut second_stream = None;
        let mut second_path = String::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept AudioWorklet sibling request");
            let (request, path) =
                read_request_head_and_path(&mut stream, "AudioWorklet sibling").await;
            assert_audio_worklet_fetch_destination(&request, "AudioWorklet sibling");
            if first_stream.is_none() {
                first_path = path;
                first_stream = Some(stream);
            } else {
                second_path = path;
                second_stream = Some(stream);
            }
        }

        let (mut a_stream, mut b_stream) = match (
            first_path.as_str(),
            first_stream.expect("first AudioWorklet sibling stream"),
            second_path.as_str(),
            second_stream.expect("second AudioWorklet sibling stream"),
        ) {
            ("/worklet/a.js", a_stream, "/worklet/b.js", b_stream)
            | ("/worklet/b.js", b_stream, "/worklet/a.js", a_stream) => (a_stream, b_stream),
            (first, _, second, _) => {
                panic!("unexpected AudioWorklet sibling paths: {first}, {second}")
            }
        };

        write_script_response(
            &mut a_stream,
            "import { child } from './a-child.js'; export const a = `a${child}`;",
            "AudioWorklet a",
        )
        .await;

        let (mut child_stream, _) = listener
            .accept()
            .await
            .expect("accept AudioWorklet child request before b finishes");
        let (child_request, child_path) =
            read_request_head_and_path(&mut child_stream, "AudioWorklet child").await;
        assert_eq!(
            child_path, "/worklet/a-child.js",
            "AudioWorklet shim should inherit per-completion descendant expansion from worker dynamic import"
        );
        assert_audio_worklet_fetch_destination(&child_request, "AudioWorklet child");
        write_script_response(
            &mut child_stream,
            "export const child = 'child';",
            "AudioWorklet child",
        )
        .await;

        write_script_response(&mut b_stream, "export const b = 'b';", "AudioWorklet b").await;
    });

    (format!("http://{addr}"), server)
}

async fn spawn_audio_worklet_json_destination_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind AudioWorklet JSON destination server");
    let addr = listener
        .local_addr()
        .expect("AudioWorklet JSON destination server addr");
    let server = tokio::spawn(async move {
        let (mut entry_stream, _) = listener
            .accept()
            .await
            .expect("accept AudioWorklet JSON entry request");
        let (entry_request, entry_path) =
            read_request_head_and_path(&mut entry_stream, "AudioWorklet JSON entry").await;
        assert_eq!(entry_path, "/worklet/entry.js");
        assert_audio_worklet_fetch_destination(&entry_request, "AudioWorklet JSON entry");
        write_script_response(
            &mut entry_stream,
            [
                "import data from './data.json' with { type: 'json' };",
                "if (data.answer !== 42) throw new Error('missing JSON module');",
                "registerProcessor('json-destination', class extends AudioWorkletProcessor {});",
            ]
            .join("\n"),
            "AudioWorklet JSON entry",
        )
        .await;

        let (mut json_stream, _) = listener
            .accept()
            .await
            .expect("accept AudioWorklet JSON dependency request");
        let (json_request, json_path) =
            read_request_head_and_path(&mut json_stream, "AudioWorklet JSON dependency").await;
        assert_eq!(json_path, "/worklet/data.json");
        assert_fetch_destination(&json_request, "json", "AudioWorklet JSON dependency");
        let body = r#"{"answer":42}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        json_stream
            .write_all(response.as_bytes())
            .await
            .expect("write AudioWorklet JSON dependency response");
    });

    (format!("http://{addr}"), server)
}

async fn run_audio_worklet_static_invalid_module_type_import_test(
    module_type: &'static str,
    dependency_file: &'static str,
) {
    let (base_url, dependency_request_rx, server) =
        spawn_audio_worklet_static_invalid_module_type_server(module_type, dependency_file).await;
    let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
    let module_url = format!("{base_url}/worklet/entry.js");
    let module_url_literal =
        serde_json::to_string(&module_url).expect("serialize worklet module URL");
    let mut page_vm = test_page_vm_with_document_url(document_url);
    let local_executor = page_vm.local_executor.clone();

    let result = local_executor
        .run(async move {
            page_vm.vm_mut().eval(&format!(
                r#"
                    (() => {{
                        globalThis.__audioWorkletInvalidTypeResult = null;
                        globalThis.__audioWorkletInvalidTypeDone = false;
                        const context = new AudioContext();
                        context.audioWorklet.addModule({module_url_literal}).then(
                            () => {{
                                globalThis.__audioWorkletInvalidTypeResult = "loaded";
                                globalThis.__audioWorkletInvalidTypeDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletInvalidTypeResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletInvalidTypeDone = true;
                            }}
                        );
                    }})()
                "#
            ))?;
            drive_websocket_until_done(
                &mut page_vm,
                "String(globalThis.__audioWorkletInvalidTypeDone === true)",
                "AudioWorklet addModule invalid static module type should settle",
            )
            .await?;
            page_vm
                .vm_mut()
                .eval("globalThis.__audioWorkletInvalidTypeResult")
        })
        .await
        .expect("AudioWorklet invalid static module type test should run on owner lane");

    let dependency_request = dependency_request_rx
        .await
        .expect("AudioWorklet invalid static module type dependency probe should finish");
    server
        .await
        .expect("AudioWorklet invalid static module type server should finish");
    assert!(
        result.contains(&format!(
            "module type `{module_type}` is not a valid module type"
        )),
        "unexpected AudioWorklet invalid static module type result: {result}"
    );
    assert_eq!(
        dependency_request, None,
        "AudioWorklet invalid static module type must fail before fetching dependency"
    );
}

async fn spawn_audio_worklet_static_invalid_module_type_server(
    module_type: &'static str,
    dependency_file: &'static str,
) -> (
    String,
    tokio::sync::oneshot::Receiver<Option<String>>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind AudioWorklet invalid static module type server");
    let addr = listener
        .local_addr()
        .expect("AudioWorklet invalid static module type server addr");
    let (dependency_tx, dependency_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut entry_stream, _) = listener
            .accept()
            .await
            .expect("accept AudioWorklet invalid static module type entry request");
        let (entry_request, entry_path) =
            read_request_head_and_path(&mut entry_stream, "AudioWorklet invalid type entry").await;
        assert_eq!(entry_path, "/worklet/entry.js");
        assert_audio_worklet_fetch_destination(&entry_request, "AudioWorklet invalid type entry");
        write_script_response(
            &mut entry_stream,
            format!(
                "import value from './{dependency_file}' with {{ type: '{module_type}' }};\n\
                 registerProcessor('unexpected-invalid-type', class extends AudioWorkletProcessor {{}});"
            ),
            "AudioWorklet invalid type entry",
        )
        .await;

        let dependency_request =
            match tokio::time::timeout(Duration::from_millis(500), listener.accept()).await {
                Ok(Ok((mut stream, _))) => {
                    let dependency_path = read_request_path(
                        &mut stream,
                        "unexpected AudioWorklet invalid type dependency",
                    )
                    .await;
                    write_script_response(
                        &mut stream,
                        "export default 'unexpected';",
                        "unexpected AudioWorklet invalid type dependency",
                    )
                    .await;
                    Some(dependency_path)
                }
                Ok(Err(error)) => {
                    panic!("accept AudioWorklet invalid type dependency request: {error}")
                }
                Err(_) => None,
            };
        let _ = dependency_tx.send(dependency_request);
    });

    (format!("http://{addr}"), dependency_rx, server)
}

async fn spawn_audio_worklet_dynamic_import_forbidden_server() -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<Option<String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind AudioWorklet dynamic import rejection test server");
    let addr = listener
        .local_addr()
        .expect("AudioWorklet dynamic import rejection server addr");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut entry_stream, _) = listener
            .accept()
            .await
            .expect("accept AudioWorklet dynamic import rejection entry request");
        let entry_path = read_request_path(&mut entry_stream, "AudioWorklet dynamic entry").await;
        assert_eq!(entry_path, "/worklet/entry.js");
        write_script_response(
            &mut entry_stream,
            "await import('./dynamic.js'); registerProcessor('never', class extends AudioWorkletProcessor {});",
            "AudioWorklet dynamic entry",
        )
        .await;

        tokio::select! {
            accept = listener.accept() => {
                let (mut stream, _) = accept.expect("accept unexpected AudioWorklet dynamic dependency request");
                Some(read_request_path(&mut stream, "unexpected AudioWorklet dynamic dependency").await)
            }
            _ = stop_rx => None,
            _ = tokio::time::sleep(Duration::from_millis(500)) => None,
        }
    });

    (format!("http://{addr}"), stop_tx, server)
}

async fn spawn_audio_worklet_repeated_add_module_server() -> (
    String,
    tokio::sync::oneshot::Receiver<usize>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind repeated AudioWorklet addModule server");
    let addr = listener
        .local_addr()
        .expect("repeated AudioWorklet addModule server addr");
    let (request_count_tx, request_count_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut request_count = 0usize;
        loop {
            let accept_result = if request_count == 0 {
                Some(
                    listener
                        .accept()
                        .await
                        .expect("accept first repeated AudioWorklet module request"),
                )
            } else {
                match tokio::time::timeout(Duration::from_millis(500), listener.accept()).await {
                    Ok(Ok(accepted)) => Some(accepted),
                    Ok(Err(error)) => {
                        panic!("accept repeated AudioWorklet module request: {error}")
                    }
                    Err(_) => None,
                }
            };
            let Some((mut stream, _)) = accept_result else {
                break;
            };
            let entry_path = read_request_path(&mut stream, "repeated AudioWorklet entry").await;
            assert_eq!(entry_path, "/worklet/entry.js");
            request_count += 1;
            write_script_response(
                &mut stream,
                "registerProcessor('cached', class extends AudioWorkletProcessor {});",
                "repeated AudioWorklet entry",
            )
            .await;
            if request_count >= 2 {
                break;
            }
        }
        let _ = request_count_tx.send(request_count);
    });

    (format!("http://{addr}"), request_count_rx, server)
}

async fn spawn_audio_worklet_failed_repeated_add_module_server() -> (
    String,
    tokio::sync::oneshot::Receiver<usize>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed repeated AudioWorklet addModule server");
    let addr = listener
        .local_addr()
        .expect("failed repeated AudioWorklet addModule server addr");
    let (request_count_tx, request_count_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut request_count = 0usize;
        loop {
            let accept_result = if request_count == 0 {
                Some(
                    listener
                        .accept()
                        .await
                        .expect("accept first failed repeated AudioWorklet module request"),
                )
            } else {
                match tokio::time::timeout(Duration::from_millis(500), listener.accept()).await {
                    Ok(Ok(accepted)) => Some(accepted),
                    Ok(Err(error)) => {
                        panic!("accept failed repeated AudioWorklet module request: {error}")
                    }
                    Err(_) => None,
                }
            };
            let Some((mut stream, _)) = accept_result else {
                break;
            };
            let entry_path =
                read_request_path(&mut stream, "failed repeated AudioWorklet entry").await;
            assert_eq!(entry_path, "/worklet/entry.js");
            request_count += 1;
            let body = "missing worklet module";
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write failed repeated AudioWorklet entry response");
            if request_count >= 2 {
                break;
            }
        }
        let _ = request_count_tx.send(request_count);
    });

    (format!("http://{addr}"), request_count_rx, server)
}

async fn spawn_audio_worklet_hanging_add_module_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind hanging AudioWorklet addModule server");
    let addr = listener
        .local_addr()
        .expect("hanging AudioWorklet addModule server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept hanging AudioWorklet module request");
        let entry_path = read_request_path(&mut stream, "hanging AudioWorklet entry").await;
        let _ = request_tx.send(entry_path);
        let _ = release_rx.await;
        let body = "registerProcessor('late', class extends AudioWorkletProcessor {});";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes()).await;
    });

    (format!("http://{addr}"), request_rx, release_tx, server)
}

async fn read_request_path(stream: &mut tokio::net::TcpStream, context: &str) -> String {
    read_request_head_and_path(stream, context).await.1
}

async fn read_request_head_and_path(
    stream: &mut tokio::net::TcpStream,
    context: &str,
) -> (String, String) {
    let request = read_http_request_head(stream)
        .await
        .unwrap_or_else(|error| panic!("read {context} request: {error}"));
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_else(|| panic!("{context} request path"))
        .to_owned();
    (request, path)
}

fn assert_audio_worklet_fetch_destination(request: &str, context: &str) {
    assert_fetch_destination(request, "audioworklet", context);
}

fn assert_fetch_destination(request: &str, destination: &str, context: &str) {
    assert!(
        request
            .to_ascii_lowercase()
            .contains(&format!("sec-fetch-dest: {destination}\r\n")),
        "{context} must use the {destination} fetch destination, request was:\n{request}"
    );
}

async fn write_script_response(
    stream: &mut tokio::net::TcpStream,
    body: impl AsRef<str>,
    context: &str,
) {
    let body = body.as_ref();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .unwrap_or_else(|error| panic!("write {context} response: {error}"));
}

mod audio_worklet_and_message_ports;
mod message_port_transfer_semantics;
mod service_and_shared_worker_lifecycle;
mod worker_messages_and_service_workers;
mod worker_security_and_audio_worklets;
