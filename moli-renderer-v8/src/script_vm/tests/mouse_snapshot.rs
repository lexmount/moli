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
            <div id=content style='width:1000px;height:1000px'>
                <div id=marker style='width:80px;height:80px'></div>
            </div>
        </div>"#,
    );
    vm.eval("window.hits=[];document.onmousemove=e=>hits.push(e.target.id)")
        .unwrap();
    refresh_layout_for_test(&mut vm);
    let before = vm.layout_pass_observability_for_test().1;
    for step in 1..=3 {
        vm.dispatch_mouse_event_at_point(60.0, 60.0, "wheel", -1, Some(0), 30.0, 50.0)
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([target.scrollLeft,target.scrollTop])")
                .unwrap(),
            format!("[{},{}]", step * 30, step * 50)
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
    }
    // Live scroll readback must not move the published geometry or hit targets.
    let marker_position =
        "JSON.stringify([marker.getBoundingClientRect().left,marker.getBoundingClientRect().top])";
    assert_eq!(vm.eval(marker_position).unwrap(), "[20,20]");
    mouse(&mut vm, "mousemove").unwrap();
    assert_eq!(vm.eval("JSON.stringify(hits)").unwrap(), r#"["marker"]"#);
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .unwrap()
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([target.scrollLeft,target.scrollTop])")
            .unwrap(),
        "[90,150]"
    );
    assert_eq!(vm.eval(marker_position).unwrap(), "[-70,-130]");
    mouse(&mut vm, "mousemove").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(hits)").unwrap(),
        r#"["marker","content"]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, before + 1);
}

#[test]
fn element_scroll_readback_accumulates_live_offsets_with_frozen_bounds() {
    for in_frame in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://scroll-readback.test/",
            r#"<!doctype html>
            <div id=scroller style='width:100px;height:100px;overflow:auto'>
                <div style='width:400px;height:500px'></div>
            </div><iframe id=frame></iframe>"#,
        );
        vm.eval(if in_frame {
            "frame.contentDocument.body.innerHTML=scroller.outerHTML;window.subject=frame.contentDocument.getElementById('scroller')"
        } else {
            "window.subject=scroller"
        })
        .unwrap();
        let offsets = "JSON.stringify([subject.scrollLeft,subject.scrollTop])";
        let cold = vm.layout_pass_observability_for_test().1;
        assert_eq!(vm.eval(offsets).unwrap(), "[0,0]");
        assert_eq!(vm.layout_pass_observability_for_test().1, cold);
        // A screenshot publishes the iframe projection as well as the root.
        publish_layout_for_test(&mut vm);
        let before = vm.layout_pass_observability_for_test().1;
        let geometry = "JSON.stringify([subject.firstElementChild.getBoundingClientRect().left,subject.firstElementChild.getBoundingClientRect().top,subject.clientWidth,subject.clientHeight,subject.scrollWidth,subject.scrollHeight])";
        let frozen = vm.eval(geometry).unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([subject.scrollWidth,subject.scrollHeight])")
                .unwrap(),
            "[400,500]"
        );
        assert_eq!(
            vm.eval(
                r#"subject.scrollLeft += 10; subject.scrollTop += 10;
                subject.scrollLeft += 10; subject.scrollTop += 10;
                JSON.stringify([subject.scrollLeft,subject.scrollTop])"#,
            )
            .unwrap(),
            "[20,20]",
            "in_frame={in_frame}, frozen={frozen}"
        );
        vm.eval("subject.scrollBy({left:5,top:7});subject.scrollTo({top:40})")
            .unwrap();
        assert_eq!(vm.eval(offsets).unwrap(), "[25,40]");
        vm.eval("subject.scrollLeft=10.25;subject.scrollTop=20.5")
            .unwrap();
        assert_eq!(vm.eval(offsets).unwrap(), "[10.25,20.5]");
        // New content size cannot enlarge the range until a new publication.
        assert_eq!(
            vm.eval(
                r#"subject.firstElementChild.style.cssText='width:1000px;height:1000px';
                subject.scrollLeft=1e6;subject.scrollTop=1e6;
                JSON.stringify([subject.scrollLeft===subject.scrollWidth-subject.clientWidth,
                    subject.scrollTop===subject.scrollHeight-subject.clientHeight])"#,
            )
            .unwrap(),
            "[true,true]"
        );
        assert_eq!(vm.eval(geometry).unwrap(), frozen);
        vm.eval("subject.remove()").unwrap();
        assert_eq!(vm.eval(offsets).unwrap(), "[0,0]");
        assert_eq!(vm.layout_pass_observability_for_test().1, before);
    }
}

#[test]
fn document_scroll_readback_matches_live_window_offsets() {
    let mut vm = new_parsed_test_vm(
        "https://scroll-readback.test/",
        r#"<!doctype html><body style='width:2000px;height:2000px'>
        <iframe id=frame></iframe>"#,
    );
    vm.eval("frame.contentDocument.body.style.cssText='width:1000px;height:1000px'")
        .unwrap();
    publish_layout_for_test(&mut vm);
    let before = vm.layout_pass_observability_for_test().1;
    let geometry = "JSON.stringify([frame.getBoundingClientRect().top,frame.contentDocument.body.getBoundingClientRect().top])";
    let frozen = vm.eval(geometry).unwrap();
    vm.eval("scrollTo(30,50);frame.contentWindow.scrollTo(7,9)")
        .unwrap();
    let offsets = r#"JSON.stringify([window,frame.contentWindow].map(w=>[
        w.scrollX,w.scrollY,w.pageXOffset,w.pageYOffset,
        w.document.scrollingElement.scrollLeft,w.document.scrollingElement.scrollTop]))"#;
    assert_eq!(
        vm.eval(offsets).unwrap(),
        "[[30,50,30,50,30,50],[7,9,7,9,7,9]]"
    );
    vm.eval(
        "for(const w of [window,frame.contentWindow]){w.document.scrollingElement.scrollLeft+=10;w.document.scrollingElement.scrollTop+=20}",
    )
    .unwrap();
    assert_eq!(
        vm.eval(offsets).unwrap(),
        "[[40,70,40,70,40,70],[17,29,17,29,17,29]]"
    );
    vm.eval("frame.contentDocument.scrollingElement.scrollBy(3,4);frame.contentDocument.scrollingElement.scrollTo({top:35})")
        .unwrap();
    assert_eq!(
        vm.eval(offsets).unwrap(),
        "[[40,70,40,70,40,70],[20,35,20,35,20,35]]"
    );
    assert_eq!(vm.eval(geometry).unwrap(), frozen);
    assert_eq!(vm.layout_pass_observability_for_test().1, before);
}

#[test]
fn dom_mutations_preserve_scroll_without_refreshing_layout() {
    // Cover nodes above, crossing, and spanning the viewport. Their removal
    // must not estimate a scroll adjustment from the old frozen rectangle.
    for (top, height) in [(300, 200), (800, 200), (800, 2000)] {
        for (mutation, remaining_height) in [
            ("target.remove()", 0),
            ("target.replaceWith(replacement)", 1500),
            ("document.body.replaceChild(replacement,target)", 1500),
            ("document.createDocumentFragment().appendChild(target)", 0),
            (
                "const detached=document.createElement('div'); detached.appendChild(document.createElement('span')); detached.insertBefore(target,detached.firstChild)",
                0,
            ),
        ] {
            let mut vm = new_parsed_test_vm(
                "https://scroll-preservation.test/",
                &format!(
                    r#"<!doctype html><body style="margin:0;width:3000px">
                    <div style="height:{top}px">before</div>
                    <div id="target" style="height:{height}px">replace</div>
                    <div id="marker" style="height:5000px">kept</div>"#
                ),
            );
            refresh_layout_for_test(&mut vm);
            vm.eval(
                r#"scrollTo(120,900);
                window.replacement=document.createDocumentFragment();
                const added=document.createElement('div'); added.style.height='1500px';
                replacement.appendChild(added);"#,
            )
            .unwrap();
            refresh_layout_for_test(&mut vm);
            let layouts = vm.layout_pass_observability_for_test().1;
            let offsets = "JSON.stringify([scrollX,scrollY,document.scrollingElement.scrollLeft,document.scrollingElement.scrollTop])";
            let marker_top = "String(marker.getBoundingClientRect().top)";
            let frozen_top = (top + height - 900).to_string();
            assert_eq!(vm.eval(offsets).unwrap(), "[120,900,120,900]");
            assert_eq!(vm.eval(marker_top).unwrap(), frozen_top);

            vm.eval(mutation).unwrap();
            assert_eq!(
                vm.eval(offsets).unwrap(),
                "[120,900,120,900]",
                "top={top}, height={height}, mutation={mutation}"
            );
            assert_eq!(vm.eval(marker_top).unwrap(), frozen_top);
            assert_eq!(vm.layout_pass_observability_for_test().1, layouts);

            // Publishing the new layout changes geometry without introducing
            // automatic scroll anchoring at publication time either.
            refresh_layout_for_test(&mut vm);
            assert_eq!(vm.eval(offsets).unwrap(), "[120,900,120,900]");
            assert_eq!(
                vm.eval(marker_top).unwrap(),
                (top + remaining_height - 900).to_string()
            );
        }
    }
}
