use super::*;

fn viewport_input_vm() -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://viewport-input.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 800,
        screen_height: 600,
        screen_avail_width: 800,
        screen_avail_height: 600,
    }))
    .unwrap();
    vm.force_fresh_layout_reads_for_test();
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.head) document.documentElement.appendChild(document.createElement('head'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        document.documentElement.style.cssText = 'margin:0;padding:0;height:0';
        document.body.style.cssText = 'margin:0;padding:0;height:0';
        globalThis.viewportClicks = [];
        globalThis.recordViewportClicks = (targetWindow, realm) => {
            targetWindow.addEventListener('click', event => viewportClicks.push([
                realm, event.target.nodeName, event.clientX, event.clientY, event.isTrusted
            ]));
        };
        recordViewportClicks(window, 'parent');
        "#,
    )
    .unwrap();
    vm
}

fn click_viewport(vm: &mut StandaloneScriptVmHarness, x: f64, y: f64) {
    for (event, button, buttons) in [("mousemove", -1, 0), ("mousedown", 0, 1), ("mouseup", 0, 0)] {
        vm.dispatch_mouse_event_at_point(x, y, event, button, Some(buttons), 0.0, 0.0)
            .unwrap();
    }
}

#[test]
fn viewport_root_input_reaches_empty_and_excluded_root_boxes() {
    for setup in [
        "",
        "document.documentElement.style.pointerEvents = 'none'",
        "document.documentElement.style.visibility = 'hidden'",
        "document.documentElement.inert = true",
    ] {
        let mut vm = viewport_input_vm();
        vm.eval(setup).unwrap();
        click_viewport(&mut vm, 80.0, 80.0);
        assert_eq!(
            vm.eval("JSON.stringify(viewportClicks)").unwrap(),
            r#"[["parent","HTML",80,80,true]]"#,
            "root setup: {setup}"
        );
        vm.eval("viewportClicks.length = 0").unwrap();
        for (x, y) in [(-1.0, 80.0), (800.0, 80.0), (80.0, 600.0)] {
            click_viewport(&mut vm, x, y);
        }
        assert_eq!(vm.eval("viewportClicks.length").unwrap(), "0");
    }
}

#[test]
fn viewport_root_input_in_iframes_preserves_document_boundaries_and_coordinates() {
    let cases = [
        ("", 80.0, 80.0, r#"[["child","HTML",36,36,true]]"#),
        (
            "child.documentElement.style.pointerEvents = 'none'",
            80.0,
            80.0,
            r#"[["child","HTML",36,36,true]]"#,
        ),
        (
            "child.documentElement.inert = true",
            80.0,
            80.0,
            r#"[["child","HTML",36,36,true]]"#,
        ),
        (
            "child.documentElement.style.visibility = 'hidden'",
            80.0,
            80.0,
            r#"[["child","HTML",36,36,true]]"#,
        ),
        (
            "child.documentElement.style.display = 'none'",
            80.0,
            80.0,
            "[]",
        ),
        ("child.documentElement.remove()", 80.0, 80.0, "[]"),
        ("", 42.0, 42.0, r#"[["parent","IFRAME",42,42,true]]"#),
        (
            "frame.style.cssText += ';transform:scale(2);transform-origin:0 0'",
            80.0,
            80.0,
            r#"[["child","HTML",16,16,true]]"#,
        ),
        (
            "frame.style.pointerEvents = 'none'",
            80.0,
            80.0,
            r#"[["parent","HTML",80,80,true]]"#,
        ),
        (
            "frame.inert = true",
            80.0,
            80.0,
            r#"[["parent","HTML",80,80,true]]"#,
        ),
        (
            "const overlay = document.createElement('div'); overlay.style.cssText = 'position:fixed;left:40px;top:40px;width:220px;height:100px'; document.body.appendChild(overlay)",
            80.0,
            80.0,
            r#"[["parent","DIV",80,80,true]]"#,
        ),
        (
            r#"
            const nested = child.createElement('iframe');
            nested.style.cssText = 'position:fixed;left:10px;top:10px;width:100px;height:30px;border:2px solid black';
            child.body.appendChild(nested);
            nested.contentDocument.documentElement.style.cssText = 'margin:0;height:0';
            nested.contentDocument.body.style.cssText = 'margin:0;height:0';
            recordViewportClicks(nested.contentWindow, 'grandchild');
            "#,
            80.0,
            80.0,
            r#"[["grandchild","HTML",24,24,true]]"#,
        ),
    ];
    for (setup, x, y, expected) in cases {
        let mut vm = viewport_input_vm();
        vm.eval(
            r#"
            globalThis.frame = document.createElement('iframe');
            frame.style.cssText = 'position:fixed;left:40px;top:40px;width:200px;height:50px;border:4px solid black';
            document.body.appendChild(frame);
            globalThis.child = frame.contentDocument;
            child.documentElement.style.cssText = 'margin:0;padding:0;height:0';
            child.body.style.cssText = 'margin:0;padding:0;height:0';
            recordViewportClicks(frame.contentWindow, 'child');
            "#,
        )
        .unwrap();
        vm.eval(setup).unwrap();
        click_viewport(&mut vm, x, y);
        assert_eq!(
            vm.eval("JSON.stringify(viewportClicks)").unwrap(),
            expected,
            "frame setup: {setup}"
        );
        if expected == "[]" {
            let hit = vm
                .observable_deep_hit_test_for_current_document(
                    moli_layout::LayoutPoint::new(x as f32, y as f32),
                    false,
                )
                .unwrap()
                .expect("the child viewport still has a Document for inspection");
            let host = vm._context_host.borrow();
            assert!(host.dom_host().node(hit).unwrap().is_document());
            assert_ne!(hit, host.document_handle());
        }
    }
}

#[test]
fn viewport_root_input_mock_fallback_requires_a_generated_box() {
    let mut vm = viewport_input_vm();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    vm.eval("document.body.remove(); document.documentElement.inert = true")
        .unwrap();
    click_viewport(&mut vm, 80.0, 80.0);
    assert_eq!(
        vm.eval("JSON.stringify(viewportClicks)").unwrap(),
        r#"[["parent","HTML",80,80,true]]"#
    );
    vm.eval("viewportClicks.length = 0; document.documentElement.style.display = 'none'")
        .unwrap();
    click_viewport(&mut vm, 80.0, 80.0);
    assert_eq!(vm.eval("viewportClicks.length").unwrap(), "0");
}

#[test]
fn viewport_root_input_wheel_keeps_child_target_and_chains_at_boundaries() {
    for (setup, target, parent_scroll, child_scroll) in [
        ("", "HTML", 120, 0),
        ("child.documentElement.inert = true", "HTML", 120, 0),
        (
            "child.documentElement.style.display = 'none'",
            "#document",
            120,
            0,
        ),
        ("child.documentElement.remove()", "#document", 120, 0),
        (
            "child.documentElement.style.overscrollBehavior = 'contain'",
            "HTML",
            0,
            0,
        ),
        (
            "child.documentElement.style.overscrollBehavior = 'none'",
            "HTML",
            0,
            0,
        ),
        (
            "child.documentElement.style.height = 'auto'; child.body.style.height = '100px'; frame.contentWindow.scrollTo(0, 40)",
            "BODY",
            0,
            50,
        ),
    ] {
        let mut vm = viewport_input_vm();
        vm.eval(
            r#"
            document.body.style.height = '2000px';
            globalThis.frame = document.createElement('iframe');
            frame.style.cssText = 'position:fixed;left:40px;top:40px;width:200px;height:50px;border:4px solid black';
            document.body.appendChild(frame);
            globalThis.child = frame.contentDocument;
            child.documentElement.style.cssText = 'margin:0;padding:0;height:0';
            child.body.style.cssText = 'margin:0;padding:0;height:0';
            globalThis.viewportWheels = [];
            for (const [targetWindow, name] of [[window, 'parent'], [frame.contentWindow, 'child']]) {
                targetWindow.addEventListener('wheel', event => viewportWheels.push([
                    name, event.target.nodeName, event.clientX, event.clientY,
                    event.deltaY, event.isTrusted
                ]));
            }
            "#,
        )
        .unwrap();
        vm.eval(setup).unwrap();
        vm.dispatch_mouse_event_at_point(80.0, 80.0, "wheel", -1, Some(0), 0.0, 120.0)
            .unwrap();
        let actual = vm
            .eval("JSON.stringify({events:viewportWheels,scrollY,childScrollY:frame.contentWindow.scrollY})")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            serde_json::json!({
                "events": [["child", target, 36, 36, 120, true]],
                "scrollY": parent_scroll,
                "childScrollY": child_scroll,
            }),
            "wheel setup: {setup}"
        );
    }
}
