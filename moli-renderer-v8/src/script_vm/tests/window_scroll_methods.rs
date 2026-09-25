use super::*;

#[test]
fn window_scroll_methods_preserve_receivers_overloads_and_conversion_effects() {
    let mut vm = new_storage_test_vm("https://window-scroll.test/");
    vm.exec(
        r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
"#,
        None,
    )
    .unwrap();
    vm.exec(
        include_str!("../../../tests/fixtures/window-scroll-methods.js"),
        None,
    )
    .unwrap();
    vm.drain_pending_child_frame_work_for_test();
    vm.exec(
        r#"
globalThis.__scrollResult = null;
windowScrollCase.run().then(
  result => { __scrollResult = result; },
  error => { __scrollResult = {error: String(error), stack: error.stack}; }
).finally(() => windowScrollCase.close());
"#,
        None,
    )
    .unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__scrollResult)").unwrap()).unwrap();
    assert_eq!(result["checks"], 489, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
