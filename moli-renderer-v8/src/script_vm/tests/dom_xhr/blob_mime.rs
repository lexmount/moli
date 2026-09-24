use super::*;

#[tokio::test(flavor = "current_thread")]
async fn xhr_blob_response_mime_preserves_parameters_in_window_and_worker() {
    let fixture = include_str!("../../../../tests/fixtures/xhr-blob-mime.js");
    for worker in [false, true] {
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm =
            new_page_task_executor_test_vm_with_loader("https://xhr-blob-mime.test/", &loader);
        vm.eval("globalThis.blobMimeResult = null;").unwrap();
        let script = if worker {
            let source = format!(
                "{fixture}\nxhrBlobMimeProbe().then(postMessage, error => postMessage({{error: String(error.stack || error)}}));"
            );
            format!(
                "const worker = new Worker('data:text/javascript,' + encodeURIComponent({})); worker.onmessage = event => {{ blobMimeResult = event.data; worker.terminate(); }}; worker.onerror = event => {{ blobMimeResult = {{error:event.message}}; event.preventDefault(); }};",
                serde_json::to_string(&source).unwrap()
            )
        } else {
            format!(
                "{fixture}\nxhrBlobMimeProbe().then(value => {{ blobMimeResult = value; }}, error => {{ blobMimeResult = {{error:String(error.stack || error)}}; }});"
            )
        };
        vm.eval(&script).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while vm.eval("blobMimeResult === null").unwrap() == "true" {
                wait_for_one_selected_page_task_executor_test_turn(&mut vm)
                    .await
                    .unwrap();
            }
        })
        .await
        .expect("XHR Blob MIME probe should settle locally");
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(blobMimeResult)").unwrap()).unwrap();
        assert_eq!(result["state"], "pass", "worker={worker}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if worker { 120 } else { 60 }
        );
    }
}
