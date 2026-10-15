use super::*;

async fn captured_child_vm(nested: bool, scaled: bool) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_html_test_vm("https://captured-pointer-coordinates.test/");
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r#"
        document.body.style.margin='0';
        document.body.innerHTML='<div id=outside style="position:absolute;left:0;top:0;width:50px;height:50px"></div>';
        globalThis.frame=document.createElement('iframe');
        frame.style.cssText='position:absolute;left:100px;top:60px;width:200px;height:120px;border:0';
        frame.srcdoc='<style>html,body{margin:0}</style><div id=captured style="position:absolute;left:20px;top:20px;width:40px;height:40px"></div>';
        document.body.append(frame);'ready'
    "#).unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "capture outer frame setup").await;
    if nested {
        vm.eval(r#"
            globalThis.inner=frame.contentDocument.createElement('iframe');
            inner.style.cssText='position:absolute;left:10px;top:10px;width:100px;height:80px;border:0';
            inner.srcdoc='<style>html,body{margin:0}</style><div id=captured style="position:absolute;left:20px;top:20px;width:40px;height:40px"></div>';
            frame.contentDocument.body.append(inner);'ready'
        "#).unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "capture inner frame setup")
            .await;
    }
    if scaled {
        vm.eval("frame.style.transform='scale(2)';frame.style.transformOrigin='0 0';'scaled'")
            .unwrap();
    }
    vm.eval(&format!(r#"
        globalThis.captureDocument={}.contentDocument;
        globalThis.captured=captureDocument.getElementById('captured');
        globalThis.events=[];globalThis.transitions=[];
        captured.addEventListener('pointerdown',event=>captured.setPointerCapture(event.pointerId));
        for(const type of ['gotpointercapture','lostpointercapture'])
            captured.addEventListener(type,event=>transitions.push([type,event.target.id]));
        for(const type of ['pointerdown','pointermove','pointerup'])
            captureDocument.addEventListener(type,event=>events.push([type,event.target.id,event.clientX,event.clientY]));
        outside.addEventListener('pointermove',event=>events.push([event.type,event.target.id,event.clientX,event.clientY]));
        'installed'
    "#,if nested { "inner" } else { "frame" })).unwrap();
    vm.publish_layout_for_test().unwrap();
    vm
}

fn dispatch(vm: &mut StandaloneScriptVmHarness, kind: &str, x: f64, y: f64, buttons: i32) {
    let result = vm
        .dispatch_mouse_event_at_point(
            x,
            y,
            kind,
            if kind == "mousemove" { -1 } else { 0 },
            Some(buttons),
            0.0,
            0.0,
        )
        .unwrap();
    assert!(result.handled);
}

#[tokio::test(flavor = "current_thread")]
async fn captured_child_pointer_uses_published_frame_coordinates_outside_its_bounds() {
    let mut vm = captured_child_vm(false, false).await;
    let builds = vm.layout_pass_observability_for_test().1;
    dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    vm.eval("frame.style.left='200px';frame.style.top='100px';'moved'")
        .unwrap();
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        builds,
        "OnDemand capture must keep the input's existing snapshot"
    );
    vm.publish_layout_for_test().unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
    dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
    dispatch(&mut vm, "mousemove", 20.0, 20.0, 0);
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["pointerdown","captured",30,30],["pointermove","captured",-80,-40],["pointermove","captured",-80,-40],["pointermove","captured",-180,-80],["pointerup","captured",-180,-80],["pointermove","outside",20,20]]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(transitions)").unwrap(),
        r#"[["gotpointercapture","captured"],["lostpointercapture","captured"]]"#
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
}

#[tokio::test(flavor = "current_thread")]
async fn captured_pointer_composes_nested_and_transformed_frame_coordinates() {
    for (nested, scaled, down, expected) in [
        (true, false, (140.0, 100.0), (-90, -50)),
        (false, true, (160.0, 120.0), (-40, -20)),
        (true, true, (180.0, 140.0), (-50, -30)),
    ] {
        let mut vm = captured_child_vm(nested, scaled).await;
        let builds = vm.layout_pass_observability_for_test().1;
        dispatch(&mut vm, "mousedown", down.0, down.1, 1);
        dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
        dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
        let facts: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
        assert_eq!(
            facts,
            serde_json::json!([
                ["pointerdown", "captured", 30, 30],
                ["pointermove", "captured", expected.0, expected.1],
                ["pointerup", "captured", expected.0, expected.1],
            ]),
            "nested={nested}, scaled={scaled}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_pointer_remains_deliverable_without_an_invertible_frame_mapping() {
    for style in ["display:none", "transform:scale(0)"] {
        let mut vm = captured_child_vm(false, false).await;
        dispatch(&mut vm, "mousedown", 130.0, 90.0, 1);
        dispatch(&mut vm, "mousemove", 131.0, 91.0, 1);
        vm.eval(&format!("frame.style.cssText+=';{style}';'unprojectable'"))
            .unwrap();
        vm.publish_layout_for_test().unwrap();
        let builds = vm.layout_pass_observability_for_test().1;
        dispatch(&mut vm, "mousemove", 20.0, 20.0, 1);
        dispatch(&mut vm, "mouseup", 20.0, 20.0, 0);
        let facts: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
        for row in &facts.as_array().unwrap()[2..] {
            assert_eq!(row[1], "captured");
            assert!(row[2].as_f64().is_some_and(f64::is_finite));
            assert!(row[3].as_f64().is_some_and(f64::is_finite));
        }
        assert_eq!(facts.as_array().unwrap().len(), 4);
        assert_eq!(
            vm.eval("JSON.stringify(transitions)").unwrap(),
            r#"[["gotpointercapture","captured"],["lostpointercapture","captured"]]"#
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    }
}
