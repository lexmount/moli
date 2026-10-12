use super::*;
use moli_page_types::LayoutPolicy;

fn frame_fixture(policy: LayoutPolicy, width: u32, height: u32) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(
        "https://realtime-frame.test/",
        &format!(
            "<!doctype html><body style=margin:0><iframe id=frame style='width:{width}px;height:{height}px;border:0'></iframe>",
        ),
    );
    vm.set_layout_policy(policy);
    vm.eval(
        "globalThis.child=frame.contentWindow;child.document.open();\
         child.document.write('<!doctype html><html style=overflow:hidden><body style=margin:0><div id=fixed style=width:100px;height:30px></div><div id=percent style=width:50%;height:20px></div>');\
         child.document.close();globalThis.fixed=child.document.getElementById('fixed');\
         globalThis.percent=child.document.getElementById('percent')",
    ).unwrap();
    vm
}

#[test]
fn realtime_layout_zero_sized_frame_preserves_fixed_and_percentage_metrics() {
    // Chromium 145, child root overflow:hidden: an empty viewport clips
    // painting/hit testing, while descendants still have layout boxes.
    for (width, height) in [(0, 0), (0, 100), (320, 0)] {
        let mut vm = frame_fixture(LayoutPolicy::Realtime, width, height);
        let before = vm.layout_pass_observability_for_test().1;
        let read = "JSON.stringify([fixed.offsetWidth,fixed.offsetHeight,\
            fixed.getBoundingClientRect().width,percent.offsetWidth,\
            child.innerWidth,child.innerHeight,getComputedStyle(percent).width,\
            child.document.elementFromPoint(1,1)])";
        let expected = format!(
            "[100,30,100,{},{width},{height},\"{}px\",null]",
            width / 2,
            width / 2
        );
        assert_eq!(
            vm.eval(read).unwrap(),
            expected,
            "viewport {width}x{height}"
        );
        assert_eq!(vm.eval(read).unwrap(), expected);
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        assert_eq!(
            vm.layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
    }
}

#[test]
fn realtime_layout_nested_zero_frame_coverage_survives_screen_capture() {
    let mut vm = frame_fixture(LayoutPolicy::Realtime, 320, 100);
    vm.eval(
        "const inner=child.document.createElement('iframe');\
         inner.style.cssText='width:0;height:0;border:0';child.document.body.append(inner);\
         globalThis.nested=inner.contentWindow;nested.document.open();\
         nested.document.write('<!doctype html><html style=overflow:hidden><body style=margin:0><div id=target style=width:100px;height:30px></div>');\
         nested.document.close();globalThis.target=nested.document.getElementById('target')",
    ).unwrap();
    let viewport = {
        let host = vm._context_host.borrow();
        host.layout_viewport_for_document(host.document_handle())
    };
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    vm.screenshot_layout_snapshot(viewport).unwrap().unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
    assert_eq!(vm.eval("String(target.offsetWidth)").unwrap(), "100");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 2);
}

#[test]
fn realtime_layout_zero_frame_resize_and_display_none_have_distinct_results() {
    let mut vm = frame_fixture(LayoutPolicy::Realtime, 0, 0);
    let read = "JSON.stringify([fixed.offsetWidth,percent.offsetWidth,getComputedStyle(fixed).width,getComputedStyle(percent).width])";
    assert_eq!(vm.eval(read).unwrap(), "[100,0,\"100px\",\"0px\"]");
    vm.eval("frame.style.width='320px';frame.style.height='100px'")
        .unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[100,160,\"100px\",\"160px\"]");
    vm.eval("frame.style.display='none'").unwrap();
    let hidden_geometry = "JSON.stringify([fixed.offsetWidth,percent.offsetWidth])";
    assert_eq!(vm.eval(hidden_geometry).unwrap(), "[0,0]");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(hidden_geometry).unwrap(), "[0,0]");
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.eval("frame.style.display='block'").unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[100,160,\"100px\",\"160px\"]");
}

#[test]
fn legacy_layout_keeps_its_existing_zero_frame_coverage() {
    let mut vm = frame_fixture(LayoutPolicy::OnDemand, 0, 0);
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval("String(fixed.offsetWidth)").unwrap(), "0");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    assert_eq!(vm.eval("String(fixed.offsetWidth)").unwrap(), "0");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}
