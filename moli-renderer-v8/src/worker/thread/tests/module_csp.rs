use super::*;

#[tokio::test]
async fn worker_synthetic_module_dynamic_imports_use_connect_csp() {
    ensure_v8();
    for module_type in ["text", "json"] {
        for case in [
            "blocked",
            "default",
            "allowed",
            "report-only",
            "redirect",
            "redirect-report-only",
        ] {
            let (cross_url, cross_server) = spawn_raw_path_response_http_server(vec![(
                "/value",
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 7\r\nConnection: close\r\n\r\n\"hello\"".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let (base_url, server) = spawn_raw_path_response_http_server(vec![
                ("/worker/value", "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 7\r\nConnection: close\r\n\r\n\"hello\"".to_owned(), Duration::ZERO),
                ("/worker/redirect", format!("HTTP/1.1 302 Found\r\nLocation: {cross_url}/value\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"), Duration::ZERO),
            ]).await;
            let policy = match case {
                "blocked" => "connect-src 'none'; script-src *",
                "default" => "default-src 'none'; script-src *",
                "redirect-report-only" => "connect-src *; script-src 'none'",
                _ => "connect-src 'self'; script-src 'none'",
            };
            let path = if case.starts_with("redirect") {
                "redirect"
            } else {
                "value"
            };
            let source = format!(
                r#"
                const events = [];
                addEventListener('securitypolicyviolation', event => {{
                    events.push([event.effectiveDirective, event.disposition]);
                }});
                try {{
                    const module = await import('./{path}', {{with: {{type: '{module_type}'}}}});
                    postMessage({{status: 'loaded', value: module.default, events}});
                }} catch (error) {{ postMessage({{status: error.name, events}}); }}
                close();
            "#
            );
            let options = WorkerSpawnOptions::new(source, format!("{base_url}/worker/main.js"))
                .with_request_client(ResourceRequestClient::new(&FetchConfig::default()).unwrap())
                .with_script_kind(WorkerScriptKind::Module)
                .with_content_security_policies(vec![policy.into()]);
            let options = if case == "redirect-report-only" {
                options
                    .with_content_security_report_only_policies(vec!["connect-src 'self'".into()])
            } else if case == "report-only" {
                options
                    .with_content_security_report_only_policies(vec!["connect-src 'none'".into()])
            } else {
                options
            };
            let mut handle = spawn_test_worker_with_options(options);
            let result = expect_post_json(
                timeout(TIMEOUT, handle.recv())
                    .await
                    .expect("worker CSP result timed out")
                    .expect("worker closed"),
            );
            let result: serde_json::Value = serde_json::from_str(&result).unwrap();
            let blocked = matches!(case, "blocked" | "default" | "redirect");
            assert_eq!(
                result["status"],
                if blocked { "TypeError" } else { "loaded" },
                "{module_type}/{case}: {result}"
            );
            if blocked || case.ends_with("report-only") {
                assert_eq!(
                    result["events"],
                    serde_json::json!([[
                        "connect-src",
                        if blocked { "enforce" } else { "report" }
                    ]]),
                    "{module_type}/{case}"
                );
            } else {
                assert_eq!(
                    result["events"],
                    serde_json::json!([]),
                    "{module_type}/{case}"
                );
            }
            handle.terminate_and_join();
            server.abort();
            cross_server.abort();
        }
    }
}

#[tokio::test]
async fn worker_synthetic_module_static_imports_use_outside_connect_csp() {
    ensure_v8();
    for module_type in ["text", "json"] {
        for blocked in [false, true] {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/worker/value",
                "HTTP/1.1 200 OK",
                "application/json",
                "\"hello\"".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let source = format!(
                r#"
                import value from './value' with {{type: '{module_type}'}};
                postMessage(value);
                close();
            "#
            );
            let mut handle = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(source, format!("{base_url}/worker/main.js"))
                    .with_request_client(
                        ResourceRequestClient::new(&FetchConfig::default()).unwrap(),
                    )
                    .with_script_kind(WorkerScriptKind::Module)
                    .with_module_static_import_initiator_url(
                        url::Url::parse(&format!("{base_url}/page.html")).unwrap(),
                    )
                    .with_module_static_import_content_security_policies(vec![if blocked {
                        "connect-src 'none'; worker-src *".into()
                    } else {
                        "connect-src 'self'; worker-src 'none'; script-src 'none'".into()
                    }])
                    .with_content_security_policies(vec!["connect-src 'none'".into()]),
            );
            let result = timeout(TIMEOUT, handle.recv())
                .await
                .expect("static worker CSP timed out")
                .expect("worker closed");
            if blocked {
                match result {
                    WorkerToParentMessage::Error { message, .. } => {
                        assert!(
                            message.contains("Content Security Policy"),
                            "{module_type}: {message}"
                        );
                        assert!(
                            message.contains("/worker/value"),
                            "{module_type}: {message}"
                        );
                    }
                    other => panic!("{module_type}: expected CSP error, got {other:?}"),
                }
            } else {
                assert_eq!(
                    expect_post_json(result),
                    if module_type == "text" {
                        r#""\"hello\"""#
                    } else {
                        r#""hello""#
                    },
                    "{module_type}"
                );
            }
            handle.terminate_and_join();
            server.abort();
        }
    }
}
