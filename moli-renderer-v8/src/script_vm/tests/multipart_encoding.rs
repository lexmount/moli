use super::*;

#[tokio::test(flavor = "current_thread")]
async fn multipart_encoding_normalizes_wire_values_and_preserves_source_form_data() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://multipart-encoding.test/", &loader);
    vm.exec("document.body.innerHTML='<iframe id=child></iframe>'", None)
        .unwrap();
    vm.exec(
        include_str!("../../../tests/fixtures/multipart-line-breaks.js"),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "Multipart encoding in Window, child realms and a real Worker",
    )
    .await;
    let facts: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(globalThis.__uiEventResults)")
            .unwrap(),
    )
    .unwrap();
    let checks = facts["checks"].as_array().unwrap();
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["passed"] != true)
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(facts["total"], 400);
    assert_eq!(facts["passed"], 400);
    assert_eq!(checks.len(), 400);
    assert_eq!(
        checks
            .iter()
            .map(|check| check["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        400
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multipart_form_submission_uses_accept_charset_on_the_native_navigation_request() {
    let loader = static_http_loader([]);
    for (label, value, expected, canonical) in [
        (
            "windows-1252",
            "café €",
            &b"caf\xe9 \x80"[..],
            "windows-1252",
        ),
        ("gbk", "家居", &b"\xbc\xd2\xbe\xd3"[..], "GBK"),
    ] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://multipart-charset.test/page",
            "<!doctype html><body></body>",
            &loader,
        );
        vm.eval(&format!(
            r#"(() => {{
          const frame = document.createElement('iframe'); frame.name = 'target';
          document.body.appendChild(frame);
          const form = document.createElement('form');
          form.method = 'post'; form.action = '/submit'; form.target = 'target';
          form.enctype = 'multipart/form-data'; form.acceptCharset = {};
          const charset = document.createElement('input');
          charset.type = 'hidden'; charset.name = '_charset_'; form.appendChild(charset);
          const input = document.createElement('input');
          input.name = 'field'; input.value = {}; form.appendChild(input);
          document.body.appendChild(form); form.submit();
        }})()"#,
            serde_json::to_string(label).unwrap(),
            serde_json::to_string(value).unwrap()
        ))
        .unwrap();
        assert!(
            vm.run_one_dom_manipulation_body_for_test(
                crate::runtime::PageDomManipulationTestFamily::FormNavigation,
            )
            .await
            .unwrap()
        );
        let handle = vm
            ._context_host
            .borrow()
            .child_browsing_context_handle_by_index(0)
            .unwrap();
        let navigation = vm
            ._context_host
            .borrow()
            .child_browsing_context_pending_live_navigation_for_test(handle)
            .unwrap();
        let crate::native_bridge::ChildBrowsingContextBootstrap::Request(request) = navigation
        else {
            panic!("multipart submission must queue a request navigation");
        };
        let body = request.body.as_deref().expect("multipart request body");
        let expected_value = [b"name=\"field\"\r\n\r\n".as_slice(), expected, b"\r\n"].concat();
        assert!(
            body.windows(expected_value.len())
                .any(|part| part == expected_value),
            "{label}: {body:?}"
        );
        let expected_charset = format!("name=\"_charset_\"\r\n\r\n{canonical}\r\n");
        assert!(
            body.windows(expected_charset.len())
                .any(|part| part == expected_charset.as_bytes()),
            "{label}: {body:?}"
        );
        assert_eq!(
            vm.eval("new FormData(document.querySelector('form')).get('field')")
                .unwrap(),
            value
        );
    }
}
