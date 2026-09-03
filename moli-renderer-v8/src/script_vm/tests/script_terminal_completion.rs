use super::*;
use crate::parser::{PreparedImportMap, PreparedImportMapSource};

#[test]
fn window_error_body_leaves_checkpoint_to_its_carrier() {
    let mut vm = new_storage_test_vm("https://script-terminal.test/body.html");
    vm.eval(
        r#"
globalThis.__windowErrorBodyOrder = [];
addEventListener("error", () => {
  __windowErrorBodyOrder.push("error");
  queueMicrotask(() => __windowErrorBodyOrder.push("microtask"));
});
"#,
    )
    .expect("Window error body listener should install");

    vm.report_window_error_body(
        "body-only Window error",
        Some("https://script-terminal.test/body.js"),
        None,
    )
    .expect("Window error body should dispatch");
    assert_eq!(
        vm.eval_without_microtask_checkpoint_for_test("__windowErrorBodyOrder.join('|')")
            .expect("body-only order should be readable"),
        "error",
        "the event body must not claim its carrier's checkpoint"
    );

    vm.perform_script_task_checkpoint(None)
        .expect("test carrier checkpoint should run");
    assert_eq!(
        vm.eval_without_microtask_checkpoint_for_test("__windowErrorBodyOrder.join('|')")
            .expect("completed order should be readable"),
        "error|microtask"
    );
}

#[test]
fn parser_import_map_failure_finishes_its_algorithm_checkpoint() {
    let mut vm = new_storage_test_vm("https://script-terminal.test/import-map.html");
    vm.eval(
        r#"
globalThis.__importMapFailureOrder = [];
addEventListener("error", () => {
  __importMapFailureOrder.push("error");
  queueMicrotask(() => __importMapFailureOrder.push("microtask"));
});
"#,
    )
    .expect("import-map error listener should install");

    let document = vm.document_runtime.document_handle();
    let html = vm.document_runtime.dom_host_mut().create_element("html");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(document, html),
        "parser import-map document element should attach"
    );
    let script = vm
        .document_runtime
        .dom_host_mut()
        .create_parser_element_without_attributes(
            "script".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .append_child(html, script),
        "parser import-map node should attach"
    );
    let document_url = Url::parse("https://script-terminal.test/import-map.html").unwrap();
    crate::module_runtime::accept_parser_owned_import_map_handoff(
        &mut vm,
        script,
        1,
        1,
        PreparedImportMap {
            position: 0,
            node_id: script,
            source: PreparedImportMapSource::Inline("{ invalid json".to_owned()),
            nonce: None,
            base_url: document_url.clone(),
            initiator_url: document_url,
        },
    );

    assert_eq!(
        vm.eval_without_microtask_checkpoint_for_test("__importMapFailureOrder.join('|')")
            .expect("import-map completion order should be readable"),
        "error|microtask",
        "the synchronous parser import-map failure algorithm must finish its error reaction before returning"
    );
}

#[test]
fn parser_import_map_registration_obeys_inline_script_csp() {
    fn append_parser_script(vm: &mut ScriptVm) -> DomHandle {
        let document = vm.document_runtime.document_handle();
        let html = vm.document_runtime.dom_host_mut().create_element("html");
        assert!(
            vm.document_runtime
                .dom_host_mut()
                .append_child(document, html)
        );
        let script = vm
            .document_runtime
            .dom_host_mut()
            .create_parser_element_without_attributes(
                "script".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
        assert!(
            vm.document_runtime
                .dom_host_mut()
                .append_child(html, script)
        );
        script
    }

    fn accept_import_map(vm: &mut ScriptVm, nonce: Option<&str>) {
        let script = append_parser_script(vm);
        let document_url = Url::parse("https://script-terminal.test/import-map.html").unwrap();
        crate::module_runtime::accept_parser_owned_import_map_handoff(
            vm,
            script,
            1,
            1,
            PreparedImportMap {
                position: 0,
                node_id: script,
                source: PreparedImportMapSource::Inline(
                    r#"{"imports":{"fixture":"/mapped.mjs"}}"#.to_owned(),
                ),
                nonce: nonce.map(str::to_owned),
                base_url: document_url.clone(),
                initiator_url: document_url,
            },
        );
    }

    let base_url = Url::parse("https://script-terminal.test/import-map.html").unwrap();

    let mut blocked = new_storage_test_vm(base_url.as_str());
    blocked.set_response_content_security_policies(&["script-src 'nonce-allowed'".to_owned()]);
    accept_import_map(&mut blocked, None);
    assert!(
        crate::module_runtime::resolve_module_specifier(&mut blocked, "fixture", &base_url)
            .is_err(),
        "a parser import map without the required nonce must not register"
    );

    let mut allowed = new_storage_test_vm(base_url.as_str());
    allowed.set_response_content_security_policies(&["script-src 'nonce-allowed'".to_owned()]);
    accept_import_map(&mut allowed, Some("allowed"));
    assert_eq!(
        crate::module_runtime::resolve_module_specifier(&mut allowed, "fixture", &base_url)
            .expect("a parser import map with a matching nonce should register")
            .as_str(),
        "https://script-terminal.test/mapped.mjs"
    );
}
