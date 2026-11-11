use super::*;

const DYNAMIC_INLINE_ERRORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dynamic-inline-errors.js"
));

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
