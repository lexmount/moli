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
            "HTTP/1.1 302 Found\r\nLocation: {final_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
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
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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

#[tokio::test]
async fn worker_post_message_flows_through_page_client_event_source() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerMessage = null;
                        globalThis.__workerMessageShape = null;
                        globalThis.__workerDone = false;
                        const worker = new Worker("data:text/javascript,postMessage('from-worker')");
                        worker.onmessage = (event) => {
                            globalThis.__workerMessage = event.data;
                            globalThis.__workerMessageShape = [
                                event instanceof MessageEvent,
                                Object.prototype.toString.call(event),
                                event.type
                            ].join("|");
                            globalThis.__workerDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker runtime event should arrive",
                )
                .await?;
                assert_eq!(page_vm.vm_mut().eval("globalThis.__workerMessage")?, "from-worker");
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__workerMessageShape")?,
                    "true|[object MessageEvent]|message"
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker event test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn dedicated_worker_main_scripts_publish_split_target_lifecycle_records() {
    run_page_vm_async_test(async move {
        let external_source = "postMessage('external-ready');";
        let blob_source = "postMessage('blob-ready');";
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/worker.js",
            "HTTP/1.1 200 OK",
            external_source.to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let external_url = Url::parse(&format!("{base_url}/worker.js")).expect("worker URL");
        let mut page_vm = test_page_vm_with_document_url(document_url.clone());
        let output_journal = crate::runtime::RendererTurnOutputJournal::new(
            crate::runtime::RendererOutputStreamIdentity::new_page_for_protocol_test(
                page_vm.page_id,
            ),
        );
        page_vm
            .vm_mut()
            .bind_renderer_output_journal_for_test(output_journal.clone());
        let local_executor = page_vm.local_executor.clone();

        let target_events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__workerMainScriptMessages = [];
  globalThis.__workerMainScriptDone = false;
  const onmessage = event => {{
    __workerMainScriptMessages.push(String(event.data));
    __workerMainScriptDone = __workerMainScriptMessages.length === 2;
  }};
  globalThis.__externalMainScriptWorker = new Worker("/worker.js#runtime-fragment");
  __externalMainScriptWorker.onmessage = onmessage;
  const blobUrl = URL.createObjectURL(new Blob(
    [{blob_source:?}],
    {{ type: "text/javascript" }}
  ));
  globalThis.__blobMainScriptWorker = new Worker(blobUrl);
  __blobMainScriptWorker.onmessage = onmessage;
}})()
"#,
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerMainScriptDone === true)",
                    "external and blob worker scripts should complete",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__workerMainScriptMessages.sort())")?,
                    r#"["blob-ready","external-ready"]"#
                );
                assert!(
                    page_vm.vm_mut().take_network_output().is_empty(),
                    "DedicatedWorker main scripts are not complete Page subresources"
                );
                let publication = output_journal
                    .settle()
                    .expect("DedicatedWorker target events should settle as Page output");
                anyhow::Ok(
                    publication
                        .into_records()
                        .into_iter()
                        .filter_map(|record| match record.into_parts().1 {
                            RendererOutputItem::OwnerAction(
                                RendererOwnerAction::DedicatedWorkerTargetLifecycle(event),
                            ) => Some(event),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .await
            .expect("worker main-script Network test should run on owner lane");

        server
            .await
            .expect("worker main-script Network server should finish");
        let created = target_events
            .iter()
            .filter_map(|event| match event {
                crate::runtime::RendererDedicatedWorkerTargetEvent::Created(info) => Some(info),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(created.len(), 2, "events: {target_events:#?}");
        assert!(
            created
                .iter()
                .all(|info| info.document_url == document_url.as_str())
        );
        let external_created = created
            .iter()
            .copied()
            .find(|info| info.request_url == external_url.as_str())
            .expect("external Worker target creation");
        let blob_created = created
            .iter()
            .copied()
            .find(|info| info.request_url.starts_with("blob:"))
            .expect("blob Worker target creation");

        let loaded = target_events
            .iter()
            .filter_map(|event| match event {
                crate::runtime::RendererDedicatedWorkerTargetEvent::ScriptLoaded {
                    instance_id,
                    script_url,
                    response,
                } => Some((*instance_id, script_url, response.as_ref())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(loaded.len(), 2, "events: {target_events:#?}");
        let (_, external_script_url, external_response) = loaded
            .iter()
            .copied()
            .find(|(instance_id, _, _)| *instance_id == external_created.instance_id)
            .expect("external Worker main-script completion");
        assert_eq!(
            external_script_url,
            &format!("{external_url}#runtime-fragment")
        );
        assert_eq!(external_response.status, 200);
        assert_eq!(external_response.body_text(), external_source);
        assert!(external_response.network_request_headers().is_some());
        assert_eq!(
            external_response.negotiated_http_version,
            Some(moli_fetch::NegotiatedHttpVersion::Http11)
        );

        let (_, blob_script_url, blob_response) = loaded
            .iter()
            .copied()
            .find(|(instance_id, _, _)| *instance_id == blob_created.instance_id)
            .expect("blob Worker main-script completion");
        assert_eq!(blob_script_url, &blob_created.request_url);
        assert_eq!(blob_response.status, 200);
        assert_eq!(blob_response.body_text(), blob_source);
        assert_eq!(blob_response.network_request_headers(), None);
        assert_eq!(blob_response.negotiated_http_version, None);
    })
    .await;
}

#[tokio::test]
async fn worker_message_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://worker-ready-source.test/page.html").expect("document URL");
        let (page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            events_after_worker_message,
            script_ready_source,
            events_after_script_ready,
            lifecycle_and_host_load_sources,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__workerReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  globalThis.__workerReadyWorker = new Worker(
    "data:text/javascript,postMessage('go')"
  );
  __workerReadyWorker.onmessage = (event) => {
    __workerReadyEvents.push("message:" + event.data);
    const frame = document.createElement("iframe");
    frame.onload = () => __workerReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__workerReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  };
})()
"#,
                )?;

                let mut completion_sources = Vec::new();
                let events_after_worker_message = loop {
                    if let Some(claimed) = page_vm.claim_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                    ) {
                        let event_kind = claimed
                            .dedicated_worker_owner_and_event_kind()
                            .map(|(_, event_kind)| event_kind)
                            .expect("DedicatedWorker selector must retain its event kind");
                        page_vm
                            .run_claimed_selected_page_task_for_test(claimed, &loader)
                            .await?;
                        completion_sources.push(RendererOwnerResourceActivitySource::Worker);
                        let events = page_vm.vm_mut().eval("__workerReadyEvents.join('|')")?;
                        if events == "message:go" {
                            assert_eq!(
                                event_kind,
                                crate::page_task_queue::RendererDedicatedWorkerClientEventKind::Message
                            );
                            break events;
                        }
                        assert!(
                            completion_sources.len() < 16,
                            "worker message handler should run after bounded completions; sources: {completion_sources:?}, events: {events}"
                        );
                        continue;
                    }
                    if page_vm.has_ready_page_websocket_task_for_test() {
                        let completion_source = page_vm
                            .run_exact_page_websocket_selected_task_for_test().await?
                            .expect("advertised WebSocket task should remain ready");
                        completion_sources.push(completion_source);
                        continue;
                    }
                    {
                        let wake = tokio::time::timeout(
                            Duration::from_secs(2),
                            owner_wake_rx.recv(),
                        )
                        .await
                        .expect("worker completion should signal its owner before timeout")
                        .expect("worker completion owner-wake route should remain open");
                        assert_eq!(
                            wake.page_id(),
                            PageId::new_for_testing(1),
                            "worker task wake must remain attached to the originating Page"
                        );
                    }
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "worker-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "worker-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready =
                    page_vm.vm_mut().eval("__workerReadyEvents.join('|')")?;
                let mut lifecycle_and_host_load_sources = Vec::new();
                let events_after_host_load = loop {
                    let source = page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await
                        .expect("child lifecycle or HostLoad source should remain ready");
                    lifecycle_and_host_load_sources.push(source);
                    let events = page_vm.vm_mut().eval("__workerReadyEvents.join('|')")?;
                    if events == "message:go|child-script:true|frame-load" {
                        break events;
                    }
                    assert_eq!(
                        source,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "only document-owned lifecycle turns may precede the final HostLoad delivery"
                    );
                    assert!(
                        lifecycle_and_host_load_sources.len() < 8,
                        "worker-created child lifecycle should reach HostLoad in bounded owner turns: {lifecycle_and_host_load_sources:?}"
                    );
                };

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_worker_message,
                    script_ready_source,
                    events_after_script_ready,
                    lifecycle_and_host_load_sources,
                    events_after_host_load,
                ))
            })
            .await
            .expect("worker ready-work source test should run");

        assert!(
            completion_sources.contains(&RendererOwnerResourceActivitySource::Worker),
            "worker handler should be driven by a Worker completion: {completion_sources:?}"
        );
        assert_eq!(
            events_after_worker_message, "message:go",
            "worker message handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "worker-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "message:go|child-script:true",
            "child parser work should run on the later DocumentScriptReady turn"
        );
        assert!(
            lifecycle_and_host_load_sources.len() >= 2,
            "document-owned lifecycle must complete before HostLoad: {lifecycle_and_host_load_sources:?}"
        );
        assert!(
            lifecycle_and_host_load_sources[..lifecycle_and_host_load_sources.len() - 1]
                .iter()
                .all(|source| *source == ChildFrameSemanticTurnKind::DocumentLifecycle),
            "only DocumentLifecycle turns may run between script execution and load delivery: {lifecycle_and_host_load_sources:?}"
        );
        assert_eq!(
            lifecycle_and_host_load_sources.last(),
            Some(&ChildFrameSemanticTurnKind::HostLoad),
            "iframe load must remain a later HostLoad turn after document lifecycle"
        );
        assert_eq!(
            events_after_host_load, "message:go|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );
    })
    .await;
}

#[tokio::test]
async fn shared_worker_error_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/missing-shared-worker.js",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            events_after_shared_worker_error,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__sharedWorkerReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const worker = new SharedWorker("/missing-shared-worker.js", "ready-output-error");
  worker.onerror = (event) => {
    __sharedWorkerReadyEvents.push("error:" + event.type);
    const frame = document.createElement("iframe");
    frame.onload = () => __sharedWorkerReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__sharedWorkerReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  };
  worker.port.start();
})()
"#,
                )?;

                let deadline = Instant::now() + Duration::from_secs(10);
                let events_after_shared_worker_error = loop {
                    // This direct PageVm fixture has no render-owner loop.
                    // Admit the SharedWorker service result explicitly before
                    // selecting its Page task; timer/WebSocket helpers must
                    // not provide this unrelated owner responsibility.
                    page_vm
                        .runtime_hooks
                        .browser_context_runtime
                        .drain_shared_worker_service_lane();
                    let loader = page_vm.main_document_resource_loader();
                    let shared_worker_event_ran = page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::SharedWorkerClientEvent,
                            loader.request_client(),
                        )
                        .await?;
                    if !shared_worker_event_ran
                        && page_vm.has_ready_page_websocket_task_for_test()
                    {
                        let _ = page_vm.run_exact_page_websocket_selected_task_for_test().await?;
                    } else if !shared_worker_event_ran {
                        page_vm
                            .advance_timers_until_deadline_for_test(loader.request_client())
                            .await?;
                        let _ = tokio::time::timeout(
                            Duration::from_millis(100),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    let events = page_vm.vm_mut().eval("__sharedWorkerReadyEvents.join('|')")?;
                    if events == "error:error" {
                        break events;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "SharedWorker error handler should run after bounded owner turns; events: {events}"
                    );
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "SharedWorker-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "SharedWorker-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__sharedWorkerReadyEvents.join('|')")?;
                let host_load_source = Some(
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "SharedWorker-created child iframe load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__sharedWorkerReadyEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    events_after_shared_worker_error,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    events_after_host_load,
                ))
            })
            .await
            .expect("SharedWorker ready-work source test should run");

        assert_eq!(
            events_after_shared_worker_error, "error:error",
            "SharedWorker error handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "SharedWorker-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "error:error|child-script:true",
            "child parser work should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a separate HostLoad turn after SharedWorker error dispatch"
        );
        assert_eq!(
            events_after_host_load, "error:error|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server
            .await
            .expect("SharedWorker ready-work server should finish");
    })
    .await;
}

#[tokio::test]
async fn blob_worker_created_in_data_iframe_inherits_opaque_broadcast_channel_owner_key() {
    run_page_vm_async_test(async move {
        let document_url =
            Url::parse("https://broadcast-channel-opaque-worker.test/page.html")
                .expect("document url");
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__opaqueWorkerBroadcastChannelMessages = [];
                        globalThis.__opaqueWorkerBroadcastChannelDone = false;
                        const topChannel = new BroadcastChannel("opaque-child-worker-owner");
                        topChannel.onmessage = event => {
                            __opaqueWorkerBroadcastChannelMessages.push("top:" + event.data + ":" + event.origin);
                        };
                        addEventListener("message", event => {
                            const value = String(event.data);
                            __opaqueWorkerBroadcastChannelMessages.push(value);
                            if (value === "child-worker:null:null") {
                                __opaqueWorkerBroadcastChannelDone = true;
                            }
                        });

                        const frame = document.createElement("iframe");
                        frame.src = "data:text/html," + encodeURIComponent(`
                            <!doctype html>
                            <script>
                                const channel = new BroadcastChannel("opaque-child-worker-owner");
                                channel.onmessage = event => {
                                    if (event.data === "ping") {
                                        channel.postMessage("pong");
                                    } else {
                                        parent.postMessage("child-worker:" + event.data + ":" + event.origin, "*");
                                    }
                                };
                                const workerSource = \`
                                    const workerChannel = new BroadcastChannel("opaque-child-worker-owner");
                                    workerChannel.postMessage("ping");
                                    workerChannel.onmessage = event => workerChannel.postMessage(event.origin);
                                \`;
                                const workerUrl = URL.createObjectURL(
                                    new Blob([workerSource], { type: "text/javascript" })
                                );
                                const worker = new Worker(workerUrl);
                                worker.onerror = event => parent.postMessage("worker-error:" + event.message, "*");
                            <\/script>
                        `);
                        (document.body || document.documentElement || document).appendChild(frame);
                    })()
                    "#,
                )?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__opaqueWorkerBroadcastChannelDone === true)",
                    "opaque child blob worker BroadcastChannel delivery should complete",
                )
                .await?;

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__opaqueWorkerBroadcastChannelMessages)")?,
                    r#"["child-worker:null:null"]"#
                );
                anyhow::Ok(())
            })
            .await
            .expect("opaque child blob worker BroadcastChannel test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn child_message_handler_external_dedicated_worker_binds_child_client_event_owner() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/worker.js",
            "HTTP/1.1 200 OK",
            "postMessage('worker-loaded');".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__childMessageWorkerCreated = false;
  globalThis.__childMessageWorkerLoaded = false;
  window.addEventListener("message", event => {
    if (event.data && event.data.kind === "child-message-worker-created") {
      __childMessageWorkerCreated = true;
    }
    if (event.data && event.data.kind === "child-message-worker-loaded") {
      __childMessageWorkerLoaded = event.data.value === "worker-loaded";
    }
  });

  const frame = document.createElement("iframe");
  frame.id = "child-message-worker-owner";
  frame.srcdoc = `
    <!doctype html>
    <script>
      onmessage = event => {
        if (event.data !== "start-worker") return;
        const worker = new Worker("/worker.js");
        worker.onmessage = event => {
          parent.postMessage({
            kind: "child-message-worker-loaded",
            value: event.data
          }, "*");
        };
        parent.postMessage({ kind: "child-message-worker-created" }, "*");
      };
    <\/script>
  `;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
                )?;

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "message-target child navigation",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "message-target child script ready",
                )
                .await;
                let child_handle = page_vm
                    .vm()
                    .element_handle_by_id_for_test("child-message-worker-owner")
                    .expect("child worker owner iframe handle");
                let child_owner = page_vm
                    .vm()
                    .current_child_document_task_owner(child_handle)
                    .expect("child worker owner document");

                page_vm.vm_mut().eval(
                    r#"
document
  .getElementById("child-message-worker-owner")
  .contentWindow
  .postMessage("start-worker", "*");
"posted"
"#,
                )?;
                drive_window_message_until(
                    &mut page_vm,
                    "String(globalThis.__childMessageWorkerLoaded === true)",
                    "child message handler should create and load its Worker",
                )
                .await?;

                let workers = page_vm.vm().dedicated_worker_execution_contexts_for_test();
                assert_eq!(workers.len(), 1);
                let worker_id = workers[0].0;
                assert_eq!(
                    workers[0].1,
                    crate::native_bridge::WindowExecutionContextOwner::Frame(
                        child_owner.local_window_id
                    ),
                    "the child message-created Worker must retain the child LocalWindow owner"
                );
                let identity = page_vm
                    .vm()
                    .current_dedicated_worker_client_event_identity(worker_id)
                    .expect("child Worker client-event identity should remain current");
                assert_eq!(
                    identity.owner(),
                    crate::native_bridge::WindowExecutionContextOwner::Frame(
                        child_owner.local_window_id
                    ),
                    "the Worker client-event producer must not bind the top Window identity"
                );
                assert_eq!(
                    identity.dispatch_scope(),
                    crate::native_bridge::OwnerDispatchScope::Child(child_handle)
                );
                anyhow::Ok(())
            })
            .await
            .expect("child message-created Worker owner test should run on owner lane");

        server
            .await
            .expect("child message-created Worker server should finish");
    })
    .await;
}

#[tokio::test]
async fn child_worker_message_without_listener_drops_and_restores_top_owner_scope() {
    run_page_vm_async_test(async move {
        let document_url =
            Url::parse("https://worker-owner-restore.test/page.html").expect("document url");
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__topBroadcastChannelMessages = [];
                        globalThis.__topBroadcastChannelDone = false;
                        globalThis.__childWorkerMessages = [];
                        globalThis.__childWorkerDone = false;
                        window.addEventListener("message", event => {
                            if (event.data && event.data.kind === "child-worker-result") {
                                globalThis.__childWorkerMessages = event.data.messages;
                                globalThis.__childWorkerDone = true;
                            }
                        });
                        const receiver = new BroadcastChannel("owner-restore-after-worker");
                        receiver.onmessage = event => {
                            __topBroadcastChannelMessages.push(event.data + ":" + event.origin);
                            __topBroadcastChannelDone = true;
                        };

                        const frame = document.createElement("iframe");
                        frame.src = "data:text/html," + encodeURIComponent(`
                            <!doctype html>
                            <script>
                                const worker = new Worker("data:text/javascript,onmessage = () => postMessage('after-listener'); postMessage('before-listener')");
                                const messages = [];
                                addEventListener("message", event => {
                                    if (event.data !== "install-late-listener") return;
                                    worker.onmessage = event => {
                                        messages.push(event.data);
                                        parent.postMessage({
                                            kind: "child-worker-result",
                                            messages
                                        }, "*");
                                    };
                                    worker.postMessage("go");
                                });
                            <\/script>
                        `);
                        (document.body || document.documentElement || document).appendChild(frame);
                    })()
                    "#,
                )?;

                drive_until_worker_completion_observed(
                    &mut page_vm,
                    "child worker no-listener owner restore setup",
                )
                .await?;

                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        document.querySelector("iframe").contentWindow.postMessage("install-late-listener", "*");
                        const sender = new BroadcastChannel("owner-restore-after-worker");
                        sender.postMessage("top-still-active");
                    })()
                    "#,
                )?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__topBroadcastChannelDone === true)",
                    "top BroadcastChannel should still use top owner after child worker no-listener completion",
                )
                .await?;

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__topBroadcastChannelMessages)")?,
                    r#"["top-still-active:https://worker-owner-restore.test"]"#
                );
                drive_window_message_until(
                    &mut page_vm,
                    "String(globalThis.__childWorkerDone === true)",
                    "child worker late listener result delivery",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__childWorkerMessages)")?,
                    r#"["after-listener"]"#
                );
                anyhow::Ok(())
            })
            .await
            .expect("child worker no-listener owner restore test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn message_port_handler_worker_uses_lightweight_popup_owner_scope() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://message-port-worker-popup-owner.test/page.html")
            .expect("document url");
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        const topChannel = new BroadcastChannel("message-port-popup-worker-owner");
                        topChannel.onmessage = event => {
                            __wsEvents.push("top-bc:" + event.data + ":" + event.origin);
                        };
                        addEventListener("message", event => {
                            const value = String(event.data);
                            __wsEvents.push(value + ":" + event.origin);
                            if (value === "popup-worker:worker-origin") {
                                __wsDone = true;
                            }
                        });

                        const popup = open("https://message-port-popup-worker-child.test/page.html");
                        popup.onmessage = event => {
                            if (event.data !== "setup") {
                                return;
                            }
                            const popupChannel = new BroadcastChannel("message-port-popup-worker-owner");
                            popupChannel.onmessage = channelEvent => {
                                event.source.postMessage("popup-worker:" + channelEvent.data, event.origin);
                            };
                            const channel = new MessageChannel();
                            channel.port2.onmessage = () => {
                                const workerSource = `
                                    const channel = new BroadcastChannel("message-port-popup-worker-owner");
                                    channel.postMessage("worker-origin");
                                    channel.onmessage = event => channel.postMessage(event.origin);
                                `;
                                const workerUrl = URL.createObjectURL(
                                    new Blob([workerSource], { type: "text/javascript" })
                                );
                                const worker = new Worker(workerUrl);
                                worker.onerror = error => event.source.postMessage("worker-error:" + error.message, event.origin);
                            };
                            channel.port1.postMessage("start");
                        };
                        popup.postMessage("setup", "*");
                    })()
                    "#,
                )?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "popup MessagePort handler Worker should use popup owner scope",
                )
                .await?;

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__wsEvents)")?,
                    r#"["popup-worker:worker-origin:https://message-port-popup-worker-child.test"]"#
                );
                anyhow::Ok(())
            })
            .await
            .expect("popup MessagePort Worker owner test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn worker_pending_activity_diagnostics_split_loading_and_running_worker_isolates() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/ready-worker.js",
            "HTTP/1.1 200 OK",
            "postMessage('ready');".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerDiagnosticDone = false;
                        const worker = new Worker("/ready-worker.js");
                        worker.onmessage = () => {
                            globalThis.__workerDiagnosticDone = true;
                        };
                    })()
                    "#,
                )?;

                let loading_snapshot = page_vm.page_diagnostics_snapshot()?;
                assert_eq!(
                    loading_snapshot.diagnostics.dedicated_worker_loading_count,
                    1
                );
                assert_eq!(
                    loading_snapshot
                        .diagnostics
                        .dedicated_worker_running_worker_isolate_count,
                    0
                );

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDiagnosticDone === true)",
                    "worker diagnostics probe should reach the running worker",
                )
                .await?;

                let running_snapshot = page_vm.page_diagnostics_snapshot()?;
                assert_eq!(
                    running_snapshot.diagnostics.dedicated_worker_loading_count,
                    0
                );
                assert_eq!(
                    running_snapshot
                        .diagnostics
                        .dedicated_worker_running_worker_isolate_count,
                    1
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker diagnostics test should run on owner lane");
        server
            .await
            .expect("worker diagnostics server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_register_starts_module_worker_global() {
    run_page_vm_async_test(async move {
        let (base_url, finished_rx, server) =
            spawn_service_worker_execution_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expected_scope = format!("{base_url}/app/");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__serviceWorkerRegisterSettled = "pending";
                    navigator.serviceWorker.register("/sw.js", {
                        scope: "/app/",
                        type: "module"
                    }).then(
                        async registration => {
                            await navigator.serviceWorker.ready;
                            const worker = registration.active ?? registration.waiting ?? registration.installing;
                            globalThis.__serviceWorkerRegisterSettled =
                                registration.scope + "|" + worker.state;
                        },
                        error => {
                            globalThis.__serviceWorkerRegisterSettled = "rejected:" + error.name;
                        }
                    );
                    "#,
                )?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__serviceWorkerRegisterSettled")?,
                    "pending"
                );
                let body = tokio::time::timeout(Duration::from_secs(5), finished_rx)
                    .await
                    .expect("service worker script should POST /finished")
                    .expect("service worker execution capture sender should stay alive");
                assert_eq!(
                    body,
                    format!(
                        "[object ServiceWorkerGlobalScope]|true|{}|function|function",
                        expected_scope
                    )
                );
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                let loader = page_vm.main_document_resource_loader();
                while tokio::time::Instant::now() < deadline {
                    page_vm
                        .runtime_hooks
                        .browser_context_runtime
                        .drain_service_worker_service_lane();
                    while page_vm
                        .run_one_oldest_ready_page_task_on_owner_lane_for_test(
                            loader.request_client(),
                        )
                        .await?
                    {
                        page_vm
                            .runtime_hooks
                            .browser_context_runtime
                            .drain_service_worker_service_lane();
                    }
                    if page_vm.vm_mut().eval("globalThis.__serviceWorkerRegisterSettled")?
                        == format!("{expected_scope}|activated")
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert!(
                    page_vm.vm_mut().eval("globalThis.__serviceWorkerRegisterSettled")?
                        == format!("{expected_scope}|activated"),
                    "service worker should activate after register and lifecycle completions"
                );
                let diagnostics = page_vm
                    .runtime_hooks
                    .browser_context_runtime
                    .moli_memory_diagnostics();
                assert_eq!(diagnostics["serviceWorker"]["runtimeRegistrations"], 1);
                assert_eq!(diagnostics["serviceWorker"]["versions"], 1);
                assert_eq!(diagnostics["serviceWorker"]["startingVersions"], 0);
                assert_eq!(diagnostics["serviceWorker"]["runningVersions"], 1);
                assert_eq!(diagnostics["serviceWorker"]["runningWorkers"], 1);
                anyhow::Ok(())
            })
            .await
            .expect("service worker execution test should run on owner lane");
        server
            .await
            .expect("service worker execution capture server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_intercepts_dedicated_worker_main_script() {
    run_page_vm_async_test(async move {
        let (base_url, worker_request_rx, server) =
            spawn_service_worker_worker_main_script_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expected_worker_url = format!("{base_url}/app/worker.js");
        let expected_service_worker_url = format!("{base_url}/app/sw.js");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerWorkerMainProbe = "pending";
                        (async () => {
                            await navigator.serviceWorker.register("sw.js", { scope: "./" });
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {
                                await new Promise(resolve => {
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        { once: true }
                                    );
                                });
                            }
                            const worker = new Worker("worker.js");
                            worker.onmessage = event => {
                                globalThis.__serviceWorkerWorkerMainProbe = event.data;
                            };
                            worker.onerror = event => {
                                globalThis.__serviceWorkerWorkerMainProbe =
                                    "error:" + event.message;
                            };
                        })().catch(error => {
                            globalThis.__serviceWorkerWorkerMainProbe =
                                "error:" + String(error && error.message);
                        });
                    })()
                    "#,
                )?;
                drive_service_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerWorkerMainProbe !== 'pending')",
                    "service worker should intercept dedicated Worker main script",
                )
                .await?;
                let result: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerWorkerMainProbe)")?,
                )
                .expect("dedicated worker controller result should be JSON");
                assert_eq!(
                    result,
                    serde_json::json!({
                        "main": format!("sw-main:{expected_worker_url}"),
                        "serviceWorkerType": "object",
                        "controllerScriptURL": expected_service_worker_url,
                        "controllerState": "activated",
                        "oncontrollerchangeIsNull": true,
                        "addEventListenerType": "function",
                    })
                );
                anyhow::Ok(())
            })
            .await
            .expect("service worker Worker main script test should run on owner lane");

        assert!(
            worker_request_rx.await.is_err(),
            "dedicated Worker main script should be served by the service worker, not network fallback"
        );
        server
            .await
            .expect("service worker Worker main script server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_claim_dispatches_controllerchange_to_dedicated_worker_client() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_service_worker_worker_controllerchange_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let expected_service_worker_url = format!("{base_url}/app/sw.js");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerWorkerControllerChangeProbe = "pending";
                        globalThis.__serviceWorkerWorkerControllerChangeReady = "pending";
                        const worker = new Worker("worker.js");
                        worker.onmessage = async event => {
                            try {
                                const message = JSON.parse(event.data);
                                if (message.ready) {
                                    globalThis.__serviceWorkerWorkerControllerChangeReady =
                                        event.data;
                                    await navigator.serviceWorker.register("sw.js", {
                                        scope: "./"
                                    });
                                    await navigator.serviceWorker.ready;
                                    return;
                                }
                                globalThis.__serviceWorkerWorkerControllerChangeProbe =
                                    event.data;
                            } catch (error) {
                                globalThis.__serviceWorkerWorkerControllerChangeProbe =
                                    "error:" + String(error && error.message);
                            }
                        };
                        worker.onerror = event => {
                            globalThis.__serviceWorkerWorkerControllerChangeProbe =
                                "error:" + event.message;
                        };
                    })()
                    "#,
                )?;
                drive_service_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerWorkerControllerChangeProbe !== 'pending')",
                    "Service Worker claim should dispatch controllerchange to a worker client",
                )
                .await?;
                let ready: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerWorkerControllerChangeReady)")?,
                )
                .expect("worker ready payload should be JSON");
                assert_eq!(ready["ready"], serde_json::json!(true));
                assert_eq!(ready["initialControllerIsNull"], serde_json::json!(true));

                let result: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerWorkerControllerChangeProbe)")?,
                )
                .expect("worker controllerchange result should be JSON");
                assert_eq!(result["initialControllerIsNull"], serde_json::json!(true));
                assert_eq!(
                    result["controllerScriptURL"],
                    serde_json::json!(expected_service_worker_url)
                );
                assert_eq!(result["controllerState"], serde_json::json!("activated"));
                let events = result["events"]
                    .as_array()
                    .expect("worker controllerchange events should be an array");
                assert!(
                    events
                        .iter()
                        .any(|event| event.as_str() == Some("listener:controllerchange")),
                    "listener should observe worker controllerchange: {events:?}"
                );
                assert!(
                    events
                        .iter()
                        .any(|event| event.as_str() == Some("handler:controllerchange")),
                    "oncontrollerchange should observe worker controllerchange: {events:?}"
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker controllerchange test should run on owner lane");

        server
            .await
            .expect("service worker worker controllerchange server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_intercepted_window_fetch_abort_rejects_and_clears_pending_job() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_service_worker_abort_fetch_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerAbortFetchDone = false;
                        globalThis.__serviceWorkerAbortFetchObserved = "pending";
                        (async () => {
                            await navigator.serviceWorker.register("sw.js", { scope: "./" });
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {
                                await new Promise(resolve => {
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        { once: true }
                                    );
                                });
                            }
                            const controller = new AbortController();
                            const promise = fetch("slow.txt", {
                                signal: controller.signal
                            }).then(
                                () => "fulfilled",
                                error => [
                                    "error",
                                    error && error.name,
                                    error instanceof DOMException,
                                    error && error.message,
                                    controller.signal.aborted
                                ].join(":")
                            );
                            setTimeout(() => controller.abort(), 0);
                            globalThis.__serviceWorkerAbortFetchObserved = await promise;
                            globalThis.__serviceWorkerAbortFetchDone = true;
                        })().catch(error => {
                            globalThis.__serviceWorkerAbortFetchObserved =
                                "outer-error:" + String(error && error.message);
                            globalThis.__serviceWorkerAbortFetchDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_service_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerAbortFetchDone === true)",
                    "service worker intercepted fetch abort should reject",
                )
                .await?;
                let observed = page_vm
                    .vm_mut()
                    .eval("String(globalThis.__serviceWorkerAbortFetchObserved)")?;
                assert_eq!(
                    page_vm.pending_subresource_request_count(),
                    0,
                    "aborted service worker fetch should not leave a pending subresource"
                );
                anyhow::Ok(observed)
            })
            .await
            .expect("service worker abort fetch test should run on owner lane");

        server
            .await
            .expect("service worker abort fetch server should finish");
        assert_eq!(
            observed,
            "error:AbortError:true:The operation was aborted.:true"
        );
    })
    .await;
}

#[tokio::test]
async fn service_worker_intercepts_shared_worker_main_script() {
    run_page_vm_async_test(async move {
        let (base_url, worker_request_rx, server) =
            spawn_service_worker_shared_worker_main_script_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expected_worker_url = format!("{base_url}/app/shared-worker.js");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerSharedWorkerMainProbe = "pending";
                        (async () => {
                            await navigator.serviceWorker.register("sw.js", { scope: "./" });
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {
                                await new Promise(resolve => {
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        { once: true }
                                    );
                                });
                            }
                            const worker = new SharedWorker(
                                "shared-worker.js",
                                "service-worker-main-script"
                            );
                            worker.port.onmessage = event => {
                                globalThis.__serviceWorkerSharedWorkerMainProbe = event.data;
                            };
                            worker.onerror = event => {
                                globalThis.__serviceWorkerSharedWorkerMainProbe =
                                    "error:" + event.message;
                            };
                            worker.port.start();
                        })().catch(error => {
                            globalThis.__serviceWorkerSharedWorkerMainProbe =
                                "error:" + String(error && error.message);
                        });
                    })()
                    "#,
                )?;
                drive_service_worker_and_shared_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerSharedWorkerMainProbe !== 'pending')",
                    "service worker should intercept SharedWorker main script",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerSharedWorkerMainProbe)")?,
                    format!("sw-main:{expected_worker_url}")
                );
                anyhow::Ok(())
            })
            .await
            .expect("service worker SharedWorker main script test should run on owner lane");

        assert!(
            worker_request_rx.await.is_err(),
            "SharedWorker main script should be served by the service worker, not network fallback"
        );
        server
            .await
            .expect("service worker SharedWorker main script server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_message_port_wasm_module_to_shared_worker_fires_messageerror() {
    run_page_vm_async_test(async move {
        let (base_url, server) =
            spawn_service_worker_shared_worker_port_messageerror_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let actual = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__serviceWorkerSharedWorkerPortMessageErrorProbe = "pending";
                        (async () => {
                            const sw = navigator.serviceWorker;
                            const registration = await sw.register("sw.js", { scope: "./" });
                            await sw.ready;

                            const sharedWorker = new SharedWorker(
                                "shared-worker.js",
                                "service-worker-cross-agent-port-messageerror"
                            );
                            let resolveSharedReady;
                            let resolvePortReady;
                            let resolvePortMessageError;
                            const sharedReady = new Promise(resolve => {
                                resolveSharedReady = resolve;
                            });
                            const portReady = new Promise(resolve => {
                                resolvePortReady = resolve;
                            });
                            const portMessageError = new Promise(resolve => {
                                resolvePortMessageError = resolve;
                            });
                            sharedWorker.port.onmessage = event => {
                                if (event.data === "shared-ready") {
                                    resolveSharedReady(true);
                                    return;
                                }
                                if (event.data === "port-ready") {
                                    resolvePortReady(true);
                                    return;
                                }
                                if (typeof event.data === "string" &&
                                    event.data.startsWith("{")) {
                                    const data = JSON.parse(event.data);
                                    if (data && data.kind === "port-messageerror") {
                                        resolvePortMessageError(data);
                                        return;
                                    }
                                }
                                globalThis.__serviceWorkerSharedWorkerPortMessageErrorProbe =
                                    "unexpected-shared-worker-message:" + String(event.data);
                            };
                            sharedWorker.port.start();
                            await sharedReady;

                            const channel = new MessageChannel();
                            sharedWorker.port.postMessage("bind-port", [channel.port2]);
                            await portReady;

                            const workerAck = new Promise(resolve => {
                                sw.onmessage = event => {
                                    resolve({
                                        data: event.data,
                                        origin: event.origin,
                                        sourceState: event.source && event.source.state
                                    });
                                };
                            });
                            registration.active.postMessage(
                                "send-wasm-over-port",
                                [channel.port1]
                            );
                            const outcome = await Promise.all([
                                workerAck,
                                portMessageError
                            ]);
                            globalThis.__serviceWorkerSharedWorkerPortMessageErrorProbe =
                                JSON.stringify({
                                    sharedReady: true,
                                    portReady: true,
                                    workerAck: outcome[0],
                                    messageError: outcome[1]
                                });
                        })().catch(error => {
                            globalThis.__serviceWorkerSharedWorkerPortMessageErrorProbe =
                                "error:" + String(error && error.name) +
                                ":" + String(error && error.message);
                        });
                    })()
                    "#,
                )?;
                drive_service_worker_and_shared_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerSharedWorkerPortMessageErrorProbe !== 'pending')",
                    "service worker SharedWorker MessagePort messageerror should settle",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__serviceWorkerSharedWorkerPortMessageErrorProbe)")
            })
            .await
            .expect(
                "service worker SharedWorker MessagePort messageerror test should run on owner lane",
            );
        let expected = format!(
            r#"{{"sharedReady":true,"portReady":true,"workerAck":{{"data":"worker-sent-module","origin":"{base_url}","sourceState":"activated"}},"messageError":{{"kind":"port-messageerror","data":null,"origin":"","source":null,"ports":0}}}}"#
        );
        assert_eq!(actual, expected);

        server
            .await
            .expect("service worker SharedWorker MessagePort messageerror server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_blob_dedicated_worker_inherits_parent_controller_for_fetch() {
    run_page_vm_async_test(async move {
        let (base_url, sample_request_rx, server) =
            spawn_service_worker_blob_worker_fetch_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let sample_url = format!("{base_url}/app/sample.txt");
        let expected_service_worker_url = format!("{base_url}/app/sw.js");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__serviceWorkerBlobWorkerFetchProbe = "pending";
                        (async () => {{
                            await navigator.serviceWorker.register("sw.js", {{ scope: "./" }});
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {{
                                await new Promise(resolve => {{
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        {{ once: true }}
                                    );
                                }});
                            }}
                            const source = `
                                const container = navigator.serviceWorker;
                                const controller = container && container.controller;
                                fetch("{sample_url}")
                                    .then(response => response.text())
                                    .then(text => postMessage(JSON.stringify({{
                                        text: "blob-worker:" + text,
                                        serviceWorkerType: typeof container,
                                        controllerScriptURL:
                                            controller && controller.scriptURL,
                                        controllerState: controller && controller.state
                                    }})))
                                    .catch(error => {{
                                        postMessage("error:" + String(error && error.message));
                                    }});
                            `;
                            const scriptUrl = URL.createObjectURL(new Blob([source], {{
                                type: "application/javascript"
                            }}));
                            const worker = new Worker(scriptUrl);
                            worker.onmessage = event => {{
                                URL.revokeObjectURL(scriptUrl);
                                globalThis.__serviceWorkerBlobWorkerFetchProbe = event.data;
                            }};
                            worker.onerror = event => {{
                                URL.revokeObjectURL(scriptUrl);
                                globalThis.__serviceWorkerBlobWorkerFetchProbe =
                                    "error:" + event.message;
                            }};
                        }})().catch(error => {{
                            globalThis.__serviceWorkerBlobWorkerFetchProbe =
                                "error:" + String(error && error.message);
                        }});
                    }})()
                    "#
                ))?;
                drive_service_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerBlobWorkerFetchProbe !== 'pending')",
                    "blob dedicated worker should inherit service worker controller for fetch",
                )
                .await?;
                let result: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerBlobWorkerFetchProbe)")?,
                )
                .expect("blob dedicated worker controller result should be JSON");
                assert_eq!(
                    result,
                    serde_json::json!({
                        "text": "blob-worker:sw-sample",
                        "serviceWorkerType": "object",
                        "controllerScriptURL": expected_service_worker_url,
                        "controllerState": "activated",
                    })
                );
                anyhow::Ok(())
            })
            .await
            .expect("service worker blob Worker fetch test should run on owner lane");

        assert!(
            sample_request_rx.await.is_err(),
            "blob dedicated worker fetch should be served by the inherited service worker controller"
        );
        server
            .await
            .expect("service worker blob Worker fetch server should finish");
    })
    .await;
}

#[tokio::test]
async fn service_worker_blob_shared_worker_inherits_parent_controller_for_fetch() {
    run_page_vm_async_test(async move {
        let (base_url, sample_request_rx, server) =
            spawn_service_worker_blob_worker_fetch_server().await;
        let document_url = Url::parse(&format!("{base_url}/app/page.html")).expect("document url");
        let sample_url = format!("{base_url}/app/sample.txt");
        let expected_service_worker_url = format!("{base_url}/app/sw.js");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__serviceWorkerBlobSharedWorkerFetchProbe = "pending";
                        (async () => {{
                            await navigator.serviceWorker.register("sw.js", {{ scope: "./" }});
                            await navigator.serviceWorker.ready;
                            if (!navigator.serviceWorker.controller) {{
                                await new Promise(resolve => {{
                                    navigator.serviceWorker.addEventListener(
                                        "controllerchange",
                                        resolve,
                                        {{ once: true }}
                                    );
                                }});
                            }}
                            const source = `
                                onconnect = event => {{
                                    const port = event.ports[0];
                                    const container = navigator.serviceWorker;
                                    const controller = container && container.controller;
                                    fetch("{sample_url}")
                                        .then(response => response.text())
                                        .then(text => {{
                                            port.postMessage(JSON.stringify({{
                                                text: "blob-sharedworker:" + text,
                                                serviceWorkerType: typeof container,
                                                controllerScriptURL:
                                                    controller && controller.scriptURL,
                                                controllerState:
                                                    controller && controller.state
                                            }}));
                                        }})
                                        .catch(error => {{
                                            port.postMessage(
                                                "error:" + String(error && error.message)
                                            );
                                        }});
                                }};
                            `;
                            const scriptUrl = URL.createObjectURL(new Blob([source], {{
                                type: "application/javascript"
                            }}));
                            const worker = new SharedWorker(
                                scriptUrl,
                                "service-worker-blob-shared-worker-fetch"
                            );
                            worker.port.onmessage = event => {{
                                URL.revokeObjectURL(scriptUrl);
                                globalThis.__serviceWorkerBlobSharedWorkerFetchProbe =
                                    event.data;
                            }};
                            worker.onerror = event => {{
                                URL.revokeObjectURL(scriptUrl);
                                globalThis.__serviceWorkerBlobSharedWorkerFetchProbe =
                                    "error:" + event.message;
                            }};
                            worker.port.start();
                        }})().catch(error => {{
                            globalThis.__serviceWorkerBlobSharedWorkerFetchProbe =
                                "error:" + String(error && error.message);
                        }});
                    }})()
                    "#
                ))?;
                drive_service_worker_and_shared_worker_page_vm_until_done(
                    &mut page_vm,
                    "String(globalThis.__serviceWorkerBlobSharedWorkerFetchProbe !== 'pending')",
                    "blob shared worker should inherit service worker controller for fetch",
                )
                .await?;
                let result: serde_json::Value = serde_json::from_str(
                    &page_vm
                        .vm_mut()
                        .eval("String(globalThis.__serviceWorkerBlobSharedWorkerFetchProbe)")?,
                )
                .expect("blob shared worker controller result should be JSON");
                assert_eq!(
                    result,
                    serde_json::json!({
                        "text": "blob-sharedworker:sw-sample",
                        "serviceWorkerType": "object",
                        "controllerScriptURL": expected_service_worker_url,
                        "controllerState": "activated",
                    })
                );
                anyhow::Ok(())
            })
            .await
            .expect("service worker blob SharedWorker fetch test should run on owner lane");

        assert!(
            sample_request_rx.await.is_err(),
            "blob shared worker fetch should be served by the inherited service worker controller"
        );
        server
            .await
            .expect("service worker blob SharedWorker fetch server should finish");
    })
    .await;
}

#[tokio::test]
async fn worker_location_accessors_preserve_declared_descriptors_and_backing() {
    run_page_vm_async_test(async move {
        let worker_source = r#"
            const descriptorShape = (name) => {
                const descriptor = Object.getOwnPropertyDescriptor(WorkerLocation.prototype, name);
                return [
                    name,
                    typeof descriptor?.get,
                    descriptor?.get?.name,
                    typeof descriptor?.set,
                    descriptor?.enumerable,
                    descriptor?.configurable,
                ].join(":");
            };
            const probe = (callback) => {
                try {
                    const value = callback();
                    return value === undefined ? "undefined" : String(value);
                } catch (error) {
                    return `throw:${error && error.name}`;
                }
            };
            const beforeHref = location.href;
            const hrefDescriptor = Object.getOwnPropertyDescriptor(WorkerLocation.prototype, "href");
            const ownNamesBefore = Object.getOwnPropertyNames(location).sort();
            WorkerLocation.prototype.__moliWorkerLocationData = { href: "https://proto-spoof.test/" };
            Object.defineProperty(location, "__moliWorkerLocationData", {
                value: { href: "https://own-spoof.test/" },
                configurable: true,
            });
            const fakeLocation = Object.create(WorkerLocation.prototype);
            Object.defineProperty(fakeLocation, "__moliWorkerLocationData", {
                value: { href: "https://fake-spoof.test/" },
                configurable: true,
            });
            postMessage(JSON.stringify({
                constructorType: typeof WorkerLocation,
                tag: Object.prototype.toString.call(location),
                ownNames: ownNamesBefore,
                protoDescriptors: [
                    descriptorShape("href"),
                    descriptorShape("origin"),
                    descriptorShape("protocol"),
                    descriptorShape("host"),
                    descriptorShape("hostname"),
                    descriptorShape("port"),
                    descriptorShape("pathname"),
                    descriptorShape("search"),
                    descriptorShape("hash"),
                ],
                href: location.href,
                origin: location.origin,
                protocol: location.protocol,
                host: location.host,
                hostname: location.hostname,
                port: location.port,
                pathname: location.pathname,
                search: location.search,
                hash: location.hash,
                stringified: String(location),
                directToString: WorkerLocation.prototype.toString.call(location),
                fakeHref: probe(() => hrefDescriptor.get.call(fakeLocation)),
                hrefAfterSpoof: location.href,
                unchanged: location.href === beforeHref,
            }));
            close();
        "#;
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/worker-location.js?srch%20",
            "HTTP/1.1 200 OK",
            worker_source.to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let worker_url = format!("{base_url}/worker-location.js?srch%20");
        let base = Url::parse(&base_url).expect("base url");
        let expected_origin = base_url.clone();
        let expected_host =
            base[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
        let expected_hostname =
            base[url::Position::BeforeHost..url::Position::AfterHost].to_owned();
        let expected_port = base.port().map(|port| port.to_string()).unwrap_or_default();
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerLocationResult = null;
                        globalThis.__workerLocationDone = false;
                        const worker = new Worker("/worker-location.js?srch%20");
                        worker.onmessage = (event) => {
                            globalThis.__workerLocationResult = event.data;
                            globalThis.__workerLocationDone = true;
                        };
                        worker.onerror = (event) => {
                            globalThis.__workerLocationResult = "error:" + event.message;
                            globalThis.__workerLocationDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerLocationDone === true)",
                    "worker location descriptor probe should post a result",
                )
                .await?;
                let result = page_vm.vm_mut().eval("globalThis.__workerLocationResult")?;
                let result: serde_json::Value =
                    serde_json::from_str(&result).expect("worker location result should be JSON");
                assert_eq!(
                    result,
                    serde_json::json!({
                        "constructorType": "function",
                        "tag": "[object WorkerLocation]",
                        "ownNames": [],
                        "protoDescriptors": [
                            "href:function:get href:undefined:true:true",
                            "origin:function:get origin:undefined:true:true",
                            "protocol:function:get protocol:undefined:true:true",
                            "host:function:get host:undefined:true:true",
                            "hostname:function:get hostname:undefined:true:true",
                            "port:function:get port:undefined:true:true",
                            "pathname:function:get pathname:undefined:true:true",
                            "search:function:get search:undefined:true:true",
                            "hash:function:get hash:undefined:true:true",
                        ],
                        "href": worker_url,
                        "origin": expected_origin,
                        "protocol": "http:",
                        "host": expected_host,
                        "hostname": expected_hostname,
                        "port": expected_port,
                        "pathname": "/worker-location.js",
                        "search": "?srch%20",
                        "hash": "",
                        "stringified": worker_url,
                        "directToString": worker_url,
                        "fakeHref": "undefined",
                        "hrefAfterSpoof": worker_url,
                        "unchanged": true,
                    })
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker location descriptor test should run on owner lane");
        server
            .await
            .expect("worker location descriptor server should finish");
    })
    .await;
}

#[tokio::test]
async fn worker_script_load_failure_does_not_dispatch_window_error() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/does-not-exist.js",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let missing_worker_url = format!("{base_url}/does-not-exist.js");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__missingWorkerEvents = [];
                        globalThis.__missingWorkerDone = false;
                        window.addEventListener("error", event => {
                            globalThis.__missingWorkerEvents.push("window:" + event.message);
                            globalThis.__missingWorkerDone = true;
                        });
                        const worker = new Worker("/does-not-exist.js");
                        worker.onerror = event => {
                            globalThis.__missingWorkerEvents.push("worker:" + event.message);
                            globalThis.__missingWorkerDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__missingWorkerDone === true)",
                    "missing worker script should dispatch Worker error",
                )
                .await?;
                while page_vm
                    .run_exact_page_websocket_selected_task_for_test()
                    .await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__missingWorkerEvents)")?,
                    format!(
                        r#"["worker:HTTP request `{missing_worker_url}` returned 404 Not Found"]"#
                    )
                );
                anyhow::Ok(())
            })
            .await
            .expect("missing worker script error test should run on owner lane");
        server
            .await
            .expect("missing worker script server should finish");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_rejects_cross_origin_redirected_script() {
    run_page_vm_async_test(async move {
        let (base_url, source_server, target_server) =
            spawn_cross_origin_redirecting_shared_worker_script_servers().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerRedirectOutcome = null;
                        globalThis.__sharedWorkerRedirectDone = false;
                        const worker = new SharedWorker("/redirect-source.js", "cross-origin-redirect-script");
                        worker.onerror = (event) => {
                            globalThis.__sharedWorkerRedirectOutcome = "error:" + event.message;
                            globalThis.__sharedWorkerRedirectDone = true;
                        };
                        worker.port.onmessage = (event) => {
                            globalThis.__sharedWorkerRedirectOutcome = "message:" + event.data;
                            globalThis.__sharedWorkerRedirectDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerRedirectDone === true)",
                    "cross-origin redirected SharedWorker script should fail",
                )
                .await?;
                let outcome = page_vm.vm_mut().eval("globalThis.__sharedWorkerRedirectOutcome")?;
                assert!(
                    outcome.starts_with("error:"),
                    "redirected cross-origin script must not execute, got {outcome:?}"
                );
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker redirect rejection test should run on owner lane");

        source_server
            .await
            .expect("shared worker redirect source server should finish");
        target_server
            .await
            .expect("shared worker redirect target server should finish");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_runtime_error_does_not_notify_client_onerror() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerAbruptEvents = [];
                        globalThis.__sharedWorkerAbruptDone = false;
                        const source = `
                            onconnect = event => {
                                const port = event.ports[0];
                                port.onmessage = event => {
                                    event.ports[0].postMessage("handler-before-throw");
                                };
                            };
                            throw new Error("uncaught-exception");
                        `;
                        const worker = new SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "runtime-abrupt-completion"
                        );
                        worker.onerror = event => {
                            globalThis.__sharedWorkerAbruptEvents.push("error:" + event.message);
                            globalThis.__sharedWorkerAbruptDone = true;
                        };
                        const channel = new MessageChannel();
                        channel.port1.onmessage = event => {
                            globalThis.__sharedWorkerAbruptEvents.push("message:" + event.data);
                            globalThis.__sharedWorkerAbruptDone = true;
                        };
                        worker.port.postMessage("", [channel.port2]);
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerAbruptDone === true)",
                    "SharedWorker runtime error should not notify client onerror",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__sharedWorkerAbruptEvents)")?,
                    r#"["message:handler-before-throw"]"#
                );
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker runtime error test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_rejects_cross_origin_intermediate_redirect() {
    run_page_vm_async_test(async move {
        let (base_url, source_server, cross_server) =
            spawn_sw_return_redirect_servers().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerRedirectChainOutcome = null;
                        globalThis.__sharedWorkerRedirectChainDone = false;
                        const worker = new SharedWorker("/redirect-source.js", "cross-origin-intermediate-redirect-script");
                        worker.onerror = (event) => {
                            globalThis.__sharedWorkerRedirectChainOutcome = "error:" + event.message;
                            globalThis.__sharedWorkerRedirectChainDone = true;
                        };
                        worker.port.onmessage = (event) => {
                            globalThis.__sharedWorkerRedirectChainOutcome = "message:" + event.data;
                            globalThis.__sharedWorkerRedirectChainDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerRedirectChainDone === true)",
                    "SharedWorker script with cross-origin intermediate redirect should fail",
                )
                .await?;
                let outcome = page_vm
                    .vm_mut()
                    .eval("globalThis.__sharedWorkerRedirectChainOutcome")?;
                assert!(
                    outcome.starts_with("error:"),
                    "cross-origin redirect chain must not execute final same-origin script, got {outcome:?}"
                );
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker intermediate redirect rejection test should run on owner lane");

        source_server
            .await
            .expect("shared worker returning redirect source server should finish");
        cross_server
            .await
            .expect("shared worker intermediate redirect server should finish");
    })
    .await;
}

#[tokio::test]
async fn module_shared_worker_credentials_omit_omits_script_cookies() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_shared_worker_script_capture_http_server(
            r#"self.onconnect = (event) => event.ports[0].postMessage("module-ok");"#,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        document.cookie = "sw_module_cookie=sent; Path=/";
                        globalThis.__sharedWorkerCredentialsOutcome = null;
                        globalThis.__sharedWorkerCredentialsDone = false;
                        const worker = new SharedWorker("/module-credentials.js", {
                            type: "module",
                            credentials: "omit",
                            name: "module-credentials-omit"
                        });
                        worker.onerror = (event) => {
                            globalThis.__sharedWorkerCredentialsOutcome = "error:" + event.message;
                            globalThis.__sharedWorkerCredentialsDone = true;
                        };
                        worker.port.onmessage = (event) => {
                            globalThis.__sharedWorkerCredentialsOutcome = "message:" + event.data;
                            globalThis.__sharedWorkerCredentialsDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerCredentialsDone === true)",
                    "module SharedWorker credentials=omit script request should complete",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__sharedWorkerCredentialsOutcome")?,
                    "message:module-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker credentials=omit test should run on owner lane");

        let request = request_rx
            .await
            .expect("shared worker credentials test should capture script request");
        assert!(
            !request.contains("sw_module_cookie=sent"),
            "credentials=omit must not send document cookie on script fetch, request was:\n{request}"
        );
        server
            .await
            .expect("shared worker credentials script server should finish");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_console_flows_through_runtime_observable_source() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();
        let (output_tx, mut output_rx) = crate::runtime::renderer_output_transport_channel();
        page_vm
            .runtime_hooks
            .browser_context_runtime
            .set_renderer_output_transport_sender(output_tx);

        local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .dispatch_inspector_protocol_message(r#"{"id":1,"method":"Runtime.enable"}"#)
                    .expect("enable Runtime before SharedWorker console");
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerConsoleDone = false;
                        const source = `
                            onconnect = (event) => {
                                console.log("shared-console", name, 7);
                                event.ports[0].postMessage("ready");
                            };
                        `;
                        const worker = new SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "console-probe"
                        );
                        worker.port.onmessage = () => {
                            globalThis.__sharedWorkerConsoleDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerConsoleDone === true)",
                    "SharedWorker console probe should connect",
                )
                .await?;
                let (snapshot, target_events) =
                    drain_until_shared_worker_console_activity(&mut page_vm, &mut output_rx)
                        .await?;
                assert!(
                    has_console_probe_created_event(&target_events),
                    "SharedWorker target lifecycle should include the running worker"
                );
                assert!(
                    has_console_probe_target_console_event(&target_events),
                    "SharedWorker console should also surface on the target lifecycle lane"
                );
                let console = shared_worker_console_entry(&snapshot)
                    .expect("SharedWorker console entry should be present");
                assert_eq!(console.args[0]["value"], "shared-console");
                assert_eq!(console.args[1]["value"], "console-probe");
                assert_eq!(console.args[2]["value"], 7);
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker console test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_close_forgets_page_client_wrapper_tracking() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerCloseEvents = [];
                        globalThis.__sharedWorkerCloseDone = false;
                        const source = `
                            onconnect = (event) => {
                                const port = event.ports[0];
                                port.postMessage("before-close");
                                close();
                            };
                        `;
                        const worker = new SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "close-wrapper-sync"
                        );
                        globalThis.__sharedWorkerCloseProbe = worker;
                        worker.port.addEventListener("message", (event) => {
                            __sharedWorkerCloseEvents.push("message:" + event.data);
                        });
                        worker.port.addEventListener("close", (event) => {
                            __sharedWorkerCloseEvents.push("close:" + event.type);
                            __sharedWorkerCloseDone = true;
                        });
                        worker.port.start();
                    })()
                    "#,
                )?;
                assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 1);
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerCloseDone === true)",
                    "SharedWorker close should notify the client port",
                )
                .await?;
                while page_vm
                    .run_exact_page_websocket_selected_task_for_test()
                    .await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__sharedWorkerCloseEvents.join('|')")?,
                    "message:before-close|close:close"
                );
                wait_for_shared_worker_client_count(
                    &mut page_vm,
                    0,
                    "SharedWorker close should release the page client wrapper",
                )
                .await?;
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker close wrapper tracking test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_declared_surface_ignores_reflection_and_spoofing() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerDone = false;
                        globalThis.__sharedWorkerMessages = [];
                        globalThis.__sharedWorkerSurfaceCalls = [];
                        const source = `
                            onconnect = (event) => {
                                const port = event.ports[0];
                                port.onmessage = (event) => {
                                    port.postMessage("pong:" + event.data);
                                    close();
                                };
                                port.postMessage("ready");
                            };
                        `;
                        const worker = new SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "declared-surface"
                        );
                        globalThis.__sharedWorkerSurfaceProbe = worker;
                        const internalNames = [
                            "__moliSharedWorkerListeners",
                            "__moliSharedWorkerClientId",
                            "__moliSharedWorkerOnError",
                            "__moliEventTargetSlot",
                            "__moliSimpleEventTargetOrderedHandlers"
                        ];
                        const reflected = Object.getOwnPropertyNames(worker)
                            .filter(name => internalNames.includes(name));
                        if (reflected.length !== 0) {
                            throw new Error(`SharedWorker internals should not be reflected: ${reflected.join(",")}`);
                        }
                        const expectedMethods = {
                            addEventListener: "true:true:true:true:function:0:addEventListener",
                            removeEventListener: "true:true:true:true:function:0:removeEventListener",
                            dispatchEvent: "true:true:true:true:function:0:dispatchEvent"
                        };
                        for (const [name, shape] of Object.entries(expectedMethods)) {
                            const descriptor = Object.getOwnPropertyDescriptor(worker, name);
                            const actual = [
                                !!descriptor,
                                descriptor && descriptor.enumerable,
                                descriptor && descriptor.configurable,
                                descriptor && descriptor.writable,
                                descriptor && typeof descriptor.value,
                                descriptor && descriptor.value.length,
                                descriptor && descriptor.value.name
                            ].join(":");
                            if (actual !== shape) {
                                throw new Error(`${name} descriptor mismatch: ${actual}`);
                            }
                        }
                        const onerrorDescriptor = Object.getOwnPropertyDescriptor(worker, "onerror");
                        const onerrorShape = [
                            !!onerrorDescriptor,
                            onerrorDescriptor && onerrorDescriptor.enumerable,
                            onerrorDescriptor && onerrorDescriptor.configurable,
                            onerrorDescriptor && typeof onerrorDescriptor.get,
                            onerrorDescriptor && onerrorDescriptor.get.name,
                            onerrorDescriptor && onerrorDescriptor.get.length,
                            onerrorDescriptor && typeof onerrorDescriptor.set,
                            onerrorDescriptor && onerrorDescriptor.set.name,
                            onerrorDescriptor && onerrorDescriptor.set.length,
                            onerrorDescriptor && ("writable" in onerrorDescriptor)
                        ].join(":");
                        if (onerrorShape !== "true:true:true:function:get onerror:0:function:set onerror:1:false") {
                            throw new Error(`onerror descriptor mismatch: ${onerrorShape}`);
                        }
                        const portDescriptor = Object.getOwnPropertyDescriptor(worker, "port");
                        const portShape = [
                            !!portDescriptor,
                            portDescriptor && portDescriptor.enumerable,
                            portDescriptor && portDescriptor.configurable,
                            portDescriptor && portDescriptor.writable,
                            portDescriptor && typeof portDescriptor.value,
                            portDescriptor && portDescriptor.value === worker.port
                        ].join(":");
                        if (portShape !== "true:false:true:false:object:true") {
                            throw new Error(`port descriptor mismatch: ${portShape}`);
                        }
                        for (const name of internalNames) {
                            worker[name] = name.includes("Ordered") ? false : null;
                        }
                        worker.port = null;
                        if (worker.port === null || typeof worker.port.postMessage !== "function") {
                            throw new Error("readonly port should ignore assignment");
                        }
                        worker.addEventListener("error", event => __sharedWorkerSurfaceCalls.push(`listener:${event.type}`));
                        worker.onerror = event => __sharedWorkerSurfaceCalls.push(`handler:${event.type}`);
                        if (typeof worker.onerror !== "function") {
                            throw new Error("onerror getter should ignore public slot spoofing");
                        }
                        worker.dispatchEvent({ type: "error" });
                        const dispatchResult = __sharedWorkerSurfaceCalls.join("|");
                        if (dispatchResult !== "listener:error|handler:error") {
                            throw new Error(`SharedWorker ordered dispatch was spoofed: ${dispatchResult}`);
                        }
                        worker.port.onmessage = (event) => {
                            __sharedWorkerMessages.push(event.data);
                            if (event.data === "ready") {
                                worker.port.postMessage("go");
                            } else if (event.data === "pong:go") {
                                __sharedWorkerDone = true;
                            }
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_probe(
                    &mut page_vm,
                    "SharedWorker declared surface should ignore spoofed internals",
                )
                .await?;
                while page_vm
                    .run_exact_page_websocket_selected_task_for_test().await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm.vm_mut().eval(
                        "globalThis.__sharedWorkerSurfaceCalls.join('|') + ';' + globalThis.__sharedWorkerMessages.join('|')"
                    )?,
                    "listener:error|handler:error;ready|pong:go"
                );
                wait_for_shared_worker_client_count(
                    &mut page_vm,
                    0,
                    "SharedWorker declared surface should close the client wrapper",
                )
                .await?;
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker declared surface test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_same_site_none_omits_lax_script_cookies() {
    run_page_vm_async_test(async move {
        let (base_url, requests_rx, server) =
            spawn_shared_worker_script_capture_http_server_for_request_count(
                r#"self.onconnect = (event) => event.ports[0].postMessage("cookie-ok");"#,
                3,
            )
            .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        document.cookie = "sw_lax_cookie=sent; Path=/; SameSite=Lax";
                        globalThis.__sharedWorkerSameSiteMessages = [];
                        globalThis.__sharedWorkerSameSiteDone = false;
                        const workers = [
                            new SharedWorker("/default-cookie.js", {
                                name: "same-site-cookie-default"
                            }),
                            new SharedWorker("/all-cookie.js", {
                                name: "same-site-cookie-all",
                                sameSiteCookies: "all"
                            }),
                            new SharedWorker("/none-cookie.js", {
                                name: "same-site-cookie-none",
                                sameSiteCookies: "none"
                            })
                        ];
                        for (const worker of workers) {
                            worker.onerror = (event) => {
                                globalThis.__sharedWorkerSameSiteMessages.push("error:" + event.message);
                                globalThis.__sharedWorkerSameSiteDone = true;
                            };
                            worker.port.onmessage = (event) => {
                                globalThis.__sharedWorkerSameSiteMessages.push(event.data);
                                if (globalThis.__sharedWorkerSameSiteMessages.length === workers.length) {
                                    globalThis.__sharedWorkerSameSiteDone = true;
                                }
                            };
                            worker.port.start();
                        }
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerSameSiteDone === true)",
                    "SharedWorker sameSiteCookies script requests should complete",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__sharedWorkerSameSiteMessages.join('|')")?,
                    "cookie-ok|cookie-ok|cookie-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker sameSiteCookies test should run on owner lane");

        let requests = requests_rx
            .await
            .expect("shared worker sameSiteCookies test should capture script requests");
        assert_eq!(
            requests.len(),
            3,
            "expected three script requests, got {requests:#?}"
        );
        let default_request = requests
            .iter()
            .find(|request| request.starts_with("GET /default-cookie.js "))
            .expect("default sameSiteCookies script request should be captured");
        assert!(
            default_request.contains("sw_lax_cookie=sent"),
            "default sameSiteCookies request should send Lax cookie, request was:\n{default_request}"
        );
        let all_request = requests
            .iter()
            .find(|request| request.starts_with("GET /all-cookie.js "))
            .expect("sameSiteCookies=all script request should be captured");
        assert!(
            all_request.contains("sw_lax_cookie=sent"),
            "sameSiteCookies=all request should send Lax cookie in first-party context, request was:\n{all_request}"
        );
        let none_request = requests
            .iter()
            .find(|request| request.starts_with("GET /none-cookie.js "))
            .expect("sameSiteCookies=none script request should be captured");
        assert!(
            !none_request.contains("sw_lax_cookie=sent"),
            "sameSiteCookies=none request must not send Lax cookie, request was:\n{none_request}"
        );
        server
            .await
            .expect("shared worker sameSiteCookies script server should finish");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_same_site_cookie_mode_partitions_matching_key() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    &format!(
                        r#"
                        (() => {{
                            globalThis.__sharedWorkerSameSiteKeyMessages = [];
                            globalThis.__sharedWorkerSameSiteKeyDone = false;
                            const url = "data:text/javascript," + encodeURIComponent({});
                            const workers = [
                                ["default", new SharedWorker(url, {{ name: "same-site-key" }})],
                                ["none-a", new SharedWorker(url, {{
                                    name: "same-site-key",
                                    sameSiteCookies: "none"
                                }})],
                                ["none-b", new SharedWorker(url, {{
                                    name: "same-site-key",
                                    sameSiteCookies: "none"
                                }})]
                            ];
                            for (const [label, worker] of workers) {{
                                worker.onerror = (event) => {{
                                    globalThis.__sharedWorkerSameSiteKeyMessages.push(label + ":error:" + event.message);
                                    globalThis.__sharedWorkerSameSiteKeyDone = true;
                                }};
                                worker.port.onmessage = (event) => {{
                                    globalThis.__sharedWorkerSameSiteKeyMessages.push(label + ":" + event.data);
                                    if (globalThis.__sharedWorkerSameSiteKeyMessages.length === workers.length) {{
                                        globalThis.__sharedWorkerSameSiteKeyDone = true;
                                    }}
                                }};
                                worker.port.start();
                            }}
                        }})()
                        "#,
                        serde_json::to_string(SHARED_WORKER_CONNECTION_COUNT_SOURCE)
                            .expect("serialize worker source")
                    ),
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerSameSiteKeyDone === true)",
                    "SharedWorker sameSiteCookies key partitioning should complete",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__sharedWorkerSameSiteKeyMessages.sort().join('|')")?,
                    "default:1|none-a:1|none-b:2"
                );
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker sameSiteCookies key test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn third_party_shared_worker_default_omits_lax_script_cookies() {
    run_page_vm_async_test(async move {
        let (child_origin, request_rx, server) =
            spawn_third_party_shared_worker_same_site_http_server("default").await;
        let child_url = format!("{child_origin}/child.html?mode=default");
        let child_url_literal =
            serde_json::to_string(&child_url).expect("serialize third-party child url");
        let top_url =
            Url::parse("http://top-level.example.test/page.html").expect("top-level url");
        let mut page_vm = test_page_vm_with_document_url(top_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__thirdPartySameSiteDone = false;
                        globalThis.__thirdPartySameSiteMessage = "";
                        window.addEventListener("message", (event) => {{
                            globalThis.__thirdPartySameSiteMessage = event.data;
                            globalThis.__thirdPartySameSiteDone = true;
                        }});
                        const frame = document.createElement("iframe");
                        frame.src = {child_url_literal};
                        document.body.appendChild(frame);
                    }})()
                    "#
                ))?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__thirdPartySameSiteDone === true)",
                    "third-party SharedWorker sameSiteCookies default should complete",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__thirdPartySameSiteMessage")?,
                    "message:ok:default"
                );
                anyhow::Ok(())
            })
            .await
            .expect("third-party SharedWorker sameSite default test should run on owner lane");

        let requests = request_rx
            .await
            .expect("third-party SharedWorker sameSite default request capture");
        assert_eq!(
            requests.len(),
            2,
            "third-party default test should load child document and worker script; requests={requests:?}"
        );
        let script_request = requests
            .iter()
            .find(|request| request.starts_with("GET /sw.js?mode=default "))
            .expect("third-party default worker script request should be captured");
        assert!(
            !script_request.contains("sw_lax_cookie=sent"),
            "third-party default SharedWorker script request must omit Lax cookie, request was:\n{script_request}"
        );
        server
            .await
            .expect("third-party SharedWorker sameSite default server should finish");
    })
    .await;
}

#[tokio::test]
async fn third_party_shared_worker_none_omits_lax_script_cookies() {
    run_page_vm_async_test(async move {
        let (child_origin, request_rx, server) =
            spawn_third_party_shared_worker_same_site_http_server("none").await;
        let child_url = format!("{child_origin}/child.html?mode=none");
        let child_url_literal =
            serde_json::to_string(&child_url).expect("serialize third-party child url");
        let top_url =
            Url::parse("http://top-level.example.test/page.html").expect("top-level url");
        let mut page_vm = test_page_vm_with_document_url(top_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__thirdPartySameSiteNoneDone = false;
                        globalThis.__thirdPartySameSiteNoneMessage = "";
                        window.addEventListener("message", (event) => {{
                            globalThis.__thirdPartySameSiteNoneMessage = event.data;
                            globalThis.__thirdPartySameSiteNoneDone = true;
                        }});
                        const frame = document.createElement("iframe");
                        frame.src = {child_url_literal};
                        document.body.appendChild(frame);
                    }})()
                    "#
                ))?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__thirdPartySameSiteNoneDone === true)",
                    "third-party SharedWorker sameSiteCookies none should complete",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__thirdPartySameSiteNoneMessage")?,
                    "message:ok:none"
                );
                anyhow::Ok(())
            })
            .await
            .expect("third-party SharedWorker sameSite none test should run on owner lane");

        let requests = request_rx
            .await
            .expect("third-party SharedWorker sameSite none request capture");
        assert_eq!(
            requests.len(),
            2,
            "third-party none test should load child document and worker script; requests={requests:?}"
        );
        let script_request = requests
            .iter()
            .find(|request| request.starts_with("GET /sw.js?mode=none "))
            .expect("third-party none worker script request should be captured");
        assert!(
            !script_request.contains("sw_lax_cookie=sent"),
            "third-party sameSiteCookies=none SharedWorker script request must omit Lax cookie, request was:\n{script_request}"
        );
        server
            .await
            .expect("third-party SharedWorker sameSite none server should finish");
    })
    .await;
}

#[tokio::test]
async fn third_party_shared_worker_all_throws_without_script_request() {
    run_page_vm_async_test(async move {
        let (child_origin, request_rx, server) =
            spawn_third_party_shared_worker_same_site_http_server("all").await;
        let child_url = format!("{child_origin}/child.html?mode=all");
        let child_url_literal =
            serde_json::to_string(&child_url).expect("serialize third-party child url");
        let top_url =
            Url::parse("http://top-level.example.test/page.html").expect("top-level url");
        let mut page_vm = test_page_vm_with_document_url(top_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__thirdPartySameSiteAllDone = false;
                        globalThis.__thirdPartySameSiteAllMessage = "";
                        window.addEventListener("message", (event) => {{
                            globalThis.__thirdPartySameSiteAllMessage = event.data;
                            globalThis.__thirdPartySameSiteAllDone = true;
                        }});
                        const frame = document.createElement("iframe");
                        frame.src = {child_url_literal};
                        document.body.appendChild(frame);
                    }})()
                    "#
                ))?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__thirdPartySameSiteAllDone === true)",
                    "third-party SharedWorker sameSiteCookies all should throw",
                )
                .await?;
                let message = page_vm
                    .vm_mut()
                    .eval("globalThis.__thirdPartySameSiteAllMessage")?;
                assert!(
                    message.starts_with("throw:SecurityError:SharedWorkers in third-party contexts cannot request SameSite Strict or Lax cookies"),
                    "third-party sameSiteCookies=all should throw SecurityError, got {message:?}"
                );
                anyhow::Ok(())
            })
            .await
            .expect("third-party SharedWorker sameSite all test should run on owner lane");

        let requests = request_rx
            .await
            .expect("third-party SharedWorker sameSite all request capture");
        assert_eq!(
            requests.len(),
            1,
            "third-party sameSiteCookies=all must not request worker script; requests={requests:?}"
        );
        assert!(
            requests[0].starts_with("GET /child.html?mode=all "),
            "only the child document request should be captured, request was:\n{}",
            requests[0]
        );
        server
            .await
            .expect("third-party SharedWorker sameSite all server should finish");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_terminal_error_forgets_page_client_wrapper_tracking() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerTerminalErrorRecord = null;
                        globalThis.__sharedWorkerTerminalErrorDone = false;
                        const source = "function( { broken syntax";
                        const worker = new SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "terminal-error-wrapper-sync"
                        );
                        globalThis.__sharedWorkerTerminalErrorProbe = worker;
                        worker.onerror = (event) => {
                            globalThis.__sharedWorkerTerminalErrorRecord = {
                                type: event.type,
                                cancelable: event.cancelable,
                                hasMessage: typeof event.message === "string" && event.message.length > 0
                            };
                            globalThis.__sharedWorkerTerminalErrorDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 1);
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerTerminalErrorDone === true)",
                    "SharedWorker terminal error should notify the client wrapper",
                )
                .await?;
                while page_vm
                    .run_exact_page_websocket_selected_task_for_test().await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__sharedWorkerTerminalErrorRecord)")?,
                    r#"{"type":"error","cancelable":true,"hasMessage":true}"#
                );
                wait_for_shared_worker_client_count(
                    &mut page_vm,
                    0,
                    "SharedWorker terminal error should release the page client wrapper",
                )
                .await?;
                anyhow::Ok(())
            })
            .await
            .expect("SharedWorker terminal error wrapper tracking test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn child_shared_worker_error_handler_uses_child_owner_scope() {
    run_page_vm_async_test(async move {
        let document_url =
            Url::parse("https://shared-worker-error-owner.test/page.html").expect("document url");
        let broken_worker_url = "data:text/javascript,function(%20%7B%20broken%20syntax";
        let broken_worker_url_literal =
            serde_json::to_string(&broken_worker_url).expect("serialize broken worker URL");
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let mut shared_worker_wake_rx =
            super::shared_worker_client_event::install_shared_worker_service_wake(&page_vm);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__childSharedWorkerOwnerMessages = [];
                        globalThis.__childSharedWorkerOwnerDone = false;
                        globalThis.__childSharedWorkerConstructError = null;
                        globalThis.__childSharedWorkerHandlerError = null;
                        const topChannel = new BroadcastChannel("child-shared-worker-error-owner");
                        topChannel.onmessage = event => {{
                            __childSharedWorkerOwnerMessages.push("top:" + event.data + ":" + event.origin);
                        }};
                        addEventListener("message", event => {{
                            const value = String(event.data);
                            __childSharedWorkerOwnerMessages.push(value);
                            if (value.startsWith("child:construct-error:")) {{
                                __childSharedWorkerConstructError =
                                    value.slice("child:construct-error:".length);
                            }}
                            if (value.startsWith("child:handler-error:")) {{
                                __childSharedWorkerHandlerError =
                                    value.slice("child:handler-error:".length);
                            }}
                            if (value === "child:error-channel-created") {{
                                __childSharedWorkerOwnerDone = true;
                            }}
                        }});

                        const frame = document.createElement("iframe");
                        frame.src = "data:text/html," + encodeURIComponent(`
                            <!doctype html>
                            <script>
                                let worker;
                                try {{
                                    worker = new SharedWorker(
                                        {broken_worker_url_literal},
                                        "child-shared-worker-error-owner"
                                    );
                                }} catch (error) {{
                                    parent.postMessage(
                                        "child:construct-error:" +
                                            String(error && error.message || error),
                                        "*"
                                    );
                                    throw error;
                                }}
                                worker.onerror = event => {{
                                    try {{
                                        const childChannel = new BroadcastChannel(
                                            "child-shared-worker-error-owner"
                                        );
                                        childChannel.postMessage("should-stay-child-scoped");
                                        parent.postMessage("child:error-channel-created", "*");
                                    }} catch (error) {{
                                        parent.postMessage(
                                            "child:handler-error:" +
                                                String(error && error.message || error),
                                            "*"
                                        );
                                        throw error;
                                    }}
                                }};
                            <\/script>
                        `);
                        document.body.appendChild(frame);
                    }})()
                    "#,
                ))?;

                wait_for_child_shared_worker_owner_probe(
                    &mut page_vm,
                    &mut resource_source,
                    &mut shared_worker_wake_rx,
                    &mut owner_wake_rx,
                    "String(globalThis.__childSharedWorkerOwnerDone === true)",
                    "child SharedWorker error handler should run in child owner scope",
                )
                .await?;

                while page_vm
                    .run_exact_page_websocket_selected_task_for_test().await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__childSharedWorkerOwnerMessages)")?,
                    r#"["child:error-channel-created"]"#
                );
                anyhow::Ok(())
            })
            .await
            .expect("child SharedWorker error owner test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn child_frame_shared_worker_client_disconnects_on_iframe_removal() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let worker_source = r#"
                    onconnect = (event) => {
                        const port = event.ports[0];
                        port.postMessage("ready");
                    };
                "#;
                let worker_source_literal =
                    serde_json::to_string(worker_source).expect("serialize worker source");
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__childSharedWorkerDone = false;
                        globalThis.__childSharedWorkerMessages = [];
                        const frame = document.createElement("iframe");
                        document.body.appendChild(frame);
                        globalThis.__childSharedWorkerFrame = frame;
                        const source = {worker_source_literal};
                        const worker = new frame.contentWindow.SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "child-frame-disconnect"
                        );
                        globalThis.__childSharedWorkerProbe = worker;
                        worker.port.onmessage = (event) => {{
                            globalThis.__childSharedWorkerMessages.push(event.data);
                            globalThis.__childSharedWorkerDone = true;
                        }};
                        worker.port.start();
                    }})()
                    "#
                ))?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__childSharedWorkerDone === true)",
                    "child frame SharedWorker should connect",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__childSharedWorkerMessages.join('|')")?,
                    "ready"
                );
                assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 1);

                page_vm
                    .vm_mut()
                    .eval("globalThis.__childSharedWorkerFrame.remove(); 'removed'")?;
                assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 0);
                anyhow::Ok(())
            })
            .await
            .expect("child frame SharedWorker removal test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn dedicated_worker_script_request_uses_creator_response_referrer_policy() {
    run_page_vm_async_test(async move {
        let (base_url, worker_request_rx, api_request_rx, server) =
            spawn_worker_script_then_api_capture_http_server("").await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm =
            test_page_vm_with_response_referrer_policy(document_url, "no-referrer");
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__dedicatedWorkerPolicyMessage = "";
                        const worker = new Worker("/worker.js");
                        worker.onmessage = event => {
                            globalThis.__dedicatedWorkerPolicyMessage = event.data;
                        };
                        worker.onerror = event => {
                            globalThis.__dedicatedWorkerPolicyMessage =
                                "error:" + event.message;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__dedicatedWorkerPolicyMessage !== '')",
                    "dedicated Worker should load after creator response referrer policy",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__dedicatedWorkerPolicyMessage")?,
                    "worker-fetch-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("dedicated Worker creator policy test should run on owner lane");

        let worker_request = worker_request_rx
            .await
            .expect("captured dedicated Worker script request");
        let _ = api_request_rx
            .await
            .expect("captured dedicated Worker follow-up fetch");
        assert!(
            !worker_request
                .to_ascii_lowercase()
                .contains("\r\nreferer:"),
            "creator response Referrer-Policy: no-referrer must suppress dedicated Worker script Referer; request was:\n{worker_request}"
        );
        server
            .await
            .expect("dedicated Worker creator policy server should finish");
    })
    .await;
}

#[tokio::test]
async fn dedicated_worker_fetch_uses_worker_response_referrer_policy() {
    run_page_vm_async_test(async move {
        let (base_url, worker_request_rx, api_request_rx, server) =
            spawn_worker_script_then_api_capture_http_server("Referrer-Policy: no-referrer\r\n")
                .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__dedicatedWorkerResponsePolicyMessage = "";
                        const worker = new Worker("/worker.js");
                        worker.onmessage = event => {
                            globalThis.__dedicatedWorkerResponsePolicyMessage = event.data;
                        };
                        worker.onerror = event => {
                            globalThis.__dedicatedWorkerResponsePolicyMessage =
                                "error:" + event.message;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__dedicatedWorkerResponsePolicyMessage !== '')",
                    "dedicated Worker should load after worker response referrer policy",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__dedicatedWorkerResponsePolicyMessage")?,
                    "worker-fetch-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("dedicated Worker response policy test should run on owner lane");

        let _ = worker_request_rx
            .await
            .expect("captured dedicated Worker script request");
        let api_request = api_request_rx
            .await
            .expect("captured dedicated Worker fetch request");
        assert!(
            !api_request.to_ascii_lowercase().contains("\r\nreferer:"),
            "Worker script response Referrer-Policy: no-referrer must suppress worker fetch Referer; request was:\n{api_request}"
        );
        server
            .await
            .expect("dedicated Worker response policy server should finish");
    })
    .await;
}

#[tokio::test]
async fn top_level_shared_worker_uses_response_referrer_policy() {
    run_page_vm_async_test(async move {
        let script_body = r#"onconnect = event => event.ports[0].postMessage("ready");"#;
        let (base_url, request_rx, server) =
            spawn_shared_worker_script_capture_http_server(script_body).await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm =
            test_page_vm_with_response_referrer_policy(document_url, "no-referrer");
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerMessages = [];
                        globalThis.__sharedWorkerDone = false;
                        const worker = new SharedWorker("/sw.js", "top-response-referrer-policy");
                        worker.onerror = event => {
                            globalThis.__sharedWorkerMessages.push("error:" + event.message);
                            globalThis.__sharedWorkerDone = true;
                        };
                        worker.port.onmessage = event => {
                            globalThis.__sharedWorkerMessages.push(event.data);
                            globalThis.__sharedWorkerDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_probe(
                    &mut page_vm,
                    "top-level SharedWorker should connect after response referrer policy",
                )
                .await?;
                assert_eq!(shared_worker_probe_messages(&mut page_vm)?, "ready");
                anyhow::Ok(())
            })
            .await
            .expect("top-level SharedWorker response policy test should run on owner lane");

        let request = request_rx
            .await
            .expect("top-level SharedWorker request should be captured");
        assert!(
            !request.to_ascii_lowercase().contains("\r\nreferer:"),
            "top-level response Referrer-Policy: no-referrer must suppress worker script Referer; request was:\n{request}"
        );
        server
            .await
            .expect("top-level referrer policy shared worker server should finish");
    })
    .await;
}

#[tokio::test]
async fn child_frame_shared_worker_uses_child_response_referrer_policy() {
    run_page_vm_async_test(async move {
        let (base_url, worker_request_rx, server) =
            spawn_child_document_referrer_policy_shared_worker_server().await;
        let document_url = Url::parse(&format!("{base_url}/parent.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__childResponsePolicyWorkerDone = false;
                        globalThis.__childResponsePolicyWorkerOutcome = null;
                        const frame = document.createElement("iframe");
                        frame.onload = () => {
                            try {
                                const worker = new frame.contentWindow.SharedWorker(
                                    "/worker.js",
                                    "child-response-referrer-policy"
                                );
                                worker.onerror = event => {
                                    globalThis.__childResponsePolicyWorkerOutcome =
                                        "error:" + event.message;
                                    globalThis.__childResponsePolicyWorkerDone = true;
                                };
                                worker.port.onmessage = event => {
                                    globalThis.__childResponsePolicyWorkerOutcome =
                                        "message:" + event.data;
                                    globalThis.__childResponsePolicyWorkerDone = true;
                                };
                                worker.port.start();
                            } catch (error) {
                                globalThis.__childResponsePolicyWorkerOutcome =
                                    "throw:" + error.name + ":" + error.message;
                                globalThis.__childResponsePolicyWorkerDone = true;
                            }
                        };
                        frame.src = "/child.html";
                        document.body.appendChild(frame);
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__childResponsePolicyWorkerDone === true)",
                    "child frame SharedWorker should connect after response referrer policy",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__childResponsePolicyWorkerOutcome")?,
                    "message:ready"
                );
                anyhow::Ok(())
            })
            .await
            .expect("child frame SharedWorker response policy test should run on owner lane");

        let request = worker_request_rx
            .await
            .expect("child frame SharedWorker request should be captured");
        assert!(
            !request.to_ascii_lowercase().contains("\r\nreferer:"),
            "child response Referrer-Policy: no-referrer must suppress worker script Referer; request was:\n{request}"
        );
        server
            .await
            .expect("child referrer policy shared worker server should finish");
    })
    .await;
}

#[tokio::test]
async fn child_frame_shared_worker_client_survives_initial_reuse_then_disconnects_on_navigation() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let worker_source = r#"
                    onconnect = (event) => {
                        const port = event.ports[0];
                        port.postMessage("ready");
                        port.onmessage = (message) => port.postMessage(message.data);
                    };
                "#;
                let worker_source_literal =
                    serde_json::to_string(worker_source).expect("serialize worker source");
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__childSharedWorkerDone = false;
                        globalThis.__childSharedWorkerMessages = [];
                        const frame = document.createElement("iframe");
                        document.body.appendChild(frame);
                        globalThis.__childSharedWorkerFrame = frame;
                        const source = {worker_source_literal};
                        const worker = new frame.contentWindow.SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "child-frame-navigation-disconnect"
                        );
                        globalThis.__childSharedWorkerProbe = worker;
                        worker.port.onmessage = (event) => {{
                            globalThis.__childSharedWorkerMessages.push(event.data);
                            globalThis.__childSharedWorkerDone = true;
                        }};
                        worker.port.start();
                    }})()
                    "#
                ))?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__childSharedWorkerDone === true)",
                    "child frame SharedWorker should connect before navigation",
                )
                .await?;
                assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 1);

                page_vm.vm_mut().eval(
                    "globalThis.__childSharedWorkerFrame.srcdoc = '<p>first</p>'; 'navigating-first'",
                )?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "first child navigation should securely reuse the initial-empty LocalWindow",
                )
                .await;
                assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 1);
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__childSharedWorkerDone = false;
                    globalThis.__childSharedWorkerProbe.port.postMessage("after-first-navigation");
                    "sent"
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__childSharedWorkerDone === true)",
                    "child SharedWorker should remain connected after initial-empty reuse",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__childSharedWorkerMessages.join('|')")?,
                    "ready|after-first-navigation"
                );

                page_vm.vm_mut().eval(
                    "globalThis.__childSharedWorkerFrame.srcdoc = '<p>later</p>'; 'navigating-later'",
                )?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "later child navigation should replace the LocalWindow",
                )
                .await;
                assert_eq!(page_vm.vm().shared_worker_client_count_for_test(), 0);
                anyhow::Ok(())
            })
            .await
            .expect("child frame SharedWorker navigation test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn credentialless_child_dedicated_worker_uses_credentialless_network_partition_key() {
    run_page_vm_async_test(async move {
        let (base_url, request_count_rx, server) =
            spawn_cacheable_worker_partition_server("dedicated").await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let worker_url_literal =
            serde_json::to_string(&format!("{base_url}/worker.js")).expect("serialize worker url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let outcome = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__credentiallessWorkerPartitionDone = false;
                        globalThis.__credentiallessWorkerPartitionResult = [];
                        const workerUrl = {worker_url_literal};
                        const credentialless = document.createElement("iframe");
                        credentialless.credentialless = true;
                        const normal = document.createElement("iframe");
                        const credentiallessLoaded = new Promise(resolve => {{
                            credentialless.onload = resolve;
                        }});
                        const normalLoaded = new Promise(resolve => {{
                            normal.onload = resolve;
                        }});
                        credentialless.src = "/child.html";
                        normal.src = "/child.html";
                        document.body.append(credentialless, normal);
                        const startWorker = (win) => new Promise((resolve, reject) => {{
                            const worker = new win.Worker(workerUrl);
                            worker.onmessage = event => resolve(event.data);
                            worker.onerror = event => reject(new Error(event.message));
                        }});
                        Promise.all([credentiallessLoaded, normalLoaded])
                          .then(() => startWorker(credentialless.contentWindow))
                          .then(first => startWorker(normal.contentWindow)
                            .then(second => {{
                                globalThis.__credentiallessWorkerPartitionResult = [first, second];
                                globalThis.__credentiallessWorkerPartitionDone = true;
                            }}))
                          .catch(error => {{
                              globalThis.__credentiallessWorkerPartitionResult =
                                  ["error", String(error)];
                              globalThis.__credentiallessWorkerPartitionDone = true;
                          }});
                    }})()
                    "#,
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__credentiallessWorkerPartitionDone === true)",
                    "credentialless child Worker script partitioning should finish",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__credentiallessWorkerPartitionResult)")
            })
            .await
            .expect("credentialless child Worker partitioning test should run on owner lane");

        let request_count = request_count_rx
            .await
            .expect("credentialless child Worker partition server should report request count");
        server
            .await
            .expect("credentialless child Worker partition server should finish");
        assert_eq!(outcome, r#"["credentialless","normal"]"#);
        assert_eq!(
            request_count, 2,
            "credentialless and normal child Worker scripts should use separate network/cache partitions"
        );
    })
    .await;
}

#[tokio::test]
async fn credentialless_child_shared_worker_uses_credentialless_network_partition_key() {
    run_page_vm_async_test(async move {
        let (base_url, request_count_rx, server) =
            spawn_cacheable_worker_partition_server("shared").await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let worker_url_literal =
            serde_json::to_string(&format!("{base_url}/worker.js")).expect("serialize worker url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let outcome = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__credentiallessSharedWorkerPartitionDone = false;
                        globalThis.__credentiallessSharedWorkerPartitionResult = [];
                        const workerUrl = {worker_url_literal};
                        const credentialless = document.createElement("iframe");
                        credentialless.credentialless = true;
                        const normal = document.createElement("iframe");
                        const credentiallessLoaded = new Promise(resolve => {{
                            credentialless.onload = resolve;
                        }});
                        const normalLoaded = new Promise(resolve => {{
                            normal.onload = resolve;
                        }});
                        credentialless.src = "/child.html";
                        normal.src = "/child.html";
                        document.body.append(credentialless, normal);
                        const startWorker = (win, name) => new Promise((resolve, reject) => {{
                            const worker = new win.SharedWorker(workerUrl, name);
                            worker.onerror = event => reject(new Error(event.message));
                            worker.port.onmessage = event => resolve(event.data);
                            worker.port.start();
                        }});
                        Promise.all([credentiallessLoaded, normalLoaded])
                          .then(() => startWorker(
                              credentialless.contentWindow,
                              "credentialless-partition"
                          ))
                          .then(first => startWorker(normal.contentWindow, "normal-partition")
                            .then(second => {{
                                globalThis.__credentiallessSharedWorkerPartitionResult =
                                    [first, second];
                                globalThis.__credentiallessSharedWorkerPartitionDone = true;
                            }}))
                          .catch(error => {{
                              globalThis.__credentiallessSharedWorkerPartitionResult =
                                  ["error", String(error)];
                              globalThis.__credentiallessSharedWorkerPartitionDone = true;
                          }});
                    }})()
                    "#,
                ))?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__credentiallessSharedWorkerPartitionDone === true)",
                    "credentialless child SharedWorker script partitioning should finish",
                )
                .await?;
                page_vm.vm_mut().eval(
                    "JSON.stringify(globalThis.__credentiallessSharedWorkerPartitionResult)",
                )
            })
            .await
            .expect("credentialless child SharedWorker partitioning test should run on owner lane");

        let request_count = request_count_rx.await.expect(
            "credentialless child SharedWorker partition server should report request count",
        );
        server
            .await
            .expect("credentialless child SharedWorker partition server should finish");
        assert_eq!(outcome, r#"["credentialless","normal"]"#);
        assert_eq!(
            request_count, 2,
            "credentialless and normal child SharedWorker scripts should use separate network/cache partitions"
        );
    })
    .await;
}

#[tokio::test]
async fn worker_global_exposes_text_codecs_and_crypto_subtle_digest() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerCodecDone = false;
                        globalThis.__workerCodecResult = null;
                        const source = `
                            (async () => {
                                const encoded = new TextEncoder().encode("hé");
                                const decoded = new TextDecoder().decode(encoded);
                                const digest = await crypto.subtle.digest(
                                    "SHA-256",
                                    new TextEncoder().encode("abc")
                                );
                                postMessage([
                                    Array.from(encoded).join(","),
                                    decoded,
                                    Array.from(new Uint8Array(digest).slice(0, 4)).join(",")
                                ].join("|"));
                            })().catch(error => postMessage("error:" + error.message));
                        `;
                        const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
                        worker.onmessage = (event) => {
                            globalThis.__workerCodecResult = event.data;
                            globalThis.__workerCodecDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerCodecDone === true)",
                    "worker text codec and crypto digest should complete",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__workerCodecResult")?,
                    "104,195,169|hé|186,120,22,191"
                );
                anyhow::Ok(())
            })
            .await
            .expect("worker codec/crypto test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn data_and_blob_workers_inherit_secure_context_webcrypto_surface() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let results = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerCryptoResults = [];
                        globalThis.__workerCryptoDone = false;
                        const source = `
                            postMessage([
                                String("subtle" in crypto),
                                typeof crypto.subtle,
                                String("SubtleCrypto" in self),
                                String("CryptoKey" in self)
                            ].join("|"));
                        `;
                        const urls = [
                            "data:text/javascript," + encodeURIComponent(source),
                            URL.createObjectURL(new Blob([source], { type: "text/javascript" }))
                        ];
                        for (const url of urls) {
                            const worker = new Worker(url);
                            worker.onmessage = (event) => {
                                globalThis.__workerCryptoResults.push(event.data);
                                if (globalThis.__workerCryptoResults.length === urls.length) {
                                    globalThis.__workerCryptoDone = true;
                                }
                            };
                            worker.onerror = (event) => {
                                globalThis.__workerCryptoResults.push("error:" + event.message);
                                globalThis.__workerCryptoDone = true;
                            };
                        }
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerCryptoDone === true)",
                    "secure data/blob workers should report WebCrypto exposure",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerCryptoResults.sort())")
            })
            .await
            .expect("secure worker WebCrypto exposure test should run on owner lane");

        assert_eq!(
            results,
            r#"["true|object|true|true","true|object|true|true"]"#
        );
    })
    .await;
}

#[tokio::test]
async fn shared_worker_creation_context_uses_document_secure_context_not_base_url() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const base = document.createElement("base");
                        base.href = "http://example.com/not-secure-base/";
                        document.head.appendChild(base);

                        globalThis.__sharedWorkerMessages = [];
                        globalThis.__sharedWorkerDone = false;
                        const source = `
                            onconnect = (event) => {
                                const port = event.ports[0];
                                port.postMessage([
                                    String("subtle" in crypto),
                                    typeof crypto.subtle,
                                    String("SubtleCrypto" in self),
                                    String("CryptoKey" in self)
                                ].join("|"));
                            };
                        `;
                        const url = "data:text/javascript," + encodeURIComponent(source);
                        const worker = new SharedWorker(url, "secure-context-not-base-url");
                        worker.onerror = (event) => {
                            globalThis.__sharedWorkerMessages.push("error:" + event.message);
                            globalThis.__sharedWorkerDone = true;
                        };
                        worker.port.onmessage = (event) => {
                            globalThis.__sharedWorkerMessages.push(event.data);
                            globalThis.__sharedWorkerDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_probe(
                    &mut page_vm,
                    "SharedWorker creation context should ignore document base URL",
                )
                .await?;
                assert_eq!(
                    shared_worker_probe_messages(&mut page_vm)?,
                    "true|object|true|true"
                );
                anyhow::Ok(())
            })
            .await
            .expect("shared worker secure-context base URL test should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn data_and_blob_workers_hide_subtle_crypto_from_nonsecure_creator_context() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("http://example.test/page.html").expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let results = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerCryptoResults = [];
                        globalThis.__workerCryptoDone = false;
                        const source = `
                            postMessage([
                                String("subtle" in crypto),
                                typeof crypto.subtle,
                                String("SubtleCrypto" in self),
                                String("CryptoKey" in self)
                            ].join("|"));
                        `;
                        const urls = [
                            "data:text/javascript," + encodeURIComponent(source),
                            URL.createObjectURL(new Blob([source], { type: "text/javascript" }))
                        ];
                        for (const url of urls) {
                            const worker = new Worker(url);
                            worker.onmessage = (event) => {
                                globalThis.__workerCryptoResults.push(event.data);
                                if (globalThis.__workerCryptoResults.length === urls.length) {
                                    globalThis.__workerCryptoDone = true;
                                }
                            };
                            worker.onerror = (event) => {
                                globalThis.__workerCryptoResults.push("error:" + event.message);
                                globalThis.__workerCryptoDone = true;
                            };
                        }
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerCryptoDone === true)",
                    "non-secure data/blob workers should report WebCrypto exposure",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerCryptoResults.sort())")
            })
            .await
            .expect("non-secure worker WebCrypto exposure test should run on owner lane");

        assert_eq!(
            results,
            r#"["false|undefined|false|false","false|undefined|false|false"]"#
        );
    })
    .await;
}

#[tokio::test]
async fn worker_script_url_query_uses_document_encoding_override() {
    run_page_vm_async_test(async move {
        let (base_url, request_path_rx, server) = spawn_worker_script_path_capture_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        page_vm.set_document_character_set("GBK");
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerQueryDone = false;
                        const worker = new Worker("/worker.js?q=家居");
                        worker.onmessage = () => {
                            globalThis.__workerQueryDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerQueryDone === true)",
                    "worker script should load through document query encoding override",
                )
                .await?;
                anyhow::Ok(())
            })
            .await
            .expect("worker query encoding test should run on owner lane");

        let request_path = request_path_rx
            .await
            .expect("worker script request path should be captured");
        server
            .await
            .expect("worker query encoding server should finish");
        assert_eq!(request_path, "/worker.js?q=%BC%D2%BE%D3");
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_processor_port_store_ignores_global_spoofing() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://audio-worklet-processor-port-state.test/page.html")
            .expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__audioWorkletPortProbe = "pending";
                        const context = new AudioContext();
                        const moduleSource = `
                            globalThis.__moliCurrentAudioWorkletPort = { spoof: "module-global" };
                            Map.prototype.set = () => {
                                throw new Error("registerProcessor observed public Map.prototype.set");
                            };
                            Map.prototype.get = () => {
                                throw new Error("AudioWorkletNode observed public Map.prototype.get");
                            };
                            Array.prototype.push = () => {
                                throw new Error("AudioWorkletNode observed public Array.prototype.push");
                            };
                            registerProcessor("port-probe", class extends AudioWorkletProcessor {
                                constructor() {
                                    super();
                                    this.port.postMessage({
                                        processorPortTag: Object.prototype.toString.call(this.port),
                                        globalSpoof: globalThis.__moliCurrentAudioWorkletPort &&
                                            globalThis.__moliCurrentAudioWorkletPort.spoof,
                                        globalValueTag: Object.prototype.toString.call(
                                            globalThis.__moliCurrentAudioWorkletPort
                                        )
                                    });
                                }
                            });
                        `;
                        const moduleURL = "data:text/javascript," + encodeURIComponent(moduleSource);
                        context.audioWorklet.addModule(moduleURL).then(
                            () => {
                                try {
                                    const node = new AudioWorkletNode(context, "port-probe");
                                    node.port.onmessage = event => {
                                        globalThis.__audioWorkletPortProbe = JSON.stringify({
                                            nodeTag: Object.prototype.toString.call(node),
                                            nodePortTag: Object.prototype.toString.call(node.port),
                                            message: event.data
                                        });
                                        context.close();
                                    };
                                    if (typeof node.port.start === "function") {
                                        node.port.start();
                                    }
                                } catch (error) {
                                    globalThis.__audioWorkletPortProbe =
                                        "node-error:" + (error && error.message ? error.message : String(error));
                                }
                            },
                            error => {
                                globalThis.__audioWorkletPortProbe =
                                    "module-error:" + (error && error.message ? error.message : String(error));
                            }
                        );
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletPortProbe !== 'pending')",
                    "AudioWorklet processor port probe should complete",
                )
                .await?;
                page_vm.vm_mut().eval("globalThis.__audioWorkletPortProbe")
            })
            .await
            .expect("AudioWorklet processor port probe should run on owner lane");

        assert_eq!(
            result,
            r#"{"nodeTag":"[object AudioWorkletNode]","nodePortTag":"[object MessagePort]","message":{"processorPortTag":"[object MessagePort]","globalSpoof":"module-global","globalValueTag":"[object Object]"}}"#
        );
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_add_module_expands_completed_sibling_descendants_before_slow_sibling_finishes()
 {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_audio_worklet_dynamic_descendant_server().await;
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
                        globalThis.__audioWorkletResult = null;
                        globalThis.__audioWorkletDone = false;
                        const context = new AudioContext();
                        context.audioWorklet.addModule({module_url_literal}).then(
                            () => {{
                                globalThis.__audioWorkletResult = "loaded";
                                globalThis.__audioWorkletDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletDone === true)",
                    "AudioWorklet addModule should load through worker dynamic import",
                )
                .await?;
                page_vm.vm_mut().eval("globalThis.__audioWorkletResult")
            })
            .await
            .expect("AudioWorklet addModule descendant test should run on owner lane");

        assert_eq!(result, "loaded");
        server
            .await
            .expect("AudioWorklet descendant server should finish");
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_json_static_import_uses_json_fetch_destination() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_audio_worklet_json_destination_server().await;
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
                        globalThis.__audioWorkletJsonResult = null;
                        globalThis.__audioWorkletJsonDone = false;
                        const context = new AudioContext();
                        context.audioWorklet.addModule({module_url_literal}).then(
                            () => {{
                                globalThis.__audioWorkletJsonResult = "loaded";
                                globalThis.__audioWorkletJsonDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletJsonResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletJsonDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletJsonDone === true)",
                    "AudioWorklet addModule with static JSON import should settle",
                )
                .await?;
                page_vm.vm_mut().eval("globalThis.__audioWorkletJsonResult")
            })
            .await
            .expect("AudioWorklet JSON destination test should run on owner lane");

        assert_eq!(result, "loaded");
        server
            .await
            .expect("AudioWorklet JSON destination server should finish");
    })
    .await;
}

#[test]
fn audio_worklet_static_css_import_rejects_invalid_module_type_without_fetching_dependency() {
    run_page_vm_large_stack_async_test("audio-worklet-static-css-invalid-type", || async {
        run_audio_worklet_static_invalid_module_type_import_test("css", "style.css").await;
    });
}

#[test]
fn audio_worklet_static_text_import_rejects_invalid_module_type_without_fetching_dependency() {
    run_page_vm_large_stack_async_test("audio-worklet-static-text-invalid-type", || async {
        run_audio_worklet_static_invalid_module_type_import_test("text", "text.txt").await;
    });
}

#[tokio::test]
async fn audio_worklet_add_module_rejects_user_dynamic_import_without_fetching_dependency() {
    run_page_vm_async_test(async move {
        let (base_url, stop_dynamic_probe, server) =
            spawn_audio_worklet_dynamic_import_forbidden_server().await;
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
                        globalThis.__audioWorkletResult = null;
                        globalThis.__audioWorkletDone = false;
                        const context = new AudioContext();
                        context.audioWorklet.addModule({module_url_literal}).then(
                            () => {{
                                globalThis.__audioWorkletResult = "loaded";
                                globalThis.__audioWorkletDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletDone === true)",
                    "AudioWorklet addModule should finish after user dynamic import rejection",
                )
                .await?;
                page_vm.vm_mut().eval("globalThis.__audioWorkletResult")
            })
            .await
            .expect("AudioWorklet dynamic import rejection test should run on owner lane");

        let _ = stop_dynamic_probe.send(());
        let dynamic_request = server
            .await
            .expect("AudioWorklet dynamic import rejection server should finish");
        assert_eq!(
            result, "loaded",
            "AudioWorklet addModule should resolve after first evaluation even when worklet import() rejects"
        );
        assert_eq!(
            dynamic_request, None,
            "rejected AudioWorklet import() must not fetch a dynamic dependency"
        );
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_add_module_resolves_after_top_level_throw_and_keeps_registered_processor() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_shared_worker_script_capture_http_server(
            "registerProcessor('before-throw', class extends AudioWorkletProcessor {}); throw new Error('top-level boom');",
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let module_url = format!("{base_url}/worklet/throw.js");
        let module_url_literal =
            serde_json::to_string(&module_url).expect("serialize worklet module URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__audioWorkletThrowResult = null;
                        globalThis.__audioWorkletThrowDone = false;
                        const context = new AudioContext();
                        context.audioWorklet.addModule({module_url_literal}).then(
                            () => {{
                                try {{
                                    new AudioWorkletNode(context, "before-throw");
                                    globalThis.__audioWorkletThrowResult = "loaded";
                                }} catch (error) {{
                                    globalThis.__audioWorkletThrowResult =
                                        "node-error:" + (error && error.message ? error.message : String(error));
                                }}
                                globalThis.__audioWorkletThrowDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletThrowResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletThrowDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletThrowDone === true)",
                    "AudioWorklet addModule should resolve after top-level evaluation throw",
                )
                .await?;
                page_vm.vm_mut().eval("globalThis.__audioWorkletThrowResult")
            })
            .await
            .expect("AudioWorklet top-level throw test should run on owner lane");

        let _request = request_rx
            .await
            .expect("AudioWorklet top-level throw request should be captured");
        server
            .await
            .expect("AudioWorklet top-level throw server should finish");
        assert_eq!(result, "loaded");
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_joined_add_module_resolves_after_top_level_throw() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_shared_worker_script_capture_http_server(
            "registerProcessor('joined-before-throw', class extends AudioWorkletProcessor {}); throw new Error('top-level boom');",
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let module_url = format!("{base_url}/worklet/joined-throw.js");
        let module_url_literal =
            serde_json::to_string(&module_url).expect("serialize worklet module URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__audioWorkletJoinedThrowResult = null;
                        globalThis.__audioWorkletJoinedThrowDone = false;
                        const context = new AudioContext();
                        Promise.all([
                            context.audioWorklet.addModule({module_url_literal}),
                            context.audioWorklet.addModule({module_url_literal})
                        ]).then(
                            () => {{
                                try {{
                                    new AudioWorkletNode(context, "joined-before-throw");
                                    globalThis.__audioWorkletJoinedThrowResult = "loaded";
                                }} catch (error) {{
                                    globalThis.__audioWorkletJoinedThrowResult =
                                        "node-error:" + (error && error.message ? error.message : String(error));
                                }}
                                globalThis.__audioWorkletJoinedThrowDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletJoinedThrowResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletJoinedThrowDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletJoinedThrowDone === true)",
                    "joined AudioWorklet addModule should resolve after top-level evaluation throw",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__audioWorkletJoinedThrowResult")
            })
            .await
            .expect("joined AudioWorklet top-level throw test should run on owner lane");

        let _request = request_rx
            .await
            .expect("joined AudioWorklet top-level throw request should be captured");
        server
            .await
            .expect("joined AudioWorklet top-level throw server should finish");
        assert_eq!(result, "loaded");
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_add_module_reuses_repeated_module_response() {
    run_page_vm_async_test(async move {
        let (base_url, request_count_rx, server) =
            spawn_audio_worklet_repeated_add_module_server().await;
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
                        globalThis.__audioWorkletResult = null;
                        globalThis.__audioWorkletDone = false;
                        const context = new AudioContext();
                        Promise.all([
                            context.audioWorklet.addModule({module_url_literal}),
                            context.audioWorklet.addModule({module_url_literal})
                        ]).then(
                            () => {{
                                try {{
                                    new AudioWorkletNode(context, "cached");
                                    globalThis.__audioWorkletResult = "loaded";
                                }} catch (error) {{
                                    globalThis.__audioWorkletResult =
                                        "node-error:" + (error && error.message ? error.message : String(error));
                                }}
                                globalThis.__audioWorkletDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletDone === true)",
                    "repeated AudioWorklet addModule calls should join one module response",
                )
                .await?;
                page_vm.vm_mut().eval("globalThis.__audioWorkletResult")
            })
            .await
            .expect("repeated AudioWorklet addModule test should run on owner lane");

        let request_count = request_count_rx
            .await
            .expect("AudioWorklet repeated module request count");
        server
            .await
            .expect("AudioWorklet repeated module server should finish");
        assert_eq!(result, "loaded");
        assert_eq!(
            request_count, 1,
            "repeated AudioWorklet addModule calls should reuse the same module response"
        );
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_add_module_reuses_failed_module_response() {
    run_page_vm_async_test(async move {
        let (base_url, request_count_rx, server) =
            spawn_audio_worklet_failed_repeated_add_module_server().await;
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
                        globalThis.__audioWorkletFailedReuseResult = null;
                        globalThis.__audioWorkletFailedReuseDone = false;
                        const context = new AudioContext();
                        const observe = promise => promise.then(
                            () => "resolved",
                            error => "rejected:" + (error && error.name ? error.name : String(error))
                        );
                        (async () => {{
                            const first = await observe(context.audioWorklet.addModule({module_url_literal}));
                            const second = await observe(context.audioWorklet.addModule({module_url_literal}));
                            globalThis.__audioWorkletFailedReuseResult = `${{first}}|${{second}}`;
                            globalThis.__audioWorkletFailedReuseDone = true;
                        }})();
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletFailedReuseDone === true)",
                    "failed AudioWorklet addModule calls should reuse failed module response",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__audioWorkletFailedReuseResult")
            })
            .await
            .expect("failed AudioWorklet addModule reuse test should run on owner lane");

        let request_count = request_count_rx
            .await
            .expect("AudioWorklet failed repeated module request count");
        server
            .await
            .expect("AudioWorklet failed repeated module server should finish");
        assert!(
            result.starts_with("rejected:"),
            "first failed AudioWorklet addModule should reject: {result}"
        );
        assert!(
            result.contains("|rejected:"),
            "second failed AudioWorklet addModule should reuse rejected response: {result}"
        );
        assert_eq!(
            request_count, 1,
            "failed AudioWorklet addModule calls should reuse the failed module response"
        );
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_close_rejects_pending_add_module_response() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, release_tx, server) =
            spawn_audio_worklet_hanging_add_module_server().await;
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
                        globalThis.__audioWorkletCloseResult = null;
                        globalThis.__audioWorkletCloseDone = false;
                        const context = new AudioContext();
                        globalThis.__audioWorkletCloseContext = context;
                        context.audioWorklet.addModule({module_url_literal}).then(
                            () => {{
                                globalThis.__audioWorkletCloseResult = "resolved";
                                globalThis.__audioWorkletCloseDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletCloseResult = [
                                    error && error.name ? error.name : "",
                                    error && error.message ? error.message : String(error)
                                ].join(":");
                                globalThis.__audioWorkletCloseDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                let request_path = tokio::time::timeout(Duration::from_secs(2), request_rx)
                    .await
                    .expect("pending AudioWorklet module request should start before timeout")
                    .expect("pending AudioWorklet module request path should be sent");
                assert_eq!(request_path, "/worklet/entry.js");
                page_vm
                    .vm_mut()
                    .eval("void globalThis.__audioWorkletCloseContext.close()")?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletCloseDone === true)",
                    "AudioContext close should reject pending AudioWorklet addModule",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__audioWorkletCloseResult")
            })
            .await
            .expect("AudioWorklet close pending addModule test should run on owner lane");

        let _ = release_tx.send(());
        server
            .await
            .expect("AudioWorklet hanging module server should finish");
        assert!(
            result.starts_with("AbortError:"),
            "AudioContext close should reject pending addModule with AbortError: {result}"
        );
    })
    .await;
}

#[tokio::test]
async fn audio_worklet_add_module_credentials_omit_omits_script_cookies() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_shared_worker_script_capture_http_server(
            "registerProcessor('credentials', class extends AudioWorkletProcessor {});",
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let module_url = format!("{base_url}/worklet/credentials.js");
        let module_url_literal =
            serde_json::to_string(&module_url).expect("serialize worklet module URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        document.cookie = "aw_module_cookie=sent; Path=/";
                        globalThis.__audioWorkletCredentialsResult = null;
                        globalThis.__audioWorkletCredentialsDone = false;
                        const context = new AudioContext();
                        context.audioWorklet.addModule({module_url_literal}, {{
                            credentials: "omit"
                        }}).then(
                            () => {{
                                globalThis.__audioWorkletCredentialsResult = "loaded";
                                globalThis.__audioWorkletCredentialsDone = true;
                            }},
                            (error) => {{
                                globalThis.__audioWorkletCredentialsResult =
                                    "error:" + (error && error.message ? error.message : String(error));
                                globalThis.__audioWorkletCredentialsDone = true;
                            }}
                        );
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__audioWorkletCredentialsDone === true)",
                    "AudioWorklet addModule credentials=omit should settle",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__audioWorkletCredentialsResult")
            })
            .await
            .expect("AudioWorklet credentials test should run on owner lane");
        assert_eq!(result, "loaded");

        let request = request_rx
            .await
            .expect("AudioWorklet credentials test should capture request");
        server
            .await
            .expect("AudioWorklet credentials server should finish");
        assert!(
            !request.contains("aw_module_cookie=sent"),
            "credentials=omit must not send document cookie on AudioWorklet module fetch, request was:\n{request}"
        );
        assert!(
            request.to_ascii_lowercase().contains("sec-fetch-dest: audioworklet\r\n"),
            "AudioWorklet addModule must use the audioworklet fetch destination, request was:\n{request}"
        );
    })
    .await;
}

#[tokio::test]
async fn busy_worker_terminate_interrupts_execution_and_drops_queued_messages() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__busyWorkerDone = false;
                        globalThis.__busyWorkerLast = -1;
                        globalThis.__busyWorkerUnexpected = false;
                        const source = `
                            onmessage = function() {
                                for (let i = 0; true; i++) {
                                    if (i % 1000 === 0) {
                                        postMessage(i);
                                    }
                                }
                            };
                        `;
                        const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
                        worker.onmessage = (event) => {
                            globalThis.__busyWorkerLast = event.data;
                            if (event.data >= 10000) {
                                worker.terminate();
                                worker.onmessage = () => {
                                    globalThis.__busyWorkerUnexpected = true;
                                };
                                setTimeout(() => {
                                    globalThis.__busyWorkerDone = true;
                                }, 100);
                            }
                        };
                        worker.postMessage("go");
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__busyWorkerDone === true)",
                    "busy worker terminate should complete",
                )
                .await?;
                let result = page_vm.vm_mut().eval(
                    "JSON.stringify({last: globalThis.__busyWorkerLast, unexpected: globalThis.__busyWorkerUnexpected})",
                )?;
                assert_eq!(result, r#"{"last":10000,"unexpected":false}"#);
                anyhow::Ok(())
            })
            .await
            .expect("busy worker terminate test should run on owner lane");
    })
    .await;
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

#[tokio::test]
async fn worker_websocket_open_and_frames_record_page_network_trace_entries() {
    run_page_vm_async_test(async move {
            let (url, server) = spawn_text_echo_websocket_server().await;
            let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();
            let drain_url = url.clone();

            let network_output = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                    (() => {{
                        globalThis.__workerWsDone = false;
                        const source = `
                            const socket = new WebSocket({url_literal});
                            socket.addEventListener('open', () => {{
                                socket.send('worker-trace-frame');
                            }});
                            socket.addEventListener('message', () => {{
                                socket.close(1000, 'worker-trace');
                            }});
                            socket.addEventListener('close', () => {{
                                postMessage('done');
                            }});
                        `;
                        const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
                        worker.onmessage = () => {{
                            globalThis.__workerWsDone = true;
                        }};
                    }})()
                    "#
                    ))?;

                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerWsDone === true)",
                        "worker websocket trace event should arrive",
                    )
                    .await?;
                    drain_until_websocket_trace_output(&mut page_vm, &drain_url).await
            })
                .await
                .expect("worker websocket trace test should run on owner lane");
            server.await.expect("worker websocket trace server should finish");
            let (records, frame_events, lifecycle_events) = split_network_output_items(network_output);

            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
            assert_eq!(record.method(), "GET");
            assert_eq!(record.url().as_str(), url);
            let socket_id = record
                .websocket_socket_id()
                .expect("worker websocket record should carry socket id");
            assert_ne!(socket_id & (1_u64 << 63), 0);
            match record.outcome() {
                SubresourceNetworkOutcome::Success {
                    status,
                    response_headers,
                    response_body,
                    ..
                } => {
                    assert_eq!(*status, 101);
                    assert!(response_body.is_empty());
                    assert!(
                        response_headers
                            .iter()
                            .any(|(name, _)| name.eq_ignore_ascii_case("sec-websocket-accept"))
                    );
                }
                outcome => panic!("expected worker websocket success record, got {outcome:?}"),
            }

            assert_eq!(frame_events.len(), 2);
            assert!(frame_events.iter().all(|event| event.socket_id() == socket_id));
            assert_eq!(
                frame_events[0].direction(),
                crate::types::WebSocketFrameDirection::Sent
            );
            assert_eq!(
                frame_events[0].opcode(),
                crate::types::WebSocketFrameOpcode::Text
            );
            assert_eq!(frame_events[0].payload_length(), "worker-trace-frame".len());
            assert_eq!(
                frame_events[1].direction(),
                crate::types::WebSocketFrameDirection::Received
            );
            assert_eq!(
                frame_events[1].opcode(),
                crate::types::WebSocketFrameOpcode::Text
            );
            assert_eq!(frame_events[1].payload_length(), "worker-trace-frame".len());

            assert_eq!(lifecycle_events.len(), 3);
            assert!(
                lifecycle_events
                    .iter()
                    .all(|event| event.socket_id() == socket_id)
            );
            assert_eq!(
                lifecycle_events[0].kind(),
                crate::types::WebSocketLifecycleKind::Open
            );
            assert_eq!(
                lifecycle_events[1].kind(),
                crate::types::WebSocketLifecycleKind::Closing
            );
            assert_eq!(
                lifecycle_events[2].kind(),
                crate::types::WebSocketLifecycleKind::Close
            );
            assert_eq!(lifecycle_events[2].close_code(), Some(1000));
            assert_eq!(lifecycle_events[2].close_reason(), Some("worker-trace"));
            assert_eq!(lifecycle_events[2].was_clean(), Some(true));
        })
        .await;
}

#[tokio::test]
async fn shared_worker_websocket_records_page_network_trace_entries() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();
        let drain_url = url.clone();

        let network_output = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__sharedWorkerWsDone = false;
                        globalThis.__sharedWorkerWsResult = null;
                        const source = `
                            onconnect = (event) => {{
                                const port = event.ports[0];
                                const socket = new WebSocket({url_literal});
                                socket.addEventListener("open", () => {{
                                    socket.send("shared-worker-trace-frame");
                                }});
                                socket.addEventListener("message", () => {{
                                    socket.close(1000, "shared-worker-trace");
                                }});
                                socket.addEventListener("close", () => {{
                                    port.postMessage("done");
                                }});
                                socket.addEventListener("error", (error) => {{
                                    port.postMessage("error:" + error.message);
                                }});
                            }};
                        `;
                        const worker = new SharedWorker(
                            "data:text/javascript," + encodeURIComponent(source),
                            "shared-worker-websocket-resource-bridge",
                        );
                        worker.port.onmessage = (event) => {{
                            globalThis.__sharedWorkerWsResult = event.data;
                            globalThis.__sharedWorkerWsDone = true;
                        }};
                        worker.port.start();
                    }})()
                    "#
                ))?;

                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerWsDone === true)",
                    "shared worker websocket trace event should arrive",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__sharedWorkerWsResult")?,
                    "done"
                );
                drain_until_websocket_trace_output(&mut page_vm, &drain_url).await
            })
            .await
            .expect("shared worker websocket trace test should run on owner lane");

        server
            .await
            .expect("shared worker websocket trace server should finish");

        let (records, frame_events, lifecycle_events) = split_network_output_items(network_output);
        let record = records
            .iter()
            .find(|record| record.url().as_str() == url)
            .unwrap_or_else(|| {
                panic!(
                    "shared worker websocket should emit a page network record; records={records:?}"
                )
            });
        assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
        assert_eq!(record.method(), "GET");
        let socket_id = record
            .websocket_socket_id()
            .expect("shared worker websocket record should carry socket id");
        assert_eq!(socket_id >> 62, 0b11);
        let SubresourceNetworkOutcome::Success { status, .. } = record.outcome() else {
            panic!(
                "expected shared worker websocket network success, got {:?}",
                record.outcome()
            );
        };
        assert_eq!(*status, 101);

        assert_eq!(frame_events.len(), 2);
        assert!(
            frame_events
                .iter()
                .all(|event| event.socket_id() == socket_id)
        );
        assert_eq!(lifecycle_events.len(), 3);
        assert!(
            lifecycle_events
                .iter()
                .all(|event| event.socket_id() == socket_id)
        );
    })
    .await;
}

#[tokio::test]
async fn shared_worker_indexed_db_roundtrips_opfs_handle() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let indexed_db_manager = crate::new_indexed_db_manager(None)
            .expect("SharedWorker IndexedDB manager should initialize");
        page_vm.vm_mut().set_indexed_db_manager(Some(
            crate::downgrade_indexed_db_manager(&indexed_db_manager),
        ));
        page_vm
            .vm_mut()
            .set_storage_bucket_store(
                crate::new_shared_storage_bucket_store_with_indexed_db_manager(
                    &indexed_db_manager,
                ),
            );
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerMessages = [];
                        globalThis.__sharedWorkerDone = false;
                        const source = `
                            const request = request => new Promise((resolve, reject) => {
                                request.onsuccess = () => resolve(request.result);
                                request.onerror = () => reject(request.error);
                            });
                            const transaction = transaction => new Promise((resolve, reject) => {
                                transaction.oncomplete = () => resolve();
                                transaction.onabort = transaction.onerror = () => reject(transaction.error);
                            });
                            onconnect = event => {
                                const port = event.ports[0];
                                (async () => {
                                    const root = await navigator.storage.getDirectory();
                                    const file = await root.getFileHandle("shared-worker.txt", {
                                        create: true
                                    });
                                    const writer = await file.createWritable();
                                    await writer.write("shared durable bytes");
                                    await writer.close();

                                    const open = indexedDB.open("shared-worker-opfs-handles", 1);
                                    open.onupgradeneeded = () => open.result.createObjectStore("values");
                                    const db = await request(open);
                                    const writeTx = db.transaction("values", "readwrite");
                                    const writeDone = transaction(writeTx);
                                    await request(writeTx.objectStore("values").put(file, "handle"));
                                    await writeDone;
                                    const clone = await request(
                                        db.transaction("values").objectStore("values").get("handle")
                                    );
                                    port.postMessage(JSON.stringify({
                                        brand: clone instanceof FileSystemFileHandle,
                                        name: clone.name,
                                        distinct: clone !== file,
                                        syncAccessHandleConstructor:
                                          typeof FileSystemSyncAccessHandle,
                                        syncAccessHandleMethod:
                                          typeof FileSystemFileHandle.prototype
                                            .createSyncAccessHandle,
                                        sameEntry: await clone.isSameEntry(file),
                                        resolved: await root.resolve(clone),
                                        text: await (await clone.getFile()).text()
                                    }));
                                    db.close();
                                })().catch(error => {
                                    port.postMessage(
                                        "error:" + (error && error.name) + ":" +
                                            (error && error.message)
                                    );
                                });
                            };
                        `;
                        const scriptUrl = URL.createObjectURL(new Blob([source], {
                            type: "application/javascript"
                        }));
                        const worker = new SharedWorker(
                            scriptUrl,
                            "shared-worker-indexeddb-opfs-handle",
                        );
                        worker.port.onmessage = event => {
                            URL.revokeObjectURL(scriptUrl);
                            globalThis.__sharedWorkerMessages.push(event.data);
                            globalThis.__sharedWorkerDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_probe(
                    &mut page_vm,
                    "SharedWorker IndexedDB OPFS handle round-trip should complete",
                )
                .await?;
                shared_worker_probe_messages(&mut page_vm)
            })
            .await
            .expect("SharedWorker IndexedDB OPFS test should run on owner lane");

        assert_eq!(
            result,
            r#"{"brand":true,"name":"shared-worker.txt","distinct":true,"syncAccessHandleConstructor":"undefined","syncAccessHandleMethod":"undefined","sameEntry":true,"resolved":["shared-worker.txt"],"text":"shared durable bytes"}"#
        );
    })
    .await;
}

#[tokio::test]
async fn shared_worker_websocket_connect_src_self_allows_same_host_ws() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_shared_worker_self_csp_websocket_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__sharedWorkerWsSelfCspDone = false;
                        globalThis.__sharedWorkerWsSelfCspResult = null;
                        const worker = new SharedWorker(
                            "/sw.js",
                            "shared-worker-websocket-self-csp",
                        );
                        worker.onerror = (event) => {
                            globalThis.__sharedWorkerWsSelfCspResult = "worker-error:" + event.message;
                            globalThis.__sharedWorkerWsSelfCspDone = true;
                        };
                        worker.port.onmessage = (event) => {
                            globalThis.__sharedWorkerWsSelfCspResult = event.data;
                            globalThis.__sharedWorkerWsSelfCspDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_until_done(
                    &mut page_vm,
                    "String(globalThis.__sharedWorkerWsSelfCspDone === true)",
                    "SharedWorker WebSocket connect-src self message should arrive",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__sharedWorkerWsSelfCspResult")?,
                    "shared-worker-self-csp"
                );
                anyhow::Ok(())
            })
            .await
            .expect("shared worker websocket self CSP test should run on owner lane");

        server
            .await
            .expect("shared worker websocket self CSP server should finish");
    })
    .await;
}

#[tokio::test]
async fn shared_worker_message_port_wasm_module_cross_agent_cluster_fires_messageerror() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
                        const workerSource = `
                            const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
                            const module = new WebAssembly.Module(bytes);
                            onconnect = (event) => {
                                const port = event.ports[0];
                                port.onmessage = (event) => {
                                    if (event.data === "send-module-to-page") {
                                        port.postMessage(module);
                                    } else {
                                        port.postMessage(
                                            "unexpected-message:" + (event.data instanceof WebAssembly.Module)
                                        );
                                    }
                                };
                                port.onmessageerror = (event) => {
                                    port.postMessage("shared-messageerror:" + (event.data === null));
                                };
                                port.start();
                                port.postMessage("ready");
                            };
                        `;
                        const worker = new SharedWorker(
                            "data:text/javascript," + encodeURIComponent(workerSource),
                            "wasm-agent-cluster-messageerror",
                        );
                        const module = new WebAssembly.Module(bytes);
                        globalThis.__sharedWorkerMessages = [];
                        globalThis.__sharedWorkerDone = false;
                        worker.port.onmessage = (event) => {
                            globalThis.__sharedWorkerMessages.push("message:" + event.data);
                            if (event.data === "ready") {
                                worker.port.postMessage(module);
                            } else if (event.data === "shared-messageerror:true") {
                                worker.port.postMessage("send-module-to-page");
                            } else {
                                globalThis.__sharedWorkerDone = true;
                            }
                        };
                        worker.port.onmessageerror = (event) => {
                            globalThis.__sharedWorkerMessages.push("messageerror:" + (event.data === null));
                            globalThis.__sharedWorkerDone = true;
                        };
                        worker.port.start();
                    })()
                    "#,
                )?;
                drive_shared_worker_probe(
                    &mut page_vm,
                    "SharedWorker WebAssembly.Module cross-agent messageerror should complete",
                )
                .await?;
                shared_worker_probe_messages(&mut page_vm)
            })
            .await
            .expect("SharedWorker wasm module messageerror test should run on owner lane");

        assert_eq!(
            result,
            "message:ready|message:shared-messageerror:true|messageerror:true"
        );
    })
    .await;
}

#[tokio::test]
async fn worker_arraybuffer_round_trip_supports_dataview_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerResult = null;
                            globalThis.__workerDone = false;
                            const worker = new Worker(
                                "data:text/javascript;base64,b25tZXNzYWdlID0gZnVuY3Rpb24oZXZlbnQpIHsgY29uc3QgdmlldyA9IG5ldyBEYXRhVmlldyhldmVudC5kYXRhKTsgcG9zdE1lc3NhZ2UobmV3IFVpbnQ4QXJyYXkoW3ZpZXcuZ2V0VWludDgoMCkgKyAxLCB2aWV3LmdldFVpbnQ4KDEpICsgMV0pKTsgfTs="
                            );
                            worker.onmessage = (event) => {
                                globalThis.__workerResult = [
                                    event.data.constructor.name,
                                    event.data.length,
                                    Array.from(event.data).join(','),
                                    String(event.data.buffer instanceof ArrayBuffer)
                                ].join('|');
                                globalThis.__workerDone = true;
                            };
                            worker.postMessage(new Uint8Array([40, 41]).buffer);
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerDone === true)",
                        "worker ArrayBuffer round-trip should complete",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__workerResult")
                })
                .await
                .expect("worker ArrayBuffer round-trip test should run on owner lane");

            assert_eq!(result, "Uint8Array|2|41,42|true");
        })
        .await;
}

#[tokio::test]
async fn worker_resizable_arraybuffer_transfers_preserve_tracking_views() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerResizableTransferResult = null;
                        globalThis.__workerResizableTransferDone = false;
                        const worker = new Worker(
                            "data:text/javascript,onmessage = (event) => postMessage(event.data, event.data.transfer);"
                        );
                        const direct = new ArrayBuffer(16, { maxByteLength: 1024 });
                        const fixed = new Uint8Array([17]).buffer;
                        const typedBuffer = new ArrayBuffer(16, { maxByteLength: 1024 });
                        const dataViewBuffer = new ArrayBuffer(16, { maxByteLength: 1024 });
                        new Uint8Array(direct)[0] = 7;
                        const typed = new Uint8Array(typedBuffer);
                        typed[0] = 11;
                        const dataView = new DataView(dataViewBuffer);
                        dataView.setUint8(0, 13);
                        worker.onmessage = (event) => {
                            const directCopy = event.data.direct;
                            const fixedCopy = event.data.fixed;
                            const typedCopy = event.data.typed;
                            const dataViewCopy = event.data.dataView;
                            const detached = [
                                direct.byteLength,
                                fixed.byteLength,
                                typedBuffer.byteLength,
                                dataViewBuffer.byteLength,
                            ].join(',');
                            const directBefore = [
                                directCopy.byteLength,
                                directCopy.maxByteLength,
                                directCopy.resizable,
                                new Uint8Array(directCopy)[0],
                            ].join(',');
                            const typedBefore = [
                                typedCopy.byteLength,
                                typedCopy.buffer.maxByteLength,
                                typedCopy.buffer.resizable,
                                typedCopy[0],
                            ].join(',');
                            const dataViewBefore = [
                                dataViewCopy.byteLength,
                                dataViewCopy.buffer.maxByteLength,
                                dataViewCopy.buffer.resizable,
                                dataViewCopy.getUint8(0),
                            ].join(',');
                            directCopy.resize(32);
                            typedCopy.buffer.resize(32);
                            dataViewCopy.buffer.resize(32);
                            globalThis.__workerResizableTransferResult = [
                                detached,
                                `${directBefore},${directCopy.byteLength}`,
                                `${fixedCopy.byteLength},${new Uint8Array(fixedCopy)[0]}`,
                                `${typedBefore},${typedCopy.byteLength}`,
                                `${dataViewBefore},${dataViewCopy.byteLength}`,
                            ].join('|');
                            worker.terminate();
                            globalThis.__workerResizableTransferDone = true;
                        };
                        const transfer = [direct, fixed, typedBuffer, dataViewBuffer];
                        worker.postMessage(
                            { direct, fixed, typed, dataView, transfer },
                            transfer,
                        );
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerResizableTransferDone === true)",
                    "worker resizable ArrayBuffer transfer should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__workerResizableTransferResult")
            })
            .await
            .expect("worker resizable ArrayBuffer transfer test should run on owner lane");

        assert_eq!(
            result,
            "0,0,0,0|16,1024,true,7,32|1,17|16,1024,true,11,32|16,1024,true,13,32"
        );
    })
    .await;
}

#[tokio::test]
async fn worker_arraybuffer_from_worker_arrives_as_arraybuffer_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerResult = null;
                            globalThis.__workerDone = false;
                            const worker = new Worker(
                                "data:text/javascript;base64,cG9zdE1lc3NhZ2UobmV3IFVpbnQ4QXJyYXkoWzcsIDgsIDldKS5idWZmZXIpOw=="
                            );
                            worker.onmessage = (event) => {
                                globalThis.__workerResult = [
                                    event.data.constructor.name,
                                    event.data.byteLength,
                                    Array.from(new Uint8Array(event.data)).join(',')
                                ].join('|');
                                globalThis.__workerDone = true;
                            };
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerDone === true)",
                        "worker ArrayBuffer delivery should complete",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__workerResult")
                })
                .await
                .expect("worker ArrayBuffer delivery test should run on owner lane");

            assert_eq!(result, "ArrayBuffer|3|7,8,9");
        })
        .await;
}

#[tokio::test]

async fn worker_messageport_transfer_from_window_to_worker_round_trips_messages() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__messagePortResult = null;
                            globalThis.__messagePortDone = false;
                            const worker = new Worker(
                                "data:text/javascript,onmessage = (event) => { if (event.data === 'connect') { const port = event.ports[0]; port.onmessage = (messageEvent) => { port.postMessage(`pong:${messageEvent.data}`); }; postMessage('worker-ready'); } };"
                            );
                            const channel = new MessageChannel();
                            channel.port1.onmessage = (event) => {
                                globalThis.__messagePortResult = [
                                    event.data,
                                    String(worker !== null),
                                ].join('|');
                                globalThis.__messagePortDone = true;
                            };
                            worker.onmessage = (event) => {
                                if (event.data === 'worker-ready') {
                                    channel.port1.postMessage('ping');
                                }
                            };
                            worker.postMessage('connect', [channel.port2]);
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__messagePortDone === true)",
                        "worker MessagePort transfer from window should complete",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__messagePortResult")
                })
                .await
                .expect("worker MessagePort transfer test should run on owner lane");

            assert_eq!(result, "pong:ping|true");
        })
        .await;
}

#[tokio::test]
async fn worker_messageport_transfer_uses_intrinsic_prototype_after_global_deletion() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__messagePortIntrinsicResult = null;
                        globalThis.__messagePortIntrinsicDone = false;
                        const worker = new Worker(
                            "data:text/javascript,onmessage = (event) => postMessage(event.data, event.data.transfer);"
                        );
                        const { port1 } = new MessageChannel();
                        const messagePortInterface = globalThis.MessagePort;
                        delete globalThis.MessagePort;
                        worker.onmessage = (event) => {
                            const transferred = event.data.data;
                            globalThis.__messagePortIntrinsicResult = [
                                transferred instanceof messagePortInterface,
                                Object.getPrototypeOf(transferred) === messagePortInterface.prototype,
                                typeof globalThis.MessagePort,
                            ].join('|');
                            globalThis.MessagePort = messagePortInterface;
                            transferred.close();
                            worker.terminate();
                            globalThis.__messagePortIntrinsicDone = true;
                        };
                        worker.postMessage({ data: port1, transfer: [port1] }, [port1]);
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortIntrinsicDone === true)",
                    "worker MessagePort transfer should not depend on the global constructor",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortIntrinsicResult")
            })
            .await
            .expect("worker MessagePort intrinsic prototype test should run on owner lane");

        assert_eq!(result, "true|true|undefined");
    })
    .await;
}

#[tokio::test]
async fn worker_messageport_close_preserves_same_task_queued_messages() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__messagePortCloseRaceResult = null;
                            globalThis.__messagePortCloseRaceDone = false;
                            const worker = new Worker(
                                "data:text/javascript," + encodeURIComponent(`
                                onmessage = (event) => {
                                  if (event.data !== 'connect') {
                                    return;
                                  }
                                  const port = event.ports[0];
                                  const messages = [];
                                  port.onmessage = (messageEvent) => {
                                    messages.push(messageEvent.data);
                                    if (messageEvent.data === 'first') {
                                      port.close();
                                    }
                                    if (messageEvent.data === 'second') {
                                      postMessage(messages.join('|'));
                                    }
                                  };
                                  postMessage('ready');
                                };
                                `)
                            );
                            const channel = new MessageChannel();
                            worker.onmessage = (event) => {
                                if (event.data === 'ready') {
                                    channel.port1.postMessage('first');
                                    channel.port1.postMessage('second');
                                    return;
                                }
                                globalThis.__messagePortCloseRaceResult = event.data;
                                worker.terminate();
                                channel.port1.close();
                                globalThis.__messagePortCloseRaceDone = true;
                            };
                            worker.postMessage('connect', [channel.port2]);
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortCloseRaceDone === true)",
                    "worker MessagePort close same-task queue test should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortCloseRaceResult")
            })
            .await
            .expect("worker MessagePort close queue test should run on owner lane");

        assert_eq!(result, "first|second");
    })
    .await;
}

#[tokio::test]
async fn worker_owned_messageport_start_and_onmessage_activation_work() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__messagePortStartResult = [];
                            globalThis.__messagePortStartDone = false;
                            const worker = new Worker(
                                "data:text/javascript," + encodeURIComponent(`
                                let listenerPort = null;
                                const listenerMessages = [];
                                onmessage = (event) => {
                                  if (event.data === 'listener-connect') {
                                    listenerPort = event.ports[0];
                                    listenerPort.addEventListener('message', (messageEvent) => {
                                      listenerMessages.push(messageEvent.data);
                                      postMessage({
                                        kind: 'listener-result',
                                        beforeStart,
                                        data: listenerMessages.join('|')
                                      });
                                    });
                                    postMessage({ kind: 'listener-ready' });
                                    return;
                                  }
                                  if (event.data === 'start-listener') {
                                    beforeStart = listenerMessages.length;
                                    listenerPort.start();
                                    return;
                                  }
                                  if (event.data === 'onmessage-connect') {
                                    const port = event.ports[0];
                                    port.onmessage = (messageEvent) => {
                                      postMessage({
                                        kind: 'onmessage-result',
                                        data: messageEvent.data
                                      });
                                    };
                                    postMessage({ kind: 'onmessage-ready' });
                                  }
                                };
                                let beforeStart = -1;
                                `)
                            );
                            const listenerChannel = new MessageChannel();
                            const onmessageChannel = new MessageChannel();
                            worker.onmessage = (event) => {
                                if (event.data.kind === 'listener-ready') {
                                    listenerChannel.port1.postMessage('queued');
                                    setTimeout(() => {
                                        worker.postMessage('start-listener');
                                    }, 0);
                                    return;
                                }
                                if (event.data.kind === 'listener-result') {
                                    globalThis.__messagePortStartResult.push(
                                        `listener:${event.data.beforeStart}:${event.data.data}`
                                    );
                                    worker.postMessage('onmessage-connect', [onmessageChannel.port2]);
                                    return;
                                }
                                if (event.data.kind === 'onmessage-ready') {
                                    onmessageChannel.port1.postMessage('auto');
                                    return;
                                }
                                if (event.data.kind === 'onmessage-result') {
                                    globalThis.__messagePortStartResult.push(
                                        `onmessage:${event.data.data}`
                                    );
                                    worker.terminate();
                                    listenerChannel.port1.close();
                                    onmessageChannel.port1.close();
                                    globalThis.__messagePortStartDone = true;
                                }
                            };
                            worker.postMessage('listener-connect', [listenerChannel.port2]);
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__messagePortStartDone === true)",
                        "worker MessagePort start/onmessage activation should complete",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__messagePortStartResult.join('|')")
                })
                .await
                .expect("worker MessagePort activation test should run on owner lane");

            assert_eq!(result, "listener:0:queued|onmessage:auto");
        })
        .await;
}

#[tokio::test]
async fn worker_owned_messageport_options_transfer_null_clone_path_responds() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__messagePortOptionsResult = [];
                            globalThis.__messagePortOptionsDone = false;
                            function nextPortMessage(port, predicate) {
                                port.start();
                                return new Promise((resolve) => {
                                    function onmessage(event) {
                                        if (predicate && !predicate(event.data)) {
                                            return;
                                        }
                                        port.removeEventListener('message', onmessage);
                                        resolve(event);
                                    }
                                    port.addEventListener('message', onmessage);
                                });
                            }
                            async function run() {
                            const worker = new Worker(
                                "data:text/javascript," + encodeURIComponent(`
                                onmessage = function (event) {
                                  if (event.data !== 'connect') {
                                    return;
                                  }
                                  const port = event.ports[0];
                                  port.onmessage = function (messageEvent) {
                                    if (messageEvent.data === 'plain') {
                                      port.postMessage({
                                        kind: 'plain',
                                        portsLength: messageEvent.ports.length
                                      });
                                      return;
                                    }
                                    const buffer = messageEvent.data.buffer;
                                    port.postMessage({
                                      kind: 'clone',
                                      isArrayBuffer: buffer instanceof ArrayBuffer,
                                      byteLength: buffer.byteLength,
                                      bytes: Array.from(new Uint8Array(buffer)).join(','),
                                      portsLength: messageEvent.ports.length
                                    });
                                  };
                                  postMessage({ kind: 'ready' });
                                };
                                `)
                            );
                            const channel = new MessageChannel();
                            const ready = new Promise((resolve) => {
                                worker.onmessage = (event) => {
                                    if (event.data.kind === 'ready') {
                                        resolve(event);
                                    }
                                };
                            });
                            worker.postMessage('connect', [channel.port2]);
                            await ready;
                            globalThis.__messagePortOptionsResult.push('ready');
                            const plain = nextPortMessage(channel.port1, (data) => data && data.kind === 'plain');
                            channel.port1.postMessage('plain', {});
                            const plainEvent = await plain;
                            globalThis.__messagePortOptionsResult.push(
                                `plain:${plainEvent.data.portsLength}`
                            );
                            const buffer = new Uint8Array([9, 10]).buffer;
                            const clone = nextPortMessage(channel.port1, (data) => data && data.kind === 'clone');
                            channel.port1.postMessage(
                                { kind: 'clone', buffer },
                                { transfer: null }
                            );
                            globalThis.__messagePortOptionsResult.push(
                                `attached:${buffer.byteLength}`
                            );
                            const event = await clone;
                            globalThis.__messagePortOptionsResult.push(
                                [
                                    'clone',
                                    event.data.isArrayBuffer,
                                    event.data.byteLength,
                                    event.data.bytes,
                                    event.data.portsLength,
                                ].join(':')
                            );
                            worker.terminate();
                            channel.port1.close();
                            globalThis.__messagePortOptionsDone = true;
                            }
                            run().catch((error) => {
                                globalThis.__messagePortOptionsResult.push(`error:${error && error.message}`);
                            });
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__messagePortOptionsDone === true)",
                        "worker MessagePort options transfer:null clone path should complete",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__messagePortOptionsResult.join('|')")
                })
                .await
                .expect("worker MessagePort options transfer:null test should run on owner lane");

            assert_eq!(result, "ready|plain:0|attached:2|clone:true:2:9,10:0");
        })
        .await;
}

#[tokio::test]
async fn messageport_dispatch_uses_registration_order_for_onmessage_and_listeners() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__messagePortEventOrderResult = [];
                            globalThis.__messagePortEventOrderDone = false;
                            let remaining = 2;
                            function poisonPortInternals(port) {
                                Object.defineProperties(port, {
                                    __lmMessagePortOnmessageHandler: {
                                        value: () => {
                                            throw new Error('own onmessage spoof called');
                                        },
                                        configurable: true
                                    },
                                    __lmMessagePortOnmessageOrder: {
                                        value: -1000,
                                        configurable: true
                                    },
                                    __lmMessagePortNextListenerOrder: {
                                        value: 1000,
                                        configurable: true
                                    },
                                    __moliMessagePortStarted: {
                                        value: false,
                                        configurable: true
                                    },
                                    __moliMessagePortClosed: {
                                        value: true,
                                        configurable: true
                                    },
                                    __moliMessagePortListeners: {
                                        value: [],
                                        configurable: true
                                    }
                                });
                            }
                            function finish(label, order, channel) {
                                if (order.length !== 2) {
                                    return;
                                }
                                globalThis.__messagePortEventOrderResult.push(
                                    `${label}:${order.join(',')}`
                                );
                                channel.port1.close();
                                channel.port2.close();
                                remaining -= 1;
                                if (remaining === 0) {
                                    globalThis.__messagePortEventOrderDone = true;
                                }
                            }

                            const listenerFirst = new MessageChannel();
                            const listenerFirstOrder = [];
                            listenerFirst.port2.addEventListener('message', () => {
                                listenerFirstOrder.push('listener');
                                finish('listener-first', listenerFirstOrder, listenerFirst);
                            });
                            listenerFirst.port2.onmessage = () => {
                                listenerFirstOrder.push('onmessage');
                                finish('listener-first', listenerFirstOrder, listenerFirst);
                            };
                            listenerFirst.port2.start();
                            poisonPortInternals(listenerFirst.port2);
                            listenerFirst.port1.postMessage('go');

                            const onmessageFirst = new MessageChannel();
                            const onmessageFirstOrder = [];
                            onmessageFirst.port2.onmessage = () => {
                                onmessageFirstOrder.push('onmessage');
                                finish('onmessage-first', onmessageFirstOrder, onmessageFirst);
                            };
                            onmessageFirst.port2.addEventListener('message', () => {
                                onmessageFirstOrder.push('listener');
                                finish('onmessage-first', onmessageFirstOrder, onmessageFirst);
                            });
                            onmessageFirst.port2.start();
                            poisonPortInternals(onmessageFirst.port2);
                            onmessageFirst.port1.postMessage('go');
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortEventOrderDone === true)",
                    "MessagePort event order should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortEventOrderResult.sort().join('|')")
            })
            .await
            .expect("MessagePort event order test should run on owner lane");

        assert_eq!(
            result,
            "listener-first:listener,onmessage|onmessage-first:onmessage,listener"
        );
    })
    .await;
}

#[tokio::test]
async fn messageport_listener_options_dedupe_once_and_capture_removal() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__messagePortListenerOptionsResult = [];
                            globalThis.__messagePortListenerOptionsDone = false;
                            let remaining = 3;
                            function finish(label, value, channel) {
                                globalThis.__messagePortListenerOptionsResult.push(
                                    `${label}:${value}`
                                );
                                channel.port1.close();
                                channel.port2.close();
                                remaining -= 1;
                                if (remaining === 0) {
                                    globalThis.__messagePortListenerOptionsDone = true;
                                }
                            }

                            const duplicate = new MessageChannel();
                            let duplicateCalls = 0;
                            function duplicateListener() {
                                duplicateCalls += 1;
                            }
                            duplicate.port2.addEventListener('message', duplicateListener);
                            duplicate.port2.addEventListener('message', duplicateListener);
                            duplicate.port2.addEventListener('message', () => {
                                finish('duplicate', String(duplicateCalls), duplicate);
                            }, { once: true });
                            duplicate.port2.start();
                            duplicate.port1.postMessage('go');

                            const once = new MessageChannel();
                            const onceEvents = [];
                            once.port2.addEventListener('message', (event) => {
                                onceEvents.push(event.data);
                            }, { once: true });
                            once.port2.addEventListener('message', (event) => {
                                if (event.data === 'second') {
                                    setTimeout(() => {
                                        finish('once', onceEvents.join(','), once);
                                    }, 0);
                                }
                            });
                            once.port2.start();
                            once.port1.postMessage('first');
                            once.port1.postMessage('second');

                            const capture = new MessageChannel();
                            let captureCalls = 0;
                            function captureListener() {
                                captureCalls += 1;
                            }
                            capture.port2.addEventListener('message', captureListener);
                            capture.port2.addEventListener('message', captureListener, true);
                            capture.port2.removeEventListener('message', captureListener);
                            capture.port2.addEventListener('message', () => {
                                finish('capture', String(captureCalls), capture);
                            }, { once: true });
                            capture.port2.start();
                            capture.port1.postMessage('go');
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortListenerOptionsDone === true)",
                    "MessagePort listener options should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortListenerOptionsResult.sort().join('|')")
            })
            .await
            .expect("MessagePort listener options test should run on owner lane");

        assert_eq!(result, "capture:1|duplicate:1|once:first");
    })
    .await;
}

#[tokio::test]
async fn messageport_init_event_is_ignored_during_dispatch() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__messagePortInitEventResult = null;
                            globalThis.__messagePortInitEventDone = false;
                            const channel = new MessageChannel();
                            channel.port2.onmessage = (event) => {
                                const before = [
                                    event.type,
                                    event.bubbles,
                                    event.cancelable,
                                    event.target === channel.port2,
                                    event.currentTarget === channel.port2,
                                    event.eventPhase,
                                    event.srcElement === channel.port2,
                                ].join('|');
                                event.initEvent('mutated', true, true);
                                const after = [
                                    event.type,
                                    event.bubbles,
                                    event.cancelable,
                                    event.target === channel.port2,
                                    event.currentTarget === channel.port2,
                                    event.eventPhase,
                                    event.srcElement === channel.port2,
                                ].join('|');
                                globalThis.__messagePortInitEventResult = `${before}->${after}`;
                                channel.port1.close();
                                channel.port2.close();
                                globalThis.__messagePortInitEventDone = true;
                            };
                            channel.port1.postMessage('go');
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortInitEventDone === true)",
                    "MessagePort initEvent dispatch suppression should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortInitEventResult")
            })
            .await
            .expect("MessagePort initEvent suppression test should run on owner lane");

        assert_eq!(
            result,
            "message|false|false|true|true|2|true->message|false|false|true|true|2|true"
        );
    })
    .await;
}

#[tokio::test]
async fn worker_messageport_transfer_from_worker_to_window_round_trips_messages() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerCreatedPortResult = null;
                            globalThis.__workerCreatedPortDone = false;
                            const worker = new Worker(
                                "data:text/javascript,const channel = new MessageChannel(); channel.port1.onmessage = (event) => { channel.port1.postMessage(`worker:${event.data}`); }; postMessage('port-ready', [channel.port2]);"
                            );
                            worker.onmessage = (event) => {
                                if (event.data !== 'port-ready') {
                                    return;
                                }
                                const port = event.ports[0];
                                port.onmessage = (messageEvent) => {
                                    globalThis.__workerCreatedPortResult = [
                                        messageEvent.data,
                                        String(event.ports.length),
                                    ].join('|');
                                    globalThis.__workerCreatedPortDone = true;
                                };
                                port.postMessage('ping');
                            };
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerCreatedPortDone === true)",
                        "worker MessagePort transfer from worker should complete",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__workerCreatedPortResult")
                })
                .await
                .expect("worker-created MessagePort transfer test should run on owner lane");

            assert_eq!(result, "worker:ping|1");
        })
        .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_messageport_without_transfer_list_in_page_vm() {
    let _ = run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            const worker = new Worker("data:text/javascript,postMessage('ready')");
                            const channel = new MessageChannel();
                            try {
                                worker.postMessage(channel.port1);
                                return "unexpected";
                            } catch (error) {
                                return error.name;
                            }
                        })()
                        "#,
                )
            })
            .await
            .expect("worker MessagePort rejection test should run on owner lane");

        assert_eq!(result, "DataCloneError");
        anyhow::Ok(())
    })
    .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_nested_messageport_without_transfer_list_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerNestedPortDone = false;
                            globalThis.__workerNestedPortResult = null;
                            const worker = new Worker("data:text/javascript,postMessage('ready')");
                            const probe = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                worker.postMessage({ port: probe.port1 });
                            } catch (error) {
                                errorName = error.name;
                            }
                            probe.port2.onmessage = (event) => {
                                globalThis.__workerNestedPortResult = `${errorName}|${String(event.data)}`;
                                globalThis.__workerNestedPortDone = true;
                            };
                            probe.port1.postMessage("still-live");
                            worker.terminate();
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerNestedPortDone === true)",
                        "worker nested MessagePort rejection should preserve the port",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__workerNestedPortResult")
                })
                .await
                .expect("worker nested MessagePort rejection test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_duplicate_messageport_options_transfer_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerDuplicateOptionsPortDone = false;
                            globalThis.__workerDuplicateOptionsPortResult = null;
                            const worker = new Worker("data:text/javascript,postMessage('ready')");
                            const probe = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                worker.postMessage("payload", { transfer: [probe.port1, probe.port1] });
                            } catch (error) {
                                errorName = error.name;
                            }
                            probe.port2.onmessage = (event) => {
                                globalThis.__workerDuplicateOptionsPortResult = `${errorName}|${String(event.data)}`;
                                globalThis.__workerDuplicateOptionsPortDone = true;
                                worker.terminate();
                                probe.port1.close();
                                probe.port2.close();
                            };
                            probe.port1.postMessage("still-live");
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerDuplicateOptionsPortDone === true)",
                        "worker duplicate MessagePort options.transfer rejection should preserve the port",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__workerDuplicateOptionsPortResult")
                })
                .await
                .expect("worker duplicate MessagePort options.transfer test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_detached_arraybuffer_transfer_in_page_vm() {
    let _ = run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            const worker = new Worker("data:text/javascript,onmessage = function () {}");
                            try {
                                const buffer = new ArrayBuffer(4);
                                worker.postMessage(buffer, [buffer]);
                                let errorName = "unexpected";
                                try {
                                    worker.postMessage(buffer, [buffer]);
                                } catch (error) {
                                    errorName = error.name;
                                }
                                return `${buffer.byteLength}|${errorName}`;
                            } finally {
                                worker.terminate();
                            }
                        })()
                        "#,
                    )
                })
                .await
                .expect("worker detached ArrayBuffer rejection test should run on owner lane");

            assert_eq!(result, "0|DataCloneError");
            anyhow::Ok(())
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_rejects_nested_messageport_without_transfer_list_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portNestedPortDone = false;
                            globalThis.__portNestedPortResult = null;
                            const channel = new MessageChannel();
                            const probe = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                channel.port1.postMessage({ port: probe.port1 });
                            } catch (error) {
                                errorName = error.name;
                            }
                            probe.port2.onmessage = (event) => {
                                globalThis.__portNestedPortResult = `${errorName}|${String(event.data)}`;
                                globalThis.__portNestedPortDone = true;
                                channel.port1.close();
                                channel.port2.close();
                                probe.port1.close();
                                probe.port2.close();
                            };
                            probe.port1.postMessage("still-live");
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portNestedPortDone === true)",
                        "MessagePort nested MessagePort rejection should preserve the port",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__portNestedPortResult")
                })
                .await
                .expect("MessagePort nested MessagePort rejection test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_rejects_source_port_transfer_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portSelfTransferDone = false;
                            globalThis.__portSelfTransferResult = null;
                            const channel = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                channel.port1.postMessage("ports", [channel.port1]);
                            } catch (error) {
                                errorName = error.name;
                            }
                            channel.port2.onmessage = (event) => {
                                globalThis.__portSelfTransferResult = `${errorName}|${String(event.data)}`;
                                globalThis.__portSelfTransferDone = true;
                                channel.port1.close();
                                channel.port2.close();
                            };
                            channel.port1.postMessage("still-live");
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portSelfTransferDone === true)",
                        "MessagePort source-port transfer rejection should preserve the port",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__portSelfTransferResult")
                })
                .await
                .expect("MessagePort source-port transfer test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_rejects_detached_arraybuffer_transfer_in_page_vm() {
    let _ = run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            const channel = new MessageChannel();
                            try {
                                const buffer = new ArrayBuffer(4);
                                channel.port1.postMessage(buffer, [buffer]);
                                let errorName = "unexpected";
                                try {
                                    channel.port1.postMessage(buffer, [buffer]);
                                } catch (error) {
                                    errorName = error.name;
                                }
                                return `${buffer.byteLength}|${errorName}`;
                            } finally {
                                channel.port1.close();
                                channel.port2.close();
                            }
                        })()
                        "#,
                )
            })
            .await
            .expect("MessagePort detached ArrayBuffer rejection test should run on owner lane");

        assert_eq!(result, "0|DataCloneError");
        anyhow::Ok(())
    })
    .await;
}

#[tokio::test]
async fn messageport_messageevent_ports_array_is_frozen_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portFrozenPortsDone = false;
                            globalThis.__portFrozenPortsResult = null;
                            const channel = new MessageChannel();
                            const transferred = new MessageChannel();
                            channel.port2.onmessage = (event) => {
                                let pushName = "no-throw";
                                try {
                                    event.ports.push("extra");
                                } catch (error) {
                                    pushName = error.name;
                                }
                                globalThis.__portFrozenPortsResult =
                                    `${Object.isFrozen(event.ports)}|${pushName}|${event.ports.length}`;
                                globalThis.__portFrozenPortsDone = true;
                                event.ports[0].close();
                                channel.port1.close();
                                channel.port2.close();
                            };
                            channel.port1.postMessage("payload", [transferred.port1]);
                            transferred.port2.close();
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portFrozenPortsDone === true)",
                        "MessageEvent.ports should be frozen during MessagePort delivery",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__portFrozenPortsResult")
                })
                .await
                .expect("MessagePort frozen ports test should run on owner lane");

            assert_eq!(result, "true|TypeError|1");
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_raw_iterable_transfer_moves_buffer_and_port() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portRawIterableResult = [];
                            globalThis.__portRawIterableDone = false;
                            let remaining = 2;
                            function finish(value) {
                                globalThis.__portRawIterableResult.push(value);
                                remaining -= 1;
                                if (remaining === 0) {
                                    globalThis.__portRawIterableDone = true;
                                }
                            }

                            const bufferChannel = new MessageChannel();
                            const transferred = new Uint8Array([31, 32]).buffer;
                            const bufferIterable = {
                                [Symbol.iterator]: function* () {
                                    yield transferred;
                                }
                            };
                            bufferChannel.port2.onmessage = (event) => {
                                finish(`buffer:${Array.from(new Uint8Array(event.data.buffer)).join(',')}:${event.ports.length}`);
                                bufferChannel.port1.close();
                                bufferChannel.port2.close();
                            };
                            bufferChannel.port1.postMessage({ buffer: transferred }, bufferIterable);
                            const detached = transferred.byteLength;

                            const control = new MessageChannel();
                            const inner = new MessageChannel();
                            const portIterable = {
                                [Symbol.iterator]: function* () {
                                    yield inner.port2;
                                }
                            };
                            control.port2.onmessage = (event) => {
                                const port = event.data.port;
                                const sameWrapper = port === event.ports[0];
                                port.onmessage = (innerEvent) => {
                                    port.postMessage(`raw:${innerEvent.data}`);
                                };
                                inner.port1.onmessage = (innerEvent) => {
                                    finish(`port:${sameWrapper}:${event.ports.length}:${innerEvent.data}:${detached}`);
                                    control.port1.close();
                                    control.port2.close();
                                    inner.port1.close();
                                    inner.port2.close();
                                };
                                inner.port1.postMessage('ping');
                            };
                            control.port1.postMessage({ port: inner.port2 }, portIterable);
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portRawIterableDone === true)",
                        "MessagePort raw iterable transfer should complete",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__portRawIterableResult.sort().join('|')")
                })
                .await
                .expect("MessagePort raw iterable transfer test should run on owner lane");

            assert_eq!(result, "buffer:31,32:0|port:true:1:raw:ping:0");
        })
        .await;
}
