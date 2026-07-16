use super::*;

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

        let error_event = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__workerError = null;
                        globalThis.__workerDone = false;
                        const worker = new Worker("/missing-worker.js");
                        worker.onerror = (event) => {
                            globalThis.__workerError = [
                                event.type,
                                Object.getPrototypeOf(event) === Event.prototype,
                                event.target === worker,
                                event.bubbles, event.cancelable, event.composed, event.isTrusted,
                                ['message', 'filename', 'lineno', 'colno', 'error'].some(name => name in event)
                            ];
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
                page_vm.vm_mut().eval("JSON.stringify(globalThis.__workerError)")
            })
            .await
            .expect("worker url failure test should run on owner lane");

        server
            .await
            .expect("worker script failure server should finish");
        assert_eq!(
            error_event,
            r#"["error",true,true,false,false,false,true,false]"#
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
async fn worker_runtime_error_return_non_boolean_truthy_propagates_to_window_onerror() {
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
                        "String(globalThis.__workerDone === true && globalThis.__windowErrorCalled === true)",
                        "non-boolean truthy worker onerror should propagate to window.onerror",
                    )
                    .await?;
                    page_vm.vm_mut().eval(
                        "JSON.stringify({windowErrorCalled: globalThis.__windowErrorCalled, listenerSawDefaultPrevented: globalThis.__listenerSawDefaultPrevented})",
                    )
                })
                .await
                .expect("worker non-boolean truthy onerror propagation test should run on owner lane");

            server
                .await
                .expect("worker non-boolean truthy onerror propagation server should finish");
            assert_eq!(
                result,
                r#"{"windowErrorCalled":true,"listenerSawDefaultPrevented":false}"#
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

#[tokio::test]
async fn blob_url_revocation_uses_window_and_worker_creator_storage_keys() {
    run_page_vm_async_test(async move {
        for document_url in ["https://example.com/", "data:text/html,opaque-parent"] {
        let mut page_vm = test_page_vm_with_document_url(Url::parse(document_url).unwrap());
        let local_executor = page_vm.local_executor.clone();
        let result = local_executor.run(async move {
            page_vm.vm_mut().eval(r#"
                globalThis.__revocationResult = 'pending';
                (async () => {
                    const check = (value, message) => { if (!value) throw new Error(message); };
                    const source = `onmessage = async event => {
                        const {action, url} = event.data;
                        if (action === 'create') postMessage(URL.createObjectURL(new Blob(['payload'])));
                        if (action === 'revoke') { URL.revokeObjectURL(url); postMessage('done'); }
                        if (action === 'read') {
                            try { postMessage(await (await fetch(url)).text()); }
                            catch (error) { postMessage(error.name); }
                        }
                    };`;
                    const sourceUrl = URL.createObjectURL(new Blob([source]));
                    const workers = [new Worker(sourceUrl), new Worker('data:text/javascript,' + encodeURIComponent(source))];
                    const rpc = (worker, action, url) => new Promise((resolve, reject) => {
                        worker.onmessage = event => resolve(event.data);
                        worker.onerror = event => reject(new Error(event.message));
                        worker.postMessage({action, url});
                    });
                    try {
                        for (let i = 0; i < workers.length; i++) {
                            const worker = workers[i];
                            const url = URL.createObjectURL(new Blob(['payload']));
                            await rpc(worker, 'revoke', url);
                            let body;
                            try { body = await (await fetch(url)).text(); }
                            catch (error) { body = error.name; }
                            check(body === (i === 0 ? 'TypeError' : 'payload'), 'worker revocation authority');
                            URL.revokeObjectURL(url);
                            const childUrl = await rpc(worker, 'create');
                            URL.revokeObjectURL(childUrl);
                            check(await rpc(worker, 'read', childUrl) === (i === 0 ? 'TypeError' : 'payload'), 'parent revocation authority');
                            await rpc(worker, 'revoke', childUrl);
                            check(await rpc(worker, 'read', childUrl) === 'TypeError', 'worker can revoke its own opaque URL');
                        }
                    } finally {
                        for (const worker of workers) worker.terminate();
                        URL.revokeObjectURL(sourceUrl);
                    }
                    return 'ok';
                })().then(value => { globalThis.__revocationResult = value; }, error => { globalThis.__revocationResult = String(error); });
            "#)?;
            drive_websocket_until_done(&mut page_vm, "String(globalThis.__revocationResult !== 'pending')", "revocation checks should finish").await?;
            page_vm.vm_mut().eval("globalThis.__revocationResult")
        }).await.expect("blob revocation checks should run on owner lane");
        assert_eq!(result, "ok", "document_url={document_url}");
        }
    }).await;
}
