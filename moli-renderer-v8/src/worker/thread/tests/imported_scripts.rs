use super::*;

struct ImportScriptHttpServer {
    url: String,
    requests: Arc<parking_lot::Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl ImportScriptHttpServer {
    async fn spawn(response: String) -> Self {
        Self::spawn_with_response(move |_, _| response.clone()).await
    }

    async fn spawn_with_response(
        mut response: impl FnMut(&str, usize) -> String + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let observed = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request_head(&mut stream).await.unwrap();
                let count = {
                    let mut requests = observed.lock();
                    requests.push(request.clone());
                    requests.len()
                };
                let response = response(&request, count);
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}

impl Drop for ImportScriptHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn import_script_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn service_script_map_options(source: String, script_url: String) -> WorkerSpawnOptions {
    let scope_url = url::Url::parse(&script_url).unwrap().join(".").unwrap();
    WorkerSpawnOptions::new(source, script_url).with_global_kind(
        super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url,
        },
    )
}

async fn import_script_console_and_resources(
    handle: &mut WorkerHandle,
) -> (String, Vec<crate::worker::WorkerScriptResource>) {
    let mut resources = Vec::new();
    loop {
        match timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap() {
            WorkerToParentMessage::Console(message) => return (message.message, resources),
            WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { resource, .. } => {
                resources.push(resource);
            }
            WorkerToParentMessage::Error { message, .. } => panic!("worker error: {message}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn service_worker_importscripts_reuses_bytes_but_other_workers_fetch_each_time() {
    ensure_v8();
    for kind in ["service", "dedicated", "shared"] {
        let server = ImportScriptHttpServer::spawn_with_response(|_, count| {
            import_script_response(&format!("self.versions.push({count});"))
        })
        .await;
        let script_url = format!("{}/sw.js", server.url);
        let source = r#"
            self.versions = [];
            importScripts('./dep.js#same', './dep.js#same');
            importScripts('./dep.js#same');
            console.log(JSON.stringify(versions));
        "#;
        let mut options = service_script_map_options(source.into(), script_url.clone());
        if kind == "dedicated" {
            options = options.with_global_kind(super::super::WorkerGlobalKind::Dedicated {
                name: String::new(),
            });
        } else if kind == "shared" {
            options = options.with_global_kind(super::super::WorkerGlobalKind::Shared {
                name: String::new(),
                storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(
                    &url::Url::parse(&script_url).unwrap(),
                    None,
                ),
            });
        }
        let mut handle = spawn_test_worker_with_options(options);
        let (console, resources) = import_script_console_and_resources(&mut handle).await;
        let (expected, requests, imports) = if kind == "service" {
            ("log: [1,1,1]", 1, 1)
        } else {
            ("log: [1,2,3]", 3, 0)
        };
        assert_eq!(console, expected, "{kind}");
        assert_eq!(server.requests.lock().len(), requests, "{kind}");
        assert_eq!(resources.len(), imports, "{kind}");
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn service_worker_importscripts_preserves_fragment_keys_across_restarts() {
    ensure_v8();
    let server = ImportScriptHttpServer::spawn_with_response(|_, count| {
        import_script_response(&format!("self.versions.push({count});"))
    })
    .await;
    let script_url = format!("{}/sw.js", server.url);
    let source = r#"
        self.versions = [];
        importScripts('./dep.js#first', './dep.js#second', './dep.js#first');
        console.log(JSON.stringify(versions));
    "#;
    let mut handle = spawn_test_worker_with_options(service_script_map_options(
        source.into(),
        script_url.clone(),
    ));
    let (console, resources) = import_script_console_and_resources(&mut handle).await;
    assert_eq!(console, "log: [1,2,1]");
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].request_url.fragment(), Some("first"));
    assert_eq!(resources[1].request_url.fragment(), Some("second"));
    handle.terminate_and_join();
    let mut restored = spawn_test_worker_with_options(
        service_script_map_options(
            r#"
        self.versions = [];
        importScripts('./dep.js#first', './dep.js#second');
        const errors = [];
        for (const url of ['./dep.js', './dep.js#third']) {
            try { importScripts(url); errors.push('ok'); }
            catch (error) { errors.push(error.name); }
        }
        console.log(JSON.stringify([versions, errors]));
        "#
            .into(),
            script_url,
        )
        .with_service_worker_script_resources(resources, false),
    );
    let (console, _) = import_script_console_and_resources(&mut restored).await;
    assert_eq!(console, "log: [[1,2],[\"NetworkError\",\"NetworkError\"]]");
    assert_eq!(server.requests.lock().len(), 2);
    restored.terminate_and_join();
}

#[tokio::test]
async fn service_worker_importscripts_keeps_redirect_aliases_in_separate_map_entries() {
    ensure_v8();
    let server = ImportScriptHttpServer::spawn_with_response(|request, count| {
        if request.starts_with("GET /shared.js ") {
            import_script_response(&format!("self.versions.push({count});"))
        } else {
            "HTTP/1.1 302 Found\r\nLocation: /shared.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
        }
    }).await;
    let mut handle = spawn_test_worker_with_options(service_script_map_options(
        r#"
        self.versions = [];
        importScripts('./alias-a.js', './alias-b.js', './alias-a.js');
        console.log(JSON.stringify(versions));
        "#
        .into(),
        format!("{}/sw.js", server.url),
    ));
    let (console, resources) = import_script_console_and_resources(&mut handle).await;
    assert_eq!(console, "log: [2,4,2]");
    assert_eq!(server.requests.lock().len(), 4);
    assert_eq!(resources.len(), 2);
    assert_ne!(resources[0].request_url, resources[1].request_url);
    assert_eq!(resources[0].final_url, resources[1].final_url);
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_importscripts_freezes_map_after_install_wait_until() {
    ensure_v8();
    let server = ImportScriptHttpServer::spawn(import_script_response("self.count++;")).await;
    let mut handle = spawn_test_worker_with_options(service_script_map_options(
        r#"
        self.count = 0;
        const data = 'data:text/javascript,self.count++';
        importScripts(data);
        addEventListener('install', event => {
            importScripts('./install.js');
            event.waitUntil(new Promise((resolve, reject) => setTimeout(() => {
                try { importScripts('./async-install.js'); resolve(); }
                catch (error) { reject(error); }
            }, 0)));
        });
        addEventListener('activate', () => {
            importScripts('./install.js', './async-install.js', data);
            const errors = [];
            for (const url of ['./new.js', data + ';']) {
                try { importScripts(url); errors.push('unexpected success'); }
                catch (error) { errors.push(error.name); }
            }
            console.log(JSON.stringify([count, errors]));
        });
        "#
        .into(),
        format!("{}/sw.js", server.url),
    ));
    let install = dispatch_service_worker_lifecycle_event_for_test(
        &mut handle,
        ServiceWorkerLifecycleEventKind::Install,
        1,
    )
    .await;
    assert_eq!(install.result, Ok(()));
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(2),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        kind: ServiceWorkerLifecycleEventKind::Activate,
    });
    let (console, _) = import_script_console_and_resources(&mut handle).await;
    assert_eq!(console, "log: [6,[\"NetworkError\",\"NetworkError\"]]");
    assert_eq!(
        server.requests.lock().len(),
        2,
        "new URL must never reach the server"
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_importscripts_restores_throwing_responses_and_muted_errors() {
    ensure_v8();
    for (foreign, source, expected) in [
        (false, "throw new RangeError('original');", "RangeError"),
        (false, "const = syntax error", "SyntaxError"),
        (true, "throw new RangeError('private');", "NetworkError"),
    ] {
        let server = ImportScriptHttpServer::spawn_with_response(move |_, count| {
            import_script_response(if count == 1 {
                source
            } else {
                "self.changed = true;"
            })
        })
        .await;
        let script_url = if foreign {
            "http://127.0.0.1/sw.js".to_owned()
        } else {
            format!("{}/sw.js", server.url)
        };
        let dep_url = serde_json::to_string(&format!("{}/dep.js", server.url)).unwrap();
        let source = format!(
            r#"
            const errors = [];
            for (let i = 0; i < 2; ++i) {{
                try {{ importScripts({dep_url}); errors.push('ok'); }}
                catch (error) {{ errors.push(error.name); }}
            }}
            console.log(JSON.stringify(errors));
        "#
        );
        let mut first = spawn_test_worker_with_options(service_script_map_options(
            source.clone(),
            script_url.clone(),
        ));
        let (console, resources) = import_script_console_and_resources(&mut first).await;
        assert_eq!(console, format!("log: [\"{expected}\",\"{expected}\"]"));
        assert_eq!(
            resources.len(),
            1,
            "successful fetch is cached before evaluation"
        );
        first.terminate_and_join();
        let mut restored = spawn_test_worker_with_options(
            service_script_map_options(source, script_url)
                .with_service_worker_script_resources(resources, false),
        );
        let (restored_console, resources) =
            import_script_console_and_resources(&mut restored).await;
        assert_eq!(restored_console, console);
        assert!(resources.is_empty(), "restored imports must not refetch");
        assert_eq!(server.requests.lock().len(), 1);
        restored.terminate_and_join();
    }
}

#[tokio::test]
async fn service_worker_importscripts_retries_failed_fetches_during_installation() {
    ensure_v8();
    let server = ImportScriptHttpServer::spawn_with_response(|_, count| {
        if count == 1 {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
        } else {
            import_script_response("self.count++;")
        }
    })
    .await;
    let mut handle = spawn_test_worker_with_options(service_script_map_options(
        r#"
        self.count = 0;
        const results = [];
        for (let i = 0; i < 3; ++i) {
            try { importScripts('./dep.js'); results.push('ok'); }
            catch (error) { results.push(error.name); }
        }
        console.log(JSON.stringify([count, results]));
        "#
        .into(),
        format!("{}/sw.js", server.url),
    ));
    let (console, resources) = import_script_console_and_resources(&mut handle).await;
    assert_eq!(console, "log: [2,[\"NetworkError\",\"ok\",\"ok\"]]");
    assert_eq!(resources.len(), 1);
    assert_eq!(server.requests.lock().len(), 2);
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_importscripts_checks_legacy_resource_hash_before_execution() {
    ensure_v8();
    for same_bytes in [true, false] {
        let server = ImportScriptHttpServer::spawn_with_response(move |_, count| {
            import_script_response(if count == 1 || same_bytes {
                "self.installed = true;"
            } else {
                "self.changed = true;"
            })
        })
        .await;
        let script_url = format!("{}/sw.js", server.url);
        let mut first = spawn_test_worker_with_options(service_script_map_options(
            "importScripts('./dep.js'); console.log('loaded');".into(),
            script_url.clone(),
        ));
        let (_, mut resources) = import_script_console_and_resources(&mut first).await;
        first.terminate_and_join();
        resources[0].classic_script = None;
        let mut restored = spawn_test_worker_with_options(
            service_script_map_options(
                r#"
            let result = 'ok';
            try { importScripts('./dep.js'); } catch (error) { result = error.name; }
            console.log(JSON.stringify([result, self.installed === true, self.changed === true]));
            "#
                .into(),
                script_url,
            )
            .with_service_worker_script_resources(resources, false),
        );
        let (console, _) = import_script_console_and_resources(&mut restored).await;
        assert_eq!(
            console,
            if same_bytes {
                "log: [\"ok\",true,false]"
            } else {
                "log: [\"NetworkError\",false,false]"
            }
        );
        assert_eq!(server.requests.lock().len(), 2);
        restored.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_importscripts_rejects_missing_or_invalid_http_mime_before_execution() {
    ensure_v8();
    for content_type in [
        None,
        Some(""),
        Some("not a mime type"),
        Some("text/javascript"),
    ] {
        let headers = content_type
            .map(|mime| format!("Content-Type: {mime}\r\n"))
            .unwrap_or_default();
        let body = "self.executed = true;";
        let server = ImportScriptHttpServer::spawn(format!(
            "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
        .await;
        let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
            r#"
            let result = 'ok';
            try { importScripts('./dep.js'); }
            catch (error) { result = error.name; }
            postMessage([result, self.executed === true]); close();
            "#
            .into(),
            format!("{}/worker.js", server.url),
        ));
        assert_eq!(
            recv_post_json(&mut handle).await,
            if content_type == Some("text/javascript") {
                "[\"ok\",true]"
            } else {
                "[\"NetworkError\",false]"
            },
            "{content_type:?}"
        );
        assert_eq!(server.requests.lock().len(), 1);
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_importscripts_fetches_data_urls_in_execution_order() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
        r#"
        let result = 'ok';
        try {
            importScripts('data:text/javascript,self.first = true', 'data:text/plain,ignored');
        } catch (error) { result = error.name; }
        postMessage([self.first === true, result]); close();
        "#
        .into(),
        "https://example.test/worker.js".into(),
    ));
    assert_eq!(recv_post_json(&mut handle).await, "[true,\"NetworkError\"]");
}

#[tokio::test]
async fn worker_importscripts_validates_captured_blob_entries_in_execution_order() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
        r#"
        const results = [];
        for (const type of ['text/plain', '', 'not a mime type']) {
            self.first = self.second = false;
            const badMime = URL.createObjectURL(new Blob(['self.second = true'], {type}));
            let result = 'ok';
            try { importScripts('data:text/javascript,self.first = true', badMime); }
            catch (error) { result = error.name; }
            URL.revokeObjectURL(badMime);
            results.push([self.first === true, self.second === true, result]);
        }
        postMessage(results); close();
        "#
        .into(),
        "https://example.test/worker.js".into(),
    ));
    assert_eq!(
        recv_post_json(&mut handle).await,
        "[[true,false,\"NetworkError\"],[true,false,\"NetworkError\"],[true,false,\"NetworkError\"]]"
    );
}

#[tokio::test]
async fn worker_importscripts_trusted_types_reports_keep_script_and_document_locations_separate() {
    ensure_v8();
    let source = "self.violatePolicy = () => {\n  try { trustedTypes.createPolicy('forbidden'); } catch {}\n};\nself.violateSink = () => {\n  try { setTimeout('blocked code'); } catch {}\n};\nviolatePolicy();\nviolateSink();";
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/imported/violations.js",
        "HTTP/1.1 200 OK",
        "text/javascript",
        source.into(),
        Duration::ZERO,
    )])
    .await;
    let worker_url = format!("{base_url}/worker/main.js");
    let options = WorkerSpawnOptions::new(
        r#"
        const violations = [];
        addEventListener('securitypolicyviolation', event => {
          violations.push([
            event.effectiveDirective, event.documentURI, event.sourceFile,
            event.lineNumber, event.columnNumber > 0
          ]);
          if (violations.length === 4) { postMessage(violations); close(); }
        });
        const setup = trustedTypes.createPolicy('bootstrap', { createScriptURL: s => s });
        importScripts(setup.createScriptURL('../imported/violations.js'));
        setTimeout(() => {
            violatePolicy(); violateSink();
        }, 0);
        "#
        .into(),
        worker_url.clone(),
    )
    .with_content_security_policies(vec![
        "require-trusted-types-for 'script'; trusted-types bootstrap".into(),
    ]);
    let mut handle = spawn_test_worker_with_options(options);
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    let script_url = format!("{base_url}/imported/violations.js");
    assert_eq!(
        actual,
        serde_json::json!([
            ["trusted-types", worker_url, script_url, 2, true],
            ["require-trusted-types-for", worker_url, script_url, 5, true],
            ["trusted-types", worker_url, script_url, 2, true],
            ["require-trusted-types-for", worker_url, script_url, 5, true],
        ])
    );
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_trusted_types_reports_do_not_expose_redirect_targets() {
    ensure_v8();
    let (foreign_url, foreign_server) = spawn_path_response_http_server(vec![(
        "/private-user/violations.js?credential=hidden",
        "HTTP/1.1 200 OK",
        "text/javascript",
        "setTimeout(() => {\n  try { trustedTypes.createPolicy('forbidden'); } catch {}\n}, 0);"
            .into(),
        Duration::ZERO,
    )])
    .await;
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {foreign_url}/private-user/violations.js?credential=hidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (base_url, server) = spawn_raw_path_response_http_server(vec![(
        "/worker/redirect.js",
        redirect,
        Duration::ZERO,
    )])
    .await;
    let worker_url = format!("{base_url}/worker/main.js");
    let options = WorkerSpawnOptions::new(
        r#"
        const violations = [];
        addEventListener('securitypolicyviolation', event => {
          violations.push([
            event.disposition, event.documentURI, event.sourceFile,
            event.lineNumber, event.columnNumber > 0
          ]);
          if (violations.length === 2) { postMessage(violations); close(); }
        });
        importScripts('./redirect.js');
        "#
        .into(),
        worker_url.clone(),
    )
    .with_content_security_policies(vec!["trusted-types 'none'".into()])
    .with_content_security_report_only_policies(vec!["trusted-types 'none'".into()]);
    let mut handle = spawn_test_worker_with_options(options);
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    let request_url = format!("{base_url}/worker/redirect.js");
    assert_eq!(
        actual,
        serde_json::json!([
            ["enforce", worker_url, request_url, 2, true],
            ["report", worker_url, request_url, 2, true],
        ])
    );
    server.await.unwrap();
    foreign_server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_respects_corp_and_coep() {
    ensure_v8();
    for (require_corp, corp, expected) in [
        (false, "same-origin", r#"["NetworkError",false]"#),
        (true, "", r#"["NetworkError",false]"#),
        (true, "cross-origin", r#"["ok",true]"#),
    ] {
        let body = "self.loaded = true;";
        let corp_header = if corp.is_empty() {
            String::new()
        } else {
            format!("Cross-Origin-Resource-Policy: {corp}\r\n")
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\n{corp_header}Content-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (foreign_url, server) =
            spawn_raw_path_response_http_server(vec![("/foreign.js", response, Duration::ZERO)])
                .await;
        let foreign = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
        let policy_context = crate::types::SubresourcePolicyContext {
            cross_origin_embedder_policy: if require_corp {
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp
            } else {
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::None
            },
            ..Default::default()
        };
        let options = WorkerSpawnOptions::new(
            format!(
                r#"
            let outcome = 'ok';
            try {{ importScripts({foreign}); }} catch (error) {{ outcome = error.name; }}
            postMessage([outcome, self.loaded === true]); close();
            "#
            ),
            "http://127.0.0.1/worker/main.js".into(),
        )
        .with_policy_context(policy_context);
        let mut handle = spawn_test_worker_with_options(options);
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert_eq!(
            expect_post_json(message),
            expected,
            "require_corp={require_corp}, corp={corp}"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn worker_importscripts_redirects_check_csp_and_ignore_redirected_paths() {
    ensure_v8();
    for (report_only, allow_foreign, expected) in [
        (false, false, r#"["NetworkError",false,["enforce"],true]"#),
        (true, false, r#"["ok",true,["report"],true]"#),
        (false, true, r#"["ok",true,[],true]"#),
    ] {
        let body = "self.loaded = true;";
        let foreign = ImportScriptHttpServer::spawn(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )).await;
        let foreign_url = &foreign.url;
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {foreign_url}/redirect-target/foreign.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (worker_url, server) = spawn_raw_path_response_http_server(vec![(
            "/worker/redirect.js",
            response,
            Duration::ZERO,
        )])
        .await;
        let policy = if allow_foreign {
            format!("script-src 'self' {foreign_url}/only-before-redirect/")
        } else {
            "script-src 'self'".to_owned()
        };
        let mut options = WorkerSpawnOptions::new(
            r#"
            const violations = [];
            let outcome = 'ok';
            try { importScripts('./redirect.js'); } catch (error) { outcome = error.name; }
            let microtaskRan = false;
            queueMicrotask(() => microtaskRan = true);
            const finish = () => {
                postMessage([outcome, self.loaded === true, violations, microtaskRan]);
                close();
            };
            if (EXPECT_VIOLATION) {
                addEventListener('securitypolicyviolation', event => {
                    violations.push(event.disposition);
                    finish();
                });
            } else {
                queueMicrotask(finish);
            }
            "#
            .replace(
                "EXPECT_VIOLATION",
                if allow_foreign { "false" } else { "true" },
            ),
            format!("{worker_url}/worker/main.js"),
        );
        options = if report_only {
            options.with_content_security_report_only_policies(vec![policy])
        } else {
            options.with_content_security_policies(vec![policy])
        };
        let mut handle = spawn_test_worker_with_options(options);
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert_eq!(
            expect_post_json(message),
            expected,
            "report={report_only}, allow={allow_foreign}"
        );
        server.await.unwrap();
        assert_eq!(
            foreign.requests.lock().len(),
            usize::from(report_only || allow_foreign),
            "an enforced CSP violation must stop before contacting the redirect target",
        );
    }
}

#[tokio::test]
async fn worker_importscripts_csp_reports_all_policies_before_and_after_redirects() {
    ensure_v8();
    for redirected in [false, true] {
        let foreign = ImportScriptHttpServer::spawn(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ).await;
        let target = format!("{}/private/script.js?secret=1", foreign.url);
        let source = ImportScriptHttpServer::spawn(format!(
            "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )).await;
        let request = if redirected {
            format!("{}/redirect.js", source.url)
        } else {
            target
        };
        let script = r#"
            let name;
            try {
                importScripts(REQUEST);
            } catch (error) { name = error.name; }
            let microtaskRan = false;
            queueMicrotask(() => microtaskRan = true);
            const events = [];
            addEventListener('securitypolicyviolation', event => {
                events.push([
                    event.originalPolicy, event.disposition, event.effectiveDirective,
                    event.blockedURI, event.sourceFile, event.lineNumber,
                    event.columnNumber, microtaskRan
                ]);
                if (events.length === 4) { postMessage({name, events}); close(); }
            });
        "#
        .replace("REQUEST", &serde_json::to_string(&request).unwrap());
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(script, format!("{}/worker.js?caller=1", source.url))
                .with_content_security_policies(vec![
                    "script-src 'self'".into(),
                    "script-src-elem 'self'".into(),
                ])
                .with_content_security_report_only_policies(vec![
                    "default-src 'self'".into(),
                    "script-src 'self'; script-src-elem 'self'".into(),
                ]),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
        let location = format!("{}/worker.js", source.url);
        assert_eq!(
            actual,
            serde_json::json!({
                "name": "NetworkError",
                "events": [
                    ["default-src 'self'", "report", "script-src-elem", request, location, 4, 17, true],
                    ["script-src 'self'; script-src-elem 'self'", "report", "script-src-elem", request, location, 4, 17, true],
                    ["script-src 'self'", "enforce", "script-src-elem", request, location, 4, 17, true],
                    ["script-src-elem 'self'", "enforce", "script-src-elem", request, location, 4, 17, true],
                ],
            })
        );
        assert_eq!(source.requests.lock().len(), usize::from(redirected));
        assert!(foreign.requests.lock().is_empty());
    }
}

#[tokio::test]
async fn worker_importscripts_cached_redirects_use_the_current_worker_policy() {
    ensure_v8();
    let body = "self.loaded = true;";
    let foreign = ImportScriptHttpServer::spawn(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )).await;
    let source = ImportScriptHttpServer::spawn(format!(
        "HTTP/1.1 302 Found\r\nLocation: {}/imported.js\r\nCache-Control: max-age=600\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        foreign.url
    )).await;
    let cache_dir = std::env::temp_dir().join(format!(
        "moli-worker-importscripts-csp-{}-{}",
        std::process::id(),
        fastrand::u64(..),
    ));
    let mut config = FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.display().to_string()));
    let client = ResourceRequestClient::new(&config).unwrap();
    for (blocked, contacts) in [(false, 1), (true, 1), (false, 2)] {
        let script = r#"
            let name;
            try { importScripts('./redirect.js'); } catch (error) { name = error.name; }
            if (name) {
                addEventListener('securitypolicyviolation', event => {
                    postMessage([name, self.loaded === true, event.effectiveDirective]);
                    close();
                });
            } else { postMessage(['ok', self.loaded === true]); close(); }
        "#;
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new_with_request_client(
                script.into(),
                format!("{}/worker.js", source.url),
                client.clone(),
            )
            .with_content_security_policies(vec![if blocked {
                "script-src 'self'".into()
            } else {
                "script-src http:".into()
            }]),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert_eq!(
            expect_post_json(message),
            if blocked {
                r#"["NetworkError",false,"script-src-elem"]"#
            } else {
                r#"["ok",true]"#
            }
        );
        assert_eq!(
            source.requests.lock().len(),
            1,
            "the redirect should come from the HTTP cache after the first load"
        );
        assert_eq!(
            foreign.requests.lock().len(),
            contacts,
            "cached redirects must honor the current worker's CSP before contacting the target"
        );
    }
    drop(client);
    std::fs::remove_dir_all(cache_dir).unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_does_not_bypass_module_cors() {
    ensure_v8();
    let (foreign_url, server) = spawn_path_response_http_server(vec![
        ("/foreign.js", "HTTP/1.1 200 OK", "text/javascript",
         "self.result = import(self.foreignModule).then(() => 'unexpected', error => error.name);".into(), Duration::ZERO),
        ("/foreign-module.js", "HTTP/1.1 200 OK", "text/javascript",
         "export const value = 'private module';".into(), Duration::ZERO),
    ]).await;
    let script = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
    let module = serde_json::to_string(&format!("{foreign_url}/foreign-module.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        self.foreignModule = {module};
        try {{
            importScripts({script});
            result.then(value => {{ postMessage(value); close(); }});
        }} catch (error) {{ postMessage('importScripts:' + error.name); close(); }}
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#""TypeError""#);
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_redirect_chain_stays_muted_after_returning() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let worker_url = format!("http://{}", listener.local_addr().unwrap());
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {worker_url}/worker/creator.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (foreign_url, foreign_server) =
        spawn_raw_path_response_http_server(vec![("/return.js", redirect, Duration::ZERO)]).await;
    let server = tokio::spawn(async move {
        let mut paths = Vec::new();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_head(&mut stream).await.unwrap();
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            paths.push(path.to_owned());
            let (status, extra, body) = match path {
                "/worker/redirect.js" => (
                    "302 Found",
                    format!("Location: {foreign_url}/return.js\r\n"),
                    "",
                ),
                "/worker/creator.js" => (
                    "200 OK",
                    String::new(),
                    "self.result = import('./leaf.js').then(() => 'unexpected', error => error.name);",
                ),
                "/worker/leaf.js" => (
                    "200 OK",
                    String::new(),
                    "export const value = 'unexpected';",
                ),
                "/done" => ("200 OK", String::new(), "done"),
                _ => panic!("unexpected importScripts request {path}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            if path == "/done" {
                return paths;
            }
        }
    });
    let mut handle = spawn_worker_with_request_client(
        r#"
        try {
            importScripts('./redirect.js');
            result.then(async value => { await fetch('/done'); postMessage(value); close(); });
        } catch (error) { postMessage('importScripts:' + error.name); close(); }
        "#
        .into(),
        format!("{worker_url}/worker/main.js"),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#""TypeError""#);
    assert_eq!(
        server.await.unwrap(),
        ["/worker/redirect.js", "/worker/creator.js", "/done"]
    );
    foreign_server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_keeps_settings_base_separate_from_dynamic_import_base() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/imported/creator.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "importScripts('./nested.js'); self.importLater = () => import('./leaf.js');".into(),
            Duration::ZERO,
        ),
        (
            "/worker/nested.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "self.nested = 'worker';".into(),
            Duration::ZERO,
        ),
        (
            "/imported/leaf.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "export const value = 'imported';".into(),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut handle = spawn_worker_with_request_client(
        r#"
        try {
            importScripts('../imported/creator.js');
            importLater().then(module => {
                postMessage({nested, value: module.value}); close();
            }, error => { postMessage({error: error.name}); close(); });
        } catch (error) { postMessage({error: error.name}); close(); }
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        r#"{"nested":"worker","value":"imported"}"#
    );
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_sanitizes_import_base_but_not_settings_origin() {
    ensure_v8();
    let (worker_url, worker_server) = spawn_path_response_http_server(vec![
        (
            "/worker/nested.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "self.nested = 'worker';".into(),
            Duration::ZERO,
        ),
        (
            "/worker/leaf.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "export const value = 'worker-module';".into(),
            Duration::ZERO,
        ),
    ])
    .await;
    let absolute = serde_json::to_string(&format!("{worker_url}/worker/leaf.js")).unwrap();
    let source = format!(
        r#"
        importScripts('./nested.js');
        self.relativeImport = import('./leaf.js').then(() => 'unexpected', error => error.name);
        self.absoluteImport = import({absolute}).then(module => module.value);
        self.importLater = () => import('./later.js').then(() => 'unexpected', error => error.name);
    "#
    );
    let (foreign_url, foreign_server) = spawn_path_response_http_server(vec![(
        "/foreign/creator.js",
        "HTTP/1.1 200 OK",
        "text/javascript",
        source,
        Duration::ZERO,
    )])
    .await;
    let foreign = serde_json::to_string(&format!("{foreign_url}/foreign/creator.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        try {{
            importScripts({foreign});
            Promise.all([relativeImport, absoluteImport, importLater()]).then(values => {{
                postMessage({{nested, values}}); close();
            }}, error => {{ postMessage({{error: error.name}}); close(); }});
        }} catch (error) {{ postMessage({{error: error.name}}); close(); }}
        "#
        ),
        format!("{worker_url}/worker/main.js"),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        r#"{"nested":"worker","values":["TypeError","worker-module","TypeError"]}"#
    );
    foreign_server.await.unwrap();
    worker_server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_async_errors_are_muted() {
    ensure_v8();
    let (foreign_url, server) = spawn_path_response_http_server(vec![(
        "/foreign.js",
        "HTTP/1.1 200 OK",
        "text/javascript",
        "setTimeout(() => { throw new Error('private message'); }, 0);".into(),
        Duration::ZERO,
    )])
    .await;
    let foreign = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        addEventListener('error', event => {{
            postMessage({{message: event.message, filename: event.filename,
                line: event.lineno, column: event.colno, errorIsNull: event.error === null}});
            event.preventDefault(); close();
        }});
        importScripts({foreign});
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        r#"{"message":"Script error.","filename":"","line":0,"column":0,"errorIsNull":true}"#
    );
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_rejections_are_not_reported() {
    ensure_v8();
    let (foreign_url, server) = spawn_path_response_http_server(vec![
        ("/foreign.js", "HTTP/1.1 200 OK", "text/javascript",
         "Promise.reject('private immediate'); setTimeout(() => Promise.reject('private timer'), 0);".into(), Duration::ZERO),
    ]).await;
    let foreign = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        const reasons = [];
        addEventListener('unhandledrejection', event => {{
            reasons.push(event.reason); event.preventDefault();
        }});
        try {{ importScripts({foreign}); }} catch (error) {{ reasons.push(error.name); }}
        Promise.reject('public');
        setTimeout(() => {{ postMessage(reasons); close(); }}, 0);
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#"["public"]"#);
    server.await.unwrap();
}
