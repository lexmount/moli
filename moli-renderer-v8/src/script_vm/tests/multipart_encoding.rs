use super::*;

#[tokio::test(flavor = "current_thread")]
async fn multipart_encoding_normalizes_wire_values_and_preserves_source_form_data() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://multipart-encoding.test/", &loader);
    vm.exec("document.body.innerHTML='<iframe id=child></iframe>'", None)
        .unwrap();
    vm.exec(
        include_str!("../../../tests/fixtures/multipart-line-breaks.js"),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "Multipart encoding in Window, child realms and a real Worker",
    )
    .await;
    let facts: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(globalThis.__uiEventResults)")
            .unwrap(),
    )
    .unwrap();
    let checks = facts["checks"].as_array().unwrap();
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["passed"] != true)
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(facts["total"], 400);
    assert_eq!(facts["passed"], 400);
    assert_eq!(checks.len(), 400);
    assert_eq!(
        checks
            .iter()
            .map(|check| check["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        400
    );
}
