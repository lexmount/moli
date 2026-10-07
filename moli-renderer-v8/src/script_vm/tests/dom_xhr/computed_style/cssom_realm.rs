use super::*;

#[test]
fn computed_style_uses_receiver_realm_and_target_document() {
    let mut vm = new_storage_page_task_executor_test_vm("https://computed-style-realm.test/");
    vm.eval(include_str!("cssom_realm.js"))
        .expect("computed-style realm matrix should evaluate");
    assert_eq!(
        vm.eval("__computedStyleRealmResults.complete").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__computedStyleRealmResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__computedStyleRealmResults.total === 2316 && __computedStyleRealmResults.passed === 2316").unwrap(),
        "true"
    );
}

#[test]
fn computed_style_preserves_discarded_window_receiver_realm() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://computed-style-discarded-realm.test/");
    assert_eq!(vm.eval(r#"(() => {
const mainMethod = window.getComputedStyle;
for (const removeDuringConversion of [false, true]) {
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const receiver = frame.contentWindow;
  const childMethod = receiver.getComputedStyle;
  const prototype = (receiver.CSSStyleProperties ?? receiver.CSSStyleDeclaration).prototype;
  if (!removeDuringConversion) frame.remove();
  for (const method of [mainMethod, childMethod]) {
    let conversions = 0;
    const result = Reflect.apply(method, receiver, [document.body, {toString() {
      conversions++;
      if (removeDuringConversion) frame.remove();
      return '';
    }}]);
    if (conversions !== 1 || Object.getPrototypeOf(result) !== prototype || result.color !== 'rgb(0, 0, 0)') return false;
  }
  frame.remove();
}
return true;
})()"#).unwrap(), "true");
}
