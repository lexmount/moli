use super::*;

#[tokio::test(flavor = "current_thread")]
async fn headers_preserve_internal_lists_in_window_and_worker() {
    let fixture = include_str!("../../../tests/fixtures/headers-list.js");
    for worker in [false, true] {
        let loader = static_http_loader([]);
        let mut vm =
            new_page_task_executor_test_vm_with_loader("https://headers-list.test/", &loader);
        vm.eval("globalThis.headerListResult = null;").unwrap();
        let script = if worker {
            let source = format!(
                "{fixture}\nheadersListProbe('https://headers-list.test').then(postMessage, error => postMessage({{error: String(error.stack || error)}}));"
            );
            format!(
                r#"
                const workerUrl = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
                const worker = new Worker(workerUrl);
                const finish = value => {{
                    headerListResult = value;
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
                "{fixture}\nheadersListProbe('https://headers-list.test').then(value => {{ headerListResult = value; }}, error => {{ headerListResult = {{error: String(error.stack || error)}}; }});"
            )
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(headerListResult !== null)",
            "true",
            "Headers list checks should finish",
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(headerListResult)").unwrap()).unwrap();
        let checks = result["checks"]
            .as_array()
            .unwrap_or_else(|| panic!("worker={worker}: {result}"));
        let failures: Vec<_> = checks
            .iter()
            .filter(|check| check["pass"] != true)
            .collect();
        assert_eq!(result["state"], "pass", "worker={worker}: {failures:?}");
        assert_eq!(checks.len(), 174, "worker={worker}");
    }
}
