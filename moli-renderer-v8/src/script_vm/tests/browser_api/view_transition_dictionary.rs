use super::*;

#[test]
fn view_transition_dictionary_conversion_precedes_active_transition_mutation() {
    let mut vm = new_storage_html_test_vm("https://view-transition-dictionary.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/view-transition-options.js"
        ))
        .unwrap(),
        "true"
    );
}
