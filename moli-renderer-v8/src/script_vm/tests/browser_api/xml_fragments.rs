use super::*;

#[test]
fn xml_fragments_preserve_native_context_and_reject_errors_before_mutation() {
    let mut vm = new_parsed_test_vm(
        "https://xml-fragments.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!("../../../../tests/fixtures/xml-fragments.js"))
        .expect("XML fragment matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "2970");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn xml_fragments_support_shadow_roots_templates_and_native_trusted_html() {
    let mut vm = new_parsed_test_vm(
        "https://xml-fragment-special-contexts.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/xml-fragment-special-contexts.js"
        ))
        .expect("XML fragment special context matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "124");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}
