use super::*;
use crate::runtime::{RendererDragData, RendererDragDataItem, RendererPointerEventProperties};

fn cross_document_drag_vm(scale: f64) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(
        "https://cross-document-drag.test/",
        "<!doctype html><html><body style='margin:0;min-height:600px'></body></html>",
    );
    vm.eval(&format!("window.__frameScale={scale}")).unwrap();
    vm.eval(
        r#"(() => {
          const state=window.__crossDrag={events:[],views:[],sourceTransfer:null,lastEnter:null};
          const documents={parent:document};
          for(const [name,left,top] of [['a',200,80],['b',420,100]]) {
            const frame=document.createElement('iframe');
            frame.id='frame-'+name;
            frame.style.cssText=`position:absolute;left:${left}px;top:${top}px;width:200px;height:160px;border:0;transform-origin:0 0;transform:scale(${window.__frameScale})`;
            document.body.appendChild(frame);
            const doc=frame.contentDocument;
            doc.documentElement.style.cssText='margin:0;padding:0';
            doc.body.style.cssText='margin:0;padding:0';
            documents[name]=doc;
          }
          const nested=documents.a.createElement('iframe');
          nested.id='frame-c';
          nested.style.cssText='position:absolute;left:10px;top:90px;width:100px;height:60px;border:0';
          documents.a.body.appendChild(nested);
          documents.c=nested.contentDocument;
          documents.c.documentElement.style.cssText='margin:0;padding:0';
          documents.c.body.style.cssText='margin:0;padding:0';
          state.documents=documents;
          for(const [name,doc] of Object.entries(documents)) {
            for(const id of name==='a'?['a','a2']:[name]) {
              const target=doc.createElement('div');
              target.id=id;target.draggable=true;
              target.style.cssText=name==='parent'
                ?'position:absolute;left:20px;top:160px;width:120px;height:80px'
                :`position:absolute;left:${id==='a2'?100:0}px;top:0;width:80px;height:80px`;
              doc.body.appendChild(target);
              target.addEventListener('dragstart',event=>{
                state.sourceTransfer=event.dataTransfer;
                event.dataTransfer.effectAllowed='copy';
                event.dataTransfer.setData('text/plain','payload');
              });
              for(const type of ['dragenter','dragover','drop']) target.addEventListener(type,event=>{
                if(type!=='drop') event.dataTransfer.dropEffect='copy';
                event.preventDefault();
              });
            }
            for(const type of ['dragenter','dragover','dragleave','drop','dragend'])
              doc.addEventListener(type,event=>{
                if(type==='dragenter')state.lastEnter=event.dataTransfer;
                if(type!=='dragend')state.views.push(event.dataTransfer);
                state.events.push({
                  type,target:event.target.id,document:name,
                  related:event.relatedTarget?event.relatedTarget.id:null,
                  relatedDocument:event.relatedTarget
                    ?Object.entries(documents).find(([,doc])=>doc===event.relatedTarget.ownerDocument)?.[0]:null,
                  clientX:event.clientX,clientY:event.clientY,
                  screenX:event.screenX,screenY:event.screenY,
                  read:event.dataTransfer.getData('text/plain'),types:Array.from(event.dataTransfer.types),
                  sameEnterView:event.dataTransfer===state.lastEnter,
                  sameSourceView:event.dataTransfer===state.sourceTransfer,
                });
              },true);
          }
        })()"#,
    )
    .unwrap();
    vm.publish_layout_for_test().unwrap();
    vm
}

fn root_position(target: &str, scale: f64) -> (f64, f64) {
    match target {
        "parent" => (40.0, 180.0),
        "a" => (200.0 + 20.0 * scale, 80.0 + 20.0 * scale),
        "a2" => (200.0 + 120.0 * scale, 80.0 + 20.0 * scale),
        "b" => (420.0 + 20.0 * scale, 100.0 + 20.0 * scale),
        "c" => (200.0 + 30.0 * scale, 80.0 + 110.0 * scale),
        _ => unreachable!(),
    }
}

fn target_document(target: &str) -> &str {
    if target == "a2" { "a" } else { target }
}

fn assert_event_coordinates(
    event: &serde_json::Value,
    document: &str,
    root: (f64, f64),
    scale: f64,
) {
    assert_eq!(event["document"], document);
    let client = match document {
        "parent" => root,
        "a" => ((root.0 - 200.0) / scale, (root.1 - 80.0) / scale),
        "b" => ((root.0 - 420.0) / scale, (root.1 - 100.0) / scale),
        "c" => (
            (root.0 - 200.0) / scale - 10.0,
            (root.1 - 80.0) / scale - 90.0,
        ),
        _ => unreachable!(),
    };
    for (field, expected) in [
        ("clientX", client.0),
        ("clientY", client.1),
        ("screenX", root.0),
        ("screenY", root.1),
    ] {
        assert!(
            (event[field].as_f64().unwrap() - expected).abs() < 0.001,
            "{field}: {event}, expected={expected}, scale={scale}"
        );
    }
}

fn external_drag_data() -> RendererDragData {
    RendererDragData {
        items: vec![RendererDragDataItem {
            mime_type: "text/plain".to_owned(),
            data: "payload".to_owned(),
            title: None,
            base_url: None,
        }],
        files: Vec::new(),
        directories: Vec::new(),
        drag_operations_mask: 1,
    }
}

fn dispatch_drag(
    vm: &mut StandaloneScriptVmHarness,
    event: &str,
    target: &str,
    scale: f64,
    data: &RendererDragData,
) {
    let (x, y) = root_position(target, scale);
    vm.dispatch_drag_event_at_point(x, y, event, data.clone(), 0)
        .unwrap();
}

fn drag_log(vm: &mut StandaloneScriptVmHarness) -> serde_json::Value {
    serde_json::from_str(
        &vm.eval(
            r#"JSON.stringify({events:window.__crossDrag.events,
              viewsDisabled:window.__crossDrag.views.every(view=>view.types.length===0&&view.items.length===0&&view.getData('text/plain')===''),
              sourceDisabled:!window.__crossDrag.sourceTransfer||window.__crossDrag.sourceTransfer.types.length===0})"#,
        )
        .unwrap(),
    )
    .unwrap()
}

fn start_native_drag(
    vm: &mut StandaloneScriptVmHarness,
    source: &str,
    scale: f64,
) -> RendererDragData {
    let (x, y) = root_position(source, scale);
    let step = if source == "parent" {
        20.0
    } else {
        20.0 * scale
    };
    for (name, x) in [("mousedown", x), ("mousemove", x + step)] {
        let outcome = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                x,
                y,
                name,
                if name == "mousedown" { 0 } else { -1 },
                Some(1),
                0,
                0.0,
                0.0,
                RendererPointerEventProperties::default(),
                0,
                true,
            )
            .unwrap();
        if name == "mousemove" {
            let intercepted = outcome
                .intercepted_drag
                .expect("native source starts a drag");
            return RendererDragData {
                items: intercepted.items,
                files: Vec::new(),
                directories: Vec::new(),
                drag_operations_mask: intercepted.drag_operations_mask,
            };
        }
    }
    unreachable!()
}

#[test]
fn drag_transitions_project_each_document_and_keep_related_targets_local() {
    for scale in [1.0, 0.5] {
        for (previous, current) in [
            ("parent", "a"),
            ("a", "parent"),
            ("a", "b"),
            ("a", "a2"),
            ("parent", "c"),
            ("c", "parent"),
            ("a", "c"),
            ("c", "a"),
            ("c", "b"),
        ] {
            let mut vm = cross_document_drag_vm(scale);
            let data = external_drag_data();
            dispatch_drag(&mut vm, "dragenter", previous, scale, &data);
            vm.eval("window.__crossDrag.events=[]").unwrap();
            dispatch_drag(&mut vm, "dragover", current, scale, &data);
            let result = drag_log(&mut vm);
            let events = result["events"].as_array().unwrap();
            assert_eq!(events.len(), 2, "{previous} -> {current}");
            assert_eq!(events[0]["type"], "dragenter");
            assert_eq!(events[0]["target"], current);
            assert_eq!(events[1]["type"], "dragleave");
            assert_eq!(events[1]["target"], previous);
            let root = root_position(current, scale);
            assert_event_coordinates(&events[0], target_document(current), root, scale);
            assert_event_coordinates(&events[1], target_document(previous), root, scale);
            for (event, related) in [(&events[0], previous), (&events[1], current)] {
                if target_document(previous) == target_document(current) {
                    assert_eq!(event["related"], related);
                    assert_eq!(event["relatedDocument"], target_document(related));
                } else if let Some(host) = match (
                    event["document"].as_str().unwrap(),
                    target_document(related),
                ) {
                    ("parent", "a" | "c") => Some("frame-a"),
                    ("parent", "b") => Some("frame-b"),
                    ("a", "c") => Some("frame-c"),
                    _ => None,
                } {
                    assert_eq!(event["related"], host);
                    assert_eq!(event["relatedDocument"], event["document"]);
                } else {
                    assert_eq!(event["related"], serde_json::Value::Null);
                    assert_eq!(event["relatedDocument"], serde_json::Value::Null);
                }
                assert_eq!(event["read"], "");
                assert_eq!(event["types"], serde_json::json!(["text/plain"]));
            }
            assert_eq!(events[1]["sameEnterView"], true);
            assert_eq!(result["viewsDisabled"], true);
        }
    }
}

#[test]
fn drag_cancel_and_drop_reproject_the_source_end_without_reviving_event_views() {
    // Moli projects client coordinates into the receiving Document, including
    // the source's dragend. Chromium currently keeps the destination manager's
    // client coordinates for source events; root/screen coordinates agree.
    for scale in [1.0, 0.5] {
        for (source, target) in [("parent", "a"), ("a", "parent"), ("a", "b"), ("b", "a")] {
            for finish in ["dragcancel", "drop"] {
                let mut vm = cross_document_drag_vm(scale);
                let data = start_native_drag(&mut vm, source, scale);
                dispatch_drag(&mut vm, "dragenter", target, scale, &data);
                dispatch_drag(&mut vm, "dragover", target, scale, &data);
                vm.eval("window.__crossDrag.events=[]").unwrap();
                dispatch_drag(&mut vm, finish, target, scale, &data);
                let result = drag_log(&mut vm);
                let events = result["events"].as_array().unwrap();
                assert_eq!(
                    events
                        .iter()
                        .map(|event| event["type"].as_str().unwrap())
                        .collect::<Vec<_>>(),
                    if finish == "drop" {
                        vec!["dragover", "drop", "dragend"]
                    } else {
                        vec!["dragleave", "dragend"]
                    },
                    "{source} -> {target}, {finish}"
                );
                let root = root_position(target, scale);
                for event in &events[..events.len() - 1] {
                    assert_eq!(event["target"], target);
                    assert_event_coordinates(event, target_document(target), root, scale);
                    assert_eq!(
                        event["read"],
                        if event["type"] == "drop" {
                            "payload"
                        } else {
                            ""
                        }
                    );
                }
                let end = events.last().unwrap();
                assert_eq!(end["target"], source);
                assert_event_coordinates(end, target_document(source), root, scale);
                assert_eq!(end["read"], "");
                assert_eq!(end["sameSourceView"], true);
                assert_eq!(result["viewsDisabled"], true);
                assert_eq!(result["sourceDisabled"], true);
            }
        }
    }
}

#[test]
fn a_drag_target_adopted_during_enter_uses_its_new_document_for_leave() {
    for scale in [1.0, 0.5] {
        let mut vm = cross_document_drag_vm(scale);
        let data = external_drag_data();
        dispatch_drag(&mut vm, "dragenter", "a", scale, &data);
        vm.eval(
            r#"document.getElementById('parent').addEventListener('dragenter',()=>{
          const docs=window.__crossDrag.documents;
          docs.b.body.appendChild(docs.a.getElementById('a'));
        });window.__crossDrag.events=[]"#,
        )
        .unwrap();
        dispatch_drag(&mut vm, "dragover", "parent", scale, &data);
        let result = drag_log(&mut vm);
        let events = result["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["type"], "dragleave");
        assert_eq!(events[1]["target"], "a");
        assert_event_coordinates(&events[1], "b", root_position("parent", scale), scale);
        assert_eq!(events[1]["related"], serde_json::Value::Null);
        assert_eq!(events[1]["sameEnterView"], true);
        assert_eq!(result["viewsDisabled"], true);
    }
}

#[test]
fn a_source_adopted_during_drop_uses_its_current_document_for_dragend() {
    for scale in [1.0, 0.5] {
        let mut vm = cross_document_drag_vm(scale);
        let data = start_native_drag(&mut vm, "a", scale);
        dispatch_drag(&mut vm, "dragenter", "parent", scale, &data);
        dispatch_drag(&mut vm, "dragover", "parent", scale, &data);
        vm.eval(
            r#"document.getElementById('parent').addEventListener('drop',()=>{
          const docs=window.__crossDrag.documents;
          docs.b.body.appendChild(docs.a.getElementById('a'));
        });window.__crossDrag.events=[]"#,
        )
        .unwrap();
        dispatch_drag(&mut vm, "drop", "parent", scale, &data);
        let result = drag_log(&mut vm);
        let events = result["events"].as_array().unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["dragover", "drop", "dragend"]
        );
        let root = root_position("parent", scale);
        for event in &events[..2] {
            assert_eq!(event["target"], "parent");
            assert_event_coordinates(event, "parent", root, scale);
            assert_eq!(
                event["read"],
                if event["type"] == "drop" {
                    "payload"
                } else {
                    ""
                }
            );
        }
        assert_eq!(events[2]["target"], "a");
        assert_event_coordinates(&events[2], "b", root, scale);
        assert_eq!(events[2]["read"], "");
        assert_eq!(events[2]["sameSourceView"], true);
        assert_eq!(result["viewsDisabled"], true);
        assert_eq!(result["sourceDisabled"], true);
    }
}
