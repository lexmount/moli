use super::*;
use crate::runtime::{RendererDragData, RendererDragDataItem, RendererPointerEventProperties};

fn source_drag_vm(cancel_drag: bool) -> StandaloneScriptVmHarness {
    let mut vm = new_rendered_test_vm(
        "https://source-drag.test/",
        r#"<html><body style="margin:0">
        <div id=source draggable=true style="position:absolute;left:0;top:0;width:100px;height:40px"></div>
        <div id=target style="position:absolute;left:0;top:100px;width:200px;height:100px"></div>
        </body></html>"#,
    );
    vm.eval(&format!(r#"(() => {{
        window.__events=[];window.__sourceViews=[];window.__sourceItems=[];
        const source=document.getElementById('source');
        source.addEventListener('dragstart',e=>{{
          e.dataTransfer.effectAllowed='copyMove';
          e.dataTransfer.setData('text/plain','original');
          window.__sourceView=e.dataTransfer;
          window.__sourceTypes=e.dataTransfer.types;
          window.__events.push(['dragstart']);
        }});
        source.addEventListener('drag',e=>{{
          if({cancel_drag})e.preventDefault();
          const view=e.dataTransfer;
          window.__sourceViews.push(view);window.__sourceItems.push(view.items[0]);
          view.setData('text/plain','forbidden');view.effectAllowed='none';
          window.__events.push(['drag',view.getData('text/plain'),Array.from(view.types),
            view.effectAllowed,view.dropEffect,view===window.__sourceView]);
          view.dropEffect='link';
        }});
        source.addEventListener('dragend',e=>window.__events.push(['dragend',e.dataTransfer.dropEffect]));
        const target=document.getElementById('target');
        for(const type of ['dragenter','dragover','drop','dragleave'])target.addEventListener(type,e=>{{
          window.__events.push([type,e.dataTransfer.getData('application/replay')]);
          if(type!=='dragleave')e.preventDefault();
        }});
    }})()"#)).unwrap();
    vm.publish_layout_for_test().unwrap();
    vm
}

fn mouse(
    vm: &mut ScriptVm,
    event: &str,
    y: f64,
    intercept: bool,
) -> crate::runtime::RendererInputDispatchOutcome {
    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        20.0,
        y,
        event,
        0,
        Some(1),
        0,
        0.0,
        0.0,
        RendererPointerEventProperties::default(),
        0,
        intercept,
    )
    .unwrap()
}

fn events(vm: &mut ScriptVm) -> serde_json::Value {
    serde_json::from_str(&vm.eval("JSON.stringify(window.__events)").unwrap()).unwrap()
}

#[test]
fn source_drag_updates_preserve_cadence_original_metadata_and_event_view_lifetimes() {
    for cancel_drag in [false, true] {
        let mut vm = source_drag_vm(cancel_drag);
        mouse(&mut vm, "mousedown", 20.0, true);
        assert!(
            mouse(&mut vm, "mousemove", 120.0, true)
                .intercepted_drag
                .is_some()
        );
        assert_eq!(events(&mut vm), serde_json::json!([["dragstart"]]));
        // Intercepted mouse traffic is not a destination update and must not
        // emit extra source drag events.
        mouse(&mut vm, "mousemove", 130.0, true);
        assert_eq!(events(&mut vm), serde_json::json!([["dragstart"]]));
        let replay = RendererDragData {
            items: vec![RendererDragDataItem {
                mime_type: "application/replay".to_owned(),
                data: "edited".to_owned(),
                title: None,
                base_url: None,
            }],
            files: Vec::new(),
            directories: Vec::new(),
            drag_operations_mask: 1,
        };
        for event in ["dragenter", "dragover", "dragover", "drop"] {
            vm.dispatch_drag_event_at_point(20.0, 120.0, event, replay.clone(), 0)
                .unwrap();
        }
        let first_source =
            serde_json::json!(["drag", "", ["text/plain"], "copyMove", "none", true]);
        let later_source =
            serde_json::json!(["drag", "", ["text/plain"], "copyMove", "link", true]);
        assert_eq!(
            events(&mut vm),
            serde_json::json!([
                ["dragstart"],
                first_source,
                ["dragenter", ""],
                ["dragover", ""],
                later_source,
                ["dragover", ""],
                later_source,
                ["dragover", ""],
                ["drop", "edited"],
                ["dragend", "copy"]
            ]),
            "cancel source drag={cancel_drag}"
        );
        assert_eq!(vm.eval(r#"JSON.stringify([
          new Set(window.__sourceViews).size,
          window.__sourceViews.every(view=>view.types.length===0&&view.items.length===0&&view.getData('text/plain')===''),
          window.__sourceView.types.length,
          window.__sourceView.getData('text/plain'),
          window.__sourceItems.every(item=>item.kind===''&&item.type===''),
          Array.from(window.__sourceTypes)
        ])"#).unwrap(), "[1,true,0,\"\",true,[\"text/plain\"]]");
        assert!(vm.active_drag_session.is_none());
    }
}

#[test]
fn ordinary_native_drag_updates_the_source_before_target_updates_and_cancel() {
    let mut vm = source_drag_vm(false);
    mouse(&mut vm, "mousedown", 20.0, false);
    assert!(
        mouse(&mut vm, "mousemove", 120.0, false)
            .intercepted_drag
            .is_none()
    );
    mouse(&mut vm, "mousemove", 125.0, false);
    mouse(&mut vm, "mousemove", 130.0, false);
    vm.cancel_dragging().unwrap();
    let types: Vec<_> = events(&mut vm)
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event[0].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        types,
        [
            "dragstart",
            "drag",
            "dragenter",
            "dragover",
            "drag",
            "dragover",
            "drag",
            "dragleave",
            "dragend"
        ]
    );
    assert!(vm.active_drag_session.is_none());
}
