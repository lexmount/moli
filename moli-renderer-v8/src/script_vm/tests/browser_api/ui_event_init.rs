use super::*;

#[test]
fn ui_and_composition_events_convert_inherited_dictionaries_and_legacy_arguments() {
    let mut vm = new_storage_html_test_vm("https://ui-event-init-webidl.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>';")
        .unwrap();
    let result = vm.eval(include_str!("ui_event_init.js")).unwrap();
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.rows.filter(row=>!row.passed))")
            .unwrap()
    );
}
