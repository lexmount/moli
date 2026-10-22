use super::*;

#[test]
fn input_patterns_use_bounded_native_regexps_without_changing_author_state() {
    let mut vm = new_parsed_test_vm(
        "https://input-pattern-matching.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/input-pattern-matching.js"
        ))
        .expect("input pattern matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3824");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}
