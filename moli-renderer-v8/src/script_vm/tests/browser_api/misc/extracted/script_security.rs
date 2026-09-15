use super::*;

#[test]
fn security_policy_violation_event_constructor_applies_init_defaults() {
    let mut vm = new_storage_test_vm("https://csp-violation-event-constructor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const defaults = new SecurityPolicyViolationEvent("securitypolicyviolation");
  const initialized = new SecurityPolicyViolationEvent("custom", {
    documentURI: "https://example.test/document",
    referrer: "https://example.test/referrer",
    blockedURI: "https://example.test/blocked",
    violatedDirective: "default-src",
    effectiveDirective: "script-src",
    originalPolicy: "default-src 'none'",
    disposition: "report",
    sourceFile: "https://example.test/source.js",
    sample: "sample",
    statusCode: 65537,
    lineNumber: -2,
    columnNumber: 3
  });
  return JSON.stringify({
    defaults: [
      defaults.documentURI,
      defaults.referrer,
      defaults.blockedURI,
      defaults.violatedDirective,
      defaults.effectiveDirective,
      defaults.originalPolicy,
      defaults.disposition,
      defaults.sourceFile,
      defaults.sample,
      defaults.statusCode,
      defaults.lineNumber,
      defaults.columnNumber
    ],
    initialized: {
      type: initialized.type,
      documentURI: initialized.documentURI,
      referrer: initialized.referrer,
      blockedURI: initialized.blockedURI,
      violatedDirective: initialized.violatedDirective,
      effectiveDirective: initialized.effectiveDirective,
      originalPolicy: initialized.originalPolicy,
      disposition: initialized.disposition,
      sourceFile: initialized.sourceFile,
      sample: initialized.sample,
      statusCode: initialized.statusCode,
      lineNumber: initialized.lineNumber,
      columnNumber: initialized.columnNumber,
      trusted: initialized.isTrusted,
      instance: initialized instanceof SecurityPolicyViolationEvent
    }
  });
})()
"#,
        )
        .expect("SecurityPolicyViolationEvent constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaults":["","","","","","","enforce","","",0,0,0],"initialized":{"type":"custom","documentURI":"https://example.test/document","referrer":"https://example.test/referrer","blockedURI":"https://example.test/blocked","violatedDirective":"default-src","effectiveDirective":"script-src","originalPolicy":"default-src 'none'","disposition":"report","sourceFile":"https://example.test/source.js","sample":"sample","statusCode":1,"lineNumber":4294967294,"columnNumber":3,"trusted":false,"instance":true}}"#
    );
}
#[test]
fn eval_csp_checks_string_sources_after_type_discrimination() {
    let mut blocked = new_storage_test_vm("https://eval-csp-blocked.test/");
    blocked.set_response_content_security_policies(&["script-src 'nonce-test'".to_owned()]);
    blocked.set_response_content_security_report_only_policies(&[
        "script-src 'nonce-test'".to_owned()
    ]);

    let blocked_result = blocked
        .eval(
            r#"
(() => {
  const input = ["0"];
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(`${event.disposition}:${event.lineNumber > 0 && event.columnNumber > 0}`);
  });
  let stringResult;
  try {
    eval("0");
    stringResult = "no-throw";
  } catch (error) {
    stringResult = `${error.name}:${error instanceof EvalError}`;
  }
  globalThis.__evalCspTypeDiscrimination = {
    stringResult,
    passThrough: eval(input) === input,
    violations
  };
  return "queued";
})()
"#,
        )
        .expect("blocked eval CSP probe should evaluate");
    assert_eq!(blocked_result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut blocked),
        2
    );
    assert_eq!(
        blocked
            .eval(
                "`${__evalCspTypeDiscrimination.stringResult}|${__evalCspTypeDiscrimination.passThrough}|${__evalCspTypeDiscrimination.violations.join(',')}`",
            )
            .expect("queued eval CSP violations should be observable"),
        "EvalError:true|true|report:true,enforce:true"
    );

    let mut allowed = new_storage_test_vm("https://eval-csp-allowed.test/");
    allowed.set_response_content_security_policies(&["script-src 'unsafe-eval'".to_owned()]);
    assert_eq!(
        allowed
            .eval("String(eval('40 + 2'))")
            .expect("unsafe-eval CSP probe should evaluate"),
        "42"
    );
}
#[tokio::test]
async fn child_eval_uses_child_document_csp_without_restricting_parent() {
    for policy in ["script-src 'none'", "require-trusted-types-for 'script'"] {
        let mut vm = new_storage_test_vm("https://child-eval-csp.test/");
        let serialized_policy =
            serde_json::to_string(policy).expect("test policy should serialize");
        vm.eval(&format!(
            r#"
(() => {{
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  const meta = frame.contentDocument.createElement("meta");
  meta.httpEquiv = "Content-Security-Policy";
  meta.content = {serialized_policy};
  (frame.contentDocument.head || frame.contentDocument.documentElement).appendChild(meta);
  return "created";
}})()
"#
        ))
        .expect("child CSP fixture should evaluate");
        assert_initial_about_blank_child_completed_synchronously_for_test(
            &mut vm,
            "child eval CSP fixture",
        )
        .await;
        let child_context_id =
            materialize_single_child_default_realm_for_test(&mut vm, "child eval CSP fixture");

        assert_eq!(
            vm.eval("eval(\"'parent-ok'\")")
                .expect("parent eval must remain independent from child CSP"),
            "parent-ok"
        );
        assert_eq!(
            vm.eval_in_child_default_context(
                child_context_id,
                r#"
(() => {
  const metaPolicy = document.querySelector('meta[http-equiv="Content-Security-Policy"]')?.content || "missing";
  try {
    eval("'child-ok'");
    return `${metaPolicy}|allowed`;
  } catch (error) {
    return `${metaPolicy}|${error.name}:${error instanceof EvalError}`;
  }
})()
"#,
            )
            .expect("child eval CSP probe should evaluate"),
            format!("{policy}|EvalError:true")
        );
        assert_eq!(
            vm.eval(
                r#"
(() => {
  try {
    frames[0].eval("'child-ok'");
    return "allowed";
  } catch (error) {
    return error.name;
  }
})()
"#,
            )
            .expect("cross-realm child eval probe should evaluate"),
            "EvalError"
        );
        let inspector_result = vm
            .begin_runtime_evaluate(
                None,
                r#"
(() => {
  try {
    frames[0].eval("'child-ok'");
    return "allowed";
  } catch (error) {
    return error.name;
  }
})()
"#,
                false,
                false,
                None,
                RuntimeEvaluateCodeGenerationPolicy::EnforceContextPolicy,
                RuntimeEvaluateResultMode::RemoteObject,
            )
            .and_then(|outcome| vm.require_completed_runtime_evaluate(outcome))
            .expect("Inspector cross-realm child eval probe should complete");
        assert_eq!(inspector_result["type"], "string");
        assert_eq!(inspector_result["value"], "EvalError");
    }
}
#[test]
fn eval_csp_distinguishes_code_like_objects_from_pass_through_objects() {
    use crate::script_vm::security_policy::{
        NonTrustedTypesCodeGenerationSource, non_trusted_types_code_generation_source,
    };

    let mut vm = new_storage_test_vm("https://eval-code-like-csp.test/");
    let (code_like, pass_through, code_like_string, plain_string) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            let object = v8::Object::new(scope);
            let string = v8::String::new(scope, "0").expect("test source should allocate");
            Ok((
                non_trusted_types_code_generation_source(object.into(), true),
                non_trusted_types_code_generation_source(object.into(), false),
                non_trusted_types_code_generation_source(string.into(), true),
                non_trusted_types_code_generation_source(string.into(), false),
            ))
        })
        .expect("eval source classification should run");

    assert_eq!(
        code_like,
        NonTrustedTypesCodeGenerationSource::CodeLikeObject
    );
    assert_eq!(
        pass_through,
        NonTrustedTypesCodeGenerationSource::PassThroughObject
    );
    assert_eq!(
        code_like_string,
        NonTrustedTypesCodeGenerationSource::String
    );
    assert_eq!(plain_string, NonTrustedTypesCodeGenerationSource::String);
}
#[test]
fn inline_event_attribute_csp_uses_script_src_attr_instead_of_the_eval_gate() {
    let mut allowed = new_storage_test_vm("https://event-attribute-csp-allowed.test/");
    allowed.set_response_content_security_policies(&[
        "script-src-attr 'unsafe-inline'; script-src 'nonce-test'".to_owned(),
    ]);
    let allowed_result = allowed
        .eval(
            r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(event.effectiveDirective);
  });
  const image = document.createElement("img");
  image.setAttribute("onload", "globalThis.__eventAttributeRan = true");
  const handler = image.onload;
  handler.call(image, new Event("load"));
  return `${typeof handler}|${globalThis.__eventAttributeRan}|${violations.join(',')}`;
})()
"#,
        )
        .expect("allowed inline event attribute probe should evaluate");
    assert_eq!(allowed_result, "function|true|");

    let mut blocked = new_storage_test_vm("https://event-attribute-csp-blocked.test/");
    blocked.set_response_content_security_policies(&[
        "script-src-attr 'none'; script-src 'unsafe-inline' 'unsafe-eval'".to_owned(),
    ]);
    blocked.set_response_content_security_report_only_policies(&[
        "script-src-attr 'none'; script-src 'unsafe-inline' 'unsafe-eval'".to_owned(),
    ]);
    let blocked_result = blocked
        .eval(
            r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(`${event.disposition}:${event.effectiveDirective}:${event.blockedURI}`);
  });
  const image = document.createElement("img");
  image.setAttribute("onload", "globalThis.__blockedEventAttributeRan = true");
  const handler = image.onload;
  globalThis.__blockedEventAttributeViolations = violations;
  return `${handler}|${globalThis.__blockedEventAttributeRan === true}|${violations.join(',')}`;
})()
"#,
        )
        .expect("blocked inline event attribute probe should evaluate");
    assert_eq!(blocked_result, "null|false|");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut blocked),
        2
    );
    assert_eq!(
        blocked
            .eval("globalThis.__blockedEventAttributeViolations.join(',')")
            .expect("queued inline event attribute violations should be observable"),
        "report:script-src-attr:inline,enforce:script-src-attr:inline"
    );
}
#[test]
fn runtime_evaluate_code_generation_policy_matches_inspector_scope() {
    let mut vm = new_storage_test_vm("https://runtime-evaluate-csp.test/");
    vm.set_response_content_security_policies(&["script-src 'nonce-test'".to_owned()]);

    let direct = vm
        .evaluate_expression_payload_with_await("40 + 2", true, false)
        .expect("Runtime.evaluate expression should bypass page eval CSP");
    assert_eq!(direct["type"], "number");
    assert_eq!(direct["value"], 42);

    let default_nested = vm
        .evaluate_expression_payload_with_await(
            r#"(() => {
  try {
    eval("1");
    return "allowed";
  } catch (error) {
    return error.name;
  }
})()"#,
            true,
            false,
        )
        .expect("Runtime.evaluate nested eval probe should complete");
    assert_eq!(default_nested["type"], "string");
    assert_eq!(default_nested["value"], "allowed");

    let enforced = vm
        .begin_runtime_evaluate(
            None,
            r#"(() => {
  try {
    return eval("40 + 2");
  } catch (error) {
    return error.name;
  }
})()"#,
            true,
            false,
            None,
            RuntimeEvaluateCodeGenerationPolicy::EnforceContextPolicy,
            RuntimeEvaluateResultMode::RemoteObject,
        )
        .and_then(|outcome| vm.require_completed_runtime_evaluate(outcome))
        .expect("explicit false Runtime.evaluate should still execute its outer expression");
    assert_eq!(enforced["type"], "string");
    assert_eq!(enforced["value"], "EvalError");

    let restored = vm
        .eval(
            r#"(() => {
  try {
    eval("1");
    return "allowed";
  } catch (error) {
    return error.name;
  }
})()"#,
        )
        .expect("page eval policy restoration probe should run");
    assert_eq!(restored, "EvalError");
}
#[test]
fn webassembly_compile_obeys_document_csp_wasm_eval() {
    let mut vm = new_storage_test_vm("https://wasm-csp.test/");
    vm.document_runtime
        .dom_host_mut()
        .reset_html_document_shell();
    let result = vm
        .eval(
            r#"
            (() => {
              const meta = document.createElement("meta");
              meta.setAttribute("http-equiv", "Content-Security-Policy");
              meta.setAttribute("content", "script-src 'self' 'unsafe-inline'");
              (document.head || document.documentElement || document).appendChild(meta);

              const events = [];
              self.addEventListener("securitypolicyviolation", event => {
                events.push([
                  event.violatedDirective,
                  event.effectiveDirective,
                  event.originalPolicy,
                  event.blockedURI,
                  event instanceof SecurityPolicyViolationEvent
                ].join("|"));
              });

              const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
              let thrown = "none";
              try {
                new WebAssembly.Module(bytes);
              } catch (error) {
                thrown = [
                  error && error.constructor && error.constructor.name,
                  error instanceof WebAssembly.CompileError
                ].join("|");
              }
              globalThis.__wasmCspCompileResult = { thrown, events };
              return "queued";
            })()
            "#,
        )
        .expect("wasm CSP compile probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__wasmCspCompileResult)")
            .expect("queued wasm CSP violation should be observable"),
        r#"{"thrown":"CompileError|true","events":["script-src|script-src|script-src 'self' 'unsafe-inline'|wasm-eval|true"]}"#
    );
}
#[test]
fn document_csp_violation_event_survives_mutated_event_globals() {
    let mut vm = new_storage_test_vm("https://wasm-csp-mutated-event.test/");
    vm.document_runtime
        .dom_host_mut()
        .reset_html_document_shell();
    let result = vm
        .eval(
            r#"
            (() => {
              const getBlockedURI = Object.getOwnPropertyDescriptor(SecurityPolicyViolationEvent.prototype, "blockedURI").get;
              self.Event = null;
              Object.defineProperty(SecurityPolicyViolationEvent.prototype, "blockedURI", {
                value: "prototype-blocked-uri",
                writable: false,
                configurable: true
              });

              const meta = document.createElement("meta");
              meta.setAttribute("http-equiv", "Content-Security-Policy");
              meta.setAttribute("content", "script-src 'self' 'unsafe-inline'");
              (document.head || document.documentElement || document).appendChild(meta);

              const events = [];
              self.addEventListener("securitypolicyviolation", event => {
                events.push({
                  type: event.type,
                  blockedURI: getBlockedURI.call(event),
                  publicBlockedURI: event.blockedURI,
                  bubbles: event.bubbles,
                  cancelable: event.cancelable,
                  composed: event.composed,
                  effectiveDirective: event.effectiveDirective,
                  disposition: event.disposition,
                  instance: event instanceof SecurityPolicyViolationEvent
                });
              });

              const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
              let thrown = "none";
              try {
                new WebAssembly.Module(bytes);
              } catch (error) {
                thrown = error && error.constructor && error.constructor.name;
              }
              globalThis.__mutatedEventCspResult = { thrown, events };
              return "queued";
            })()
            "#,
        )
        .expect("document CSP mutated event probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__mutatedEventCspResult)")
            .expect("queued CSP event should survive mutated event globals"),
        r#"{"thrown":"CompileError","events":[{"type":"securitypolicyviolation","blockedURI":"wasm-eval","publicBlockedURI":"prototype-blocked-uri","bubbles":true,"cancelable":false,"composed":true,"effectiveDirective":"script-src","disposition":"enforce","instance":true}]}"#
    );
}
#[test]
fn document_csp_report_only_wasm_eval_dispatches_without_blocking() {
    let mut vm = new_storage_test_vm("https://wasm-csp-report-only.test/");
    vm.set_response_content_security_report_only_policies(&[String::from(
        "script-src 'self' 'unsafe-inline'",
    )]);
    let result = vm
        .eval(
            r#"
            (() => {
              const events = [];
              self.addEventListener("securitypolicyviolation", event => {
                events.push({
                  blockedURI: event.blockedURI,
                  effectiveDirective: event.effectiveDirective,
                  disposition: event.disposition,
                  instance: event instanceof SecurityPolicyViolationEvent
                });
              });

              const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
              globalThis.__reportOnlyWasmCspResult = {
                module: new WebAssembly.Module(bytes) instanceof WebAssembly.Module,
                events
              };
              return "queued";
            })()
            "#,
        )
        .expect("document CSP report-only wasm probe should evaluate");

    assert_eq!(result, "queued");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__reportOnlyWasmCspResult)")
            .expect("queued report-only wasm CSP violation should be observable"),
        r#"{"module":true,"events":[{"blockedURI":"wasm-eval","effectiveDirective":"script-src","disposition":"report","instance":true}]}"#
    );
}
#[test]
fn webassembly_compile_allows_wasm_unsafe_eval_source() {
    let mut vm = new_storage_test_vm("https://wasm-csp-allowed.test/");
    let result = vm
        .eval(
            r#"
            (() => {
              const meta = document.createElement("meta");
              meta.setAttribute("http-equiv", "Content-Security-Policy");
              meta.setAttribute(
                "content",
                "script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'"
              );
              (document.head || document.documentElement || document).appendChild(meta);

              const events = [];
              self.addEventListener("securitypolicyviolation", () => events.push("event"));

              const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
              return JSON.stringify({
                module: new WebAssembly.Module(bytes) instanceof WebAssembly.Module,
                events
              });
            })()
            "#,
        )
        .expect("wasm CSP allow probe should evaluate");

    assert_eq!(result, r#"{"module":true,"events":[]}"#);
}
#[test]
fn exec_with_script_url_sets_eval_error_stack_source_name() {
    let page_url = Url::parse("https://example.com/path/page.html").expect("page url");
    let mut vm = new_storage_test_vm(page_url.as_str());

    vm.exec(
        r#"
        (() => {
            try {
                eval("throw new Error('eval-probe')");
            } catch (error) {
                globalThis.__evalStackSourceProbe = String(error.stack);
            }
        })();
        "#,
        Some(&page_url),
    )
    .expect("script execution should succeed");

    let stack = vm
        .eval("globalThis.__evalStackSourceProbe")
        .expect("eval stack probe should evaluate");

    assert!(
        stack.contains(page_url.as_str()),
        "eval stack should include script resource name: {stack}"
    );
    assert!(
        !stack.contains("unknown source"),
        "eval stack should not degrade to unknown source: {stack}"
    );
}
