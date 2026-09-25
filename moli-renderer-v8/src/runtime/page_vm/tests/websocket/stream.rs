use super::*;

#[tokio::test]
async fn websocket_stream_opened_readable_writable_round_trip_text() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [`constructed:${{stream.url}}`];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            globalThis.__wsStreamEvents.push(
                                `opened:${{opened.readable instanceof ReadableStream}}:${{opened.writable instanceof WritableStream}}:${{opened.protocol}}:${{opened.extensions}}`
                            );
                            const writer = opened.writable.getWriter();
                            const writePromise = writer.write('hello-stream');
                            globalThis.__wsStreamEvents.push(`write-promise:${{writePromise instanceof Promise}}`);
                            await writePromise;
                            const reader = opened.readable.getReader();
                            const result = await reader.read();
                            globalThis.__wsStreamEvents.push(`read:${{result.value}}:${{result.done}}`);
                            await writer.close();
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream text round-trip should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream text round-trip should run on owner lane");

        server
            .await
            .expect("websocket stream text round-trip server should finish");
        assert_eq!(
            events,
            format!(
                r#"["constructed:{}","opened:true:true::","write-promise:true","read:hello-stream:false","closed:1005:"]"#,
                url
            )
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_wrapper_slots_ignore_reflection_spoofing_during_dispatch() {
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
                        const stream = new WebSocketStream({url_literal});
                        const openedPromise = stream.opened;
                        const closedPromise = stream.closed;
                        const internalNames = Object.getOwnPropertyNames(stream)
                            .filter(name => name.startsWith('__moliWebSocketStream'))
                            .sort();
                        globalThis.__wsStreamEvents = [
                            `internal:${{internalNames.join(',')}}`
                        ];
                        globalThis.__wsStreamDone = false;
                        stream.__moliWebSocketStreamUrl = 'wss://spoofed.test/socket';
                        stream.__moliWebSocketStreamOpened = Promise.resolve('fake-opened');
                        stream.__moliWebSocketStreamClosed = Promise.resolve('fake-closed');
                        stream.__moliWebSocketStreamOpenedResolve = () => {{
                            globalThis.__wsStreamEvents.push('fake-opened-resolve');
                        }};
                        stream.__moliWebSocketStreamClosedResolve = () => {{
                            globalThis.__wsStreamEvents.push('fake-closed-resolve');
                        }};
                        globalThis.__wsStreamEvents.push(
                            `getter:${{stream.url}}:${{stream.opened === openedPromise}}:${{stream.closed === closedPromise}}`
                        );
                        stream.opened.then(async (opened) => {{
                            globalThis.__wsStreamEvents.push(
                                `opened:${{opened.readable instanceof ReadableStream}}:${{opened.writable instanceof WritableStream}}`
                            );
                            stream.__moliWebSocketStreamReadable = new ReadableStream({{
                                start(controller) {{
                                    controller.enqueue('fake-message');
                                }}
                            }});
                            stream.__moliWebSocketStreamWritable = new WritableStream({{
                                write() {{
                                    globalThis.__wsStreamEvents.push('fake-write');
                                }}
                            }});
                            stream.__moliWebSocketStreamError =
                                new WebSocketError('fake', {{ closeCode: 4002 }});
                            const reader = opened.readable.getReader();
                            const writer = opened.writable.getWriter();
                            await writer.write('real-message');
                            const result = await reader.read();
                            globalThis.__wsStreamEvents.push(`read:${{result.value}}:${{result.done}}`);
                            await writer.close();
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream wrapper private slots should ignore spoofed own properties",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream private slot spoofing test should run on owner lane");

        server
            .await
            .expect("websocket stream private slot spoofing server should finish");
        assert_eq!(
            events,
            format!(
                r#"["internal:","getter:{}:true:true","opened:true:true","read:real-message:false","closed:1005:"]"#,
                url
            )
        );
    })
    .await;
}

// Promise settlement is part of the selected WebSocket task's completion.
// Keep this witness in PageVm so it cannot silently replace the production
// dispatcher with a ScriptVm-only body/checkpoint adapter.
#[tokio::test]
async fn websocket_stream_pending_write_resolvers_ignore_public_spoofing() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm_with_document_url(
            Url::parse("https://websocket-stream-pending-slot.test/").unwrap(),
        );
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().set_fetch_subresource_interception(
                    true,
                    Some(SubresourceResourceType::WebSocket),
                );
                let setup = page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const events = [];
                        Object.defineProperty(Object.prototype, '__moliWebSocketStreamPromiseResolve', {
                            configurable: true,
                            value() { events.push('prototype-resolve'); }
                        });
                        Object.defineProperty(Object.prototype, '__moliWebSocketStreamPromiseReject', {
                            configurable: true,
                            value() { events.push('prototype-reject'); }
                        });
                        Object.defineProperty(Object.prototype, '__moliWebSocketStreamPendingWrites', {
                            configurable: true,
                            value: [{
                                __moliWebSocketStreamPromise: Promise.resolve('fake-write'),
                                __moliWebSocketStreamPromiseResolve() { events.push('prototype-pending-resolve'); },
                                __moliWebSocketStreamPromiseReject() { events.push('prototype-pending-reject'); }
                            }]
                        });
                        Object.defineProperty(Object.prototype, '__moliWebSocketStreamSinkReadyPromise', {
                            configurable: true,
                            value: Promise.resolve('fake-ready')
                        });
                        Object.defineProperty(Object.prototype, '__moliWebSocketStreamSinkReadyResolve', {
                            configurable: true,
                            value() { events.push('prototype-ready-resolve'); }
                        });
                        Object.defineProperty(Object.prototype, '__moliWebSocketStreamSinkReadyReject', {
                            configurable: true,
                            value() { events.push('prototype-ready-reject'); }
                        });
                        const stream = new WebSocketStream('wss://example.test/socket');
                        stream.opened.then(({ writable }) => {
                            events.push('opened');
                            const writer = writable.getWriter();
                            writer.ready.then(() => events.push('ready:resolved'));
                            writer.write('payload').then(
                                () => {
                                    events.push('write:resolved');
                                    globalThis.__wsStreamDone = true;
                                },
                                error => {
                                    events.push(`write:${error && error.name}`);
                                    globalThis.__wsStreamDone = true;
                                }
                            );
                            events.push('write:queued');
                        }, error => {
                            events.push(`opened:${error && error.name}`);
                            globalThis.__wsStreamDone = true;
                        });
                        globalThis.__webSocketStreamPendingWriteEvents = events;
                        globalThis.__wsStreamDone = false;
                        return 'ready';
                    })()
                    "#,
                )?;
                assert_eq!(setup, "ready");

                let mut pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
                let pending_index = pending
                    .iter()
                    .position(|info| info.resource_type == SubresourceResourceType::WebSocket)
                    .expect("WebSocketStream handshake should be intercepted");
                let pending = pending.remove(pending_index);
                assert_eq!(
                    pending.url.as_str(),
                    "wss://example.test/socket",
                    "test should fulfill the intercepted WebSocketStream handshake"
                );

                page_vm.vm_mut().fulfill_pending_subresource_fetch(
                    pending.internal_id,
                    101,
                    Vec::new(),
                    crate::runtime::RendererSyntheticResponseBody::empty(),
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "WebSocketStream pending write should settle through selected Page tasks",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__webSocketStreamPendingWriteEvents)")
            })
            .await
            .expect("WebSocketStream pending write spoofing test should run on owner lane");

        assert_eq!(
            events,
            r#"["opened","write:queued","ready:resolved","write:resolved"]"#
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_stream_opened_readable_writable_round_trip_binary() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            const writer = opened.writable.getWriter();
                            await writer.write(new Uint8Array([7, 8, 255]));
                            const reader = opened.readable.getReader();
                            const result = await reader.read();
                            globalThis.__wsStreamEvents.push(
                                `read:${{result.value instanceof Uint8Array}}:${{result.value.constructor === Uint8Array}}:${{ArrayBuffer.isView(result.value)}}:${{result.value.length}}:${{result.value.byteLength}}:${{Array.from(result.value).join(',')}}:${{result.done}}`
                            );
                            await writer.close();
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream binary round-trip should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream binary round-trip should run on owner lane");

        server
            .await
            .expect("websocket stream binary round-trip server should finish");
        assert_eq!(
            events,
            r#"["read:true:true:true:3:3:7,8,255:false","closed:1005:"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_writer_write_waits_for_frame_sent_event() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            const writer = opened.writable.getWriter();
                            const reader = opened.readable.getReader();
                            const writePromise = writer.write('deferred-write');
                            let settled = false;
                            writePromise.then(() => {{
                                settled = true;
                                globalThis.__wsStreamEvents.push('write-settled');
                            }});
                            await Promise.resolve();
                            globalThis.__wsStreamEvents.push(`before-frame-sent:${{settled}}`);
                            await writePromise;
                            globalThis.__wsStreamEvents.push(`after-frame-sent:${{settled}}`);
                            const result = await reader.read();
                            globalThis.__wsStreamEvents.push(`read:${{result.value}}:${{result.done}}`);
                            await writer.close();
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream writer.write should wait for the frame-sent event",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream write promise test should run on owner lane");

        server
            .await
            .expect("websocket stream write promise server should finish");
        assert_eq!(
            events,
            r#"["before-frame-sent:false","write-settled","after-frame-sent:true","read:deferred-write:false","closed:1005:"]"#
        );
        })
        .await;
}

#[test]
fn websocket_stream_writer_write_rejects_unsupported_payloads() {
    // Nested WebSocketStream promise reactions need the local-runtime PageVm harness.
    run_page_vm_local_runtime_test("page-vm-ws-stream-unsupported-payload", || async {
        run_page_vm_async_test(async move {
            async fn run_case(label: &str, payload_expression: &str) -> String {
                let (url, server) = spawn_text_echo_websocket_server().await;
                let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
                let label_literal = serde_json::to_string(label).expect("serialize case label");
                let mut page_vm = test_page_vm();
                let local_executor = page_vm.local_executor.clone();

                let events = local_executor
                    .run(async move {
                        page_vm.vm_mut().eval(&format!(
                            r#"
                            (() => {{
                                const stream = new WebSocketStream({url_literal});
                                globalThis.__wsStreamEvents = [];
                                globalThis.__wsStreamDone = false;
                                stream.opened.then(async (opened) => {{
                                    const writer = opened.writable.getWriter();
                                    const result = await writer.write({payload_expression}).then(
                                        () => 'write-ok',
                                        error => `write-error:${{error instanceof TypeError}}:${{error && error.name}}`
                                    );
                                    globalThis.__wsStreamEvents.push({label_literal} + ':' + result);
                                    stream.close();
                                }}, (error) => {{
                                    globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                    globalThis.__wsStreamDone = true;
                                }});
                                stream.closed.then(() => {{
                                    globalThis.__wsStreamDone = true;
                                }}, (error) => {{
                                    globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                    globalThis.__wsStreamDone = true;
                                }});
                            }})()
                            "#
                        ))?;

                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(globalThis.__wsStreamDone === true)",
                            "websocket stream unsupported write payload should reject",
                        )
                        .await?;
                        page_vm
                            .vm_mut()
                            .eval("JSON.stringify(globalThis.__wsStreamEvents)")
                    })
                    .await
                    .expect("websocket stream unsupported payload test should run on owner lane");

                server
                    .await
                    .expect("websocket stream unsupported payload server should finish");
                events
            }

            assert_eq!(
                run_case("cannot-stringify", "({ toString() { return this; } })").await,
                r#"["cannot-stringify:write-error:true:TypeError"]"#
            );
            assert_eq!(
                run_case(
                    "resizable-array-buffer",
                    "new ArrayBuffer(8, { maxByteLength: 16 })",
                )
                .await,
                r#"["resizable-array-buffer:write-error:true:TypeError"]"#
            );
            assert_eq!(
                run_case(
                    "shared-array-buffer-view",
                    "new Uint8Array(new SharedArrayBuffer(8))",
                )
                .await,
                r#"["shared-array-buffer-view:write-error:true:TypeError"]"#
            );
        })
        .await;
    });
}

#[tokio::test]
async fn websocket_stream_opened_exposes_selected_subprotocol() {
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
                        const stream = new WebSocketStream({url_literal}, {{
                            protocols: ['chat', 'superchat']
                        }});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then((opened) => {{
                            globalThis.__wsStreamEvents.push(`opened:${{opened.protocol}}:${{opened.extensions}}`);
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                        }});
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream selected subprotocol should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream selected subprotocol test should run on owner lane");

        server
            .await
            .expect("websocket stream selected subprotocol server should finish");
        assert_eq!(events, r#"["opened:superchat:","closed:1005:"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_failed_handshake_rejects_opened_and_closed() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamSettled = 0;
                        globalThis.__wsStreamDone = false;
                        function note(entry) {{
                            globalThis.__wsStreamEvents.push(entry);
                            globalThis.__wsStreamSettled += 1;
                            if (globalThis.__wsStreamSettled === 2) {{
                                globalThis.__wsStreamDone = true;
                            }}
                        }}
                        stream.opened.then(() => {{
                            note('opened');
                        }}, (error) => {{
                            note(`opened-error:${{error instanceof WebSocketError}}:${{error && error.name}}:${{error && error.closeCode}}`);
                        }});
                        stream.closed.then(() => {{
                            note('closed');
                        }}, (error) => {{
                            note(`closed-error:${{error instanceof WebSocketError}}:${{error && error.name}}:${{error && error.closeCode}}`);
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream failed handshake promises should reject",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream failed handshake test should run on owner lane");

        server
            .await
            .expect("websocket stream failed handshake server should finish");
        assert_eq!(
            events,
            r#"["opened-error:true:WebSocketError:1006","closed-error:true:WebSocketError:1006"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_pre_aborted_signal_rejects_without_connecting() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const controller = new AbortController();
                        controller.abort();
                        const stream = new WebSocketStream('ws://127.0.0.1:1/not-started', {
                            signal: controller.signal
                        });
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamSettled = 0;
                        globalThis.__wsStreamDone = false;
                        function note(entry) {
                            globalThis.__wsStreamEvents.push(entry);
                            globalThis.__wsStreamSettled += 1;
                            if (globalThis.__wsStreamSettled === 2) {
                                globalThis.__wsStreamDone = true;
                            }
                        }
                        stream.opened.then(() => {
                            note('opened');
                        }, (error) => {
                            note(`opened-error:${error instanceof DOMException}:${error instanceof WebSocketError}:${error && error.name}`);
                        });
                        stream.closed.then(() => {
                            note('closed');
                        }, (error) => {
                            note(`closed-error:${error instanceof DOMException}:${error instanceof WebSocketError}:${error && error.name}`);
                        });
                    })()
                    "#,
                )?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream pre-aborted signal promises should reject",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream pre-aborted signal test should run on owner lane");

        assert_eq!(
            events,
            r#"["opened-error:true:false:AbortError","closed-error:true:false:AbortError"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_abort_during_handshake_rejects_promises() {
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
                        const controller = new AbortController();
                        const stream = new WebSocketStream({url_literal}, {{
                            signal: controller.signal
                        }});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamSettled = 0;
                        globalThis.__wsStreamDone = false;
                        function note(entry) {{
                            globalThis.__wsStreamEvents.push(entry);
                            globalThis.__wsStreamSettled += 1;
                            if (globalThis.__wsStreamSettled === 2) {{
                                globalThis.__wsStreamDone = true;
                            }}
                        }}
                        stream.opened.then(() => {{
                            note('opened');
                        }}, (error) => {{
                            note(`opened-error:${{error instanceof DOMException}}:${{error instanceof WebSocketError}}:${{error && error.name}}`);
                        }});
                        stream.closed.then(() => {{
                            note('closed');
                        }}, (error) => {{
                            note(`closed-error:${{error instanceof DOMException}}:${{error instanceof WebSocketError}}:${{error && error.name}}`);
                        }});
                        setTimeout(() => controller.abort(), 0);
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream handshake abort promises should reject",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream handshake abort test should run on owner lane");

        server.abort();
        let _ = server.await;
        assert_eq!(
            events,
            r#"["opened-error:true:false:AbortError","closed-error:true:false:AbortError"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_abort_after_open_does_not_close_stream() {
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
                        const controller = new AbortController();
                        const stream = new WebSocketStream({url_literal}, {{
                            signal: controller.signal
                        }});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            globalThis.__wsStreamEvents.push('opened');
                            controller.abort();
                            const writer = opened.writable.getWriter();
                            const reader = opened.readable.getReader();
                            await writer.write('after-abort');
                            const result = await reader.read();
                            globalThis.__wsStreamEvents.push(`read:${{result.value}}:${{result.done}}`);
                            await writer.close();
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream abort after open should not close the stream",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream abort after open test should run on owner lane");

        server
            .await
            .expect("websocket stream abort after open server should finish");
        assert_eq!(
            events,
            r#"["opened","read:after-abort:false","closed:1005:"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_close_method_sends_code_and_reason() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(() => {{
                            globalThis.__wsStreamEvents.push('opened');
                            stream.close({{ closeCode: 3000, reason: 'stream-close' }});
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream close info should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream close info test should run on owner lane");

        server
            .await
            .expect("websocket stream close info server should finish");
        assert_eq!(events, r#"["opened","closed:3000:stream-close"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_writer_close_promise_waits_for_close_handshake() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        let writerDone = false;
                        let closedDone = false;
                        function maybeDone() {{
                            globalThis.__wsStreamDone = writerDone && closedDone;
                        }}
                        stream.closed.then((closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            closedDone = true;
                            maybeDone();
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            closedDone = true;
                            maybeDone();
                        }});
                        stream.opened.then(async (opened) => {{
                            const writer = opened.writable.getWriter();
                            const started = Date.now();
                            const value = await writer.close();
                            const elapsed = Date.now() - started;
                            globalThis.__wsStreamEvents.push(`writer-close:${{value === undefined}}:${{elapsed >= 900}}`);
                            writerDone = true;
                            maybeDone();
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            writerDone = true;
                            closedDone = true;
                            maybeDone();
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream writer.close promise should wait for close handshake",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream writer.close wait test should run on owner lane");

        server
            .await
            .expect("websocket stream writer.close wait server should finish");
        assert_eq!(events, r#"["closed:1005:","writer-close:true:true"]"#);
        })
        .await;
}

#[test]
fn websocket_stream_readable_cancel_and_writable_abort_close_with_websocket_error() {
    run_page_vm_local_runtime_test("page-vm-ws-stream-cancel-abort", || async {
        run_page_vm_async_test(async move {
        async fn run_case(action_expression: &str) -> String {
            let (url, server) = spawn_text_echo_websocket_server().await;
            let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let events = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            const stream = new WebSocketStream({url_literal});
                            globalThis.__wsStreamEvents = [];
                            globalThis.__wsStreamDone = false;
                            stream.opened.then((opened) => {{
                                globalThis.__wsStreamEvents.push('opened');
                                {action_expression};
                            }}, (error) => {{
                                globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                globalThis.__wsStreamDone = true;
                            }});
                            stream.closed.then((closeInfo) => {{
                                globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                                globalThis.__wsStreamDone = true;
                            }}, (error) => {{
                                globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                globalThis.__wsStreamDone = true;
                            }});
                        }})()
                        "#
                    ))?;

                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__wsStreamDone === true)",
                        "websocket stream cancel/abort close reason should complete",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__wsStreamEvents)")
                })
                .await
                .expect("websocket stream cancel/abort test should run on owner lane");

            server
                .await
                .expect("websocket stream cancel/abort server should finish");
            events
        }

        assert_eq!(
            run_case("opened.readable.cancel({ closeCode: 3333, reason: 'ignored' })").await,
            r#"["opened","closed:1005:"]"#
        );
        assert_eq!(
            run_case(
                "opened.readable.cancel(new WebSocketError('', { closeCode: 3333, reason: 'read-cancel' }))"
            )
            .await,
            r#"["opened","closed:3333:read-cancel"]"#
        );
        assert_eq!(
            run_case("opened.writable.abort(new WebSocketError('', { reason: 'write-abort' }))")
                .await,
            r#"["opened","closed:1000:write-abort"]"#
        );
        assert_eq!(
            run_case(
                "const error = new DOMException('nope', 'DataCloneError'); error.closeCode = 4000; error.reason = 'ignored'; opened.writable.abort(error)"
            )
            .await,
            r#"["opened","closed:1005:"]"#
        );
            })
            .await;
    });
}

#[test]
fn websocket_stream_writer_abort_close_info_matches_wpt() {
    // Nested stream promise reactions use the same local-runtime owner lane
    // as the adjacent cancel/abort cases.
    run_page_vm_local_runtime_test("page-vm-ws-stream-writer-abort", || async {
        run_page_vm_async_test(async move {
        async fn run_case(action_expression: &str) -> String {
            let (url, server) = spawn_text_echo_websocket_server().await;
            let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let events = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            const stream = new WebSocketStream({url_literal});
                            globalThis.__wsStreamEvents = [];
                            globalThis.__wsStreamDone = false;
                            stream.opened.then((opened) => {{
                                globalThis.__wsStreamEvents.push('opened');
                                {action_expression};
                            }}, (error) => {{
                                globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                globalThis.__wsStreamDone = true;
                            }});
                            stream.closed.then((closeInfo) => {{
                                globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                                globalThis.__wsStreamDone = true;
                            }}, (error) => {{
                                globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                globalThis.__wsStreamDone = true;
                            }});
                        }})()
                        "#
                    ))?;

                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__wsStreamDone === true)",
                        "websocket stream writer.abort close reason should complete",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__wsStreamEvents)")
                })
                .await
                .expect("websocket stream writer.abort test should run on owner lane");

            server
                .await
                .expect("websocket stream writer.abort server should finish");
            events
        }

        assert_eq!(
            run_case(
                "opened.writable.getWriter().abort(new WebSocketError('', { closeCode: 3334, reason: 'writer-abort' }))"
            )
            .await,
            r#"["opened","closed:3334:writer-abort"]"#
        );
        assert_eq!(
            run_case("opened.writable.getWriter().abort({ closeCode: 3334, reason: 'ignored' })")
                .await,
            r#"["opened","closed:1005:"]"#
        );
    })
    .await;
    });
}

#[test]
fn websocket_stream_close_without_code_variants_report_1005() {
    // Nested WebSocketStream promise reactions need the local-runtime PageVm harness.
    run_page_vm_local_runtime_test("page-vm-ws-stream-close-without-code", || async {
        run_page_vm_async_test(async move {
            async fn run_case(close_expression: &str) -> String {
                let (url, server) = spawn_text_echo_websocket_server().await;
                let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
                let mut page_vm = test_page_vm();
                let local_executor = page_vm.local_executor.clone();

                let events = local_executor
                    .run(async move {
                        page_vm.vm_mut().eval(&format!(
                            r#"
                            (() => {{
                                const stream = new WebSocketStream({url_literal});
                                globalThis.__wsStreamEvents = [];
                                globalThis.__wsStreamDone = false;
                                stream.opened.then(() => {{
                                    globalThis.__wsStreamEvents.push('opened');
                                    {close_expression};
                                }}, (error) => {{
                                    globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                    globalThis.__wsStreamDone = true;
                                }});
                                stream.closed.then((closeInfo) => {{
                                    globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                                    globalThis.__wsStreamDone = true;
                                }}, (error) => {{
                                    globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                    globalThis.__wsStreamDone = true;
                                }});
                            }})()
                            "#
                        ))?;

                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(globalThis.__wsStreamDone === true)",
                            "websocket stream close-without-code variant should complete",
                        )
                        .await?;
                        page_vm
                            .vm_mut()
                            .eval("JSON.stringify(globalThis.__wsStreamEvents)")
                    })
                    .await
                    .expect("websocket stream close-without-code test should run on owner lane");

                server
                    .await
                    .expect("websocket stream close-without-code server should finish");
                events
            }

            assert_eq!(
                run_case("stream.close()").await,
                r#"["opened","closed:1005:"]"#
            );
            assert_eq!(
                run_case("stream.close(undefined)").await,
                r#"["opened","closed:1005:"]"#
            );
            assert_eq!(
                run_case("stream.close(null)").await,
                r#"["opened","closed:1005:"]"#
            );
        })
        .await;
    });
}

#[tokio::test]
async fn websocket_stream_pending_read_resolves_done_on_server_close() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamSettled = 0;
                        globalThis.__wsStreamDone = false;
                        function note(entry) {{
                            globalThis.__wsStreamEvents.push(entry);
                            globalThis.__wsStreamSettled += 1;
                            if (globalThis.__wsStreamSettled === 2) {{
                                globalThis.__wsStreamDone = true;
                            }}
                        }}
                        stream.opened.then(async (opened) => {{
                            globalThis.__wsStreamEvents.push('opened');
                            const reader = opened.readable.getReader();
                            const result = await reader.read();
                            note(`read:${{String(result.value)}}:${{result.done}}`);
                        }}, (error) => {{
                            note(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                        }});
                        stream.closed.then((closeInfo) => {{
                            note(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                        }}, (error) => {{
                            note(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream pending read should finish on server close",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream pending read close test should run on owner lane");

        server
            .await
            .expect("websocket stream pending read close server should finish");
        assert_eq!(
            events,
            r#"["opened","read:undefined:true","closed:3001:server done"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_writer_promises_reflect_remote_clean_close() {
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
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        let writer;
                        stream.opened.then((opened) => {{
                            writer = opened.writable.getWriter();
                            globalThis.__wsStreamEvents.push('opened');
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                        stream.closed.then(async (closeInfo) => {{
                            globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                            const ready = await writer.ready.then(
                                () => 'ready-ok',
                                error => `ready-error:${{error && error.name}}`
                            );
                            const closed = await writer.closed.then(
                                value => `writer-closed:${{value === undefined}}`,
                                error => `writer-closed-error:${{error && error.name}}`
                            );
                            const write = await writer.write('after-close').then(
                                () => 'write-ok',
                                error => `write-error:${{error && error.name}}`
                            );
                            globalThis.__wsStreamEvents.push(ready, closed, write);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream writer promises should reflect remote clean close",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream writer promise close test should run on owner lane");

        server
            .await
            .expect("websocket stream writer promise close server should finish");
        assert_eq!(
            events,
            r#"["opened","closed:3001:server done","ready-error:InvalidStateError","writer-closed-error:InvalidStateError","write-error:InvalidStateError"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_pending_write_rejects_when_server_closes_first() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_close_after_goodbye_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            const writer = opened.writable.getWriter();
                            const goodbyePromise = writer.write('Goodbye');
                            const bigMessagePromise = writer.write(new Uint8Array(8 * 1024 * 1024));
                            const goodbye = await goodbyePromise.then(
                                () => 'goodbye-ok',
                                error => `goodbye-error:${{error && error.name}}`
                            );
                            globalThis.__wsStreamEvents.push(goodbye);
                            const closed = await stream.closed.then(
                                () => 'closed-ok',
                                error => `closed-error:${{error && error.constructor === WebSocketError}}:${{error && error.name}}:${{error && error.closeCode}}`
                            );
                            globalThis.__wsStreamEvents.push(closed);
                            const pendingError = await bigMessagePromise.then(
                                () => null,
                                error => error
                            );
                            globalThis.__wsStreamEvents.push(
                                `pending:${{pendingError && pendingError.name}}:${{pendingError instanceof DOMException}}`
                            );
                            const writerClosed = await writer.closed.then(
                                () => 'writer-closed-ok',
                                error => `writer-closed-error:${{error && error.name}}:${{error === pendingError}}`
                            );
                            globalThis.__wsStreamEvents.push(writerClosed);
                            const later = await writer.write('word').then(
                                () => 'later-ok',
                                error => `later-error:${{error && error.name}}:${{error === pendingError}}`
                            );
                            globalThis.__wsStreamEvents.push(later);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream pending write should reject when server closes first",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream pending write close test should run on owner lane");

        server
            .await
            .expect("websocket stream pending write close server should finish");
        assert_eq!(
            events,
            r#"["goodbye-ok","closed-error:true:WebSocketError:1000","pending:InvalidStateError:true","writer-closed-error:InvalidStateError:true","later-error:InvalidStateError:true"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_wpt_target_sent_messages_observe_backpressure() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_send_backpressure_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            const writer = opened.writable.getWriter();
                            const readyBefore = writer.ready;
                            globalThis.__wsStreamEvents.push(
                                `ready-before-same:${{readyBefore === writer.ready}}`
                            );
                            const start = Date.now();
                            const writePromise = writer.write(new Uint8Array(8 * 1024 * 1024));
                            globalThis.__wsStreamEvents.push(`desired-during:${{writer.desiredSize}}`);
                            const readyDuring = writer.ready;
                            globalThis.__wsStreamEvents.push(
                                `ready-during-same:${{readyDuring === writer.ready}}`
                            );
                            let readySettled = false;
                            readyDuring.then(
                                () => {{ readySettled = true; }},
                                () => {{ readySettled = true; }}
                            );
                            await Promise.resolve();
                            globalThis.__wsStreamEvents.push(
                                `ready-during:${{readySettled ? 'resolved' : 'pending'}}`
                            );
                            await writePromise;
                            const elapsed = Date.now() - start;
                            globalThis.__wsStreamEvents.push(`write-elapsed:${{elapsed >= 1800}}`);
                            globalThis.__wsStreamEvents.push(`desired-after:${{writer.desiredSize}}`);
                            globalThis.__wsStreamEvents.push(
                                `ready-after-same:${{writer.ready === writer.ready}}`
                            );
                            const readyAfter = await writer.ready.then(
                                () => 'ready-after:resolved',
                                error => `ready-after-error:${{error && error.name}}`
                            );
                            globalThis.__wsStreamEvents.push(readyAfter);
                            const closed = await stream.closed.then(
                                info => `closed:${{info.closeCode}}:${{info.reason}}`,
                                error => `closed-error:${{error && error.name}}:${{error && error.closeCode}}`
                            );
                            globalThis.__wsStreamEvents.push(closed);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream sent-message backpressure should delay write resolution",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream send-backpressure test should run on owner lane");

        server
            .await
            .expect("websocket stream send-backpressure server should finish");
        assert_eq!(
            events,
            r#"["ready-before-same:true","desired-during:0","ready-during-same:true","ready-during:pending","write-elapsed:true","desired-after:1","ready-after-same:true","ready-after:resolved","closed:1005:"]"#
        );
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_wpt_target_received_messages_observe_backpressure() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_receive_backpressure_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            const reader = opened.readable.getReader();
                            await new Promise(resolve => setTimeout(resolve, 2000));
                            await reader.read();
                            for (let i = 0; i < 32; ++i) {{
                                await reader.read();
                            }}
                            const elapsed = await reader.read();
                            globalThis.__wsStreamEvents.push(
                                `receive-elapsed:${{Number(elapsed.value) >= 1.8}}`
                            );
                            const closeInfo = await stream.closed;
                            globalThis.__wsStreamEvents.push(
                                `closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`
                            );
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(
                                `opened-error:${{error && error.name}}:${{error && error.closeCode}}`
                            );
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                for _ in 0..400 {
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_some()
                    {}
                    let loader = page_vm.main_document_resource_loader();
                    page_vm
                        .advance_timers_until_deadline_for_test(loader.request_client())
                        .await?;
                    if page_vm.vm_mut().eval("String(globalThis.__wsStreamDone === true)")?
                        == "true"
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                assert_eq!(
                    page_vm.vm_mut().eval("String(globalThis.__wsStreamDone === true)")?,
                    "true",
                    "websocket stream received-message backpressure should complete; events={}",
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__wsStreamEvents)")
                        .unwrap_or_else(|error| format!("<failed to read __wsStreamEvents: {error}>"))
                );
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream receive-backpressure test should run on owner lane");

        server
            .await
            .expect("websocket stream receive-backpressure server should finish");
        assert_eq!(events, r#"["receive-elapsed:true","closed:1005:"]"#);
        })
        .await;
}

#[tokio::test]
async fn websocket_stream_abrupt_close_errors_readable_and_writable_with_same_object() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_abrupt_close_after_open_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let events = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const stream = new WebSocketStream({url_literal});
                        globalThis.__wsStreamEvents = [];
                        globalThis.__wsStreamDone = false;
                        stream.opened.then(async (opened) => {{
                            globalThis.__wsStreamEvents.push('opened');
                            const writer = opened.writable.getWriter();
                            const readPromise = opened.readable.getReader().read();
                            const closedError = await stream.closed.then(
                                () => 'closed-ok',
                                error => error
                            );
                            globalThis.__wsStreamEvents.push(
                                `closed-error:${{closedError instanceof WebSocketError}}:${{closedError && closedError.name}}:${{closedError && closedError.closeCode}}`
                            );
                            const read = await readPromise.then(
                                () => 'read-ok',
                                error => `read-error:${{error === closedError}}:${{error instanceof WebSocketError}}:${{error && error.name}}:${{error && error.closeCode}}`
                            );
                            const ready = await writer.ready.then(
                                () => 'ready-ok',
                                error => `ready-error:${{error === closedError}}:${{error instanceof WebSocketError}}:${{error && error.name}}:${{error && error.closeCode}}`
                            );
                            const writerClosed = await writer.closed.then(
                                () => 'writer-closed-ok',
                                error => `writer-closed-error:${{error === closedError}}:${{error instanceof WebSocketError}}:${{error && error.name}}:${{error && error.closeCode}}`
                            );
                            const write = await writer.write('after-abrupt-close').then(
                                () => 'write-ok',
                                error => `write-error:${{error === closedError}}:${{error instanceof WebSocketError}}:${{error && error.name}}:${{error && error.closeCode}}`
                            );
                            globalThis.__wsStreamEvents.push(read, ready, writerClosed, write);
                            globalThis.__wsStreamDone = true;
                        }}, (error) => {{
                            globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                            globalThis.__wsStreamDone = true;
                        }});
                    }})()
                    "#
                ))?;

                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__wsStreamDone === true)",
                    "websocket stream abrupt close should error streams",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__wsStreamEvents)")
            })
            .await
            .expect("websocket stream abrupt close test should run on owner lane");

        server
            .await
            .expect("websocket stream abrupt close server should finish");
        assert_eq!(
            events,
            r#"["opened","closed-error:true:WebSocketError:1006","read-error:true:true:WebSocketError:1006","ready-error:true:true:WebSocketError:1006","writer-closed-error:true:true:WebSocketError:1006","write-error:true:true:WebSocketError:1006"]"#
        );
        })
        .await;
}

#[test]
fn websocket_stream_remote_close_variants_match_wpt() {
    run_page_vm_local_runtime_test("page-vm-ws-stream-remote-close", || async {
        run_page_vm_async_test(async move {
        async fn run_case(
            close_frame: Option<(u16, String)>,
            after_open_expression: &str,
        ) -> String {
            let (url, server) = spawn_server_close_websocket_server_with_frame(close_frame).await;
            let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let events = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            const stream = new WebSocketStream({url_literal});
                            globalThis.__wsStreamEvents = [];
                            globalThis.__wsStreamDone = false;
                            stream.opened.then(() => {{
                                globalThis.__wsStreamEvents.push('opened');
                                {after_open_expression};
                            }}, (error) => {{
                                globalThis.__wsStreamEvents.push(`opened-error:${{error && error.name}}:${{error && error.closeCode}}`);
                                globalThis.__wsStreamDone = true;
                            }});
                            stream.closed.then((closeInfo) => {{
                                globalThis.__wsStreamEvents.push(`closed:${{closeInfo.closeCode}}:${{closeInfo.reason}}`);
                                globalThis.__wsStreamDone = true;
                            }}, (error) => {{
                                globalThis.__wsStreamEvents.push(`closed-error:${{error && error.name}}:${{error && error.closeCode}}:${{error && error.reason}}`);
                                globalThis.__wsStreamDone = true;
                            }});
                        }})()
                        "#
                    ))?;

                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__wsStreamDone === true)",
                        "websocket stream remote close variant should complete",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("JSON.stringify(globalThis.__wsStreamEvents)")
                })
                .await
                .expect("websocket stream remote close variant should run on owner lane");

            server
                .await
                .expect("websocket stream remote close variant server should finish");
            events
        }

        assert_eq!(run_case(None, "").await, r#"["opened","closed:1005:"]"#);
        assert_eq!(
            run_case(Some((4000, "robot".to_owned())), "").await,
            r#"["opened","closed:4000:robot"]"#
        );
        assert_eq!(
            run_case(Some((4000, "ロボット".to_owned())), "").await,
            r#"["opened","closed:4000:ロボット"]"#
        );
        assert_eq!(
            run_case(
                Some((4222, "remote".to_owned())),
                "stream.close({ closeCode: 4111, reason: 'local' })",
            )
            .await,
            r#"["opened","closed:4222:remote"]"#
        );
            })
            .await;
    });
}
