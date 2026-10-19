use super::*;

#[test]
fn css_computed_background_preserves_coordinated_layers() {
    let mut vm = new_parsed_test_vm("https://computed-background.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_computed_background.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_computed_shorthands_retain_property_identity_and_native_realms() {
    let mut vm = new_parsed_test_vm(
        "https://computed-shorthand-values.test/",
        "<!doctype html><body><iframe></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_computed_shorthands.js")).unwrap(),
        "true"
    );
}
