use super::*;
use crate::runtime::{RendererDragData, RendererPointerEventProperties};

#[test]
fn native_editor_selection_moves_to_the_drop_caret_and_honors_copy_and_beforeinput() {
    for (case, mask, modifiers, expected) in [
        ("move", 19, 0, "ABEFCD"),
        ("copy-mask-only", 1, 0, "ABCDEF"),
        (
            "copy-key",
            19,
            if cfg!(target_os = "macos") { 1 } else { 2 },
            "ABCDEF CD",
        ),
        ("start", 19, 0, "CDABEF"),
        ("inside", 19, 0, "ABCDEF"),
        ("cancel-delete", 19, 0, "ABCDEFCD"),
        ("cancel-insert", 19, 0, "ABEF"),
        ("disable-delete", 19, 0, "ABCDEF"),
        ("remove-delete", 19, 0, "ABCDEF"),
        ("remove-insert", 19, 0, "ABEF"),
    ] {
        let mut vm = new_rendered_test_vm(
            "https://drag-editor.test/",
            "<!doctype html><body style='margin:0'><div id=editor contenteditable style='position:absolute;left:20px;top:20px;width:400px;height:100px;font:20px/30px monospace'>ABCDEF</div>",
        );
        let points:Vec<f64>=serde_json::from_str(&vm.eval(&format!(r#"(() => {{
          const editor=document.getElementById('editor'),text=editor.firstChild;window.__editor=editor;
          const range=document.createRange();range.setStart(text,2);range.setEnd(text,4);
          editor.focus();getSelection().removeAllRanges();getSelection().addRange(range);
          const source=range.getBoundingClientRect();
          const start=document.createRange();start.setStart(text,0);start.setEnd(text,1);
          const end=document.createRange();end.setStart(text,5);end.setEnd(text,6);
          window.__editing=[];window.__ends=[];
          editor.addEventListener('beforeinput',event=>{{
            __editing.push(['beforeinput',event.inputType,event.data]);
            if({case:?}==='disable-delete'&&event.inputType==='deleteByDrag')editor.contentEditable='false';
            if(({case:?}==='remove-delete'&&event.inputType==='deleteByDrag')||
               ({case:?}==='remove-insert'&&event.inputType==='insertFromDrop'))editor.remove();
            if(({case:?}==='cancel-delete'&&event.inputType==='deleteByDrag')||
               ({case:?}==='cancel-insert'&&event.inputType==='insertFromDrop'))event.preventDefault();
          }});
          editor.addEventListener('input',event=>__editing.push(['input',event.inputType,event.data]));
          editor.addEventListener('dragend',event=>__ends.push(event.dataTransfer.dropEffect));
          return JSON.stringify([source.left+2,source.top+2,start.getBoundingClientRect().left+1,end.getBoundingClientRect().right+2]);
        }})()"#)).unwrap()).unwrap();
        vm.publish_layout_for_test().unwrap();
        vm.dispatch_mouse_event_at_point(points[0], points[1], "mousedown", 0, Some(1), 0.0, 0.0)
            .unwrap();
        vm.pending_drag_gesture.as_mut().unwrap().pressed_at -=
            std::time::Duration::from_millis(250);
        let drag = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                points[0] + 15.0,
                points[1],
                "mousemove",
                -1,
                Some(1),
                0,
                0.0,
                0.0,
                RendererPointerEventProperties::default(),
                0,
                true,
            )
            .unwrap()
            .intercepted_drag
            .unwrap();
        let data = RendererDragData {
            items: drag.items,
            files: vec![],
            directories: vec![],
            drag_operations_mask: mask,
        };
        let x = match case {
            "start" => points[2],
            "inside" => points[0],
            _ => points[3],
        };
        for event in ["dragenter", "dragover", "drop"] {
            vm.dispatch_drag_event_at_point(x, points[1], event, data.clone(), modifiers)
                .unwrap();
        }
        assert_eq!(
            vm.eval("window.__editor.textContent").unwrap(),
            expected,
            "{case}"
        );
        let events: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(__editing)").unwrap()).unwrap();
        let types: Vec<_> = events
            .as_array()
            .unwrap()
            .iter()
            .map(|event| (event[0].as_str().unwrap(), event[1].as_str().unwrap()))
            .collect();
        let mut expected_events = vec![];
        if !matches!(case, "copy-mask-only" | "copy-key" | "inside") {
            expected_events.push(("beforeinput", "deleteByDrag"));
            if !matches!(case, "cancel-delete" | "disable-delete" | "remove-delete") {
                expected_events.push(("input", "deleteByDrag"));
            }
        }
        if !matches!(case, "inside" | "copy-mask-only" | "remove-delete") {
            expected_events.push(("beforeinput", "insertFromDrop"));
            if !matches!(case, "cancel-insert" | "disable-delete" | "remove-insert") {
                expected_events.push(("input", "insertFromDrop"));
            }
        }
        assert_eq!(types, expected_events, "{case}: {events}");
        let effect = match case {
            "inside" | "copy-mask-only" => "none",
            "copy-key" => "copy",
            _ => "move",
        };
        assert_eq!(
            vm.eval("JSON.stringify(__ends)").unwrap(),
            serde_json::json!([effect]).to_string(),
            "{case}: canceled edits retain the accepted drag operation"
        );
        assert!(vm.active_drag_session.is_none());
    }
}
