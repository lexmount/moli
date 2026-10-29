use super::*;

#[tokio::test(flavor = "current_thread")]
async fn readable_stream_from_preserves_sync_protocol_across_realms_and_workers() {
    let loader = static_http_loader([]);
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://streams-sync-iterator.test/", &loader);
    vm.exec(
        "if (!document.documentElement) document.appendChild(document.createElement('html')); if (!document.body) document.documentElement.appendChild(document.createElement('body'));",
        None,
    ).unwrap();
    vm.exec(
        include_str!("../../../tests/fixtures/streams-sync-iterator.js"),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "typeof __uiEventResults",
        "object",
        "ReadableStream.from sync protocol Window/iframe/worker fixture",
    )
    .await;
    let facts: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__uiEventResults)").unwrap()).unwrap();
    assert_eq!(facts["readableStreamFromAvailable"], true, "{facts}");
    let checks = facts["checks"].as_array().unwrap();
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["passed"] != true)
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(checks.len(), 60, "{facts}");
    let reference = facts["nativeProtocol"].as_array().unwrap();
    assert_eq!(reference.len(), 2);
    assert!(
        reference.iter().all(|check| check["passed"] == true),
        "{facts}"
    );
}
