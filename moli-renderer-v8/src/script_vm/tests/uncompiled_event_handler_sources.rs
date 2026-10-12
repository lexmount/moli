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

#[test]
fn uncompiled_event_handlers_keep_original_locations_across_writes_cloning_and_adoption() {
    for url in [
        "http://localhost/",
        "https://localhost/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_html_test_vm(url);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/uncompiled-event-handler-locations.js"
        ))
        .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]",
            "{url}",
        );
        assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "201", "{url}");
    }
}

#[test]
fn prebuilt_parser_event_handler_errors_keep_original_url_and_utf16_coordinates() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for prefix in ["", "😀é", "one\rtwo\r\n三\n", "&amp;&#10;&#x1f600;&notit;"] {
        let before = format!("<!doctype html><body>{prefix}<button id=bad onclick='let = ;'>");
        let normalized = before.replace("\r\n", "\n").replace('\r', "\n");
        let line = normalized.bytes().filter(|&c| c == b'\n').count() + 1;
        let column = normalized
            .rsplit('\n')
            .next()
            .unwrap()
            .encode_utf16()
            .count()
            + 7;
        let original = "https://parser-handler-location.test/original.html?query#fragment";
        let mut vm = new_parsed_page_task_executor_test_vm(
            original,
            &format!("{before}</button><iframe></iframe>"),
            &loader,
        );
        let result = vm
            .eval(
                r#"
          const target = document.getElementById('bad');
          const child = document.querySelector('iframe').contentWindow;
          const reports = [];
          child.addEventListener('error', event => {
            reports.push({source: event.filename, line: event.lineno, column: event.colno,
              childRealm: event.error instanceof child.SyntaxError});
            event.preventDefault();
          });
          history.replaceState({}, '', '/changed.html');
          child.document.adoptNode(target);
          void target.onclick;
          JSON.stringify(reports)
        "#,
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!([{"source": original, "line": line,
                "column": column, "childRealm": true}]),
            "prefix={prefix:?}"
        );
    }
}
