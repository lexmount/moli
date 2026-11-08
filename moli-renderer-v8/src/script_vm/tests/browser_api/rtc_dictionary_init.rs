use super::*;

#[test]
fn rtc_constructor_dictionaries_convert_once_and_preserve_v8_strings() {
    let mut vm = new_storage_html_test_vm("https://rtc-constructor-dictionary.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/rtc-constructor-dictionaries.js"
        ))
        .unwrap(),
        "true"
    );
}
