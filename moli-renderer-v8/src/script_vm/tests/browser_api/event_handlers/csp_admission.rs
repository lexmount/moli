use super::*;

const ADMISSION_PROBE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/inline-handler-csp-admission.js"
));

#[test]
fn inline_handler_csp_admission_runs_on_attribute_changes_for_each_window_owner() {
    for owner in ["window", "frame.contentWindow", "popup"] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://inline-handler-admission.test/",
            "<!doctype html><body></body>",
            &loader,
        );
        vm.eval(&format!(
            "const frame = document.createElement('iframe'); document.body.appendChild(frame); \
             const popup = open(); const admissionProbe = ({ADMISSION_PROBE})({owner});"
        ))
        .expect("mutations should perform inline handler CSP admission");
        vm.eval_after_selected_page_tasks("admissionProbe.beforeGetter()")
            .expect("read admitted handlers after CSP tasks");
        let result: serde_json::Value = serde_json::from_str(
            &vm.eval_after_selected_page_tasks("JSON.stringify(admissionProbe.finish())")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(result["total"], 33, "{owner}: {result:#}");
        assert_eq!(result["passed"], result["total"], "{owner}: {result:#}");
        vm.eval("popup.close(); frame.remove()")
            .expect("close admission fixture owners");
    }
}

#[test]
fn report_only_inline_handler_admission_reports_once_per_mutation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://inline-handler-report-only.test/",
        "<!doctype html><body><button id=target></button></body>",
        &loader,
    );
    vm.set_response_content_security_report_only_policies(&[
        "script-src-attr 'none' 'report-sample'".to_owned(),
    ]);
    vm.eval(
        r#"
globalThis.admissionEvents = [];
document.addEventListener('securitypolicyviolation', event => admissionEvents.push([
  event.disposition, event.effectiveDirective, event.target.id, event.sample
]));
const target = document.getElementById('target');
target.setAttribute('onclick', 'return 61');
target.getAttributeNode('onclick').value = 'return 61';
"#,
    )
    .unwrap();
    let expected = r#"[["report","script-src-attr","target","return 61"],["report","script-src-attr","target","return 61"]]"#;
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(admissionEvents)")
            .unwrap(),
        expected
    );
    assert_eq!(vm.eval("target.onclick() === 61").unwrap(), "true");
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(admissionEvents)")
            .unwrap(),
        expected
    );
}

#[test]
fn unmodeled_document_handler_sources_keep_their_fallback_csp_check_after_adoption() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://inert-handler-admission.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.set_response_content_security_policies(&[
        "script-src-attr 'none' 'report-sample'".to_owned()
    ]);
    vm.eval(
        r#"
globalThis.inertAdmissionEvents = [];
document.addEventListener('securitypolicyviolation', event =>
  inertAdmissionEvents.push([event.target.id, event.sample]));
const template = document.createElement('template');
const inertDocuments = [document.implementation.createHTMLDocument(''), template.content.ownerDocument];
const inertButtons = inertDocuments.map((doc, index) => {
  const button = doc.createElement('button');
  button.id = 'inert-' + index;
  button.setAttribute('onclick', 'return ' + (index + 1));
  return button;
});
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(inertAdmissionEvents)")
            .unwrap(),
        "[]"
    );
    // These owners lack native CSP policy state. Preserve the existing deferred
    // check rather than treating their sources as if admission had succeeded.
    assert_eq!(
        vm.eval(
            "inertButtons.every(button => { document.body.appendChild(button); return button.onclick === null; })"
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(inertAdmissionEvents)")
            .unwrap(),
        r#"[["inert-0","return 1"],["inert-1","return 2"]]"#
    );
}

#[test]
fn inline_handler_admission_only_checks_content_attributes_supported_by_the_element() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://handler-interface-admission.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.set_response_content_security_policies(&[
        "script-src-attr 'none' 'report-sample'".to_owned()
    ]);
    vm.eval(
        r#"
globalThis.interfaceAdmissionEvents = [];
document.addEventListener('securitypolicyviolation', event =>
  interfaceAdmissionEvents.push([event.target.localName, event.sample]));
const ordinary = document.createElement('button');
document.body.appendChild(ordinary);
for (const name of ['onstorage', 'onencrypted', 'onwaitingforkey', 'onbegin', 'onend', 'onrepeat']) {
  ordinary.setAttribute(name, 'return 1');
}
const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
document.body.appendChild(svg);
svg.setAttribute('onbegin', 'return 1');
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(interfaceAdmissionEvents)")
            .unwrap(),
        "[]"
    );
    vm.eval(
        r#"
const video = document.createElement('video');
document.body.appendChild(video);
video.setAttribute('onencrypted', 'return 2');
video.setAttribute('onwaitingforkey', 'return 3');
const animate = document.createElementNS('http://www.w3.org/2000/svg', 'animate');
svg.appendChild(animate);
animate.setAttribute('onbegin', 'return 4');
animate.setAttribute('onend', 'return 5');
animate.setAttribute('onrepeat', 'return 6');
document.body.setAttribute('onstorage', 'return 7');
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(interfaceAdmissionEvents)")
            .unwrap(),
        r#"[["video","return 2"],["video","return 3"],["animate","return 4"],["animate","return 5"],["animate","return 6"],["body","return 7"]]"#
    );
}

#[tokio::test]
async fn live_parser_checks_handler_attributes_before_getters_and_keeps_window_idl_handlers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://parser-inline-handler-admission.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    let markup = r#"<!doctype html><html><head>
<meta http-equiv="Content-Security-Policy" content="script-src-attr 'none' 'report-sample'">
<script>
window.admissionEvents = [];
window.previousResize = () => 73;
window.onresize = previousResize;
document.addEventListener('securitypolicyviolation', event => admissionEvents.push([
  event.target.localName, event.sample
]));
</script></head><body onresize="return 71">
<button id="target" onclick="return 67"></button>
<script>window.admissionParserDone = true;</script></body></html>"#;
    vm.eval(&format!(
        "const frame = document.createElement('iframe'); frame.srcdoc = {}; document.body.appendChild(frame);",
        serde_json::to_string(markup).unwrap()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(frame.contentWindow.admissionParserDone === true && frame.contentWindow.admissionEvents.length === 2)",
        "true",
        "parser handler CSP tasks before getters",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(frame.contentWindow.admissionEvents)")
            .unwrap(),
        r#"[["body","return 71"],["button","return 67"]]"#
    );
    assert_eq!(
        vm.eval("frame.contentWindow.onresize === frame.contentWindow.previousResize && frame.contentDocument.body.onresize === frame.contentWindow.previousResize && frame.contentDocument.getElementById('target').onclick === null").unwrap(),
        "true"
    );
    vm.eval("frame.remove()").unwrap();
}
