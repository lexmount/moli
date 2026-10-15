use super::*;

#[tokio::test(flavor = "current_thread")]
async fn local_fetch_integrity_validates_blob_and_data_bytes_in_window_and_worker() {
    let fixture = include_str!("../../../tests/fixtures/local-fetch-integrity.js");
    for worker in [false, true] {
        let loader = static_http_loader([]);
        let mut vm = new_page_task_executor_test_vm_with_loader(
            "https://local-fetch-integrity.test/",
            &loader,
        );
        vm.eval("globalThis.localIntegrityResult = null;").unwrap();
        let script = if worker {
            let source = format!(
                "{fixture}\nlocalFetchIntegrityProbe().then(postMessage, error => postMessage({{error: String(error.stack || error)}}));"
            );
            format!(
                r#"
                const workerUrl = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
                const worker = new Worker(workerUrl);
                const finish = value => {{
                    localIntegrityResult = value;
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
                "{fixture}\nlocalFetchIntegrityProbe().then(value => {{ localIntegrityResult = value; }}, error => {{ localIntegrityResult = {{error: String(error.stack || error)}}; }});"
            )
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(localIntegrityResult !== null)",
            "true",
            "Local response integrity must settle without network transport",
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(localIntegrityResult)").unwrap())
                .unwrap();
        assert_eq!(result["state"], "pass", "worker={worker}: {result}");
        assert_eq!(result["checks"], 16, "worker={worker}: {result}");
    }
}
