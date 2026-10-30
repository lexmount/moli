use super::*;
use crate::runtime::{
    RendererDragData, RendererInputDispatchOutcome, RendererPointerEventProperties,
};

const POINTER_DRAG_PAGE: &str = r#"<html><body style="margin:0">
<section id=outer style="position:absolute;left:0;top:0;width:300px;height:200px">
  <div id=parent style="width:200px;height:150px">
    <div id=source draggable=true style="width:120px;height:120px">
      <span id=leaf style="display:block;width:80px;height:80px"></span>
    </div>
  </div>
</section>
<aside id=capture style="position:absolute;left:400px;top:0;width:100px;height:100px"></aside>
</body></html>"#;

fn pointer_drag_mouse(
    vm: &mut StandaloneScriptVmHarness,
    name: &str,
    x: f64,
    y: f64,
) -> RendererInputDispatchOutcome {
    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        x,
        y,
        name,
        if name == "mousedown" { 0 } else { -1 },
        Some(1),
        0,
        0.0,
        0.0,
        RendererPointerEventProperties::default(),
        2,
        true,
    )
    .expect("native pointer drag input")
}

fn install_suppression_log(vm: &mut StandaloneScriptVmHarness, capture: Option<&str>) {
    vm.eval(&format!(
        r#"(() => {{
          window.__suppression=[];window.__started=false;
          const captureName={};
          const source=document.getElementById('source');
          const leaf=document.getElementById('leaf');
          if(captureName) leaf.addEventListener('pointerdown',event=>
            document.getElementById(captureName).setPointerCapture(event.pointerId));
          source.addEventListener('dragstart',event=>{{
            event.dataTransfer.setData('text/plain','payload');
            window.__started=true;
          }});
          for(const type of ['pointercancel','lostpointercapture','pointerout','pointerleave']) {{
            document.addEventListener(type,event=>{{
              if(!window.__started) return;
              window.__suppression.push([type,event.target.id||event.target.nodeName,
                event.relatedTarget===null,event.buttons,event.pressure,event.cancelable,
                event.bubbles,event.ctrlKey,
                captureName?document.getElementById(captureName).hasPointerCapture(event.pointerId):false]);
            }},true);
          }}
        }})()"#,
        serde_json::json!(capture),
    ))
    .expect("pointer suppression listeners");
}

fn suppression_log(vm: &mut StandaloneScriptVmHarness) -> serde_json::Value {
    serde_json::from_str(
        &vm.eval("JSON.stringify(window.__suppression)")
            .expect("pointer suppression log"),
    )
    .unwrap()
}

fn expected_suppression(target: &str, ancestors: &[&str], captured: bool) -> serde_json::Value {
    let mut expected = vec![serde_json::json!([
        "pointercancel",
        target,
        true,
        0,
        0,
        false,
        true,
        true,
        captured
    ])];
    if captured {
        expected.push(serde_json::json!([
            "lostpointercapture",
            target,
            true,
            0,
            0,
            false,
            true,
            true,
            false
        ]));
    }
    expected.push(serde_json::json!([
        "pointerout",
        target,
        true,
        0,
        0,
        true,
        true,
        true,
        false
    ]));
    for ancestor in ancestors {
        expected.push(serde_json::json!([
            "pointerleave",
            ancestor,
            true,
            0,
            0,
            false,
            false,
            true,
            false
        ]));
    }
    serde_json::Value::Array(expected)
}

#[test]
fn successful_native_drag_suppresses_the_effective_pointer_target_and_its_flat_ancestors() {
    for (capture, target, ancestors) in [
        (
            None,
            "leaf",
            vec![
                "leaf",
                "source",
                "parent",
                "outer",
                "BODY",
                "HTML",
                "#document",
            ],
        ),
        (
            Some("source"),
            "source",
            vec!["source", "parent", "outer", "BODY", "HTML", "#document"],
        ),
        (
            Some("parent"),
            "parent",
            vec!["parent", "outer", "BODY", "HTML", "#document"],
        ),
        (
            Some("capture"),
            "capture",
            vec!["capture", "BODY", "HTML", "#document"],
        ),
    ] {
        let mut vm =
            new_rendered_test_vm("https://drag-pointer-boundaries.test/", POINTER_DRAG_PAGE);
        install_suppression_log(&mut vm, capture);
        vm.publish_layout_for_test()
            .expect("publish pointer drag layout");
        pointer_drag_mouse(&mut vm, "mousedown", 20.0, 20.0);
        let outcome = pointer_drag_mouse(&mut vm, "mousemove", 40.0, 40.0);
        assert!(outcome.intercepted_drag.is_some(), "capture={capture:?}");
        assert_eq!(
            suppression_log(&mut vm),
            expected_suppression(target, &ancestors, capture.is_some()),
            "capture={capture:?}"
        );
        pointer_drag_mouse(&mut vm, "mousemove", 45.0, 45.0);
        assert_eq!(
            suppression_log(&mut vm),
            expected_suppression(target, &ancestors, capture.is_some()),
            "active drag does not repeat boundaries: capture={capture:?}"
        );
    }
}

#[test]
fn removing_the_pressed_child_during_dragstart_cancels_its_stream_without_boundary_events() {
    let mut vm = new_rendered_test_vm("https://drag-removed-press-child.test/", POINTER_DRAG_PAGE);
    install_suppression_log(&mut vm, None);
    vm.eval(
        r#"(() => {
      const leaf=document.getElementById('leaf');
      window.__removedLeafEvents=[];
      for(const type of ['pointercancel','pointerout','pointerleave'])
        leaf.addEventListener(type,event=>window.__removedLeafEvents.push(
          [type,event.target===leaf,event.buttons,event.pressure]));
      document.getElementById('source').addEventListener('dragstart',()=>leaf.remove());
    })()"#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    pointer_drag_mouse(&mut vm, "mousedown", 20.0, 20.0);
    let outcome = pointer_drag_mouse(&mut vm, "mousemove", 40.0, 40.0);
    assert!(outcome.intercepted_drag.is_some());
    assert_eq!(suppression_log(&mut vm), serde_json::json!([]));
    assert_eq!(
        vm.eval("JSON.stringify(window.__removedLeafEvents)")
            .unwrap(),
        r#"[["pointercancel",true,0,0]]"#
    );
}

#[test]
fn a_finished_drag_restarts_pointer_boundaries_without_resetting_legacy_mouse_hover() {
    for capture in [None, Some("source"), Some("parent")] {
        for finish in ["dragcancel", "drop"] {
            for different_target in [false, true] {
                let mut vm =
                    new_rendered_test_vm("https://drag-pointer-recovery.test/", POINTER_DRAG_PAGE);
                install_suppression_log(&mut vm, capture);
                vm.eval(r#"(() => {
                  window.__recovery=[];
                  for(const type of ['pointerover','pointerenter','pointerout','pointerleave',
                    'mouseover','mouseenter','mouseout','mouseleave'])
                    document.addEventListener(type,event=>window.__recovery.push([
                      type,event.target.id||event.target.nodeName,
                      event.relatedTarget?(event.relatedTarget.id||event.relatedTarget.nodeName):null
                    ]),true);
                  document.addEventListener('dragover',event=>event.preventDefault());
                  document.addEventListener('drop',event=>event.preventDefault());
                })()"#).unwrap();
                vm.publish_layout_for_test().unwrap();
                pointer_drag_mouse(&mut vm, "mousedown", 20.0, 20.0);
                let outcome = pointer_drag_mouse(&mut vm, "mousemove", 40.0, 40.0);
                let intercepted = outcome.intercepted_drag.expect("native drag intercepted");
                let data = RendererDragData {
                    items: intercepted.items,
                    files: Vec::new(),
                    directories: Vec::new(),
                    drag_operations_mask: intercepted.drag_operations_mask,
                };
                if finish == "drop" {
                    vm.dispatch_drag_event_at_point(45.0, 45.0, "dragenter", data.clone(), 2)
                        .unwrap();
                    vm.dispatch_drag_event_at_point(45.0, 45.0, "dragover", data.clone(), 2)
                        .unwrap();
                }
                vm.dispatch_drag_event_at_point(45.0, 45.0, finish, data, 2)
                    .unwrap();
                vm.eval("window.__recovery=[]").unwrap();
                let x = if different_target { 420.0 } else { 45.0 };
                pointer_drag_mouse(&mut vm, "mousemove", x, 45.0);
                let observed: Vec<serde_json::Value> =
                    serde_json::from_str(&vm.eval("JSON.stringify(window.__recovery)").unwrap())
                        .unwrap();
                let target = if different_target { "capture" } else { "leaf" };
                let ancestors = if different_target {
                    vec!["#document", "HTML", "BODY", "capture"]
                } else {
                    vec![
                        "#document",
                        "HTML",
                        "BODY",
                        "outer",
                        "parent",
                        "source",
                        "leaf",
                    ]
                };
                let mut expected_pointer = vec![serde_json::json!(["pointerover", target, null])];
                expected_pointer.extend(
                    ancestors
                        .into_iter()
                        .map(|ancestor| serde_json::json!(["pointerenter", ancestor, null])),
                );
                let actual_pointer: Vec<_> = observed
                    .iter()
                    .filter(|event| event[0].as_str().unwrap().starts_with("pointer"))
                    .cloned()
                    .collect();
                assert_eq!(
                    actual_pointer, expected_pointer,
                    "capture={capture:?}, finish={finish}, different={different_target}"
                );
                let mouse: Vec<_> = observed
                    .iter()
                    .filter(|event| event[0].as_str().unwrap().starts_with("mouse"))
                    .cloned()
                    .collect();
                if !different_target && capture.is_none() {
                    assert!(mouse.is_empty(), "same-target legacy hover stays inside");
                } else {
                    let previous = capture.unwrap_or("leaf");
                    assert!(mouse.contains(&serde_json::json!(["mouseout", previous, target])));
                    assert!(mouse.contains(&serde_json::json!(["mouseover", target, previous])));
                    if different_target {
                        assert!(mouse.contains(&serde_json::json!([
                            "mouseleave",
                            previous,
                            target
                        ])));
                    } else {
                        assert!(
                            mouse.iter().all(|event| event[0] != "mouseleave"),
                            "releasing capture into a descendant stays inside the old ancestor"
                        );
                    }
                }
                vm.eval("window.__recovery=[]").unwrap();
                pointer_drag_mouse(&mut vm, "mousemove", x + 1.0, 46.0);
                assert_eq!(
                    vm.eval("JSON.stringify(window.__recovery)").unwrap(),
                    "[]",
                    "reentry happens once: capture={capture:?}, finish={finish}"
                );
            }
        }
    }
}

#[test]
fn a_dragstart_that_disconnects_or_adopts_the_source_does_not_suppress_pointer_input() {
    for mutation in [
        "document.getElementById('source').remove()",
        "window.__other=document.implementation.createHTMLDocument('other');window.__other.body.appendChild(document.getElementById('source'))",
        "event.preventDefault()",
    ] {
        let mut vm =
            new_rendered_test_vm("https://drag-source-invalidated.test/", POINTER_DRAG_PAGE);
        install_suppression_log(&mut vm, None);
        vm.eval(&format!(
            "document.getElementById('source').addEventListener('dragstart',event=>{{{mutation}}})"
        ))
        .unwrap();
        vm.publish_layout_for_test().unwrap();
        pointer_drag_mouse(&mut vm, "mousedown", 20.0, 20.0);
        let outcome = pointer_drag_mouse(&mut vm, "mousemove", 40.0, 40.0);
        assert!(outcome.intercepted_drag.is_none(), "{mutation}");
        assert_eq!(
            suppression_log(&mut vm),
            serde_json::json!([]),
            "{mutation}"
        );
    }
}
