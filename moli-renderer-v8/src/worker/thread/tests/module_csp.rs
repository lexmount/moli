use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::task::JoinSet;

struct RedirectServers {
    origin: String,
    origin_hits: Arc<AtomicUsize>,
    cross_hits: Arc<AtomicUsize>,
    _tasks: JoinSet<()>,
}

impl RedirectServers {
    async fn spawn(module_type: &str, roundtrip: bool) -> Self {
        let a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", a.local_addr().unwrap());
        let cross = format!("http://{}", b.local_addr().unwrap());
        let origin_hits = Arc::new(AtomicUsize::new(0));
        let cross_hits = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();
        for (listener, hits) in [(a, origin_hits.clone()), (b, cross_hits.clone())] {
            let origin = origin.clone();
            let cross = cross.clone();
            let module_type = module_type.to_owned();
            tasks.spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else { break; };
                    let request = read_http_request_head(&mut stream).await.unwrap();
                    hits.fetch_add(1, Ordering::SeqCst);
                    let path = request.split_whitespace().nth(1).unwrap().split('?').next().unwrap();
                    let request_origin = request.lines().filter_map(|line| line.split_once(':'))
                        .find(|(key, _)| key.eq_ignore_ascii_case("origin")).map(|(_, value)| value.trim()).unwrap_or("*");
                    let (mime, body) = match module_type.as_str() {
                        "text" | "json" => ("application/json", b"\"ok\"".as_slice()),
                        "wasm" => ("application/wasm", b"\0asm\x01\0\0\0".as_slice()),
                        _ => ("text/javascript", b"export default 'ok';".as_slice()),
                    };
                    let response = if path == "/start" || (path == "/middle" && roundtrip) {
                        let target = if path == "/start" { format!("{cross}/middle?token=SECRET-redirect") } else { format!("{origin}/final") };
                        format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nAccess-Control-Allow-Origin: {request_origin}\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    } else {
                        format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccess-Control-Allow-Origin: {request_origin}\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                    };
                    stream.write_all(response.as_bytes()).await.unwrap();
                    if path != "/start" && !(path == "/middle" && roundtrip) {
                        stream.write_all(body).await.unwrap();
                    }
                }
            });
        }
        Self {
            origin,
            origin_hits,
            cross_hits,
            _tasks: tasks,
        }
    }
}

#[tokio::test]
async fn worker_module_csp_redirect_privacy_in_report_uri_and_report_to() {
    ensure_v8();
    for module_type in ["text", "javascript"] {
        for report_only in [false, true] {
            for report_to in [false, true] {
                let servers = RedirectServers::spawn(module_type, false).await;
                let reporter = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let endpoint = format!("http://{}/report", reporter.local_addr().unwrap());
                let (tx, rx) = tokio::sync::oneshot::channel();
                let report_task = tokio::spawn(async move {
                    let (mut stream, _) = reporter.accept().await.unwrap();
                    let request = read_http_request_with_body(&mut stream).await.unwrap();
                    stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    tx.send(request).unwrap();
                });
                let report_directive = if report_to {
                    "report-to csp".to_owned()
                } else {
                    format!("report-uri {endpoint}")
                };
                let restrictive =
                    format!("connect-src 'self'; script-src 'self'; {report_directive}");
                let source = format!(
                    r#"
                  const events = [];
                  addEventListener('securitypolicyviolation', event => events.push(event.blockedURI));
                  try {{ await import('./start', {}); postMessage({{status:'loaded', events}}); }}
                  catch (error) {{ postMessage({{status:error.name, message:error.message, events}}); }}
                  close();
                "#,
                    if module_type == "text" {
                        "{with:{type:'text'}}"
                    } else {
                        "{}"
                    }
                );
                let script_url = url::Url::parse(&format!("{}/root.js", servers.origin)).unwrap();
                let endpoints = crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
                    &[("Reporting-Endpoints".into(), format!("csp=\"{endpoint}\"").into_bytes())], &script_url,
                );
                let mut options = WorkerSpawnOptions::new(source, script_url.to_string())
                    .with_request_client(
                        ResourceRequestClient::new(&FetchConfig::default()).unwrap(),
                    )
                    .with_script_kind(WorkerScriptKind::Module)
                    .with_content_security_policies(vec![if report_only {
                        "connect-src *; script-src *".into()
                    } else {
                        restrictive.clone().into()
                    }])
                    .with_content_security_reporting_endpoints(endpoints);
                if report_only {
                    options = options
                        .with_content_security_report_only_policies(vec![restrictive.into()]);
                }
                let mut worker = spawn_test_worker_with_options(options);
                let message =
                    expect_post_json(timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap());
                assert!(
                    !message.contains("SECRET-redirect") && !message.contains("/middle"),
                    "{message}"
                );
                let result: serde_json::Value = serde_json::from_str(&message).unwrap();
                assert_eq!(
                    result["status"],
                    if report_only { "loaded" } else { "TypeError" }
                );
                assert_eq!(
                    result["events"],
                    serde_json::json!([format!("{}/start", servers.origin)])
                );
                let report = timeout(TIMEOUT, rx)
                    .await
                    .expect("redirect CSP report timed out")
                    .unwrap();
                report_task.await.unwrap();
                let body = report.split_once("\r\n\r\n").unwrap().1;
                assert!(
                    !body.contains("SECRET-redirect") && !body.contains("/middle"),
                    "{body}"
                );
                let body: serde_json::Value = serde_json::from_str(body).unwrap();
                assert_eq!(
                    if report_to {
                        &body[0]["body"]["blockedURL"]
                    } else {
                        &body["csp-report"]["blocked-uri"]
                    },
                    &serde_json::json!(format!("{}/start", servers.origin))
                );
                assert_eq!(
                    servers.cross_hits.load(Ordering::SeqCst),
                    usize::from(report_only)
                );
                worker.terminate_and_join();
            }
        }
    }
}

#[tokio::test]
async fn worker_module_csp_redirects_block_network_hops_and_report_intermediate_urls() {
    ensure_v8();
    for module_type in ["text", "json", "javascript", "wasm"] {
        for static_import in [false, true] {
            for report_only in [false, true] {
                // Static graphs use the outside enforced policy, as before.
                if static_import && report_only {
                    continue;
                }
                for roundtrip in [false, true] {
                    let servers = RedirectServers::spawn(module_type, roundtrip).await;
                    let attributes = if matches!(module_type, "text" | "json") {
                        format!("with {{type:'{module_type}'}}")
                    } else {
                        String::new()
                    };
                    let source = if static_import {
                        format!(
                            "import * as module from './start' {attributes}; postMessage('loaded'); close();"
                        )
                    } else {
                        let options = if attributes.is_empty() {
                            "{}".to_owned()
                        } else {
                            format!("{{with:{{type:'{module_type}'}}}}")
                        };
                        format!(
                            r#"
                          const events = [];
                          addEventListener('securitypolicyviolation', e => events.push([e.effectiveDirective, e.disposition, e.blockedURI]));
                          try {{ await import('./start', {options}); postMessage({{status:'loaded', events}}); }}
                          catch (error) {{ postMessage({{status:error.name, message:error.message, events}}); }}
                          close();
                        "#
                        )
                    };
                    let restrictive = "connect-src 'self'; worker-src 'self'; script-src 'self' 'wasm-unsafe-eval'";
                    let permissive = "connect-src *; worker-src *; script-src * 'wasm-unsafe-eval'";
                    let mut options =
                        WorkerSpawnOptions::new(source, format!("{}/root.js", servers.origin))
                            .with_request_client(
                                ResourceRequestClient::new(&FetchConfig::default()).unwrap(),
                            )
                            .with_script_kind(WorkerScriptKind::Module)
                            .with_module_static_import_initiator_url(
                                url::Url::parse(&format!("{}/page.html", servers.origin)).unwrap(),
                            )
                            .with_module_static_import_content_security_policies(vec![
                                restrictive.into(),
                            ])
                            .with_content_security_policies(vec![
                                if static_import || report_only {
                                    permissive
                                } else {
                                    restrictive
                                }
                                .into(),
                            ]);
                    if report_only {
                        options = options
                            .with_content_security_report_only_policies(vec![restrictive.into()]);
                    }
                    let mut worker = spawn_test_worker_with_options(options);
                    let result = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
                    if static_import {
                        match result {
                            WorkerToParentMessage::Error { message, .. } => assert!(
                                message.contains("Content Security Policy")
                                    && !message.contains("SECRET-redirect")
                                    && !message.contains("/middle"),
                                "{module_type}: {message}"
                            ),
                            other => panic!(
                                "static {module_type}: expected blocked redirect, got {other:?}"
                            ),
                        }
                    } else {
                        let result: serde_json::Value =
                            serde_json::from_str(&expect_post_json(result)).unwrap();
                        assert_eq!(
                            result["status"],
                            if report_only { "loaded" } else { "TypeError" },
                            "{module_type}, roundtrip={roundtrip}: {result}"
                        );
                        assert_eq!(result["events"].as_array().unwrap().len(), 1, "{result}");
                        assert_eq!(
                            result["events"][0][0],
                            if matches!(module_type, "text" | "json") {
                                "connect-src"
                            } else {
                                "script-src-elem"
                            }
                        );
                        assert_eq!(
                            result["events"][0][1],
                            if report_only { "report" } else { "enforce" }
                        );
                        assert_eq!(result["events"][0][2], format!("{}/start", servers.origin));
                        assert!(!result.to_string().contains("SECRET-redirect"), "{result}");
                        assert!(!result.to_string().contains("/middle"), "{result}");
                    }
                    assert_eq!(
                        servers.cross_hits.load(Ordering::SeqCst),
                        usize::from(report_only),
                        "{module_type}: blocked host was contacted"
                    );
                    assert_eq!(
                        servers.origin_hits.load(Ordering::SeqCst),
                        if roundtrip && report_only { 2 } else { 1 }
                    );
                    worker.terminate_and_join();
                }
            }
        }
    }
}

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
