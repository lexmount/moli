use super::*;

#[test]
fn page_reveal_event_converts_typed_dictionary_and_guards_its_getter() {
    let mut vm = new_storage_html_test_vm("https://page-reveal-event.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/page-reveal-event.js"
        ))
        .unwrap(),
        "true"
    );
}
