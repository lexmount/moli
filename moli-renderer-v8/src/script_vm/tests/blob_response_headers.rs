use super::*;

#[tokio::test(flavor = "current_thread")]
async fn blob_url_response_headers_match_bytes_and_type_in_window_and_worker() {
    let fixture = include_str!("../../../tests/fixtures/blob-response-headers.js");
    for worker in [false, true] {
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm = new_page_task_executor_test_vm_with_loader(
            "https://blob-response-headers.test/",
            &loader,
        );
        vm.eval("globalThis.blobHeadersResult = null;").unwrap();
        let script = if worker {
            let source = format!(
                "{fixture}\nblobResponseHeaderProbe().then(postMessage, error => postMessage({{error: String(error.stack || error)}}));"
            );
            format!(
                r#"
                const workerUrl = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
                const worker = new Worker(workerUrl);
                const finish = value => {{
                    blobHeadersResult = value;
                    worker.terminate();
                    URL.revokeObjectURL(workerUrl);
                }};
                worker.onmessage = event => finish(event.data);
                worker.onerror = event => {{
                    finish({{error: event.message}});
                    event.preventDefault();
                }};
                "#,
                serde_json::to_string(&source).unwrap()
            )
        } else {
            format!(
                "{fixture}\nblobResponseHeaderProbe().then(value => {{ blobHeadersResult = value; }}, error => {{ blobHeadersResult = {{error:String(error.stack || error)}}; }});"
            )
        };
        vm.eval(&script).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while vm.eval("blobHeadersResult === null").unwrap() == "true" {
                wait_for_one_selected_page_task_executor_test_turn(&mut vm)
                    .await
                    .unwrap();
            }
        })
        .await
        .expect("Blob URL responses should settle locally");
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(blobHeadersResult)").unwrap()).unwrap();
        let checks = result["checks"]
            .as_array()
            .unwrap_or_else(|| panic!("worker={worker}: {result}"));
        let failures: Vec<_> = checks
            .iter()
            .filter(|check| check["pass"] != true)
            .collect();
        assert_eq!(result["state"], "pass", "worker={worker}: {failures:?}");
        assert_eq!(checks.len(), 490, "worker={worker}");
    }
}
