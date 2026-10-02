use super::*;

#[test]
fn connected_checkables_dispatch_composed_input_in_the_owner_document() {
    let mut vm = new_parsed_test_vm(
        "https://native-control-input.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const child = document.querySelector('#child').contentWindow;
  const live = document;
  const detached = document.implementation.createHTMLDocument('');
  const rows = [];
  for (const owner of [live, detached, child.document]) {
    const host = owner.body.appendChild(owner.createElement('div'));
    const shadow = host.attachShadow({mode:'open'});
    for (const connected of [true, false]) {
      const input = owner.createElement('input');
      input.type = 'checkbox';
      if (connected) shadow.appendChild(input);
      const events = [];
      const bubble = event => events.push(event.type + ':' + (event.target === host));
      host.addEventListener('input', bubble, {once:true});
      host.addEventListener('change', bubble, {once:true});
      for (const name of ['input', 'change']) input.addEventListener(name, event => {
        const global = owner.defaultView || window;
        events.push(name + ':' + (event instanceof global.Event && !event.cancelable));
      });
      input.click();
      host.removeEventListener('input', bubble);
      host.removeEventListener('change', bubble);
      rows.push(events.join(','));
    }
  }
  return JSON.stringify(rows);
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["input:true,input:true,change:true","","input:true,input:true,change:true","","input:true,input:true,change:true",""]"#
    );
}

#[cfg(feature = "devtools")]
#[test]
fn native_select_radio_and_file_input_events_cross_shadow_boundaries() {
    let mut vm = new_storage_html_test_vm("https://native-control-input.test/");
    vm.eval(r#"
      const host=document.body.appendChild(document.createElement('div'));
      const root=host.attachShadow({mode:'open'});
      root.innerHTML='<select id=select><option value=a>A</option><option value=b>B</option></select><form><input id=radio1 type=radio name=group checked><fieldset disabled><input id=ignored type=radio name=group></fieldset><input id=radio2 type=radio name=group></form><input id=outsider type=radio name=group><input id=upload type=file>';
      const select=root.getElementById('select'),radio1=root.getElementById('radio1'),
        radio2=root.getElementById('radio2');
      globalThis.upload=root.getElementById('upload');
      globalThis.nativeControlRows=[]; globalThis.nativeControlHostRows=[];
      for (const control of [select,radio1,radio2,upload]) for (const type of ['input','change']) {
        control.addEventListener(type,event=>nativeControlRows.push({
          type,target:event.target===control,constructor:event.constructor===Event,
          ui:!(event instanceof UIEvent),trusted:event.isTrusted,bubbles:event.bubbles,
          cancelable:!event.cancelable,composed:event.composed===(type==='input')}));
      }
      for (const type of ['input','change']) host.addEventListener(type,event=>{
        nativeControlHostRows.push({type,target:event.target===host});
      });
      select.focus(); 'ready'
    "#).unwrap();
    vm.dispatch_key_event("keydown", "ArrowDown", "ArrowDown", "", 0, false, false)
        .unwrap();
    vm.eval("select.options[0].click(); radio1.focus(); 'ready'")
        .unwrap();
    vm.dispatch_key_event("keydown", "ArrowRight", "ArrowRight", "", 0, false, false)
        .unwrap();
    assert_eq!(vm.eval("radio2.checked && root.activeElement===radio2 && !root.getElementById('ignored').checked && !root.getElementById('outsider').checked").unwrap(), "true");
    vm.dispatch_key_event("keydown", "ArrowRight", "ArrowRight", "", 0, false, false)
        .unwrap();
    assert_eq!(vm.eval("radio1.checked && root.activeElement===radio1 && !root.getElementById('outsider').checked").unwrap(), "true");
    let upload = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let upload = global
                .get(scope, crate::util::v8str(scope, "upload").into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .unwrap();
            Ok(
                crate::native_bridge::node_runtime_and_handle_from_object(scope, upload)
                    .unwrap()
                    .1,
            )
        })
        .unwrap();
    assert!(
        vm.set_file_input_files(
            upload,
            vec![crate::dom::native::SelectedFile {
                data: b"content".to_vec().into(),
                mime_type: "text/plain".to_owned(),
                name: "example.txt".to_owned(),
                relative_path: String::new(),
                last_modified: 1.0,
            }],
            false,
        )
        .unwrap()
    );
    let facts: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify({rows:nativeControlRows,host:nativeControlHostRows})")
            .unwrap(),
    )
    .unwrap();
    let rows = facts["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 10, "{facts}");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["type"], if index % 2 == 0 { "input" } else { "change" });
        for (field, value) in row.as_object().unwrap() {
            if field != "type" {
                assert_eq!(value, true, "{field}: {facts}");
            }
        }
    }
    assert_eq!(
        facts["host"],
        serde_json::json!([
        {"type":"input","target":true},{"type":"input","target":true},
        {"type":"input","target":true},{"type":"input","target":true},
        {"type":"input","target":true}])
    );
}
