use super::*;

#[tokio::test]
async fn response_url_getters_exclude_fragments_without_changing_request_urls() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm_with_document_url(
            Url::parse("https://response-url-fragment.test/").unwrap(),
        );
        let local_executor = page_vm.local_executor.clone();
        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (async () => {
                      const results = [];
                      for (const url of [
                        "data:text/plain,hello#fragment",
                        "data:text/plain,empty#",
                        "data:text/plain,encoded%23hash?query#fragment",
                        "data:text/plain,plain"
                      ]) {
                        const request = new Request(url);
                        const response = await fetch(request);
                        const clone = response.clone();
                        const asyncXhr = new XMLHttpRequest();
                        asyncXhr.open("GET", url);
                        await new Promise((resolve, reject) => {
                          asyncXhr.onload = resolve;
                          asyncXhr.onerror = () => reject(new Error("XHR failed"));
                          asyncXhr.send();
                        });
                        const syncXhr = new XMLHttpRequest();
                        syncXhr.open("GET", url, false);
                        syncXhr.send();
                        results.push({
                          request: request.url,
                          response: response.url,
                          clone: clone.url,
                          asyncXhr: asyncXhr.responseURL,
                          syncXhr: syncXhr.responseURL,
                          body: await response.text(),
                          cloneBody: await clone.text(),
                          asyncBody: asyncXhr.responseText,
                          syncBody: syncXhr.responseText
                        });
                      }
                      return results;
                    })().then(
                      results => { globalThis.__responseUrls = results; },
                      error => { globalThis.__responseUrls = { error: String(error) }; }
                    ).finally(() => { globalThis.__responseUrlsDone = true; });
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__responseUrlsDone === true)",
                    "response URL fragment checks should finish",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__responseUrls)")
            })
            .await
            .expect("response URL test should run on owner lane");
        let expected = [
            (
                "data:text/plain,hello#fragment",
                "data:text/plain,hello",
                "hello",
            ),
            ("data:text/plain,empty#", "data:text/plain,empty", "empty"),
            (
                "data:text/plain,encoded%23hash?query#fragment",
                "data:text/plain,encoded%23hash?query",
                "encoded#hash?query",
            ),
            ("data:text/plain,plain", "data:text/plain,plain", "plain"),
        ]
        .map(|(request, response, body)| {
            serde_json::json!({
                "request": request,
                "response": response,
                "clone": response,
                "asyncXhr": response,
                "syncXhr": response,
                "body": body,
                "cloneBody": body,
                "asyncBody": body,
                "syncBody": body,
            })
        });
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&observed).unwrap(),
            serde_json::json!(expected)
        );
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
async fn fetch_and_xhr_preserve_empty_headers_without_typing_binary_bodies() {
    run_page_vm_async_test(async move {
        for worker in [false, true] {
            for api in ["fetch", "fetch-clone", "xhr-async", "xhr-sync"] {
                for method in ["POST", "PUT"] {
                    for empty_content_type in [false, true] {
                        // Sync XHR blocks the VM's thread, so the fixture must
                        // keep accepting and responding on a separate runtime.
                        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                        let server = std::thread::spawn(move || {
                            tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .expect("request capture runtime")
                                .block_on(async move {
                                    let (base_url, request_rx, server) =
                                        spawn_request_capture_http_server().await;
                                    ready_tx.send(base_url).expect("fixture ready receiver");
                                    let request = request_rx.await.expect("captured request");
                                    server.await.expect("request capture server should finish");
                                    request
                                })
                        });
                        let base_url = ready_rx.await.expect("request capture fixture ready");
                        let document_url = Url::parse(&format!("{base_url}/page.html"))
                            .expect("document url");
                        let mut page_vm = test_page_vm_with_document_url(document_url);
                        let local_executor = page_vm.local_executor.clone();
                        let send = format!(
                            r#"(async () => {{
                                const url = {base_url:?} + "/empty-headers";
                                const body = new Uint8Array([1, 2]);
                                const headers = [["X-Empty", " \t "]];
                                if ({empty_content_type}) headers.push(["Content-Type", ""]);
                                if ({api:?}.startsWith("fetch")) {{
                                    const init = {{method: {method:?}, body, headers}};
                                    const response = {api:?} === "fetch-clone"
                                        ? await fetch(new Request(url, init).clone())
                                        : await fetch(url, init);
                                    if (response.status !== 204) throw new Error("fetch status " + response.status);
                                    await response.text();
                                }} else {{
                                    await new Promise((resolve, reject) => {{
                                        const xhr = new XMLHttpRequest();
                                        xhr.open({method:?}, url, {api:?} === "xhr-async");
                                        for (const [name, value] of headers) xhr.setRequestHeader(name, value);
                                        xhr.onload = () => xhr.status === 204 ? resolve() : reject(new Error("XHR status " + xhr.status));
                                        xhr.onerror = () => reject(new Error("XHR network error"));
                                        xhr.send(body);
                                        if ({api:?} === "xhr-sync") {{
                                            if (xhr.status !== 204) reject(new Error("sync XHR status " + xhr.status));
                                            else resolve();
                                        }}
                                    }});
                                }}
                            }})()"#
                        );
                        let script = if worker {
                            let worker_source = serde_json::to_string(&format!(
                                "{send}.then(() => postMessage('ok'), error => postMessage(String(error)))"
                            ))
                            .expect("serialize worker source");
                            format!(
                                r#"
                                globalThis.__emptyHeaderResult = "pending";
                                const source = URL.createObjectURL(new Blob([{worker_source}]));
                                const worker = new Worker(source);
                                worker.onmessage = event => {{
                                    globalThis.__emptyHeaderResult = event.data;
                                    worker.terminate();
                                    URL.revokeObjectURL(source);
                                }};
                                worker.onerror = event => {{ globalThis.__emptyHeaderResult = event.message; }};
                                "#
                            )
                        } else {
                            format!(
                                "globalThis.__emptyHeaderResult = 'pending'; {send}.then(() => {{ globalThis.__emptyHeaderResult = 'ok'; }}, error => {{ globalThis.__emptyHeaderResult = String(error); }})"
                            )
                        };
                        let result = local_executor
                            .run(async move {
                                page_vm.vm_mut().eval(&script)?;
                                drive_websocket_until_done(
                                    &mut page_vm,
                                    "String(globalThis.__emptyHeaderResult !== 'pending')",
                                    "empty header request should complete",
                                )
                                .await?;
                                page_vm.vm_mut().eval("globalThis.__emptyHeaderResult")
                            })
                            .await
                            .expect("empty header test should run on owner lane");
                        assert_eq!(
                            result, "ok",
                            "worker={worker}, {api}, {method}, empty_content_type={empty_content_type}"
                        );
                        let request = server.join().expect("request capture server should finish");
                        let head = request.split("\r\n\r\n").next().expect("request head");
                        let head = head.to_ascii_lowercase();
                        assert!(head.lines().any(|line| line == "x-empty:"), "{request}");
                        let content_types = head
                            .lines()
                            .filter(|line| line.starts_with("content-type:"))
                            .collect::<Vec<_>>();
                        if empty_content_type {
                            assert_eq!(content_types, ["content-type:"], "{request}");
                        } else {
                            assert!(content_types.is_empty(), "{request}");
                        }
                    }
                }
            }
        }
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
async fn popup_initial_about_blank_fetch_inherits_opener_response_referrer_policy() {
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
                            globalThis.__popupFetchDone = false;
                            globalThis.__popupFetchOutcome = "";
                            const popup = window.open("about:blank");
                            popup.fetch("/api")
                                .then((response) => response.text())
                                .then(() => {
                                    globalThis.__popupFetchOutcome = "ok";
                                    globalThis.__popupFetchDone = true;
                                })
                                .catch((error) => {
                                    globalThis.__popupFetchOutcome =
                                        error.name + ":" + error.message;
                                    globalThis.__popupFetchDone = true;
                                });
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__popupFetchDone === true)",
                    "popup initial about:blank fetch should complete",
                )
                .await?;
                assert_eq!(page_vm.vm_mut().eval("globalThis.__popupFetchOutcome")?, "ok");
                anyhow::Ok(())
            })
            .await
            .expect("popup fetch referrer policy test should run on owner lane");

        let request = request_rx.await.expect("captured popup fetch request");
        server.await.expect("popup fetch capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("GET /api HTTP/1.1\r\n"));
        assert!(
            !request_lower.contains("\r\nreferer:"),
            "popup initial about:blank fetch must inherit opener Referrer-Policy: no-referrer; request was:\n{request}"
        );
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
async fn opener_calling_popup_fetch_uses_popup_response_csp() {
    run_page_vm_async_test(async move {
        let popup_body =
            r#"<!doctype html><script>opener.postMessage("popup-ready", "*");</script>"#.to_owned();
        let (base_url, server) = spawn_popup_document_with_response_csp_server(
            "/popup-csp.html",
            "connect-src 'none'",
            popup_body,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__popupFetchCspDone = false;
                            globalThis.__popupFetchCspObserved = null;
                            globalThis.__popupFetchCspEvents = [];
                            const popup = window.open("/popup-csp.html");
                            addEventListener("message", event => {
                                if (event.data !== "popup-ready") {
                                    return;
                                }
                                popup.addEventListener("securitypolicyviolation", event => {
                                    globalThis.__popupFetchCspEvents.push({
                                        blockedURI: event.blockedURI,
                                        effectiveDirective: event.effectiveDirective,
                                        disposition: event.disposition,
                                        instance: event instanceof SecurityPolicyViolationEvent,
                                    });
                                });
                                popup.fetch("data:text/plain,blocked").then(
                                    () => {
                                        globalThis.__popupFetchCspObserved = JSON.stringify({
                                            fulfilled: true,
                                            events: globalThis.__popupFetchCspEvents,
                                        });
                                    },
                                    error => {
                                        globalThis.__popupFetchCspObserved = JSON.stringify({
                                            name: error && error.name,
                                            isTypeError: error instanceof TypeError,
                                            hasCspMessage: String(error && error.message)
                                                .includes("Content Security Policy"),
                                            events: globalThis.__popupFetchCspEvents,
                                        });
                                    }
                                ).finally(() => {
                                    globalThis.__popupFetchCspDone = true;
                                });
                            });
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__popupFetchCspDone === true)",
                    "opener-issued popup fetch should obey popup response CSP",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__popupFetchCspObserved)")
            })
            .await
            .expect("popup fetch CSP test should run on owner lane");

        server
            .await
            .expect("popup response CSP server should finish");
        let observed: serde_json::Value =
            serde_json::from_str(&observed).expect("parse popup fetch CSP observation");
        assert_eq!(
            observed,
            json!({
                "name": "TypeError",
                "isTypeError": true,
                "hasCspMessage": true,
                "events": [{
                    "blockedURI": "data",
                    "effectiveDirective": "connect-src",
                    "disposition": "enforce",
                    "instance": true,
                }],
            })
        );
    })
    .await;
}

#[tokio::test]
async fn popup_response_websocket_uses_popup_response_csp() {
    run_page_vm_async_test(async move {
        let popup_body = r#"<!doctype html><script>
(() => {
  const events = [];
  addEventListener("securitypolicyviolation", event => {
    events.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent,
    });
  });
  const socket = new WebSocket("/socket");
  opener.__popupWebSocketCspObserved = JSON.stringify({
    url: socket.url,
    readyState: socket.readyState,
    events,
  });
  opener.postMessage("popup-websocket-csp-done", "*");
})();
</script>"#
            .to_owned();
        let (base_url, server) = spawn_popup_document_with_response_csp_server(
            "/popup-csp.html",
            "connect-src 'none'",
            popup_body,
        )
        .await;
        let expected_socket_url = format!("{base_url}/socket").replacen("http://", "ws://", 1);
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__popupWebSocketCspDone = false;
                            globalThis.__popupWebSocketCspObserved = null;
                            addEventListener("message", event => {
                                if (event.data === "popup-websocket-csp-done") {
                                    globalThis.__popupWebSocketCspDone = true;
                                }
                            });
                            window.open("/popup-csp.html");
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__popupWebSocketCspDone === true)",
                    "popup WebSocket should obey popup response CSP",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__popupWebSocketCspObserved)")
            })
            .await
            .expect("popup WebSocket CSP test should run on owner lane");

        server
            .await
            .expect("popup response WebSocket CSP server should finish");
        let observed: serde_json::Value =
            serde_json::from_str(&observed).expect("parse popup WebSocket CSP observation");
        assert_eq!(
            observed,
            json!({
                "url": expected_socket_url,
                "readyState": 0,
                "events": [{
                    "blockedURI": expected_socket_url,
                    "effectiveDirective": "connect-src",
                    "disposition": "enforce",
                    "instance": true,
                }],
            })
        );
    })
    .await;
}

#[tokio::test]
async fn popup_response_fetch_uses_response_policy_after_initial_no_referrer_inheritance() {
    run_page_vm_async_test(async move {
        let popup_body = r#"<!doctype html><script>
fetch("/api")
  .then(response => response.text())
  .then(() => opener.postMessage("popup-fetch-ok", "*"))
  .catch(error => opener.postMessage("popup-fetch-error:" + error.name + ":" + error.message, "*"));
</script>"#
            .to_owned();
        let (base_url, request_rx, server) =
            spawn_document_then_api_capture_server("/popup.html", popup_body).await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm =
            test_page_vm_with_response_referrer_policy(document_url, "no-referrer");
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__popupResponseFetchMessage = "";
                            addEventListener("message", event => {
                                globalThis.__popupResponseFetchMessage = String(event.data);
                            });
                            window.open("/popup.html");
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__popupResponseFetchMessage !== '')",
                    "popup response fetch should report completion",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__popupResponseFetchMessage")?,
                    "popup-fetch-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("popup response fetch policy test should run on owner lane");

        let request = request_rx.await.expect("captured popup response fetch");
        server
            .await
            .expect("popup response fetch capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("GET /api HTTP/1.1\r\n"));
        assert!(request_lower.contains("\r\nreferer: "));
        assert!(
            request_lower.contains("/popup.html\r\n"),
            "popup response commit must replace inherited no-referrer policy; request was:\n{request}"
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
async fn blob_fetch_and_xhr_reject_non_get_methods_in_window_and_worker() {
    check_blob_fetch_and_xhr_methods_in_window_and_worker(false).await;
}

#[tokio::test]
async fn blob_fetch_and_xhr_bypass_interception_in_window_and_worker() {
    check_blob_fetch_and_xhr_methods_in_window_and_worker(true).await;
}

#[tokio::test]
async fn request_init_exceptions_preserve_identity_without_fetching_or_consuming_input() {
    run_page_vm_async_test(async move {
        for worker in [false, true] {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();
            let probe = r#"(async () => {
                const check = (value, message) => { if (!value) throw new Error(message); };
                const url = 'data:text/plain,must-not-fetch';
                for (const api of ['Request', 'fetch']) {
                    for (const member of ['method', 'headers', 'signal']) {
                        for (const sentinel of [undefined, null, false, 0, 'sentinel', {}, new Error('sentinel')]) {
                            const input = new Request(url, {method:'POST', body:'kept'});
                            const init = {};
                            Object.defineProperty(init, member, {get() { throw sentinel; }});
                            let result, caught, threw = false;
                            try { result = api === 'Request' ? new Request(input, init) : fetch(input, init); }
                            catch (error) { threw = true; caught = error; }
                            if (api === 'Request') {
                                check(threw && Object.is(caught, sentinel), 'Request must rethrow ' + member);
                            } else {
                                check(!threw && result instanceof Promise, 'fetch conversion must return a Promise');
                                let rejected = false;
                                await result.then(() => {}, error => { rejected = true; caught = error; });
                                check(rejected && Object.is(caught, sentinel), 'fetch must reject with original ' + member);
                            }
                            check(!input.bodyUsed, 'failed ' + member + ' conversion consumed input body');
                        }
                    }
                }
                if (typeof document !== 'undefined') {
                    const frame = document.createElement('iframe');
                    document.body.appendChild(frame);
                    const other = frame.contentWindow;
                    const sentinel = new other.Error('cross-realm');
                    const promise = other.fetch.call(window, url, {get headers() { throw sentinel; }});
                    check(promise instanceof other.Promise, 'conversion rejection must belong to function realm');
                    let caught;
                    await promise.catch(error => { caught = error; });
                    check(caught === sentinel, 'cross-realm exception identity');
                    frame.remove();
                }
                return 'ok';
            })()"#;
            let script = if worker {
                let source = serde_json::to_string(&format!("{probe}.then(postMessage, error => postMessage(String(error)))")).unwrap();
                format!(r#"globalThis.__initExceptionResult = 'pending';
                    const source = URL.createObjectURL(new Blob([{source}]));
                    const worker = new Worker(source);
                    worker.onmessage = e => {{ globalThis.__initExceptionResult = e.data; worker.terminate(); URL.revokeObjectURL(source); }};
                    worker.onerror = e => {{ globalThis.__initExceptionResult = e.message; }};"#)
            } else {
                format!("globalThis.__initExceptionResult = 'pending'; {probe}.then(value => {{ globalThis.__initExceptionResult = value; }}, error => {{ globalThis.__initExceptionResult = String(error); }})")
            };
            let (result, network_output) = local_executor.run(async move {
                page_vm.vm_mut().eval(&script)?;
                drive_websocket_until_done(&mut page_vm, "String(globalThis.__initExceptionResult !== 'pending')", "RequestInit exception checks should finish").await?;
                let result = page_vm.vm_mut().eval("globalThis.__initExceptionResult")?;
                Ok::<_, anyhow::Error>((result, page_vm.vm_mut().take_network_output()))
            }).await.expect("RequestInit exceptions should run on owner lane");
            assert_eq!(result, "ok", "worker={worker}");
            let (records, _, _) = split_network_output_items(network_output);
            assert!(records.iter().all(|record| record.resource_type() != SubresourceResourceType::Fetch), "failed argument conversion dispatched a fetch; worker={worker}");
        }
    }).await;
}

#[tokio::test]
async fn blob_url_entries_survive_request_cloning_and_xhr_open_in_window_and_worker() {
    run_page_vm_async_test(async move {
        for worker in [false, true] {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();
            let probe = r#"(async () => {
                const check = (value, message) => { if (!value) throw new Error(message); };
                const make = () => URL.createObjectURL(new Blob(['payload'], {type: 'text/plain'}));
                const read = async (request) => {
                    const response = await fetch(request);
                    check(response.status === 200 && !response.url.includes('#'), 'response metadata');
                    check(await response.text() === 'payload', 'captured payload');
                };
                const rejects = async input => {
                    let failure;
                    try { await fetch(input); } catch (error) { failure = error; }
                    check(failure instanceof TypeError, 'fresh revoked URL must reject');
                };
                for (const fragment of ['', '#fragment']) {
                    const url = make();
                    const request = new Request(url + fragment);
                    const before = request.clone();
                    URL.revokeObjectURL(url);
                    for (const copy of [request, before, request.clone(), new Request(request)]) await read(copy);
                    await rejects(url + fragment);
                    await rejects(new Request(url + fragment));

                    for (const inherited of [false, true]) {
                        for (const member of ['method', 'headers', 'signal']) {
                            const getterUrl = make();
                            const input = inherited ? new Request(getterUrl + fragment) : getterUrl + fragment;
                            const init = {};
                            Object.defineProperty(init, member, {get() {
                                URL.revokeObjectURL(getterUrl);
                                return member === 'method' ? 'GET' : member === 'headers' ? [] : null;
                            }});
                            const copy = new Request(input, init);
                            if (inherited) await read(copy); else await rejects(copy);
                        }
                    }
                    const immediateUrl = make();
                    const pending = fetch(immediateUrl + fragment);
                    URL.revokeObjectURL(immediateUrl);
                    check(await (await pending).text() === 'payload', 'fetch must capture before returning');

                    for (const async of [false, true]) {
                        for (const reopen of [false, true]) {
                            const xhrUrl = make();
                            const xhr = new XMLHttpRequest();
                            xhr.open('GET', xhrUrl + fragment, async);
                            URL.revokeObjectURL(xhrUrl);
                            try { xhr.open('GET', 'http://['); } catch (error) {
                                check(error.name === 'SyntaxError', 'failed open must preserve previous request');
                            }
                            if (reopen) xhr.open('GET', xhrUrl + fragment, async);
                            if (async) {
                                await new Promise(resolve => { xhr.onloadend = resolve; xhr.send(); });
                            } else {
                                let failure;
                                try { xhr.send(); } catch (error) { failure = error; }
                                check(reopen ? failure?.name === 'NetworkError' : !failure, 'synchronous outcome');
                            }
                            check(xhr.status === (reopen ? 0 : 200), 'XHR reopened entry status');
                            check(xhr.responseText === (reopen ? '' : 'payload'), 'XHR captured body');
                        }
                    }
                }
                return 'ok';
            })()"#;
            let script = if worker {
                let source = serde_json::to_string(&format!(
                    "{probe}.then(postMessage, error => postMessage(String(error)))"
                )).unwrap();
                format!(r#"globalThis.__snapshotResult = 'pending';
                    const source = URL.createObjectURL(new Blob([{source}]));
                    const worker = new Worker(source);
                    worker.onmessage = e => {{ globalThis.__snapshotResult = e.data; worker.terminate(); URL.revokeObjectURL(source); }};
                    worker.onerror = e => {{ globalThis.__snapshotResult = e.message; }};"#)
            } else {
                format!("globalThis.__snapshotResult = 'pending'; {probe}.then(value => {{ globalThis.__snapshotResult = value; }}, error => {{ globalThis.__snapshotResult = String(error); }})")
            };
            let result = local_executor.run(async move {
                page_vm.vm_mut().eval(&script)?;
                drive_websocket_until_done(&mut page_vm, "String(globalThis.__snapshotResult !== 'pending')", "blob entry lifetime checks should finish").await?;
                page_vm.vm_mut().eval("globalThis.__snapshotResult")
            }).await.expect("blob entry probe should run on owner lane");
            assert_eq!(result, "ok", "worker={worker}");
        }
    }).await;
}

#[tokio::test]
async fn intercepted_blob_url_requests_keep_their_entry_and_respect_url_and_method_overrides() {
    run_page_vm_async_test(async move {
        for (worker, xhr) in [(false, false), (false, true), (true, false)] {
            for change in ["none", "fragment", "url", "method"] {
                let mut page_vm = test_page_vm();
                let local_executor = page_vm.local_executor.clone();
                let result = local_executor.run(async move {
                    page_vm.vm_mut().set_fetch_subresource_interception(true, Some(if xhr {
                        SubresourceResourceType::Xhr
                    } else {
                        SubresourceResourceType::Fetch
                    }));
                    let probe = format!(r#"(() => {{
                        const original = URL.createObjectURL(new Blob(['payload']));
                        const other = URL.createObjectURL(new Blob(['replacement']));
                        const finish = value => {{
                            URL.revokeObjectURL(other);
                            {finish}
                        }};
                        if ({xhr}) {{
                            const xhr = new XMLHttpRequest();
                            xhr.open('GET', original + '#original');
                            URL.revokeObjectURL(original);
                            xhr.onload = () => finish(xhr.responseText);
                            xhr.onerror = () => finish('error');
                            xhr.send();
                        }} else {{
                            const request = new Request(original + '#original');
                            URL.revokeObjectURL(original);
                            fetch(request).then(r => r.text()).then(finish, () => finish('error'));
                        }}
                        {ready}
                    }})()"#,
                        finish = if worker { "postMessage({value});" } else { "globalThis.__snapshotResult = value;" },
                        ready = if worker { "postMessage({ready: true, other});" } else { "globalThis.__snapshotReady = true; globalThis.__snapshotOther = other;" },
                    );
                    let script = if worker {
                        let source = serde_json::to_string(&probe).unwrap();
                        format!(r#"globalThis.__snapshotResult = 'pending';
                            const source = URL.createObjectURL(new Blob([{source}]));
                            const worker = new Worker(source);
                            worker.onmessage = e => {{
                                if (e.data.ready) {{ globalThis.__snapshotReady = true; globalThis.__snapshotOther = e.data.other; }}
                                else {{ globalThis.__snapshotResult = e.data.value; worker.terminate(); URL.revokeObjectURL(source); }}
                            }};"#)
                    } else {
                        format!("globalThis.__snapshotResult = 'pending'; {probe}")
                    };
                    page_vm.vm_mut().eval(&script)?;
                    drive_websocket_until_done(&mut page_vm, "String(globalThis.__snapshotReady === true)", "blob request should reach interception").await?;
                    let pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
                    assert_eq!(pending.len(), 1, "worker={worker}, xhr={xhr}, change={change}");
                    let pending = &pending[0];
                    let url = match change {
                        "fragment" => { let mut url = pending.url.clone(); url.set_fragment(Some("new")); Some(url) }
                        "url" => Some(Url::parse(&page_vm.vm_mut().eval("globalThis.__snapshotOther")?)?),
                        _ => None,
                    };
                    let method = (change == "method").then(|| "POST".to_owned());
                    page_vm.continue_pending_subresource_fetch(pending.internal_id, url, method, None, None, false, false)?;
                    drive_websocket_until_done(&mut page_vm, "String(globalThis.__snapshotResult !== 'pending')", "continued blob request should complete").await?;
                    page_vm.vm_mut().eval("globalThis.__snapshotResult")
                }).await.expect("intercepted blob request should run on owner lane");
                assert_eq!(result, match change { "url" => "replacement", "method" => "error", _ => "payload" }, "worker={worker}, xhr={xhr}, change={change}");
            }
        }
    }).await;
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
                                urlMatchesRequest: response.url === {fetch_url_literal},
                                bodyIsNull: response.body === null,
                                headers: Array.from(response.headers),
                                bodyUsedBefore,
                                bodyUsedAfter: response.bodyUsed,
                                text,
                                cloneType: clone.type,
                                cloneStatus: clone.status,
                                cloneUrlMatchesRequest: clone.url === {fetch_url_literal},
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
                r#"{"type":"opaqueredirect","status":0,"ok":false,"statusText":"","redirected":false,"urlMatchesRequest":true,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":false,"text":"","cloneType":"opaqueredirect","cloneStatus":0,"cloneUrlMatchesRequest":true,"cloneBodyIsNull":true,"cloneText":""}"#
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
                header.0.eq_ignore_ascii_case("location") && header.1 == b"/target"
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
                    // The opaque response resolves at headers, before its hidden
                    // body finishes and produces the completed network record.
                    drain_page_work_until_no_pending_subresources(
                        &mut page_vm,
                        "fetch no-cors network record should complete",
                    ).await?;
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
                r#"{"type":"opaque","status":0,"ok":false,"statusText":"","url":"","redirected":false,"bodyIsNull":true,"headers":[],"bodyUsedBefore":false,"bodyUsedAfter":false,"text":"","cloneType":"opaque","cloneStatus":0,"cloneBodyIsNull":true,"cloneText":""}"#
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
                    // The opaque response resolves at its headers. Its ORB
                    // network terminal arrives after the body has been checked.
                    drain_page_work_until_no_pending_subresources(
                        &mut page_vm,
                        "fetch no-cors ORB network record should complete",
                    ).await?;
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
async fn request_and_fetch_argument_errors_preserve_exceptions_without_side_effects() {
    run_page_vm_async_test(async move {
        for worker in [false, true] {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();
            let probe = r#"(async () => {
                const check = (value, message) => { if (!value) throw new Error(message); };
                const url = 'data:text/plain,must-not-fetch';
                for (const api of ['Request', 'fetch']) {
                    for (const member of ['method', 'headers', 'signal', 'body']) {
                        for (const sentinel of [undefined, null, false, 0, 'sentinel', {}, new Error('sentinel')]) {
                            const input = new Request(url, {method:'POST', body:'kept'});
                            const init = {};
                            Object.defineProperty(init, member, {get() { throw sentinel; }});
                            let result, caught, threw = false;
                            try { result = api === 'Request' ? new Request(input, init) : fetch(input, init); }
                            catch (error) { threw = true; caught = error; }
                            if (api === 'Request') {
                                check(threw && Object.is(caught, sentinel), 'Request must rethrow ' + member);
                            } else {
                                check(!threw && result instanceof Promise, 'fetch conversion must return a Promise');
                                let rejected = false;
                                await result.then(() => {}, error => { rejected = true; caught = error; });
                                check(rejected && Object.is(caught, sentinel), 'fetch must reject with original ' + member);
                            }
                            check(!input.bodyUsed, 'failed ' + member + ' conversion consumed input body');
                        }
                    }
                }
                const rejection = async promise => {
                    check(promise instanceof Promise, 'fetch conversion must return a Promise');
                    let rejected = false, caught;
                    await promise.then(() => {}, error => { rejected = true; caught = error; });
                    check(rejected, 'invalid arguments must reject');
                    return caught;
                };
                for (const sentinel of [undefined, null, false, 0, 'sentinel', {}, new Error('sentinel')]) {
                    const throwSentinel = () => { throw sentinel; };
                    const inits = [
                        {method: {toString: throwSentinel}},
                        {headers: {[Symbol.iterator]: throwSentinel}},
                        ...['method', 'body', 'headers', 'signal'].map(member => new Proxy({}, {
                            has(target, key) {
                                if (key === member) throw sentinel;
                                return Reflect.has(target, key);
                            },
                        })),
                    ];
                    for (const init of inits) {
                        const input = new Request(url, {method: 'POST', body: 'kept'});
                        check(Object.is(await rejection(fetch(input, init)), sentinel), 'nested conversion exception identity');
                        check(!input.bodyUsed, 'nested conversion failure consumed input body');
                    }
                    check(Object.is(await rejection(fetch({toString: throwSentinel})), sentinel), 'URL conversion exception identity');
                }
                const interfaceName = typeof document === 'undefined' ? 'DedicatedWorkerGlobalScope' : 'Window';
                const noCorsInterface = typeof document === 'undefined' ? " on 'DedicatedWorkerGlobalScope'" : '';
                for (const [init, message] of [
                    [{method: 'TRACE'}, 'Request method is forbidden'],
                    [{method: 'GET', body: 'invalid'}, 'Request with GET/HEAD method cannot have body'],
                    [{mode: 'invalid'}, 'RequestInit: mode is not a valid enum value of type RequestMode'],
                    [{signal: false}, `Failed to execute 'fetch' on '${interfaceName}': signal must be an AbortSignal.`],
                    [{signal: {}}, `Failed to execute 'fetch' on '${interfaceName}': signal must be an AbortSignal.`],
                    [{mode: 'no-cors', method: 'PATCH'}, `Failed to execute 'fetch'${noCorsInterface}: method \`PATCH\` is unsupported in no-cors mode.`],
                ]) {
                    const input = new Request(url, {method: 'POST', body: 'kept'});
                    const error = await rejection(fetch(input, init));
                    check(error instanceof TypeError, 'validation must reject with a TypeError');
                    check(error.message === message, 'validation message: ' + error.message);
                    check(!input.bodyUsed, 'validation failure consumed input body');
                }
                const missing = await rejection(fetch());
                check(missing instanceof TypeError && missing.message === 'fetch: Argument 1 is required', 'missing argument validation');
                const badUrl = await rejection(fetch('http://['));
                check(badUrl instanceof TypeError && badUrl.message.startsWith('failed to resolve url `http://[`:'), 'invalid URL validation');
                if (typeof document !== 'undefined') {
                    const frame = document.createElement('iframe');
                    document.body.appendChild(frame);
                    const other = frame.contentWindow;
                    const sentinel = new other.Error('cross-realm');
                    const promise = other.fetch.call(window, url, {get headers() { throw sentinel; }});
                    check(promise instanceof other.Promise, 'conversion rejection must belong to function realm');
                    let caught;
                    await promise.catch(error => { caught = error; });
                    check(caught === sentinel, 'cross-realm exception identity');
                    const invalidSignal = other.fetch.call(window, url, {signal: false});
                    check(invalidSignal instanceof other.Promise, 'validation rejection must belong to function realm');
                    await invalidSignal.catch(error => { caught = error; });
                    check(caught instanceof other.TypeError, 'validation TypeError must belong to function realm');
                    const invalidUrl = other.fetch.call(window, 'http://[');
                    check(invalidUrl instanceof Promise, 'preparation rejection must belong to receiver realm');
                    await invalidUrl.catch(error => { caught = error; });
                    check(caught instanceof TypeError && caught.message.startsWith('failed to resolve url `http://[`:'), 'preparation TypeError must belong to receiver realm');
                    frame.remove();
                }
                return 'ok';
            })()"#;
            let script = if worker {
                let source = serde_json::to_string(&format!("{probe}.then(postMessage, error => postMessage(String(error)))")).unwrap();
                format!(r#"globalThis.__initExceptionResult = 'pending';
                    const source = URL.createObjectURL(new Blob([{source}]));
                    const worker = new Worker(source);
                    worker.onmessage = e => {{ globalThis.__initExceptionResult = e.data; worker.terminate(); URL.revokeObjectURL(source); }};
                    worker.onerror = e => {{ globalThis.__initExceptionResult = e.message; }};"#)
            } else {
                format!("globalThis.__initExceptionResult = 'pending'; {probe}.then(value => {{ globalThis.__initExceptionResult = value; }}, error => {{ globalThis.__initExceptionResult = String(error); }})")
            };
            let (result, network_output) = local_executor.run(async move {
                page_vm.vm_mut().eval(&script)?;
                drive_websocket_until_done(&mut page_vm, "String(globalThis.__initExceptionResult !== 'pending')", "RequestInit exception checks should finish").await?;
                let result = page_vm.vm_mut().eval("globalThis.__initExceptionResult")?;
                Ok::<_, anyhow::Error>((result, page_vm.vm_mut().take_network_output()))
            }).await.expect("RequestInit exceptions should run on owner lane");
            assert_eq!(result, "ok", "worker={worker}");
            let (records, _, _) = split_network_output_items(network_output);
            assert!(records.iter().all(|record| record.resource_type() != SubresourceResourceType::Fetch), "failed argument conversion dispatched a fetch; worker={worker}");
        }
    }).await;
}
