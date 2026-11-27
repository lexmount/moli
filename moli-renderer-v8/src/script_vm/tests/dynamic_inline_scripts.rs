use super::*;

const DYNAMIC_INLINE_ERRORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dynamic-inline-errors.js"
));

#[test]
fn current_script_remains_visible_after_adopted_node_root_lookup() {
    let mut vm = new_parsed_test_vm(
        "https://adopted-node.test/page.html",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
            globalThis.detachedDocument = document.implementation.createHTMLDocument();
            const node = detachedDocument.createElement('span');
            document.body.appendChild(node);
            const rootIsDocument = node.getRootNode() === document;

            globalThis.probeScript = document.createElement('script');
            probeScript.textContent = `
              globalThis.scriptState = [
                document.currentScript === probeScript,
                detachedDocument.currentScript === null,
              ];
            `;
            document.head.appendChild(probeScript);

            JSON.stringify([rootIsDocument, ...scriptState, document.currentScript === null])
            "#,
        )
        .unwrap();
    assert_eq!(result, "[true,true,true,true]");
}

#[test]
fn current_script_stays_with_its_execution_document_after_adoption() {
    let mut vm = new_parsed_test_vm(
        "https://inline-adoption.test/page.html",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
            globalThis.adoptionResults = [];
            globalThis.outer = document.createElement('script');
            outer.textContent = `
              globalThis.inner = document.createElement('script');
              inner.textContent = \`
                adoptionResults.push(document.currentScript === inner);
                globalThis.destination = document.implementation.createHTMLDocument();
                destination.body.appendChild(inner);
                adoptionResults.push(inner.ownerDocument === destination);
                adoptionResults.push(document.currentScript === inner);
                adoptionResults.push(destination.currentScript === null);
              \`;
              document.body.appendChild(inner);
              adoptionResults.push(document.currentScript === outer);
            `;
            document.body.appendChild(outer);
            adoptionResults.push(document.currentScript === null);
            JSON.stringify(adoptionResults)
            "#,
        )
        .unwrap();
    assert_eq!(result, "[true,true,true,true,true,true]");
}

#[test]
fn main_dynamic_inline_script_errors_are_synchronous_and_restore_current_script() {
    for borrow_child_insertion in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://inline-errors.test/page.html",
            "<!doctype html><body></body>",
        );
        let result: serde_json::Value = serde_json::from_str(
            &vm.eval(&format!(
                "JSON.stringify(({DYNAMIC_INLINE_ERRORS})({borrow_child_insertion}))"
            ))
            .expect("inserted script exceptions must not escape the DOM operation"),
        )
        .unwrap();
        let expected_events = serde_json::json!([
            "outer:outer",
            "error:syntax",
            "returned:syntax:outer",
            "error:runtime",
            "recovery:recovery",
            "handler-restored:runtime",
            "returned:runtime:outer",
            "error:value",
            "returned:value:outer",
            "success:success",
            "returned:success:outer",
            "error:null",
            "shadow-returned:outer",
            "returned:null"
        ]);
        assert_eq!(result["events"], expected_events, "{result}");
        assert_eq!(
            result["errors"],
            serde_json::json!([
                ["SyntaxError", "syntax", true, true],
                ["TypeError", "runtime", true, true],
                ["marker", "value", true, true],
                ["SyntaxError", null, true, true]
            ]),
            "borrow_child_insertion={borrow_child_insertion}: {result}"
        );
        assert_eq!(result["elementEvents"], serde_json::json!([]));
        let mut after_cleanup = expected_events.as_array().unwrap().clone();
        after_cleanup.extend(std::iter::repeat_n(serde_json::json!("microtask:null"), 4));
        assert_eq!(
            vm.eval("JSON.stringify(__dynamicInlineProbe.events)")
                .unwrap(),
            serde_json::to_string(&after_cleanup).unwrap(),
            "error-handler microtasks must wait for the inserting script to finish"
        );
    }
}

#[test]
fn main_dynamic_inline_script_errors_from_isolated_world_use_document_realm() {
    let mut vm = new_parsed_test_vm(
        "https://inline-errors.test/page.html",
        "<!doctype html><body></body>",
    );
    vm.exec(
        r#"
        globalThis.inlineErrors = [];
        window.onerror = (message, source, line, column, error) => {
          inlineErrors.push([error.name, error instanceof Error, document.currentScript.id]);
          return true;
        };
        "#,
        None,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("inline-script-insertion", false)
        .unwrap();
    let result = vm
        .eval_in_isolated_context(
            isolated,
            r#"
        (() => {
          globalThis.executedInline = false;
          for (const [id, source] of [
            ['syntax', '{'],
            ['runtime', "globalThis.executedInline = true; throw new TypeError('test')"],
          ]) {
            const script = document.createElement('script');
            script.id = id;
            script.textContent = source;
            document.body.appendChild(script);
          }
          return String(executedInline) + ':' + String(document.currentScript);
        })()
    "#,
        )
        .unwrap();
    assert_eq!(result, "false:null");
    assert_eq!(vm.eval("String(executedInline)").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(inlineErrors)").unwrap(),
        r#"[["SyntaxError",true,"syntax"],["TypeError",true,"runtime"]]"#
    );
}

#[tokio::test]
async fn empty_src_assignment_prepares_a_connected_html_script_once() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://empty-script-source.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.eval(
        r#"
        globalThis.emptySourceErrors = 0;
        const script = document.createElement('script');
        script.onerror = () => emptySourceErrors++;
        document.body.append(script);
        script.src = '';
    "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "emptySourceErrors",
        "1",
        "empty src reports its preparation error",
    )
    .await;
}
