use super::*;

#[test]
fn computed_style_arguments_use_native_element_brands_and_preserve_exceptions() {
    let mut vm = new_storage_page_task_executor_test_vm("https://computed-style-arguments.test/");
    vm.eval(include_str!("cssom_arguments.js"))
        .expect("computed-style argument conversions should evaluate");
    assert_eq!(
        vm.eval("__computedStyleArgumentsResults.complete").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify(__computedStyleArgumentsResults.checks.filter(row => !row.passed))"
        )
        .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__computedStyleArgumentsResults.total === 318 && __computedStyleArgumentsResults.passed === 318")
            .unwrap(),
        "true"
    );
}

#[test]
fn detached_iframe_computed_style_uses_the_shared_argument_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://detached-style-arguments.test/");
    vm.eval(
        r#"
const detachedDocument = document.implementation.createHTMLDocument('');
const frame = detachedDocument.createElement('iframe');
detachedDocument.body.appendChild(frame);
const child = frame.contentWindow;
const target = child.document.createElement('select');
child.document.body.appendChild(target);
globalThis.__detachedStyleArgumentChecks = [];
const check = value => __detachedStyleArgumentChecks.push(value);
check(child.getComputedStyle(target) instanceof CSSStyleDeclaration);
for (const value of [undefined, null, {}, child.document,
                    child.document.createTextNode('x'), Object.create(target),
                    new Proxy(target, {}), Object.create(Element.prototype)]) {
  let conversions = 0;
  let caught;
  try {
    child.getComputedStyle(value, {toString() { conversions++; return ''; }});
  } catch (error) { caught = error; }
  check(caught instanceof TypeError);
  check(conversions === 0);
}
const sentinel = {};
let caught;
try { child.getComputedStyle(target, {toString() { throw sentinel; }}); }
catch (error) { caught = error; }
check(caught === sentinel);
check(child.getComputedStyle(target, null) instanceof CSSStyleDeclaration);
"#,
    )
    .expect("synthetic iframe should share argument conversion");
    assert_eq!(
        vm.eval("__detachedStyleArgumentChecks.length === 19 && __detachedStyleArgumentChecks.every(Boolean)")
            .unwrap(),
        "true"
    );
}
