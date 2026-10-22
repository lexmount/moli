use super::*;

#[test]
fn bulk_child_removal_preserves_option_selectedness_and_one_observer_record() {
    let mut vm = new_parsed_test_vm(
        "https://bulk-child-removal.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/bulk-child-removal.js"
        ))
        .expect("bulk child removal matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1794");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn bulk_child_removal_preserves_ranges_and_custom_element_reaction_order() {
    let mut vm = new_parsed_test_vm(
        "https://bulk-child-removal-boundaries.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/bulk-child-removal-boundaries.js"
        ))
        .expect("bulk child removal boundary matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "32");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}

#[test]
fn bulk_child_removal_keeps_prior_independent_observer_records() {
    let mut vm = new_parsed_test_vm(
        "https://bulk-child-removal-record-history.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(include_str!(
            "../../../../tests/fixtures/bulk-child-removal-record-history.js"
        ))
        .expect("bulk child removal record history matrix should complete");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "32");
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .unwrap(),
    );
}
