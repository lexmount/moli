use super::*;

#[tokio::test(flavor = "current_thread")]
async fn body_mime_extraction_in_window() {
    run_body_mime_probe(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn body_mime_extraction_in_worker() {
    run_body_mime_probe(true).await;
}

async fn run_body_mime_probe(worker: bool) {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader("https://body-mime.test/", &loader);
    let fixture = include_str!("../../../tests/fixtures/body-mime.js")
        .trim()
        .trim_end_matches(';');
    vm.eval("globalThis.bodyMimeResult = null;").unwrap();
    let script = if worker {
        let source = format!(
            "({fixture}).then(postMessage, error => postMessage(JSON.stringify({{error:String(error)}})));"
        );
        format!(
            r#"
            const workerURL = URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}));
            const worker = new Worker(workerURL);
            const finish = value => {{
                bodyMimeResult = value;
                worker.terminate();
                URL.revokeObjectURL(workerURL);
            }};
            worker.onmessage = event => finish(event.data);
            worker.onerror = event => {{
                finish(JSON.stringify({{error:event.message}}));
                event.preventDefault();
            }};
            "#,
            serde_json::to_string(&source).unwrap(),
        )
    } else {
        format!(
            "({fixture}).then(value => bodyMimeResult = value, error => bodyMimeResult = JSON.stringify({{error:String(error)}}));"
        )
    };
    vm.eval(&script).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(bodyMimeResult !== null)",
        "true",
        "Body MIME probe",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("bodyMimeResult").unwrap()).unwrap();
    assert_eq!(result["total"], 119, "worker={worker}: {result}");
    assert_eq!(result["failures"], serde_json::json!([]), "worker={worker}");
}
