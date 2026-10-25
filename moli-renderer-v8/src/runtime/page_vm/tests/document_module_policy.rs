use super::*;

async fn held_module_response(
    kind: &'static str,
    root_source: Option<String>,
) -> (
    String,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<anyhow::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        if let Some(source) = root_source {
            let (mut stream, _) = listener.accept().await?;
            let head = read_http_request_head(&mut stream).await?;
            assert!(head.starts_with("GET /entry.js "), "{head}");
            stream
                .write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{source}",
                    source.len(),
                ).as_bytes())
                .await?;
        }
        let (mut stream, _) = listener.accept().await?;
        let head = read_http_request_head(&mut stream).await?;
        assert!(head.starts_with("GET /module.js "), "{head}");
        seen_tx.send(()).unwrap();
        release_rx.await?;
        let (body, mime) = match kind {
            "json" => (r#"{"answer":42}"#, "application/json"),
            "text" => ("module-text", "text/plain"),
            _ => ("export default 42;", "text/javascript"),
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        stream.write_all(response.as_bytes()).await?;
        anyhow::Ok(())
    });
    (base, seen_rx, release_tx, server)
}

fn queue_import(page_vm: &mut PageVm, child: bool, kind: &str, url: &str) -> anyhow::Result<()> {
    if child {
        page_vm.vm_mut().eval("const frame=document.createElement('iframe');frame.id='module-policy';document.body.appendChild(frame);")?;
        materialize_child_realm_through_page_turn_for_test(page_vm, "module-policy")?;
    }
    let expression = if kind == "javascript" {
        format!("import({url:?})")
    } else {
        format!("import({url:?},{{with:{{type:{kind:?}}}}})")
    };
    page_vm.vm_mut().eval(&format!(
        r#"
globalThis.modulePolicyRows=[];
globalThis.modulePolicyStatus='pending';
globalThis.modulePolicyReturned=false;
const view={};
view.document.addEventListener('securitypolicyviolation',event=>{{
  if(!event.blockedURI.endsWith('/module.js'))return;
  modulePolicyRows.push([event.disposition,event.originalPolicy,
    event instanceof view.SecurityPolicyViolationEvent,event.isTrusted,
    event.target===view.document,modulePolicyReturned,modulePolicyStatus]);
  queueMicrotask(()=>modulePolicyRows.push('microtask'));
}});
view.eval({expression:?}).then(module=>{{
  globalThis.modulePolicyValue={};
  modulePolicyStatus='fulfilled';
}},error=>{{
  globalThis.modulePolicyFailure=[error.name,error instanceof view.TypeError];
  modulePolicyStatus='rejected';
}});
modulePolicyReturned=true;
"#,
        if child {
            "document.getElementById('module-policy').contentWindow"
        } else {
            "window"
        },
        if kind == "json" {
            "module.default.answer"
        } else {
            "module.default"
        }
    ))?;
    Ok(())
}

fn queue_runtime_module(page_vm: &mut PageVm, child: bool, url: &str) -> anyhow::Result<()> {
    // The parsed test fixture models insertion after DOMContentLoaded, as in
    // the CDP reproducer. Runtime module work otherwise waits for that boundary.
    page_vm
        .vm_mut()
        .document_runtime
        .note_dom_content_loaded_dispatched();
    if child {
        page_vm.vm_mut().eval("const frame=document.createElement('iframe');frame.id='module-policy';document.body.appendChild(frame);")?;
        materialize_child_realm_through_page_turn_for_test(page_vm, "module-policy")?;
    }
    page_vm.vm_mut().eval(&format!(
        r#"
globalThis.modulePolicyRows=[];
globalThis.modulePolicyUrls=[];
globalThis.modulePolicyStatus='pending';
globalThis.modulePolicyReturned=false;
const view={};
view.document.addEventListener('securitypolicyviolation',event=>{{
  if(!event.blockedURI.endsWith('/module.js'))return;
  modulePolicyRows.push([event.disposition,event.originalPolicy,
    event instanceof view.SecurityPolicyViolationEvent,event.isTrusted,
    event.target===view.document,modulePolicyReturned,modulePolicyStatus]);
  modulePolicyUrls.push([event.blockedURI,event.sourceFile]);
  queueMicrotask(()=>modulePolicyRows.push('microtask'));
}});
const script=view.document.createElement('script');
script.type='module';
script.src={url:?};
script.onload=()=>{{modulePolicyStatus='fulfilled';}};
script.onerror=event=>{{
  globalThis.modulePolicyFailure=[event.type,event instanceof view.Event,event.isTrusted,event.target===script];
  modulePolicyStatus='rejected';
}};
view.document.body.appendChild(script);
modulePolicyReturned=true;
"#,
        if child { "document.getElementById('module-policy').contentWindow" } else { "window" },
    ))?;
    Ok(())
}

fn policies(kind: &str) -> Vec<String> {
    if kind == "javascript" {
        vec![
            "script-src 'unsafe-eval'".into(),
            "default-src 'none'; script-src 'unsafe-eval'".into(),
        ]
    } else {
        vec![
            "connect-src 'none'".into(),
            "default-src 'none'; script-src 'self' 'unsafe-eval'".into(),
        ]
    }
}

fn dependency_policies(kind: &str, base: &str) -> Vec<String> {
    if kind == "javascript" {
        vec![
            format!("script-src 'unsafe-eval' {base}/entry.js"),
            format!("default-src 'none'; script-src 'unsafe-eval' {base}/entry.js"),
        ]
    } else {
        policies(kind)
    }
}

fn dependency_root_source(kind: &str) -> String {
    let attributes = if kind == "javascript" {
        String::new()
    } else {
        format!(" with {{type:{kind:?}}}")
    };
    let value = if kind == "json" {
        "value.answer"
    } else {
        "value"
    };
    format!(
        "import value from './module.js'{attributes}; globalThis.moduleDependencyValue={value};"
    )
}

fn expected_rows(policies: &[String], dispositions: &[&str]) -> serde_json::Value {
    let mut rows = Vec::new();
    for disposition in dispositions {
        for policy in policies {
            rows.push(json!([
                disposition,
                policy,
                true,
                true,
                true,
                true,
                "pending"
            ]));
            rows.push(json!("microtask"));
        }
    }
    json!(rows)
}

#[tokio::test(flavor = "current_thread")]
async fn document_module_policy_reports_dispatch_before_response_headers() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            for kind in ["javascript", "json", "text"] {
                let (base, seen, release, server) = held_module_response(kind, None).await;
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse(&format!("{base}/page.html"))?,
                );
                let policies = policies(kind);
                if kind != "javascript" {
                    page_vm.vm_mut().set_response_content_security_policies(&[
                        "script-src 'unsafe-eval'".into(),
                    ]);
                }
                page_vm
                    .vm_mut()
                    .set_response_content_security_report_only_policies(&policies);
                let executor = page_vm.local_executor.clone();
                executor
                    .run(async move {
                        queue_import(&mut page_vm, child, kind, &format!("{base}/module.js"))?;
                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(modulePolicyRows.length === 4)",
                            "Document module reports before response headers",
                        )
                        .await?;
                        tokio::time::timeout(Duration::from_secs(5), seen).await??;
                        let expected = expected_rows(&policies, &["report"]);
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(
                                &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                            )?,
                            expected
                        );
                        assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "pending");
                        release.send(()).unwrap();
                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(modulePolicyStatus !== 'pending')",
                            "Document module response release",
                        )
                        .await?;
                        assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "fulfilled");
                        assert_eq!(
                            page_vm.vm_mut().eval("JSON.stringify(modulePolicyValue)")?,
                            if kind == "text" {
                                "\"module-text\""
                            } else {
                                "42"
                            }
                        );
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(
                                &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                            )?,
                            expected
                        );
                        server.await??;
                        anyhow::Ok(())
                    })
                    .await?;
            }
        }
        anyhow::Ok(())
    })
    .await
    .expect("Document module reports must not wait for the response");
}

#[tokio::test(flavor = "current_thread")]
async fn document_module_policy_reports_checkpoint_before_enforced_rejection_without_network() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            for kind in ["javascript", "json", "text"] {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let base = format!("http://{}", listener.local_addr()?);
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse(&format!("{base}/page.html"))?,
                );
                let policies = policies(kind);
                page_vm
                    .vm_mut()
                    .set_response_content_security_policies(&policies);
                page_vm
                    .vm_mut()
                    .set_response_content_security_report_only_policies(&policies);
                let executor = page_vm.local_executor.clone();
                executor
                    .run(async move {
                        queue_import(&mut page_vm, child, kind, &format!("{base}/module.js"))?;
                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(modulePolicyStatus !== 'pending')",
                            "Document module CSP rejection",
                        )
                        .await?;
                        assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "rejected");
                        assert_eq!(
                            page_vm
                                .vm_mut()
                                .eval("JSON.stringify(modulePolicyFailure)")?,
                            "[\"TypeError\",true]"
                        );
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(
                                &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                            )?,
                            expected_rows(&policies, &["report", "enforce"])
                        );
                        assert!(
                            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                                .await
                                .is_err(),
                            "CSP must block before transport"
                        );
                        anyhow::Ok(())
                    })
                    .await?;
            }
        }
        anyhow::Ok(())
    })
    .await
    .expect("each Document module policy report must precede promise rejection");
}

#[tokio::test(flavor = "current_thread")]
async fn document_module_policy_preserves_parser_inserted_metadata() {
    run_page_vm_async_test(async move {
        let mut page_vm =
            test_page_vm_with_document_url(Url::parse("https://module-policy.test/page")?);
        page_vm.vm_mut().set_response_content_security_policies(&[
            "script-src 'nonce-good' 'strict-dynamic'".into(),
        ]);
        for parser_inserted in [false, true] {
            let metadata = crate::module_runtime::ModuleFetchMetadata {
                parser_inserted,
                ..Default::default()
            };
            let context = page_vm
                .vm_mut()
                .capture_module_fetch_context(
                    crate::native_bridge::OwnerDispatchScope::Top,
                    crate::module_runtime::ModuleKind::JavaScript,
                    &metadata,
                    None,
                )
                .map_err(anyhow::Error::msg)?;
            assert_eq!(
                context
                    .check_initial(&Url::parse("https://module-policy.test/module.js")?)
                    .is_err(),
                parser_inserted
            );
        }
        anyhow::Ok(())
    })
    .await
    .expect("module policy must retain the originating parser metadata");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_module_policy_reports_dispatch_before_response_headers() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            let (base, seen, release, server) = held_module_response("javascript", None).await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
            let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse(&format!("{base}/page.html"))?,
            );
            let policies = policies("javascript");
            page_vm
                .vm_mut()
                .set_response_content_security_report_only_policies(&policies);
            let executor = page_vm.local_executor.clone();
            executor
                .run(async move {
                    queue_runtime_module(&mut page_vm, child, &format!("{base}/module.js"))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(modulePolicyRows.length === 4)",
                        &format!("runtime module reports before response headers, child={child}"),
                    )
                    .await?;
                    tokio::time::timeout(Duration::from_secs(5), seen).await??;
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(
                            &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                        )?,
                        expected_rows(&policies, &["report"]),
                        "child={child}"
                    );
                    assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "pending");
                    release.send(()).unwrap();
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(modulePolicyStatus !== 'pending')",
                        &format!("runtime module response release, child={child}"),
                    )
                    .await?;
                    assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "fulfilled");
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(
                            &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                        )?,
                        expected_rows(&policies, &["report"]),
                        "child={child}"
                    );
                    server.await??;
                    anyhow::Ok(())
                })
                .await?;
        }
        anyhow::Ok(())
    })
    .await
    .expect("runtime module reports must not wait for response headers");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_module_policy_reports_before_element_error_without_network() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let base = format!("http://{}", listener.local_addr()?);
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
            let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse(&format!("{base}/page.html"))?,
            );
            let policies = policies("javascript");
            page_vm
                .vm_mut()
                .set_response_content_security_policies(&policies);
            page_vm
                .vm_mut()
                .set_response_content_security_report_only_policies(&policies);
            let executor = page_vm.local_executor.clone();
            executor
                .run(async move {
                    queue_runtime_module(&mut page_vm, child, &format!("{base}/module.js"))?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(modulePolicyStatus !== 'pending')",
                        &format!("runtime module policy rejection, child={child}"),
                    )
                    .await?;
                    assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "rejected");
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(
                            &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                        )?,
                        expected_rows(&policies, &["report", "enforce"]),
                        "child={child}"
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("JSON.stringify(modulePolicyFailure)")?,
                        r#"["error",true,true,true]"#,
                        "child={child}"
                    );
                    assert!(
                        tokio::time::timeout(Duration::from_millis(20), listener.accept())
                            .await
                            .is_err(),
                        "runtime module CSP must stop before TCP"
                    );
                    anyhow::Ok(())
                })
                .await?;
        }
        anyhow::Ok(())
    })
    .await
    .expect("all runtime module reports must checkpoint before its error event");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_module_redirect_policy_reports_before_contacting_the_blocked_origin() {
    run_page_vm_async_test(async move {
        let blocked = TcpListener::bind("127.0.0.1:0").await?;
        let target = format!("http://{}/module.js", blocked.local_addr()?);
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let head = read_http_request_head(&mut stream).await?;
            assert!(head.starts_with("GET /module.js "), "{head}");
            stream
                .write_all(format!(
                    "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ).as_bytes())
                .await?;
            anyhow::Ok(())
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse(&format!("{base}/page.html"))?,
        );
        let policies = vec![
            "script-src 'self' 'unsafe-eval'".to_owned(),
            "script-src-elem 'self'; script-src 'unsafe-eval'".to_owned(),
        ];
        page_vm
            .vm_mut()
            .set_response_content_security_policies(&policies);
        page_vm
            .vm_mut()
            .set_response_content_security_report_only_policies(&policies);
        let executor = page_vm.local_executor.clone();
        executor
            .run(async move {
                let request_url = format!("{base}/module.js");
                queue_runtime_module(&mut page_vm, false, &request_url)?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(modulePolicyStatus !== 'pending')",
                    "runtime module redirect policy rejection",
                )
                .await?;
                assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "rejected");
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(
                        &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                    )?,
                    expected_rows(&policies, &["report", "enforce"])
                );
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(
                        &page_vm.vm_mut().eval("JSON.stringify(modulePolicyUrls)")?
                    )?,
                    json!([
                        [request_url, ""],
                        [request_url, ""],
                        [request_url, ""],
                        [request_url, ""]
                    ])
                );
                assert!(
                    tokio::time::timeout(Duration::from_millis(20), blocked.accept())
                        .await
                        .is_err(),
                    "redirect CSP must stop before contacting the blocked origin"
                );
                server.await??;
                anyhow::Ok(())
            })
            .await
    })
    .await
    .expect("runtime module redirects must use the captured native CSP guard");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_module_dependency_policy_reports_before_response_headers() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            for kind in ["javascript", "json", "text"] {
                let (base, seen, release, server) =
                    held_module_response(kind, Some(dependency_root_source(kind))).await;
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse(&format!("{base}/page.html"))?,
                );
                let policies = dependency_policies(kind, &base);
                page_vm
                    .vm_mut()
                    .set_response_content_security_report_only_policies(&policies);
                let executor = page_vm.local_executor.clone();
                executor
                    .run(async move {
                        queue_runtime_module(&mut page_vm, child, &format!("{base}/entry.js"))?;
                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(modulePolicyRows.length === 4)",
                            &format!("static dependency reports, child={child}, kind={kind}"),
                        )
                        .await?;
                        tokio::time::timeout(Duration::from_secs(5), seen).await??;
                        let expected = expected_rows(&policies, &["report"]);
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(
                                &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                            )?,
                            expected,
                            "child={child}, kind={kind}"
                        );
                        assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "pending");
                        release.send(()).unwrap();
                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(modulePolicyStatus !== 'pending')",
                            &format!("static dependency response, child={child}, kind={kind}"),
                        )
                        .await?;
                        assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "fulfilled");
                        assert_eq!(
                            page_vm
                                .vm_mut()
                                .eval("JSON.stringify(view.moduleDependencyValue)")?,
                            if kind == "text" {
                                "\"module-text\""
                            } else {
                                "42"
                            }
                        );
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(
                                &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                            )?,
                            expected
                        );
                        server.await??;
                        anyhow::Ok(())
                    })
                    .await?;
            }
        }
        anyhow::Ok(())
    })
    .await
    .expect("static dependency reports must use the requesting Document before response headers");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_module_dependency_policy_blocks_before_network() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            for kind in ["javascript", "json", "text"] {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let base = format!("http://{}", listener.local_addr()?);
                let root_source = dependency_root_source(kind);
                let server = tokio::spawn(async move {
                    let (mut stream, _) = listener.accept().await?;
                    let head = read_http_request_head(&mut stream).await?;
                    assert!(head.starts_with("GET /entry.js "), "{head}");
                    stream
                        .write_all(format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{root_source}",
                            root_source.len(),
                        ).as_bytes())
                        .await?;
                    anyhow::Ok(listener)
                });
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse(&format!("{base}/page.html"))?,
                );
                let policies = dependency_policies(kind, &base);
                page_vm.vm_mut().set_response_content_security_policies(&policies);
                page_vm
                    .vm_mut()
                    .set_response_content_security_report_only_policies(&policies);
                let executor = page_vm.local_executor.clone();
                executor
                    .run(async move {
                        queue_runtime_module(&mut page_vm, child, &format!("{base}/entry.js"))?;
                        drive_websocket_until_done(
                            &mut page_vm,
                            "String(modulePolicyStatus !== 'pending')",
                            &format!("static dependency rejection, child={child}, kind={kind}"),
                        )
                        .await?;
                        assert_eq!(page_vm.vm_mut().eval("modulePolicyStatus")?, "rejected");
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(
                                &page_vm.vm_mut().eval("JSON.stringify(modulePolicyRows)")?
                            )?,
                            expected_rows(&policies, &["report", "enforce"]),
                            "child={child}, kind={kind}"
                        );
                        assert_eq!(
                            page_vm.vm_mut().eval("JSON.stringify(modulePolicyFailure)")?,
                            r#"["error",true,true,true]"#,
                            "child={child}, kind={kind}"
                        );
                        let listener = server.await??;
                        assert!(
                            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                                .await
                                .is_err(),
                            "static dependency CSP must stop before TCP, child={child}, kind={kind}"
                        );
                        anyhow::Ok(())
                    })
                    .await?;
            }
        }
        anyhow::Ok(())
    })
    .await
    .expect("static dependency policy reports must precede its error event and prevent transport");
}
