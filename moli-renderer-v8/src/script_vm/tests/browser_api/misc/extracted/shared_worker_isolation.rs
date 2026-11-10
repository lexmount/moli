use super::*;

const WORKER_POLICY_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/shared-worker-isolation.js"
));

const WORKER_ANCESTOR_PROBE_HTML: &str = r#"<!doctype html><script>
    const dedicated = new Worker('dedicated.js', {
        type: location.hash.slice(1) || 'classic'
    });
    dedicated.onmessage = ({data: dedicatedState}) => {
        dedicated.terminate();
        const shared = new SharedWorker('shared.js');
        shared.port.onmessage = ({data}) => {
            top.postMessage(JSON.stringify({
                window: isSecureContext,
                dedicated: dedicatedState.secure,
                dedicatedSubtle: dedicatedState.subtle,
                shared: data.shared,
                nested: data.nested,
                subtle: typeof crypto.subtle !== 'undefined'
            }), '*');
            shared.port.close();
        };
        shared.onerror = error => top.postMessage(JSON.stringify({error: error.message}), '*');
        shared.port.start();
    };
    dedicated.onerror = error => top.postMessage(JSON.stringify({error: error.message}), '*');
    </script>"#;

const DEDICATED_ANCESTOR_PROBE: &str =
    "postMessage({secure: isSecureContext, subtle: typeof crypto.subtle !== 'undefined'});";

const SHARED_ANCESTOR_PROBE: &str = r#"onconnect = ({ports}) => {
    const worker = new Worker('nested.js');
    worker.onmessage = ({data}) => {
        ports[0].postMessage({shared: isSecureContext, nested: data});
        worker.terminate();
    };
};"#;

async fn receive_worker_ancestor_probe(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    runtime: &crate::runtime::RendererBrowserContextRuntime,
    loader: &ResourceRequestClient,
) -> serde_json::Value {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm.eval("typeof workerAncestorResult").unwrap() == "undefined" {
            runtime.drain_shared_worker_service_lane();
            drain_service_worker_test_turn(vm, runtime, loader).await;
        }
    })
    .await
    .expect("window and worker secure-context probes must reply");
    serde_json::from_str(&vm.eval("workerAncestorResult").unwrap()).unwrap()
}

#[tokio::test]
async fn dedicated_shared_and_nested_workers_keep_the_creators_insecure_ancestors() {
    for (parent_url, script_kind, expected_secure) in [
        ("http://insecure.example.test/parent", "classic", false),
        ("http://insecure.example.test/parent", "module", false),
        ("https://secure.example.test/parent", "classic", true),
        ("https://secure.example.test/parent", "module", true),
    ] {
        let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
            (
                "/child.html",
                "text/html",
                vec![],
                WORKER_ANCESTOR_PROBE_HTML,
            ),
            (
                "/dedicated.js",
                "text/javascript",
                vec![],
                DEDICATED_ANCESTOR_PROBE,
            ),
            (
                "/shared.js",
                "text/javascript",
                vec![],
                SHARED_ANCESTOR_PROBE,
            ),
            (
                "/nested.js",
                "text/javascript",
                vec![],
                "postMessage(isSecureContext);",
            ),
        ])
        .await;
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                parent_url, &loader,
            );
        vm.eval(&format!(
            r#"onmessage = ({{data}}) => globalThis.workerAncestorResult = data;
            const frame = document.createElement('iframe');
            frame.src = '{base_url}/child.html#{script_kind}';
            document.body.append(frame);"#
        ))
        .unwrap();
        let facts = receive_worker_ancestor_probe(&mut vm, &runtime, &loader).await;
        assert_eq!(
            facts,
            serde_json::json!({
                "window": expected_secure, "dedicated": expected_secure,
                "dedicatedSubtle": expected_secure, "shared": expected_secure,
                "nested": expected_secure, "subtle": expected_secure
            }),
            "{parent_url}, {script_kind}"
        );
        server.await.unwrap();
    }
}

async fn spawn_worker_https_response_server(
    tls: &moli_websocket::test_support::TlsWebSocketFixture,
    responses: Vec<(&'static str, &'static str, &'static str)>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = tls.server_acceptor(false);
    let server = tokio::spawn(async move {
        for (expected_path, content_type, body) in responses {
            let (socket, _) = listener.accept().await.unwrap();
            let mut stream = acceptor.accept(socket).await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0, "HTTPS worker request must contain headers");
                request.extend_from_slice(&buffer[..count]);
            }
            assert_eq!(
                String::from_utf8(request)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1),
                Some(expected_path)
            );
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    (format!("https://localhost:{port}"), server)
}

#[tokio::test]
async fn https_workers_keep_an_insecure_middle_frame_ancestor() {
    use crate::script_vm::tests::http_fixture::StaticHttpServer;

    for script_kind in ["classic", "module"] {
        let tls = moli_websocket::test_support::TlsWebSocketFixture::new();
        let (base_url, server) = spawn_worker_https_response_server(
            &tls,
            vec![
                ("/child.html", "text/html", WORKER_ANCESTOR_PROBE_HTML),
                ("/dedicated.js", "text/javascript", DEDICATED_ANCESTOR_PROBE),
                ("/shared.js", "text/javascript", SHARED_ANCESTOR_PROBE),
                (
                    "/nested.js",
                    "text/javascript",
                    "postMessage(isSecureContext);",
                ),
            ],
        )
        .await;
        let middle = StaticHttpServer::spawn_with_bodies(vec![format!(
            "<!doctype html><iframe src='{base_url}/child.html#{script_kind}'></iframe>"
        )])
        .await;
        let middle_url = middle.url_for_host("insecure.example.test", "/middle.html");
        let mut config = moli_fetch::FetchConfig::default();
        config.set_http_host_resolve(vec![middle.resolve_entry("insecure.example.test")]);
        config.set_http_no_proxy(Some("*".to_owned()));
        config.set_tls_credentials(tls.tls_config().ca_cert, None, None, None);
        let loader = ResourceRequestClient::new(&config).unwrap();
        let (mut vm, runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://secure.example.test/top.html",
                &loader,
            );
        vm.eval(&format!(
            r#"onmessage = ({{data}}) => globalThis.workerAncestorResult = data;
            const frame = document.createElement('iframe');
            frame.src = '{middle_url}';
            document.body.append(frame);"#
        ))
        .unwrap();
        assert_eq!(
            receive_worker_ancestor_probe(&mut vm, &runtime, &loader).await,
            serde_json::json!({
                "window": false, "dedicated": false, "dedicatedSubtle": false,
                "shared": false, "nested": false, "subtle": false
            }),
            "HTTPS top -> HTTP middle -> HTTPS child, {script_kind}"
        );
        assert_eq!(middle.finish_targets().await, vec!["/middle.html"]);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn popup_workers_use_the_popup_security_instead_of_the_opener() {
    for (opener_url, popup_is_secure) in [
        ("http://insecure.example.test/opener", true),
        ("https://secure.example.test/opener", false),
    ] {
        let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![
            (
                "/popup.html",
                "text/html",
                vec![],
                r#"<!doctype html><script>
                (async () => {
                    const dedicated = new Worker(new URL('/dedicated.js', location.href));
                    const dedicatedState = await new Promise(resolve => {
                        dedicated.onmessage = ({data}) => resolve(data);
                    });
                    dedicated.terminate();
                    const local = new Worker('data:text/javascript,' + encodeURIComponent(
                        "postMessage({secure: isSecureContext, subtle: typeof crypto.subtle !== 'undefined'})"
                    ));
                    const localState = await new Promise(resolve => {
                        local.onmessage = ({data}) => resolve(data);
                    });
                    local.terminate();
                    const shared = new SharedWorker(new URL('/shared.js', location.href));
                    const sharedState = await new Promise(resolve => {
                        shared.port.onmessage = ({data}) => resolve(data);
                        shared.port.start();
                    });
                    shared.port.close();
                    opener.postMessage(JSON.stringify({
                        dedicated: dedicatedState, local: localState, shared: sharedState
                    }), '*');
                })().catch(error => opener.postMessage(JSON.stringify({error: String(error)}), '*'));
                </script>"#,
            ),
            ("/dedicated.js", "text/javascript", vec![], DEDICATED_ANCESTOR_PROBE),
            (
                "/shared.js",
                "text/javascript",
                vec![],
                "onconnect = ({ports}) => ports[0].postMessage({secure: isSecureContext, subtle: typeof crypto.subtle !== 'undefined'});",
            ),
        ])
        .await;
        let mut popup_url = Url::parse(&format!("{base_url}/popup.html")).unwrap();
        let mut config = moli_fetch::FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        if !popup_is_secure {
            popup_url.set_host(Some("insecure.example.test")).unwrap();
            config.set_http_host_resolve(vec![format!(
                "insecure.example.test:{}:127.0.0.1",
                popup_url.port().unwrap()
            )]);
        }
        let loader = ResourceRequestClient::new(&config).unwrap();
        let (mut vm, runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                opener_url, &loader,
            );
        vm.eval(&format!(
            "onmessage = ({{data}}) => globalThis.workerAncestorResult = data; open('{popup_url}');"
        ))
        .unwrap();
        let state = serde_json::json!({"secure": popup_is_secure, "subtle": popup_is_secure});
        assert_eq!(
            receive_worker_ancestor_probe(&mut vm, &runtime, &loader).await,
            serde_json::json!({"dedicated": state, "local": state, "shared": state}),
            "{opener_url}, {popup_url}"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn shared_worker_isolation_does_not_inherit_document_capability() {
    for document_dip in [false, true] {
        for script_kind in ["classic", "module"] {
            for response_headers in [
                None,
                Some(vec![
                    ("Cross-Origin-Opener-Policy", "same-origin"),
                    ("Cross-Origin-Embedder-Policy", "require-corp"),
                ]),
                Some(vec![(
                    "Document-Isolation-Policy",
                    "isolate-and-require-corp",
                )]),
            ] {
                let (base_url, server) = if let Some(headers) = &response_headers {
                    let (url, server) = spawn_service_worker_response_server_with_headers(vec![(
                        "/app/worker.js",
                        "text/javascript",
                        headers.clone(),
                        WORKER_POLICY_SOURCE,
                    )])
                    .await;
                    (url, Some(server))
                } else {
                    ("https://shared-worker-isolation.test".to_owned(), None)
                };
                let page_url = Url::parse(&format!("{base_url}/app/page.html")).unwrap();
                let headers = if document_dip {
                    vec![(
                        "Document-Isolation-Policy".to_owned(),
                        b"isolate-and-require-corp".to_vec(),
                    )]
                } else {
                    vec![
                        (
                            "Cross-Origin-Opener-Policy".to_owned(),
                            b"same-origin".to_vec(),
                        ),
                        (
                            "Cross-Origin-Embedder-Policy".to_owned(),
                            b"require-corp".to_vec(),
                        ),
                    ]
                };
                let loader =
                    ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
                let (mut vm, runtime) =
                    new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                        page_url.as_str(),
                        &loader,
                    );
                vm.set_main_navigation_policy_container(
                    crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                        &headers, &page_url,
                    ),
                );
                assert_eq!(vm.eval("String(crossOriginIsolated)").unwrap(), "true");
                let worker_url = if response_headers.is_some() {
                    "'worker.js'".to_owned()
                } else {
                    format!(
                        "URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}))",
                        serde_json::to_string(WORKER_POLICY_SOURCE).unwrap()
                    )
                };
                vm.eval(&format!(
                    r#"
                    const workerURL = {worker_url};
                    const worker = new SharedWorker(workerURL, {{type: "{script_kind}"}});
                    worker.port.onmessage = ({{data}}) => {{
                        globalThis.workerIsolationResult = JSON.stringify([
                            data.secure, data.isolated
                        ]);
                        worker.port.close();
                        URL.revokeObjectURL(workerURL);
                    }};
                    worker.onerror = error => globalThis.workerIsolationResult = "error:" + error.message;
                    worker.port.start();
                    worker.port.postMessage("facts");
                    "#
                ))
                .unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    while vm
                        .eval("String(globalThis.workerIsolationResult !== undefined)")
                        .unwrap()
                        != "true"
                    {
                        runtime.drain_shared_worker_service_lane();
                        drain_service_worker_test_turn(&mut vm, &runtime, &loader).await;
                    }
                })
                .await
                .expect("SharedWorker isolation probe should reply");
                assert_eq!(
                    vm.eval("globalThis.workerIsolationResult").unwrap(),
                    r#"[true,false]"#,
                    "document DIP={document_dip}, {script_kind}, {response_headers:?}"
                );
                if let Some(server) = server {
                    server.await.unwrap();
                }
            }
        }
    }
}
