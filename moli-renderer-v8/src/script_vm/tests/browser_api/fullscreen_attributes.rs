use super::*;

#[test]
fn fullscreen_attributes_validate_native_receivers_and_retain_callee_realms() {
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-attributes.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(include_str!(
        "../../../../tests/fixtures/fullscreen-native-attributes.js"
    ))
    .expect("fullscreen attribute matrix should evaluate");
    assert_eq!(
        vm.eval("__fullscreenAttributeResults.complete").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("__fullscreenAttributeResults.total").unwrap(),
        "881"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__fullscreenAttributeResults.rows.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
}
