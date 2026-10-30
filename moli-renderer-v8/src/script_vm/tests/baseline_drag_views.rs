use super::*;
use crate::runtime::{RendererDragData, RendererDragDataItem, RendererDraggedFile};

const DRAG_VIEW_PAGE: &str = r#"<html><body style="margin:0">
<div id=source draggable=true style="position:absolute;left:0;top:0;width:100px;height:40px"></div>
<div id=target style="position:absolute;left:0;top:100px;width:200px;height:100px"></div>
</body></html>"#;

fn drag_payload(name: &str, mask: i32) -> RendererDragData {
    RendererDragData {
        items: vec![
            RendererDragDataItem {
                mime_type: "text/plain".to_owned(),
                data: format!("{name}-text"),
                title: None,
                base_url: None,
            },
            RendererDragDataItem {
                mime_type: format!("application/x-{name}"),
                data: format!("{name}-metadata"),
                title: None,
                base_url: None,
            },
        ],
        files: vec![RendererDraggedFile {
            name: format!("{name}.bin"),
            mime_type: "application/octet-stream".to_owned(),
            bytes: name.as_bytes().to_vec(),
            last_modified: 123.0,
        }],
        directories: Vec::new(),
        drag_operations_mask: mask,
    }
}

#[test]
fn baseline_native_drag_retains_protected_source_and_closes_each_target_view() {
    let mut vm = new_rendered_test_vm("https://baseline-native-drag-views.test/", DRAG_VIEW_PAGE);
    vm.eval(r#"
window.__views=[];window.__records=[];window.__prior=[];
window.DataTransfer=function(){throw new Error('author constructor must not run');};
document.getElementById('source').addEventListener('dragstart', e => {
  e.dataTransfer.setData('text/plain','original');
  e.dataTransfer.setData('application/x-source','metadata');
  e.dataTransfer.effectAllowed='copy';
  window.__source=e.dataTransfer;window.__sourceItem=e.dataTransfer.items[0];
  window.__sourceTypes=e.dataTransfer.types;
});
const target=document.getElementById('target');
for(const type of ['dragover','drop'])target.addEventListener(type, e => {
  window.__prior.push(window.__views.map(view => [view.getData('text/plain'),view.types.length,view.items.length,view.files.length]));
  const view=e.dataTransfer;window.__views.push(view);
  view.setData('text/plain','replaced');view.effectAllowed='none';
  window.__records.push([type,view===window.__source,view.getData('text/plain'),
    view.getData('application/x-source'),Array.from(view.types),
    Array.from(view.items,item => [item.kind,item.type]),view.effectAllowed,
    [window.__source.getData('text/plain'),Array.from(window.__source.types),
      window.__sourceItem.kind,window.__sourceItem.type]]);
  e.preventDefault();
});
"#).expect("install native view listeners");
    vm.publish_layout_for_test().expect("publish drag geometry");
    for (event, y, buttons) in [
        ("mousedown", 20.0, 1),
        ("mousemove", 120.0, 1),
        ("mousemove", 125.0, 1),
        ("mouseup", 125.0, 0),
    ] {
        vm.dispatch_mouse_event_at_point(20.0, y, event, 0, Some(buttons), 0.0, 0.0)
            .expect("dispatch native drag event");
    }
    let observed: serde_json::Value = serde_json::from_str(&vm.eval(
        "JSON.stringify({records:window.__records,prior:window.__prior,distinct:new Set(window.__views).size,sourceTypes:Array.from(window.__sourceTypes),sourceAfter:[window.__source.getData('text/plain'),window.__source.types.length,window.__source.items.length,window.__sourceItem.kind,window.__sourceItem.type],closed:window.__views.map(view => [view.getData('text/plain'),view.types.length,view.items.length,view.files.length])})",
    ).expect("native drag view result")).expect("native result JSON");
    let mut records = Vec::new();
    for (event, text, metadata) in [
        ("dragover", "", ""),
        ("dragover", "", ""),
        ("drop", "original", "metadata"),
    ] {
        records.push(serde_json::json!([
            event,
            false,
            text,
            metadata,
            ["text/plain", "application/x-source"],
            [["string", "text/plain"], ["string", "application/x-source"]],
            "copy",
            [
                "",
                ["text/plain", "application/x-source"],
                "string",
                "text/plain"
            ]
        ]));
    }
    assert_eq!(
        observed,
        serde_json::json!({
            "records":records,"prior":[[],[["",0,0,0]],[["",0,0,0],["",0,0,0]]],
            "distinct":3,"sourceTypes":["text/plain","application/x-source"],
            "sourceAfter":["",0,0,"",""],
            "closed":[["",0,0,0],["",0,0,0],["",0,0,0]]
        })
    );
    assert!(vm.active_drag_session.is_none());
}

#[test]
fn baseline_native_source_views_close_on_all_non_drop_exit_paths() {
    for finish in [
        "canceled-start",
        "cancel",
        "outside-release",
        "no-buttons",
        "new-press",
        "retire",
    ] {
        let mut vm =
            new_rendered_test_vm("https://baseline-drag-source-exits.test/", DRAG_VIEW_PAGE);
        vm.eval(
            r#"
document.getElementById('source').addEventListener('dragstart', e => {
  window.__source=e.dataTransfer;window.__item=e.dataTransfer.items.add('original','text/plain');
  if(window.__cancelStart)e.preventDefault();
});
"#,
        )
        .expect("source exit listener");
        vm.eval(if finish == "canceled-start" {
            "window.__cancelStart=true"
        } else {
            "window.__cancelStart=false"
        })
        .expect("configure start cancellation");
        vm.publish_layout_for_test().expect("source exit geometry");
        vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, Some(1), 0.0, 0.0)
            .expect("press source");
        vm.dispatch_mouse_event_at_point(20.0, 120.0, "mousemove", 0, Some(1), 0.0, 0.0)
            .expect("start native drag");
        if finish != "canceled-start" {
            assert_eq!(vm.eval("JSON.stringify([window.__source.getData('text/plain'),window.__source.types.length,window.__source.items.length,window.__item.kind,window.__item.type])").expect("protected source state"),
                r#"["",1,1,"string","text/plain"]"#, "{finish}");
        }
        match finish {
            "cancel" => vm
                .clear_active_drag_data_transfer()
                .expect("cancel drag data access"),
            "outside-release" => {
                vm.dispatch_mouse_event_at_point(-1000.0, -1000.0, "mouseup", 0, Some(0), 0.0, 0.0)
                    .expect("release outside document");
            }
            "no-buttons" => {
                vm.dispatch_mouse_event_at_point(20.0, 130.0, "mousemove", 0, Some(0), 0.0, 0.0)
                    .expect("observe released primary button");
            }
            "new-press" => {
                vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, Some(1), 0.0, 0.0)
                    .expect("new press retires previous source view");
            }
            "retire" => crate::script_vm::input_helpers::clear_input_dispatch_state(&mut vm),
            _ => {}
        }
        assert!(vm.active_drag_session.is_none(), "{finish}");
        assert_eq!(vm.eval("JSON.stringify([window.__source.getData('text/plain'),window.__source.types.length,window.__source.items.length,window.__source.files.length,window.__item.kind,window.__item.type])").expect("closed source state"),
            r#"["",0,0,0,"",""]"#, "{finish}");
    }
}

#[test]
fn baseline_external_drag_replaces_enter_and_drop_payload_but_only_updates_over_mask() {
    for empty_enter in [false, true] {
        let mut vm =
            new_rendered_test_vm("https://baseline-external-drag-views.test/", DRAG_VIEW_PAGE);
        vm.eval(r#"
window.__views=[];window.__records=[];window.__prior=[];
for(const type of ['dragenter','dragover','drop'])document.getElementById('target').addEventListener(type, e => {
  window.__prior.push(window.__views.map(view => [view.getData('text/plain'),view.types.length,view.items.length,view.files.length]));
  const view=e.dataTransfer;window.__views.push(view);
  window.__records.push([type,view.effectAllowed,view.dropEffect,view.getData('text/plain'),
    view.getData('application/x-drop'),Array.from(view.types),
    Array.from(view.items,item => [item.kind,item.type]),
    Array.from(view.files,file => [file.name,file.type,file.size,file.lastModified])]);
  e.preventDefault();
  if(type==='dragover')view.dropEffect='move';
  if(type==='drop')view.files[0].text().then(text => window.__fileText=text);
});
"#).expect("external view listeners");
        vm.publish_layout_for_test()
            .expect("external drag geometry");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", drag_payload("seed", 1), 0)
            .expect("seed external store");
        let mut enter = drag_payload("enter", 1);
        if empty_enter {
            enter.items.clear();
            enter.files.clear();
        }
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragenter", enter, 0)
            .expect("replace enter payload including empty payload");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "dragover", drag_payload("ignored", 16), 0)
            .expect("update over operation mask only");
        vm.dispatch_drag_event_at_point(20.0, 120.0, "drop", drag_payload("drop", 16), 0)
            .expect("replace drop payload");
        let observed: serde_json::Value = serde_json::from_str(&vm.eval(
            "JSON.stringify({records:window.__records,prior:window.__prior,fileText:window.__fileText,closed:window.__views.map(view => [view.getData('text/plain'),view.types.length,view.items.length,view.files.length])})",
        ).expect("external view result")).expect("external result JSON");
        let entered_types = if empty_enter {
            serde_json::json!([])
        } else {
            serde_json::json!(["text/plain", "application/x-enter", "Files"])
        };
        let entered_items = if empty_enter {
            serde_json::json!([])
        } else {
            serde_json::json!([
                ["string", "text/plain"],
                ["string", "application/x-enter"],
                ["file", "application/octet-stream"]
            ])
        };
        assert_eq!(
            observed,
            serde_json::json!({
                "records":[
                    ["dragenter","copy","copy","","",["text/plain","application/x-seed","Files"],[["string","text/plain"],["string","application/x-seed"],["file","application/octet-stream"]],[]],
                    ["dragover","copy","copy","","",entered_types,entered_items,[]],
                    ["dragover","move","move","","",entered_types,entered_items,[]],
                    ["dragover","move","move","","",entered_types,entered_items,[]],
                    ["drop","move","move","drop-text","drop-metadata",["text/plain","application/x-drop","Files"],[["string","text/plain"],["string","application/x-drop"],["file","application/octet-stream"]],[["drop.bin","application/octet-stream",4,123]]]
                ],
                "prior":[[],[["",0,0,0]],[["",0,0,0],["",0,0,0]],[["",0,0,0],["",0,0,0],["",0,0,0]],[["",0,0,0],["",0,0,0],["",0,0,0],["",0,0,0]]],
                "fileText":"drop","closed":[["",0,0,0],["",0,0,0],["",0,0,0],["",0,0,0],["",0,0,0]]
            }),
            "empty_enter={empty_enter}"
        );
        assert!(vm.active_drag_session.is_none());
    }
}

#[test]
fn baseline_drop_closes_dom_view_before_editing_and_retains_input_snapshot() {
    let page = r#"<html><body style="margin:0"><div id=target contenteditable=true style="width:200px;height:100px"></div></body></html>"#;
    let mut vm = new_rendered_test_vm("https://baseline-drop-editing-view.test/", page);
    vm.eval(r#"
window.__events=[];window.__inputs=[];
const target=document.getElementById('target');target.focus();
const range=document.createRange();range.selectNodeContents(target);range.collapse(false);
getSelection().removeAllRanges();getSelection().addRange(range);
target.addEventListener('drop', e => {window.__drop=e.dataTransfer;window.__dropItem=e.dataTransfer.items[0];});
for(const type of ['beforeinput','input'])target.addEventListener(type, e => {
  window.__inputs.push(e.dataTransfer);
  e.dataTransfer.setData('text/plain','must-not-replace');
  window.__events.push([type,e.data,e.inputType,e.dataTransfer===window.__drop,
    e.dataTransfer.getData('text/plain'),Array.from(e.dataTransfer.types),
    [window.__drop.getData('text/plain'),window.__drop.types.length,window.__drop.items.length,
      window.__dropItem.kind,window.__dropItem.type]]);
});
"#).expect("drop editing listeners");
    vm.publish_layout_for_test()
        .expect("editable drop geometry");
    let mut data = drag_payload("input", 1);
    data.files.clear();
    for event in ["dragenter", "dragover", "drop"] {
        vm.dispatch_drag_event_at_point(20.0, 20.0, event, data.clone(), 0)
            .expect("dispatch editable drop event");
    }
    let observed: serde_json::Value = serde_json::from_str(&vm.eval(
        "JSON.stringify([window.__events,window.__inputs[0]===window.__inputs[1],window.__inputs[0].getData('text/plain'),document.getElementById('target').textContent,window.__drop.types.length])",
    ).expect("drop editing result")).expect("editing result JSON");
    assert_eq!(
        observed,
        serde_json::json!([
            [
                [
                    "beforeinput",
                    null,
                    "insertFromDrop",
                    false,
                    "input-text",
                    ["text/plain", "application/x-input"],
                    ["", 0, 0, "", ""]
                ],
                [
                    "input",
                    null,
                    "insertFromDrop",
                    false,
                    "input-text",
                    ["text/plain", "application/x-input"],
                    ["", 0, 0, "", ""]
                ]
            ],
            true,
            "input-text",
            "input-text",
            0
        ])
    );
}
