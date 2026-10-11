use super::*;

#[test]
fn uncompiled_event_handlers_preserve_utf16_and_window_sources_after_adoption() {
    for url in [
        "http://localhost/",
        "https://localhost/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_html_test_vm(url);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/uncompiled-event-handler-sources.js"
        ))
        .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "147", "{url}");
    }
}
