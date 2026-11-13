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
fn realtime_frame_viewport_reads_refresh_ancestors_and_use_the_receiver_realm() {
    let mut vm = new_parsed_test_vm("https://realtime-frame.test/", "<!doctype html><body>");
    vm.set_layout_policy(LayoutPolicy::Realtime);
    // Run the identical synchronous fixture against the release CDP entry and
    // Chromium. Neither screenshots nor lifecycle callbacks can refresh a read.
    let fixture = include_str!("../../../tests/fixtures/realtime-frame-viewport-reads.js");
    let result = vm.eval(&format!("JSON.stringify({fixture})")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn realtime_frame_viewport_reads_share_current_layout_and_propagate_failures() {
    for (read, expected) in [
        ("String(query.matches)", "true"),
        ("String(child.matchMedia('(width:400px)').matches)", "true"),
        ("String(viewport.width)", "400"),
        ("String(viewport.height)", "120"),
    ] {
        let mut vm = frame_fixture(LayoutPolicy::Realtime, 320, 100);
        vm.eval("globalThis.viewport=child.visualViewport;globalThis.query=child.matchMedia('(width:400px)');frame.offsetWidth")
            .unwrap();
        let before = vm.layout_pass_observability_for_test().1;
        vm.eval("frame.style.width='400px';frame.style.height='120px'")
            .unwrap();
        vm._context_host
            .borrow()
            .simulate_active_layout_pass_for_test(true);
        let result = vm.eval(read);
        vm._context_host
            .borrow()
            .simulate_active_layout_pass_for_test(false);
        let error = result.unwrap_err();
        assert!(
            error.to_string().contains("Layout failed:"),
            "{read}: {error}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
        assert_eq!(vm.eval(read).unwrap(), expected, "{read}");
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
        assert_eq!(vm.eval(read).unwrap(), expected, "{read}");
        assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    }
}

#[test]
fn viewport_reads_do_not_add_layout_to_legacy_modes_or_top_level_queries() {
    for policy in [
        LayoutPolicy::Mock,
        LayoutPolicy::OnDemand,
        LayoutPolicy::Realtime,
    ] {
        let mut vm = frame_fixture(policy, 320, 100);
        vm.eval("globalThis.viewport=child.visualViewport;globalThis.query=child.matchMedia('(width:400px)');frame.offsetWidth")
            .unwrap();
        let before = vm.layout_pass_observability_for_test().1;
        vm.eval("frame.style.width='400px';document.body.style.width='800px'")
            .unwrap();
        // Root viewport sizes do not depend on a descendant's box. Reading
        // them or a root media query must not unnecessarily flush the tree.
        vm.eval("visualViewport.width;visualViewport.height;matchMedia('(min-width:1px)').matches")
            .unwrap();
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            before,
            "{policy:?}"
        );
        if policy != LayoutPolicy::Realtime {
            vm.eval("viewport.width;viewport.height;query.matches;child.matchMedia('(width:400px)').matches")
                .unwrap();
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                before,
                "{policy:?}"
            );
            assert!(
                !vm._context_host
                    .borrow()
                    .realtime_layout_allocated_for_test()
            );
        }
    }
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

#[test]
fn realtime_hidden_frame_cssom_preserves_computed_values_and_retained_declarations() {
    let mut vm = frame_fixture(LayoutPolicy::Realtime, 320, 100);
    vm.eval(
        "percent.style.cssText='width:50%;height:50%;margin-left:10%;padding-top:5%;position:relative;left:10%;font-size:20px;line-height:2';\
         child.document.body.style.height='100px';\
         globalThis.retained=child.getComputedStyle(percent);\
         globalThis.properties=['width','height','margin-left','left','line-height'];\
         globalThis.readStyle=()=>JSON.stringify([\
           properties.map(p=>retained.getPropertyValue(p)),\
           properties.map(p=>getComputedStyle(percent).getPropertyValue(p))])",
    ).unwrap();
    let visible = "[[\"160px\",\"50px\",\"32px\",\"32px\",\"40px\"],[\"160px\",\"50px\",\"32px\",\"32px\",\"40px\"]]";
    assert_eq!(vm.eval("readStyle()").unwrap(), visible);
    vm.eval("frame.style.display='none'").unwrap();
    let hidden =
        "[[\"50%\",\"50%\",\"10%\",\"10%\",\"40px\"],[\"50%\",\"50%\",\"10%\",\"10%\",\"40px\"]]";
    assert_eq!(vm.eval("readStyle()").unwrap(), hidden);
    assert_eq!(vm.eval("getComputedStyle(fixed).width").unwrap(), "100px");
    assert_eq!(vm.eval("retained.paddingTop").unwrap(), "5%");
    assert_eq!(vm.eval("String(percent.offsetWidth)").unwrap(), "0");
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval("readStyle()").unwrap(), hidden);
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.eval("frame.style.display='block'").unwrap();
    assert_eq!(vm.eval("readStyle()").unwrap(), visible);
    vm.eval("document.body.style.display='none'").unwrap();
    assert_eq!(vm.eval("readStyle()").unwrap(), hidden);
    vm.eval("frame.remove()").unwrap();
    assert_eq!(
        vm.eval("readStyle()").unwrap(),
        "[[\"\",\"\",\"\",\"\",\"\"],[\"\",\"\",\"\",\"\",\"\"]]"
    );
}

#[test]
fn realtime_child_window_reads_refresh_then_preserve_hidden_viewports() {
    let mut vm = frame_fixture(LayoutPolicy::Realtime, 320, 100);
    vm.eval("frame.style.display='none'").unwrap();
    let read = "JSON.stringify([child.innerWidth,child.innerHeight])";
    assert_eq!(vm.eval(read).unwrap(), "[0,0]");
    vm.eval("frame.style.display='block'").unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[320,100]");
    vm.eval("frame.style.display='none';frame.style.width='360px';frame.style.height='120px'")
        .unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[320,100]");
    vm.eval("frame.style.display='block'").unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[360,120]");
    vm.eval("document.body.style.display='none';frame.style.width='400px'")
        .unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[360,120]");
    vm.eval("frame.remove()").unwrap();
    assert_eq!(vm.eval(read).unwrap(), "[0,0]");
}

#[test]
fn legacy_hidden_frame_cssom_keeps_its_existing_empty_declaration() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = frame_fixture(policy, 320, 100);
        vm.eval("globalThis.retained=getComputedStyle(fixed);frame.style.display='none'")
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([retained.width,getComputedStyle(fixed).width])")
                .unwrap(),
            "[\"\",\"\"]"
        );
    }
}

#[test]
fn realtime_required_frame_depth_fails_instead_of_publishing_missing_geometry() {
    let mut vm = frame_fixture(LayoutPolicy::Realtime, 320, 100);
    vm.eval(
        "let parent=child;\
         for(let i=0;i<33;i++){\
           const nested=parent.document.createElement('iframe');\
           nested.style.cssText='width:320px;height:100px;border:0';\
           parent.document.body.append(nested);parent=nested.contentWindow;\
           parent.document.open();parent.document.write('<!doctype html><body><div id=deep style=width:100px></div>');parent.document.close();\
         }\
         globalThis.deep=parent.document.getElementById('deep')",
    ).unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    let error = vm.eval("deep.offsetWidth").unwrap_err();
    assert!(error.to_string().contains("required Document"), "{error}");
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.eval("frame.style.display='none'").unwrap();
    assert_eq!(vm.eval("String(deep.offsetWidth)").unwrap(), "0");
}
