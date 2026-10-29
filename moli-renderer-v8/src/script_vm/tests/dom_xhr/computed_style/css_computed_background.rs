use super::*;

#[test]
fn css_computed_background_preserves_coordinated_layers() {
    let mut vm = new_parsed_test_vm("https://computed-background.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_computed_background.js")).unwrap(),
        "true"
    );
}
