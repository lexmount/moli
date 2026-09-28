use super::*;

#[test]
fn css_numeric_factories_create_native_values_in_the_callee_realm() {
    let mut vm = new_parsed_test_vm(
        "https://css-numeric-factories.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_numeric_factories.js")).unwrap(),
        "true"
    );
}
