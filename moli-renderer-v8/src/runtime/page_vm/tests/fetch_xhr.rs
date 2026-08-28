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

#[tokio::test]
async fn event_source_streams_sse_and_records_incremental_network_output() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind EventSource test server");
        let addr = listener.local_addr().expect("EventSource server address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept EventSource request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read EventSource request");
            assert!(request.starts_with("GET /events HTTP/1.1"));
            let request_lower = request.to_ascii_lowercase();
            assert!(request_lower.contains("accept: text/event-stream"));
            assert!(request_lower.contains("cache-control: no-cache"));

            let body = concat!(
                "id: 7\nevent: update\ndata: first\ndata: second\n\n",
                "id: 8\nevent: update\ndata: must-not-dispatch\n\n",
            );
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(head.as_bytes())
                .await
                .expect("write EventSource response head");
            stream
                .write_all(&body.as_bytes()[..17])
                .await
                .expect("write first EventSource chunk");
            tokio::time::sleep(Duration::from_millis(10)).await;
            stream
                .write_all(&body.as_bytes()[17..])
                .await
                .expect("write second EventSource chunk");
        });

        let document_url =
            Url::parse(&format!("http://{addr}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let (result, output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__eventSourceDone = false;
                    globalThis.__eventSourceEvents = [];
                    globalThis.setTimeout = () => {
                        throw new Error("EventSource must not call Window.setTimeout");
                    };
                    globalThis.clearTimeout = () => {
                        throw new Error("EventSource must not call Window.clearTimeout");
                    };
                    const source = new EventSource("/events");
                    globalThis.__eventSourceInitial = [
                        source.url,
                        source.withCredentials,
                        source.readyState,
                        EventSource.CONNECTING,
                        EventSource.OPEN,
                        EventSource.CLOSED,
                    ];
                    source.onopen = () => {
                        globalThis.__eventSourceEvents.push(`open:${source.readyState}`);
                    };
                    source.addEventListener("update", (event) => {
                        globalThis.__eventSourceEvents.push(
                            `${event.type}:${event.lastEventId}:${event.data}:${event.isTrusted}`
                        );
                        source.close();
                        globalThis.__eventSourceEvents.push(`closed:${source.readyState}`);
                        globalThis.__eventSourceDone = true;
                    });
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__eventSourceDone === true)",
                    "EventSource should receive the streamed SSE message",
                )
                .await?;
                drain_page_work_until_no_pending_subresources(
                    &mut page_vm,
                    "completed EventSource response should publish its real network terminal",
                )
                .await?;
                let result = page_vm.vm_mut().eval(
                    "JSON.stringify([globalThis.__eventSourceInitial, globalThis.__eventSourceEvents])",
                )?;
                Ok::<_, anyhow::Error>((result, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("EventSource test should run on owner lane");

        server.await.expect("EventSource server should finish");
        assert_eq!(
            result,
            format!(
                r#"[["http://{addr}/events",false,0,0,1,2],["open:1","update:7:first\nsecond:true","closed:2"]]"#
            )
        );

        let items = output.into_items().collect::<Vec<_>>();
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                if request.resource_type() == SubresourceResourceType::EventSource
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceResponseStarted(response)
                if response.status() == 200
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceDataReceived(data)
                if data.data_length() > 0
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(message)
                if message.event_name() == "update"
                    && message.event_id() == "7"
                    && message.data() == "first\nsecond"
        )));
        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(
                    item,
                    ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
                ))
                .count(),
            1,
            "closing in the first message handler must stop later messages from the same chunk",
        );
        let message_index = items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(message)
                        if message.event_name() == "update" && message.event_id() == "7"
                )
            })
            .expect("EventSource message must be observable");
        let body_terminals = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => Some((index, body)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            body_terminals.len(),
            1,
            "a finite EventSource response must have one network terminal",
        );
        let (terminal_index, terminal) = body_terminals[0];
        assert!(
            message_index < terminal_index,
            "the final SSE message must be observed before the body terminal",
        );
        assert!(matches!(
            terminal.result(),
            SubresourceBodyFinishedResult::Ready(_)
        ));
    })
    .await;
}

#[tokio::test]
async fn event_source_close_from_message_handler_cancels_live_stream() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind live EventSource test server");
        let addr = listener
            .local_addr()
            .expect("live EventSource server address");
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept live EventSource request");
            let _ = read_http_request_head(&mut stream)
                .await
                .expect("read live EventSource request");
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Cache-Control: no-store\r\n",
                        "\r\n",
                        "data: live\n\n",
                    )
                    .as_bytes(),
                )
                .await
                .expect("write live EventSource response");
            stream.flush().await.expect("flush live EventSource event");
            let _ = release_rx.await;
        });

        let document_url = Url::parse(&format!("http://{addr}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let (result, output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__eventSourceDone = false;
                    globalThis.__eventSourceEvents = [];
                    const source = new EventSource("/events");
                    source.onmessage = event => {
                        globalThis.__eventSourceEvents.push(event.data);
                        source.close();
                        globalThis.__eventSourceEvents.push(`closed:${source.readyState}`);
                        globalThis.__eventSourceDone = true;
                    };
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__eventSourceDone === true)",
                    "live EventSource should receive its message",
                )
                .await?;
                let result = page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__eventSourceEvents)")?;
                Ok::<_, anyhow::Error>((result, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("live EventSource test should run on owner lane");

        let _ = release_tx.send(());
        server.await.expect("live EventSource server should finish");
        assert_eq!(result, r#"["live","closed:2"]"#);

        let items = output.into_items().collect::<Vec<_>>();
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(message)
                if message.data() == "live"
        )));
        let terminals = items
            .iter()
            .filter_map(|item| match item {
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => Some(body),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert!(matches!(
            terminals[0].result(),
            SubresourceBodyFinishedResult::FailedWithPartialBody { error_text, .. }
                if error_text == crate::network_host::ABORTED_ERROR_TEXT
        ));
    })
    .await;
}

#[tokio::test]
async fn event_source_close_from_open_handler_preserves_completed_response_terminal() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind EventSource open-close test server");
        let addr = listener.local_addr().expect("EventSource server address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept EventSource request");
            let _ = read_http_request_head(&mut stream)
                .await
                .expect("read EventSource request");
            // A zero-length declared body is complete when the response head is
            // accepted. This makes close() from onopen exercise the committed
            // transport boundary instead of racing the client reading a body
            // that the server has merely written to its socket.
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/event-stream\r\n",
                "Content-Length: 0\r\n",
                "Connection: close\r\n",
                "\r\n",
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write EventSource response");
        });

        let document_url = Url::parse(&format!("http://{addr}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let (result, output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__eventSourceDone = false;
                    globalThis.__eventSourceEvents = [];
                    const source = new EventSource("/events");
                    source.onopen = () => {
                        globalThis.__eventSourceEvents.push(`open:${source.readyState}`);
                        source.close();
                        globalThis.__eventSourceEvents.push(`closed:${source.readyState}`);
                        globalThis.__eventSourceDone = true;
                    };
                    source.onmessage = () => {
                        globalThis.__eventSourceEvents.push("unexpected-message");
                    };
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__eventSourceDone === true)",
                    "EventSource open handler should run",
                )
                .await?;
                drain_page_work_until_no_pending_subresources(
                    &mut page_vm,
                    "completed EventSource response should retain its network terminal",
                )
                .await?;
                let result = page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__eventSourceEvents)")?;
                Ok::<_, anyhow::Error>((result, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("EventSource open-close test should run on owner lane");

        server.await.expect("EventSource server should finish");
        assert_eq!(result, r#"["open:1","closed:2"]"#);

        let items = output.into_items().collect::<Vec<_>>();
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceResponseStarted(response)
                if response.status() == 200
        )));
        assert!(!items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
        )));
        let terminals = items
            .iter()
            .filter_map(|item| match item {
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => Some(body),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert!(matches!(
            terminals[0].result(),
            SubresourceBodyFinishedResult::Ready(_)
        ));
    })
    .await;
}

#[tokio::test]
async fn event_source_close_from_open_handler_cancels_live_stream() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind live EventSource open-close test server");
        let addr = listener
            .local_addr()
            .expect("live EventSource server address");
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept live EventSource request");
            let _ = read_http_request_head(&mut stream)
                .await
                .expect("read live EventSource request");
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Cache-Control: no-store\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                    )
                    .as_bytes(),
                )
                .await
                .expect("write live EventSource response head");
            stream
                .flush()
                .await
                .expect("flush live EventSource response head");
            let _ = release_rx.await;
        });

        let document_url = Url::parse(&format!("http://{addr}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let (result, output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__eventSourceDone = false;
                    globalThis.__eventSourceEvents = [];
                    const source = new EventSource("/events");
                    source.onopen = () => {
                        globalThis.__eventSourceEvents.push(`open:${source.readyState}`);
                        source.close();
                        globalThis.__eventSourceEvents.push(`closed:${source.readyState}`);
                        globalThis.__eventSourceDone = true;
                    };
                    source.onmessage = () => {
                        globalThis.__eventSourceEvents.push("unexpected-message");
                    };
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__eventSourceDone === true)",
                    "live EventSource open handler should run",
                )
                .await?;
                let result = page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__eventSourceEvents)")?;
                Ok::<_, anyhow::Error>((result, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("live EventSource open-close test should run on owner lane");

        let _ = release_tx.send(());
        server.await.expect("live EventSource server should finish");
        assert_eq!(result, r#"["open:1","closed:2"]"#);

        let items = output.into_items().collect::<Vec<_>>();
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceResponseStarted(response)
                if response.status() == 200
        )));
        assert!(!items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
        )));
        let terminals = items
            .iter()
            .filter_map(|item| match item {
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => Some(body),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert!(matches!(
            terminals[0].result(),
            SubresourceBodyFinishedResult::FailedWithPartialBody { error_text, .. }
                if error_text == crate::network_host::ABORTED_ERROR_TEXT
        ));
    })
    .await;
}

#[tokio::test]
async fn event_source_close_from_open_handler_preserves_materialized_response_terminal() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://example.test/page.html").expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let (result, output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__eventSourceDone = false;
                    globalThis.__eventSourceEvents = [];
                    const source = new EventSource(
                        "data:text/event-stream,data%3A%20must-not-dispatch%0A%0A"
                    );
                    source.onopen = () => {
                        globalThis.__eventSourceEvents.push(`open:${source.readyState}`);
                        source.close();
                        globalThis.__eventSourceEvents.push(`closed:${source.readyState}`);
                        globalThis.__eventSourceDone = true;
                    };
                    source.onmessage = () => {
                        globalThis.__eventSourceEvents.push("unexpected-message");
                    };
                    source.onerror = () => {
                        globalThis.__eventSourceEvents.push(`error:${source.readyState}`);
                        globalThis.__eventSourceDone = true;
                    };
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__eventSourceDone === true)",
                    "materialized EventSource open handler should run",
                )
                .await?;
                drain_page_work_until_no_pending_subresources(
                    &mut page_vm,
                    "materialized EventSource should publish its real network terminal",
                )
                .await?;
                let result = page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__eventSourceEvents)")?;
                Ok::<_, anyhow::Error>((result, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("materialized EventSource test should run on owner lane");

        assert_eq!(result, r#"["open:1","closed:2"]"#);
        let items = output.into_items().collect::<Vec<_>>();
        assert!(!items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
        )));
        let terminals = items
            .iter()
            .filter_map(|item| match item {
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => Some(body),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert!(matches!(
            terminals[0].result(),
            SubresourceBodyFinishedResult::Ready(_)
        ));
    })
    .await;
}

#[tokio::test]
async fn event_source_reconnects_to_final_redirect_url_with_last_event_id() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind EventSource reconnect test server");
        let addr = listener
            .local_addr()
            .expect("EventSource reconnect server address");
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut event_stream_visits = 0;
            loop {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .expect("accept EventSource reconnect request");
                let request = read_http_request_head(&mut stream)
                    .await
                    .expect("read EventSource reconnect request");
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_ascii_whitespace().nth(1))
                    .expect("EventSource reconnect request path")
                    .to_owned();
                requests.push(path.clone());

                if path == "/redirect" {
                    stream
                        .write_all(
                            b"HTTP/1.1 302 Found\r\nLocation: /events\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .expect("write EventSource redirect response");
                    continue;
                }

                assert_eq!(path, "/events");
                event_stream_visits += 1;
                let body = if event_stream_visits == 1 {
                    "id: 41\nretry: 0\n\n"
                } else {
                    assert!(
                        request
                            .to_ascii_lowercase()
                            .contains("last-event-id: 41"),
                        "reconnected EventSource request must carry Last-Event-ID: {request}",
                    );
                    "id: 42\nevent: update\ndata: reconnected\n\n"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write EventSource reconnect response");
                if event_stream_visits == 2 {
                    return requests;
                }
            }
        });

        let original_url = format!("http://{addr}/redirect");
        let document_url =
            Url::parse(&format!("http://{addr}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__eventSourceDone = false;
                    globalThis.__eventSourceEvents = [];
                    const source = new EventSource("/redirect");
                    globalThis.__eventSourcePublicUrl = source.url;
                    source.onopen = () => {
                        globalThis.__eventSourceEvents.push(`open:${source.readyState}`);
                    };
                    source.onerror = () => {
                        globalThis.__eventSourceEvents.push(`error:${source.readyState}`);
                    };
                    source.addEventListener("update", (event) => {
                        globalThis.__eventSourceEvents.push(
                            `${event.type}:${event.lastEventId}:${event.data}`
                        );
                        source.close();
                        globalThis.__eventSourceDone = true;
                    });
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__eventSourceDone === true)",
                    "EventSource should reconnect after the first response ends",
                )
                .await?;
                page_vm.vm_mut().eval(
                    "JSON.stringify([globalThis.__eventSourcePublicUrl, globalThis.__eventSourceEvents])",
                )
            })
            .await
            .expect("EventSource reconnect test should run on owner lane");

        let requests = server
            .await
            .expect("EventSource reconnect server should finish");
        assert_eq!(requests, ["/redirect", "/events", "/events"]);
        assert_eq!(
            result,
            format!(
                r#"["{original_url}",["open:1","error:0","open:1","update:42:reconnected"]]"#
            ),
        );
    })
    .await;
}

#[tokio::test]
async fn worker_url_loads_script_and_flushes_messages_sent_while_loading() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 200 OK",
            r#"
            postMessage("loaded");
            onmessage = (event) => {
                postMessage(`pong:${event.data}`);
            };
            "#
            .to_owned(),
            Duration::from_millis(75),
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        globalThis.__workerDone = false;
                        const worker = new Worker("/worker.js");
                        worker.onmessage = (event) => {
                            globalThis.__workerEvents.push(event.data);
                            if (globalThis.__workerEvents.length >= 2) {
                                globalThis.__workerDone = true;
                            }
                        };
                        worker.postMessage("queued");
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker url load should finish and deliver queued postMessage",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker url load test should run on owner lane");

        server.await.expect("worker script server should finish");
        assert_eq!(events, r#"["loaded","pong:queued"]"#);
    })
    .await;
}

#[tokio::test]
async fn worker_url_busy_loop_can_be_terminated_after_message_burst() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 200 OK",
            r#"
            onmessage = function() {
                for (var i = 0; true; i++) {
                    if (i % 1000 == 0) {
                        postMessage(i);
                    }
                }
            };
            "#
            .to_owned(),
            Duration::from_millis(0),
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__busyWorkerDone = false;
                        globalThis.__busyWorkerLast = -1;
                        globalThis.__busyWorkerUnexpected = false;
                        const worker = new Worker("/worker.js");
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
                    "external busy worker terminate should complete",
                )
                .await?;
                page_vm.vm_mut().eval(
                    "JSON.stringify({last: globalThis.__busyWorkerLast, unexpected: globalThis.__busyWorkerUnexpected})",
                )
            })
            .await
            .expect("external busy worker terminate test should run on owner lane");

        server.await.expect("worker script server should finish");
        assert_eq!(result, r#"{"last":10000,"unexpected":false}"#);
    })
    .await;
}

#[tokio::test]
async fn window_fetch_emits_browser_style_subresource_headers_on_wire() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_header_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let fetch_url = format!("{base_url}/api");
        let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            fetch({fetch_url_literal}, {{
                                headers: {{ "X-Test": "fetch" }}
                            }})
                              .then((response) => response.text())
                              .then(() => {{ globalThis.__fetchDone = true; }});
                        }})()
                        "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__fetchDone === true)",
                    "fetch header capture request should complete",
                )
                .await
            })
            .await
            .expect("fetch header capture test should run on owner lane");

        let request = request_rx.await.expect("captured fetch request");
        server.await.expect("header capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("GET /api HTTP/1.1\r\n"));
        assert!(request_lower.contains("x-test: fetch\r\n"));
        assert!(request_lower.contains("referer: "));
        assert!(request_lower.contains("/page.html\r\n"));
        assert!(request_lower.contains("accept: */*\r\n"));
        assert!(request_lower.contains("accept-language: en-us,en;q=0.9\r\n"));
        assert!(request_lower.contains("sec-fetch-site: same-origin\r\n"));
        assert!(request_lower.contains("sec-fetch-mode: cors\r\n"));
        assert!(request_lower.contains("sec-fetch-dest: empty\r\n"));
        assert!(request_lower.contains("sec-ch-ua: "));
        assert!(request_lower.contains("sec-ch-ua-mobile: ?0\r\n"));
        let expected_platform_header = format!(
            "sec-ch-ua-platform: {}\r\n",
            DEFAULT_SEC_CH_UA_PLATFORM.to_ascii_lowercase()
        );
        assert!(request_lower.contains(&expected_platform_header));
    })
    .await;
}

#[tokio::test]
async fn navigator_send_beacon_posts_no_cors_ping_subresource() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_request_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (returned, request, network_output) = local_executor
            .run(async move {
                let returned = page_vm.vm_mut().eval(
                    r#"
                    (() => String(navigator.sendBeacon("/beacon", "payload")))()
                    "#,
                )?;
                let request = tokio::time::timeout(Duration::from_secs(3), request_rx)
                    .await
                    .expect("sendBeacon request should reach fixture")
                    .expect("sendBeacon fixture should capture request");
                drain_page_work_until_no_pending_subresources(
                    &mut page_vm,
                    "sendBeacon network completion should be observed",
                )
                .await?;
                Ok::<_, anyhow::Error>((returned, request, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("sendBeacon test should run on owner lane");

        server.await.expect("request capture server should finish");

        let request_lower = request.to_ascii_lowercase();
        assert_eq!(returned, "true");
        assert!(request.starts_with("POST /beacon HTTP/1.1\r\n"));
        assert!(request_lower.contains("content-type: text/plain;charset=utf-8\r\n"));
        assert!(request_lower.contains("sec-fetch-mode: no-cors\r\n"));
        assert!(request_lower.ends_with("\r\n\r\npayload"));

        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.resource_type(), SubresourceResourceType::Ping);
        assert_eq!(record.request_body(), Some("payload"));
        let SubresourceNetworkOutcome::Success { status, .. } = record.outcome() else {
            panic!(
                "expected sendBeacon network success, got {:?}",
                record.outcome()
            );
        };
        assert_eq!(*status, 204);
    })
    .await;
}

#[tokio::test]
async fn window_fetch_form_data_blob_request_body_preserves_raw_bytes() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_request_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let network_output = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__fetchDone = false;
                        const formData = new FormData();
                        const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0xff]);
                        const blob = new Blob([bytes], { type: "image/png" });
                        formData.append("test_image", blob, "image.png");
                        fetch("/upload", { method: "POST", body: formData })
                          .then((response) => response.text())
                          .then(() => {
                            globalThis.__fetchDone = true;
                          });
                    })()
                    "#,
                )?;
                let _request = tokio::time::timeout(Duration::from_secs(3), request_rx)
                    .await
                    .expect("multipart fetch request should reach fixture")
                    .expect("multipart fetch fixture should capture request");
                drain_page_work_until_no_pending_subresources(
                    &mut page_vm,
                    "multipart fetch network completion should be observed",
                )
                .await?;
                Ok::<_, anyhow::Error>(page_vm.vm_mut().take_network_output())
            })
            .await
            .expect("multipart fetch test should run on owner lane");

        server.await.expect("request capture server should finish");

        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        let body = record
            .request_body_bytes()
            .expect("multipart request body bytes should be captured");
        assert!(
            body.windows(5)
                .any(|window| window == [0x89, 0x50, 0x4e, 0x47, 0xff]),
            "multipart request body should contain raw PNG-like bytes: {body:?}"
        );
        assert!(
            std::str::from_utf8(body).is_err(),
            "raw multipart body containing image bytes must not be valid UTF-8"
        );
    })
    .await;
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

#[tokio::test]
async fn credentialless_child_fetch_uses_credentialless_network_partition_key() {
    run_page_vm_async_test(async move {
        let (base_url, shutdown_server, server) =
            spawn_credentialless_partition_fetch_cache_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let data_url_literal =
            serde_json::to_string(&format!("{base_url}/data")).expect("serialize data url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let outcome = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__credentiallessPartitionDone = false;
                        globalThis.__credentiallessPartitionResult = null;
                        const credentialless = document.createElement("iframe");
                        credentialless.credentialless = true;
                        const normal = document.createElement("iframe");
                        document.body.append(credentialless, normal);
                        Promise.resolve()
                          .then(() => credentialless.contentWindow.fetch({data_url_literal}))
                          .then((response) => response.text())
                          .then((first) => normal.contentWindow.fetch({data_url_literal})
                            .then((response) => response.text())
                            .then((second) => {{
                              globalThis.__credentiallessPartitionResult = [first, second];
                              globalThis.__credentiallessPartitionDone = true;
                            }}))
                          .catch((error) => {{
                            globalThis.__credentiallessPartitionResult = ["error", String(error)];
                            globalThis.__credentiallessPartitionDone = true;
                          }});
                    }})()
                    "#,
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__credentiallessPartitionDone === true)",
                    "credentialless child fetch partitioning should finish",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__credentiallessPartitionResult)")
            })
            .await
            .expect("credentialless child fetch partitioning test should run on owner lane");

        let _ = shutdown_server.send(());
        let request_count = server
            .await
            .expect("credentialless child fetch partition server should finish");
        assert_eq!(outcome, r#"["credentialless","normal"]"#);
        assert_eq!(
            request_count, 2,
            "credentialless and normal child fetches should use separate network/cache partitions"
        );
    })
    .await;
}

#[tokio::test]
async fn credentialless_child_navigation_uses_credentialless_network_partition_key() {
    run_page_vm_async_test(async move {
        let (base_url, shutdown_server, server) =
            spawn_credentialless_partition_child_navigation_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let outcome = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__credentiallessChildNavigationPartitionDone = false;
                        globalThis.__credentiallessChildNavigationPartitionResult = [];
                        const normal = document.createElement("iframe");
                        normal.src = "/child.html";
                        const credentialless = document.createElement("iframe");
                        credentialless.credentialless = true;
                        credentialless.src = "/child.html";
                        window.addEventListener("message", (event) => {
                            if (!event.data || event.data.type !== "child-nav-partition") {
                                return;
                            }
                            globalThis.__credentiallessChildNavigationPartitionResult.push(
                                event.data.value
                            );
                            if (
                                globalThis.__credentiallessChildNavigationPartitionResult.length === 1
                            ) {
                                document.body.appendChild(normal);
                            } else {
                                globalThis.__credentiallessChildNavigationPartitionDone = true;
                            }
                        });
                        document.body.appendChild(credentialless);
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__credentiallessChildNavigationPartitionDone === true)",
                    "credentialless child navigation partitioning should finish",
                )
                .await?;
                page_vm.vm_mut().eval(
                    "JSON.stringify(globalThis.__credentiallessChildNavigationPartitionResult)",
                )
            })
            .await
            .expect("credentialless child navigation partitioning test should run on owner lane");

        let _ = shutdown_server.send(());
        let request_count = server
            .await
            .expect("credentialless child navigation partition server should finish");
        assert_eq!(outcome, r#"["credentialless","normal"]"#);
        assert_eq!(
            request_count, 2,
            "credentialless and normal child navigations should use separate network/cache partitions"
        );
    })
    .await;
}

#[tokio::test]
async fn credentialless_child_xhr_uses_credentialless_network_partition_key() {
    run_page_vm_async_test(async move {
        let (base_url, shutdown_server, server) =
            spawn_credentialless_partition_fetch_cache_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let data_url_literal =
            serde_json::to_string(&format!("{base_url}/data")).expect("serialize data url");
        let credentialless_srcdoc = format!(
            r#"<!doctype html><script>
(() => {{
  const xhr = new XMLHttpRequest();
  xhr.onload = () => parent.postMessage({{ label: "credentialless", text: xhr.responseText }}, "*");
  xhr.onerror = () => parent.postMessage({{ label: "credentialless", error: "error" }}, "*");
  xhr.open("GET", {data_url_literal});
  xhr.send();
}})();
</script>"#
        );
        let normal_srcdoc = format!(
            r#"<!doctype html><script>
(() => {{
  const xhr = new XMLHttpRequest();
  xhr.onload = () => parent.postMessage({{ label: "normal", text: xhr.responseText }}, "*");
  xhr.onerror = () => parent.postMessage({{ label: "normal", error: "error" }}, "*");
  xhr.open("GET", {data_url_literal});
  xhr.send();
}})();
</script>"#
        );
        let credentialless_srcdoc_literal =
            serde_json::to_string(&credentialless_srcdoc).expect("serialize credentialless srcdoc");
        let normal_srcdoc_literal =
            serde_json::to_string(&normal_srcdoc).expect("serialize normal srcdoc");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let outcome = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__credentiallessXhrPartitionDone = false;
                        globalThis.__credentiallessXhrPartitionResult = [];
                        const normalSrcdoc = {normal_srcdoc_literal};
                        window.addEventListener("message", (event) => {{
                            if (!event.data || !event.data.label) {{
                                return;
                            }}
                            globalThis.__credentiallessXhrPartitionResult.push(
                                event.data.error ? "error:" + event.data.error : event.data.text
                            );
                            if (event.data.label === "credentialless") {{
                                const normal = document.createElement("iframe");
                                normal.srcdoc = normalSrcdoc;
                                document.body.append(normal);
                            }} else {{
                                globalThis.__credentiallessXhrPartitionDone = true;
                            }}
                        }});
                        const credentialless = document.createElement("iframe");
                        credentialless.credentialless = true;
                        credentialless.srcdoc = {credentialless_srcdoc_literal};
                        document.body.append(credentialless);
                    }})()
                    "#,
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__credentiallessXhrPartitionDone === true)",
                    "credentialless child XHR partitioning should finish",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__credentiallessXhrPartitionResult)")
            })
            .await
            .expect("credentialless child XHR partitioning test should run on owner lane");

        let _ = shutdown_server.send(());
        let request_count = server
            .await
            .expect("credentialless child XHR partition server should finish");
        assert_eq!(outcome, r#"["credentialless","normal"]"#);
        assert_eq!(
            request_count, 2,
            "credentialless and normal child XHRs should use separate network/cache partitions"
        );
    })
    .await;
}

#[tokio::test]
async fn navigator_send_beacon_without_body_does_not_synthesize_content_type() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_request_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (returned, request) = local_executor
            .run(async move {
                let returned = page_vm.vm_mut().eval(
                    r#"
                    (() => String(navigator.sendBeacon("/beacon")))()
                    "#,
                )?;
                let request = tokio::time::timeout(Duration::from_secs(3), request_rx)
                    .await
                    .expect("sendBeacon request should reach fixture")
                    .expect("sendBeacon fixture should capture request");
                drain_page_work_until_no_pending_subresources(
                    &mut page_vm,
                    "sendBeacon network completion should be observed",
                )
                .await?;
                Ok::<_, anyhow::Error>((returned, request))
            })
            .await
            .expect("sendBeacon test should run on owner lane");

        server.await.expect("request capture server should finish");

        let request_lower = request.to_ascii_lowercase();
        assert_eq!(returned, "true");
        assert!(request.starts_with("POST /beacon HTTP/1.1\r\n"));
        assert!(!request_lower.contains("content-type:"));
        assert!(request_lower.contains("sec-fetch-mode: no-cors\r\n"));
    })
    .await;
}

#[tokio::test]
async fn anchor_ping_click_posts_ping_subresource_before_navigation() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_request_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url.clone());
        let local_executor = page_vm.local_executor.clone();

        let (request, network_output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r##"
                    (() => {
                        const link = document.createElement("a");
                        link.href = "#next";
                        link.ping = "/audit";
                        document.body.appendChild(link);
                        link.click();
                        return location.href;
                    })()
                    "##,
                )?;
                let request = tokio::time::timeout(Duration::from_secs(3), request_rx)
                    .await
                    .expect("anchor ping request should reach fixture")
                    .expect("anchor ping fixture should capture request");
                drain_page_work_until_no_pending_subresources(
                    &mut page_vm,
                    "anchor ping network completion should be observed",
                )
                .await?;
                Ok::<_, anyhow::Error>((request, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("anchor ping test should run on owner lane");

        server.await.expect("request capture server should finish");

        let request_lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /audit HTTP/1.1\r\n"));
        assert!(request_lower.contains("content-type: text/ping\r\n"));
        assert!(request_lower.contains("cache-control: max-age=0\r\n"));
        assert!(request.contains(&format!("Ping-To: {document_url}#next\r\n")));
        assert!(request.contains(&format!("Ping-From: {document_url}\r\n")));
        assert!(request_lower.contains("sec-fetch-mode: no-cors\r\n"));
        assert!(request.ends_with("\r\n\r\nPING"));

        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.resource_type(), SubresourceResourceType::Ping);
        assert_eq!(record.request_body(), Some("PING"));
    })
    .await;
}

#[tokio::test]
async fn window_fetch_abort_cancels_inflight_network_request_and_rejects_once() {
    run_page_vm_async_test(async move {
            let (base_url, disconnect_rx, server) = spawn_disconnect_observing_http_server().await;
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url = format!("{base_url}/fetch");
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let observed = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            globalThis.__fetchEvents = [];
                            globalThis.__fetchObserved = null;
                            const controller = new AbortController();
                            fetch({fetch_url_literal}, {{ signal: controller.signal }}).then(
                                () => {{
                                    globalThis.__fetchEvents.push("fulfilled");
                                }},
                                (error) => {{
                                    globalThis.__fetchEvents.push(
                                        "error:" + error.name + ":" + (error instanceof DOMException) + ":" + error.message
                                    );
                                }},
                            ).finally(() => {{
                                setTimeout(() => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        events: globalThis.__fetchEvents,
                                        signalAborted: controller.signal.aborted,
                                    }});
                                    globalThis.__fetchDone = true;
                                }}, 60);
                            }});
                            setTimeout(() => controller.abort(), 40);
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch abort should reject exactly once",
                    )
                    .await?;
                    page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")
                })
                .await
                .expect("fetch abort test should run on owner lane");

            let disconnected = tokio::time::timeout(Duration::from_secs(3), disconnect_rx)
                .await
                .expect("disconnect observation should complete")
                .expect("disconnect observation should be sent");
            server
                .await
                .expect("disconnect-observing server should finish");

            assert!(disconnected);
            assert_eq!(
                observed.as_str(),
                r#"{"events":["error:AbortError:true:The operation was aborted."],"signalAborted":true}"#
            );
    })
    .await;
}

#[tokio::test]
async fn window_fetch_abort_after_headers_records_network_failure_terminal() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind headers-first fetch abort server");
        let addr = listener
            .local_addr()
            .expect("headers-first fetch abort server address");
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept headers-first fetch abort request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read headers-first fetch abort request");
            assert!(request.starts_with("GET /stream HTTP/1.1"));
            stream
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/plain; charset=utf-8\r\n",
                        "Connection: close\r\n",
                        "\r\n",
                        "first",
                    )
                    .as_bytes(),
                )
                .await
                .expect("write headers-first fetch abort response");
            stream
                .flush()
                .await
                .expect("flush headers-first fetch abort response");
            let _ = release_rx.await;
        });

        let document_url = Url::parse(&format!("http://{addr}/page.html"))
            .expect("headers-first fetch abort document URL");
        let fetch_url = format!("http://{addr}/stream");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let (observed, network_output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__headersFirstAbortDone = false;
                        globalThis.__headersFirstAbortObserved = null;
                        const controller = new AbortController();
                        (async () => {
                            const response = await fetch("/stream", {
                                signal: controller.signal,
                            });
                            const reader = response.body.getReader();
                            const first = await reader.read();
                            controller.abort();
                            const failure = await reader.read().then(
                                () => "fulfilled",
                                (error) => error && error.name,
                            );
                            globalThis.__headersFirstAbortObserved = [
                                response.status,
                                new TextDecoder().decode(first.value),
                                failure,
                            ].join("|");
                        })().catch((error) => {
                            globalThis.__headersFirstAbortObserved =
                                "outer:" + String(error && error.name);
                        }).finally(() => {
                            globalThis.__headersFirstAbortDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__headersFirstAbortDone === true)",
                    "headers-first fetch abort should reject the body reader",
                )
                .await?;
                let observed = page_vm
                    .vm_mut()
                    .eval("String(globalThis.__headersFirstAbortObserved)")?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("headers-first fetch abort test should run on owner lane");

        let _ = release_tx.send(());
        server
            .await
            .expect("headers-first fetch abort server should finish");
        assert_eq!(observed, "200|first|AbortError");

        let items = network_output.into_items().collect::<Vec<_>>();
        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(
                    item,
                    ScriptNetworkOutputItem::SubresourceRequestStarted(_)
                ))
                .count(),
            1,
        );
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                if request.url().as_str() == fetch_url
                    && request.resource_type() == SubresourceResourceType::Fetch
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            ScriptNetworkOutputItem::SubresourceResponseStarted(response)
                if response.status() == 200
        )));
        let terminals = items
            .iter()
            .filter_map(|item| match item {
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => Some(body),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 1);
        assert!(matches!(
            terminals[0].result(),
            SubresourceBodyFinishedResult::FailedWithPartialBody { error_text, .. }
                if error_text == crate::network_host::ABORTED_ERROR_TEXT
        ));
    })
    .await;
}

#[tokio::test]
async fn child_initial_about_blank_fetch_inherits_parent_response_referrer_policy() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_header_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm =
            test_page_vm_with_response_referrer_policy(document_url, "no-referrer");
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__childFetchDone = false;
                            globalThis.__childFetchOutcome = "";
                            const frame = document.createElement("iframe");
                            document.body.append(frame);
                            frame.contentWindow.fetch("/api")
                                .then((response) => response.text())
                                .then(() => {
                                    globalThis.__childFetchOutcome = "ok";
                                    globalThis.__childFetchDone = true;
                                })
                                .catch((error) => {
                                    globalThis.__childFetchOutcome =
                                        error.name + ":" + error.message;
                                    globalThis.__childFetchDone = true;
                                });
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__childFetchDone === true)",
                    "child initial about:blank fetch should complete",
                )
                .await?;
                assert_eq!(page_vm.vm_mut().eval("globalThis.__childFetchOutcome")?, "ok");
                anyhow::Ok(())
            })
            .await
            .expect("child fetch referrer policy test should run on owner lane");

        let request = request_rx.await.expect("captured child fetch request");
        server.await.expect("child fetch capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("GET /api HTTP/1.1\r\n"));
        assert!(
            !request_lower.contains("\r\nreferer:"),
            "child initial about:blank fetch must inherit parent Referrer-Policy: no-referrer; request was:\n{request}"
        );
    })
    .await;
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

#[tokio::test]
async fn child_response_fetch_uses_response_policy_after_initial_no_referrer_inheritance() {
    run_page_vm_async_test(async move {
        let child_body = r#"<!doctype html><script>
fetch("/api")
  .then(response => response.text())
  .then(() => parent.postMessage("child-fetch-ok", "*"))
  .catch(error => parent.postMessage("child-fetch-error:" + error.name + ":" + error.message, "*"));
</script>"#
            .to_owned();
        let (base_url, request_rx, server) =
            spawn_document_then_api_capture_server("/child.html", child_body).await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm =
            test_page_vm_with_response_referrer_policy(document_url, "no-referrer");
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__childResponseFetchMessage = "";
                            addEventListener("message", event => {
                                globalThis.__childResponseFetchMessage = String(event.data);
                            });
                            const frame = document.createElement("iframe");
                            frame.src = "/child.html";
                            document.body.append(frame);
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__childResponseFetchMessage !== '')",
                    "child response fetch should report completion",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__childResponseFetchMessage")?,
                    "child-fetch-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("child response fetch policy test should run on owner lane");

        let request = request_rx.await.expect("captured child response fetch");
        server
            .await
            .expect("child response fetch capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("GET /api HTTP/1.1\r\n"));
        assert!(request_lower.contains("\r\nreferer: "));
        assert!(
            request_lower.contains("/child.html\r\n"),
            "child response commit must replace inherited no-referrer policy; request was:\n{request}"
        );
    })
    .await;
}

#[tokio::test]
async fn cross_site_child_credentialed_xhr_reports_active_storage_access() {
    run_page_vm_async_test(async move {
        let child_body = r#"<!doctype html><script>
const xhr = new XMLHttpRequest();
xhr.open("POST", "/api");
xhr.withCredentials = true;
xhr.onload = () => parent.postMessage("child-xhr-ok", "*");
xhr.onerror = () => parent.postMessage("child-xhr-error", "*");
xhr.send();
</script>"#
            .to_owned();
        let (child_origin, request_rx, server) =
            spawn_document_then_api_capture_server("/child.html", child_body).await;
        let document_url = Url::parse("http://top.test/page.html").expect("document url");
        let child_url = serde_json::to_string(&format!("{child_origin}/child.html"))
            .expect("serialize child URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__childStorageAccessMessage = "";
                            addEventListener("message", event => {{
                                globalThis.__childStorageAccessMessage = String(event.data);
                            }});
                            const frame = document.createElement("iframe");
                            frame.src = {child_url};
                            document.body.append(frame);
                        }})()
                        "#,
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__childStorageAccessMessage !== '')",
                    "credentialed cross-site child XHR should report completion",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__childStorageAccessMessage")?,
                    "child-xhr-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("child storage-access XHR test should run on owner lane");

        let request = request_rx
            .await
            .expect("captured credentialed child XHR request");
        server
            .await
            .expect("child storage-access capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("POST /api HTTP/1.1\r\n"));
        assert!(
            request_lower.contains("\r\nsec-fetch-site: same-origin\r\n"),
            "child XHR initiator must be its committed Document; request was:\n{request}"
        );
        assert!(
            request_lower.contains("\r\nsec-fetch-storage-access: active\r\n"),
            "credentialed child XHR must expose its active third-party cookie access; request was:\n{request}"
        );
    })
    .await;
}

#[tokio::test]
async fn xhr_emits_browser_style_subresource_headers_on_wire() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_header_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__xhrDone = false;
                            const xhr = new XMLHttpRequest();
                            xhr.open("GET", {xhr_url_literal});
                            xhr.setRequestHeader("X-Test", "xhr");
                            xhr.onload = () => {{
                                globalThis.__xhrDone = true;
                            }};
                            xhr.send();
                        }})()
                        "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__xhrDone === true)",
                    "xhr header capture request should complete",
                )
                .await
            })
            .await
            .expect("xhr header capture test should run on owner lane");

        let request = request_rx.await.expect("captured xhr request");
        server.await.expect("header capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("GET /xhr HTTP/1.1\r\n"));
        assert!(request_lower.contains("x-test: xhr\r\n"));
        assert!(request_lower.contains("referer: "));
        assert!(request_lower.contains("/page.html\r\n"));
        assert!(request_lower.contains("accept: */*\r\n"));
        assert!(request_lower.contains("accept-language: en-us,en;q=0.9\r\n"));
        assert!(request_lower.contains("sec-fetch-site: same-origin\r\n"));
        assert!(request_lower.contains("sec-fetch-mode: cors\r\n"));
        assert!(request_lower.contains("sec-fetch-dest: empty\r\n"));
        assert!(request_lower.contains("sec-ch-ua: "));
        assert!(request_lower.contains("sec-ch-ua-mobile: ?0\r\n"));
        let expected_platform_header = format!(
            "sec-ch-ua-platform: {}\r\n",
            DEFAULT_SEC_CH_UA_PLATFORM.to_ascii_lowercase()
        );
        assert!(request_lower.contains(&expected_platform_header));
    })
    .await;
}

#[tokio::test]
async fn xhr_load_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 200 OK",
            "xhr-ok".to_owned(),
            Duration::ZERO,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let (
            completion_sources,
            events_after_xhr,
            script_ready_source,
            events_after_script_ready,
            lifecycle_and_host_load_sources,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__xhrReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const xhr = new XMLHttpRequest();
  xhr.onload = () => {{
    __xhrReadyEvents.push("xhr-load:" + xhr.responseText);
    const frame = document.createElement("iframe");
    frame.onload = () => __xhrReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__xhrReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  }};
  xhr.open("GET", {xhr_url_literal});
  xhr.send();
}})()
"#
                ))?;

                let mut completion_sources = Vec::new();
                let events_after_xhr = loop {
                    if !page_vm.page_resource_completion_queue().has_ready_completion() {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("xhr completion should arrive before timeout");
                    }
                    let completion =
                        run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    completion_sources.push(completion.action.source());
                    let events = page_vm.vm_mut().eval("__xhrReadyEvents.join('|')")?;
                    if events == "xhr-load:xhr-ok" {
                        break events;
                    }
                    assert!(
                        completion_sources.len() < 8,
                        "xhr load should dispatch after a bounded number of completions; sources: {completion_sources:?}, events: {events}"
                    );
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "XHR-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "XHR-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm.vm_mut().eval("__xhrReadyEvents.join('|')")?;
                let mut lifecycle_and_host_load_sources = Vec::new();
                let events_after_host_load = loop {
                    let source = page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await
                        .expect("child lifecycle or HostLoad source should remain ready");
                    lifecycle_and_host_load_sources.push(source);
                    let events = page_vm.vm_mut().eval("__xhrReadyEvents.join('|')")?;
                    if events == "xhr-load:xhr-ok|child-script:true|frame-load" {
                        break events;
                    }
                    assert_eq!(
                        source,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "only document-owned lifecycle turns may precede final HostLoad delivery"
                    );
                    assert!(
                        lifecycle_and_host_load_sources.len() < 8,
                        "XHR-created child lifecycle should reach HostLoad in bounded owner turns: {lifecycle_and_host_load_sources:?}"
                    );
                };

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_xhr,
                    script_ready_source,
                    events_after_script_ready,
                    lifecycle_and_host_load_sources,
                    events_after_host_load,
                ))
            })
            .await
            .expect("xhr ready-work source test should run");

        assert!(
            completion_sources
                .iter()
                .all(|source| *source == RendererOwnerResourceActivitySource::AsyncSubresource),
            "XHR load should be driven only by async-subresource completions: {completion_sources:?}"
        );
        assert_eq!(
            events_after_xhr, "xhr-load:xhr-ok",
            "XHR load handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "XHR-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "xhr-load:xhr-ok|child-script:true",
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
            "only DocumentLifecycle turns may run between XHR-created script execution and load delivery: {lifecycle_and_host_load_sources:?}"
        );
        assert_eq!(
            lifecycle_and_host_load_sources.last(),
            Some(&ChildFrameSemanticTurnKind::HostLoad),
            "iframe load must remain a later HostLoad turn after document lifecycle"
        );
        assert_eq!(
            events_after_host_load, "xhr-load:xhr-ok|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server.await.expect("xhr ready-work server should finish");
    })
    .await;
}

#[tokio::test]
async fn xhr_event_target_inherits_event_target_methods_without_own_shadowing() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.addEventListener("readystatechange", () => events.push("listener"));
                        xhr.onreadystatechange = () => events.push("property");
                        xhr.dispatchEvent(new Event("readystatechange"));
                        return JSON.stringify({
                            xhrEventTargetOwnAdd: Object.hasOwn(XMLHttpRequestEventTarget.prototype, "addEventListener"),
                            xhrEventTargetOwnRemove: Object.hasOwn(XMLHttpRequestEventTarget.prototype, "removeEventListener"),
                            xhrEventTargetOwnDispatch: Object.hasOwn(XMLHttpRequestEventTarget.prototype, "dispatchEvent"),
                            inheritedName: XMLHttpRequestEventTarget.prototype.addEventListener.name,
                            inheritedLength: XMLHttpRequestEventTarget.prototype.addEventListener.length,
                            instanceOfEventTarget: xhr instanceof EventTarget,
                            instanceOfXhrEventTarget: xhr instanceof XMLHttpRequestEventTarget,
                            events,
                        });
                    })()
                    "#,
                )
            })
            .await
            .expect("xhr EventTarget prototype test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"xhrEventTargetOwnAdd":false,"xhrEventTargetOwnRemove":false,"xhrEventTargetOwnDispatch":false,"inheritedName":"addEventListener","inheritedLength":2,"instanceOfEventTarget":true,"instanceOfXhrEventTarget":true,"events":["listener","property"]}"#
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_blocks_send_and_returns_materialized_response() {
    run_page_vm_async_test(async move {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind sync XHR test server");
        let addr = listener.local_addr().expect("sync XHR server local addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let body = "sync-ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write sync XHR response");
            })
            .expect("spawn sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        for (const type of ["loadstart", "progress", "load", "loadend"]) {{
                            xhr.addEventListener(type, event => events.push(
                                `${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                            ));
                            xhr.upload.addEventListener(type, event => events.push(
                                `upload.${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                            ));
                        }}
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
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
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("sync XHR test should run on owner lane");

        server.join().expect("sync XHR server should finish");
        assert_eq!(
            observed,
            format!(
                r#"{{"events":["readystatechange:1","readystatechange:4","load:7:7:true","loadend:7:7:true"],"readyState":4,"status":200,"statusText":"OK","responseText":"sync-ok","responseURL":"{xhr_url}","contentType":"text/plain; charset=utf-8","allHeaders":"content-type: text/plain; charset=utf-8\r\ncontent-length: 7\r\nconnection: close\r\n"}}"#
            )
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.url().as_str(), xhr_url);
        assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
        let SubresourceNetworkOutcome::Success { status, .. } = record.outcome() else {
            panic!("expected sync XHR network success, got {:?}", record.outcome());
        };
        assert_eq!(*status, 200);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_rejects_cross_origin_response_without_cors_headers() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-deny",
            "cross-origin-secret",
            vec![],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-deny");
        let document_url = Url::parse("http://source.test/page.html").expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("cross-origin synchronous XHR probe should run on owner lane");

        server
            .join()
            .expect("cross-origin synchronous XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text.contains("CORS check failed: no Access-Control-Allow-Origin")
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_allows_cross_origin_response_with_matching_cors_origin() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-allow",
            "cors-visible",
            vec![
                ("Access-Control-Allow-Origin", "http://source.test"),
                ("Access-Control-Expose-Headers", "X-Visible-Token"),
                ("X-Visible-Token", "public-value"),
                ("X-Internal-Token", "private-value"),
            ],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-allow");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize XHR URL");
        let document_url = Url::parse("http://source.test/page.html").expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
                            readyState: xhr.readyState,
                            status: xhr.status,
                            responseText: xhr.responseText,
                            responseURL: xhr.responseURL,
                            visibleToken: xhr.getResponseHeader("X-Visible-Token"),
                            internalToken: xhr.getResponseHeader("X-Internal-Token"),
                            allowOrigin: xhr.getResponseHeader("Access-Control-Allow-Origin"),
                        }});
                    }})()
                    "#,
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("allowed cross-origin synchronous XHR should run on owner lane");

        server
            .join()
            .expect("allowed cross-origin synchronous XHR server should finish");
        assert_eq!(
            observed,
            format!(
                r#"{{"readyState":4,"status":200,"responseText":"cors-visible","responseURL":"{xhr_url}","visibleToken":"public-value","internalToken":null,"allowOrigin":null}}"#
            )
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Success { status: 200, .. }
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_rejects_cross_origin_response_with_mismatched_cors_origin() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-mismatch",
            "must-not-be-visible",
            vec![("Access-Control-Allow-Origin", "http://other.test")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-mismatch");
        let observed_and_network = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_failure_probe_expression(&xhr_url),
        )
        .await;

        server
            .join()
            .expect("mismatched-origin synchronous XHR server should finish");
        let (observed, network_output) = observed_and_network;
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        assert_single_synchronous_xhr_network_failure(
            network_output,
            "Access-Control-Allow-Origin `http://other.test` does not allow http://source.test",
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_allows_wildcard_cors_without_credentials() {
    run_page_vm_async_test(async move {
        let body = "wildcard-visible";
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-wildcard",
            body,
            vec![("Access-Control-Allow-Origin", "*")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-wildcard");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_success_probe_expression(&xhr_url, false),
        )
        .await;

        server
            .join()
            .expect("wildcard synchronous XHR server should finish");
        assert_synchronous_xhr_success_surface(&observed, &xhr_url, body);
        assert_single_synchronous_xhr_network_success(network_output);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_rejects_wildcard_cors_with_credentials() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-wildcard-credentials",
            "must-not-be-visible",
            vec![
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Credentials", "true"),
            ],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-wildcard-credentials");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_failure_probe_expression_with_credentials(&xhr_url, true),
        )
        .await;

        server
            .join()
            .expect("credentialed wildcard synchronous XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        assert_single_synchronous_xhr_network_failure(
            network_output,
            "wildcard Access-Control-Allow-Origin does not allow credentialed requests",
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_requires_allow_credentials_for_credentialed_cors() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-missing-credentials",
            "must-not-be-visible",
            vec![("Access-Control-Allow-Origin", "http://source.test")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-missing-credentials");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_failure_probe_expression_with_credentials(&xhr_url, true),
        )
        .await;

        server
            .join()
            .expect("missing-credentials synchronous XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        assert_single_synchronous_xhr_network_failure(
            network_output,
            "require Access-Control-Allow-Credentials: true",
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_allows_credentialed_cors_with_explicit_opt_in() {
    run_page_vm_async_test(async move {
        let body = "credentialed-visible";
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-credentials-allow",
            body,
            vec![
                ("Access-Control-Allow-Origin", "http://source.test"),
                ("Access-Control-Allow-Credentials", "true"),
            ],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-credentials-allow");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_success_probe_expression(&xhr_url, true),
        )
        .await;

        server
            .join()
            .expect("credentialed synchronous XHR server should finish");
        assert_synchronous_xhr_success_surface(&observed, &xhr_url, body);
        assert_single_synchronous_xhr_network_success(network_output);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_same_origin_response_keeps_non_cors_headers_visible() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-same-origin-headers",
            "same-origin-visible",
            vec![("X-Same-Origin-Token", "same-origin-secret")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-same-origin-headers");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize XHR URL");
        let expression = format!(
            r#"
            (() => {{
                const xhr = new XMLHttpRequest();
                xhr.open("GET", {xhr_url_literal}, false);
                xhr.send();
                return JSON.stringify({{
                    status: xhr.status,
                    responseText: xhr.responseText,
                    token: xhr.getResponseHeader("X-Same-Origin-Token"),
                }});
            }})()
            "#,
        );
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse(&format!("{target_base_url}/page.html")).expect("document url"),
            expression,
        )
        .await;

        server
            .join()
            .expect("same-origin synchronous XHR server should finish");
        assert_eq!(
            observed,
            r#"{"status":200,"responseText":"same-origin-visible","token":"same-origin-secret"}"#
        );
        assert_single_synchronous_xhr_network_success(network_output);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_uses_chromium_progress_totals_without_progress_events() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind synchronous XHR progress server");
        let addr = listener
            .local_addr()
            .expect("synchronous XHR progress server address");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("sync-xhr-progress-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                for expected_path in ["/without-length", "/no-content"] {
                    let (mut stream, _) = listener
                        .accept()
                        .expect("accept synchronous XHR progress request");
                    let mut request = Vec::new();
                    let mut byte = [0_u8; 1];
                    loop {
                        stream
                            .read_exact(&mut byte)
                            .expect("read synchronous XHR progress request");
                        request.push(byte[0]);
                        if request.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8(request).expect("request should be UTF-8");
                    assert!(request.starts_with(&format!("GET {expected_path} HTTP/1.1\r\n")));
                    let response = if expected_path == "/without-length" {
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOK"
                    } else {
                        "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
                    };
                    stream
                        .write_all(response.as_bytes())
                        .expect("write synchronous XHR progress response");
                }
            })
            .expect("spawn synchronous XHR progress server");

        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let base_url_literal = serde_json::to_string(&base_url).expect("serialize base URL");
        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                      const probe = url => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push(`readystatechange:${{xhr.readyState}}`);
                        for (const type of ["loadstart", "progress", "load", "loadend"]) {{
                          xhr.addEventListener(type, event => events.push(
                            `${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                          ));
                          xhr.upload.addEventListener(type, event => events.push(
                            `upload.${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                          ));
                        }}
                        xhr.open("GET", url, false);
                        xhr.send("ignored body");
                        return {{url, events, status: xhr.status, responseText: xhr.responseText}};
                      }};
                      return JSON.stringify([
                        probe({base_url_literal} + "/without-length"),
                        probe({base_url_literal} + "/no-content"),
                        probe("data:text/plain,ok")
                      ]);
                    }})()
                    "#
                ))
            })
            .await
            .expect("synchronous XHR progress probe should run on owner lane");

        server
            .join()
            .expect("synchronous XHR progress server should finish");
        let observed: serde_json::Value =
            serde_json::from_str(&observed).expect("progress probe should return JSON");
        assert_eq!(
            observed[0],
            serde_json::json!({
                "url": format!("{base_url}/without-length"),
                "events": [
                    "readystatechange:1",
                    "readystatechange:4",
                    "load:2:0:false",
                    "loadend:2:0:false"
                ],
                "status": 200,
                "responseText": "OK"
            })
        );
        assert_eq!(
            observed[1],
            serde_json::json!({
                "url": format!("{base_url}/no-content"),
                "events": [
                    "readystatechange:1",
                    "readystatechange:4",
                    "load:0:0:false",
                    "loadend:0:0:false"
                ],
                "status": 204,
                "responseText": ""
            })
        );
        assert_eq!(
            observed[2],
            serde_json::json!({
                "url": "data:text/plain,ok",
                "events": [
                    "readystatechange:1",
                    "readystatechange:4",
                    "load:2:2:true",
                    "loadend:2:2:true"
                ],
                "status": 200,
                "responseText": "ok"
            })
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_normalizes_lowercase_standard_method_before_fetch() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind lowercase sync XHR test server");
        let addr = listener
            .local_addr()
            .expect("lowercase sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("lowercase-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let body = "lowercase-ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write sync XHR response");
                String::from_utf8(request).expect("request should be utf-8")
            })
            .expect("spawn lowercase sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/sync-xhr-lowercase");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        xhr.open("get", {xhr_url_literal}, false);
                        xhr.send("ignored body");
                        return `${{xhr.status}}|${{xhr.responseText}}`;
                    }})()
                    "#
                ))
            })
            .await
            .expect("lowercase sync XHR test should run on owner lane");
        let request = server.join().expect("lowercase sync XHR server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert_eq!(observed, "200|lowercase-ok");
        assert!(request.starts_with("GET /sync-xhr-lowercase HTTP/1.1\r\n"));
        assert!(!request_lower.contains("content-length:"));
        assert!(!request_lower.contains("content-type:"));
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_without_timeout_waits_for_slow_response() {
    run_page_vm_async_test(async move {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind slow sync XHR test server");
        let addr = listener.local_addr().expect("slow sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("slow-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept slow sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read slow sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(150));
                let body = "late";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            })
            .expect("spawn slow sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/slow-sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let started = Instant::now();
        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        xhr.onerror = () => events.push("error");
                        xhr.onload = () => events.push("load");
                        xhr.onloadend = () => events.push("loadend");
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                            responseText: xhr.responseText,
                        }});
                    }})()
                    "#
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("slow sync XHR test should run on owner lane");
        let elapsed = started.elapsed();

        server.join().expect("slow sync XHR server should finish");
        assert!(
            elapsed >= Duration::from_millis(120),
            "sync XHR without timeout should wait for the slow response, elapsed={elapsed:?}"
        );
        assert_eq!(
            observed,
            r#"{"events":["readystatechange:1","readystatechange:4","load","loadend"],"readyState":4,"status":200,"responseText":"late"}"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.url().as_str(), xhr_url);
        assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
        let SubresourceNetworkOutcome::Success { status, .. } = record.outcome() else {
            panic!("expected sync XHR network success, got {:?}", record.outcome());
        };
        assert_eq!(*status, 200);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_aborts_when_page_context_is_cancelled() {
    run_page_vm_async_test(async move {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind cancelled sync XHR server");
        let addr = listener.local_addr().expect("cancelled sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("cancelled-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept cancelled sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read cancelled sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlate",
                );
            })
            .expect("spawn cancelled sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let cancel_tx = page_vm.vm().page_context_cancel_sender();
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/cancelled-sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let canceller = std::thread::Builder::new()
            .name("sync-xhr-page-canceller".to_owned())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(40));
                cancel_tx.cancel(crate::runtime::RendererPageContextCancelReason::PageClosed);
            })
            .expect("spawn sync XHR page canceller");

        let started = Instant::now();
        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        xhr.onabort = () => events.push("abort");
                        xhr.onerror = () => events.push("error");
                        xhr.onload = () => events.push("load");
                        xhr.onloadend = () => events.push("loadend");
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                            responseText: xhr.responseText,
                        }});
                    }})()
                    "#
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("cancelled sync XHR test should run on owner lane");
        let elapsed = started.elapsed();

        canceller.join().expect("sync XHR page canceller should finish");
        server
            .join()
            .expect("cancelled sync XHR server should finish");
        assert!(
            elapsed < Duration::from_millis(150),
            "sync XHR should abort when the page context is cancelled, elapsed={elapsed:?}"
        );
        assert_eq!(
            observed,
            r#"{"events":["readystatechange:1","abort","loadend"],"readyState":0,"status":0,"responseText":""}"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.url().as_str(), xhr_url);
        let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
            panic!(
                "expected sync XHR page-cancel failure, got {:?}",
                record.outcome()
            );
        };
        assert!(
            error_text.contains("Synchronous XMLHttpRequest aborted because page was closed"),
            "error_text={error_text}"
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_cancel_is_replayed_for_abort_handler_xhr() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind chained cancelled sync XHR server");
        let addr = listener.local_addr().expect("chained sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("chained-cancelled-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener
                    .accept()
                    .expect("accept chained cancelled sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read chained cancelled sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlate",
                );
            })
            .expect("spawn chained cancelled sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let cancel_tx = page_vm.vm().page_context_cancel_sender();
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/chained-cancelled-sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let canceller = std::thread::Builder::new()
            .name("chained-sync-xhr-page-canceller".to_owned())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(40));
                cancel_tx.cancel(crate::runtime::RendererPageContextCancelReason::PageClosed);
            })
            .expect("spawn chained sync XHR page canceller");

        let started = Instant::now();
        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const events = [];
                        const first = new XMLHttpRequest();
                        first.onabort = () => {{
                            events.push("first-abort");
                            const second = new XMLHttpRequest();
                            second.onabort = () => events.push("second-abort");
                            second.ontimeout = () => events.push("second-timeout");
                            second.onerror = () => events.push("second-error");
                            second.onload = () => events.push("second-load");
                            second.onloadend = () => events.push("second-loadend");
                            second.open("GET", {xhr_url_literal}, false);
                            second.send();
                            events.push("second-after-send:" + second.readyState);
                        }};
                        first.open("GET", {xhr_url_literal}, false);
                        first.send();
                        return JSON.stringify(events);
                    }})()
                    "#
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("chained cancelled sync XHR test should run on owner lane");
        let elapsed = started.elapsed();

        canceller
            .join()
            .expect("chained sync XHR page canceller should finish");
        server
            .join()
            .expect("chained cancelled sync XHR server should finish");
        assert!(
            elapsed < Duration::from_millis(150),
            "chained sync XHR should replay page cancellation, elapsed={elapsed:?}"
        );
        assert_eq!(
            observed,
            r#"["first-abort","second-abort","second-loadend","second-after-send:0"]"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 2);
        for record in records {
            let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
                panic!(
                    "expected sync XHR page-cancel failure, got {:?}",
                    record.outcome()
                );
            };
            assert!(
                error_text.contains("Synchronous XMLHttpRequest aborted because page was closed"),
                "error_text={error_text}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_repeated_url_limit_breaks_request_floods() {
    run_page_vm_async_test(async move {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind sync XHR flood test server");
        let addr = listener.local_addr().expect("sync XHR flood server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("sync-xhr-flood-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                for _ in 0..32 {
                    let (mut stream, _) = listener.accept().expect("accept sync XHR flood request");
                    let mut request = Vec::new();
                    let mut byte = [0_u8; 1];
                    loop {
                        stream
                            .read_exact(&mut byte)
                            .expect("read sync XHR flood request");
                        request.push(byte[0]);
                        if request.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }
                    let body = "retry";
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write sync XHR flood response");
                }
            })
            .expect("spawn sync XHR flood test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/sync-xhr-flood");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        let completed = 0;
                        let thrown = "";
                        for (let i = 0; i < 33; i++) {{
                            const xhr = new XMLHttpRequest();
                            xhr.open("GET", {xhr_url_literal}, false);
                            try {{
                                xhr.send();
                                completed++;
                            }} catch (error) {{
                                thrown = `${{error && error.name}}:${{error && error.message}}`;
                                break;
                            }}
                        }}
                        return JSON.stringify({{ completed, thrown }});
                    }})()
                    "#
                ))
            })
            .await
            .expect("sync XHR flood test should run on owner lane");

        server.join().expect("sync XHR flood server should finish");
        assert_eq!(
            observed,
            r#"{"completed":32,"thrown":"TypeError:Synchronous XMLHttpRequest request limit exceeded"}"#
        );
    })
    .await;
}

#[tokio::test]
async fn xhr_abort_cancels_inflight_network_request_and_suppresses_late_failure() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen_rx, disconnect_rx, server) =
            spawn_request_seen_disconnect_observing_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onabort = () => globalThis.__xhrEvents.push("abort");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                setTimeout(() => {{
                                    globalThis.__xhrObserved = JSON.stringify({{
                                        events: globalThis.__xhrEvents,
                                        readyState: xhr.readyState,
                                        status: xhr.status,
                                        responseText: xhr.responseText,
                                    }});
                                    globalThis.__xhrDone = true;
                                }}, 60);
                            }};
                            globalThis.__abortXhr = () => xhr.abort();
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                ))?;
                let mut request_seen_rx = request_seen_rx;
                let request_seen_deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    match request_seen_rx.try_recv() {
                        Ok(()) => break,
                        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                            panic!("xhr abort server closed before observing the request");
                        }
                    }
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_some()
                    {}
                    let loader = page_vm.main_document_resource_loader();
                    page_vm
                        .advance_timers_until_deadline_for_test(loader.request_client())
                        .await?;
                    if Instant::now() >= request_seen_deadline {
                        panic!("timed out waiting for xhr abort server to observe the request");
                    }
                    let _ = tokio::time::timeout(
                        Duration::from_millis(10),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await;
                }
                page_vm.vm_mut().eval("globalThis.__abortXhr()")?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__xhrDone === true)",
                    "xhr abort should complete exactly one abort/loadend sequence",
                )
                .await?;
                page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")
            })
            .await
            .expect("xhr abort test should run on owner lane");

        let disconnected = tokio::time::timeout(Duration::from_secs(3), disconnect_rx)
            .await
            .expect("disconnect observation should complete")
            .expect("disconnect observation channel should stay open");
        server
            .await
            .expect("disconnect-observing http server should finish");

        assert!(
            disconnected,
            "expected xhr abort to close the underlying transport early"
        );
        assert_eq!(
            observed,
            r#"{"events":["abort","loadend"],"readyState":0,"status":0,"responseText":""}"#
        );
    })
    .await;
}

#[tokio::test]
async fn xhr_timeout_cancels_inflight_network_request_and_dispatches_timeout() {
    run_page_vm_async_test(async move {
            let (base_url, request_seen_rx, disconnect_rx, server) =
                spawn_request_seen_disconnect_observing_http_server().await;
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url = format!("{base_url}/xhr-timeout");
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let observed = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.ontimeout = () => globalThis.__xhrEvents.push("timeout");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.timeout = 1000;
                            xhr.send();
                            globalThis.__setXhrTimeout = () => {{
                                xhr.timeout = 20;
                            }};
                        }})()
                        "#
                    ))?;
                    let mut request_seen_rx = request_seen_rx;
                    let request_seen_deadline = Instant::now() + Duration::from_secs(3);
                    loop {
                        match request_seen_rx.try_recv() {
                            Ok(()) => break,
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                                panic!("xhr timeout server closed before observing the request");
                            }
                        }
                        while page_vm
                            .run_exact_page_websocket_selected_task_for_test().await?
                            .is_some()
                        {}
                        let loader = page_vm.main_document_resource_loader();
                        page_vm.advance_timers_until_deadline_for_test(loader.request_client()).await?;
                        if Instant::now() >= request_seen_deadline {
                            panic!("timed out waiting for xhr timeout server to observe the request");
                        }
                        let _ = tokio::time::timeout(
                            Duration::from_millis(10),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    page_vm.vm_mut().eval("globalThis.__setXhrTimeout()")?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr timeout should complete exactly one timeout/loadend sequence",
                    )
                    .await?;
                    page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")
                })
                .await
                .expect("xhr timeout test should run on owner lane");

            let disconnected = tokio::time::timeout(Duration::from_secs(3), disconnect_rx)
                .await
                .expect("disconnect observation should complete")
                .expect("disconnect observation channel should stay open");
            server
                .await
                .expect("delayed disconnect-observing http server should finish");

            assert!(
                disconnected,
                "expected xhr timeout to close the underlying transport early"
            );
            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","readystatechange:4","timeout","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
        })
        .await;
}

#[tokio::test]
async fn window_fetch_connection_refused_rejects_and_records_network_failure() {
    run_page_vm_async_test(async move {
            let (base_url, server) =
                spawn_connection_drop_http_server("/fetch-connection-refused").await;
            let fetch_url = format!("{base_url}/fetch-connection-refused");
            let document_url = Url::parse("http://127.0.0.1/page.html").expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            globalThis.__fetchObserved = null;
                            fetch({fetch_url_literal}).then(
                                () => {{
                                    globalThis.__fetchObserved = JSON.stringify({{ fulfilled: true }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasMessage: String(error && error.message).length > 0,
                                        stringStartsWithTypeError: String(error).startsWith("TypeError"),
                                    }});
                                }},
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch connection failure should reject",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch connection failure test should run on owner lane");

            assert_eq!(
                observed,
                r#"{"name":"TypeError","isTypeError":true,"hasMessage":true,"stringStartsWithTypeError":true}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
            ));
            server
                .await
                .expect("connection-drop fetch server should finish");
        })
        .await;
}

#[tokio::test]
async fn window_fetch_file_url_rejects_before_interception_or_transport() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://example.test/page.html").unwrap();
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (observed, pending_count, network_output) = local_executor
            .run(async move {
                page_vm.vm_mut().set_fetch_subresource_interception(
                    true,
                    Some(SubresourceResourceType::Fetch),
                );
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__fileFetchDone = false;
                        globalThis.__fileFetchObserved = null;
                        fetch("file:///moli-policy-must-not-open").then(
                            () => {
                                globalThis.__fileFetchObserved = JSON.stringify({ fulfilled: true });
                            },
                            (error) => {
                                globalThis.__fileFetchObserved = JSON.stringify({
                                    name: error && error.name,
                                    message: error && error.message,
                                    isTypeError: error instanceof TypeError,
                                });
                            },
                        ).finally(() => {
                            globalThis.__fileFetchDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__fileFetchDone === true)",
                    "file URL fetch should reject before interception",
                )
                .await?;
                let observed = page_vm
                    .vm_mut()
                    .eval("String(globalThis.__fileFetchObserved)")?;
                let pending_count = page_vm
                    .vm_mut()
                    .take_pending_subresource_fetch_infos()
                    .len();
                Ok::<_, anyhow::Error>((
                    observed,
                    pending_count,
                    page_vm.vm_mut().take_network_output(),
                ))
            })
            .await
            .expect("file URL fetch test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"name":"TypeError","message":"URL scheme \"file\" is not supported.","isTypeError":true}"#
        );
        assert_eq!(pending_count, 0, "unsupported schemes must not reach Fetch interception");
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Fetch);
        assert_eq!(
            records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "URL scheme \"file\" is not supported.".to_owned(),
            }
        );
    })
    .await;
}

#[tokio::test]
async fn window_fetch_revoked_blob_url_records_file_not_found_failure() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (observed, network_output) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__revokedBlobFetchDone = false;
                        globalThis.__revokedBlobFetchObserved = null;
                        const url = URL.createObjectURL(new Blob(["retired"]));
                        globalThis.__revokedBlobFetchUrl = url;
                        URL.revokeObjectURL(url);
                        fetch(url).then(
                            () => {
                                globalThis.__revokedBlobFetchObserved = "fulfilled";
                            },
                            (error) => {
                                globalThis.__revokedBlobFetchObserved = [
                                    error && error.name,
                                    error instanceof TypeError,
                                ].join("|");
                            },
                        ).finally(() => {
                            globalThis.__revokedBlobFetchDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__revokedBlobFetchDone === true)",
                    "revoked Blob URL fetch should reject",
                )
                .await?;
                let observed = page_vm.vm_mut().eval(
                    "globalThis.__revokedBlobFetchObserved + '|' + globalThis.__revokedBlobFetchUrl",
                )?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("revoked Blob URL fetch test should run on owner lane");

        let (error_shape, url) = observed
            .split_once("|blob:")
            .map(|(shape, suffix)| (shape, format!("blob:{suffix}")))
            .expect("probe should include its Blob URL");
        assert_eq!(error_shape, "TypeError|true");
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].url().as_str(), url);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Fetch);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text == crate::network_host::FILE_NOT_FOUND_ERROR_TEXT
        ));
    })
    .await;
}

#[tokio::test]
async fn window_fetch_dns_failure_rejects_and_records_network_failure() {
    run_page_vm_async_test(async move {
            let fetch_url = "http://moli-dns-failure.invalid./fetch-dns-failure";
            let mut page_vm = test_page_vm_with_config(dns_failure_fetch_config(), Vec::new());
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            globalThis.__fetchObserved = null;
                            fetch({fetch_url_literal}).then(
                                () => {{
                                    globalThis.__fetchObserved = JSON.stringify({{ fulfilled: true }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasMessage: String(error && error.message).length > 0,
                                        stringStartsWithTypeError: String(error).startsWith("TypeError"),
                                    }});
                                }},
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch DNS failure should reject",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch DNS failure test should run on owner lane");

            assert_eq!(
                observed,
                r#"{"name":"TypeError","isTypeError":true,"hasMessage":true,"stringStartsWithTypeError":true}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
                panic!("expected DNS fetch failure, got {:?}", record.outcome());
            };
            assert!(
                error_text.to_ascii_lowercase().contains("resolv"),
                "expected DNS-resolution error text, got {error_text:?}"
            );
        })
        .await;
}

#[tokio::test]
async fn window_fetch_redirect_error_rejects_before_following_redirect() {
    run_page_vm_async_test(async move {
            let (base_url, server) =
                spawn_single_redirect_http_server("/fetch-redirect-error", "/target").await;
            let fetch_url = format!("{base_url}/fetch-redirect-error");
            let document_url =
                Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            globalThis.__fetchObserved = null;
                            fetch({fetch_url_literal}, {{ redirect: "error" }}).then(
                                () => {{
                                    globalThis.__fetchObserved = JSON.stringify({{ fulfilled: true }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasRedirectModeMessage: String(error && error.message).includes("redirect mode error"),
                                    }});
                                }},
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch redirect error should reject",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch redirect-error test should run on owner lane");

            server.await.expect("redirect-error fetch server should finish");
            assert_eq!(
                observed,
                r#"{"name":"TypeError","isTypeError":true,"hasRedirectModeMessage":true}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("redirect mode error")
            ));
        })
        .await;
}

#[tokio::test]
async fn window_fetch_manual_redirect_returns_opaqueredirect_filtered_response() {
    run_page_vm_async_test(async move {
            let (base_url, server) =
                spawn_single_redirect_http_server("/fetch-redirect-manual", "/target").await;
            let fetch_url = format!("{base_url}/fetch-redirect-manual");
            let document_url =
                Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal =
                serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (async () => {{
                            const response = await fetch({fetch_url_literal}, {{ redirect: "manual" }});
                            const clone = response.clone();
                            const bodyUsedBefore = response.bodyUsed;
                            const text = await response.text();
                            const cloneText = await clone.text();
                            globalThis.__fetchObserved = JSON.stringify({{
                                type: response.type,
                                status: response.status,
                                ok: response.ok,
                                statusText: response.statusText,
                                redirected: response.redirected,
                                urlIsEmpty: response.url === "",
                                bodyIsNull: response.body === null,
                                headers: Array.from(response.headers),
                                bodyUsedBefore,
                                bodyUsedAfter: response.bodyUsed,
                                text,
                                cloneType: clone.type,
                                cloneStatus: clone.status,
                                cloneBodyIsNull: clone.body === null,
                                cloneText,
                            }});
                            globalThis.__fetchDone = true;
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch manual redirect should resolve",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch manual redirect test should run on owner lane");

            server.await.expect("manual redirect fetch server should finish");
            assert_eq!(
                observed,
                r#"{"type":"opaqueredirect","status":0,"ok":false,"statusText":"","redirected":false,"urlIsEmpty":true,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":true,"text":"","cloneType":"opaqueredirect","cloneStatus":0,"cloneBodyIsNull":true,"cloneText":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            let SubresourceNetworkOutcome::Success {
                status,
                response_headers,
                ..
            } = record.outcome()
            else {
                panic!(
                    "expected manual redirect network success, got {:?}",
                    record.outcome()
                );
            };
            assert_eq!(*status, 302);
            assert!(response_headers.iter().any(|header| {
                header.0.eq_ignore_ascii_case("location") && header.1 == "/target"
            }));
        })
        .await;
}

#[tokio::test]
async fn window_fetch_no_cors_cross_origin_returns_opaque_filtered_response() {
    run_page_vm_async_test(async move {
            let (base_url, request_rx, server) = spawn_header_capture_http_server().await;
            let fetch_url = format!("{base_url}/opaque-data");
            let server_origin = Url::parse(&base_url).expect("server url");
            let document_url = Url::parse(&format!(
                "http://127.0.0.1:{}/page.html",
                server_origin.port().expect("server port") + 1
            ))
            .expect("cross-origin document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (async () => {{
                            const response = await fetch({fetch_url_literal}, {{ mode: "no-cors" }});
                            const clone = response.clone();
                            const bodyUsedBefore = response.bodyUsed;
                            const text = await response.text();
                            const cloneText = await clone.text();
                            globalThis.__fetchObserved = JSON.stringify({{
                                type: response.type,
                                status: response.status,
                                ok: response.ok,
                                statusText: response.statusText,
                                url: response.url,
                                redirected: response.redirected,
                                bodyIsNull: response.body === null,
                                headers: Array.from(response.headers),
                                bodyUsedBefore,
                                bodyUsedAfter: response.bodyUsed,
                                text,
                                cloneType: clone.type,
                                cloneStatus: clone.status,
                                cloneBodyIsNull: clone.body === null,
                                cloneText,
                            }});
                            globalThis.__fetchDone = true;
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch no-cors should resolve",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch no-cors test should run on owner lane");

            let request = request_rx.await.expect("capture no-cors request");
            server.await.expect("no-cors fetch server should finish");
            assert!(request
                .to_ascii_lowercase()
                .contains("sec-fetch-mode: no-cors\r\n"));
            assert_eq!(
                observed,
                r#"{"type":"opaque","status":0,"ok":false,"statusText":"","url":"","redirected":false,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":true,"text":"","cloneType":"opaque","cloneStatus":0,"cloneBodyIsNull":true,"cloneText":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            let SubresourceNetworkOutcome::Success { status, .. } = record.outcome() else {
                panic!(
                    "expected no-cors network success, got {:?}",
                    record.outcome()
                );
            };
            assert_eq!(*status, 200);
        })
        .await;
}

#[tokio::test]
async fn window_fetch_no_cors_opaque_response_blocking_returns_empty_opaque_response() {
    run_page_vm_async_test(async move {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind no-cors ORB server");
            let addr = listener.local_addr().expect("no-cors ORB addr");
            let fetch_url = format!("http://{addr}/orb-data");
            let (request_tx, request_rx) = tokio::sync::oneshot::channel::<String>();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .expect("accept no-cors ORB request");
                let request = read_http_request_head(&mut stream)
                    .await
                    .expect("read no-cors ORB request");
                let _ = request_tx.send(request);
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"secret\":true}",
                    )
                    .await
                    .expect("write no-cors ORB response");
            });
            let document_url =
                Url::parse("http://127.0.0.1:1/page.html").expect("cross-origin document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            fetch({fetch_url_literal}, {{ mode: "no-cors" }}).then(
                                (response) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        fulfilled: true,
                                        type: response.type,
                                        status: response.status,
                                        url: response.url,
                                        bodyIsNull: response.body === null,
                                        headerCount: Array.from(response.headers).length,
                                    }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasOrbMessage: String(error && error.message).includes("OpaqueResponseBlocking"),
                                    }});
                                }}
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch no-cors ORB should settle",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch no-cors ORB test should run on owner lane");

            let request = request_rx.await.expect("capture no-cors ORB request");
            server.await.expect("no-cors ORB server should finish");
            assert!(request
                .to_ascii_lowercase()
                .contains("sec-fetch-mode: no-cors\r\n"));
            assert_eq!(
                observed,
                r#"{"fulfilled":true,"type":"opaque","status":0,"url":"","bodyIsNull":true,"headerCount":0}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == crate::network_host::ABORTED_ERROR_TEXT
            ));
        })
        .await;
}

#[tokio::test]
async fn window_fetch_no_cors_orb_allows_mislabeled_javascript_body() {
    run_page_vm_async_test(async move {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind no-cors ORB JS server");
            let addr = listener.local_addr().expect("no-cors ORB JS addr");
            let fetch_url = format!("http://{addr}/script-as-json");
            let (request_tx, request_rx) = tokio::sync::oneshot::channel::<String>();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .expect("accept no-cors ORB JS request");
                let request = read_http_request_head(&mut stream)
                    .await
                    .expect("read no-cors ORB JS request");
                let _ = request_tx.send(request);
                let body = b"\"use strict\";\nfunction fn() { return 42; }";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write no-cors ORB JS headers");
                stream
                    .write_all(body)
                    .await
                    .expect("write no-cors ORB JS body");
            });
            let document_url =
                Url::parse("http://127.0.0.1:1/page.html").expect("cross-origin document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let observed = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            fetch({fetch_url_literal}, {{ mode: "no-cors" }}).then(
                                (response) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        type: response.type,
                                        status: response.status,
                                        bodyIsNull: response.body === null,
                                    }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        rejected: true,
                                        message: String(error && error.message),
                                    }});
                                }}
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch no-cors ORB JS should resolve",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")
                })
                .await
                .expect("fetch no-cors ORB JS test should run on owner lane");

            let request = request_rx.await.expect("capture no-cors ORB JS request");
            server.await.expect("no-cors ORB JS server should finish");
            assert!(request
                .to_ascii_lowercase()
                .contains("sec-fetch-mode: no-cors\r\n"));
            assert_eq!(
                observed,
                r#"{"type":"opaque","status":0,"bodyIsNull":true}"#
            );
        })
        .await;
}

#[tokio::test]
async fn window_fetch_no_cors_cross_origin_resource_policy_blocks_response() {
    run_page_vm_async_test(async move {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind no-cors CORP server");
            let addr = listener.local_addr().expect("no-cors CORP addr");
            let fetch_url = format!("http://{addr}/corp-data");
            let (request_tx, request_rx) = tokio::sync::oneshot::channel::<String>();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .expect("accept no-cors CORP request");
                let request = read_http_request_head(&mut stream)
                    .await
                    .expect("read no-cors CORP request");
                let _ = request_tx.send(request);
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nCross-Origin-Resource-Policy: same-origin\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret",
                    )
                    .await
                    .expect("write no-cors CORP response");
            });
            let document_url =
                Url::parse("http://127.0.0.1:1/page.html").expect("cross-origin document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            fetch({fetch_url_literal}, {{ mode: "no-cors" }}).then(
                                () => {{
                                    globalThis.__fetchObserved = JSON.stringify({{ fulfilled: true }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasCorpMessage: String(error && error.message).includes("Cross-Origin-Resource-Policy"),
                                    }});
                                }}
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch no-cors CORP should reject",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch no-cors CORP test should run on owner lane");

            let request = request_rx.await.expect("capture no-cors CORP request");
            server.await.expect("no-cors CORP server should finish");
            assert!(request
                .to_ascii_lowercase()
                .contains("sec-fetch-mode: no-cors\r\n"));
            assert_eq!(
                observed,
                r#"{"name":"TypeError","isTypeError":true,"hasCorpMessage":true}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("Cross-Origin-Resource-Policy")
            ));
        })
        .await;
}

#[tokio::test]
async fn window_fetch_redirect_loop_rejects_and_records_network_failure() {
    run_page_vm_async_test(async move {
            let (base_url, server) = spawn_redirect_loop_http_server("/fetch-loop").await;
            let fetch_url = format!("{base_url}/fetch-loop");
            let document_url = Url::parse("http://127.0.0.1/page.html").expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            globalThis.__fetchObserved = null;
                            fetch({fetch_url_literal}).then(
                                () => {{
                                    globalThis.__fetchObserved = JSON.stringify({{ fulfilled: true }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasRedirectLimitMessage: String(error && error.message).includes("redirect limit exceeded"),
                                    }});
                                }},
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch redirect loop should reject",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch redirect-loop test should run on owner lane");

            server.await.expect("redirect-loop fetch server should finish");
            assert_eq!(
                observed,
                r#"{"name":"TypeError","isTypeError":true,"hasRedirectLimitMessage":true}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("redirect limit exceeded")
            ));
        })
        .await;
}

#[tokio::test]
async fn window_fetch_cross_origin_redirect_without_cors_rejects_and_records_failure() {
    run_page_vm_async_test(async move {
            let (source_base_url, _, source_server, target_server) =
                spawn_cross_origin_redirect_without_cors_http_servers(
                    "/fetch-cors-redirect-deny",
                    "/fetch-cors-denied-target",
                )
                .await;
            let fetch_url = format!("{source_base_url}/fetch-cors-redirect-deny");
            let document_url =
                Url::parse(&format!("{source_base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchDone = false;
                            globalThis.__fetchObserved = null;
                            fetch({fetch_url_literal}).then(
                                () => {{
                                    globalThis.__fetchObserved = JSON.stringify({{ fulfilled: true }});
                                }},
                                (error) => {{
                                    globalThis.__fetchObserved = JSON.stringify({{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasCorsMessage: String(error && error.message).includes("CORS check failed"),
                                    }});
                                }},
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch CORS redirect deny should reject",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch CORS redirect deny test should run on owner lane");

            source_server
                .await
                .expect("CORS redirect source server should finish");
            target_server
                .await
                .expect("CORS redirect target server should finish");
            assert_eq!(
                observed,
                r#"{"name":"TypeError","isTypeError":true,"hasCorsMessage":true}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
                panic!("expected CORS network failure, got {:?}", record.outcome());
            };
            assert_eq!(error_text, crate::network_host::FAILED_ERROR_TEXT);
        })
        .await;
}

#[tokio::test]
async fn window_fetch_document_csp_blocks_cross_origin_redirect_final_url() {
    run_page_vm_async_test(async move {
            let (source_base_url, target_base_url, source_server, target_server) =
                spawn_cross_origin_redirect_with_cors_http_servers(
                    "/fetch-csp-redirect-source",
                    "/fetch-csp-redirect-target",
                    "cors-allowed-target",
                )
                .await;
            let fetch_url = format!("{source_base_url}/fetch-csp-redirect-source");
            let target_url = format!("{target_base_url}/fetch-csp-redirect-target");
            let document_url =
                Url::parse(&format!("{source_base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            page_vm
                .vm_mut()
                .set_response_content_security_policies(&[String::from("connect-src 'self'")]);
            let local_executor = page_vm.local_executor.clone();
            let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__fetchCspEvents = [];
                            globalThis.__fetchDone = false;
                            globalThis.__fetchObserved = null;
                            self.addEventListener("securitypolicyviolation", event => {{
                                globalThis.__fetchCspEvents.push({{
                                    blockedURI: event.blockedURI,
                                    effectiveDirective: event.effectiveDirective,
                                    disposition: event.disposition,
                                    instance: event instanceof SecurityPolicyViolationEvent,
                                }});
                            }});
                            fetch({fetch_url_literal}).then(
                                response => response.text().then(text => {{
                                    globalThis.__fetchObserved = {{
                                        fulfilled: true,
                                        status: response.status,
                                        text,
                                        events: globalThis.__fetchCspEvents,
                                    }};
                                }}),
                                error => {{
                                    globalThis.__fetchObserved = {{
                                        name: error && error.name,
                                        isTypeError: error instanceof TypeError,
                                        hasCspMessage: String(error && error.message).includes("Content Security Policy"),
                                        events: globalThis.__fetchCspEvents,
                                    }};
                                }},
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__fetchDone === true)",
                        "fetch CSP redirect final URL should reject",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .drain_pre_domcontentloaded_content_security_policy_violation_tasks_for_test(),
                        1
                    );
                    let observed = page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__fetchObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("fetch CSP redirect test should run on owner lane");

            source_server
                .await
                .expect("fetch CSP redirect source server should finish");
            target_server
                .await
                .expect("fetch CSP redirect target server should finish");
            let observed: serde_json::Value =
                serde_json::from_str(&observed).expect("parse fetch CSP redirect observation");
            assert_eq!(
                observed,
                json!({
                    "name": "TypeError",
                    "isTypeError": true,
                    "hasCspMessage": true,
                    "events": [{
                        "blockedURI": target_url,
                        "effectiveDirective": "connect-src",
                        "disposition": "enforce",
                        "instance": true,
                    }],
                })
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), fetch_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Fetch);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("Content Security Policy")
            ));
        })
        .await;
}

#[tokio::test]
async fn window_fetch_document_csp_report_only_records_cross_origin_redirect_final_url() {
    run_page_vm_async_test(async move {
        let (source_base_url, target_base_url, source_server, target_server) =
            spawn_cross_origin_redirect_with_cors_http_servers(
                "/fetch-csp-report-redirect-source",
                "/fetch-csp-report-redirect-target",
                "cors-allowed-target",
            )
            .await;
        let fetch_url = format!("{source_base_url}/fetch-csp-report-redirect-source");
        let target_url = format!("{target_base_url}/fetch-csp-report-redirect-target");
        let document_url =
            Url::parse(&format!("{source_base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        page_vm
            .vm_mut()
            .set_response_content_security_report_only_policies(&[String::from(
                "connect-src 'self'",
            )]);
        let local_executor = page_vm.local_executor.clone();
        let fetch_url_literal = serde_json::to_string(&fetch_url).expect("serialize fetch url");

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__fetchReportEvents = [];
                            globalThis.__fetchDone = false;
                            globalThis.__fetchObserved = null;
                            self.addEventListener("securitypolicyviolation", event => {{
                                globalThis.__fetchReportEvents.push({{
                                    blockedURI: event.blockedURI,
                                    effectiveDirective: event.effectiveDirective,
                                    disposition: event.disposition,
                                    instance: event instanceof SecurityPolicyViolationEvent,
                                }});
                            }});
                            fetch({fetch_url_literal}).then(
                                response => response.text().then(text => {{
                                    globalThis.__fetchObserved = {{
                                        status: response.status,
                                        text,
                                        events: globalThis.__fetchReportEvents,
                                    }};
                                }}),
                                error => {{
                                    globalThis.__fetchObserved = {{
                                        rejected: error && error.name,
                                        events: globalThis.__fetchReportEvents,
                                    }};
                                }},
                            ).finally(() => {{
                                globalThis.__fetchDone = true;
                            }});
                        }})()
                        "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__fetchDone === true)",
                    "fetch CSP report-only redirect final URL should resolve",
                )
                .await?;
                while page_vm
                    .run_exact_page_websocket_selected_task_for_test().await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .drain_pre_domcontentloaded_content_security_policy_violation_tasks_for_test(),
                    1
                );
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__fetchObserved)")
            })
            .await
            .expect("fetch CSP report-only redirect test should run on owner lane");

        source_server
            .await
            .expect("fetch CSP report-only redirect source server should finish");
        target_server
            .await
            .expect("fetch CSP report-only redirect target server should finish");
        let observed: serde_json::Value = serde_json::from_str(&observed)
            .expect("parse fetch CSP report-only redirect observation");
        assert_eq!(
            observed,
            json!({
                "status": 200,
                "text": "cors-allowed-target",
                "events": [{
                    "blockedURI": target_url,
                    "effectiveDirective": "connect-src",
                    "disposition": "report",
                    "instance": true,
                }],
            })
        );
    })
    .await;
}

#[tokio::test]
async fn window_fetch_document_csp_report_uri_posts_violation_body() {
    run_page_vm_async_test(async move {
        let (report_base_url, report_rx, report_server) = spawn_request_capture_http_server().await;
        let document_url =
            Url::parse(&format!("{report_base_url}/page.html")).expect("document url");
        let blocked_url = format!("{report_base_url}/blocked-data");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        page_vm
            .vm_mut()
            .set_response_content_security_policies(&[String::from(
                "connect-src 'none'; report-uri /csp-report",
            )]);
        let local_executor = page_vm.local_executor.clone();
        let blocked_url_literal = serde_json::to_string(&blocked_url).expect("serialize URL");

        let request = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__fetchReportDone = false;
                        fetch({blocked_url_literal}).catch(() => {{
                            globalThis.__fetchReportDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__fetchReportDone === true)",
                    "fetch CSP report-uri rejection should settle",
                )
                .await?;
                let request = tokio::time::timeout(Duration::from_secs(5), report_rx)
                    .await
                    .expect("timed out waiting for CSP report")
                    .expect("CSP report capture channel closed");
                Ok::<_, anyhow::Error>(request)
            })
            .await
            .expect("fetch CSP report-uri test should run on owner lane");

        report_server
            .await
            .expect("CSP report capture server should finish");
        assert!(request.starts_with("POST /csp-report HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/csp-report")
        );
        let body = request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("captured request should contain body");
        let body: serde_json::Value =
            serde_json::from_str(body).expect("CSP report body should be JSON");
        assert_eq!(
            body["csp-report"]["document-uri"],
            format!("{report_base_url}/page.html")
        );
        assert_eq!(body["csp-report"]["blocked-uri"], blocked_url);
        assert_eq!(body["csp-report"]["effective-directive"], "connect-src");
        assert_eq!(body["csp-report"]["violated-directive"], "connect-src");
        assert_eq!(body["csp-report"]["disposition"], "enforce");
    })
    .await;
}

#[tokio::test]
async fn xhr_connection_refused_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let (base_url, server) =
                spawn_connection_drop_http_server("/xhr-connection-refused").await;
            let xhr_url = format!("{base_url}/xhr-connection-refused");
            let document_url = Url::parse("http://127.0.0.1/page.html").expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr connection failure should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("xhr connection failure test should run on owner lane");

            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
            ));
            server
                .await
                .expect("connection-drop xhr server should finish");
        })
        .await;
}

#[tokio::test]
async fn window_xhr_file_url_rejects_before_interception_or_transport() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://example.test/page.html").unwrap();
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (observed, pending_count, network_output) = local_executor
            .run(async move {
                page_vm.vm_mut().set_fetch_subresource_interception(
                    true,
                    Some(SubresourceResourceType::Xhr),
                );
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const events = [];
                        globalThis.__fileXhrDone = false;
                        globalThis.__fileXhrObserved = null;
                        const xhr = new XMLHttpRequest();
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        xhr.onloadstart = () => events.push("loadstart");
                        xhr.onerror = () => events.push("error");
                        xhr.onload = () => events.push("load");
                        xhr.onloadend = () => {
                            events.push("loadend");
                            globalThis.__fileXhrObserved = JSON.stringify({
                                events,
                                readyState: xhr.readyState,
                                status: xhr.status,
                                responseURL: xhr.responseURL,
                                responseText: xhr.responseText,
                            });
                            globalThis.__fileXhrDone = true;
                        };
                        xhr.open("GET", "file:///moli-policy-must-not-open");
                        xhr.send();
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__fileXhrDone === true)",
                    "file URL XHR should fail before interception",
                )
                .await?;
                let observed = page_vm.vm_mut().eval("String(globalThis.__fileXhrObserved)")?;
                let pending_count = page_vm
                    .vm_mut()
                    .take_pending_subresource_fetch_infos()
                    .len();
                Ok::<_, anyhow::Error>((
                    observed,
                    pending_count,
                    page_vm.vm_mut().take_network_output(),
                ))
            })
            .await
            .expect("file URL XHR test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"responseURL":"","responseText":""}"#
        );
        assert_eq!(pending_count, 0, "unsupported schemes must not reach XHR interception");
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Xhr);
        assert_eq!(
            records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "URL scheme \"file\" is not supported.".to_owned(),
            }
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_file_url_throws_network_error_without_progress_events() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://example.test/page.html").unwrap();
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (observed, pending_count, network_output) = local_executor
            .run(async move {
                page_vm.vm_mut().set_fetch_subresource_interception(
                    true,
                    Some(SubresourceResourceType::Xhr),
                );
                let observed = page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const events = [];
                        const xhr = new XMLHttpRequest();
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        xhr.onloadstart = () => events.push("loadstart");
                        xhr.onerror = () => events.push("error");
                        xhr.onloadend = () => events.push("loadend");
                        xhr.open("GET", "file:///moli-policy-must-not-open", false);
                        let error = null;
                        try {
                            xhr.send();
                        } catch (caught) {
                            error = {
                                name: caught && caught.name,
                                message: caught && caught.message,
                                isDomException: caught instanceof DOMException,
                            };
                        }
                        return JSON.stringify({
                            error,
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                        });
                    })()
                    "#,
                )?;
                let pending_count = page_vm
                    .vm_mut()
                    .take_pending_subresource_fetch_infos()
                    .len();
                Ok::<_, anyhow::Error>((
                    observed,
                    pending_count,
                    page_vm.vm_mut().take_network_output(),
                ))
            })
            .await
            .expect("synchronous file URL XHR test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'file:///moli-policy-must-not-open'.","isDomException":true},"events":["readystatechange:1"],"readyState":4,"status":0}"#
        );
        assert_eq!(pending_count, 0);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "URL scheme \"file\" is not supported.".to_owned(),
            }
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_bad_port_throws_network_error_without_progress_events() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://example.test/page.html").unwrap();
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const events = [];
                        const xhr = new XMLHttpRequest();
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        for (const type of ["loadstart", "error", "timeout", "loadend"]) {
                            xhr.addEventListener(type, () => events.push(type));
                            xhr.upload.addEventListener(type, () => events.push("upload." + type));
                        }
                        xhr.open("POST", "http://example.test:1/", false);
                        let error = null;
                        try {
                            xhr.send("body");
                        } catch (caught) {
                            error = {
                                name: caught && caught.name,
                                message: caught && caught.message,
                                isDomException: caught instanceof DOMException,
                            };
                        }
                        return JSON.stringify({
                            error,
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                            statusText: xhr.statusText,
                            responseText: xhr.responseText,
                            responseURL: xhr.responseURL,
                        });
                    })()
                    "#,
                )?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous bad-port XHR test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'http://example.test:1/'.","isDomException":true},"events":["readystatechange:1"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":""}"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Xhr);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text == "xhr: blocked bad port for `http://example.test:1/`"
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_connection_reset_throws_without_progress_events() {
    run_page_vm_async_test(async move {
        let (base_url, server) =
            spawn_blocking_connection_drop_http_server("/sync-xhr-connection-reset");
        let xhr_url = format!("{base_url}/sync-xhr-connection-reset");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous connection-reset XHR probe should run on owner lane");

        server
            .join()
            .expect("synchronous connection-reset XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].url().as_str(), xhr_url);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Xhr);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_malformed_data_url_throws_without_progress_events() {
    run_page_vm_async_test(async move {
        // Ported from WPT xhr/send-network-error-sync-events.sub.htm and
        // calibrated against Debian Chromium 145.0.7632.116.
        let xhr_url = "data:text/html;charset=utf-8;base64,PT0NUWVBFIGh0bWw%2BDQo8";
        let document_url = Url::parse("https://example.test/page.html").expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous malformed-data XHR probe should run on owner lane");

        assert_synchronous_xhr_network_error_surface(&observed, xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].url().as_str(), xhr_url);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_rejects_unsupported_redirect_schemes() {
    run_page_vm_async_test(async move {
        // Ported from WPT xhr/send-redirect-bogus-sync.sub.htm. Network-host
        // cases are represented separately by the bad-port/reset tests.
        for (path, location) in [
            ("/sync-xhr-redirect-foobar", "foobar://abcd"),
            ("/sync-xhr-redirect-mailto", "mailto:someone@example.org"),
            ("/sync-xhr-redirect-tel", "tel:1234567890"),
        ] {
            let (base_url, server) = spawn_blocking_single_redirect_http_server(path, location);
            let xhr_url = format!("{base_url}{path}");
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

            let (observed, network_output) = local_executor
                .run(async move {
                    let observed = page_vm.vm_mut().eval(&expression)?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("synchronous redirect-scheme XHR probe should run on owner lane");

            server
                .join()
                .expect("synchronous redirect-scheme XHR server should finish");
            assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].url().as_str(), xhr_url);
            assert!(matches!(
                records[0].outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("not supported")
                        || error_text.contains("HTTP(S)")
                        || error_text.contains("redirect")
            ));
        }
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_redirect_loop_throws_without_progress_events() {
    run_page_vm_async_test(async move {
        let (base_url, server) =
            spawn_blocking_redirect_loop_http_server("/sync-xhr-redirect-loop");
        let xhr_url = format!("{base_url}/sync-xhr-redirect-loop");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous redirect-loop XHR probe should run on owner lane");

        server
            .join()
            .expect("synchronous redirect-loop XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text.contains("redirect limit exceeded")
        ));
    })
    .await;
}

#[tokio::test]
async fn xhr_dns_failure_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let xhr_url = "http://moli-dns-failure.invalid./xhr-dns-failure";
            let mut page_vm = test_page_vm_with_config(dns_failure_fetch_config(), Vec::new());
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr DNS failure should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("xhr DNS failure test should run on owner lane");

            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
                panic!("expected DNS XHR failure, got {:?}", record.outcome());
            };
            assert!(
                error_text.to_ascii_lowercase().contains("resolv"),
                "expected DNS-resolution error text, got {error_text:?}"
            );
        })
        .await;
}

#[tokio::test]
async fn xhr_redirect_loop_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let (base_url, server) = spawn_redirect_loop_http_server("/xhr-loop").await;
            let xhr_url = format!("{base_url}/xhr-loop");
            let document_url = Url::parse("http://127.0.0.1/page.html").expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr redirect loop should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("xhr redirect-loop test should run on owner lane");

            server.await.expect("redirect-loop xhr server should finish");
            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("redirect limit exceeded")
            ));
        })
        .await;
}

#[tokio::test]
async fn xhr_cross_origin_redirect_without_cors_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let (source_base_url, _, source_server, target_server) =
                spawn_cross_origin_redirect_without_cors_http_servers(
                    "/xhr-cors-redirect-deny",
                    "/xhr-cors-denied-target",
                )
                .await;
            let xhr_url = format!("{source_base_url}/xhr-cors-redirect-deny");
            let document_url =
                Url::parse(&format!("{source_base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "XHR CORS redirect deny should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("XHR CORS redirect deny test should run on owner lane");

            source_server
                .await
                .expect("XHR CORS redirect source server should finish");
            target_server
                .await
                .expect("XHR CORS redirect target server should finish");
            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == crate::network_host::FAILED_ERROR_TEXT
            ));
        })
        .await;
}

#[tokio::test]
async fn xhr_document_csp_blocks_cross_origin_redirect_final_url() {
    run_page_vm_async_test(async move {
            let (source_base_url, target_base_url, source_server, target_server) =
                spawn_cross_origin_redirect_with_cors_http_servers(
                    "/xhr-csp-redirect-source",
                    "/xhr-csp-redirect-target",
                    "cors-allowed-xhr-target",
                )
                .await;
            let xhr_url = format!("{source_base_url}/xhr-csp-redirect-source");
            let target_url = format!("{target_base_url}/xhr-csp-redirect-target");
            let document_url =
                Url::parse(&format!("{source_base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            page_vm
                .vm_mut()
                .set_response_content_security_policies(&[String::from("connect-src 'self'")]);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrCspEvents = [];
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            self.addEventListener("securitypolicyviolation", event => {{
                                globalThis.__xhrCspEvents.push({{
                                    blockedURI: event.blockedURI,
                                    effectiveDirective: event.effectiveDirective,
                                    disposition: event.disposition,
                                    instance: event instanceof SecurityPolicyViolationEvent,
                                }});
                            }});
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = {{
                                    events: globalThis.__xhrEvents,
                                    cspEvents: globalThis.__xhrCspEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                }};
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "XHR CSP redirect final URL should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .drain_pre_domcontentloaded_content_security_policy_violation_tasks_for_test(),
                        1
                    );
                    let observed = page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("XHR CSP redirect test should run on owner lane");

            source_server
                .await
                .expect("XHR CSP redirect source server should finish");
            target_server
                .await
                .expect("XHR CSP redirect target server should finish");
            let observed: serde_json::Value =
                serde_json::from_str(&observed).expect("parse XHR CSP redirect observation");
            assert_eq!(
                observed,
                json!({
                    "events": [
                        "readystatechange:1",
                        "loadstart",
                        "readystatechange:4",
                        "error",
                        "loadend",
                    ],
                    "cspEvents": [{
                        "blockedURI": target_url,
                        "effectiveDirective": "connect-src",
                        "disposition": "enforce",
                        "instance": true,
                    }],
                    "readyState": 4,
                    "status": 0,
                    "statusText": "",
                    "responseText": "",
                    "responseURL": "",
                })
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("Content Security Policy")
            ));
        })
        .await;
}

#[tokio::test]

async fn worker_url_terminate_while_loading_drops_queued_messages_and_late_script_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 200 OK",
            r#"
            postMessage("loaded");
            onmessage = (event) => {
                postMessage(`pong:${event.data}`);
            };
            "#
            .to_owned(),
            Duration::from_millis(75),
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        const worker = new Worker("/worker.js");
                        worker.onmessage = (event) => {
                            globalThis.__workerEvents.push(`message:${event.data}`);
                        };
                        worker.onerror = (event) => {
                            globalThis.__workerEvents.push(`error:${event.message}`);
                        };
                        worker.postMessage("queued-before-terminate");
                        worker.terminate();
                    })()
                    "#,
                )?;

                tokio::time::sleep(Duration::from_millis(150)).await;
                for _ in 0..8 {
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_some()
                    {}
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }

                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker terminate-while-loading test should run on owner lane");

        server.abort();
        assert_eq!(events, r#"[]"#);
    })
    .await;
}

#[tokio::test]
async fn worker_url_fetch_failure_dispatches_error_event() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let error_message = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerError = null;
                        globalThis.__workerDone = false;
                        const worker = new Worker("/missing-worker.js");
                        worker.onerror = (event) => {
                            globalThis.__workerError = event.message;
                            globalThis.__workerDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker url load failure should dispatch an error event",
                )
                .await?;
                page_vm.vm_mut().eval("String(globalThis.__workerError)")
            })
            .await
            .expect("worker url failure test should run on owner lane");

        server
            .await
            .expect("worker script failure server should finish");
        assert!(
            error_message.contains("HTTP request")
                && error_message.contains("404")
                && error_message.contains("/missing-worker.js"),
            "unexpected worker load error: {error_message}"
        );
    })
    .await;
}

#[tokio::test]
async fn worker_url_fetch_failure_notifies_onerror_and_error_listener_in_registration_order() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        globalThis.__workerDone = false;
                        const worker = new Worker("/missing-worker.js");
                        worker.addEventListener("error", () => {
                            globalThis.__workerEvents.push("listener");
                            if (globalThis.__workerEvents.length >= 2) {
                                globalThis.__workerDone = true;
                            }
                        });
                        worker.onerror = () => {
                            globalThis.__workerEvents.push("onerror");
                            if (globalThis.__workerEvents.length >= 2) {
                                globalThis.__workerDone = true;
                            }
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker url load failure should notify both error surfaces",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker dual error surface test should run on owner lane");

        server
            .await
            .expect("worker dual error server should finish");
        assert_eq!(events, r#"["listener","onerror"]"#);
    })
    .await;
}

#[tokio::test]
async fn worker_url_fetch_failure_keeps_onerror_position_when_assigned_first() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        globalThis.__workerDone = false;
                        const worker = new Worker("/missing-worker.js");
                        worker.onerror = () => {
                            globalThis.__workerEvents.push("onerror");
                            if (globalThis.__workerEvents.length >= 2) {
                                globalThis.__workerDone = true;
                            }
                        };
                        worker.addEventListener("error", () => {
                            globalThis.__workerEvents.push("listener");
                            if (globalThis.__workerEvents.length >= 2) {
                                globalThis.__workerDone = true;
                            }
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker url load failure should keep onerror registration position",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker ordered onerror test should run on owner lane");

        server
            .await
            .expect("worker ordered onerror server should finish");
        assert_eq!(events, r#"["onerror","listener"]"#);
    })
    .await;
}

#[tokio::test]
async fn worker_runtime_error_propagates_to_window_onerror_with_matching_fields() {
    run_page_vm_async_test(async move {
            let (base_url, server) = spawn_single_response_http_server(
                "HTTP/1.1 200 OK",
                r#"throw new TypeError("worker-boom");"#.to_owned(),
                Duration::ZERO,
            )
            .await;
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                    (() => {
                        globalThis.__workerError = null;
                        globalThis.__windowError = null;
                        globalThis.__windowErrorEvent = null;
                        globalThis.__workerDone = false;
                        window.onerror = (message, filename, lineno, colno, error, sixth) => {
                            globalThis.__windowError = {
                                message,
                                filename,
                                linenoType: typeof lineno,
                                colnoType: typeof colno,
                                errorIsNull: error === null,
                                errorType: error && error.constructor && error.constructor.name,
                                errorMessage: error && error.message,
                                errorStackHasWorkerUrl: !!(error && error.stack && error.stack.includes("/worker-error.js")),
                                sameErrorObject: error === globalThis.__workerErrorObject,
                                sixthUndefined: sixth === undefined,
                            };
                            globalThis.__workerDone = true;
                            return true;
                        };
                        window.addEventListener("error", event => {
                            globalThis.__windowErrorEvent = {
                                isErrorEvent: event instanceof ErrorEvent,
                                typeString: Object.prototype.toString.call(event),
                                message: event.message,
                                filename: event.filename,
                                linenoType: typeof event.lineno,
                                colnoType: typeof event.colno,
                                errorIsNull: event.error === null,
                                errorType: event.error && event.error.constructor && event.error.constructor.name,
                                errorMessage: event.error && event.error.message,
                                errorStackHasWorkerUrl: !!(event.error && event.error.stack && event.error.stack.includes("/worker-error.js")),
                                sameErrorObject: event.error === globalThis.__workerErrorObject,
                                defaultPrevented: event.defaultPrevented
                            };
                        });
                        const worker = new Worker("/worker-error.js");
                        worker.onerror = (event, second, third) => {
                            globalThis.__workerErrorObject = event.error;
                            globalThis.__workerError = {
                                typeString: Object.prototype.toString.call(event),
                                message: event.message,
                                filename: event.filename,
                                linenoType: typeof event.lineno,
                                colnoType: typeof event.colno,
                                errorIsNull: event.error === null,
                                errorType: event.error && event.error.constructor && event.error.constructor.name,
                                errorMessage: event.error && event.error.message,
                                errorStackHasWorkerUrl: !!(event.error && event.error.stack && event.error.stack.includes("/worker-error.js")),
                                extraArgsUndefined: second === undefined && third === undefined,
                            };
                        };
                    })()
                    "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerDone === true)",
                        "worker runtime error should propagate to window.onerror",
                    )
                    .await?;
                    page_vm.vm_mut().eval(
                        "JSON.stringify({worker: globalThis.__workerError, window: globalThis.__windowError, windowEvent: globalThis.__windowErrorEvent})",
                    )
                })
                .await
                .expect("worker runtime error propagation test should run on owner lane");

            server
                .await
                .expect("worker runtime error server should finish");
            assert!(
                result.contains(r#""typeString":"[object ErrorEvent]""#),
                "result: {result}"
            );
            assert!(
                result.contains(r#""message":"Uncaught TypeError: worker-boom""#),
                "result: {result}"
            );
            assert!(
                result.contains(&format!(r#""filename":"{base_url}/worker-error.js""#)),
                "result: {result}"
            );
            assert!(
                result.contains(r#""linenoType":"number""#)
                    && result.contains(r#""colnoType":"number""#),
                "result: {result}"
            );
            assert!(
                result.contains(r#""errorIsNull":true"#),
                "worker object error event should not expose the worker exception object: {result}"
            );
            assert!(
                result.contains(r#""errorIsNull":true"#)
                    && result.contains(r#""sameErrorObject":true"#)
                    && result.contains(r#""extraArgsUndefined":true"#)
                    && result.contains(r#""sixthUndefined":true"#),
                "result: {result}"
            );
            assert!(
                result.contains(r#""isErrorEvent":true"#)
                    && result.contains(r#""typeString":"[object ErrorEvent]""#)
                    && result.contains(r#""defaultPrevented":true"#),
                "window listener should receive a cancelable ErrorEvent: {result}"
            );
        })
        .await;
}

#[tokio::test]
async fn worker_runtime_error_prevent_default_suppresses_window_onerror() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 200 OK",
            r#"throw new Error("worker-boom");"#.to_owned(),
            Duration::ZERO,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerDone = false;
                        globalThis.__windowErrorCalled = false;
                        window.onerror = () => {
                            globalThis.__windowErrorCalled = true;
                            return true;
                        };
                        const worker = new Worker("/worker-error.js");
                        worker.onerror = (event) => {
                            event.preventDefault();
                            globalThis.__workerDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "handled worker error should not reach window.onerror",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__windowErrorCalled)")
            })
            .await
            .expect("worker handled error suppression test should run on owner lane");

        server
            .await
            .expect("worker handled error server should finish");
        assert_eq!(result, "false");
    })
    .await;
}

#[tokio::test]
async fn worker_runtime_error_promise_reaction_can_prevent_window_propagation() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 200 OK",
            r#"throw new Error("worker-boom");"#.to_owned(),
            Duration::ZERO,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerDone = false;
                        globalThis.__windowErrorCalled = false;
                        window.onerror = () => {
                            globalThis.__windowErrorCalled = true;
                            return true;
                        };
                        const worker = new Worker("/worker-error.js");
                        new Promise(resolve => {
                            worker.onerror = resolve;
                        }).then(event => {
                            event.preventDefault();
                            globalThis.__workerDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker error Promise reaction should run",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__windowErrorCalled)")
            })
            .await
            .expect("worker Promise error suppression test should run on owner lane");

        server
            .await
            .expect("worker Promise error server should finish");
        assert_eq!(result, "false");
    })
    .await;
}

#[tokio::test]
async fn worker_runtime_error_return_truthy_suppresses_window_onerror() {
    run_page_vm_async_test(async move {
            let (base_url, server) = spawn_single_response_http_server(
                "HTTP/1.1 200 OK",
                r#"throw new Error("worker-boom");"#.to_owned(),
                Duration::ZERO,
            )
            .await;
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                    (() => {
                        globalThis.__workerDone = false;
                        globalThis.__windowErrorCalled = false;
                        globalThis.__listenerSawDefaultPrevented = false;
                        window.onerror = () => {
                            globalThis.__windowErrorCalled = true;
                            return true;
                        };
                        const worker = new Worker("/worker-error.js");
                        worker.onerror = () => {
                            globalThis.__workerDone = true;
                            return 1;
                        };
                        worker.addEventListener("error", event => {
                            globalThis.__listenerSawDefaultPrevented = event.defaultPrevented;
                        });
                    })()
                    "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerDone === true)",
                        "truthy worker onerror should suppress window.onerror",
                    )
                    .await?;
                    page_vm.vm_mut().eval(
                        "JSON.stringify({windowErrorCalled: globalThis.__windowErrorCalled, listenerSawDefaultPrevented: globalThis.__listenerSawDefaultPrevented})",
                    )
                })
                .await
                .expect("worker truthy onerror suppression test should run on owner lane");

            server
                .await
                .expect("worker truthy onerror suppression server should finish");
            assert_eq!(
                result,
                r#"{"windowErrorCalled":false,"listenerSawDefaultPrevented":true}"#
            );
        })
        .await;
}

#[tokio::test]
async fn worker_importscripts_loads_relative_scripts_like_chromium_classic_worker() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/worker.js",
                "HTTP/1.1 200 OK",
                r#"
                importScripts("dep.js");
                postMessage(`main:${globalThis.__depLoaded}`);
                "#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/dep.js",
                "HTTP/1.1 200 OK",
                r#"
                globalThis.__depLoaded = "ok";
                postMessage("dep");
                "#
                .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        globalThis.__workerDone = false;
                        const worker = new Worker("/worker.js");
                        worker.onmessage = (event) => {
                            globalThis.__workerEvents.push(event.data);
                            if (globalThis.__workerEvents.length >= 2) {
                                globalThis.__workerDone = true;
                            }
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker importScripts should load relative dependency",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker importScripts test should run on owner lane");

        server
            .await
            .expect("worker importScripts server should finish");
        assert_eq!(events, r#"["dep","main:ok"]"#);
    })
    .await;
}

#[tokio::test]
async fn worker_importscripts_keeps_prior_side_effects_before_later_fetch_failure() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/worker.js",
                "HTTP/1.1 200 OK",
                r#"
                try {
                    importScripts("first.js", "missing.js", "second.js");
                    postMessage("unexpected-success");
                } catch (error) {
                    postMessage(`caught:${globalThis.__firstLoaded}:${globalThis.__secondLoaded}:${error.message}`);
                }
                "#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/first.js",
                "HTTP/1.1 200 OK",
                r#"
                globalThis.__firstLoaded = "yes";
                postMessage("first");
                "#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/missing.js",
                "HTTP/1.1 404 Not Found",
                "missing".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        globalThis.__workerDone = false;
                        const worker = new Worker("/worker.js");
                        worker.onmessage = (event) => {
                            globalThis.__workerEvents.push(event.data);
                            if (globalThis.__workerEvents.length >= 2) {
                                globalThis.__workerDone = true;
                            }
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker importScripts should preserve earlier side effects before later failure",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker importScripts failure test should run on owner lane");

        server
            .await
            .expect("worker importScripts failure server should finish");
        assert!(
            events.contains(r#""first""#),
            "expected first imported script side effect, got {events}"
        );
        assert!(
            events.contains("caught:yes:undefined:HTTP request"),
            "expected caught importScripts failure with preserved first side effect, got {events}"
        );
        })
        .await;
}

#[tokio::test]
async fn worker_blob_url_loads_script_like_chromium() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        globalThis.__workerDone = false;
                        const blob = new Blob(['postMessage("worker_OK")']);
                        const url = URL.createObjectURL(blob);
                        const worker = new Worker(url);
                        worker.onmessage = (event) => {
                            globalThis.__workerEvents.push(event.data);
                            globalThis.__workerDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker should load from blob URL",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker blob url test should run on owner lane");

        assert_eq!(events, r#"["worker_OK"]"#);
    })
    .await;
}

#[tokio::test]
async fn worker_blob_url_survives_immediate_revoke_like_chromium() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerEvents = [];
                        globalThis.__workerDone = false;
                        const blob = new Blob(['postMessage("worker_OK")']);
                        const url = URL.createObjectURL(blob);
                        const worker = new Worker(url);
                        URL.revokeObjectURL(url);
                        worker.onmessage = (event) => {
                            globalThis.__workerEvents.push(event.data);
                            globalThis.__workerDone = true;
                        };
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__workerDone === true)",
                    "worker should load from blob URL after immediate revoke",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__workerEvents)")
            })
            .await
            .expect("worker revoked blob url test should run on owner lane");

        assert_eq!(events, r#"["worker_OK"]"#);
    })
    .await;
}
