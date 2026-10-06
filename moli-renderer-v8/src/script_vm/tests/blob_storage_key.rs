use super::*;

#[tokio::test(flavor = "current_thread")]
async fn blob_url_fetch_and_xhr_use_creator_storage_keys_in_window_and_worker() {
    let loader = static_http_loader([]);
    let mut vm = new_page_task_executor_test_vm_with_loader("https://blob-storage.test/", &loader);
    let fixture = include_str!("../../../tests/fixtures/blob-storage-key.js");
    vm.eval(&format!(
        "globalThis.blobStorageResult = null; {fixture}.then(value => {{blobStorageResult = value;}}, error => {{blobStorageResult = String(error.stack || error);}});"
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(blobStorageResult !== null)",
        "true",
        "Blob storage-key probes should finish without network access",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__uiEventResults)").unwrap()).unwrap();
    let checks = result["checks"].as_array().unwrap();
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["passed"] != true)
        .collect();
    assert_eq!(checks.len(), 46);
    assert_eq!(result["complete"], true);
    assert_eq!(
        vm.eval("String(blobStorageResult)").unwrap(),
        "true",
        "{failures:?}"
    );
    assert!(failures.is_empty(), "{failures:?}");
}
