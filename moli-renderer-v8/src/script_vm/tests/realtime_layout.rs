use super::*;
use moli_page_types::LayoutPolicy;

fn realtime_vm(markup: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm("https://realtime-layout.test/", markup);
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm
}

#[test]
fn realtime_layout_writes_and_reads_in_one_call_share_only_current_inputs() {
    let mut vm = realtime_vm(
        "<!doctype html><div id=target style='width:100px;height:10px;padding:0;border:0'></div>",
    );
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "const widths=[target.offsetWidth];target.style.width='200px';\
             widths.push(target.offsetWidth,target.offsetWidth);\
             target.style.width='300px';widths.push(target.offsetWidth);JSON.stringify(widths)",
        )
        .unwrap(),
        "[100,200,200,300]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
    vm.eval("getComputedStyle(target).color;target.getBoundingClientRect();target.offsetWidth")
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 3);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn realtime_layout_observes_cssom_sources_without_dom_attribute_mutations() {
    let mut vm = realtime_vm(
        "<!doctype html><style>#target{width:100px;height:10px}</style><div id=target></div>",
    );
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "document.styleSheets[0].cssRules[0].style.width='240px';String(target.offsetWidth)",
        )
        .unwrap(),
        "240"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(
        vm.eval(
            "const sheet=new CSSStyleSheet();sheet.replaceSync('#target{width:360px}');\
             document.adoptedStyleSheets=[sheet];String(target.offsetWidth)",
        )
        .unwrap(),
        "360"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "360");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
}

#[test]
fn realtime_layout_refresh_does_not_call_page_callbacks() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(
        vm.eval(
            "window.callbacks=[];\
             new MutationObserver(()=>callbacks.push('mutation')).observe(target,{attributes:true});\
             requestAnimationFrame(()=>callbacks.push('frame'));\
             target.offsetWidth;target.style.width='200px';\
             JSON.stringify([target.offsetWidth,callbacks.slice()])",
        )
        .unwrap(),
        "[200,[]]"
    );
}

#[test]
fn realtime_layout_child_reads_refresh_parent_viewport_without_replacing_root() {
    let mut vm = realtime_vm(
        "<!doctype html><iframe id=frame style='width:200px;height:100px;border:0'></iframe>",
    );
    vm.eval(
        "window.child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><body style=margin:0><div id=target style=width:100%;height:10px></div>');\
         child.document.close();window.childTarget=child.document.getElementById('target')",
    )
    .unwrap();
    assert_eq!(vm.eval("String(childTarget.offsetWidth)").unwrap(), "200");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "frame.style.width='320px';JSON.stringify([childTarget.offsetWidth,frame.offsetWidth,childTarget.offsetWidth])",
        )
        .unwrap(),
        "[320,320,320]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_layout_does_not_keep_geometry_from_a_replaced_document() {
    let mut vm = realtime_vm("<!doctype html><div id=target style='width:100px'></div>");
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval(
            "document.open();document.write('<!doctype html><div id=newTarget style=width:270px></div>');\
             document.close();String(newTarget.offsetWidth)",
        )
        .unwrap(),
        "270"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn realtime_layout_native_state_and_scroll_commit_before_geometry_reads() {
    let mut vm = realtime_vm(
        "<!doctype html><style>#scroller{width:100px}#flag:checked~#scroller{width:200px}</style>\
         <input id=flag type=checkbox><div id=scroller style='height:50px;overflow:auto'>\
         <div id=target style='width:20px;height:500px'></div></div>",
    );
    assert_eq!(vm.eval("String(scroller.offsetWidth)").unwrap(), "100");
    assert_eq!(
        vm.eval(
            "flag.checked=true;\
             const before=target.getBoundingClientRect().top;scroller.scrollTop=30;\
             JSON.stringify([scroller.offsetWidth,before-target.getBoundingClientRect().top])",
        )
        .unwrap(),
        "[200,30]"
    );
}

#[test]
fn legacy_layout_modes_do_not_allocate_realtime_state_or_refresh_after_writes() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = new_parsed_test_vm(
            "https://legacy-layout.test/",
            "<!doctype html><div id=target style='width:100px'></div>",
        );
        vm.set_layout_policy(policy);
        assert!(
            !vm._context_host
                .borrow()
                .realtime_layout_allocated_for_test()
        );
        vm.eval("target.offsetWidth").unwrap();
        let before = vm.layout_pass_observability_for_test().1;
        let width = vm
            .eval("target.style.width='200px';String(target.offsetWidth)")
            .unwrap();
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
        if policy == LayoutPolicy::OnDemand {
            assert_eq!(width, "100");
        }
        assert!(
            !vm._context_host
                .borrow()
                .realtime_layout_allocated_for_test()
        );
    }
}
