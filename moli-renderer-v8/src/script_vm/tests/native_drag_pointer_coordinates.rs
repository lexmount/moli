use super::*;
use crate::runtime::RendererPointerEventProperties;

fn coordinate_drag_vm(scale: Option<f64>, capture: bool) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(
        "https://drag-pointer-frame-coordinates.test/",
        "<!doctype html><html><body style='margin:0'></body></html>",
    );
    vm.eval(&format!(
        "window.__dragFrameScale = {}; window.__dragCapture = {capture};",
        serde_json::json!(scale)
    ))
    .expect("configure native drag coordinate fixture");
    vm.eval(
        r#"(() => {
          const state=window.__dragCoordinates={started:false,events:[],parentEvents:[]};
          let doc=document;
          if(window.__dragFrameScale!==null) {
            const frame=document.createElement('iframe');
            frame.id='drag-frame';
            frame.style.cssText='position:absolute;left:100px;top:120px;width:300px;height:300px;border:0;transform-origin:0 0';
            frame.style.transform=`scale(${window.__dragFrameScale})`;
            document.body.appendChild(frame);
            doc=frame.contentDocument;
            doc.documentElement.style.cssText='margin:0;padding:0';
            doc.body.style.cssText='margin:0;padding:0';
          }
          const source=doc.createElement('div');
          source.id='source';source.draggable=true;
          source.style.cssText='position:absolute;left:0;top:0;width:120px;height:120px';
          if(doc===document) {
            source.style.left='100px';source.style.top='120px';
          }
          const leaf=doc.createElement('span');
          leaf.id='leaf';leaf.style.cssText='display:block;width:100px;height:100px';
          source.appendChild(leaf);doc.body.appendChild(source);
          if(window.__dragCapture) source.addEventListener('pointerdown',event=>
            source.setPointerCapture(event.pointerId));
          source.addEventListener('dragstart',event=>{
            state.started=true;
            event.dataTransfer.setData('text/plain','frame-coordinate-payload');
          });
          const record=event=>({
            type:event.type,target:event.target.id||event.target.nodeName,
            clientX:event.clientX,clientY:event.clientY,
            screenX:event.screenX,screenY:event.screenY,
          });
          for(const type of ['pointercancel','lostpointercapture','pointerout','pointerleave']) {
            doc.addEventListener(type,event=>{
              if(state.started) state.events.push(record(event));
            },true);
            if(doc!==document) document.addEventListener(type,event=>{
              if(state.started) state.parentEvents.push(record(event));
            },true);
          }
        })()"#,
    )
    .expect("install native drag coordinate fixture");
    vm.publish_layout_for_test()
        .expect("publish native drag coordinate geometry");
    vm
}

fn start_coordinate_drag(vm: &mut StandaloneScriptVmHarness, scale: f64) -> serde_json::Value {
    for (event, position) in [("mousedown", 20.0), ("mousemove", 40.0)] {
        let outcome = vm
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                100.0 + position * scale,
                120.0 + position * scale,
                event,
                if event == "mousedown" { 0 } else { -1 },
                Some(1),
                0,
                0.0,
                0.0,
                RendererPointerEventProperties::default(),
                0,
                true,
            )
            .expect("dispatch native drag in root coordinates");
        assert_eq!(
            outcome.intercepted_drag.is_some(),
            event == "mousemove",
            "only threshold-crossing movement starts the native drag"
        );
    }
    serde_json::from_str(
        &vm.eval("JSON.stringify(window.__dragCoordinates)")
            .expect("read native drag coordinate events"),
    )
    .expect("native drag coordinate events are JSON")
}

#[test]
fn iframe_native_drag_startup_preserves_child_client_and_root_screen_coordinates() {
    for scale in [1.0, 0.5] {
        for capture in [false, true] {
            // Dispatch the same root points into a top-level source and an
            // offset/scaled iframe. Only the child client coordinates differ.
            let parent = start_coordinate_drag(&mut coordinate_drag_vm(None, capture), scale);
            let child = start_coordinate_drag(&mut coordinate_drag_vm(Some(scale), capture), scale);
            let target = if capture { "source" } else { "leaf" };
            let mut expected = vec![serde_json::json!(["pointercancel", target])];
            if capture {
                expected.push(serde_json::json!(["lostpointercapture", target]));
            }
            expected.push(serde_json::json!(["pointerout", target]));
            if !capture {
                expected.push(serde_json::json!(["pointerleave", "leaf"]));
            }
            for ancestor in ["source", "BODY", "HTML", "#document"] {
                expected.push(serde_json::json!(["pointerleave", ancestor]));
            }
            for log in [&parent, &child] {
                let sequence: Vec<_> = log["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| serde_json::json!([event["type"], event["target"]]))
                    .collect();
                assert_eq!(sequence, expected, "scale={scale}, capture={capture}");
            }
            assert_eq!(
                child["parentEvents"],
                serde_json::json!([]),
                "child suppression stays in its own Document"
            );
            let root_x = 100.0 + 40.0 * scale;
            let root_y = 120.0 + 40.0 * scale;
            for (parent_event, child_event) in parent["events"]
                .as_array()
                .unwrap()
                .iter()
                .zip(child["events"].as_array().unwrap())
            {
                for (field, expected) in [("clientX", root_x), ("clientY", root_y)] {
                    assert!(
                        (parent_event[field].as_f64().unwrap() - expected).abs() < 0.001,
                        "top-level {field}: {parent_event}, scale={scale}, capture={capture}"
                    );
                }
                for field in ["clientX", "clientY"] {
                    assert!(
                        (child_event[field].as_f64().unwrap() - 40.0).abs() < 0.001,
                        "child {field}: {child_event}, scale={scale}, capture={capture}"
                    );
                }
                for (field, expected) in [("screenX", root_x), ("screenY", root_y)] {
                    assert_eq!(
                        parent_event[field], child_event[field],
                        "{field} stays in root space for both documents"
                    );
                    assert!(
                        (child_event[field].as_f64().unwrap() - expected).abs() < 0.001,
                        "child {field}: {child_event}, scale={scale}, capture={capture}"
                    );
                }
            }
        }
    }
}

#[test]
fn captured_iframe_drag_startup_maps_coordinates_after_the_pointer_leaves_the_frame() {
    for scale in [1.0, 0.5] {
        let mut vm = coordinate_drag_vm(Some(scale), true);
        vm.eval(
            r#"(() => {
              const source=document.getElementById('drag-frame').contentDocument.getElementById('source');
              window.__captureCoordinates=[];
              for(const type of ['gotpointercapture','lostpointercapture']) source.addEventListener(type,event=>
                window.__captureCoordinates.push({
                  type,target:event.target.id,captured:source.hasPointerCapture(event.pointerId),
                  clientX:event.clientX,clientY:event.clientY,screenX:event.screenX,screenY:event.screenY,
                }));
            })()"#,
        )
        .expect("observe capture coordinates across frame exit");
        for (event, x, y) in [
            ("mousedown", 100.0 + 20.0 * scale, 120.0 + 20.0 * scale),
            ("mousemove", 20.0, 20.0),
        ] {
            let outcome = vm
                .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                    x,
                    y,
                    event,
                    if event == "mousedown" { 0 } else { -1 },
                    Some(1),
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                    true,
                )
                .expect("dispatch captured drag across iframe boundary");
            assert_eq!(outcome.intercepted_drag.is_some(), event == "mousemove");
        }
        let result: serde_json::Value = serde_json::from_str(
            &vm.eval("JSON.stringify({startup:window.__dragCoordinates,capture:window.__captureCoordinates})")
                .expect("read captured iframe drag coordinates"),
        )
        .unwrap();
        let events = result["startup"]["events"].as_array().unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| serde_json::json!([event["type"], event["target"]]))
                .collect::<Vec<_>>(),
            vec![
                serde_json::json!(["pointercancel", "source"]),
                serde_json::json!(["lostpointercapture", "source"]),
                serde_json::json!(["pointerout", "source"]),
                serde_json::json!(["pointerleave", "source"]),
                serde_json::json!(["pointerleave", "BODY"]),
                serde_json::json!(["pointerleave", "HTML"]),
                serde_json::json!(["pointerleave", "#document"]),
            ],
            "scale={scale}"
        );
        assert_eq!(result["startup"]["parentEvents"], serde_json::json!([]));
        let capture = result["capture"].as_array().unwrap();
        assert_eq!(
            capture
                .iter()
                .map(|event| serde_json::json!([event["type"], event["target"], event["captured"]]))
                .collect::<Vec<_>>(),
            vec![
                serde_json::json!(["gotpointercapture", "source", true]),
                serde_json::json!(["lostpointercapture", "source", false]),
            ],
            "source capture is acquired and released exactly once, scale={scale}"
        );
        // This is Moli's root-screen / target-client coordinate contract.
        // Chromium's UA-generated cancellation currently supplies zeroes.
        for event in events.iter().chain(capture) {
            for (field, expected) in [
                ("clientX", -80.0 / scale),
                ("clientY", -100.0 / scale),
                ("screenX", 20.0),
                ("screenY", 20.0),
            ] {
                assert!(
                    (event[field].as_f64().unwrap() - expected).abs() < 0.001,
                    "captured {field}: {event}, scale={scale}"
                );
            }
        }
    }
}
