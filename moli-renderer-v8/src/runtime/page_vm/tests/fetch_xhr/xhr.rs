use super::*;

#[tokio::test]
async fn cross_site_child_credentialed_xhr_reports_active_storage_access() {
    run_page_vm_async_test(async move {
        let child_body = r#"<!doctype html><script>
const xhr = new XMLHttpRequest();
xhr.open("POST", "/api");
xhr.withCredentials = true;
xhr.onload = () => parent.postMessage("child-xhr-ok", "*");
xhr.onerror = () => parent.postMessage("child-xhr-error", "*");
xhr.send();
</script>"#
            .to_owned();
        let (child_origin, request_rx, server) =
            spawn_document_then_api_capture_server("/child.html", child_body).await;
        let document_url = Url::parse("http://top.test/page.html").expect("document url");
        let child_url = serde_json::to_string(&format!("{child_origin}/child.html"))
            .expect("serialize child URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__childStorageAccessMessage = "";
                            addEventListener("message", event => {{
                                globalThis.__childStorageAccessMessage = String(event.data);
                            }});
                            const frame = document.createElement("iframe");
                            frame.src = {child_url};
                            document.body.append(frame);
                        }})()
                        "#,
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__childStorageAccessMessage !== '')",
                    "credentialed cross-site child XHR should report completion",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__childStorageAccessMessage")?,
                    "child-xhr-ok"
                );
                anyhow::Ok(())
            })
            .await
            .expect("child storage-access XHR test should run on owner lane");

        let request = request_rx
            .await
            .expect("captured credentialed child XHR request");
        server
            .await
            .expect("child storage-access capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("POST /api HTTP/1.1\r\n"));
        assert!(
            request_lower.contains("\r\nsec-fetch-site: same-origin\r\n"),
            "child XHR initiator must be its committed Document; request was:\n{request}"
        );
        assert!(
            request_lower.contains("\r\nsec-fetch-storage-access: active\r\n"),
            "credentialed child XHR must expose its active third-party cookie access; request was:\n{request}"
        );
    })
    .await;
}

#[tokio::test]
async fn xhr_emits_browser_style_subresource_headers_on_wire() {
    run_page_vm_async_test(async move {
        let (base_url, request_rx, server) = spawn_header_capture_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__xhrDone = false;
                            const xhr = new XMLHttpRequest();
                            xhr.open("GET", {xhr_url_literal});
                            xhr.setRequestHeader("X-Test", "xhr");
                            xhr.onload = () => {{
                                globalThis.__xhrDone = true;
                            }};
                            xhr.send();
                        }})()
                        "#
                ))?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__xhrDone === true)",
                    "xhr header capture request should complete",
                )
                .await
            })
            .await
            .expect("xhr header capture test should run on owner lane");

        let request = request_rx.await.expect("captured xhr request");
        server.await.expect("header capture server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert!(request.starts_with("GET /xhr HTTP/1.1\r\n"));
        assert!(request_lower.contains("x-test: xhr\r\n"));
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
async fn xhr_load_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_single_response_http_server(
            "HTTP/1.1 200 OK",
            "xhr-ok".to_owned(),
            Duration::ZERO,
        )
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let (
            completion_sources,
            events_after_xhr,
            script_ready_source,
            events_after_script_ready,
            lifecycle_and_host_load_sources,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__xhrReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const xhr = new XMLHttpRequest();
  xhr.onload = () => {{
    __xhrReadyEvents.push("xhr-load:" + xhr.responseText);
    const frame = document.createElement("iframe");
    frame.onload = () => __xhrReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__xhrReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  }};
  xhr.open("GET", {xhr_url_literal});
  xhr.send();
}})()
"#
                ))?;

                let mut completion_sources = Vec::new();
                let events_after_xhr = loop {
                    if !page_vm.page_resource_completion_queue().has_ready_completion() {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("xhr completion should arrive before timeout");
                    }
                    let completion =
                        run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    completion_sources.push(completion.action.source());
                    let events = page_vm.vm_mut().eval("__xhrReadyEvents.join('|')")?;
                    if events == "xhr-load:xhr-ok" {
                        break events;
                    }
                    assert!(
                        completion_sources.len() < 8,
                        "xhr load should dispatch after a bounded number of completions; sources: {completion_sources:?}, events: {events}"
                    );
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "XHR-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "XHR-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm.vm_mut().eval("__xhrReadyEvents.join('|')")?;
                let mut lifecycle_and_host_load_sources = Vec::new();
                let events_after_host_load = loop {
                    let source = page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await
                        .expect("child lifecycle or HostLoad source should remain ready");
                    lifecycle_and_host_load_sources.push(source);
                    let events = page_vm.vm_mut().eval("__xhrReadyEvents.join('|')")?;
                    if events == "xhr-load:xhr-ok|child-script:true|frame-load" {
                        break events;
                    }
                    assert_eq!(
                        source,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "only document-owned lifecycle turns may precede final HostLoad delivery"
                    );
                    assert!(
                        lifecycle_and_host_load_sources.len() < 8,
                        "XHR-created child lifecycle should reach HostLoad in bounded owner turns: {lifecycle_and_host_load_sources:?}"
                    );
                };

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_xhr,
                    script_ready_source,
                    events_after_script_ready,
                    lifecycle_and_host_load_sources,
                    events_after_host_load,
                ))
            })
            .await
            .expect("xhr ready-work source test should run");

        assert!(
            completion_sources
                .iter()
                .all(|source| *source == RendererOwnerResourceActivitySource::AsyncSubresource),
            "XHR load should be driven only by async-subresource completions: {completion_sources:?}"
        );
        assert_eq!(
            events_after_xhr, "xhr-load:xhr-ok",
            "XHR load handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "XHR-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "xhr-load:xhr-ok|child-script:true",
            "child parser work should run on the later DocumentScriptReady turn"
        );
        assert!(
            lifecycle_and_host_load_sources.len() >= 2,
            "document-owned lifecycle must complete before HostLoad: {lifecycle_and_host_load_sources:?}"
        );
        assert!(
            lifecycle_and_host_load_sources[..lifecycle_and_host_load_sources.len() - 1]
                .iter()
                .all(|source| *source == ChildFrameSemanticTurnKind::DocumentLifecycle),
            "only DocumentLifecycle turns may run between XHR-created script execution and load delivery: {lifecycle_and_host_load_sources:?}"
        );
        assert_eq!(
            lifecycle_and_host_load_sources.last(),
            Some(&ChildFrameSemanticTurnKind::HostLoad),
            "iframe load must remain a later HostLoad turn after document lifecycle"
        );
        assert_eq!(
            events_after_host_load, "xhr-load:xhr-ok|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server.await.expect("xhr ready-work server should finish");
    })
    .await;
}

#[tokio::test]
async fn xhr_event_target_inherits_event_target_methods_without_own_shadowing() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.addEventListener("readystatechange", () => events.push("listener"));
                        xhr.onreadystatechange = () => events.push("property");
                        xhr.dispatchEvent(new Event("readystatechange"));
                        return JSON.stringify({
                            xhrEventTargetOwnAdd: Object.hasOwn(XMLHttpRequestEventTarget.prototype, "addEventListener"),
                            xhrEventTargetOwnRemove: Object.hasOwn(XMLHttpRequestEventTarget.prototype, "removeEventListener"),
                            xhrEventTargetOwnDispatch: Object.hasOwn(XMLHttpRequestEventTarget.prototype, "dispatchEvent"),
                            inheritedName: XMLHttpRequestEventTarget.prototype.addEventListener.name,
                            inheritedLength: XMLHttpRequestEventTarget.prototype.addEventListener.length,
                            instanceOfEventTarget: xhr instanceof EventTarget,
                            instanceOfXhrEventTarget: xhr instanceof XMLHttpRequestEventTarget,
                            events,
                        });
                    })()
                    "#,
                )
            })
            .await
            .expect("xhr EventTarget prototype test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"xhrEventTargetOwnAdd":false,"xhrEventTargetOwnRemove":false,"xhrEventTargetOwnDispatch":false,"inheritedName":"addEventListener","inheritedLength":2,"instanceOfEventTarget":true,"instanceOfXhrEventTarget":true,"events":["listener","property"]}"#
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_blocks_send_and_returns_materialized_response() {
    run_page_vm_async_test(async move {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind sync XHR test server");
        let addr = listener.local_addr().expect("sync XHR server local addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let body = "sync-ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write sync XHR response");
            })
            .expect("spawn sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        for (const type of ["loadstart", "progress", "load", "loadend"]) {{
                            xhr.addEventListener(type, event => events.push(
                                `${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                            ));
                            xhr.upload.addEventListener(type, event => events.push(
                                `upload.${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                            ));
                        }}
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                            statusText: xhr.statusText,
                            responseText: xhr.responseText,
                            responseURL: xhr.responseURL,
                            contentType: xhr.getResponseHeader("Content-Type"),
                            allHeaders: xhr.getAllResponseHeaders(),
                        }});
                    }})()
                    "#
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("sync XHR test should run on owner lane");

        server.join().expect("sync XHR server should finish");
        assert_eq!(
            observed,
            format!(
                r#"{{"events":["readystatechange:1","readystatechange:4","load:7:7:true","loadend:7:7:true"],"readyState":4,"status":200,"statusText":"OK","responseText":"sync-ok","responseURL":"{xhr_url}","contentType":"text/plain; charset=utf-8","allHeaders":"connection: close\r\ncontent-length: 7\r\ncontent-type: text/plain; charset=utf-8\r\n"}}"#
            )
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.url().as_str(), xhr_url);
        assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
        let SubresourceNetworkOutcome::Success { status, .. } = record.outcome() else {
            panic!("expected sync XHR network success, got {:?}", record.outcome());
        };
        assert_eq!(*status, 200);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_rejects_cross_origin_response_without_cors_headers() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-deny",
            "cross-origin-secret",
            vec![],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-deny");
        let document_url = Url::parse("http://source.test/page.html").expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("cross-origin synchronous XHR probe should run on owner lane");

        server
            .join()
            .expect("cross-origin synchronous XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text.contains("CORS check failed: no Access-Control-Allow-Origin")
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_allows_cross_origin_response_with_matching_cors_origin() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-allow",
            "cors-visible",
            vec![
                ("Access-Control-Allow-Origin", "http://source.test"),
                ("Access-Control-Expose-Headers", "X-Visible-Token"),
                ("X-Visible-Token", "public-value"),
                ("X-Internal-Token", "private-value"),
            ],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-allow");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize XHR URL");
        let document_url = Url::parse("http://source.test/page.html").expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
                            readyState: xhr.readyState,
                            status: xhr.status,
                            responseText: xhr.responseText,
                            responseURL: xhr.responseURL,
                            visibleToken: xhr.getResponseHeader("X-Visible-Token"),
                            internalToken: xhr.getResponseHeader("X-Internal-Token"),
                            allowOrigin: xhr.getResponseHeader("Access-Control-Allow-Origin"),
                        }});
                    }})()
                    "#,
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("allowed cross-origin synchronous XHR should run on owner lane");

        server
            .join()
            .expect("allowed cross-origin synchronous XHR server should finish");
        assert_eq!(
            observed,
            format!(
                r#"{{"readyState":4,"status":200,"responseText":"cors-visible","responseURL":"{xhr_url}","visibleToken":"public-value","internalToken":null,"allowOrigin":null}}"#
            )
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Success { status: 200, .. }
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_rejects_cross_origin_response_with_mismatched_cors_origin() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-mismatch",
            "must-not-be-visible",
            vec![("Access-Control-Allow-Origin", "http://other.test")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-mismatch");
        let observed_and_network = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_failure_probe_expression(&xhr_url),
        )
        .await;

        server
            .join()
            .expect("mismatched-origin synchronous XHR server should finish");
        let (observed, network_output) = observed_and_network;
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        assert_single_synchronous_xhr_network_failure(
            network_output,
            "Access-Control-Allow-Origin `http://other.test` does not allow http://source.test",
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_allows_wildcard_cors_without_credentials() {
    run_page_vm_async_test(async move {
        let body = "wildcard-visible";
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-wildcard",
            body,
            vec![("Access-Control-Allow-Origin", "*")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-wildcard");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_success_probe_expression(&xhr_url, false),
        )
        .await;

        server
            .join()
            .expect("wildcard synchronous XHR server should finish");
        assert_synchronous_xhr_success_surface(&observed, &xhr_url, body);
        assert_single_synchronous_xhr_network_success(network_output);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_rejects_wildcard_cors_with_credentials() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-wildcard-credentials",
            "must-not-be-visible",
            vec![
                ("Access-Control-Allow-Origin", "*"),
                ("Access-Control-Allow-Credentials", "true"),
            ],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-wildcard-credentials");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_failure_probe_expression_with_credentials(&xhr_url, true),
        )
        .await;

        server
            .join()
            .expect("credentialed wildcard synchronous XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        assert_single_synchronous_xhr_network_failure(
            network_output,
            "wildcard Access-Control-Allow-Origin does not allow credentialed requests",
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_requires_allow_credentials_for_credentialed_cors() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-missing-credentials",
            "must-not-be-visible",
            vec![("Access-Control-Allow-Origin", "http://source.test")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-missing-credentials");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_failure_probe_expression_with_credentials(&xhr_url, true),
        )
        .await;

        server
            .join()
            .expect("missing-credentials synchronous XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        assert_single_synchronous_xhr_network_failure(
            network_output,
            "require Access-Control-Allow-Credentials: true",
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_allows_credentialed_cors_with_explicit_opt_in() {
    run_page_vm_async_test(async move {
        let body = "credentialed-visible";
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-cors-credentials-allow",
            body,
            vec![
                ("Access-Control-Allow-Origin", "http://source.test"),
                ("Access-Control-Allow-Credentials", "true"),
            ],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-cors-credentials-allow");
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse("http://source.test/page.html").expect("document url"),
            synchronous_xhr_success_probe_expression(&xhr_url, true),
        )
        .await;

        server
            .join()
            .expect("credentialed synchronous XHR server should finish");
        assert_synchronous_xhr_success_surface(&observed, &xhr_url, body);
        assert_single_synchronous_xhr_network_success(network_output);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_same_origin_response_keeps_non_cors_headers_visible() {
    run_page_vm_async_test(async move {
        let (target_base_url, server) = spawn_blocking_xhr_response_server(
            "/sync-xhr-same-origin-headers",
            "same-origin-visible",
            vec![("X-Same-Origin-Token", "same-origin-secret")],
        );
        let xhr_url = format!("{target_base_url}/sync-xhr-same-origin-headers");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize XHR URL");
        let expression = format!(
            r#"
            (() => {{
                const xhr = new XMLHttpRequest();
                xhr.open("GET", {xhr_url_literal}, false);
                xhr.send();
                return JSON.stringify({{
                    status: xhr.status,
                    responseText: xhr.responseText,
                    token: xhr.getResponseHeader("X-Same-Origin-Token"),
                }});
            }})()
            "#,
        );
        let (observed, network_output) = evaluate_synchronous_xhr_probe(
            Url::parse(&format!("{target_base_url}/page.html")).expect("document url"),
            expression,
        )
        .await;

        server
            .join()
            .expect("same-origin synchronous XHR server should finish");
        assert_eq!(
            observed,
            r#"{"status":200,"responseText":"same-origin-visible","token":"same-origin-secret"}"#
        );
        assert_single_synchronous_xhr_network_success(network_output);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_uses_chromium_progress_totals_without_progress_events() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind synchronous XHR progress server");
        let addr = listener
            .local_addr()
            .expect("synchronous XHR progress server address");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("sync-xhr-progress-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                for expected_path in ["/without-length", "/no-content"] {
                    let (mut stream, _) = listener
                        .accept()
                        .expect("accept synchronous XHR progress request");
                    let mut request = Vec::new();
                    let mut byte = [0_u8; 1];
                    loop {
                        stream
                            .read_exact(&mut byte)
                            .expect("read synchronous XHR progress request");
                        request.push(byte[0]);
                        if request.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8(request).expect("request should be UTF-8");
                    assert!(request.starts_with(&format!("GET {expected_path} HTTP/1.1\r\n")));
                    let response = if expected_path == "/without-length" {
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nOK"
                    } else {
                        "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
                    };
                    stream
                        .write_all(response.as_bytes())
                        .expect("write synchronous XHR progress response");
                }
            })
            .expect("spawn synchronous XHR progress server");

        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let base_url_literal = serde_json::to_string(&base_url).expect("serialize base URL");
        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                      const probe = url => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push(`readystatechange:${{xhr.readyState}}`);
                        for (const type of ["loadstart", "progress", "load", "loadend"]) {{
                          xhr.addEventListener(type, event => events.push(
                            `${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                          ));
                          xhr.upload.addEventListener(type, event => events.push(
                            `upload.${{type}}:${{event.loaded}}:${{event.total}}:${{event.lengthComputable}}`
                          ));
                        }}
                        xhr.open("GET", url, false);
                        xhr.send("ignored body");
                        return {{url, events, status: xhr.status, responseText: xhr.responseText}};
                      }};
                      return JSON.stringify([
                        probe({base_url_literal} + "/without-length"),
                        probe({base_url_literal} + "/no-content"),
                        probe("data:text/plain,ok")
                      ]);
                    }})()
                    "#
                ))
            })
            .await
            .expect("synchronous XHR progress probe should run on owner lane");

        server
            .join()
            .expect("synchronous XHR progress server should finish");
        let observed: serde_json::Value =
            serde_json::from_str(&observed).expect("progress probe should return JSON");
        assert_eq!(
            observed[0],
            serde_json::json!({
                "url": format!("{base_url}/without-length"),
                "events": [
                    "readystatechange:1",
                    "readystatechange:4",
                    "load:2:0:false",
                    "loadend:2:0:false"
                ],
                "status": 200,
                "responseText": "OK"
            })
        );
        assert_eq!(
            observed[1],
            serde_json::json!({
                "url": format!("{base_url}/no-content"),
                "events": [
                    "readystatechange:1",
                    "readystatechange:4",
                    "load:0:0:false",
                    "loadend:0:0:false"
                ],
                "status": 204,
                "responseText": ""
            })
        );
        assert_eq!(
            observed[2],
            serde_json::json!({
                "url": "data:text/plain,ok",
                "events": [
                    "readystatechange:1",
                    "readystatechange:4",
                    "load:2:2:true",
                    "loadend:2:2:true"
                ],
                "status": 200,
                "responseText": "ok"
            })
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_normalizes_lowercase_standard_method_before_fetch() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind lowercase sync XHR test server");
        let addr = listener
            .local_addr()
            .expect("lowercase sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("lowercase-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let body = "lowercase-ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write sync XHR response");
                String::from_utf8(request).expect("request should be utf-8")
            })
            .expect("spawn lowercase sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/sync-xhr-lowercase");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        xhr.open("get", {xhr_url_literal}, false);
                        xhr.send("ignored body");
                        return `${{xhr.status}}|${{xhr.responseText}}`;
                    }})()
                    "#
                ))
            })
            .await
            .expect("lowercase sync XHR test should run on owner lane");
        let request = server.join().expect("lowercase sync XHR server should finish");
        let request_lower = request.to_ascii_lowercase();

        assert_eq!(observed, "200|lowercase-ok");
        assert!(request.starts_with("GET /sync-xhr-lowercase HTTP/1.1\r\n"));
        assert!(!request_lower.contains("content-length:"));
        assert!(!request_lower.contains("content-type:"));
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_without_timeout_waits_for_slow_response() {
    run_page_vm_async_test(async move {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind slow sync XHR test server");
        let addr = listener.local_addr().expect("slow sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("slow-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept slow sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read slow sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(150));
                let body = "late";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            })
            .expect("spawn slow sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/slow-sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let started = Instant::now();
        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        xhr.onerror = () => events.push("error");
                        xhr.onload = () => events.push("load");
                        xhr.onloadend = () => events.push("loadend");
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                            responseText: xhr.responseText,
                        }});
                    }})()
                    "#
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("slow sync XHR test should run on owner lane");
        let elapsed = started.elapsed();

        server.join().expect("slow sync XHR server should finish");
        assert!(
            elapsed >= Duration::from_millis(120),
            "sync XHR without timeout should wait for the slow response, elapsed={elapsed:?}"
        );
        assert_eq!(
            observed,
            r#"{"events":["readystatechange:1","readystatechange:4","load","loadend"],"readyState":4,"status":200,"responseText":"late"}"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.url().as_str(), xhr_url);
        assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
        let SubresourceNetworkOutcome::Success { status, .. } = record.outcome() else {
            panic!("expected sync XHR network success, got {:?}", record.outcome());
        };
        assert_eq!(*status, 200);
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_aborts_when_page_context_is_cancelled() {
    run_page_vm_async_test(async move {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind cancelled sync XHR server");
        let addr = listener.local_addr().expect("cancelled sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("cancelled-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener.accept().expect("accept cancelled sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read cancelled sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlate",
                );
            })
            .expect("spawn cancelled sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let cancel_tx = page_vm.vm().page_context_cancel_sender();
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/cancelled-sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let canceller = std::thread::Builder::new()
            .name("sync-xhr-page-canceller".to_owned())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(40));
                cancel_tx.cancel(crate::runtime::RendererPageContextCancelReason::PageClosed);
            })
            .expect("spawn sync XHR page canceller");

        let started = Instant::now();
        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const xhr = new XMLHttpRequest();
                        const events = [];
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        xhr.onabort = () => events.push("abort");
                        xhr.onerror = () => events.push("error");
                        xhr.onload = () => events.push("load");
                        xhr.onloadend = () => events.push("loadend");
                        xhr.open("GET", {xhr_url_literal}, false);
                        xhr.send();
                        return JSON.stringify({{
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                            responseText: xhr.responseText,
                        }});
                    }})()
                    "#
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("cancelled sync XHR test should run on owner lane");
        let elapsed = started.elapsed();

        canceller.join().expect("sync XHR page canceller should finish");
        server
            .join()
            .expect("cancelled sync XHR server should finish");
        assert!(
            elapsed < Duration::from_millis(150),
            "sync XHR should abort when the page context is cancelled, elapsed={elapsed:?}"
        );
        assert_eq!(
            observed,
            r#"{"events":["readystatechange:1","abort","loadend"],"readyState":0,"status":0,"responseText":""}"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.url().as_str(), xhr_url);
        let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
            panic!(
                "expected sync XHR page-cancel failure, got {:?}",
                record.outcome()
            );
        };
        assert!(
            error_text.contains("Synchronous XMLHttpRequest aborted because page was closed"),
            "error_text={error_text}"
        );
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_cancel_is_replayed_for_abort_handler_xhr() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind chained cancelled sync XHR server");
        let addr = listener.local_addr().expect("chained sync XHR server addr");
        let base_url = format!("http://{addr}");
        let server = std::thread::Builder::new()
            .name("chained-cancelled-sync-xhr-test-server".to_owned())
            .spawn(move || {
                use std::io::{Read, Write};

                let (mut stream, _) = listener
                    .accept()
                    .expect("accept chained cancelled sync XHR request");
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                loop {
                    stream
                        .read_exact(&mut byte)
                        .expect("read chained cancelled sync XHR request");
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlate",
                );
            })
            .expect("spawn chained cancelled sync XHR test server");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let cancel_tx = page_vm.vm().page_context_cancel_sender();
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/chained-cancelled-sync-xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let canceller = std::thread::Builder::new()
            .name("chained-sync-xhr-page-canceller".to_owned())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(40));
                cancel_tx.cancel(crate::runtime::RendererPageContextCancelReason::PageClosed);
            })
            .expect("spawn chained sync XHR page canceller");

        let started = Instant::now();
        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const events = [];
                        const first = new XMLHttpRequest();
                        first.onabort = () => {{
                            events.push("first-abort");
                            const second = new XMLHttpRequest();
                            second.onabort = () => events.push("second-abort");
                            second.ontimeout = () => events.push("second-timeout");
                            second.onerror = () => events.push("second-error");
                            second.onload = () => events.push("second-load");
                            second.onloadend = () => events.push("second-loadend");
                            second.open("GET", {xhr_url_literal}, false);
                            second.send();
                            events.push("second-after-send:" + second.readyState);
                        }};
                        first.open("GET", {xhr_url_literal}, false);
                        first.send();
                        return JSON.stringify(events);
                    }})()
                    "#
                ))?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("chained cancelled sync XHR test should run on owner lane");
        let elapsed = started.elapsed();

        canceller
            .join()
            .expect("chained sync XHR page canceller should finish");
        server
            .join()
            .expect("chained cancelled sync XHR server should finish");
        assert!(
            elapsed < Duration::from_millis(150),
            "chained sync XHR should replay page cancellation, elapsed={elapsed:?}"
        );
        assert_eq!(
            observed,
            r#"["first-abort","second-abort","second-loadend","second-after-send:0"]"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 2);
        for record in records {
            let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
                panic!(
                    "expected sync XHR page-cancel failure, got {:?}",
                    record.outcome()
                );
            };
            assert!(
                error_text.contains("Synchronous XMLHttpRequest aborted because page was closed"),
                "error_text={error_text}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_repeated_url_succeeds_across_objects_and_script_turns() {
    run_page_vm_async_test(async move {
        for status in [200, 400] {
            let (base_url, shutdown, server) = spawn_repeated_synchronous_xhr_server(status);
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url = serde_json::to_string(&format!("{base_url}/repeated"))
                .expect("serialize repeated XHR URL");

            local_executor
                .run(async move {
                    for reuse in [false, true] {
                        for count in [64, 16] {
                            let observed = page_vm
                                .vm_mut()
                                .eval(&format!(
                                    r#"
                                    (() => {{
                                        globalThis.repeatedXhr ??= new XMLHttpRequest();
                                        const results = [];
                                        for (let i = 0; i < {count}; i++) {{
                                            const xhr = {reuse} ? repeatedXhr : new XMLHttpRequest();
                                            xhr.open("GET", {xhr_url}, false);
                                            xhr.send();
                                            results.push([xhr.status, xhr.responseText, xhr.readyState]);
                                        }}
                                        return JSON.stringify(results);
                                    }})()
                                    "#
                                ))
                                .expect("finite repeated synchronous XHR should complete");
                            let observed: Vec<(u16, String, u8)> =
                                serde_json::from_str(&observed).expect("parse repeated XHR results");
                            assert_eq!(observed, vec![(status, "reply".to_owned(), 4); count]);
                        }
                    }
                })
                .await;

            drop(shutdown);
            assert_eq!(server.join().expect("join repeated XHR server"), 160);
        }
    })
    .await;
}

#[tokio::test]
async fn synchronous_xhr_runaway_loop_is_terminated_by_watchdog_and_recovers() {
    run_page_vm_async_test(async move {
        let (base_url, shutdown, server) = spawn_repeated_synchronous_xhr_server(200);
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = serde_json::to_string(&format!("{base_url}/repeated"))
            .expect("serialize repeated XHR URL");

        local_executor
            .run(async move {
                let _watchdog_timeout =
                    crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
                        crate::v8_execution_watchdog::V8ExecutionWatchdogKind::ScriptTurn,
                        Duration::from_millis(500),
                    );
                let started = Instant::now();
                let error = page_vm
                    .vm_mut()
                    .exec(
                        &format!(
                            r#"
                            for (;;) {{
                                const xhr = new XMLHttpRequest();
                                xhr.open("GET", {xhr_url}, false);
                                xhr.send();
                            }}
                            "#
                        ),
                        None,
                    )
                    .expect_err("watchdog should terminate a runaway synchronous XHR loop");
                assert!(started.elapsed() < Duration::from_secs(4));
                assert!(
                    error.to_string().contains("script execution exceeded"),
                    "unexpected watchdog error: {error}"
                );
                let recovered = page_vm
                    .vm_mut()
                    .eval(&format!(
                        r#"
                        (() => {{
                            const xhr = new XMLHttpRequest();
                            xhr.open("GET", {xhr_url}, false);
                            xhr.send();
                            return `${{xhr.status}}:${{xhr.responseText}}`;
                        }})()
                        "#
                    ))
                    .expect("synchronous XHR should work after watchdog termination");
                assert_eq!(recovered, "200:reply");
            })
            .await;

        drop(shutdown);
        assert!(server.join().expect("join repeated XHR server") > 1);
    })
    .await;
}

#[tokio::test]
async fn xhr_abort_cancels_inflight_network_request_and_suppresses_late_failure() {
    run_page_vm_async_test(async move {
        let (base_url, request_seen_rx, disconnect_rx, server) =
            spawn_request_seen_disconnect_observing_http_server().await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let xhr_url = format!("{base_url}/xhr");
        let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

        let observed = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onabort = () => globalThis.__xhrEvents.push("abort");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                setTimeout(() => {{
                                    globalThis.__xhrObserved = JSON.stringify({{
                                        events: globalThis.__xhrEvents,
                                        readyState: xhr.readyState,
                                        status: xhr.status,
                                        responseText: xhr.responseText,
                                    }});
                                    globalThis.__xhrDone = true;
                                }}, 60);
                            }};
                            globalThis.__abortXhr = () => xhr.abort();
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                ))?;
                let mut request_seen_rx = request_seen_rx;
                let request_seen_deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    match request_seen_rx.try_recv() {
                        Ok(()) => break,
                        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                            panic!("xhr abort server closed before observing the request");
                        }
                    }
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test()
                        .await?
                        .is_some()
                    {}
                    let loader = page_vm.main_document_resource_loader();
                    page_vm
                        .advance_timers_until_deadline_for_test(loader.request_client())
                        .await?;
                    if Instant::now() >= request_seen_deadline {
                        panic!("timed out waiting for xhr abort server to observe the request");
                    }
                    let _ = tokio::time::timeout(
                        Duration::from_millis(10),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await;
                }
                page_vm.vm_mut().eval("globalThis.__abortXhr()")?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__xhrDone === true)",
                    "xhr abort should complete exactly one abort/loadend sequence",
                )
                .await?;
                page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")
            })
            .await
            .expect("xhr abort test should run on owner lane");

        let disconnected = tokio::time::timeout(Duration::from_secs(3), disconnect_rx)
            .await
            .expect("disconnect observation should complete")
            .expect("disconnect observation channel should stay open");
        server
            .await
            .expect("disconnect-observing http server should finish");

        assert!(
            disconnected,
            "expected xhr abort to close the underlying transport early"
        );
        assert_eq!(
            observed,
            r#"{"events":["abort","loadend"],"readyState":0,"status":0,"responseText":""}"#
        );
    })
    .await;
}

#[tokio::test]
async fn xhr_timeout_cancels_inflight_network_request_and_dispatches_timeout() {
    run_page_vm_async_test(async move {
            let (base_url, request_seen_rx, disconnect_rx, server) =
                spawn_request_seen_disconnect_observing_http_server().await;
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url = format!("{base_url}/xhr-timeout");
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let observed = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.ontimeout = () => globalThis.__xhrEvents.push("timeout");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.timeout = 1000;
                            xhr.send();
                            globalThis.__setXhrTimeout = () => {{
                                xhr.timeout = 20;
                            }};
                        }})()
                        "#
                    ))?;
                    let mut request_seen_rx = request_seen_rx;
                    let request_seen_deadline = Instant::now() + Duration::from_secs(3);
                    loop {
                        match request_seen_rx.try_recv() {
                            Ok(()) => break,
                            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                                panic!("xhr timeout server closed before observing the request");
                            }
                        }
                        while page_vm
                            .run_exact_page_websocket_selected_task_for_test().await?
                            .is_some()
                        {}
                        let loader = page_vm.main_document_resource_loader();
                        page_vm.advance_timers_until_deadline_for_test(loader.request_client()).await?;
                        if Instant::now() >= request_seen_deadline {
                            panic!("timed out waiting for xhr timeout server to observe the request");
                        }
                        let _ = tokio::time::timeout(
                            Duration::from_millis(10),
                            page_vm.wait_for_page_work_arrival_without_timeout(false),
                        )
                        .await;
                    }
                    page_vm.vm_mut().eval("globalThis.__setXhrTimeout()")?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr timeout should complete exactly one timeout/loadend sequence",
                    )
                    .await?;
                    page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")
                })
                .await
                .expect("xhr timeout test should run on owner lane");

            let disconnected = tokio::time::timeout(Duration::from_secs(3), disconnect_rx)
                .await
                .expect("disconnect observation should complete")
                .expect("disconnect observation channel should stay open");
            server
                .await
                .expect("delayed disconnect-observing http server should finish");

            assert!(
                disconnected,
                "expected xhr timeout to close the underlying transport early"
            );
            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","readystatechange:4","timeout","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
        })
        .await;
}

#[tokio::test]
async fn xhr_connection_refused_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let (base_url, server) =
                spawn_connection_drop_http_server("/xhr-connection-refused").await;
            let xhr_url = format!("{base_url}/xhr-connection-refused");
            let document_url = Url::parse("http://127.0.0.1/page.html").expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr connection failure should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("xhr connection failure test should run on owner lane");

            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
            ));
            server
                .await
                .expect("connection-drop xhr server should finish");
        })
        .await;
}

#[tokio::test]
async fn synchronous_window_xhr_file_url_throws_network_error_without_progress_events() {
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
                let observed = page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const events = [];
                        const xhr = new XMLHttpRequest();
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        xhr.onloadstart = () => events.push("loadstart");
                        xhr.onerror = () => events.push("error");
                        xhr.onloadend = () => events.push("loadend");
                        xhr.open("GET", "file:///moli-policy-must-not-open", false);
                        let error = null;
                        try {
                            xhr.send();
                        } catch (caught) {
                            error = {
                                name: caught && caught.name,
                                message: caught && caught.message,
                                isDomException: caught instanceof DOMException,
                            };
                        }
                        return JSON.stringify({
                            error,
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                        });
                    })()
                    "#,
                )?;
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
            .expect("synchronous file URL XHR test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'file:///moli-policy-must-not-open'.","isDomException":true},"events":["readystatechange:1"],"readyState":4,"status":0}"#
        );
        assert_eq!(pending_count, 0);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
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
async fn synchronous_window_xhr_bad_port_throws_network_error_without_progress_events() {
    run_page_vm_async_test(async move {
        let document_url = Url::parse("https://example.test/page.html").unwrap();
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const events = [];
                        const xhr = new XMLHttpRequest();
                        xhr.onreadystatechange = () => events.push("readystatechange:" + xhr.readyState);
                        for (const type of ["loadstart", "error", "timeout", "loadend"]) {
                            xhr.addEventListener(type, () => events.push(type));
                            xhr.upload.addEventListener(type, () => events.push("upload." + type));
                        }
                        xhr.open("POST", "http://example.test:1/", false);
                        let error = null;
                        try {
                            xhr.send("body");
                        } catch (caught) {
                            error = {
                                name: caught && caught.name,
                                message: caught && caught.message,
                                isDomException: caught instanceof DOMException,
                            };
                        }
                        return JSON.stringify({
                            error,
                            events,
                            readyState: xhr.readyState,
                            status: xhr.status,
                            statusText: xhr.statusText,
                            responseText: xhr.responseText,
                            responseURL: xhr.responseURL,
                        });
                    })()
                    "#,
                )?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous bad-port XHR test should run on owner lane");

        assert_eq!(
            observed,
            r#"{"error":{"name":"NetworkError","message":"Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'http://example.test:1/'.","isDomException":true},"events":["readystatechange:1"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":""}"#
        );
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Xhr);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text == "xhr: blocked bad port for `http://example.test:1/`"
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_connection_reset_throws_without_progress_events() {
    run_page_vm_async_test(async move {
        let (base_url, server) =
            spawn_blocking_connection_drop_http_server("/sync-xhr-connection-reset");
        let xhr_url = format!("{base_url}/sync-xhr-connection-reset");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous connection-reset XHR probe should run on owner lane");

        server
            .join()
            .expect("synchronous connection-reset XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].url().as_str(), xhr_url);
        assert_eq!(records[0].resource_type(), SubresourceResourceType::Xhr);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_malformed_data_url_throws_without_progress_events() {
    run_page_vm_async_test(async move {
        // Ported from WPT xhr/send-network-error-sync-events.sub.htm and
        // calibrated against Debian Chromium 145.0.7632.116.
        let xhr_url = "data:text/html;charset=utf-8;base64,PT0NUWVBFIGh0bWw%2BDQo8";
        let document_url = Url::parse("https://example.test/page.html").expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous malformed-data XHR probe should run on owner lane");

        assert_synchronous_xhr_network_error_surface(&observed, xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].url().as_str(), xhr_url);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text } if !error_text.is_empty()
        ));
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_rejects_unsupported_redirect_schemes() {
    run_page_vm_async_test(async move {
        // Ported from WPT xhr/send-redirect-bogus-sync.sub.htm. Network-host
        // cases are represented separately by the bad-port/reset tests.
        for (path, location) in [
            ("/sync-xhr-redirect-foobar", "foobar://abcd"),
            ("/sync-xhr-redirect-mailto", "mailto:someone@example.org"),
            ("/sync-xhr-redirect-tel", "tel:1234567890"),
        ] {
            let (base_url, server) = spawn_blocking_single_redirect_http_server(path, location);
            let xhr_url = format!("{base_url}{path}");
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

            let (observed, network_output) = local_executor
                .run(async move {
                    let observed = page_vm.vm_mut().eval(&expression)?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("synchronous redirect-scheme XHR probe should run on owner lane");

            server
                .join()
                .expect("synchronous redirect-scheme XHR server should finish");
            assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].url().as_str(), xhr_url);
            assert!(matches!(
                records[0].outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("not supported")
                        || error_text.contains("HTTP(S)")
                        || error_text.contains("redirect")
            ));
        }
    })
    .await;
}

#[tokio::test]
async fn synchronous_window_xhr_redirect_loop_throws_without_progress_events() {
    run_page_vm_async_test(async move {
        let (base_url, server) =
            spawn_blocking_redirect_loop_http_server("/sync-xhr-redirect-loop");
        let xhr_url = format!("{base_url}/sync-xhr-redirect-loop");
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        let expression = synchronous_xhr_failure_probe_expression(&xhr_url);

        let (observed, network_output) = local_executor
            .run(async move {
                let observed = page_vm.vm_mut().eval(&expression)?;
                Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
            })
            .await
            .expect("synchronous redirect-loop XHR probe should run on owner lane");

        server
            .join()
            .expect("synchronous redirect-loop XHR server should finish");
        assert_synchronous_xhr_network_error_surface(&observed, &xhr_url);
        let (records, _, _) = split_network_output_items(network_output);
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].outcome(),
            SubresourceNetworkOutcome::Failure { error_text }
                if error_text.contains("redirect limit exceeded")
        ));
    })
    .await;
}

#[tokio::test]
async fn xhr_dns_failure_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let xhr_url = "http://moli-dns-failure.invalid./xhr-dns-failure";
            let mut page_vm = test_page_vm_with_config(dns_failure_fetch_config(), Vec::new());
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr DNS failure should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("xhr DNS failure test should run on owner lane");

            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            let SubresourceNetworkOutcome::Failure { error_text } = record.outcome() else {
                panic!("expected DNS XHR failure, got {:?}", record.outcome());
            };
            assert!(
                error_text.to_ascii_lowercase().contains("resolv"),
                "expected DNS-resolution error text, got {error_text:?}"
            );
        })
        .await;
}

#[tokio::test]
async fn xhr_redirect_loop_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let (base_url, server) = spawn_redirect_loop_http_server("/xhr-loop").await;
            let xhr_url = format!("{base_url}/xhr-loop");
            let document_url = Url::parse("http://127.0.0.1/page.html").expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "xhr redirect loop should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("xhr redirect-loop test should run on owner lane");

            server.await.expect("redirect-loop xhr server should finish");
            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("redirect limit exceeded")
            ));
        })
        .await;
}

#[tokio::test]
async fn xhr_cross_origin_redirect_without_cors_reports_network_error_surface() {
    run_page_vm_async_test(async move {
            let (source_base_url, _, source_server, target_server) =
                spawn_cross_origin_redirect_without_cors_http_servers(
                    "/xhr-cors-redirect-deny",
                    "/xhr-cors-denied-target",
                )
                .await;
            let xhr_url = format!("{source_base_url}/xhr-cors-redirect-deny");
            let document_url =
                Url::parse(&format!("{source_base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = JSON.stringify({{
                                    events: globalThis.__xhrEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                    contentType: xhr.getResponseHeader("Content-Type"),
                                    allHeaders: xhr.getAllResponseHeaders(),
                                }});
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "XHR CORS redirect deny should deliver error/loadend",
                    )
                    .await?;
                    while page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .is_some()
                    {}
                    let observed = page_vm.vm_mut().eval("String(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("XHR CORS redirect deny test should run on owner lane");

            source_server
                .await
                .expect("XHR CORS redirect source server should finish");
            target_server
                .await
                .expect("XHR CORS redirect target server should finish");
            assert_eq!(
                observed,
                r#"{"events":["readystatechange:1","loadstart","readystatechange:4","error","loadend"],"readyState":4,"status":0,"statusText":"","responseText":"","responseURL":"","contentType":null,"allHeaders":""}"#
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text == crate::network_host::FAILED_ERROR_TEXT
            ));
        })
        .await;
}

#[tokio::test]
async fn xhr_document_csp_blocks_cross_origin_redirect_final_url() {
    run_page_vm_async_test(async move {
            let (source_base_url, target_base_url, source_server, target_server) =
                spawn_cross_origin_redirect_with_cors_http_servers(
                    "/xhr-csp-redirect-source",
                    "/xhr-csp-redirect-target",
                    "cors-allowed-xhr-target",
                )
                .await;
            let xhr_url = format!("{source_base_url}/xhr-csp-redirect-source");
            let target_url = format!("{target_base_url}/xhr-csp-redirect-target");
            let document_url =
                Url::parse(&format!("{source_base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            page_vm
                .vm_mut()
                .set_response_content_security_policies(&[String::from("connect-src 'self'")]);
            let local_executor = page_vm.local_executor.clone();
            let xhr_url_literal = serde_json::to_string(&xhr_url).expect("serialize xhr url");

            let (observed, network_output) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(&format!(
                        r#"
                        (() => {{
                            globalThis.__xhrCspEvents = [];
                            globalThis.__xhrEvents = [];
                            globalThis.__xhrDone = false;
                            globalThis.__xhrObserved = null;
                            self.addEventListener("securitypolicyviolation", event => {{
                                globalThis.__xhrCspEvents.push({{
                                    blockedURI: event.blockedURI,
                                    effectiveDirective: event.effectiveDirective,
                                    disposition: event.disposition,
                                    instance: event instanceof SecurityPolicyViolationEvent,
                                }});
                            }});
                            const xhr = new XMLHttpRequest();
                            xhr.onreadystatechange = () => globalThis.__xhrEvents.push("readystatechange:" + xhr.readyState);
                            xhr.onloadstart = () => globalThis.__xhrEvents.push("loadstart");
                            xhr.onerror = () => globalThis.__xhrEvents.push("error");
                            xhr.onload = () => globalThis.__xhrEvents.push("load");
                            xhr.onloadend = () => {{
                                globalThis.__xhrEvents.push("loadend");
                                globalThis.__xhrObserved = {{
                                    events: globalThis.__xhrEvents,
                                    cspEvents: globalThis.__xhrCspEvents,
                                    readyState: xhr.readyState,
                                    status: xhr.status,
                                    statusText: xhr.statusText,
                                    responseText: xhr.responseText,
                                    responseURL: xhr.responseURL,
                                }};
                                globalThis.__xhrDone = true;
                            }};
                            xhr.open("GET", {xhr_url_literal});
                            xhr.send();
                        }})()
                        "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__xhrDone === true)",
                        "XHR CSP redirect final URL should deliver error/loadend",
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
                        .eval("JSON.stringify(globalThis.__xhrObserved)")?;
                    Ok::<_, anyhow::Error>((observed, page_vm.vm_mut().take_network_output()))
                })
                .await
                .expect("XHR CSP redirect test should run on owner lane");

            source_server
                .await
                .expect("XHR CSP redirect source server should finish");
            target_server
                .await
                .expect("XHR CSP redirect target server should finish");
            let observed: serde_json::Value =
                serde_json::from_str(&observed).expect("parse XHR CSP redirect observation");
            assert_eq!(
                observed,
                json!({
                    "events": [
                        "readystatechange:1",
                        "loadstart",
                        "readystatechange:4",
                        "error",
                        "loadend",
                    ],
                    "cspEvents": [{
                        "blockedURI": target_url,
                        "effectiveDirective": "connect-src",
                        "disposition": "enforce",
                        "instance": true,
                    }],
                    "readyState": 4,
                    "status": 0,
                    "statusText": "",
                    "responseText": "",
                    "responseURL": "",
                })
            );
            let (records, _, _) = split_network_output_items(network_output);
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.url().as_str(), xhr_url);
            assert_eq!(record.resource_type(), SubresourceResourceType::Xhr);
            assert!(matches!(
                record.outcome(),
                SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("Content Security Policy")
            ));
        })
        .await;
}
