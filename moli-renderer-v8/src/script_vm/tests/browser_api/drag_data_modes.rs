use super::*;

fn mode_input(vm: &mut ScriptVm, pointer_type: &str, input: (&str, f64, i32, u8)) {
    let (event, x, buttons, modifiers) = input;
    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        x,
        80.0,
        event,
        if event == "mousemove" { -1 } else { 0 },
        Some(buttons),
        i32::from(event != "mousemove"),
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties {
            pointer_id: 17,
            pointer_type: pointer_type.to_owned(),
            pressure: if buttons == 0 { 0.0 } else { 0.5 },
            ..Default::default()
        },
        modifiers,
    )
    .unwrap();
}

#[tokio::test]
async fn native_drag_data_modes_invalidate_views_and_preserve_queued_strings() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for pointer in ["mouse", "pen"] {
        for capture in [false, true] {
            for mode in ["cancel", "drop", "reject", "escape"] {
                for modifiers in [0, 8, 15] {
                    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
                        "https://drag-data-modes.test/",
                        &loader,
                    );
                    vm.eval("if (!document.documentElement) document.appendChild(document.createElement('html')); if (!document.body) document.documentElement.appendChild(document.createElement('body')); 'ready'").unwrap();
                    vm.eval(&format!("{}; globalThis.__dragResults=__installDragDataModeProbe('{mode}',{capture},{modifiers}); 'ready'", include_str!("drag_data_modes.js"))).unwrap();
                    vm.publish_layout_for_test().unwrap();
                    for input in [
                        ("mousemove", 80.0, 0, modifiers),
                        ("mousedown", 80.0, 1, modifiers),
                        ("mousemove", 100.0, 1, modifiers),
                    ] {
                        mode_input(&mut vm, pointer, input);
                    }
                    if mode == "escape" {
                        vm.dispatch_key_event("keydown", "Escape", "Escape", "", 0, false, false)
                            .unwrap();
                    } else {
                        for x in [320.0, 520.0] {
                            mode_input(&mut vm, pointer, ("mousemove", x, 1, modifiers ^ 8));
                        }
                    }
                    vm.eval("__dragResults.afterDispatch('before-release'); 'ready'")
                        .unwrap();
                    mode_input(
                        &mut vm,
                        pointer,
                        (
                            "mouseup",
                            if mode == "escape" { 100.0 } else { 520.0 },
                            0,
                            modifiers ^ 8,
                        ),
                    );
                    vm.eval("__dragResults.afterDispatch('after-release'); 'ready'")
                        .unwrap();
                    let mut tasks = 0;
                    while vm
                        .run_one_user_interaction_executor_turn(&loader)
                        .await
                        .unwrap()
                    {
                        tasks += 1;
                        assert!(
                            tasks <= 2,
                            "protected/disabled items must not schedule callbacks"
                        );
                    }
                    assert_eq!(tasks, if mode == "drop" { 2 } else { 1 });
                    assert_eq!(vm.eval("globalThis.__dragFinished=__dragResults.finish(); __dragFinished.complete").unwrap(), "true", "{pointer}, {mode}, capture {capture}, modifiers {modifiers}: {}", vm.eval("JSON.stringify(__dragFinished.checks.filter(c=>!c.pass))").unwrap());
                    assert_eq!(vm.eval("__dragFinished.microtasks.length===__dragFinished.rows.length && __dragFinished.microtasks.every(row=>row.items===4 && row.data===(['dragstart','drop'].includes(row.phase)?'payload':''))").unwrap(), "true", "listener cleanup must run while the event view is still associated");
                }
            }
        }
    }
}

#[test]
fn constructed_transfer_remains_writable_after_synthetic_event_dispatch() {
    let mut vm = new_storage_test_vm("https://synthetic-transfer.test/");
    assert_eq!(
        vm.eval(
            r#"
      globalThis.dt=new DataTransfer(); const item=dt.items.add('before','text/plain');
      document.dispatchEvent(new DragEvent('drop',{dataTransfer:dt}));
      dt.setData('text/plain','after');
      JSON.stringify([dt.getData('text/plain'),dt.items[0]===item,item.kind,dt.items.length])
    "#
        )
        .unwrap(),
        r#"["after",true,"string",1]"#
    );
}

#[test]
fn transfer_receiver_validation_precedes_argument_conversion() {
    let mut vm = new_storage_test_vm("https://transfer-receiver.test/");
    assert_eq!(vm.eval(r#"
      const dt=new DataTransfer(); dt.setData('text/plain','value');
      const classes=[DataTransfer,DataTransferItemList,DataTransferItem];
      const objects=[dt,dt.items,dt.items[0]];
      let conversions=0; const format={toString(){conversions++; return 'text/plain';}};
      const failures=[];
      for(let i=0;i<classes.length;i++) {
        const proto=classes[i].prototype;
        for(const receiver of [{},Object.create(objects[i]),new Proxy(objects[i],{})]) {
          const method=i===0?()=>proto.getData.call(receiver,format):i===1?()=>proto.remove.call(receiver,{valueOf(){conversions++;return 0;}}):()=>proto.getAsString.call(receiver,{});
          try {method(); failures.push(false);} catch(e) {failures.push(e instanceof TypeError);}
        }
      }
      JSON.stringify([failures.every(Boolean),conversions])
    "#).unwrap(), "[true,0]");
}

#[test]
fn external_drag_views_hide_payload_until_drop_and_keep_text_default_action() {
    use crate::runtime::{RendererDragData, RendererDragDataItem};
    let mut vm = new_rendered_test_vm(
        "https://external-drag.test/",
        "<html><body style='margin:0'><input id='target' style='width:200px;height:100px'></body></html>",
    );
    vm.eval(r#"
      globalThis.transfers=[]; globalThis.observations=[];
      for (const type of ['dragenter','dragover','drop']) target.addEventListener(type,event=>{
        const dt=event.dataTransfer;
        observations.push([type,dt.getData('text/plain'),dt.items.length,Array.from(dt.types)]);
        dt.setData('text/plain','tampered'); dt.clearData();
        observations.push([dt.getData('text/plain'),transfers.every(old=>old.items.length===0 && old.getData('text/plain')==='')]);
        transfers.push(dt);
      }); 'ready'
    "#).unwrap();
    let data = RendererDragData {
        items: vec![RendererDragDataItem {
            mime_type: "text/plain".to_owned(),
            data: "external".to_owned(),
            title: None,
            base_url: None,
        }],
        files: vec![],
        directories: vec![],
        drag_operations_mask: 1,
    };
    for event in ["dragenter", "dragover", "drop"] {
        vm.dispatch_drag_event_at_point(10.0, 10.0, event, data.clone(), 0)
            .unwrap();
    }
    assert_eq!(vm.eval("JSON.stringify([observations,target.value,transfers.every(dt=>dt.items.length===0 && dt.types.length===0 && dt.getData('text/plain')==='')])").unwrap(), r#"[[["dragenter","",1,["text/plain"]],["",true],["dragover","",1,["text/plain"]],["",true],["drop","external",1,["text/plain"]],["external",true]],"external",true]"#);
}

#[test]
fn external_file_drop_uses_backing_store_after_event_view_is_disabled() {
    use crate::runtime::{RendererDragData, RendererDraggedFile};
    let mut vm = new_rendered_test_vm(
        "https://external-file-drag.test/",
        "<html><body style='margin:0'><input id='target' type='file' style='width:200px;height:100px'></body></html>",
    );
    vm.eval(r#"
      globalThis.saved=null;globalThis.payload=null;globalThis.events=[];
      target.addEventListener('drop',event=>{
        saved=event.dataTransfer; payload=saved.items[0];
        events.push([saved.files.length,payload.getAsFile()===saved.files[0],payload.getAsFile().name]);
        saved.items.clear(); saved.setData('text/plain','tampered');
      });
      target.addEventListener('input',()=>events.push(['input',saved.files.length,saved.items.length]));
      'ready'
    "#).unwrap();
    let data = RendererDragData {
        items: vec![],
        files: vec![RendererDraggedFile {
            bytes: b"contents".to_vec(),
            mime_type: "text/plain".to_owned(),
            name: "upload.txt".to_owned(),
            last_modified: 7.0,
        }],
        directories: vec![],
        drag_operations_mask: 1,
    };
    vm.dispatch_drag_event_at_point(10.0, 10.0, "drop", data, 0)
        .unwrap();
    assert_eq!(vm.eval("JSON.stringify([events,target.files[0].name,target.files[0].size,saved.files.length,payload.kind,payload.getAsFile()])").unwrap(),r#"[[[1,true,"upload.txt"],["input",0,0]],"upload.txt",8,0,"",null]"#);
}

#[test]
fn native_drag_event_views_use_each_target_documents_realm() {
    for origin in ["parent", "child"] {
        let mut vm = new_rendered_test_vm(
            "https://drag-realms.test/",
            "<html><body style='margin:0'><div id='outside' style='position:absolute;left:40px;top:40px;width:120px;height:120px'></div><iframe id='frame' style='position:absolute;left:300px;top:40px;width:160px;height:160px;border:0'></iframe></body></html>",
        );
        vm.eval(&format!(r#"
          globalThis.child=frame.contentWindow;
          child.document.body.innerHTML='<div id="inside" style="position:absolute;left:0;top:0;width:160px;height:160px"></div>';
          const source={source};source.draggable=true;
          globalThis.rows=[];globalThis.saved=[];
          for(const [win,name] of [[window,'parent'],[child,'child']]) {{
            for(const type of ['dragstart','dragenter','dragover','drop','dragend']) win.document.addEventListener(type,event=>{{
              const dt=event.dataTransfer;
              if(type==='dragstart') {{dt.setData('text/plain','realm-payload');dt.effectAllowed='copy';}}
              rows.push([name,type,event instanceof win.DragEvent,dt instanceof win.DataTransfer,dt.items instanceof win.DataTransferItemList,dt.items[0] instanceof win.DataTransferItem,event.view===win,saved.every(old=>old.items.length===0 && old.types.length===0),dt.getData('text/plain')]);
              saved.push(dt);
              if(type==='dragenter'||type==='dragover') {{dt.dropEffect='copy';event.preventDefault();}}
              if(type==='drop') event.preventDefault();
            }});
          }} 'ready'
        "#,source=if origin=="parent" {"outside"} else {"child.document.getElementById('inside')"})).unwrap();
        vm.publish_layout_for_test().unwrap();
        let (start, end) = if origin == "parent" {
            (80.0, 320.0)
        } else {
            (320.0, 80.0)
        };
        for input in [
            ("mousemove", start, 0, 0),
            ("mousedown", start, 1, 0),
            ("mousemove", start + 20.0, 1, 0),
            ("mousemove", end, 1, 0),
            ("mouseup", end, 0, 0),
        ] {
            mode_input(&mut vm, "mouse", input);
        }
        assert_eq!(vm.eval("rows.every(row=>row.slice(2,8).every(Boolean) && row[8]===(['dragstart','drop'].includes(row[1])?'realm-payload':'')) && rows.filter(row=>row[1]==='drop').length===1 && saved.every(dt=>dt.items.length===0)").unwrap(),"true","{origin}: {}",vm.eval("JSON.stringify(rows)").unwrap());
    }
}
