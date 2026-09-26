use super::*;

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
                    r#"{"type":"error","cancelable":false,"hasMessage":false}"#
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
            super::super::shared_worker_client_event::install_shared_worker_service_wake(&page_vm);
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
