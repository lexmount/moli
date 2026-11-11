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
async fn service_worker_intercepts_worker_module_imports_with_worker_client_metadata() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let cross_origin = format!("http://localhost:{}", address.port());
        let expected_referrer = format!("{origin}/");
        let module_source = format!(
            r#"import "{cross_origin}/worker/static.js";
               import("{cross_origin}/worker/dynamic.js");"#
        );
        let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            for index in 0..4 {
                let accepted = if index < 2 {
                    Some(listener.accept().await)
                } else {
                    tokio::time::timeout(Duration::from_millis(500), listener.accept()).await.ok()
                };
                let Some(accepted) = accepted else { break };
                let (mut stream, _) = accepted.unwrap();
                let request = read_http_request_head(&mut stream).await.unwrap();
                let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                request_tx.send(path.to_owned()).unwrap();
                let body = match path {
                    "/sw.js" => r#"
                        self.oninstall = event => event.waitUntil(self.skipWaiting());
                        self.onfetch = event => {
                            const path = new URL(event.request.url).pathname;
                            if (path !== "/worker/static.js" && path !== "/worker/dynamic.js") return;
                            const data = {path, destination: event.request.destination,
                                mode: event.request.mode, clientId: event.clientId,
                                resultingClientId: event.resultingClientId,
                                referrer: event.request.referrer};
                            event.respondWith(new Response(
                                "postMessage(" + JSON.stringify(data) + ")",
                                {headers: {"Content-Type": "text/javascript"}}));
                        };
                    "#,
                    "/worker/main.js" => &module_source,
                    _ => r#"postMessage({source: "network"})"#,
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let mut page_vm = test_page_vm_with_document_url(
            Url::parse(&format!("{origin}/index.html")).unwrap()
        );
        let local_executor = page_vm.local_executor.clone();
        local_executor.run(async move {
            page_vm.vm_mut().eval(r#"
                globalThis.moduleResults = [];
                globalThis.moduleError = null;
                globalThis.pageController = null;
                (async () => {
                    const registration = await navigator.serviceWorker.register("/sw.js", {
                        scope: "/worker/main.js"
                    });
                    const active = registration.installing || registration.active;
                    if (active.state !== "activated") {
                        await new Promise(resolve => active.addEventListener("statechange", () => {
                            if (active.state === "activated") resolve();
                        }));
                    }
                    pageController = navigator.serviceWorker.controller;
                    globalThis.moduleWorker = new Worker("/worker/main.js", {type: "module"});
                    moduleWorker.onmessage = event => moduleResults.push(event.data);
                    moduleWorker.onerror = event => moduleError = event.message;
                })().catch(error => moduleError = String(error));
            "#)?;
            drive_service_worker_page_vm_until_done(
                &mut page_vm,
                "String(moduleResults.length === 2 || moduleError !== null)",
                "controlled worker should intercept static and dynamic module imports",
            ).await?;
            assert_eq!(page_vm.vm_mut().eval("String(moduleError)")?, "null");
            assert_eq!(page_vm.vm_mut().eval("String(pageController)")?, "null");
            let results: serde_json::Value = serde_json::from_str(
                &page_vm.vm_mut().eval("JSON.stringify(moduleResults)")?
            )?;
            let results = results.as_array().unwrap();
            assert_eq!(results.len(), 2);
            let client_id = results[0]["clientId"].as_str().unwrap();
            assert!(!client_id.is_empty());
            for (result, kind, destination) in [
                (&results[0], "static", "worker"),
                (&results[1], "dynamic", "script"),
            ] {
                assert_eq!(result["path"], format!("/worker/{kind}.js"));
                assert_eq!(result["destination"], destination);
                assert_eq!(result["mode"], "cors");
                assert_eq!(result["clientId"], client_id);
                assert_eq!(result["resultingClientId"], "");
                assert_eq!(result["referrer"], expected_referrer);
            }
            anyhow::Ok(())
        }).await.unwrap();
        server.await.unwrap();
        assert_eq!(request_rx.recv().await.as_deref(), Some("/sw.js"));
        assert_eq!(request_rx.recv().await.as_deref(), Some("/worker/main.js"));
        assert!(request_rx.recv().await.is_none());
    }).await;
}

#[tokio::test]
async fn service_worker_intercepts_synchronous_xhr_with_native_document_and_worker_clients() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let cross_origin = format!("http://localhost:{}", address.port());
        let server_origin = origin.clone();
        let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
        let worker_source = format!(r#"
                try {{
                    const xhr = new XMLHttpRequest();
                    xhr.open("POST", "{cross_origin}/sw-echo", false);
                    xhr.withCredentials = true;
                    xhr.setRequestHeader("X-Fixture", "body-marker");
                    xhr.send("payload");
                    postMessage({{
                        data: JSON.parse(xhr.responseText),
                        status: xhr.status, marker: xhr.getResponseHeader("X-Marker"),
                        cookie: xhr.getResponseHeader("Set-Cookie")
                    }});
                }} catch (error) {{ postMessage({{error: String(error)}}); }}
        "#);
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
            let listener = TcpListener::from_std(listener).unwrap();
            for index in 0..6 {
                let wait = if index < 5 {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(500)
                };
                let accepted = tokio::time::timeout(wait, listener.accept()).await.ok();
                let Some(accepted) = accepted else { break };
                let (mut stream, _) = accepted.unwrap();
                let request = read_http_request_head(&mut stream).await.unwrap();
                let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                request_tx.send(request.lines().next().unwrap().to_owned()).unwrap();
                if request.starts_with("OPTIONS ") {
                    assert!(request.to_ascii_lowercase().contains("access-control-request-headers: x-fixture"));
                }
                let body = match path {
                    "/app/sw.js" => r#"
                        const knownClients = new Map();
                        self.oninstall = event => event.waitUntil(self.skipWaiting());
                        self.onactivate = event => event.waitUntil(self.clients.claim());
                        self.onmessage = event => {
                            knownClients.set(event.source.id, event.source);
                            event.ports[0].postMessage(event.source.id);
                        };
                        self.onfetch = event => {
                            const path = new URL(event.request.url).pathname;
                            if (path === "/app/redirect") {
                                const target = new URL("/redirect-target", event.request.url);
                                target.hostname = "localhost";
                                event.respondWith(Response.redirect(target, 307));
                                return;
                            }
                            if (path === "/app/cors-proxy") {
                                const target = new URL("/cors-source", event.request.url);
                                target.hostname = "localhost";
                                event.respondWith(fetch(target));
                                return;
                            }
                            if (path !== "/sw-echo") return;
                            event.respondWith((async () => {
                                const client = knownClients.get(event.clientId) || await self.clients.get(event.clientId);
                                const text = JSON.stringify({
                                    method: event.request.method, destination: event.request.destination,
                                    mode: event.request.mode, credentials: event.request.credentials,
                                    clientId: event.clientId, resultingClientId: event.resultingClientId,
                                    clientType: client.type, clientUrl: client.url,
                                    referrer: event.request.referrer, body: await event.request.text(),
                                    header: event.request.headers.get("X-Fixture")
                                });
                                const bytes = new TextEncoder().encode(text);
                                return new Response(new ReadableStream({start(controller) {
                                    controller.enqueue(bytes.slice(0, 8));
                                    setTimeout(() => {
                                        controller.enqueue(bytes.slice(8));
                                        controller.close();
                                    }, 5);
                                }}), {headers: {
                                    "Content-Type": "application/json", "X-Marker": "visible",
                                    "Set-Cookie": "secret=value"
                                }});
                            })());
                        };
                    "#,
                    "/app/worker.js" => &worker_source,
                    "/redirect-target" => "redirect response",
                    "/cors-source" => "cross-origin response",
                    _ => "unexpected network request",
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nAccess-Control-Allow-Origin: {server_origin}\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Headers: x-fixture\r\nAccess-Control-Allow-Methods: POST\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
                });
        });
        let loader_owner = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        // Synchronous XHR blocks the test's JavaScript executor just as it
        // blocks a real renderer. Resource work needs the browser executor.
        hooks.resource_task_runner = Some(loader_owner.browser_resource_runtime().resource_task_runner());
        let mut page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader_owner,
            Vec::new(),
            Url::parse(&format!("{origin}/app/page.html")).unwrap(),
            hooks,
        );
        page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
        let local_executor = page_vm.local_executor.clone();
        local_executor.run(async move {
            page_vm.vm_mut().eval(&format!(r#"
                globalThis.syncXhrResults = null;
                globalThis.syncXhrError = null;
                (async () => {{
                    await navigator.serviceWorker.register("/app/sw.js", {{scope: "/app/"}});
                    await navigator.serviceWorker.ready;
                    if (!navigator.serviceWorker.controller) await new Promise(resolve =>
                        navigator.serviceWorker.addEventListener("controllerchange", resolve, {{once:true}}));
                    const channel = new MessageChannel();
                    const identity = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
                    navigator.serviceWorker.controller.postMessage("identity", [channel.port2]);
                    const clientId = await identity;
                    const events = [];
                    Promise.resolve().then(() => events.push("microtask"));
                    setTimeout(() => events.push("timer"), 0);
                    const xhr = new XMLHttpRequest();
                    xhr.open("POST", "{cross_origin}/sw-echo", false);
                    xhr.withCredentials = true;
                    xhr.setRequestHeader("X-Fixture", "body-marker");
                    xhr.send("payload");
                    const window = {{clientId, data: JSON.parse(xhr.responseText),
                        status: xhr.status, marker: xhr.getResponseHeader("X-Marker"),
                        cookie: xhr.getResponseHeader("Set-Cookie"), callbacksDuringSend: events.length}};
                    const redirected = new XMLHttpRequest();
                    redirected.open("POST", "/app/redirect", false);
                    redirected.withCredentials = true;
                    redirected.setRequestHeader("X-Fixture", "body-marker");
                    redirected.send("payload");
                    window.redirectStatus = redirected.status;
                    window.redirectBody = redirected.responseText;
                    globalThis.syncXhrWorker = new Worker("/app/worker.js");
                    syncXhrWorker.onmessage = event => syncXhrResults = {{window, worker: event.data}};
                    syncXhrWorker.onerror = event => syncXhrError = event.message;
                }})().catch(error => syncXhrError = String(error));
            "#))?;
            drive_service_worker_page_vm_until_done(
                &mut page_vm,
                "String(syncXhrResults !== null || syncXhrError !== null)",
                "native controllers should serve synchronous Document and Worker XHR",
            ).await?;
            assert_eq!(page_vm.vm_mut().eval("String(syncXhrError)")?, "null");
            let results: serde_json::Value = serde_json::from_str(
                &page_vm.vm_mut().eval("JSON.stringify(syncXhrResults)")?
            )?;
            assert_eq!(results["window"]["callbacksDuringSend"], 0);
            assert_eq!(results["window"]["redirectStatus"], 200);
            assert_eq!(results["window"]["redirectBody"], "redirect response");
            assert_ne!(results["window"]["data"]["clientId"], results["worker"]["data"]["clientId"]);
            for kind in ["window", "worker"] {
                let result = &results[kind];
                assert_eq!(result["status"], 200);
                assert_eq!(result["marker"], "visible");
                assert!(result["cookie"].is_null());
                let data = &result["data"];
                if kind == "window" {
                    assert_eq!(data["clientId"], result["clientId"]);
                }
                assert!(!data["clientId"].as_str().unwrap().is_empty());
                assert_eq!(data["clientType"], kind);
                assert_eq!(data["clientUrl"], format!("{origin}/app/{}", if kind == "window" { "page.html" } else { "worker.js" }));
                assert_eq!(data["resultingClientId"], "");
                assert_eq!(data["method"], "POST");
                assert_eq!(data["mode"], "cors");
                assert_eq!(data["credentials"], "include");
                assert_eq!(data["destination"], "");
                assert_eq!(data["referrer"], format!("{origin}/"));
                assert_eq!(data["body"], "payload");
                assert_eq!(data["header"], "body-marker");
            }
            let csp_result = page_vm.vm_mut().eval(r#"
                (() => {
                    globalThis.syncCspEvents = [];
                    document.addEventListener("securitypolicyviolation", event =>
                        syncCspEvents.push(event.effectiveDirective));
                    const meta = document.createElement("meta");
                    meta.httpEquiv = "Content-Security-Policy";
                    meta.content = "connect-src 'self'";
                    document.head.append(meta);
                    const xhr = new XMLHttpRequest();
                    xhr.open("GET", "/app/cors-proxy", false);
                    try { xhr.send(); return "allowed"; }
                    catch (error) { return error.name + ":" + syncCspEvents.length; }
                })()
            "#)?;
            assert_eq!(csp_result, "NetworkError:0");
            run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
            assert_eq!(page_vm.vm_mut().eval("JSON.stringify(syncCspEvents)")?, r#"["connect-src"]"#);
            anyhow::Ok(())
        }).await.unwrap();
        server.join().unwrap();
        assert_eq!(request_rx.recv().await.as_deref(), Some("GET /app/sw.js HTTP/1.1"));
        assert_eq!(request_rx.recv().await.as_deref(), Some("OPTIONS /redirect-target HTTP/1.1"));
        assert_eq!(request_rx.recv().await.as_deref(), Some("POST /redirect-target HTTP/1.1"));
        assert_eq!(request_rx.recv().await.as_deref(), Some("GET /app/worker.js HTTP/1.1"));
        assert_eq!(request_rx.recv().await.as_deref(), Some("GET /cors-source HTTP/1.1"));
        assert!(request_rx.recv().await.is_none());
    }).await;
}

#[tokio::test]
async fn service_worker_intercepts_asynchronous_worker_xhr_with_native_clients_and_response_filters()
 {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let cross_origin = format!("http://localhost:{}", address.port());
        let server_origin = origin.clone();
        let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
        let worker_source = format!(r#"
            function take(url) {{
                return new Promise(resolve => {{
                    const xhr = new XMLHttpRequest();
                    xhr.open("POST", url, true);
                    xhr.withCredentials = true;
                    xhr.setRequestHeader("X-Fixture", "body-marker");
                    let returned = false;
                    xhr.onloadend = () => resolve({{
                        returned, status: xhr.status, text: xhr.responseText,
                        visible: xhr.getResponseHeader("X-Visible"),
                        hidden: xhr.getResponseHeader("X-Hidden"),
                        cookie: xhr.getResponseHeader("Set-Cookie")
                    }});
                    xhr.send("payload");
                    returned = true;
                }});
            }}
            async function run() {{
                const violations = [];
                self.addEventListener("securitypolicyviolation", event => violations.push({{
                    directive: event.effectiveDirective, disposition: event.disposition,
                    blocked: event.blockedURI
                }}));
                const echo = await take("{cross_origin}/sw-echo");
                const data = JSON.parse(echo.text);
                data.paddingLength = data.padding.length;
                delete data.padding;
                delete echo.text;
                const cors = await take("/app/cors-proxy");
                await new Promise(resolve => setTimeout(resolve, 50));
                return {{echo, data, cors, violations,
                    resourceNames: performance.getEntriesByType("resource").map(entry => entry.name)}};
            }}
            self.onmessage = () => run().then(value => self.postMessage(value)).catch(error => self.postMessage({{error: String(error)}}));
            self.onconnect = event => {{
                const port = event.ports[0];
                port.onmessage = () => run().then(value => port.postMessage(value))
                    .catch(error => port.postMessage({{error: String(error)}}));
            }};
        "#);
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                for index in 0..6 {
                    let wait = if index < 5 { Duration::from_secs(5) } else { Duration::from_millis(500) };
                    let Ok(accepted) = tokio::time::timeout(wait, listener.accept()).await else { break };
                    let (mut stream, _) = accepted.unwrap();
                    let request = read_http_request_head(&mut stream).await.unwrap();
                    let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                    requests.send(path.to_owned()).unwrap();
                    let body = match path {
                        "/app/sw.js" => r#"
                            self.oninstall = event => event.waitUntil(self.skipWaiting());
                            self.onfetch = event => {
                                const path = new URL(event.request.url).pathname;
                                if (path === "/app/cors-proxy") {
                                    const target = new URL("/cors-source", event.request.url);
                                    target.hostname = "localhost";
                                    event.respondWith(fetch(target, {credentials: "include"}));
                                } else if (path === "/sw-echo") {
                                    event.respondWith((async () => {
                                        const client = await self.clients.get(event.clientId);
                                        const text = JSON.stringify({
                                            method: event.request.method, mode: event.request.mode,
                                            credentials: event.request.credentials,
                                            destination: event.request.destination,
                                            clientId: event.clientId, resultingClientId: event.resultingClientId,
                                            clientType: client.type, clientUrl: client.url,
                                            referrer: event.request.referrer,
                                            header: event.request.headers.get("X-Fixture"),
                                            body: await event.request.text(), padding: "x".repeat(1024 * 1024)
                                        });
                                        const bytes = new TextEncoder().encode(text);
                                        return new Response(new ReadableStream({start(controller) {
                                            let offset = 0;
                                            function push() {
                                                if (offset === bytes.length) { controller.close(); return; }
                                                const next = Math.min(offset + 65536, bytes.length);
                                                controller.enqueue(bytes.slice(offset, next));
                                                offset = next;
                                                setTimeout(push, 0);
                                            }
                                            push();
                                        }}), {headers: {
                                            "Content-Type": "application/json", "X-Visible": "visible",
                                            "X-Hidden": "basic-visible", "Set-Cookie": "secret=value"
                                        }});
                                    })());
                                }
                            };
                        "#,
                        "/app/actor.js" => &worker_source,
                        "/cors-source" => "cors response",
                        _ => "unexpected network request",
                    };
                    let policy = if path == "/app/actor.js" { "Content-Security-Policy-Report-Only: connect-src 'self'\r\n" } else { "" };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: no-store\r\n{policy}Access-Control-Allow-Origin: {server_origin}\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Expose-Headers: X-Visible\r\nX-Visible: visible\r\nX-Hidden: network-hidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });
        });
        let loader_owner = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        hooks.resource_task_runner = Some(loader_owner.browser_resource_runtime().resource_task_runner());
        let mut page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader_owner, Vec::new(), Url::parse(&format!("{origin}/page.html")).unwrap(), hooks,
        );
        page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
        let local_executor = page_vm.local_executor.clone();
        local_executor.run(async move {
            page_vm.vm_mut().eval(r#"
                globalThis.asyncXhrResults = null;
                globalThis.asyncXhrError = null;
                (async () => {
                    const registration = await navigator.serviceWorker.register("/app/sw.js", {scope: "/app/actor.js"});
                    const active = registration.installing || registration.active;
                    if (active.state !== "activated") await new Promise(resolve => active.addEventListener("statechange", () => {
                        if (active.state === "activated") resolve();
                    }));
                    globalThis.asyncXhrDedicated = new Worker("/app/actor.js");
                    const dedicated = new Promise((resolve, reject) => {
                        asyncXhrDedicated.onmessage = event => resolve(event.data);
                        asyncXhrDedicated.onerror = event => reject(event.message);
                    });
                    asyncXhrDedicated.postMessage("run");
                    globalThis.asyncXhrShared = new SharedWorker("/app/actor.js");
                    const shared = new Promise((resolve, reject) => {
                        asyncXhrShared.port.onmessage = event => resolve(event.data);
                        asyncXhrShared.onerror = event => reject(event.message);
                    });
                    asyncXhrShared.port.postMessage("run");
                    asyncXhrResults = {dedicated: await dedicated, shared: await shared};
                })().catch(error => asyncXhrError = String(error));
            "#)?;
            drive_service_worker_and_shared_worker_page_vm_until_done(
                &mut page_vm, "String(asyncXhrResults !== null || asyncXhrError !== null)",
                "native Worker clients should intercept asynchronous XHR without a controlled creator",
            ).await?;
            assert_eq!(page_vm.vm_mut().eval("String(asyncXhrError)")?, "null");
            assert_eq!(page_vm.vm_mut().eval("String(navigator.serviceWorker.controller)")?, "null");
            let results: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval("JSON.stringify(asyncXhrResults)")?)?;
            assert_ne!(results["dedicated"]["data"]["clientId"], results["shared"]["data"]["clientId"]);
            for (kind, expected_type) in [("dedicated", "worker"), ("shared", "sharedworker")] {
                let result = &results[kind];
                assert!(result.get("error").is_none(), "{result}");
                assert_eq!(result["echo"]["returned"], true);
                assert_eq!(result["echo"]["status"], 200);
                assert_eq!(result["echo"]["visible"], "visible");
                assert_eq!(result["echo"]["hidden"], "basic-visible");
                assert!(result["echo"]["cookie"].is_null());
                let data = &result["data"];
                assert!(!data["clientId"].as_str().unwrap().is_empty());
                assert_eq!(data["clientType"], expected_type);
                assert_eq!(data["clientUrl"], format!("{origin}/app/actor.js"));
                assert_eq!(data["resultingClientId"], "");
                assert_eq!(data["destination"], "");
                assert_eq!(data["method"], "POST");
                assert_eq!(data["mode"], "cors");
                assert_eq!(data["credentials"], "include");
                assert_eq!(data["referrer"], format!("{origin}/"));
                assert_eq!(data["body"], "payload");
                assert_eq!(data["header"], "body-marker");
                assert_eq!(data["paddingLength"], 1024 * 1024);
                assert_eq!(result["cors"]["returned"], true);
                assert_eq!(result["cors"]["status"], 200);
                assert_eq!(result["cors"]["text"], "cors response");
                assert_eq!(result["cors"]["visible"], "visible");
                assert!(result["cors"]["hidden"].is_null());
                assert!(result["cors"]["cookie"].is_null());
                let violations = result["violations"].as_array().unwrap();
                assert_eq!(violations.len(), 3, "{kind}: {violations:?}");
                assert_eq!(violations.iter().filter(|v| v["blocked"] == format!("{cross_origin}/sw-echo")).count(), 2);
                assert_eq!(violations.iter().filter(|v| v["blocked"] == format!("{origin}/app/cors-proxy")).count(), 1);
                assert!(violations.iter().all(|v| v["directive"] == "connect-src" && v["disposition"] == "report"));
                let names = result["resourceNames"].as_array().unwrap();
                assert!(names.contains(&serde_json::json!(format!("{cross_origin}/sw-echo"))));
                assert!(names.contains(&serde_json::json!(format!("{origin}/app/cors-proxy"))));
                assert!(!names.contains(&serde_json::json!(format!("{cross_origin}/cors-source"))));
            }
            anyhow::Ok(())
        }).await.unwrap();
        server.join().unwrap();
        let mut paths = Vec::new();
        while let Some(path) = received.recv().await { paths.push(path); }
        paths.sort();
        assert_eq!(paths, ["/app/actor.js", "/app/actor.js", "/app/sw.js", "/cors-source", "/cors-source"]);
    }).await;
}

#[tokio::test]
async fn service_worker_intercepts_classic_worker_imports_with_native_clients_and_response_filters()
{
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let server_origin = origin.clone();
        let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
        let worker_source = r#"
            async function run() {
                const errors = {}, violations = [];
                self.addEventListener("securitypolicyviolation", event => violations.push({
                    directive: event.effectiveDirective, disposition: event.disposition,
                    blocked: event.blockedURI
                }));
                for (const name of ["synthetic", "cors", "opaque", "bad-mime", "error", "fallback"]) {
                    try { importScripts("/app/" + name + ".js"); }
                    catch (error) { errors[name] = {name: error.name, message: error.message}; }
                }
                await new Promise(resolve => setTimeout(resolve, 50));
                return {metadata: self.importMetadata, corsRan: self.corsRan,
                    opaqueRan: self.opaqueRan, fallbackRan: self.fallbackRan, errors, violations,
                    resources: performance.getEntriesByType("resource").map(entry => ({
                        name: entry.name, status: entry.responseStatus, initiatorType: entry.initiatorType
                    }))};
            }
            self.onmessage = () => run().then(value => self.postMessage(value))
                .catch(error => self.postMessage({error: String(error)}));
            self.onconnect = event => {
                const port = event.ports[0];
                port.onmessage = () => run().then(value => port.postMessage(value))
                    .catch(error => port.postMessage({error: String(error)}));
            };
        "#;
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                for index in 0..10 {
                    let wait = if index < 7 { Duration::from_secs(5) } else { Duration::from_millis(500) };
                    let Ok(accepted) = tokio::time::timeout(wait, listener.accept()).await else { break };
                    let (mut stream, _) = accepted.unwrap();
                    let request = read_http_request_head(&mut stream).await.unwrap();
                    let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                    requests.send(path.to_owned()).unwrap();
                    let body = match path {
                        "/app/sw.js" => r#"
                            self.oninstall = event => event.waitUntil(self.skipWaiting());
                            self.onfetch = event => {
                                const path = new URL(event.request.url).pathname;
                                if (path === "/app/synthetic.js") {
                                    event.respondWith((async () => {
                                        const client = await self.clients.get(event.clientId);
                                        const data = {method: event.request.method, mode: event.request.mode,
                                            credentials: event.request.credentials, destination: event.request.destination,
                                            clientId: event.clientId, resultingClientId: event.resultingClientId,
                                            clientType: client.type, clientUrl: client.url, referrer: event.request.referrer};
                                        const source = "self.importMetadata = " + JSON.stringify(data) +
                                            "; throw new Error('basic-visible');";
                                        return new Response(new ReadableStream({start(controller) {
                                            const bytes = new TextEncoder().encode(source);
                                            controller.enqueue(bytes.slice(0, 20));
                                            setTimeout(() => { controller.enqueue(bytes.slice(20)); controller.close(); }, 0);
                                        }}), {headers: {"Content-Type": "text/javascript"}});
                                    })());
                                } else if (path === "/app/cors.js" || path === "/app/opaque.js") {
                                    const target = new URL(path === "/app/cors.js" ? "/cors-source.js" : "/opaque-source.js", event.request.url);
                                    target.hostname = "localhost";
                                    event.respondWith(fetch(target, {mode: path === "/app/cors.js" ? "cors" : "no-cors"}));
                                } else if (path === "/app/bad-mime.js") {
                                    event.respondWith(new Response("throw new Error('must not execute');", {headers: {"Content-Type": "text/plain"}}));
                                } else if (path === "/app/error.js") {
                                    event.respondWith(Response.error());
                                }
                            };
                        "#,
                        "/app/actor.js" => worker_source,
                        "/cors-source.js" => "self.corsRan = true; throw new Error('cors-visible');",
                        "/opaque-source.js" => "self.opaqueRan = true; throw new Error('opaque-private');",
                        "/app/fallback.js" => "self.fallbackRan = true;",
                        _ => "throw new Error('unexpected network request');",
                    };
                    let policy = if path == "/app/actor.js" { "Content-Security-Policy-Report-Only: script-src 'self'\r\n" } else { "" };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: no-store\r\n{policy}Access-Control-Allow-Origin: {server_origin}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });
        });
        let loader_owner = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        hooks.resource_task_runner = Some(loader_owner.browser_resource_runtime().resource_task_runner());
        let mut page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader_owner, Vec::new(), Url::parse(&format!("{origin}/page.html")).unwrap(), hooks,
        );
        page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
        let local_executor = page_vm.local_executor.clone();
        local_executor.run(async move {
            page_vm.vm_mut().eval(r#"
                globalThis.importResults = null;
                globalThis.importError = null;
                (async () => {
                    const registration = await navigator.serviceWorker.register("/app/sw.js", {scope: "/app/actor.js"});
                    const active = registration.installing || registration.active;
                    if (active.state !== "activated") await new Promise(resolve => active.addEventListener("statechange", () => {
                        if (active.state === "activated") resolve();
                    }));
                    globalThis.importDedicated = new Worker("/app/actor.js");
                    const dedicated = new Promise((resolve, reject) => {
                        importDedicated.onmessage = event => resolve(event.data);
                        importDedicated.onerror = event => reject(event.message);
                    });
                    importDedicated.postMessage("run");
                    globalThis.importShared = new SharedWorker("/app/actor.js");
                    const shared = new Promise((resolve, reject) => {
                        importShared.port.onmessage = event => resolve(event.data);
                        importShared.onerror = event => reject(event.message);
                    });
                    importShared.port.postMessage("run");
                    importResults = {dedicated: await dedicated, shared: await shared};
                })().catch(error => importError = String(error));
            "#)?;
            drive_service_worker_and_shared_worker_page_vm_until_done(
                &mut page_vm, "String(importResults !== null || importError !== null)",
                "classic imports should use native Worker clients while their realms are paused",
            ).await?;
            assert_eq!(page_vm.vm_mut().eval("String(importError)")?, "null");
            assert_eq!(page_vm.vm_mut().eval("String(navigator.serviceWorker.controller)")?, "null");
            let results: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval("JSON.stringify(importResults)")?)?;
            assert_ne!(results["dedicated"]["metadata"]["clientId"], results["shared"]["metadata"]["clientId"]);
            for (kind, expected_type) in [("dedicated", "worker"), ("shared", "sharedworker")] {
                let result = &results[kind];
                assert!(result.get("error").is_none(), "{result}");
                let data = &result["metadata"];
                assert!(!data["clientId"].as_str().unwrap().is_empty());
                assert_eq!(data["clientType"], expected_type);
                assert_eq!(data["clientUrl"], format!("{origin}/app/actor.js"));
                assert_eq!(data["resultingClientId"], "");
                assert_eq!(data["destination"], "script");
                assert_eq!(data["method"], "GET");
                assert_eq!(data["mode"], "no-cors");
                assert_eq!(data["credentials"], "same-origin");
                assert_eq!(data["referrer"], format!("{origin}/app/actor.js"));
                assert_eq!(result["corsRan"], true);
                assert_eq!(result["opaqueRan"], true);
                assert_eq!(result["fallbackRan"], true);
                assert_eq!(result["errors"]["synthetic"]["name"], "Error");
                assert_eq!(result["errors"]["synthetic"]["message"], "basic-visible");
                assert_eq!(result["errors"]["cors"]["name"], "Error");
                assert_eq!(result["errors"]["cors"]["message"], "cors-visible");
                for name in ["opaque", "bad-mime", "error"] {
                    assert_eq!(result["errors"][name]["name"], "NetworkError", "{result}");
                }
                assert!(result["errors"].get("fallback").is_none());
                let violations = result["violations"].as_array().unwrap();
                assert_eq!(violations.len(), 2, "{kind}: {violations:?}");
                assert!(violations.iter().all(|v| v["directive"] == "script-src-elem" && v["disposition"] == "report"));
                let resources = result["resources"].as_array().unwrap();
                assert_eq!(resources.len(), 6, "{resources:?}");
                assert!(resources.iter().all(|entry| entry["initiatorType"] == "other" && entry["name"].as_str().unwrap().starts_with(&origin)));
                assert_eq!(resources.iter().find(|entry| entry["name"] == format!("{origin}/app/cors.js")).unwrap()["status"], 200);
                assert_eq!(resources.iter().find(|entry| entry["name"] == format!("{origin}/app/opaque.js")).unwrap()["status"], 0);
                assert_eq!(violations.iter().filter(|v| v["blocked"] == format!("{origin}/app/cors.js")).count(), 1);
                assert_eq!(violations.iter().filter(|v| v["blocked"] == format!("{origin}/app/opaque.js")).count(), 1);
            }
            anyhow::Ok(())
        }).await.unwrap();
        server.join().unwrap();
        let mut paths = Vec::new();
        while let Some(path) = received.recv().await { paths.push(path); }
        // Concurrent script-cache consumers may share the fallback transport.
        let fallbacks = paths.iter().filter(|path| *path == "/app/fallback.js").count();
        assert!((1..=2).contains(&fallbacks));
        paths.retain(|path| path != "/app/fallback.js");
        paths.sort();
        assert_eq!(paths, ["/app/actor.js", "/app/actor.js", "/app/sw.js", "/cors-source.js", "/cors-source.js", "/opaque-source.js", "/opaque-source.js"]);
    }).await;
}

#[tokio::test]
async fn worker_main_script_fetch_separates_creator_identity_from_the_workers_own_controller() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
        let worker_source = r#"
            async function run() {
                const subresource = await (await fetch('/app/identity')).json();
                return {main: mainMetadata, subresource};
            }
            self.onmessage = () => run().then(result => self.postMessage(result))
                .catch(error => self.postMessage({error: String(error)}));
            self.onconnect = event => {
                const port = event.ports[0];
                port.onmessage = () => run().then(result => port.postMessage(result))
                    .catch(error => port.postMessage({error: String(error)}));
            };
        "#;
        let service_source = r#"
            self.oninstall = event => event.waitUntil(self.skipWaiting());
            self.onmessage = event => event.ports[0].postMessage({id: event.source.id, type: event.source.type});
            self.onfetch = event => {
                const destination = event.request.destination;
                if (destination === 'worker' || destination === 'sharedworker') {
                    event.respondWith((async () => {
                        const client = await self.clients.get(event.clientId);
                        const metadata = {clientId: event.clientId, resultingClientId: event.resultingClientId,
                            destination, sourceType: client.type, sourceUrl: client.url, serviceWorker: self.location.pathname};
                        return new Response('const mainMetadata = ' + JSON.stringify(metadata) + ';\n' + __WORKER_SOURCE__,
                            {headers: {'Content-Type': 'text/javascript'}});
                    })());
                } else if (new URL(event.request.url).pathname === '/app/identity') {
                    event.respondWith((async () => {
                        const client = await self.clients.get(event.clientId);
                        return new Response(JSON.stringify({clientId: event.clientId, resultingClientId: event.resultingClientId,
                            destination, sourceType: client.type, sourceUrl: client.url, serviceWorker: self.location.pathname}),
                            {headers: {'Content-Type': 'application/json'}});
                    })());
                }
            };
        "#.replace("__WORKER_SOURCE__", &serde_json::to_string(worker_source).unwrap());
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                for index in 0..3 {
                    let wait = if index < 2 { Duration::from_secs(30) } else { Duration::from_millis(500) };
                    let Ok(accepted) = tokio::time::timeout(wait, listener.accept()).await else { break };
                    let (mut stream, _) = accepted.unwrap();
                    let request = read_http_request_head(&mut stream).await.unwrap();
                    let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                    requests.send(path.to_owned()).unwrap();
                    let body = match path {
                        "/app/sw.js" => service_source.as_str(),
                        "/sw.js" => r#"
                            self.oninstall = event => event.waitUntil(self.skipWaiting());
                            self.onactivate = event => event.waitUntil(self.clients.claim());
                            self.onfetch = event => event.respondWith(new Response('wrong controller', {headers: {'Content-Type': 'text/javascript'}}));
                        "#,
                        _ => "throw new Error('unexpected network request');",
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });
        });
        let loader_owner = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        hooks.resource_task_runner = Some(loader_owner.browser_resource_runtime().resource_task_runner());
        let mut page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader_owner, Vec::new(), Url::parse(&format!("{origin}/page.html")).unwrap(), hooks,
        );
        page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
        let local_executor = page_vm.local_executor.clone();
        local_executor.run(async move {
            page_vm.vm_mut().eval(r#"
                globalThis.mainScriptResults = null;
                globalThis.mainScriptError = null;
                globalThis.mainScriptWorkers = [];
                async function activated(registration) {
                    const active = registration.installing || registration.active;
                    if (active.state !== 'activated') await new Promise(resolve => active.addEventListener('statechange', () => {
                        if (active.state === 'activated') resolve();
                    }));
                    return active;
                }
                async function pair(phase) {
                    const dedicated = new Worker('/app/dedicated.js');
                    const shared = new SharedWorker('/app/shared.js', {name: phase});
                    mainScriptWorkers.push(dedicated, shared);
                    const one = new Promise((resolve, reject) => {
                        dedicated.onmessage = event => resolve(event.data);
                        dedicated.onerror = event => reject(event.message);
                    });
                    const two = new Promise((resolve, reject) => {
                        shared.port.onmessage = event => resolve(event.data);
                        shared.onerror = event => reject(event.message);
                    });
                    dedicated.postMessage('run'); shared.port.postMessage('run');
                    return {dedicated: await one, shared: await two};
                }
                (async () => {
                    const active = await activated(await navigator.serviceWorker.register('/app/sw.js', {scope: '/app/'}));
                    const channel = new MessageChannel();
                    const identity = new Promise(resolve => channel.port1.onmessage = event => resolve(event.data));
                    active.postMessage('identity', [channel.port2]);
                    const windowIdentity = await identity;
                    const uncontrolled = !navigator.serviceWorker.controller;
                    const first = await pair('uncontrolled');
                    const claimed = new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
                    await activated(await navigator.serviceWorker.register('/sw.js', {scope: '/'}));
                    await claimed;
                    const second = await pair('controlled');
                    mainScriptResults = {windowIdentity, uncontrolled, first, second, controller: navigator.serviceWorker.controller.scriptURL};
                })().catch(error => mainScriptError = String(error));
            "#)?;
            drive_service_worker_and_shared_worker_page_vm_until_done(
                &mut page_vm, "String(mainScriptResults !== null || mainScriptError !== null)",
                "Worker script fetches should expose their creator but select their own controller",
            ).await?;
            assert_eq!(page_vm.vm_mut().eval("String(mainScriptError)")?, "null");
            let results: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval("JSON.stringify(mainScriptResults)")?)?;
            assert_eq!(results["uncontrolled"], true);
            assert_eq!(results["windowIdentity"]["type"], "window");
            assert_eq!(results["controller"], format!("{origin}/sw.js"));
            let mut clients = std::collections::HashSet::new();
            for phase in ["first", "second"] {
                for (kind, destination, client_type) in [("dedicated", "worker", "worker"), ("shared", "sharedworker", "sharedworker")] {
                    let result = &results[phase][kind];
                    assert!(result.get("error").is_none(), "{result}");
                    let main = &result["main"];let subresource = &result["subresource"];
                    assert_eq!(main["clientId"], results["windowIdentity"]["id"]);
                    assert_eq!(main["sourceType"], "window");
                    assert_eq!(main["sourceUrl"], format!("{origin}/page.html"));
                    assert_eq!(main["destination"], destination);
                    assert_eq!(main["serviceWorker"], "/app/sw.js");
                    assert_ne!(main["clientId"], main["resultingClientId"]);
                    assert_eq!(main["resultingClientId"], subresource["clientId"]);
                    assert_eq!(subresource["resultingClientId"], "");
                    assert_eq!(subresource["destination"], "");
                    assert_eq!(subresource["sourceType"], client_type);
                    assert_eq!(subresource["sourceUrl"], format!("{origin}/app/{kind}.js"));
                    assert_eq!(subresource["serviceWorker"], "/app/sw.js");
                    assert!(clients.insert(subresource["clientId"].as_str().unwrap().to_owned()));
                }
            }
            anyhow::Ok(())
        }).await.unwrap();
        server.join().unwrap();
        let mut paths = Vec::new();
        while let Some(path) = received.recv().await { paths.push(path); }
        assert_eq!(paths, ["/app/sw.js", "/sw.js"]);
    }).await;
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
        let mut servers = spawn_cross_origin_redirecting_shared_worker_script_servers().await;
        let base_url = &servers.origin;
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

        servers.assert_rejected_before_cross_origin_fetch().await;
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
        let mut servers = spawn_sw_return_redirect_servers().await;
        let base_url = &servers.origin;
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

        servers.assert_rejected_before_cross_origin_fetch().await;
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
                            addEventListener: "true:true:true:true:function:2:addEventListener",
                            removeEventListener: "true:true:true:true:function:2:removeEventListener",
                            dispatchEvent: "true:true:true:true:function:1:dispatchEvent"
                        };
                        for (const [name, shape] of Object.entries(expectedMethods)) {
                            if (Object.hasOwn(worker, name) || worker[name] !== EventTarget.prototype[name])
                    throw new Error(`inherited EventTarget method ${name}`);
                const descriptor = Object.getOwnPropertyDescriptor(EventTarget.prototype, name);
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
