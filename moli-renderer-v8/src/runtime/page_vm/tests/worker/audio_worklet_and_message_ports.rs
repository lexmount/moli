use super::*;

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
                                };
                                `)
                            );
                            const channel = new MessageChannel();
                            worker.onmessage = (event) => {
                                globalThis.__messagePortCloseRaceResult = event.data;
                                worker.terminate();
                                channel.port1.close();
                                globalThis.__messagePortCloseRaceDone = true;
                            };
                            // Queue both messages before transferring the receiving
                            // port. One task on this agent does not stop the worker
                            // from handling 'first' and closing an already-transferred
                            // port before 'second' is enqueued.
                            channel.port1.postMessage('first');
                            channel.port1.postMessage('second');
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
