use super::*;

const PROBE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/iframe-javascript-csp-admission.js"
));

fn check_admission_scenarios(scenarios: &[&str]) {
    for owner in ["window", "hostFrame.contentWindow", "popup"] {
        for setter in ["idl", "setAttribute"] {
            for scenario in scenarios {
                let mut vm =
                    new_storage_page_task_executor_test_vm("https://iframe-csp-admission.test/");
                vm.eval(&format!(
                    "const hostFrame = document.createElement('iframe'); document.body.appendChild(hostFrame); \
                     const popup = open(); globalThis.admissionProbe = ({PROBE})({owner}, {scenario:?}, {setter:?});",
                ))
                .unwrap();
                let result: serde_json::Value = serde_json::from_str(
                    &vm.eval_after_selected_page_tasks("JSON.stringify(admissionProbe.finish())")
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    result["passed"], result["total"],
                    "owner={owner}, setter={setter}, scenario={scenario}: {result:#}",
                );
                assert!(result["total"].as_u64().unwrap() >= 12);
                vm.eval("popup.close(); hostFrame.remove()").unwrap();
            }
        }
    }
}

#[test]
fn iframe_javascript_url_checks_source_and_target_document_policies() {
    check_admission_scenarios(&[
        "inherited",
        "source-only",
        "target-only",
        "permissive",
        "no-opt-in",
    ]);
}

#[test]
fn iframe_javascript_url_freezes_source_policy_at_actual_attribute_admission() {
    check_admission_scenarios(&["source-policy-after-admission", "policy-before-connection"]);
}

#[test]
fn iframe_javascript_url_reports_once_per_mutation_and_retargets_at_dispatch() {
    check_admission_scenarios(&["repeat-same-src", "getter-reads", "retarget-before-report"]);
}

#[test]
fn iframe_javascript_url_admission_respects_srcdoc_detachment_and_navigation_replacement() {
    check_admission_scenarios(&["srcdoc-priority", "detached", "cancel-before-execution"]);
}

#[test]
fn iframe_javascript_url_source_report_only_policy_does_not_block_execution() {
    let mut vm = new_storage_page_task_executor_test_vm("https://iframe-csp-report-only.test/");
    vm.eval(
        "const frame = document.createElement('iframe'); document.body.appendChild(frame); \
         const target = frame.contentWindow; globalThis.violations = []; \
         document.addEventListener('securitypolicyviolation', event => violations.push([event.target === frame, event.disposition, event.sample]));",
    )
    .unwrap();
    vm.set_response_content_security_report_only_policies(&[
        "script-src 'none' 'report-sample'".to_owned()
    ]);
    vm.eval("frame.src = 'javascript:globalThis.reportOnlyRan = 1'")
        .unwrap();
    assert_eq!(vm.eval("violations.length").unwrap(), "0");
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify([target.reportOnlyRan, violations])")
            .unwrap(),
        r#"[1,[[true,"report","javascript:globalThis.reportOnlyRan = 1"]]]"#,
    );
}
