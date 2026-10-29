use super::*;

#[test]
fn font_face_event_converts_interfaces_during_iteration_before_freezing_the_snapshot() {
    let mut vm = new_storage_html_test_vm("https://font-face-event-dictionary.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/font-face-event-dictionary.js"
        ))
        .unwrap(),
        "true"
    );
    // Keep the existing native getter's array identity while migrating input.
    assert_eq!(vm.eval("(() => {const event=new FontFaceSetLoadEvent('audit'); return event.fontfaces===event.fontfaces;})()").unwrap(), "true");
}
