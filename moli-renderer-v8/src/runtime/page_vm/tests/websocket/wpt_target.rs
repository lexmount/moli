use super::*;

#[tokio::test]
async fn websocket_wpt_target_message_event_origin_matches_socket_origin() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_text_echo_websocket_server().await;
        let expected_origin = Url::parse(&url)
            .expect("websocket url")
            .origin()
            .ascii_serialization();
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
                        socket.addEventListener('open', () => socket.send('origin'));
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.origin}}:${{event.data}}`);
                            socket.close(1000, 'origin');
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
                    "target websocket MessageEvent.origin event should arrive",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket message origin test should run on owner lane");

        server
            .await
            .expect("target WebSocket message origin server should finish");
        assert_eq!(
            events,
            format!(r#"["message:{expected_origin}:origin","close:true"]"#)
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_wpt_target_event_order_and_metadata_match_wpt() {
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
                        function record(label, expectedType, event) {{
                            globalThis.__wsEvents.push([
                                label,
                                event.type,
                                event instanceof Event,
                                expectedType === 'message' ? event instanceof MessageEvent : true,
                                expectedType === 'close' ? event instanceof CloseEvent : true,
                                event.target === socket,
                                event.currentTarget === socket,
                                event.eventPhase,
                                event.bubbles,
                                event.cancelable
                            ].join(':'));
                        }}
                        socket.addEventListener('open', event => {{
                            record('open-listener-1', 'open', event);
                            socket.send('event-order');
                        }});
                        socket.onopen = event => record('open-on', 'open', event);
                        socket.addEventListener('open', event => record('open-listener-2', 'open', event));
                        socket.addEventListener('message', event => record('message-listener-1', 'message', event));
                        socket.onmessage = event => {{
                            record('message-on', 'message', event);
                            socket.close(1000, 'events');
                        }};
                        socket.addEventListener('message', event => record('message-listener-2', 'message', event));
                        socket.addEventListener('close', event => record('close-listener-1', 'close', event));
                        socket.onclose = event => record('close-on', 'close', event);
                        socket.addEventListener('close', event => {{
                            record('close-listener-2', 'close', event);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket event ordering should complete",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket event ordering test should run on owner lane");

        server
            .await
            .expect("target WebSocket event ordering server should finish");
        assert_eq!(
            events,
            r#"["open-listener-1:open:true:true:true:true:true:2:false:false","open-on:open:true:true:true:true:true:2:false:false","open-listener-2:open:true:true:true:true:true:2:false:false","message-listener-1:message:true:true:true:true:true:2:false:false","message-on:message:true:true:true:true:true:2:false:false","message-listener-2:message:true:true:true:true:true:2:false:false","close-listener-1:close:true:true:true:true:true:2:false:false","close-on:close:true:true:true:true:true:2:false:false","close-listener-2:close:true:true:true:true:true:2:false:false"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_event_to_string_and_flags_match_wpt() {
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
                        function record(label, event) {{
                            globalThis.__wsEvents.push([
                                label,
                                String(event),
                                Object.prototype.toString.call(event),
                                event.bubbles,
                                event.cancelable
                            ].join(':'));
                        }}
                        socket.addEventListener('open', event => {{
                            record('open', event);
                            socket.send('event-flags');
                        }});
                        socket.addEventListener('message', event => {{
                            record('message', event);
                            socket.close(1000, 'event-flags');
                        }});
                        socket.addEventListener('close', event => {{
                            record('close', event);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket event toString/flags should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket event toString/flags test should run on owner lane");

        server
            .await
            .expect("target WebSocket event toString/flags server should finish");
        assert_eq!(
            events,
            r#"["open:[object Event]:[object Event]:false:false","message:[object MessageEvent]:[object MessageEvent]:false:false","close:[object CloseEvent]:[object CloseEvent]:false:false"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_document_cookie_is_sent_on_handshake() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_cookie_echo_websocket_server().await;
        let socket_url = Url::parse(&url).expect("websocket url");
        let page_url = format!(
            "http://{}:{}/page",
            socket_url.host_str().expect("socket host"),
            socket_url.port().expect("socket port")
        );
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&page_url).expect("page url"));
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        document.cookie = 'ws_target_cookie=ok; Path=/';
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(String(event.data).includes('ws_target_cookie=ok'));
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
                    "target websocket cookie echo event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket cookie test should run on owner lane");

        server
            .await
            .expect("target WebSocket cookie echo server should finish");
        assert_eq!(events, r#"[true,"close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_while_connecting_fires_error_then_close() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_sleeping_handshake_websocket_server().await;
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
                        socket.addEventListener('error', () => {{
                            globalThis.__wsEvents.push(`error:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.code}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                        socket.close();
                        globalThis.__wsEvents.push(`afterClose:${{socket.readyState}}`);
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket connecting close event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket connecting close test should run on owner lane");

        server.abort();
        let _ = server.await;
        assert_eq!(events, r#"["afterClose:2","error:3","close:3:1006:false"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_multiple_is_idempotent() {
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
                            socket.close(1000, 'first');
                            socket.close(1000, 'second');
                            socket.close();
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            setTimeout(() => {{
                                globalThis.__wsEvents.push(`count:${{globalThis.__wsEvents.length}}`);
                                globalThis.__wsDone = true;
                            }}, 0);
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket multiple close event should arrive once",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket multiple close test should run on owner lane");

        server
            .await
            .expect("target WebSocket multiple close server should finish");
        assert_eq!(events, r#"["close:1000:first:true","count:1"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_nested_is_idempotent() {
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
                        socket.addEventListener('open', () => socket.close());
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.wasClean}}`);
                            socket.close();
                            globalThis.__wsEvents.push(`afterNested:${{socket.readyState}}`);
                            setTimeout(() => {{
                                globalThis.__wsEvents.push(`count:${{globalThis.__wsEvents.length}}`);
                                globalThis.__wsDone = true;
                            }}, 0);
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket nested close event should arrive once",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket nested close test should run on owner lane");

        server
            .await
            .expect("target WebSocket nested close server should finish");
        assert_eq!(events, r#"["close:3:true","afterNested:3","count:2"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_buffered_amount_tracks_unicode_byte_lengths() {
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
                            socket.send('x');
                            globalThis.__wsEvents.push(`after-x:${{socket.bufferedAmount}}`);
                            socket.send('\u00E5');
                            globalThis.__wsEvents.push(`after-a-ring:${{socket.bufferedAmount}}`);
                            socket.send('\u5336');
                            globalThis.__wsEvents.push(`after-cjk:${{socket.bufferedAmount}}`);
                            socket.send('\uD801\uDC7E');
                            globalThis.__wsEvents.push(`after-nonbmp:${{socket.bufferedAmount}}`);
                        }});
                        let seen = 0;
                        socket.addEventListener('message', event => {{
                            seen++;
                            // `bufferedAmount` drains asynchronously as sent frames leave the
                            // client.  The exact drain/message interleaving is transport-timing
                            // dependent, so this test only pins immediate byte accounting and
                            // eventual zero after close.
                            globalThis.__wsEvents.push(`message:${{seen}}:${{event.data}}`);
                            if (seen === 4) socket.close(1000, 'buffered');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.wasClean}}:${{socket.bufferedAmount}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket bufferedAmount unicode events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket bufferedAmount unicode test should run on owner lane");

        server
            .await
            .expect("target WebSocket bufferedAmount server should finish");
        assert_eq!(
            events,
            r#"["after-x:1","after-a-ring:3","after-cjk:6","after-nonbmp:10","message:1:x","message:2:å","message:3:匶","message:4:𐑾","close:true:0"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_paired_surrogate_echoes_original_text() {
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
                        const data = '\uD801\uDC07';
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            socket.send(data);
                            globalThis.__wsEvents.push(`buffered:${{socket.bufferedAmount}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data === data}}:${{event.data.length}}`);
                            socket.close(1000, 'paired');
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
                    "target websocket paired surrogate events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket paired surrogate test should run on owner lane");

        server
            .await
            .expect("target WebSocket paired surrogate server should finish");
        assert_eq!(events, r#"["buffered:4","message:true:2","close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_unpaired_surrogate_uses_replacement_character() {
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
                        socket.addEventListener('open', () => socket.send('\uD807'));
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`${{event.data}}:${{event.data.charCodeAt(0).toString(16)}}`);
                            socket.close(1000, 'surrogate');
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
                    "target websocket surrogate events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket surrogate test should run on owner lane");

        server
            .await
            .expect("target WebSocket surrogate server should finish");
        assert_eq!(events, r#"["�:fffd","close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_unicode_combining_and_non_bmp_text() {
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
                        const data = '\u00E5 a\u030A \uD801\uDC7E';
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            socket.send(data);
                            globalThis.__wsEvents.push(`buffered:${{socket.bufferedAmount}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data === data}}:${{event.data.length}}:${{socket.bufferedAmount}}`);
                            socket.close(1000, 'unicode');
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
                    "target websocket unicode text events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket unicode text test should run on owner lane");

        server
            .await
            .expect("target WebSocket unicode text server should finish");
        assert_eq!(events, r#"["buffered:11","message:true:7:0","close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_65k_text_frame_echoes_cleanly() {
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
                        const data = 'c'.repeat(65000);
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            socket.send(data);
                            globalThis.__wsEvents.push(`buffered:${{socket.bufferedAmount}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data.length}}:${{event.data === data}}`);
                            socket.close(1000, '65k');
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
                    "target websocket 65K text events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket 65K text test should run on owner lane");

        server
            .await
            .expect("target WebSocket 65K text server should finish");
        assert_eq!(
            events,
            r#"["buffered:65000","message:65000:true","close:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_binary_payload_matrix() {
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
                        let index = 0;
                        socket.addEventListener('open', () => {{
                            const buffer = new ArrayBuffer(15);
                            const whole = new Uint8Array(buffer);
                            for (let i = 0; i < whole.length; i++) whole[i] = i;
                            socket.send(buffer);
                            globalThis.__wsEvents.push(`after-arraybuffer:${{socket.bufferedAmount}}`);

                            const viewBuffer = new ArrayBuffer(8);
                            const all = new Uint8Array(viewBuffer);
                            for (let i = 0; i < all.length; i++) all[i] = i + 1;
                            socket.send(new Uint8Array(viewBuffer, 2, 4));
                            globalThis.__wsEvents.push(`after-view:${{socket.bufferedAmount}}`);

                            socket.send(new Blob(['abc']));
                            globalThis.__wsEvents.push(`after-blob:${{socket.bufferedAmount}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            if (index === 0) {{
                                globalThis.__wsEvents.push(`arraybuffer:${{event.data.byteLength}}`);
                            }} else if (index === 1) {{
                                globalThis.__wsEvents.push(`view:${{Array.from(new Uint8Array(event.data)).join(',')}}`);
                                socket.binaryType = 'blob';
                            }} else {{
                                globalThis.__wsEvents.push(`blob:${{event.data instanceof Blob}}:${{event.data.size}}`);
                                socket.close(1000, 'binary-matrix');
                            }}
                            index++;
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
                    "target websocket binary send matrix events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket binary send matrix test should run on owner lane");

        server
            .await
            .expect("target WebSocket binary send matrix server should finish");
        assert_eq!(
            events,
            r#"["after-arraybuffer:15","after-view:19","after-blob:22","arraybuffer:15","view:3,4,5,6","blob:true:3","close:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_buffered_amount_binary_payload_lengths() {
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
                            const buffer = new ArrayBuffer(10);
                            socket.send(buffer);
                            globalThis.__wsEvents.push(`arraybuffer:${{socket.bufferedAmount}}`);

                            const viewBuffer = new ArrayBuffer(8);
                            const all = new Uint8Array(viewBuffer);
                            for (let i = 0; i < all.length; i++) all[i] = i;
                            socket.send(new Uint8Array(viewBuffer, 2, 3));
                            globalThis.__wsEvents.push(`view:${{socket.bufferedAmount}}`);

                            socket.send(new Blob(['abcdefg']));
                            globalThis.__wsEvents.push(`blob:${{socket.bufferedAmount}}`);
                        }});
                        let seen = 0;
                        socket.addEventListener('message', event => {{
                            seen++;
                            // See the unicode bufferedAmount test: the stable contract here is
                            // immediate byte accounting plus eventual drain, not exact
                            // send-completion/message event ordering.
                            globalThis.__wsEvents.push(`message:${{seen}}:${{event.data.byteLength}}`);
                            if (seen === 3) socket.close(1000, 'buffered-binary');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.wasClean}}:${{socket.bufferedAmount}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket bufferedAmount binary payload events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket bufferedAmount binary payload test should run on owner lane");

        server
            .await
            .expect("target WebSocket bufferedAmount binary payload server should finish");
        assert_eq!(
            events,
            r#"["arraybuffer:10","view:13","blob:20","message:1:10","message:2:3","message:3:7","close:true:0"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_binary_type_controls_message_data_type() {
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
                        let index = 0;
                        socket.addEventListener('open', () => {{
                            socket.send(new Uint8Array([1, 2]));
                        }});
                        socket.addEventListener('message', event => {{
                            if (index === 0) {{
                                globalThis.__wsEvents.push(
                                    `default:${{socket.binaryType}}:${{event.data instanceof Blob}}:${{event.data.size}}`
                                );
                                socket.binaryType = 'arraybuffer';
                                socket.send(new Uint8Array([3, 4, 5]));
                            }} else {{
                                const bytes = Array.from(new Uint8Array(event.data)).join(',');
                                globalThis.__wsEvents.push(
                                    `arraybuffer:${{event.data instanceof ArrayBuffer}}:${{bytes}}`
                                );
                                socket.close(1000, 'binary-type');
                            }}
                            index++;
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
                    "target websocket binaryType message data events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket binaryType message data test should run on owner lane");

        server
            .await
            .expect("target WebSocket binaryType message data server should finish");
        assert_eq!(
            events,
            r#"["default:blob:true:2","arraybuffer:true:3,4,5","close:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_typed_array_view_matrix() {
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
                        const specs = [
                            {{
                                label: 'int8',
                                ctor: Int8Array,
                                make() {{
                                    const view = new Int8Array(new ArrayBuffer(8));
                                    for (let i = 0; i < view.length; i++) view[i] = i - 4;
                                    return view;
                                }}
                            }},
                            {{
                                label: 'int16-offset',
                                ctor: Int16Array,
                                make() {{
                                    const view = new Int16Array(new ArrayBuffer(8), 2);
                                    for (let i = 0; i < view.length; i++) view[i] = i + 10;
                                    return view;
                                }}
                            }},
                            {{
                                label: 'uint16-offset-length',
                                ctor: Uint16Array,
                                make() {{
                                    const view = new Uint16Array(new ArrayBuffer(8), 2, 2);
                                    for (let i = 0; i < view.length; i++) view[i] = i + 20;
                                    return view;
                                }}
                            }},
                            {{
                                label: 'uint32-offset',
                                ctor: Uint32Array,
                                make() {{
                                    const view = new Uint32Array(new ArrayBuffer(8), 0);
                                    for (let i = 0; i < view.length; i++) view[i] = i + 30;
                                    return view;
                                }}
                            }},
                            {{
                                label: 'float32',
                                ctor: Float32Array,
                                make() {{
                                    const view = new Float32Array(new ArrayBuffer(8));
                                    for (let i = 0; i < view.length; i++) view[i] = i + 0.5;
                                    return view;
                                }}
                            }},
                            {{
                                label: 'float64',
                                ctor: Float64Array,
                                make() {{
                                    const view = new Float64Array(new ArrayBuffer(8));
                                    view[0] = 42.5;
                                    return view;
                                }}
                            }}
                        ];
                        let index = 0;
                        let currentView = null;
                        function sameView(left, right) {{
                            if (left.length !== right.length) return false;
                            for (let i = 0; i < left.length; i++) {{
                                if (left[i] !== right[i]) return false;
                            }}
                            return true;
                        }}
                        function sendNext() {{
                            const spec = specs[index];
                            currentView = spec.make();
                            socket.send(currentView);
                        }}
                        socket.addEventListener('open', sendNext);
                        socket.addEventListener('message', event => {{
                            const spec = specs[index];
                            const result = new spec.ctor(event.data);
                            globalThis.__wsEvents.push(`${{spec.label}}:${{sameView(result, currentView)}}`);
                            index++;
                            if (index === specs.length) {{
                                socket.close(1000, 'typed-array-matrix');
                            }} else {{
                                sendNext();
                            }}
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
                    "target websocket typed-array view matrix events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket typed-array view matrix test should run on owner lane");

        server
            .await
            .expect("target WebSocket typed-array view matrix server should finish");
        assert_eq!(
            events,
            r#"["int8:true","int16-offset:true","uint16-offset-length:true","uint32-offset:true","float32:true","float64:true","close:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_many_64k_messages_with_backpressure() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_backpressure_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        const message = new Uint8Array(65536);
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            for (let i = 0; i < 50; i++) socket.send(message);
                            globalThis.__wsEvents.push(`sent:${{socket.bufferedAmount}}`);
                        }});
                        let replies = 0;
                        socket.addEventListener('message', event => {{
                            if (event.data !== '65536') globalThis.__wsEvents.push(`bad:${{event.data}}`);
                            replies++;
                            if (replies === 50) socket.close(1000, 'backpressure');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{replies}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket backpressure events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket backpressure test should run on owner lane");

        server
            .await
            .expect("target WebSocket backpressure server should finish");
        assert_eq!(events, r#"["sent:3276800","close:50:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_invalid_handshake_closes_uncleanly() {
    run_page_vm_async_test(async move {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        let (url, server) = spawn_raw_websocket_response_server("invalid", response).await;
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
                        socket.addEventListener('open', () => globalThis.__wsEvents.push('open'));
                        socket.addEventListener('message', () => globalThis.__wsEvents.push('message'));
                        socket.addEventListener('error', () => globalThis.__wsEvents.push(`error:${{socket.readyState}}`));
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.code}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket invalid handshake event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket invalid handshake test should run on owner lane");

        server
            .await
            .expect("target WebSocket invalid handshake server should finish");
        assert_eq!(events, r#"["error:3","close:1006:false"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_no_subprotocol_selected_when_server_omits_protocol() {
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
                        const socket = new WebSocket({url_literal}, ['chat', 'superchat']);
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            globalThis.__wsEvents.push(`open:${{socket.protocol}}`);
                            socket.close(1000, 'no-protocol');
                        }});
                        socket.addEventListener('error', () => globalThis.__wsEvents.push('error'));
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
                    "target websocket no-protocol handshake event should arrive",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket no-protocol test should run on owner lane");

        server
            .await
            .expect("target WebSocket no-protocol server should finish");
        assert_eq!(events, r#"["open:","close:true"]"#);
    })
    .await;
}

#[tokio::test]
async fn websocket_wpt_target_selected_subprotocol_is_exposed_after_open() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_subprotocol_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal}, ['chat', 'superchat']);
                        globalThis.__wsEvents = [`initial:${{socket.protocol}}`];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            globalThis.__wsEvents.push(`open:${{socket.readyState}}:${{socket.protocol}}`);
                            socket.close(1000, 'protocol');
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
                    "target websocket selected-protocol event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket selected-protocol test should run on owner lane");

        server
            .await
            .expect("target WebSocket selected-protocol server should finish");
        assert_eq!(events, r#"["initial:","open:1:superchat","close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_without_code_reports_1005() {
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
                        socket.addEventListener('open', () => socket.close());
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
                    "target websocket close-without-code event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket close-without-code test should run on owner lane");

        server
            .await
            .expect("target WebSocket close-without-code server should finish");
        assert_eq!(events, r#"["close:1005::true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_runtime_close_event_readonly_and_delete_surface() {
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
                        socket.addEventListener('open', () => socket.close(1000, 'readonly'));
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(
                                `initial:${{event instanceof CloseEvent}}:${{event.code}}:${{event.reason}}:${{event.wasClean}}`
                            );
                            event.code = 3000;
                            event.reason = 'changed';
                            event.wasClean = false;
                            globalThis.__wsEvents.push(`assigned:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            delete event.code;
                            delete event.reason;
                            delete event.wasClean;
                            globalThis.__wsEvents.push(`delete-own:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                            delete CloseEvent.prototype.code;
                            delete CloseEvent.prototype.reason;
                            delete CloseEvent.prototype.wasClean;
                            globalThis.__wsEvents.push(
                                `delete-prototype:${{event.code}}:${{event.reason}}:${{event.wasClean}}`
                            );
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket runtime CloseEvent readonly/delete event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket runtime CloseEvent readonly/delete test should run on owner lane");

        server
            .await
            .expect("target WebSocket runtime CloseEvent readonly/delete server should finish");
        assert_eq!(
            events,
            r#"["initial:true:1000:readonly:true","assigned:1000:readonly:true","delete-own:1000:readonly:true","delete-prototype:undefined:undefined:undefined"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_1000_reason_is_reported() {
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
                        socket.addEventListener('open', () => socket.close(1000, 'Clean Close'));
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
                    "target websocket close 1000 reason event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket close 1000 reason test should run on owner lane");

        server
            .await
            .expect("target WebSocket close 1000 reason server should finish");
        assert_eq!(events, r#"["close:1000:Clean Close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_undefined_completes_cleanly() {
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
                            globalThis.__wsEvents.push(`open:${{socket.readyState}}`);
                            socket.close(undefined);
                            globalThis.__wsEvents.push(`afterClose:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket close(undefined) event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket close(undefined) test should run on owner lane");

        server
            .await
            .expect("target WebSocket close(undefined) server should finish");
        assert_eq!(events, r#"["open:1","afterClose:2","close:3:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_argument_conversion_order_and_reason_stringification() {
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
                        const steps = [];
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            const code = {{
                                valueOf() {{
                                    steps.push('code');
                                    return 1000;
                                }}
                            }};
                            const reason = {{
                                toString() {{
                                    steps.push('reason');
                                    return 'converted';
                                }}
                            }};
                            socket.close(code, reason);
                            globalThis.__wsEvents.push(`after-close:${{steps.join('|')}}:${{socket.readyState}}`);
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
                    "target websocket close conversion event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket close conversion test should run on owner lane");

        server
            .await
            .expect("target WebSocket close conversion server should finish");
        assert_eq!(
            events,
            r#"["after-close:code|reason:2","close:1000:converted:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_delayed_passive_close_waits_for_server_reply() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_delayed_passive_close_websocket_server().await;
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
                        let started = 0;
                        socket.addEventListener('open', () => {{
                            started = Date.now();
                            socket.close(1000, 'delayed');
                            globalThis.__wsEvents.push(`after-close-call:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            const elapsed = Date.now() - started;
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.wasClean}}:${{elapsed >= 900}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket delayed passive close event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket delayed passive close test should run on owner lane");

        server
            .await
            .expect("target WebSocket delayed passive close server should finish");
        assert_eq!(events, r#"["after-close-call:2","close:3:true:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_blocked_ports_fail_without_open() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const socket = new WebSocket('ws://127.0.0.1:25/blocked-port');
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => globalThis.__wsEvents.push('open'));
                        socket.addEventListener('error', () => globalThis.__wsEvents.push(`error:${socket.readyState}`));
                        socket.addEventListener('close', event => {
                            globalThis.__wsEvents.push(`close:${event.code}:${event.wasClean}`);
                            globalThis.__wsDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket blocked-port event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket blocked-port test should run on owner lane");

        assert_eq!(events, r#"["error:3","close:1006:false"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_failed_connection_error_event_is_event_instance() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_dropping_websocket_server().await;
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
                        socket.addEventListener('open', () => globalThis.__wsEvents.push('open'));
                        socket.addEventListener('error', event => {{
                            globalThis.__wsEvents.push(`error:${{event instanceof Event}}:${{event.type}}:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event instanceof CloseEvent}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket failed connection error event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket failed connection error test should run on owner lane");

        server
            .await
            .expect("target WebSocket failed connection error server should finish");
        assert_eq!(events, r#"["error:true:error:3","close:true:false"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_2999_is_invalid_after_open() {
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
                            try {{
                                socket.close(2999, 'below application range');
                                globalThis.__wsEvents.push('no-throw');
                            }} catch (error) {{
                                globalThis.__wsEvents.push(`${{error.name}}:${{socket.readyState}}`);
                                socket.close(1000, 'cleanup');
                            }}
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
                    "target websocket invalid close code event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket invalid close code test should run on owner lane");

        server
            .await
            .expect("target WebSocket invalid close code server should finish");
        assert_eq!(
            events,
            r#"["InvalidAccessError:1","close:1000:cleanup:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_application_code_boundaries_are_valid() {
    run_page_vm_async_test(async move {
        let (first_url, first_server) = spawn_text_echo_websocket_server().await;
        let (second_url, second_server) = spawn_text_echo_websocket_server().await;
        let first_url_literal = serde_json::to_string(&first_url).expect("serialize first url");
        let second_url_literal = serde_json::to_string(&second_url).expect("serialize second url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const sockets = [
                            [new WebSocket({first_url_literal}), 3000, 'lower'],
                            [new WebSocket({second_url_literal}), 4999, 'upper']
                        ];
                        let closed = 0;
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        for (const [socket, code, reason] of sockets) {{
                            socket.addEventListener('open', () => socket.close(code, reason));
                            socket.addEventListener('error', () => globalThis.__wsEvents.push(`error:${{code}}`));
                            socket.addEventListener('close', event => {{
                                globalThis.__wsEvents.push(`close:${{event.code}}:${{event.reason}}:${{event.wasClean}}`);
                                closed++;
                                if (closed === sockets.length) {{
                                    globalThis.__wsDone = true;
                                }}
                            }});
                        }}
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket application close code events should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents.slice().sort())")
            })
            .await
            .expect("target WebSocket application close code test should run on owner lane");

        first_server
            .await
            .expect("target WebSocket first close boundary server should finish");
        second_server
            .await
            .expect("target WebSocket second close boundary server should finish");
        assert_eq!(
            events,
            r#"["close:3000:lower:true","close:4999:upper:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_only_reason_is_invalid_without_closing() {
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
                            try {{
                                socket.close('Close with only reason');
                                globalThis.__wsEvents.push('no-throw');
                            }} catch (error) {{
                                globalThis.__wsEvents.push(`${{error.name}}:${{socket.readyState}}`);
                                socket.close(1000, 'cleanup');
                            }}
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
                    "target websocket close only reason event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket close only reason test should run on owner lane");

        server
            .await
            .expect("target WebSocket close only reason server should finish");
        assert_eq!(
            events,
            r#"["InvalidAccessError:1","close:1000:cleanup:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_reason_123_bytes_is_allowed() {
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
                        const reason = 'x'.repeat(123);
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => socket.close(1000, reason));
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.code}}:${{event.reason.length}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket 123-byte close reason event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket 123-byte close reason test should run on owner lane");

        server
            .await
            .expect("target WebSocket 123-byte close reason server should finish");
        assert_eq!(events, r#"["close:1000:123:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_close_reason_unpaired_surrogate_is_replaced() {
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
                        socket.addEventListener('open', () => socket.close(1000, '\uD807'));
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.reason.charCodeAt(0).toString(16)}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket surrogate close reason event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket surrogate close reason test should run on owner lane");

        server
            .await
            .expect("target WebSocket surrogate close reason server should finish");
        assert_eq!(events, r#"["close:fffd:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_null_and_empty_string_echo() {
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
                            socket.send(null);
                            socket.send('');
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data}}:${{event.data.length}}`);
                            if (globalThis.__wsEvents.length === 2) socket.close(1000, 'send');
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
                    "target websocket send null/empty event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket send null/empty test should run on owner lane");

        server
            .await
            .expect("target WebSocket send null/empty server should finish");
        assert_eq!(events, r#"["message:null:4","message::0","close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_non_string_values_are_stringified() {
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
                        const values = [
                            null,
                            undefined,
                            1,
                            true,
                            {{ toString() {{ return 'custom-object'; }} }},
                            ['a', 'b']
                        ];
                        let index = 0;
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        function sendNext() {{
                            if (index >= values.length) {{
                                socket.close(1000, 'stringify');
                                return;
                            }}
                            socket.send(values[index]);
                        }}
                        socket.addEventListener('open', sendNext);
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`${{index}}:${{event.data}}`);
                            index++;
                            sendNext();
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
                    "target websocket send non-string events should arrive",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket send non-string test should run on owner lane");

        server
            .await
            .expect("target WebSocket send non-string server should finish");
        assert_eq!(
            events,
            r#"["0:null","1:undefined","2:1","3:true","4:custom-object","5:a,b","close:true"]"#
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_after_close_is_ignored() {
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
                            socket.close(1000, 'closing');
                            globalThis.__wsEvents.push(`afterClose:${{socket.readyState}}:${{String(socket.send('late'))}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data}}`);
                        }});
                        socket.addEventListener('error', () => {{
                            globalThis.__wsEvents.push('error');
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.wasClean}}`);
                            setTimeout(() => {{
                                globalThis.__wsEvents.push(`final:${{globalThis.__wsEvents.length}}`);
                                globalThis.__wsDone = true;
                            }}, 0);
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket send-after-close event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket send-after-close test should run on owner lane");

        server
            .await
            .expect("target WebSocket send-after-close server should finish");
        assert_eq!(
            events,
            r#"["afterClose:2:undefined","close:true","final:2"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_in_close_handler_is_ignored() {
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
                            socket.send('Goodbye');
                            socket.close(1000, 'closing');
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data}}`);
                        }});
                        socket.addEventListener('error', () => globalThis.__wsEvents.push('error'));
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.wasClean}}:${{String(socket.send('late'))}}`);
                            setTimeout(() => {{
                                globalThis.__wsEvents.push(`final:${{globalThis.__wsEvents.length}}`);
                                globalThis.__wsDone = true;
                            }}, 0);
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket send-in-close event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket send-in-close test should run on owner lane");

        server
            .await
            .expect("target WebSocket send-in-close server should finish");
        assert_eq!(
            events,
            r#"["message:Goodbye","close:true:undefined","final:2"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_while_closing_is_ignored() {
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
                            socket.close(1000, 'closing-send');
                            globalThis.__wsEvents.push(`after-close:${{socket.readyState}}:${{socket.bufferedAmount}}`);
                            try {{
                                socket.send('late');
                                globalThis.__wsEvents.push(`send-ok:${{socket.readyState}}:${{socket.bufferedAmount}}`);
                            }} catch (error) {{
                                globalThis.__wsEvents.push(`send-error:${{error && error.name}}`);
                            }}
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket send-while-closing event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket send-while-closing test should run on owner lane");

        server
            .await
            .expect("target WebSocket send-while-closing server should finish");
        assert_eq!(
            events,
            r#"["after-close:2:0","send-ok:2:0","close:3:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_return_value_is_undefined() {
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
                            globalThis.__wsEvents.push(`send:${{String(socket.send('return-value'))}}`);
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data}}`);
                            socket.close(1000, 'send-return');
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
                    "target websocket send return-value event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket send return-value test should run on owner lane");

        server
            .await
            .expect("target WebSocket send return-value server should finish");
        assert_eq!(
            events,
            r#"["send:undefined","message:return-value","close:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_send_without_argument_throws_when_open() {
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
                            try {{
                                socket.send();
                                globalThis.__wsEvents.push('no-throw');
                            }} catch (error) {{
                                globalThis.__wsEvents.push(`${{error.name}}:${{socket.readyState}}`);
                            }}
                            socket.close(1000, 'cleanup');
                        }});
                        socket.addEventListener('message', event => {{
                            globalThis.__wsEvents.push(`message:${{event.data}}`);
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
                    "target websocket send-without-argument event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket send-without-argument test should run on owner lane");

        server
            .await
            .expect("target WebSocket send-without-argument server should finish");
        assert_eq!(events, r#"["TypeError:1","close:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_ready_state_is_closing_immediately_after_close() {
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
                            socket.close();
                            globalThis.__wsEvents.push(`afterClose:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket closing readyState event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket closing readyState test should run on owner lane");

        server
            .await
            .expect("target WebSocket closing readyState server should finish");
        assert_eq!(events, r#"["afterClose:2","close:3:true"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_ready_state_full_lifecycle_sequence() {
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
                        globalThis.__wsEvents = [`constructed:${{socket.readyState === WebSocket.CONNECTING}}:${{socket.readyState}}`];
                        globalThis.__wsDone = false;
                        socket.addEventListener('open', () => {{
                            globalThis.__wsEvents.push(`open:${{socket.readyState === socket.OPEN}}:${{socket.readyState}}`);
                            socket.close();
                            globalThis.__wsEvents.push(`after-close:${{socket.readyState === socket.CLOSING}}:${{socket.readyState}}`);
                        }});
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{socket.readyState === socket.CLOSED}}:${{socket.readyState}}:${{event.wasClean}}`);
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket readyState lifecycle event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket readyState lifecycle test should run on owner lane");

        server
            .await
            .expect("target WebSocket readyState lifecycle server should finish");
        assert_eq!(
            events,
            r#"["constructed:true:0","open:true:1","after-close:true:2","close:true:3:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_extensions_empty_after_open() {
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
                            globalThis.__wsEvents.push(`open:${{socket.extensions}}`);
                            socket.close(1000, 'extensions');
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
                    "target websocket extensions event should arrive",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket extensions test should run on owner lane");

        server
            .await
            .expect("target WebSocket extensions server should finish");
        assert_eq!(events, r#"["open:","close:true"]"#);
    })
    .await;
}

#[tokio::test]
async fn websocket_wpt_target_handshake_set_cookie_updates_document_cookie() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_set_cookie_websocket_server().await;
        let socket_url = Url::parse(&url).expect("websocket url");
        let page_url = format!(
            "http://{}:{}/page",
            socket_url.host_str().expect("socket host"),
            socket_url.port().expect("socket port")
        );
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&page_url).expect("page url"));
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsEvents = [];
                        globalThis.__wsDone = false;
                        socket.addEventListener('close', event => {{
                            globalThis.__wsEvents.push(`close:${{event.wasClean}}`);
                            globalThis.__wsEvents.push(document.cookie.includes('ws_response_cookie=ok'));
                            globalThis.__wsDone = true;
                        }});
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsDone === true)",
                    "target websocket set-cookie event should arrive",
                )
                .await?;
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__wsEvents)")
            })
            .await
            .expect("target WebSocket set-cookie test should run on owner lane");

        server
            .await
            .expect("target WebSocket set-cookie server should finish");
        assert_eq!(events, r#"["close:true",true]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_wpt_target_opening_handshake_origin_header_matches_document_origin() {
    run_page_vm_async_test(async move {
        let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm_with_document_url(
            Url::parse("https://origin-source.example.test/path").expect("page url"),
        );
        let local_executor = page_vm.local_executor.clone();

        let headers = local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .eval(&format!("new WebSocket({url_literal})"))?;
                tokio::time::timeout(Duration::from_secs(3), headers_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("target websocket origin headers did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("target websocket origin sender dropped"))
            })
            .await
            .expect("target WebSocket origin header test should run on owner lane");
        server
            .await
            .expect("target WebSocket origin header server should finish");

        assert_eq!(
            header_value(&headers, "origin").as_deref(),
            Some("https://origin-source.example.test")
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_wpt_target_multi_global_url_parsing_uses_relevant_global() {
    run_page_vm_async_test(async move {
        let (origin, headers_rx, server) =
            spawn_child_document_and_header_capture_websocket_server(
                "/child/base.html",
                "<!doctype html><title>child</title>",
            )
            .await;
        let parent_url =
            Url::parse(&format!("{origin}/parent/page.html")).expect("parent document url");
        let child_url = format!("{origin}/child/base.html");
        let child_url_literal = serde_json::to_string(&child_url).expect("serialize child url");
        let expected_socket_url =
            format!("{}/child/relative", origin.replacen("http://", "ws://", 1));
        let mut page_vm = test_page_vm_with_document_url(parent_url);
        let local_executor = page_vm.local_executor.clone();

        let (result, headers) = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        globalThis.__wsDone = false;
                        globalThis.__wsResult = "";
                        const frame = document.createElement("iframe");
                        frame.onload = () => {{
                            try {{
                                const socket = new frame.contentWindow.WebSocket("relative");
                                globalThis.__wsResult = [
                                    frame.contentWindow.location.href,
                                    socket.url,
                                    socket instanceof WebSocket,
                                    socket instanceof frame.contentWindow.WebSocket
                                ].join("|");
                                socket.addEventListener("open", () => socket.close());
                            }} catch (error) {{
                                globalThis.__wsResult = `error|${{error.name}}|${{error.message}}`;
                            }}
                            globalThis.__wsDone = true;
                        }};
                        frame.src = {child_url_literal};
                        document.body.appendChild(frame);
                    }})()
                    "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "globalThis.__wsDone === true",
                    "child WebSocket relevant settings should resolve",
                )
                .await?;
                let result = page_vm.vm_mut().eval("globalThis.__wsResult")?;
                let headers = tokio::time::timeout(Duration::from_secs(3), headers_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("child websocket headers did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("child websocket header sender dropped"))?;
                Ok::<_, anyhow::Error>((result, headers))
            })
            .await
            .expect("child WebSocket relevant settings test should run on owner lane");
        server
            .await
            .expect("child document websocket server should finish");

        assert_eq!(
            result,
            // The constructor and wrapper belong to the child relevant realm.
            // Chromium therefore reports false for the parent constructor and
            // true for the child constructor.
            format!("{child_url}|{expected_socket_url}|false|true")
        );
        assert_eq!(
            header_value(&headers, ":path").as_deref(),
            Some("/child/relative")
        );
        assert_eq!(
            header_value(&headers, "origin").as_deref(),
            Some(origin.as_str())
        );
    })
    .await;
}
