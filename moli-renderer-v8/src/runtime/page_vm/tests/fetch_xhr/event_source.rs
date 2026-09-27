use super::*;

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
