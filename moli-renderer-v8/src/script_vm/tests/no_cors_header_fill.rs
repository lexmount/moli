use super::*;

#[tokio::test(flavor = "current_thread")]
async fn no_cors_header_fill_uses_append_guards_in_window_and_worker() {
    let fixture = include_str!("../../../tests/fixtures/no-cors-header-fill.js");
    for worker in [false, true] {
        let server = StaticHttpServer::spawn_echo(64).await;
        let base = server.base_url();
        let origin = serde_json::to_string(base.as_str().trim_end_matches('/')).unwrap();
        let loader = static_http_loader([]);
        let mut vm = new_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
        vm.eval("globalThis.headerFillResult = null;").unwrap();
        let script = if worker {
            let source = format!(
                "{fixture}\nnoCorsFillProbe({origin}).then(postMessage, error => postMessage({{error: String(error.stack || error)}}));"
            );
            format!(
                r#"
                const workerUrl = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
                const worker = new Worker(workerUrl);
                const finish = value => {{
                    headerFillResult = value;
                    worker.terminate();
                    URL.revokeObjectURL(workerUrl);
                }};
                worker.onmessage = event => finish(event.data);
                worker.onerror = event => {{ finish({{error: event.message}}); event.preventDefault(); }};
                "#,
                serde_json::to_string(&source).unwrap()
            )
        } else {
            format!(
                "{fixture}\nnoCorsFillProbe({origin}).then(value => {{ headerFillResult = value; }}, error => {{ headerFillResult = {{error: String(error.stack || error)}}; }});"
            )
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(headerFillResult !== null)",
            "true",
            "Request header filling should finish",
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(headerFillResult)").unwrap()).unwrap();
        let checks = result["checks"]
            .as_array()
            .unwrap_or_else(|| panic!("worker={worker}: {result}"));
        let failures: Vec<_> = checks
            .iter()
            .filter(|check| check["pass"] != true)
            .collect();
        assert_eq!(result["state"], "pass", "worker={worker}: {failures:?}");
        assert_eq!(checks.len(), 320, "worker={worker}");
        assert_eq!(server.finish().await.len(), 64, "worker={worker}");
    }
}
