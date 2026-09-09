use super::*;

#[tokio::test]
async fn websocket_text_echo_delivers_browser_style_async_events() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsEvents = [`constructed:${{socket.readyState}}`];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            globalThis.__wsEvents.push(`open:${{socket.readyState}}:${{socket.protocol}}`);
                            socket.send('hello');
                            globalThis.__wsEvents.push(`bufferedAfterSend:${{socket.bufferedAmount}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data}}:${{socket.bufferedAmount}}`);
                            socket.close(1000, 'done');
                            globalThis.__wsEvents.push(`afterCloseCall:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;

                for _ in 0..20 {
                    while page_vm.run_exact_page_websocket_selected_task_for_test().await?.is_some() {}
                    if page_vm.vm_mut().eval("String(globalThis.__wsDone === true)")? == "true" {
                        break;
                    }
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(1),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await
                    .unwrap_or(false);
                    assert!(arrived, "websocket runtime event should arrive");
                }
                while page_vm.run_exact_page_websocket_selected_task_for_test().await?.is_some() {}
                assert_eq!(
                    page_vm.vm_mut().eval("String(globalThis.__wsDone === true)")?,
                    "true"
                );
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("websocket echo test should run on owner lane");

        server.await.expect("websocket echo server should finish");
        assert_eq!(
            events,
            r#"["constructed:0","open:1:","bufferedAfterSend:5","message:hello:0","afterCloseCall:2","close:3:1000:done:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn main_document_open_preserves_websocket_execution_context() {
    run_page_vm_async_test(async move {
        let (url, opened_rx, message_tx, server) = spawn_triggered_text_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (retired_owner, current_owner, events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
                    globalThis.__mainOwnerWebSocketEvents = [];
                    globalThis.__mainOwnerWebSocket = new WebSocket({url_literal});
                    __mainOwnerWebSocket.onmessage = event =>
                      __mainOwnerWebSocketEvents.push(event.data);
                    "#
                ))?;
                tokio::time::timeout(Duration::from_secs(2), opened_rx)
                    .await
                    .expect("main WebSocket server should accept")
                    .expect("main WebSocket server open signal");
                let deadline = Instant::now() + Duration::from_secs(2);
                while page_vm
                    .vm_mut()
                    .eval("String(__mainOwnerWebSocket.readyState)")?
                    != "1"
                {
                    if page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_none()
                    {
                        let _ = tokio::time::timeout(
                            Duration::from_millis(100),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    assert!(Instant::now() < deadline, "main WebSocket should open");
                }
                let retired_owner = page_vm
                    .vm_mut()
                    .current_main_document_task_owner()
                    .expect("main owner before document.open");
                page_vm
                    .vm_mut()
                    .eval("document.open(); document.close(); 'replaced'")?;
                let current_owner = page_vm
                    .vm_mut()
                    .current_main_document_task_owner()
                    .expect("main owner after document.open");
                message_tx
                    .send("after-document-open".to_owned())
                    .expect("trigger main WebSocket message");

                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_some()
                    {}
                    let events = page_vm
                        .vm_mut()
                        .eval("__mainOwnerWebSocketEvents.join('|')")?;
                    if events == "after-document-open" {
                        break Ok::<_, anyhow::Error>((retired_owner, current_owner, events));
                    }
                    let _ = tokio::time::timeout(
                        Duration::from_millis(100),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await;
                    assert!(
                        Instant::now() < deadline,
                        "same execution-context WebSocket message should arrive after document.open"
                    );
                }
            })
            .await
            .expect("main document.open WebSocket proof should run");

        server.await.expect("triggered main WebSocket server");
        assert_ne!(retired_owner.document_id, current_owner.document_id);
        assert_eq!(retired_owner.local_window_id, current_owner.local_window_id);
        assert_eq!(events, "after-document-open");
    })
    .await;
}

#[tokio::test]
async fn child_navigation_retires_websocket_execution_context() {
    run_page_vm_async_test(async move {
        let (url, opened_rx, message_tx, server) = spawn_triggered_text_websocket_server().await;
        let (replacement_url, replacement_opened_rx, replacement_message_tx, replacement_server) =
            spawn_triggered_text_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let replacement_url_literal =
            serde_json::to_string(&replacement_url).expect("serialize replacement websocket url");
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__retiredChildWebSocketEvents = [];
                    const frame = document.createElement("iframe");
                    globalThis.__retiredChildWebSocketFrame = frame;
                    (document.body || document.documentElement).appendChild(frame);
                    void frame.contentWindow.WebSocket;
                    "#,
                )?;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "synchronous initial about:blank WebSocket realm",
                )
                .await;
                assert_eq!(
                    page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await,
                    None,
                    "materialized synchronous initial about:blank must not leave child task work"
                );
                let initial_context_id = page_vm
                    .vm_mut()
                    .live_child_default_runtime_realm_inventory()
                    .into_iter()
                    .next()
                    .expect("initial child WebSocket realm")
                    .context_id;
                page_vm
                    .vm_mut()
                    .eval("__retiredChildWebSocketFrame.srcdoc = '<p>committed</p>'; 'queued'")?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "first child WebSocket document",
                )
                .await;
                run_child_interactive_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "first child WebSocket document",
                )
                .await;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .live_child_default_runtime_realm_inventory()
                        .into_iter()
                        .next()
                        .expect("committed child WebSocket realm")
                        .context_id,
                    initial_context_id,
                    "the first secure commit must preserve the initial-empty LocalWindow realm"
                );

                page_vm.vm_mut().eval(&format!(
                    r#"
                    const ChildWebSocket =
                      __retiredChildWebSocketFrame.contentWindow.WebSocket;
                    globalThis.__retiredChildWebSocketConstructor = ChildWebSocket;
                    globalThis.__retiredChildWebSocketStreamConstructor =
                      __retiredChildWebSocketFrame.contentWindow.WebSocketStream;
                    globalThis.__retiredChildWebSocketUrl = {url_literal};
                    globalThis.__retiredChildWebSocket = new ChildWebSocket({url_literal});
                    __retiredChildWebSocket.onmessage = event =>
                      __retiredChildWebSocketEvents.push(event.data);
                    "#
                ))?;
                tokio::time::timeout(Duration::from_secs(2), opened_rx)
                    .await
                    .expect("child WebSocket server should accept")
                    .expect("child WebSocket server open signal");
                let deadline = Instant::now() + Duration::from_secs(2);
                while page_vm
                    .vm_mut()
                    .eval("String(__retiredChildWebSocket.readyState)")?
                    != "1"
                {
                    if page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_none()
                    {
                        let _ = tokio::time::timeout(
                            Duration::from_millis(100),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    assert!(Instant::now() < deadline, "child WebSocket should open");
                }

                page_vm
                    .vm_mut()
                    .eval("__retiredChildWebSocketFrame.srcdoc = '<p>replacement</p>'; 'queued'")?;
                assert_eq!(
                    page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await,
                    Some(ChildFrameSemanticTurnKind::NavigationCommit),
                    "child replacement should rotate the LocalWindow before the server message"
                );
                assert_eq!(
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                          try {
                            new __retiredChildWebSocketConstructor(__retiredChildWebSocketUrl);
                            return "constructed";
                          } catch (error) {
                            return error.name;
                          }
                        })()
                        "#,
                    )?,
                    "TypeError",
                    "constructor captured by the retired child LocalWindow must fail closed"
                );
                assert_eq!(
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                          try {
                            new __retiredChildWebSocketStreamConstructor(
                              __retiredChildWebSocketUrl
                            );
                            return "constructed";
                          } catch (error) {
                            return error.name;
                          }
                        })()
                        "#,
                    )?,
                    "TypeError",
                    "WebSocketStream constructor captured by the retired child LocalWindow must fail closed"
                );
                page_vm.vm_mut().eval(&format!(
                    r#"
                    const CurrentChildWebSocket =
                      __retiredChildWebSocketFrame.contentWindow.WebSocket;
                    globalThis.__currentChildWebSocket =
                      new CurrentChildWebSocket({replacement_url_literal});
                    __currentChildWebSocket.onmessage = event =>
                      __retiredChildWebSocketEvents.push("current:" + event.data);
                    "#
                ))?;
                tokio::time::timeout(Duration::from_secs(2), replacement_opened_rx)
                    .await
                    .expect("replacement child WebSocket server should accept")
                    .expect("replacement child WebSocket server open signal");
                let deadline = Instant::now() + Duration::from_secs(2);
                while page_vm
                    .vm_mut()
                    .eval("String(__currentChildWebSocket.readyState)")?
                    != "1"
                {
                    if page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_none()
                    {
                        let _ = tokio::time::timeout(
                            Duration::from_millis(100),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "replacement child WebSocket should open"
                    );
                }
                message_tx
                    .send("stale-child-message".to_owned())
                    .expect("trigger stale child WebSocket message");
                replacement_message_tx
                    .send("current-child-message".to_owned())
                    .expect("trigger replacement child WebSocket message");
                server.await.expect("triggered child WebSocket server");
                replacement_server
                    .await
                    .expect("triggered replacement child WebSocket server");

                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_some()
                    {}
                    let events = page_vm
                        .vm_mut()
                        .eval("__retiredChildWebSocketEvents.join('|')")?;
                    if events == "current:current-child-message" {
                        break Ok::<_, anyhow::Error>(events);
                    }
                    let _ = tokio::time::timeout(
                        Duration::from_millis(100),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await;
                    assert!(
                        Instant::now() < deadline,
                        "replacement LocalWindow should receive its WebSocket event"
                    );
                }
            })
            .await
            .expect("child replacement WebSocket proof should run");

        assert_eq!(
            events, "current:current-child-message",
            "only the replacement child LocalWindow may receive queued WebSocket events"
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_message_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            events_after_websocket_message,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__websocketReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const socket = new WebSocket({url_literal});
  socket.addEventListener("open", () => {{
    __websocketReadyEvents.push("open");
    socket.send("hello");
  }});
  socket.addEventListener("message", event => {{
    __websocketReadyEvents.push("message:" + event.data);
    const frame = document.createElement("iframe");
    frame.onload = () => __websocketReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__websocketReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
    socket.close(1000, "done");
  }});
  socket.addEventListener("close", event => {{
    __websocketReadyEvents.push("close:" + event.code);
  }});
}})()
"#
                ))?;

                let deadline = Instant::now() + Duration::from_secs(10);
                let mut completion_sources = Vec::new();
                let events_after_websocket_message = loop {
                    if page_vm.has_ready_page_websocket_task_for_test() {
                        if let Some(completion_source) =
                            page_vm.run_exact_page_websocket_selected_task_for_test().await?
                        {
                            completion_sources.push(completion_source);
                        }
                    } else {
                        let _ = tokio::time::timeout(
                            Duration::from_millis(100),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    let events = page_vm.vm_mut().eval("__websocketReadyEvents.join('|')")?;
                    if events.contains("message:hello") {
                        break events;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "WebSocket message handler should run after bounded completions; sources: {completion_sources:?}, events: {events}"
                    );
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "WebSocket-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "WebSocket-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__websocketReadyEvents.join('|')")?;
                let host_load_source = Some(
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "WebSocket-created child iframe load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__websocketReadyEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_websocket_message,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    events_after_host_load,
                ))
            })
            .await
            .expect("WebSocket ready-work source test should run");

        assert!(
            completion_sources.contains(&RendererOwnerResourceActivitySource::WebSocket),
            "WebSocket handler should be driven by a WebSocket completion: {completion_sources:?}"
        );
        assert!(
            events_after_websocket_message.contains("message:hello"),
            "WebSocket message handler should create the child frame without running its parser script inline: {events_after_websocket_message}"
        );
        assert!(
            !events_after_websocket_message.contains("child-script:true")
                && !events_after_websocket_message.contains("frame-load"),
            "WebSocket message turn should not run child parser or load work inline: {events_after_websocket_message}"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "WebSocket-created child parser work should follow its navigation commit"
        );
        assert!(
            events_after_script_ready.contains("child-script:true"),
            "child parser work should run on the later DocumentScriptReady turn: {events_after_script_ready}"
        );
        assert!(
            !events_after_script_ready.contains("frame-load"),
            "DocumentScriptReady turn should still not dispatch iframe load inline: {events_after_script_ready}"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a separate HostLoad turn after WebSocket message dispatch"
        );
        assert!(
            events_after_host_load.contains("frame-load"),
            "iframe load should dispatch only on the HostLoad turn: {events_after_host_load}"
        );

        server.await.expect("websocket ready-work server should finish");
    })
    .await;
}

#[tokio::test]

async fn websocket_binary_echo_delivers_arraybuffer_when_binary_type_requests_it() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        socket.binaryType = 'arraybuffer';
                        globalThis.__wsEvents = [`constructed:${{socket.readyState}}:${{socket.binaryType}}`];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            socket.send(new Uint8Array([1, 2, 255]));
                            globalThis.__wsEvents.push(`bufferedAfterSend:${{socket.bufferedAmount}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            const view = new Uint8Array(event.data);
                            globalThis.__wsEvents.push(`message:${{event.data instanceof ArrayBuffer}}:${{Array.from(view).join(',')}}:${{socket.bufferedAmount}}`);
                            socket.close(1000, 'binary');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;

                for _ in 0..20 {
                    while page_vm.run_exact_page_websocket_selected_task_for_test().await?.is_some() {}
                    if page_vm.vm_mut().eval("String(globalThis.__wsDone === true)")? == "true" {
                        break;
                    }
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(1),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await
                    .unwrap_or(false);
                    assert!(arrived, "websocket binary runtime event should arrive");
                }
                while page_vm.run_exact_page_websocket_selected_task_for_test().await?.is_some() {}
                assert_eq!(
                    page_vm.vm_mut().eval("String(globalThis.__wsDone === true)")?,
                    "true"
                );
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("websocket binary echo test should run on owner lane");

        server.await.expect("websocket echo server should finish");
        assert_eq!(
            events,
            r#"["constructed:0:arraybuffer","bufferedAfterSend:3","message:true:1,2,255:0","close:1000:binary:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_binary_echo_delivers_blob_by_default() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsEvents = [`constructed:${{socket.binaryType}}`];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            socket.send(new Blob([new Uint8Array([4, 5])]));
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data instanceof Blob}}:${{event.data.size}}`);
                            socket.close(1000, 'blob');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;

                for _ in 0..20 {
                    while page_vm.run_exact_page_websocket_selected_task_for_test().await?.is_some() {}
                    if page_vm.vm_mut().eval("String(globalThis.__wsDone === true)")? == "true" {
                        break;
                    }
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(1),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await
                    .unwrap_or(false);
                    assert!(arrived, "websocket binary blob runtime event should arrive");
                }
                while page_vm.run_exact_page_websocket_selected_task_for_test().await?.is_some() {}
                assert_eq!(
                    page_vm.vm_mut().eval("String(globalThis.__wsDone === true)")?,
                    "true"
                );
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("websocket binary blob test should run on owner lane");

        server.await.expect("websocket echo server should finish");
        assert_eq!(
            events,
            r#"["constructed:blob","message:true:2","close:1000:blob:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_binary_type_invalid_values_throw_syntax_error() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsEvents = [`initial:${{socket.binaryType}}`];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            try {{
                                socket.binaryType = 'notBlobOrArrayBuffer';
                                globalThis.__wsEvents.push(`invalid-write:${{socket.binaryType}}`);
                            }} catch (error) {{
                                globalThis.__wsEvents.push(`invalid:${{error.name}}:${{socket.binaryType}}`);
                            }}
                            socket.binaryType = 'arraybuffer';
                            globalThis.__wsEvents.push(`arraybuffer:${{socket.binaryType}}`);
                            socket.binaryType = 'blob';
                            globalThis.__wsEvents.push(`blob:${{socket.binaryType}}`);
                            socket.close(1000, 'binary-type');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "websocket binaryType event should arrive",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("websocket binaryType test should run on owner lane");

        server
            .await
            .expect("websocket binaryType server should finish");
        assert_eq!(
            events,
            r#"["initial:blob","invalid:SyntaxError:blob","arraybuffer:arraybuffer","blob:blob","close:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_send_typed_array_view_preserves_offset_and_length() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        socket.binaryType = 'arraybuffer';
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            const buffer = new ArrayBuffer(8);
                            const all = new Uint8Array(buffer);
                            for (let i = 0; i < all.length; i++) all[i] = i + 1;
                            socket.send(new Uint8Array(buffer, 2, 4));
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(Array.from(new Uint8Array(event.data)).join(','));
                            socket.close(1000, 'typed-array');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "websocket typed-array runtime event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("websocket typed-array test should run on owner lane");

        server
            .await
            .expect("websocket typed-array server should finish");
        assert_eq!(events, r#"["3,4,5,6","close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_close_rejects_reason_over_123_utf8_bytes_without_closing() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            const reason = 'x'.repeat(124);
                            try {{
                                socket.close(1000, reason);
                                globalThis.__wsEvents.push('no-throw');
                            }} catch (error) {{
                                globalThis.__wsEvents.push(`${{error.name}}:${{socket.readyState}}`);
                            }}
                            socket.close(1000, 'ok');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "websocket close reason event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("websocket close reason test should run on owner lane");

        server
            .await
            .expect("websocket close reason server should finish");
        assert_eq!(events, r#"["SyntaxError:1","close:1000:ok:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_server_initiated_close_reports_code_reason_and_clean_state() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_server_close_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            globalThis.__wsEvents.push(`open:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "websocket server-close event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("websocket server-close test should run on owner lane");

        server
            .await
            .expect("websocket server-close server should finish");
        assert_eq!(events, r#"["open:1","close:3:3001:server done:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_open_and_frames_record_network_trace_entries() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let network_output = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            socket.send('trace-frame');
                        }});
                        socket.addEventListener('message', () => {{
                            socket.close(1000, 'trace');
                        }});
                        socket.addEventListener('close', () => {{
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;

                for _ in 0..20 {
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_some()
                    {}
                    if page_vm
                        .vm_mut()
                        .eval("String(globalThis.__wsDone === true)")?
                        == "true"
                    {
                        break;
                    }
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(1),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await
                    .unwrap_or(false);
                    assert!(arrived, "websocket frame event should arrive");
                }
                while page_vm
                    .run_exact_page_websocket_selected_task_for_test()
                    .await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("String(globalThis.__wsDone === true)")?,
                    "true"
                );
                Ok::<_, anyhow::Error>(page_vm.vm_mut().take_network_output())
            })
            .await
            .expect("websocket trace test should run on owner lane");
        server.await.expect("websocket trace server should finish");
        let (records, frame_events, lifecycle_events) = split_network_output_items(network_output);

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
        assert_eq!(record.method(), "GET");
        assert_eq!(record.url().as_str(), url);
        assert!(
            record
                .request_headers()
                .iter()
                .any(|(name, _): &(String, String)| name.eq_ignore_ascii_case("origin"))
        );
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
                        .any(|(name, _)| { name.eq_ignore_ascii_case("sec-websocket-accept") })
                );
            }
            outcome => panic!("expected websocket success record, got {outcome:?}"),
        }

        assert_eq!(frame_events.len(), 2);
        assert_eq!(
            frame_events[0].direction(),
            crate::types::WebSocketFrameDirection::Sent
        );
        assert_eq!(
            frame_events[0].opcode(),
            crate::types::WebSocketFrameOpcode::Text
        );
        assert_eq!(frame_events[0].payload_length(), "trace-frame".len());
        assert_eq!(
            frame_events[1].direction(),
            crate::types::WebSocketFrameDirection::Received
        );
        assert_eq!(
            frame_events[1].opcode(),
            crate::types::WebSocketFrameOpcode::Text
        );
        assert_eq!(frame_events[1].payload_length(), "trace-frame".len());
        assert_eq!(lifecycle_events.len(), 3);
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
        assert_eq!(lifecycle_events[2].close_reason(), Some("trace"));
        assert_eq!(lifecycle_events[2].was_clean(), Some(true));
    })
    .await;
}

#[tokio::test]
async fn dropping_page_releases_native_websocket_and_websocket_stream_transports() {
    run_page_vm_async_test(async move {
        for constructor in ["WebSocket", "WebSocketStream"] {
            let (url, server) =
                moli_websocket::test_support::spawn_transport_retirement_websocket_server().await;
            let page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();
            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    page_vm.vm_mut().eval(&format!(
                        r#"
                    globalThis.__nativeTransportOpened = false;
                    globalThis.__nativeTransport = new {constructor}({url:?});
                    if ({constructor:?} === 'WebSocket') {{
                        __nativeTransport.onopen = () => {{ __nativeTransportOpened = true; }};
                    }} else {{
                        __nativeTransport.opened.then(() => {{ __nativeTransportOpened = true; }});
                    }}
                "#
                    ))?;
                    tokio::time::timeout(Duration::from_secs(3), async {
                        loop {
                            while page_vm
                                .run_exact_page_websocket_selected_task_for_test()
                                .await?
                                .is_some()
                            {}
                            if page_vm.vm_mut().eval("String(__nativeTransportOpened)")? == "true" {
                                break;
                            }
                            page_vm
                                .wait_for_page_work_arrival_without_timeout(false)
                                .await;
                        }
                        Ok::<(), anyhow::Error>(())
                    })
                    .await
                    .expect("native transport should open")?;
                    drop(page_vm);
                    Ok::<(), anyhow::Error>(())
                })
                .await
                .expect("page retirement should run on owner lane");
            server
                .await
                .expect("dropping Page releases its native socket");
        }
    })
    .await;
}
