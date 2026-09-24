use super::*;
use crate::RendererInputDispatchOutcome;

fn mouse(
    vm: &mut StandaloneScriptVmHarness,
    event: &str,
) -> anyhow::Result<RendererInputDispatchOutcome> {
    vm.dispatch_mouse_event_at_point(
        60.0,
        60.0,
        event,
        0,
        Some(i32::from(event == "mousedown")),
        0.0,
        0.0,
    )
}

#[test]
fn mouse_input_rejects_missing_snapshot_without_layout_or_input_state_changes() {
    let mut vm = new_parsed_test_vm(
        "https://mouse-snapshot.test/",
        "<!doctype html><button id=target style='position:absolute;left:20px;top:20px;width:100px;height:100px'>go</button>",
    );
    vm.eval("window.events=[];for(const type of ['mousemove','mousedown','mouseup','click','wheel'])document.addEventListener(type,e=>events.push(type))").unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    for event in ["mousemove", "mousedown", "mouseup", "wheel"] {
        let error = mouse(&mut vm, event).unwrap_err();
        assert_eq!(
            error.downcast_ref::<moli_layout::LayoutError>(),
            Some(&moli_layout::LayoutError::NoLayoutSnapshot)
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
        assert_eq!(vm.pressed_mouse_buttons, 0);
        assert!(vm.pending_mouse_press.is_none());
    }
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), "[]");
    assert_eq!(
        vm.eval("String(target.getBoundingClientRect().width)")
            .unwrap(),
        "0"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    refresh_layout_for_test(&mut vm);
    let prepared = vm.layout_pass_observability_for_test().1;
    // A rejected down must not leave a press that a subsequent up can activate.
    mouse(&mut vm, "mouseup").unwrap();
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), r#"["mouseup"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, prepared);
}

#[test]
fn mouse_input_rejects_a_snapshot_from_a_replaced_document() {
    let mut vm = new_parsed_test_vm(
        "https://mouse-snapshot.test/",
        "<!doctype html><button>old</button>",
    );
    refresh_layout_for_test(&mut vm);
    vm.eval("document.open();document.write('<button>new</button>');document.close()")
        .unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    let error = mouse(&mut vm, "mousedown").unwrap_err();
    assert_eq!(
        error.downcast_ref::<moli_layout::LayoutError>(),
        Some(&moli_layout::LayoutError::NoLayoutSnapshot)
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    assert_eq!(vm.pressed_mouse_buttons, 0);
}

#[test]
fn mouse_click_consumes_snapshot_through_hover_focus_and_dom_mutation() {
    for markup in [
        "<button id=target>go</button>",
        "<input id=target>",
        "<div id=target style='overflow:auto'><div style='height:1000px;pointer-events:none'>scroll</div></div>",
        "<form onsubmit='event.preventDefault()'><input id=target type=image></form>",
        "<a id=target href='#destination'>go</a><div id=destination style='position:absolute;top:2000px'>destination</div>",
    ] {
        let mut vm = new_parsed_test_vm(
            "https://mouse-snapshot.test/",
            &format!(
                "<!doctype html><style>#target{{position:absolute;left:20px;top:20px;width:100px;height:100px}}#target:hover{{left:300px}}</style>{markup}"
            ),
        );
        vm.eval(
            r#"window.events=[];
            for(const type of ['mousemove','mousedown','mouseup','click'])
                document.addEventListener(type,e=>events.push([type,e.target.id]));
            target.addEventListener('mousedown',()=>{
                const veil=document.createElement('div');
                veil.style.cssText='position:fixed;inset:0;z-index:999';
                document.body.appendChild(veil);
            });
            "#,
        )
        .unwrap();
        refresh_layout_for_test(&mut vm);
        let before = vm.layout_pass_observability_for_test().1;
        for event in ["mousemove", "mousedown", "mouseup"] {
            mouse(&mut vm, event).unwrap();
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                before,
                "{markup}: {event}"
            );
        }
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"[["mousemove","target"],["mousedown","target"],["mouseup","target"],["click","target"]]"#,
            "{markup}"
        );
        assert!(
            vm.layout_snapshot_cache_observability_for_test()
                .3
                .is_some()
        );
    }
}

#[test]
fn mouse_and_geometry_reuse_the_snapshot_until_fresh_paint() {
    let mut vm = new_parsed_test_vm(
        "https://mouse-snapshot.test/",
        r#"<!doctype html><style>body{min-height:100px}</style>
        <button id=target style='position:absolute;left:20px;top:20px;width:100px;height:100px'>go</button>"#,
    );
    refresh_layout_for_test(&mut vm);
    vm.eval(
        "window.hits=[];document.onmousemove=e=>hits.push(e.target.id);target.style.left='300px'",
    )
    .unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    mouse(&mut vm, "mousemove").unwrap();
    assert_eq!(vm.eval("JSON.stringify(hits)").unwrap(), r#"["target"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    // DOM and hover changes do not invalidate ordinary geometry reads.
    assert_eq!(
        vm.eval("String(target.getBoundingClientRect().left)")
            .unwrap(),
        "20"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(1920, 1080, 1.0))
        .unwrap()
        .unwrap();
    assert_eq!(
        vm.eval("String(target.getBoundingClientRect().left)")
            .unwrap(),
        "300"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    mouse(&mut vm, "mousemove").unwrap();
    assert_eq!(vm.eval("JSON.stringify(hits)").unwrap(), r#"["target",""]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn mouse_handler_geometry_reads_reuse_the_published_snapshot() {
    let mut vm = new_parsed_test_vm(
        "https://mouse-snapshot.test/",
        r#"<!doctype html>
        <button id=target style='position:absolute;left:20px;top:20px;width:100px;height:100px'>go</button>"#,
    );
    refresh_layout_for_test(&mut vm);
    vm.eval("target.onmousemove=()=>{target.style.left='300px';window.observedLeft=target.getBoundingClientRect().left}").unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    mouse(&mut vm, "mousemove").unwrap();
    assert_eq!(vm.eval("String(observedLeft)").unwrap(), "20");
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
}

#[test]
fn mouse_wheel_updates_scroll_without_refreshing_the_rendered_world() {
    let mut vm = new_parsed_test_vm(
        "https://mouse-snapshot.test/",
        r#"<!doctype html>
        <div id=target style='position:absolute;left:20px;top:20px;width:100px;height:100px;overflow:auto'>
            <div style='height:1000px'>content</div>
        </div>"#,
    );
    refresh_layout_for_test(&mut vm);
    let before = vm.layout_pass_observability_for_test().1;
    for _ in 0..3 {
        vm.dispatch_mouse_event_at_point(60.0, 60.0, "wheel", -1, Some(0), 0.0, 50.0)
            .unwrap();
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
    }
    // Scroll metrics and bounds still come from the published snapshot.
    assert_eq!(vm.eval("String(target.scrollTop)").unwrap(), "0");
    vm.eval("target.getBoundingClientRect()").unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(1920, 1080, 1.0))
        .unwrap()
        .unwrap();
    assert_eq!(vm.eval("String(target.scrollTop)").unwrap(), "150");
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
    vm.eval("target.getBoundingClientRect()").unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}
