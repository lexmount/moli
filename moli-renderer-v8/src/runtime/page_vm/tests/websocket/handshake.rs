use super::*;

#[tokio::test]
async fn websocket_inherited_csp_relative_report_uri_popup_posts_to_delivery_path() {
    check_websocket_inherited_csp_relative_report_uri(false).await;
}

#[tokio::test]
async fn websocket_inherited_csp_relative_report_uri_sandbox_srcdoc_posts_to_delivery_path() {
    check_websocket_inherited_csp_relative_report_uri(true).await;
}

async fn check_websocket_inherited_csp_relative_report_uri(sandbox_srcdoc: bool) {
    run_page_vm_async_test(async move {
        for constructor in ["WebSocket", "WebSocketStream"] {
            let (base_url, report_rx, server) = spawn_request_capture_http_server().await;
            let document_url = Url::parse(&format!("{base_url}/app/page.html")).unwrap();
            let mut page_vm = test_page_vm_with_document_url(document_url.clone());
            page_vm.vm_mut().set_main_navigation_policy_container(
                crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                    &[("Content-Security-Policy".to_owned(),
                        b"connect-src 'none'; report-uri csp-report".to_vec())],
                    &document_url,
                ),
            );
            page_vm.vm_mut().set_fetch_subresource_interception(
                true, Some(SubresourceResourceType::WebSocket),
            );
            let script = format!(
                r#"
                const reportWindow = top.opener || top;
                addEventListener('securitypolicyviolation', event => {{
                    reportWindow.postMessage({{
                        document: document.URL,
                        directive: event.effectiveDirective,
                        documentURI: event.documentURI,
                    }}, '*');
                }});
                const base = document.createElement('base');
                base.href = 'https://different.test/changed/';
                document.head.appendChild(base);
                new {constructor}('wss://blocked-report.test/socket');
                void 0;
                "#,
            );
            let local_executor = page_vm.local_executor.clone();
            let request = local_executor.run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    globalThis.__inheritedCspReport = null;
                    addEventListener('message', event => {
                        __inheritedCspReport = event.data;
                    });
                    "#,
                )?;
                if sandbox_srcdoc {
                    let markup = serde_json::to_string(&format!("<script>{script}</script>"))?;
                    page_vm.vm_mut().eval(&format!(
                        "const frame = document.createElement('iframe');\
                         frame.sandbox = 'allow-scripts'; frame.srcdoc = {markup};\
                         document.body.appendChild(frame);"
                    ))?;
                } else {
                    let source = serde_json::to_string(&script)?;
                    page_vm.vm_mut().eval(&format!(
                        "globalThis.__reportPopup = open('about:blank');\
                         __reportPopup.location.href = 'javascript:' + encodeURIComponent({source});"
                    ))?;
                }
                drive_websocket_until_done(&mut page_vm,
                    "__inheritedCspReport !== null", "inherited WebSocket CSP violation"
                ).await?;
                let observed: serde_json::Value = serde_json::from_str(
                    &page_vm.vm_mut().eval("JSON.stringify(__inheritedCspReport)")?
                )?;
                assert_eq!(observed, json!({
                    "document": if sandbox_srcdoc { "about:srcdoc" } else { "about:blank" },
                    "directive": "connect-src",
                    "documentURI": "about",
                }));
                assert!(page_vm.vm_mut().take_pending_subresource_fetch_infos().is_empty());
                tokio::time::timeout(Duration::from_secs(5), report_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!(
                        "missing CSP report: {constructor}, sandbox_srcdoc={sandbox_srcdoc}"
                    ))?
                    .map_err(|_| anyhow::anyhow!("CSP report capture channel closed"))
            }).await.expect("inherited CSP report test should run on owner lane");
            server.await.expect("inherited CSP report server should finish");
            assert!(request.starts_with("POST /app/csp-report HTTP/1.1\r\n"),
                "report must use the full delivery URL: {constructor}, \
                 sandbox_srcdoc={sandbox_srcdoc}, request={request}");
            assert!(request.to_ascii_lowercase().contains("content-type: application/csp-report"));
            let (_, body) = request.split_once("\r\n\r\n").expect("report request body");
            let body: serde_json::Value = serde_json::from_str(body).expect("CSP report JSON");
            assert_eq!(body["csp-report"]["document-uri"], "about");
            assert_eq!(body["csp-report"]["effective-directive"], "connect-src");
            assert_eq!(body["csp-report"]["disposition"], "enforce");
        }
    }).await;
}

#[tokio::test]
async fn websocket_sandbox_srcdoc_inherits_csp_self_origin_with_opaque_handshake_origin() {
    run_page_vm_async_test(async move {
        for constructor in ["WebSocket", "WebSocketStream"] {
            for change_base in [false, true] {
                for in_popup in [false, true] {
                    let (url, headers_rx, server) = spawn_tls_header_capture_websocket_server().await;
                    let mut document_url = moli_websocket::websocket_cookie_url(&Url::parse(&url).unwrap());
                    document_url.set_path("/app/page.html");
                    let url_literal = serde_json::to_string(&url).unwrap();
                    let child_script = format!(
                        r#"
                        const reportWindow = top.opener || top;
                        addEventListener('securitypolicyviolation', event => {{
                            reportWindow.postMessage({{kind: 'violation', directive: event.effectiveDirective}}, '*');
                        }});
                        if ({change_base}) {{
                            const base = document.createElement('base');
                            base.href = 'https://different-origin.test/';
                            document.head.appendChild(base);
                        }}
                        try {{
                            const socket = new {constructor}({url_literal});
                            const opened = () => reportWindow.postMessage({{
                                kind: 'result', value: document.URL + '|opened'
                            }}, '*');
                            if (socket.opened) {{
                                socket.opened.then(opened, error => reportWindow.postMessage({{
                                    kind: 'result', value: 'rejected:' + error.name
                                }}, '*'));
                            }} else {{
                                socket.onopen = opened;
                                socket.onerror = () => reportWindow.postMessage({{
                                    kind: 'result', value: 'error'
                                }}, '*');
                            }}
                        }} catch (error) {{
                            reportWindow.postMessage({{kind: 'result', value: error.name}}, '*');
                        }}
                        "#,
                    );
                    let markup = serde_json::to_string(&format!("<script>{child_script}</script>"))
                        .unwrap();
                    let mut config = FetchConfig::default();
                    config.set_tls_verify_host(false);
                    let loader = crate::network::ResourceRequestClient::new(&config).unwrap();
                    let policy_container = crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                        &[("Content-Security-Policy".to_owned(), b"connect-src 'self'".to_vec())],
                        &document_url,
                    );
                    let mut page_vm = test_page_vm_with_loader_and_document_url(
                        &loader, Vec::new(), document_url.clone(),
                    );
                    page_vm.vm_mut().set_main_navigation_policy_container(policy_container);
                    page_vm.retain_standalone_request_client_owner_for_test(loader);
                    let local_executor = page_vm.local_executor.clone();
                    let headers = local_executor.run(async move {
                        page_vm.vm_mut().eval(
                            r#"
                            globalThis.__sandboxWebSocketResult = 'pending';
                            globalThis.__sandboxWebSocketViolations = [];
                            addEventListener('message', event => {
                                if (event.data.kind === 'result') {
                                    __sandboxWebSocketResult = event.data.value;
                                } else if (event.data.kind === 'violation') {
                                    __sandboxWebSocketViolations.push(event.data.directive);
                                }
                            });
                            "#,
                        )?;
                        let create_frame = format!(
                            "const frame = document.createElement('iframe');\
                             frame.sandbox = 'allow-scripts'; frame.srcdoc = {markup};\
                             document.body.appendChild(frame); void 0;"
                        );
                        if in_popup {
                            page_vm.vm_mut().eval("globalThis.__sandboxPopup = open('about:blank')")?;
                            // Make the host policy differ from the popup's captured
                            // policy so a fallback through the opener is observable.
                            page_vm.vm_mut().set_main_navigation_policy_container(
                                crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                                    &[("Content-Security-Policy".to_owned(), b"connect-src 'none'".to_vec())],
                                    &document_url,
                                ),
                            );
                            let source = serde_json::to_string(&create_frame)?;
                            page_vm.vm_mut().eval(&format!(
                                "__sandboxPopup.location.href = 'javascript:' + encodeURIComponent({source})"
                            ))?;
                        } else {
                            page_vm.vm_mut().eval(&create_frame)?;
                        }
                        drive_websocket_until_done(
                            &mut page_vm,
                            "__sandboxWebSocketResult !== 'pending'",
                            "sandbox srcdoc WebSocket should report from its own script",
                        ).await?;
                        assert_eq!(
                            page_vm.vm_mut().eval("__sandboxWebSocketResult")?,
                            "about:srcdoc|opened",
                            "{constructor}, change_base={change_base}, in_popup={in_popup}",
                        );
                        assert_eq!(page_vm.vm_mut().eval("__sandboxWebSocketViolations.length")?, "0");
                        tokio::time::timeout(Duration::from_secs(3), headers_rx)
                            .await
                            .map_err(|_| anyhow::anyhow!("sandbox WebSocket handshake did not arrive"))?
                            .map_err(|_| anyhow::anyhow!("sandbox WebSocket header sender dropped"))
                    }).await.expect("sandbox WebSocket handshake test should run on owner lane");
                    server.await.expect("sandbox WebSocket server should finish");
                    assert_eq!(header_value(&headers, "origin").as_deref(), Some("null"));
                }
            }
        }
    }).await;
}

#[tokio::test]
async fn websocket_sandbox_srcdoc_inherited_self_still_blocks_foreign_base_origin() {
    run_page_vm_async_test(async move {
        for constructor in ["WebSocket", "WebSocketStream"] {
            let document_url = Url::parse("https://sandbox-csp.test/app/page.html").unwrap();
            let mut page_vm = test_page_vm_with_document_url(document_url.clone());
            page_vm.vm_mut().set_main_navigation_policy_container(
                crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                    &[("Content-Security-Policy".to_owned(), b"connect-src 'self'".to_vec())],
                    &document_url,
                ),
            );
            page_vm.vm_mut().set_fetch_subresource_interception(
                true, Some(SubresourceResourceType::WebSocket),
            );
            let script = format!(
                r#"
                addEventListener('securitypolicyviolation', event => {{
                    parent.postMessage({{kind: 'violation', value: [
                        event.effectiveDirective, event.documentURI, event.disposition
                    ].join('|')}}, '*');
                }});
                const base = document.createElement('base');
                base.href = 'https://different-origin.test/';
                document.head.appendChild(base);
                let result = 'constructed';
                try {{ new {constructor}('wss://different-origin.test/socket'); }}
                catch (error) {{ result = error.name; }}
                parent.postMessage({{kind: 'result', value: result}}, '*');
                "#,
            );
            let markup = serde_json::to_string(&format!("<script>{script}</script>")).unwrap();
            let local_executor = page_vm.local_executor.clone();
            local_executor.run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    globalThis.__sandboxForeignResult = 'pending';
                    globalThis.__sandboxForeignViolation = 'pending';
                    addEventListener('message', event => {{
                        if (event.data.kind === 'result') __sandboxForeignResult = event.data.value;
                        if (event.data.kind === 'violation') __sandboxForeignViolation = event.data.value;
                    }});
                    const frame = document.createElement('iframe');
                    frame.sandbox = 'allow-scripts';
                    frame.srcdoc = {markup};
                    document.body.appendChild(frame);
                    "#,
                ))?;
                drive_websocket_until_done(&mut page_vm,
                    "__sandboxForeignResult !== 'pending'", "sandbox WebSocket construction result"
                ).await?;
                // CSP failures are asynchronous. A blocked socket must never
                // reach the intercepted network request boundary.
                assert_eq!(page_vm.vm_mut().eval("__sandboxForeignResult")?, "constructed");
                assert!(page_vm.vm_mut().take_pending_subresource_fetch_infos().is_empty());
                drive_websocket_until_done(&mut page_vm,
                    "__sandboxForeignViolation !== 'pending'", "sandbox WebSocket CSP violation event"
                ).await?;
                assert_eq!(page_vm.vm_mut().eval("__sandboxForeignViolation")?, "connect-src|about|enforce");
                Ok::<_, anyhow::Error>(())
            }).await.expect("foreign sandbox WebSocket CSP test should run on owner lane");
        }
    }).await;
}

#[tokio::test]
async fn websocket_popup_base_changes_preserve_inherited_handshake_origin() {
    run_page_vm_async_test(async move {
        for constructor in ["WebSocket", "WebSocketStream"] {
            for relative in [false, true] {
                for same_origin in [false, true] {
                    let (url, headers_rx, server) = spawn_tls_header_capture_websocket_server().await;
                    let cookie_url = moli_websocket::websocket_cookie_url(&Url::parse(&url).unwrap());
                    let document_url = if same_origin {
                        cookie_url.clone()
                    } else {
                        Url::parse("https://example.com/").unwrap()
                    };
                    let expected_origin = moli_url::origin_ascii_serialization(&document_url);
                    let cookie_store = moli_cookie_jar::new_shared_browser_cookie_store();
                    let reports = cookie_store.lock().unwrap().store_response_headers_with_reports(
                        &cookie_url,
                        &[
                            ("set-cookie".to_owned(), b"strict=1; Path=/; SameSite=Strict; Secure".to_vec()),
                            ("set-cookie".to_owned(), b"cross=1; Path=/; SameSite=None; Secure".to_vec()),
                        ],
                    );
                    assert!(reports.iter().all(|report| report.is_accepted()));
                    let base = if relative {
                        cookie_url.to_string()
                    } else {
                        "https://different-origin.test/socket-base/".to_owned()
                    };
                    let input = if relative { "headers" } else { &url };
                    let base_literal = serde_json::to_string(&base).unwrap();
                    let input_literal = serde_json::to_string(input).unwrap();
                    let script = serde_json::to_string(&format!(
                        r#"javascript:
                        const base = document.createElement('base');
                        base.href = {base_literal};
                        document.head.appendChild(base);
                        if ({same_origin}) {{
                            const meta = document.createElement('meta');
                            meta.httpEquiv = 'Content-Security-Policy';
                            meta.content = "connect-src 'self'";
                            document.head.appendChild(meta);
                        }}
                        const socket = new {constructor}({input_literal});
                        opener.__popupWebSocketResult = [document.URL, socket.url].join('|');
                        void 0;"#
                    )).unwrap();
                    let mut config = FetchConfig::default();
                    config.set_tls_verify_host(false);
                    let loader = crate::network::ResourceRequestClient::new_with_cookie_store(
                        &config, cookie_store,
                    ).unwrap();
                    let mut page_vm = test_page_vm_with_loader_and_document_url(
                        &loader, Vec::new(), document_url,
                    );
                    if same_origin {
                        page_vm
                            .vm_mut()
                            .set_response_content_security_policies(&["connect-src 'self'".to_owned()]);
                    }
                    page_vm.retain_standalone_request_client_owner_for_test(loader);
                    let local_executor = page_vm.local_executor.clone();

                    let headers = local_executor.run(async move {
                        page_vm.vm_mut().eval(&format!(
                            r#"(() => {{
                                const popup = window.open('about:blank');
                                popup.location.href = {script};
                            }})()"#
                        ))?;
                        drive_websocket_until_done(
                            &mut page_vm,
                            "typeof globalThis.__popupWebSocketResult !== 'undefined'",
                            "popup WebSocket script should run",
                        ).await?;
                        let result = page_vm.vm_mut().eval("String(__popupWebSocketResult)")?;
                        assert_eq!(result, format!("about:blank|{url}"));
                        tokio::time::timeout(Duration::from_secs(3), headers_rx)
                            .await
                            .map_err(|_| anyhow::anyhow!("popup WebSocket handshake did not arrive"))?
                            .map_err(|_| anyhow::anyhow!("popup WebSocket header sender dropped"))
                    }).await.expect("popup WebSocket handshake test should run on owner lane");
                    server.await.expect("popup WebSocket server should finish");
                    assert_eq!(
                        header_value(&headers, "origin").as_deref(),
                        Some(expected_origin.as_str()),
                        "{constructor}, relative={relative}, same_origin={same_origin}: inherited Origin"
                    );
                    let cookie = header_value(&headers, "cookie").expect("SameSite=None cookie");
                    let cookies = cookie.split(';').map(str::trim).collect::<Vec<_>>();
                    assert!(cookies.contains(&"cross=1"));
                    assert_eq!(cookies.contains(&"strict=1"), same_origin,
                        "{constructor}, relative={relative}: cookie context must use the native origin");
                }
            }
        }
    }).await;
}

#[tokio::test]
async fn websocket_handshake_inherits_origin_user_agent_and_extra_headers() {
    run_page_vm_async_test(async move {
        let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut config = FetchConfig::default();
        config.set_user_agent("Moli-WebSocket-Test/1.0");
        let mut page_vm = test_page_vm_with_config(
            config,
            vec![
                ("X-Moli-Trace".to_owned(), "socket".to_owned()),
                // Protocol control headers are generated by tungstenite and must not be
                // overridden by page-level extra headers.
                ("Sec-WebSocket-Version".to_owned(), "999".to_owned()),
            ],
        );
        let local_executor = page_vm.local_executor.clone();

        let headers = local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .eval(&format!("new WebSocket({url_literal})"))?;
                tokio::time::timeout(Duration::from_secs(3), headers_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("websocket handshake headers did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("websocket header sender dropped"))
            })
            .await
            .expect("websocket header test should run on owner lane");
        server.await.expect("websocket header server should finish");

        assert_eq!(
            header_value(&headers, "origin").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            header_value(&headers, "user-agent").as_deref(),
            Some("Moli-WebSocket-Test/1.0")
        );
        assert_eq!(
            header_value(&headers, "x-moli-trace").as_deref(),
            Some("socket")
        );
        assert_eq!(
            header_value(&headers, "sec-websocket-version").as_deref(),
            Some("13")
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_handshake_omits_referer_header_like_fetch() {
    run_page_vm_async_test(async move {
        let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm_with_document_url(
            Url::parse("https://example.com/referrer-source").expect("page url"),
        );
        let local_executor = page_vm.local_executor.clone();

        let headers = local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .eval(&format!("new WebSocket({url_literal})"))?;
                tokio::time::timeout(Duration::from_secs(3), headers_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("websocket referrer headers did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("websocket referrer header sender dropped"))
            })
            .await
            .expect("websocket referrer test should run on owner lane");
        server
            .await
            .expect("websocket referrer header server should finish");

        assert_eq!(header_value(&headers, "referer"), None);
    })
    .await;
}

#[tokio::test]
async fn websocket_handshake_inherits_browser_cookie_store() {
    run_page_vm_async_test(async move {
        let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let cookie_store = moli_cookie_jar::new_shared_browser_cookie_store();
        let ws_url = Url::parse(&url).expect("websocket url");
        let cookie_url = moli_websocket::websocket_cookie_url(&ws_url);
        {
            let mut store = cookie_store.lock().unwrap();
            let reports = store.store_response_headers_with_reports(
                &cookie_url,
                &[(
                    "set-cookie".to_owned(),
                    b"sid=server; Path=/; HttpOnly; SameSite=None; Secure".to_vec(),
                )],
            );
            assert!(reports.iter().any(|report| report.is_accepted()));
        }
        let loader = crate::network::ResourceRequestClient::new_with_cookie_store(
            &FetchConfig::default(),
            cookie_store,
        )
        .expect("loader with cookie store");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        let local_executor = page_vm.local_executor.clone();

        let headers = local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .eval(&format!("new WebSocket({url_literal})"))?;
                tokio::time::timeout(Duration::from_secs(3), headers_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("websocket handshake headers did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("websocket header sender dropped"))
            })
            .await
            .expect("websocket cookie test should run on owner lane");
        server.await.expect("websocket header server should finish");

        assert_eq!(
            header_value(&headers, "cookie").as_deref(),
            Some("sid=server")
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_handshake_respects_disabled_tls_verify_for_self_signed_wss() {
    run_page_vm_async_test(async move {
        let (url, headers_rx, server) = spawn_tls_header_capture_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut config = FetchConfig::default();
        config.set_tls_verify_host(false);
        let mut page_vm = test_page_vm_with_config(config, Vec::new());
        let local_executor = page_vm.local_executor.clone();

        let headers = local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .eval(&format!("new WebSocket({url_literal})"))?;
                tokio::time::timeout(Duration::from_secs(3), headers_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("websocket TLS handshake headers did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("websocket TLS header sender dropped"))
            })
            .await
            .expect("websocket TLS verify test should run on owner lane");
        server
            .await
            .expect("websocket TLS header server should finish");

        assert_eq!(
            header_value(&headers, "origin").as_deref(),
            Some("https://example.com")
        );
    })
    .await;
}

#[tokio::test]
async fn websocket_handshake_uses_explicit_http_proxy_connect() {
    run_page_vm_async_test(async move {
        let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
        let (proxy_url, proxy_request_rx, proxy) = spawn_http_connect_proxy().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut config = FetchConfig::default();
        config.set_proxy_options(Some(proxy_url), Some("proxy-token".to_owned()));
        config.set_http_no_proxy(Some(String::new()));
        let mut page_vm = test_page_vm_with_config(config, Vec::new());
        let local_executor = page_vm.local_executor.clone();

        let (proxy_request, headers) = local_executor
            .run(async move {
                page_vm
                    .vm_mut()
                    .eval(&format!("new WebSocket({url_literal})"))?;
                let proxy_request = tokio::time::timeout(Duration::from_secs(3), proxy_request_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("websocket proxy CONNECT did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("websocket proxy sender dropped"))?;
                let headers = tokio::time::timeout(Duration::from_secs(3), headers_rx)
                    .await
                    .map_err(|_| anyhow::anyhow!("websocket handshake headers did not arrive"))?
                    .map_err(|_| anyhow::anyhow!("websocket header sender dropped"))?;
                Ok::<_, anyhow::Error>((proxy_request, headers))
            })
            .await
            .expect("websocket proxy test should run on owner lane");
        server.await.expect("websocket header server should finish");
        proxy.await.expect("websocket proxy should finish");

        let target = Url::parse(&url).expect("websocket target url");
        let expected_connect = format!(
            "CONNECT {}:{} HTTP/1.1",
            target.host_str().expect("target host"),
            target.port_or_known_default().expect("target port")
        );
        assert!(
            proxy_request.starts_with(&expected_connect),
            "unexpected CONNECT request: {proxy_request:?}"
        );
        assert!(
            proxy_request.contains("\r\nProxy-Authorization: Bearer proxy-token\r\n"),
            "proxy bearer token should be sent only on CONNECT: {proxy_request:?}"
        );
        assert_eq!(
            header_value(&headers, "origin").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(header_value(&headers, "proxy-authorization"), None);
    })
    .await;
}

#[tokio::test]
async fn websocket_handshake_failure_records_network_trace_entry() {
    run_page_vm_async_test(async move {
        let (url, server) = spawn_dropping_websocket_server().await;
        let url_literal = serde_json::to_string(&url).expect("serialize websocket url");
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let network_output = local_executor
            .run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const socket = new WebSocket({url_literal});
                        globalThis.__wsFailed = false;
                        globalThis.__wsClosed = false;
                        socket.addEventListener('error', () => {{
                            globalThis.__wsFailed = true;
                        }});
                        socket.addEventListener('close', () => {{
                            globalThis.__wsClosed = true;
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
                    if page_vm.vm_mut().eval(
                        "String(globalThis.__wsFailed === true && globalThis.__wsClosed === true)",
                    )? == "true"
                    {
                        break;
                    }
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(1),
                        page_vm.wait_for_page_work_arrival_without_timeout(false),
                    )
                    .await
                    .unwrap_or(false);
                    assert!(arrived, "websocket failure event should arrive");
                }
                while page_vm
                    .run_exact_page_websocket_selected_task_for_test()
                    .await?
                    .is_some()
                {}
                assert_eq!(
                    page_vm.vm_mut().eval(
                        "String(globalThis.__wsFailed === true && globalThis.__wsClosed === true)"
                    )?,
                    "true"
                );
                Ok::<_, anyhow::Error>(page_vm.vm_mut().take_network_output())
            })
            .await
            .expect("websocket failure trace test should run on owner lane");
        server.await.expect("websocket drop server should finish");
        let (records, _frame_events, lifecycle_events) = split_network_output_items(network_output);

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
        assert_eq!(record.method(), "GET");
        assert_eq!(record.url().as_str(), url);
        match record.outcome() {
            SubresourceNetworkOutcome::Failure { error_text } => {
                assert!(error_text.contains("WebSocket connection failed"));
            }
            outcome => panic!("expected websocket failure record, got {outcome:?}"),
        }
        assert_eq!(lifecycle_events.len(), 2);
        assert_eq!(
            lifecycle_events[0].kind(),
            crate::types::WebSocketLifecycleKind::Error
        );
        assert!(
            lifecycle_events[0]
                .error_text()
                .is_some_and(|text| text.contains("WebSocket connection failed"))
        );
        assert_eq!(
            lifecycle_events[1].kind(),
            crate::types::WebSocketLifecycleKind::Close
        );
        assert_eq!(lifecycle_events[1].close_code(), Some(1006));
        assert_eq!(lifecycle_events[1].was_clean(), Some(false));
    })
    .await;
}
