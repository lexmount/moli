use super::*;

const WORKER_POLICY_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/shared-worker-isolation.js"
));

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
