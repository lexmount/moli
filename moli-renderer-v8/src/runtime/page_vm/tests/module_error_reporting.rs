use super::*;

fn bound_parser_module(page_vm: &mut PageVm, position: u32, url: Url) -> PreparedScript {
    let mut script = prepared_external_module_for_page_vm_test_with_node(page_vm, position, url);
    let runtime = &mut page_vm.vm_mut().document_runtime;
    let body = runtime.snapshot_document().document_body_handle().unwrap();
    let node = runtime
        .dom_host_mut()
        .create_parser_element_without_attributes(
            "script".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
    assert!(runtime.dom_host_mut().append_child(body, node));
    script.node_id = node;
    script.host_script_handle = Some(runtime.bind_parser_owned_script_handle_for_node(node));
    script
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_error_reporting_preserves_inline_source_origin() {
    run_page_vm_async_test(async {
        for (source, expected_line, expected_column) in [
            ("missingModuleReference", 17, 23),
            ("\nmissingModuleReference", 18, 1),
        ] {
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let document_url = Url::parse("https://example.com/inline-module-location.html").unwrap();
            let mut page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());
            page_vm.vm_mut().eval(r#"
                globalThis.__locations = [];
                globalThis.__onerrorLocations = [];
                addEventListener('error', event => {
                    __locations.push([event.filename, event.lineno, event.colno, event.error instanceof ReferenceError]);
                    event.preventDefault();
                });
                onerror = (message, filename, line, column, error) => {
                    __onerrorLocations.push([filename, line, column, error instanceof ReferenceError]);
                    return true;
                };
            "#).unwrap();
            let mut script = bound_parser_module(&mut page_vm, 9201, document_url.clone());
            script.source_kind = ScriptSourceKind::Inline;
            script.source = crate::planning::ScriptSource::Inline(source.to_owned());
            script.base_url = Url::parse("https://example.com/import-base/").unwrap();
            page_vm.vm_mut().document_runtime.note_parser_script_start_position(script.node_id, 17, 23);
            let work = install_parser_module_defer_work(&mut page_vm, script);
            page_vm.execute_post_parse_page_owned_task_on_named_owner_lane(&loader, work).await.unwrap();
            run_parser_module_completion_turns_for_test(&mut page_vm, &loader, 0, "inline module location").await;
            let expected = format!(r#"[["{document_url}",{expected_line},{expected_column},true]]"#);
            assert_eq!(page_vm.vm_mut().eval("JSON.stringify(__locations)").unwrap(), expected);
            assert_eq!(page_vm.vm_mut().eval("JSON.stringify(__onerrorLocations)").unwrap(), expected);
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_error_reporting_preserves_deferred_source_location() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/tla-location.html").unwrap(),
        );
        page_vm.vm_mut().eval(r#"
            globalThis.__locations = [];
            addEventListener('error', event => {
                __locations.push([event.filename, event.lineno, event.colno,
                    event.error === __locatedOriginal]);
                event.preventDefault();
            });
        "#).unwrap();
        let url = Url::parse("https://example.com/located-tla.mjs").unwrap();
        let script = bound_parser_module(&mut page_vm, 9202, url.clone());
        let work = install_parser_module_defer_work(&mut page_vm, script);
        page_vm.execute_post_parse_page_owned_task_on_named_owner_lane(&loader, work).await.unwrap();
        let source = "\nglobalThis.__locatedOriginal = Object.freeze(new TypeError('original'));\nawait new Promise((_, reject) => { globalThis.__rejectLocatedModule = () => reject(__locatedOriginal); });";
        enqueue_parser_owned_module_script_fetch_completion_for_test(&mut page_vm, 0, &url, source);
        assert!(run_next_main_module_fetch_terminal_for_test(&mut page_vm).unwrap().is_some());
        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "located TLA module").await;
        assert_eq!(page_vm.vm_mut().eval("__locations.length").unwrap(), "0");
        page_vm.vm_mut().eval("__rejectLocatedModule(); 'rejected'").unwrap();
        run_parser_module_completion_turns_for_test(&mut page_vm, &loader, 1, "located TLA module").await;
        let column = source.lines().nth(1).unwrap().find("new TypeError").unwrap() + 1;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(__locations)").unwrap(),
            format!(r#"[["{url}",2,{column},true]]"#));
    }).await;
}

async fn assert_parser_module_reports_original_value(value: &str, reject_later: bool) {
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut page_vm = test_page_vm_with_loader_and_document_url(
        &loader,
        Vec::new(),
        Url::parse("https://example.com/module-reporting.html").unwrap(),
    );
    page_vm
        .vm_mut()
        .eval(&format!(
            r#"
        globalThis.__thrown = {value};
        globalThis.__hadFileName = Object.hasOwn(Object(__thrown), 'fileName');
        globalThis.__errors = [];
        globalThis.__onerrors = [];
        globalThis.__coercions = 0;
        globalThis.__unhandled = 0;
        globalThis.__loads = 0;
        addEventListener('unhandledrejection', event => {{
            ++__unhandled;
            event.preventDefault();
        }});
        addEventListener('error', event => {{
            __errors.push(event.error);
            event.preventDefault();
        }});
        onerror = (message, filename, line, column, error) => {{
            __onerrors.push(error);
            return true;
        }};
        "#,
        ))
        .expect("install original exception and both observers");
    let module_url = Url::parse("https://example.com/report-original.mjs").unwrap();
    let script = bound_parser_module(&mut page_vm, 9101, module_url.clone());
    page_vm
        .vm_mut()
        .eval("document.querySelector('script').addEventListener('load', () => ++__loads);")
        .unwrap();
    let work = install_parser_module_defer_work(&mut page_vm, script);
    page_vm
        .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, work)
        .await
        .expect("module waits for its graph");
    let source = if reject_later {
        "await new Promise((_, reject) => { globalThis.__rejectModule = reject; });"
    } else {
        "throw globalThis.__thrown;"
    };
    enqueue_parser_owned_module_script_fetch_completion_for_test(
        &mut page_vm,
        0,
        &module_url,
        source,
    );
    assert!(
        run_next_main_module_fetch_terminal_for_test(&mut page_vm)
            .unwrap()
            .is_some()
    );
    run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "original module exception")
        .await;
    if reject_later {
        assert_eq!(page_vm.vm_mut().eval("__errors.length").unwrap(), "0");
        page_vm
            .vm_mut()
            .eval("__rejectModule(__thrown); 'rejected'")
            .unwrap();
    }
    run_parser_module_completion_turns_for_test(
        &mut page_vm,
        &loader,
        usize::from(reject_later),
        "original module exception",
    )
    .await;
    assert_eq!(
        page_vm.vm_mut().eval(
            "JSON.stringify([__errors.length, __onerrors.length, Object.is(__errors[0], __thrown), Object.is(__onerrors[0], __thrown), __coercions, Object.hasOwn(Object(__thrown), 'fileName') === __hadFileName])"
        ).unwrap(),
        "[1,1,true,true,0,true]",
        "module error reporting must preserve {value}, reject_later={reject_later}",
    );
    assert_eq!(
        page_vm
            .vm_mut()
            .eval("[__loads, __unhandled].join('|')")
            .unwrap(),
        "1|0",
        "module scripts must load once without leaking their internal rejection"
    );
    assert_eq!(
        page_vm
            .report
            .runs
            .iter()
            .filter(|run| run.url() == &module_url)
            .count(),
        1,
        "reporting must not complete a module script twice",
    );
}

const THROWN_VALUES: &[&str] = &[
    "Object.freeze({ marker: 42 })",
    "Object.freeze(new TypeError('original'))",
    "undefined",
    "null",
    "false",
    "-0",
    "NaN",
    "17n",
    "Symbol('original')",
    "'original string'",
    "({ toString() { ++__coercions; throw new Error('must not coerce'); } })",
    "({ get fileName() { ++__coercions; throw new Error('must not read'); }, set fileName(value) { ++__coercions; throw new Error('must not write'); } })",
];

#[tokio::test(flavor = "current_thread")]
async fn parser_module_error_reporting_preserves_synchronous_thrown_values() {
    run_page_vm_async_test(async {
        for value in THROWN_VALUES {
            assert_parser_module_reports_original_value(value, false).await;
        }
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_error_reporting_preserves_deferred_rejection_values() {
    run_page_vm_async_test(async {
        for value in THROWN_VALUES {
            assert_parser_module_reports_original_value(value, true).await;
        }
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_error_reporting_reuses_the_cached_parse_exception() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/parse-error.html").unwrap(),
        );
        page_vm.vm_mut().eval(r#"
            globalThis.__errors = [];
            addEventListener('error', event => {
                __errors.push(event.error);
                event.preventDefault();
            });
        "#).unwrap();
        let url = Url::parse("https://example.com/shared-syntax-error.mjs").unwrap();
        let owner = page_vm.vm().current_main_document_task_owner().unwrap();
        for position in [9101, 9102] {
            let script = bound_parser_module(&mut page_vm, position, url.clone());
            assert!(page_vm.vm_mut().claim_main_parser_deferred_script(
                owner, script, None, None, Default::default(),
            ).unwrap());
        }
        let work = page_vm.seal_main_parser_deferred_scripts(owner).unwrap();
        page_vm.execute_post_parse_page_owned_task_on_named_owner_lane(&loader, work).await.unwrap();
        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm, 0, &url, "export const broken = ;",
        );
        run_next_main_module_fetch_terminal_for_test(&mut page_vm).unwrap();
        while page_vm.vm_mut().has_ready_native_module_owner_actions() {
            run_next_native_module_owner_event_for_test(&mut page_vm, &loader, "shared syntax error").await;
        }
        for _ in 0..2 {
            run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "shared syntax error").await;
        }
        assert_eq!(page_vm.vm_mut().eval(
            "[__errors.length, __errors[0] instanceof SyntaxError, __errors[0] === __errors[1]].join('|')",
        ).unwrap(), "2|true|true");
    }).await;
}
