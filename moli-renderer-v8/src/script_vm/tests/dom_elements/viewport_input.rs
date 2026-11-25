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
        ..Default::default()
    }))
    .unwrap();
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
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
fn viewport_root_input_keeps_rootless_iframe_document_for_inspection() {
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
    ).unwrap();
    click_viewport(&mut vm, 80.0, 80.0);
    assert_eq!(
        vm.eval("JSON.stringify(viewportClicks)").unwrap(),
        r#"[["child","HTML",36,36,true]]"#
    );

    vm.eval("viewportClicks.length = 0; child.documentElement.remove()")
        .unwrap();
    click_viewport(&mut vm, 80.0, 80.0);
    assert_eq!(vm.eval("viewportClicks.length").unwrap(), "0");
    let hit = vm
        .observable_deep_hit_test_for_current_document(
            moli_layout::LayoutPoint::new(80.0, 80.0),
            crate::native_bridge::element::InputHitTestPolicy::INTERACTIVE,
        )
        .unwrap()
        .expect("the child viewport still has a Document for inspection");
    let host = vm._context_host.borrow();
    assert!(host.dom_host().node(hit).unwrap().is_document());
    assert_ne!(hit, host.document_handle());
}
