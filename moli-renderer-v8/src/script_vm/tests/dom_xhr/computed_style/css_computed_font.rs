use super::*;

#[test]
fn css_computed_font_serializes_resolved_longhands_and_retains_values() {
    let mut vm = new_parsed_test_vm("https://computed-font.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_computed_font.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_computed_font_size_adjust_uses_native_reset_and_cascade() {
    let mut vm = new_parsed_test_vm("https://font-size-adjust.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_computed_font_size_adjust.js"))
            .unwrap(),
        "true"
    );
}
