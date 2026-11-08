use super::*;

#[tokio::test]
async fn rejected_synchronous_idb_writes_release_request_admission_and_complete() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-rejected-request.test/",
        &loader,
    );
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("idb_rejected_request.js")).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__idbRejectedRequest?.complete)",
        "true",
        "rejected synchronous IndexedDB write requests",
    )
    .await;
    assert_eq!(vm.eval("__idbRejectedRequest.total").unwrap(), "26");
    assert_eq!(
        vm.eval("JSON.stringify(__idbRejectedRequest.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
}
