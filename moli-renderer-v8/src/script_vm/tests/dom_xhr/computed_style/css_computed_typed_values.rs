use super::*;

#[test]
fn css_computed_typed_values_preserve_math_types_and_native_realms() {
    let mut vm = new_parsed_test_vm(
        "https://computed-typed-values.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_computed_typed_values.js"))
            .unwrap(),
        "true"
    );
}
