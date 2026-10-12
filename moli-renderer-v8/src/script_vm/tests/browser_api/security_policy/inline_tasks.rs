use super::*;
use crate::runtime::{PageSelectedTaskTestSelector, PageVmTaskExecutorTestHarness};

const OWNER_TASK_PROBE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/inline-csp-owner-tasks.js"
));

pub(super) async fn drain_inline_csp_page_tasks_for_test(
    vm: &mut PageVmTaskExecutorTestHarness,
) -> usize {
    drain_inline_page_tasks_for_test(vm, PageSelectedTaskTestSelector::MiscPlatformApi).await
}

pub(super) async fn drain_inline_style_page_tasks_for_test(
    vm: &mut PageVmTaskExecutorTestHarness,
) -> bool {
    drain_inline_page_tasks_for_test(vm, PageSelectedTaskTestSelector::StyleElementEvent).await > 0
}

async fn drain_inline_page_tasks_for_test(
    vm: &mut PageVmTaskExecutorTestHarness,
    selector: PageSelectedTaskTestSelector,
) -> usize {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for count in 0..64 {
        if !vm
            .run_one_exact_selected_page_task_executor_turn(selector, &loader)
            .await
            .expect("inline CSP task must execute through the selected Page dispatcher")
        {
            return count;
        }
    }
    panic!("inline CSP fixture did not drain its reporting tasks");
}

#[test]
fn popup_dynamic_inline_classic_scripts_run_synchronously_once_in_their_owner() {
    let mut vm = new_storage_page_task_executor_test_vm("https://popup-inline-script-owner.test/");
    vm.eval(
        r#"
const popup = open(); popup.popupTrace = [];
const scripts = ['first', 'second'].map(id => {
  const script = popup.document.createElement('script'); script.id = id;
  script.textContent = 'window.popupTrace.push([document.currentScript.id, globalThis === window, document === window.document]);';
  popup.document.body.appendChild(script); return script;
});
const immediate = JSON.stringify(popup.popupTrace);
scripts[0].remove(); popup.document.body.appendChild(scripts[0]);
"#,
    )
    .unwrap();
    let expected = r#"[["first",true,true],["second",true,true]]"#;
    assert_eq!(vm.eval("immediate").unwrap(), expected);
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(popup.popupTrace)")
            .unwrap(),
        expected,
    );
    assert_eq!(
        vm.eval("popup.document.currentScript === null").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("globalThis.popupTrace === undefined").unwrap(),
        "true"
    );
    vm.eval("popup.close()").unwrap();
}

#[test]
fn inline_csp_reports_share_owner_task_order_and_dispatch_time_targets() {
    for owner in ["window", "frame.contentWindow", "popup"] {
        let mut vm = new_storage_page_task_executor_test_vm("https://inline-csp-owner.test/");
        vm.eval(&format!(
            "const frame = document.createElement('iframe'); document.body.appendChild(frame); \
             const popup = open(); const owner = {owner}; \
             globalThis.inlineCspProbe = owner.eval({});",
            serde_json::to_string(&format!("({OWNER_TASK_PROBE})(globalThis)")).unwrap(),
        ))
        .unwrap();
        let result: serde_json::Value = serde_json::from_str(
            &vm.eval_after_selected_page_tasks("JSON.stringify(inlineCspProbe.finish())")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(result["total"], 15, "{owner}: {result:#}");
        assert_eq!(result["passed"], 15, "{owner}: {result:#}");
        vm.eval("popup.close(); frame.remove()").unwrap();
    }
}

#[test]
fn inline_styles_use_the_element_owner_policy_in_both_directions() {
    for owner in ["frame.contentWindow", "popup"] {
        for block_top in [false, true] {
            let mut vm = new_storage_page_task_executor_test_vm("https://inline-style-owner.test/");
            vm.eval(&format!(
                r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const popup = open(); const owner = {owner};
const reports = {{ top: [], other: [] }};
document.addEventListener('securitypolicyviolation', event => reports.top.push(event.effectiveDirective));
owner.document.addEventListener('securitypolicyviolation', event => reports.other.push(event.effectiveDirective));
const blockedDocument = {blocked};
const meta = blockedDocument.createElement('meta');
meta.httpEquiv = 'Content-Security-Policy'; meta.content = "style-src 'none'";
blockedDocument.head.appendChild(meta);
const addStyle = doc => {{
  const style = doc.createElement('style'); style.textContent = 'div {{ margin-left: 11px; }}';
  doc.body.appendChild(style);
  const div = doc.createElement('div'); doc.body.appendChild(div);
  div.setAttribute('style', 'margin-right: 13px;');
  return [style.sheet === null, div.style.length];
}};
const topStyle = addStyle(document); const otherStyle = addStyle(owner.document);
"#,
                blocked = if block_top { "document" } else { "owner.document" },
            ))
            .unwrap();
            let expected = if block_top {
                r#"[[true,0],[false,1],{"top":["style-src-elem","style-src-attr"],"other":[]}]"#
            } else {
                r#"[[false,1],[true,0],{"top":[],"other":["style-src-elem","style-src-attr"]}]"#
            };
            assert_eq!(
                vm.eval_after_selected_page_tasks(
                    "JSON.stringify([topStyle, otherStyle, reports])"
                )
                .unwrap(),
                expected,
                "owner={owner}, block_top={block_top}",
            );
            vm.eval("popup.close(); frame.remove()").unwrap();
        }
    }
}

#[test]
fn retired_inline_csp_tasks_cannot_retarget_a_replaced_child_document() {
    let mut vm = new_storage_page_task_executor_test_vm("https://inline-csp-retirement.test/");
    vm.eval(
        r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const owner = frame.contentWindow; const oldDocument = owner.document;
globalThis.retiredInlineReports = [];
const meta = oldDocument.createElement('meta'); meta.httpEquiv = 'Content-Security-Policy';
meta.content = "script-src 'none'; style-src 'none'"; oldDocument.head.appendChild(meta);
const style = oldDocument.createElement('style'); style.textContent = 'body { margin: 1px; }';
oldDocument.body.appendChild(style);
const button = oldDocument.createElement('button'); oldDocument.body.appendChild(button);
button.setAttribute('onclick', 'return 1'); button.setAttribute('style', 'margin: 2px;');
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval(
            "JSON.stringify([style.sheet === null, button.onclick === null, button.style.length])"
        )
        .unwrap(),
        "[true,true,0]",
    );
    vm.eval(
        r#"
owner.document.open(); owner.document.write('<!doctype html><body id="replacement"></body>');
owner.document.close();
owner.document.addEventListener('securitypolicyviolation', event => retiredInlineReports.push(event.sample));
document.addEventListener('securitypolicyviolation', event => retiredInlineReports.push(event.sample));
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks(
            "JSON.stringify([frame.contentDocument.body.id, retiredInlineReports])"
        )
        .unwrap(),
        r#"["replacement",[]]"#
    );
}

#[test]
fn inline_stylesheet_adoption_rechecks_the_destination_policy_and_preserves_report_owner() {
    for owner in ["frame.contentWindow", "popup"] {
        for block_top in [false, true] {
            let mut vm =
                new_storage_page_task_executor_test_vm("https://inline-style-adoption.test/");
            vm.eval(&format!(
                r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const popup = open(); const owner = {owner};
const reports = [];
document.addEventListener('securitypolicyviolation', event => reports.push([
  'top', event.target === document, event.effectiveDirective
]));
owner.document.addEventListener('securitypolicyviolation', event => reports.push([
  'other', event.target === owner.document, event.effectiveDirective
]));
const blockedDocument = {blocked}; const allowedDocument = {allowed};
const meta = blockedDocument.createElement('meta'); meta.httpEquiv = 'Content-Security-Policy';
meta.content = "style-src 'none'"; blockedDocument.head.appendChild(meta);
const style = allowedDocument.createElement('style'); style.textContent = 'body {{ margin: 19px; }}';
allowedDocument.head.appendChild(style); const allowedBefore = style.sheet !== null;
blockedDocument.head.appendChild(style); const blockedDuring = style.sheet === null;
allowedDocument.head.appendChild(style); const allowedAfter = style.sheet !== null;
"#,
                blocked = if block_top { "document" } else { "owner.document" },
                allowed = if block_top { "owner.document" } else { "document" },
            ))
            .unwrap();
            assert_eq!(
                vm.eval_after_selected_page_tasks(
                    "JSON.stringify([allowedBefore, blockedDuring, allowedAfter, reports])"
                )
                .unwrap(),
                format!(
                    r#"[true,true,true,[["{}",true,"style-src-elem"]]]"#,
                    if block_top { "top" } else { "other" },
                ),
                "owner={owner}, block_top={block_top}",
            );
            vm.eval("popup.close(); frame.remove()").unwrap();
        }
    }
}

#[test]
fn inline_csp_captures_javascript_navigation_call_location_before_reporting() {
    let mut vm = new_storage_page_task_executor_test_vm("https://inline-csp-location.test/");
    vm.set_response_content_security_policies(&["script-src 'none'".to_owned()]);
    vm.eval(
        "globalThis.navigationCspReports = []; document.addEventListener('securitypolicyviolation', \
         event => navigationCspReports.push([event.sourceFile, event.lineNumber, event.columnNumber]));",
    )
    .unwrap();
    vm.eval(
        "\n\n\nlocation.href = 'javascript:globalThis.blockedNavigationRan = true';\n\
         //# sourceURL=https://navigation-source.test/start.js?secret=removed#removed",
    )
    .unwrap();
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(navigationCspReports)")
            .unwrap(),
        r#"[["https://navigation-source.test/start.js",4,15]]"#,
    );
    assert_eq!(
        vm.eval("globalThis.blockedNavigationRan === true").unwrap(),
        "false"
    );
}

#[test]
fn declarative_css_modules_report_both_blocking_policies_for_each_owner() {
    for owner in ["window", "frame.contentWindow", "popup"] {
        let mut vm =
            new_storage_page_task_executor_test_vm("https://css-module-policy-owner.test/");
        vm.eval(&format!(
            r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const popup = open(); const owner = {owner}; const doc = owner.document;
globalThis.moduleCspReports = [];
doc.addEventListener('securitypolicyviolation', event => moduleCspReports.push([
  event.effectiveDirective, event.target.id
]));
const meta = doc.createElement('meta'); meta.httpEquiv = 'Content-Security-Policy';
meta.content = "style-src 'none'; script-src 'none'"; doc.head.appendChild(meta);
const style = doc.createElement('style'); style.type = 'module'; style.id = 'blocked-module';
style.setAttribute('specifier', 'theme'); style.textContent = 'body {{ margin: 17px; }}';
doc.head.appendChild(style);
"#,
        ))
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("JSON.stringify(moduleCspReports)")
                .unwrap(),
            r#"[["style-src-elem","blocked-module"],["script-src-elem","blocked-module"]]"#,
            "{owner}",
        );
        vm.eval("popup.close(); frame.remove()").unwrap();
    }
}
