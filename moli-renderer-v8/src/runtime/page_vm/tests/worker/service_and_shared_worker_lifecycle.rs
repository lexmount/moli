use super::*;

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
                            event.preventDefault();
                            globalThis.__missingWorkerEvents.push({
                                target: "worker",
                                type: event.type,
                                constructor: event.constructor.name,
                                trusted: event.isTrusted,
                                cancelable: event.cancelable,
                                defaultPrevented: event.defaultPrevented,
                                hasErrorDetails: ["message", "filename", "lineno", "colno", "error"]
                                    .some(name => name in event)
                            });
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
                    r#"[{"target":"worker","type":"error","constructor":"Event","trusted":true,"cancelable":false,"defaultPrevented":false,"hasErrorDetails":false}]"#
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
                            globalThis.__sharedWorkerRedirectOutcome = JSON.stringify([
                                event.type, event.constructor.name, event.cancelable,
                                "message" in event, event.isTrusted
                            ]);
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
                assert_eq!(
                    outcome,
                    r#"["error","Event",false,false,true]"#,
                    "redirected cross-origin script must fail with a plain Event"
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
                        worker.dispatchEvent(new Event("error"));
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
