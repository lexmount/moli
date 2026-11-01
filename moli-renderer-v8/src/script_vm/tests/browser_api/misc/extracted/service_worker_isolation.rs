use super::*;

const WORKER_POLICY_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/service-worker-isolation.js"
));

#[tokio::test]
async fn service_worker_isolation_uses_own_coep() {
    for script_kind in ["classic", "module"] {
        for (headers, isolated) in [
            (vec![("Cross-Origin-Embedder-Policy", "require-corp")], true),
            (
                vec![("Cross-Origin-Embedder-Policy", "credentialless")],
                true,
            ),
            (
                vec![
                    ("Cross-Origin-Embedder-Policy", "require-corp"),
                    ("Permissions-Policy", "cross-origin-isolated=()"),
                ],
                true,
            ),
            (vec![], false),
            (vec![("Cross-Origin-Opener-Policy", "same-origin")], false),
            (
                vec![("Document-Isolation-Policy", "isolate-and-require-corp")],
                false,
            ),
        ] {
            let (base_url, server) = spawn_service_worker_response_server_with_headers(vec![(
                "/app/worker.js",
                "text/javascript",
                headers.clone(),
                WORKER_POLICY_SOURCE,
            )])
            .await;
            let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
            let (mut vm, runtime) =
                new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                    &format!("{base_url}/app/page.html"),
                    &loader,
                );
            assert_eq!(vm.eval("String(crossOriginIsolated)").unwrap(), "false");
            vm.eval(&format!(
                r#"
                (async () => {{
                    const registration = await navigator.serviceWorker.register("worker.js", {{
                        scope: "./", type: "{script_kind}"
                    }});
                    await navigator.serviceWorker.ready;
                    const channel = new MessageChannel();
                    const reply = new Promise(resolve => channel.port1.onmessage = resolve);
                    registration.active.postMessage("facts", [channel.port2]);
                    const {{data}} = await reply;
                    channel.port1.close();
                    globalThis.workerIsolationResult = JSON.stringify([
                        data.secure, data.isolated
                    ]);
                }})().catch(error => globalThis.workerIsolationResult = "error:" + error);
                "#
            ))
            .unwrap();
            drain_service_worker_test_until_eval_equals(
                &mut vm,
                &runtime,
                &loader,
                "String(globalThis.workerIsolationResult !== undefined)",
                "true",
            )
            .await;
            let expected = if isolated {
                r#"[true,true]"#
            } else {
                r#"[true,false]"#
            };
            assert_eq!(
                vm.eval("globalThis.workerIsolationResult").unwrap(),
                expected,
                "{script_kind}, {headers:?}"
            );
            server.await.unwrap();
        }
    }
}
