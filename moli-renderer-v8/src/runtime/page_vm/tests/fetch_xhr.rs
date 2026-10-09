use super::*;
use moli_browser_profile::DEFAULT_SEC_CH_UA_PLATFORM;

fn synchronous_xhr_failure_probe_expression(url: &str) -> String {
    synchronous_xhr_failure_probe_expression_with_credentials(url, false)
}

fn synchronous_xhr_failure_probe_expression_with_credentials(
    url: &str,
    with_credentials: bool,
) -> String {
    let url_literal = serde_json::to_string(url).expect("serialize synchronous XHR URL");
    format!(
        r#"
        (() => {{
          const events = [];
          const xhr = new XMLHttpRequest();
          xhr.onreadystatechange = () => events.push(`readystatechange:${{xhr.readyState}}`);
          for (const type of ["loadstart", "progress", "error", "timeout", "load", "loadend"]) {{
            xhr.addEventListener(type, () => events.push(type));
            xhr.upload.addEventListener(type, () => events.push(`upload.${{type}}`));
          }}
          xhr.open("GET", {url_literal}, false);
          xhr.withCredentials = {with_credentials};
          let error = null;
          try {{
            xhr.send("ignored GET body");
          }} catch (caught) {{
            error = {{
              name: caught && caught.name,
              message: caught && caught.message,
              isDomException: caught instanceof DOMException,
            }};
          }}
          return JSON.stringify({{
            error,
            events,
            readyState: xhr.readyState,
            status: xhr.status,
            statusText: xhr.statusText,
            responseText: xhr.responseText,
            responseURL: xhr.responseURL,
            contentType: xhr.getResponseHeader("Content-Type"),
            allHeaders: xhr.getAllResponseHeaders(),
          }});
        }})()
        "#
    )
}

fn assert_synchronous_xhr_network_error_surface(observed: &str, url: &str) {
    let observed: serde_json::Value =
        serde_json::from_str(observed).expect("synchronous XHR probe should return JSON");
    assert_eq!(
        observed,
        serde_json::json!({
            "error": {
                "name": "NetworkError",
                "message": format!(
                    "Failed to execute 'send' on 'XMLHttpRequest': Failed to load '{url}'."
                ),
                "isDomException": true,
            },
            "events": ["readystatechange:1"],
            "readyState": 4,
            "status": 0,
            "statusText": "",
            "responseText": "",
            "responseURL": "",
            "contentType": null,
            "allHeaders": "",
        })
    );
}

fn synchronous_xhr_success_probe_expression(url: &str, with_credentials: bool) -> String {
    let url_literal = serde_json::to_string(url).expect("serialize synchronous XHR URL");
    format!(
        r#"
        (() => {{
          const xhr = new XMLHttpRequest();
          xhr.open("GET", {url_literal}, false);
          xhr.withCredentials = {with_credentials};
          xhr.send();
          return JSON.stringify({{
            readyState: xhr.readyState,
            status: xhr.status,
            responseText: xhr.responseText,
            responseURL: xhr.responseURL,
          }});
        }})()
        "#
    )
}

async fn evaluate_synchronous_xhr_probe(
    document_url: Url,
    expression: String,
) -> (String, ScriptNetworkOutput) {
    let mut page_vm = test_page_vm_with_document_url(document_url);
    let local_executor = page_vm.local_executor.clone();
    local_executor
        .run(async move {
            let observed = page_vm.vm_mut().eval(&expression)?;
            Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
        })
        .await
        .expect("synchronous XHR probe should run on owner lane")
}

fn assert_synchronous_xhr_success_surface(observed: &str, url: &str, body: &str) {
    assert_eq!(
        observed,
        format!(
            r#"{{"readyState":4,"status":200,"responseText":{},"responseURL":{}}}"#,
            serde_json::to_string(body).expect("serialize XHR response body"),
            serde_json::to_string(url).expect("serialize XHR response URL"),
        )
    );
}

fn assert_single_synchronous_xhr_network_failure(
    network_output: ScriptNetworkOutput,
    expected_error: &str,
) {
    let (records, _, _) = split_network_output_items(network_output);
    assert_eq!(records.len(), 1);
    assert!(matches!(
        records[0].outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text.contains(expected_error)
    ));
}

fn assert_single_synchronous_xhr_network_success(network_output: ScriptNetworkOutput) {
    let (records, _, _) = split_network_output_items(network_output);
    assert_eq!(records.len(), 1);
    assert!(matches!(
        records[0].outcome(),
        SubresourceNetworkOutcome::Success { status: 200, .. }
    ));
}

fn read_blocking_http_request_head(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;

    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        stream
            .read_exact(&mut byte)
            .expect("read blocking HTTP request");
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(request).expect("blocking HTTP request should be UTF-8")
}

fn spawn_blocking_connection_drop_http_server(
    path: &'static str,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind blocking connection-drop HTTP server");
    let addr = listener
        .local_addr()
        .expect("blocking connection-drop server address");
    let server = std::thread::Builder::new()
        .name("sync-xhr-connection-drop-server".to_owned())
        .spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("accept blocking connection-drop request");
            let request = read_blocking_http_request_head(&mut stream);
            assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
            drop(stream);
        })
        .expect("spawn blocking connection-drop HTTP server");
    (format!("http://{addr}"), server)
}

fn spawn_blocking_single_redirect_http_server(
    path: &'static str,
    location: &'static str,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::Write;

    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind blocking redirect HTTP server");
    let addr = listener
        .local_addr()
        .expect("blocking redirect server address");
    let server = std::thread::Builder::new()
        .name("sync-xhr-single-redirect-server".to_owned())
        .spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept blocking redirect request");
            let request = read_blocking_http_request_head(&mut stream);
            assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .expect("write blocking redirect response");
        })
        .expect("spawn blocking redirect HTTP server");
    (format!("http://{addr}"), server)
}

fn spawn_blocking_redirect_loop_http_server(
    path: &'static str,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::Write;

    const REDIRECT_LOOP_REQUESTS: usize = 11;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind blocking redirect-loop HTTP server");
    let addr = listener
        .local_addr()
        .expect("blocking redirect-loop server address");
    let server = std::thread::Builder::new()
        .name("sync-xhr-redirect-loop-server".to_owned())
        .spawn(move || {
            for _ in 0..REDIRECT_LOOP_REQUESTS {
                let (mut stream, _) = listener
                    .accept()
                    .expect("accept blocking redirect-loop request");
                let request = read_blocking_http_request_head(&mut stream);
                assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
                let response = format!(
                    "HTTP/1.1 301 Moved Permanently\r\nLocation: {path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write blocking redirect-loop response");
            }
        })
        .expect("spawn blocking redirect-loop HTTP server");
    (format!("http://{addr}"), server)
}

fn spawn_blocking_xhr_response_server(
    path: &'static str,
    body: &'static str,
    response_headers: Vec<(&'static str, &'static str)>,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::Write;

    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind blocking XHR response server");
    let addr = listener
        .local_addr()
        .expect("blocking XHR response server address");
    let server = std::thread::Builder::new()
        .name("sync-xhr-response-server".to_owned())
        .spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("accept blocking XHR response request");
            let request = read_blocking_http_request_head(&mut stream);
            assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
            let response_headers = response_headers
                .into_iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\n{response_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .expect("write blocking XHR response");
        })
        .expect("spawn blocking XHR response server");
    (format!("http://{addr}"), server)
}

async fn spawn_credentialless_partition_fetch_cache_server()
-> (String, tokio::sync::oneshot::Sender<()>, JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind credentialless fetch partition server");
    let addr = listener
        .local_addr()
        .expect("credentialless fetch partition server addr");
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut accepted = 0;
        for body in ["credentialless", "normal"] {
            let (mut stream, _) = tokio::select! {
                accepted = listener.accept() => {
                    accepted.expect("accept credentialless fetch partition request")
                }
                _ = &mut shutdown_rx => {
                    return accepted;
                }
            };
            accepted += 1;
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read credentialless fetch partition request");
            assert!(
                request.starts_with("GET /data HTTP/1.1"),
                "unexpected credentialless fetch partition request:\n{request}"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nAccess-Control-Allow-Origin: null\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write credentialless fetch partition response");
        }
        accepted
    });
    (format!("http://{addr}"), shutdown_tx, server)
}

async fn spawn_credentialless_partition_child_navigation_server()
-> (String, tokio::sync::oneshot::Sender<()>, JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind credentialless child navigation partition server");
    let addr = listener
        .local_addr()
        .expect("credentialless child navigation partition server addr");
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut accepted = 0;
        for label in ["credentialless", "normal"] {
            let (mut stream, _) = tokio::select! {
                accepted = listener.accept() => {
                    accepted.expect("accept credentialless child navigation partition request")
                }
                _ = &mut shutdown_rx => {
                    return accepted;
                }
            };
            accepted += 1;
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read credentialless child navigation partition request");
            assert!(
                request.starts_with("GET /child.html "),
                "unexpected credentialless child navigation partition request:\n{request}"
            );
            let body = format!(
                r#"<!doctype html><script>parent.postMessage({{type:"child-nav-partition", value:"{label}"}}, "*");</script>"#
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write credentialless child navigation partition response");
        }
        accepted
    });
    (format!("http://{addr}"), shutdown_tx, server)
}

async fn spawn_document_then_api_capture_server(
    document_path: &'static str,
    document_body: String,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind document-then-api capture server");
    let addr = listener
        .local_addr()
        .expect("document-then-api capture server addr");
    let (api_request_tx, api_request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept document request");
        let document_request = read_http_request_head(&mut stream)
            .await
            .expect("read document request");
        assert!(
            document_request.starts_with(&format!("GET {document_path} HTTP/1.1\r\n")),
            "unexpected document request:\n{document_request}"
        );
        let document_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            document_body.len(),
            document_body
        );
        stream
            .write_all(document_response.as_bytes())
            .await
            .expect("write document response");

        let (mut stream, _) = listener.accept().await.expect("accept api request");
        let api_request = read_http_request_head(&mut stream)
            .await
            .expect("read api request");
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
            .expect("write api response");
    });
    (format!("http://{addr}"), api_request_rx, server)
}

async fn spawn_popup_document_with_response_csp_server(
    document_path: &'static str,
    response_csp: &'static str,
    document_body: String,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind popup response CSP server");
    let addr = listener
        .local_addr()
        .expect("popup response CSP server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept popup response CSP document request");
        let document_request = read_http_request_head(&mut stream)
            .await
            .expect("read popup response CSP document request");
        assert!(
            document_request.starts_with(&format!("GET {document_path} HTTP/1.1\r\n")),
            "unexpected popup response CSP document request:\n{document_request}"
        );
        let document_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: {response_csp}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            document_body.len(),
            document_body
        );
        stream
            .write_all(document_response.as_bytes())
            .await
            .expect("write popup response CSP document response");
    });
    (format!("http://{addr}"), server)
}

fn spawn_repeated_synchronous_xhr_server(
    status: u16,
) -> (
    String,
    std::sync::mpsc::Sender<()>,
    std::thread::JoinHandle<usize>,
) {
    use std::io::Write;
    use std::sync::mpsc::{RecvTimeoutError, TryRecvError};

    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind repeated synchronous XHR server");
    let addr = listener.local_addr().expect("synchronous XHR server addr");
    listener
        .set_nonblocking(true)
        .expect("allow synchronous XHR server shutdown");
    let (shutdown, stop) = std::sync::mpsc::channel();
    let server = std::thread::Builder::new()
        .name("sync-xhr-repetition-server".to_owned())
        .spawn(move || {
            let mut received = 0;
            while matches!(stop.try_recv(), Err(TryRecvError::Empty)) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if !matches!(
                            stop.recv_timeout(Duration::from_millis(1)),
                            Err(RecvTimeoutError::Timeout)
                        ) {
                            break;
                        }
                        continue;
                    }
                    Err(error) => panic!("accept repeated synchronous XHR: {error}"),
                };
                // macOS can inherit O_NONBLOCK from the listening socket.
                // This fixture reads the complete request synchronously; the
                // socket timeout alone does not turn WouldBlock into a wait.
                stream
                    .set_nonblocking(false)
                    .expect("make accepted synchronous XHR stream blocking");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("bound synchronous XHR request read");
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("bound synchronous XHR response write");
                let request = read_blocking_http_request_head(&mut stream);
                assert!(request.starts_with("GET /repeated HTTP/1.1\r\n"));
                let reason = if status == 200 { "OK" } else { "Bad Request" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: 5\r\nConnection: close\r\n\r\nreply"
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write repeated synchronous XHR response");
                received += 1;
            }
            received
        })
        .expect("spawn repeated synchronous XHR server");
    (format!("http://{addr}"), shutdown, server)
}

fn check_blob_fetch_and_xhr_methods_in_window_and_worker(
    interception: bool,
) -> impl std::future::Future<Output = ()> {
    Box::pin(run_page_vm_async_test(async move {
        for worker in [false, true] {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();
            let probe = r#"(async () => {
                const check = (value, message) => { if (!value) throw new Error(message); };
                const blob = URL.createObjectURL(new Blob(['payload'], {type: 'text/plain'}));
                try {
                    for (const api of ['fetch', 'fetch-clone', 'xhr-async', 'xhr-sync']) {
                        for (const scheme of ['blob', 'data']) {
                            const raw = scheme === 'blob' ? blob : 'data:text/plain,payload';
                            const methods = scheme === 'blob'
                                ? ['GET', 'gEt', 'HEAD', 'POST', 'PUT', 'DELETE', 'OPTIONS', 'PATCH', 'CUSTOM']
                                : ['GET', 'POST', 'CUSTOM'];
                            for (const fragment of ['', '#fragment']) {
                                for (const method of methods) {
                                    const url = raw + fragment;
                                    const success = scheme === 'data' || method.toUpperCase() === 'GET';
                                    const label = [api, scheme, fragment, method].join(':');
                                    if (api.startsWith('fetch')) {
                                        let response;
                                        try {
                                            response = await (api === 'fetch-clone'
                                                ? fetch(new Request(url, {method}).clone())
                                                : fetch(url, {method}));
                                        } catch (error) {
                                            check(!success && error instanceof TypeError, label + ': wrong rejection ' + error);
                                            continue;
                                        }
                                        check(success, label + ': unexpectedly fulfilled');
                                        check(response.status === 200 && response.url.split('#')[0] === raw, label + ': response metadata');
                                        check(await response.text() === 'payload', label + ': response body');
                                        continue;
                                    }
                                    const async = api === 'xhr-async';
                                    const xhr = new XMLHttpRequest();
                                    xhr.open(method, url, async);
                                    const checkResponse = () => {
                                        check(xhr.readyState === 4 && xhr.status === (success ? 200 : 0), label + ': state/status');
                                        check(xhr.responseText === (success ? 'payload' : ''), label + ': response text');
                                        check(xhr.responseURL.split('#')[0] === (success ? raw : ''), label + ': response URL');
                                        check(xhr.getResponseHeader('Content-Type') === (success ? 'text/plain' : null), label + ': headers');
                                    };
                                    if (!async) {
                                        let failure;
                                        try { xhr.send(); } catch (error) { failure = error; }
                                        check(success ? failure === undefined : failure instanceof DOMException && failure.name === 'NetworkError', label + ': sync exception');
                                        checkResponse();
                                        continue;
                                    }
                                    await new Promise((resolve, reject) => {
                                        let returned = false;
                                        const events = [];
                                        xhr.onload = () => events.push('load');
                                        xhr.onerror = () => events.push('error');
                                        xhr.onloadend = () => {
                                            events.push('loadend');
                                            try {
                                                check(returned, label + ': completion during send');
                                                check(events.join(',') === (success ? 'load,loadend' : 'error,loadend'), label + ': terminal events');
                                                checkResponse();
                                                resolve();
                                            } catch (error) { reject(error); }
                                        };
                                        xhr.send();
                                        returned = true;
                                    });
                                }
                            }
                        }
                    }
                } finally { URL.revokeObjectURL(blob); }
            })()"#;
            let script = if worker {
                let source = serde_json::to_string(&format!(
                    "{probe}.then(() => postMessage('ok'), error => postMessage(String(error)))"
                ))
                .expect("worker source");
                format!(
                    r#"globalThis.__blobMethodResult = 'pending';
                    const source = URL.createObjectURL(new Blob([{source}]));
                    const worker = new Worker(source);
                    worker.onmessage = event => {{
                        globalThis.__blobMethodResult = event.data;
                        worker.terminate();
                        URL.revokeObjectURL(source);
                    }};
                    worker.onerror = event => {{ globalThis.__blobMethodResult = event.message; }};"#
                )
            } else {
                format!(
                    "globalThis.__blobMethodResult = 'pending'; {probe}.then(() => {{ globalThis.__blobMethodResult = 'ok'; }}, error => {{ globalThis.__blobMethodResult = String(error); }})"
                )
            };
            let (result, pending_count, network_output) = local_executor
                .run(async move {
                    page_vm
                        .vm_mut()
                        .set_fetch_subresource_interception(interception, None);
                    page_vm.vm_mut().eval(&script)?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__blobMethodResult !== 'pending')",
                        "blob method checks should complete",
                    )
                    .await?;
                    let result = page_vm.vm_mut().eval("globalThis.__blobMethodResult")?;
                    let pending_count = page_vm
                        .vm_mut()
                        .take_pending_subresource_fetch_infos()
                        .len();
                    Ok::<_, anyhow::Error>((
                        result,
                        pending_count,
                        page_vm.vm_mut().take_network_output(),
                    ))
                })
                .await
                .expect("blob method probe should run on owner lane");
            assert_eq!(result, "ok", "worker={worker}");
            assert_eq!(
                pending_count, 0,
                "local fetch and XHR must bypass interception; worker={worker}"
            );
            let (records, _, _) = split_network_output_items(network_output);
            let failures = records
                .iter()
                .filter(|record| {
                    matches!(record.outcome(), SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("blob URL fetch requires GET"))
                })
                .count();
            assert_eq!(
                failures, 56,
                "each rejected request must record a local network error; worker={worker}"
            );
        }
    }))
}

mod event_source;
mod fetch;
mod workers;
mod xhr;
