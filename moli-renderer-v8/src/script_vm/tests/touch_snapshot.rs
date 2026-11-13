use super::*;

const FIXTURE: &str = "<!doctype html><style>body{margin:0;min-height:200px}#target{position:absolute;left:20px;top:20px;width:100px;height:100px;margin:0}</style><input id=target type=checkbox>";

fn tap(vm: &mut StandaloneScriptVmHarness, x: f64) {
    vm.dispatch_touch_event_at_point(x, 60.0, "touchstart", false)
        .unwrap();
    vm.dispatch_touch_event_at_points(&[], "touchend", false)
        .unwrap();
}

#[test]
fn touch_input_initializes_layout_once_through_hover_and_focus() {
    let mut vm = new_parsed_test_vm("https://touch-snapshot.test/", FIXTURE);
    vm.eval("window.events=[];for(const type of ['mousemove','mousedown','mouseup','click'])document.addEventListener(type,e=>events.push([type,e.target.id]));window.tapNode=document.getElementById('target')").unwrap();
    let passes = vm.layout_pass_observability_for_test().1;
    let publishes = vm.layout_snapshot_cache_observability_for_test().2;
    tap(&mut vm, 60.0);
    assert_eq!(vm.layout_pass_observability_for_test().1, passes + 1);
    assert_eq!(
        vm.layout_snapshot_cache_observability_for_test().2,
        publishes + 1,
        "hover and focus must not republish the cold-start layout"
    );
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["mousemove","target"],["mousedown","target"],["mouseup","target"],["click","target"]]"#
    );
    assert_eq!(vm.eval("String(tapNode.checked)").unwrap(), "true");
    tap(&mut vm, 60.0);
    assert_eq!(vm.layout_pass_observability_for_test().1, passes + 1);
    assert_eq!(
        vm.layout_snapshot_cache_observability_for_test().2,
        publishes + 1
    );
    assert_eq!(vm.eval("String(tapNode.checked)").unwrap(), "false");
}

#[test]
fn touch_callbacks_reuse_geometry_until_an_explicit_paint() {
    for event in ["touchend", "mousemove", "mousedown", "focus"] {
        let mut vm = new_parsed_test_vm("https://touch-snapshot.test/", FIXTURE);
        vm.eval(&format!(
            "window.tapNode=document.getElementById('target');window.clicks=0;tapNode.onclick=()=>clicks++;tapNode.addEventListener('{event}',()=>{{tapNode.style.left='300px';window.callbackLeft=tapNode.getBoundingClientRect().left}},{{once:true}})"
        )).unwrap();
        refresh_layout_for_test(&mut vm);
        let passes = vm.layout_pass_observability_for_test().1;
        let publishes = vm.layout_snapshot_cache_observability_for_test().2;
        tap(&mut vm, 60.0);
        assert_eq!(vm.layout_pass_observability_for_test().1, passes, "{event}");
        assert_eq!(
            vm.layout_snapshot_cache_observability_for_test().2,
            publishes,
            "{event}"
        );
        assert_eq!(vm.eval("String(callbackLeft)").unwrap(), "20", "{event}");
        assert_eq!(
            vm.eval(
                "JSON.stringify([clicks,tapNode.style.left,tapNode.getBoundingClientRect().left])"
            )
            .unwrap(),
            r#"[1,"300px",20]"#,
            "{event}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);

        vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
            .unwrap()
            .unwrap();
        assert_eq!(
            vm.layout_snapshot_cache_observability_for_test().2,
            publishes + 1
        );
        assert_eq!(
            vm.eval("String(tapNode.getBoundingClientRect().left)")
                .unwrap(),
            "300"
        );
        tap(&mut vm, 60.0);
        assert_eq!(vm.eval("String(clicks)").unwrap(), "1", "{event}");
        tap(&mut vm, 340.0);
        assert_eq!(vm.eval("String(clicks)").unwrap(), "2", "{event}");
        assert_eq!(vm.layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            vm.layout_snapshot_cache_observability_for_test().2,
            publishes + 1
        );
    }
}

#[test]
fn touch_frozen_geometry_revalidates_live_activation_targets() {
    for event in ["touchend", "mousemove", "mousedown", "focus", "mouseup"] {
        for mutation in ["remove", "adopt", "disable", "replace-root"] {
            let mut vm = new_parsed_test_vm("https://touch-snapshot.test/", FIXTURE);
            let change = match mutation {
                "remove" => "tapNode.remove()",
                "adopt" => "destination.contentDocument.body.appendChild(tapNode)",
                "disable" => "tapNode.disabled=true",
                _ => "document.documentElement.replaceWith(document.createElement('html'))",
            };
            vm.eval(&format!(
                "window.tapNode=document.getElementById('target');window.clicks=0;tapNode.onclick=()=>clicks++;const destination=document.createElement('iframe');destination.style.cssText='position:absolute;left:400px;top:0';document.body.appendChild(destination);tapNode.addEventListener('{event}',()=>{{{change}}},{{once:true}})"
            )).unwrap();
            refresh_layout_for_test(&mut vm);
            let passes = vm.layout_pass_observability_for_test().1;
            let publishes = vm.layout_snapshot_cache_observability_for_test().2;
            tap(&mut vm, 60.0);
            assert_eq!(
                vm.eval("JSON.stringify([clicks,tapNode.checked])").unwrap(),
                "[0,false]",
                "{event}: {mutation}"
            );
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                passes,
                "{event}: {mutation}"
            );
            assert_eq!(
                vm.layout_snapshot_cache_observability_for_test().2,
                publishes,
                "{event}: {mutation}"
            );
        }
    }
}
