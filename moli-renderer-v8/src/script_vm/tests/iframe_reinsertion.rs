use super::*;

#[test]
fn iframe_reinsertion_recreates_contexts_but_atomic_moves_preserve_them() {
    let mut vm = new_storage_test_vm("https://iframe-reinsertion.test/");
    vm.exec(
        r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
"#,
        None,
    )
    .unwrap();
    vm.exec(
        include_str!("../../../tests/fixtures/iframe-reinsertion.js"),
        None,
    )
    .unwrap();
    vm.exec("iframeReinsertionCase.setup()", None).unwrap();
    vm.drain_pending_child_frame_work_for_test();
    vm.exec("iframeReinsertionCase.move()", None).unwrap();
    vm.drain_pending_child_frame_work_for_test();
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(iframeReinsertionCase.result())")
            .unwrap(),
    )
    .unwrap();
    vm.exec("iframeReinsertionCase.close()", None).unwrap();
    assert_eq!(result["cases"], 13, "{result}");
    assert_eq!(result["checks"], 221, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
