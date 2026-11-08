use super::*;

#[test]
fn notification_constructor_converts_complete_options_before_materialization() {
    let mut vm = new_storage_html_test_vm("https://notification-options.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/notification-options-dictionary.js"
        ))
        .unwrap(),
        "true"
    );
}
