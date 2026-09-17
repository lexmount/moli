use super::*;

#[tokio::test(flavor = "current_thread")]
async fn blob_url_ranges_apply_to_fetch_and_xhr_in_window_and_worker() {
    let fixture = include_str!("../../../tests/fixtures/blob-range.js");
    for worker in [false, true] {
        let loader = static_http_loader([]);
        let mut vm =
            new_page_task_executor_test_vm_with_loader("https://blob-range.test/", &loader);
        vm.eval("globalThis.blobRangeResult = null;").unwrap();
        let script = if worker {
            let source = format!(
                "{fixture}\nblobRangeProbe().then(postMessage, error => postMessage({{error: String(error.stack || error)}}));"
            );
            format!(
                r#"
                const workerUrl = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
                const worker = new Worker(workerUrl);
                const finish = value => {{
                    blobRangeResult = value;
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
                "{fixture}\nblobRangeProbe().then(value => {{ blobRangeResult = value; }}, error => {{ blobRangeResult = {{error: String(error.stack || error)}}; }});"
            )
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(blobRangeResult !== null)",
            "true",
            "Blob URL range requests should settle locally",
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(blobRangeResult)").unwrap()).unwrap();
        let checks = result["checks"]
            .as_array()
            .unwrap_or_else(|| panic!("worker={worker}: {result}"));
        let failures: Vec<_> = checks
            .iter()
            .filter(|check| check["pass"] != true)
            .collect();
        assert_eq!(result["state"], "pass", "worker={worker}: {failures:?}");
        assert_eq!(checks.len(), 203, "worker={worker}");
    }
}
