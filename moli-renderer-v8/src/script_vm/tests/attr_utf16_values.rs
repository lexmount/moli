use super::*;

#[test]
fn attr_values_and_clones_preserve_utf16_in_live_foreign_and_windowless_documents() {
    let mut vm = new_storage_html_test_vm("https://attr-utf16-values.test/");
    let result = vm.eval(include_str!("attr_utf16_values.js")).unwrap();
    assert_eq!(result, "[]");
}
