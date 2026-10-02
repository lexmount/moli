use super::*;

#[tokio::test]
async fn input_value_modes_and_type_changes_use_native_state_in_every_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://input-value-modes.test/",
        &loader,
    );
    vm.eval(
        r#"
      if (!document.documentElement) document.appendChild(document.createElement('html'));
      if (!document.body) document.documentElement.appendChild(document.createElement('body'));
      const frame=document.body.appendChild(document.createElement('iframe'));
      frame.id='child'; frame.srcdoc='<body></body>'; void frame.contentWindow;
      'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader,
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(vm.eval(include_str!("input_value_modes.js")).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true)).slice(0,30)})").unwrap());
}

#[test]
fn input_type_writeback_notifies_observers_and_custom_element_reactions() {
    let mut vm = new_storage_test_vm("https://input-type-reactions.test/");
    assert_eq!(vm.eval(r#"
      (() => {
        const changes=[];
        class ModeInput extends HTMLInputElement {
          static get observedAttributes() { return ['type','value']; }
          attributeChangedCallback(name,oldValue,newValue) { changes.push([name,oldValue,newValue]); }
        }
        customElements.define('input-mode-reactions',ModeInput,{extends:'input'});
        for (const api of ['property','attribute','namespace','attr-node']) {
          const input=new ModeInput();
          input.type='text';
          input.setAttribute('value','seed');
          input.value='live';
          changes.length=0;
          const observer=new MutationObserver(()=>{});
          observer.observe(input,{attributes:true,attributeOldValue:true});
          if (api==='property') input.type='hidden';
          else if (api==='attribute') input.setAttribute('type','hidden');
          else if (api==='namespace') input.setAttributeNS(null,'type','hidden');
          else input.getAttributeNode('type').value='hidden';
          const expected=[['type','text','hidden'],['value','seed','live']];
          if (JSON.stringify(changes)!==JSON.stringify(expected)) throw Error(api+' reactions '+JSON.stringify(changes));
          const records=observer.takeRecords().map(r=>[r.attributeName,r.oldValue]);
          if (JSON.stringify(records)!==JSON.stringify([['type','text'],['value','seed']])) throw Error(api+' observer '+JSON.stringify(records));
          input.setAttribute('value','follow');
          if (input.value!=='follow') throw Error(api+' default mode');
          input.removeAttribute('type');
          input.setAttribute('value','clean');
          if (input.value!=='clean') throw Error(api+' dirty reset');
          observer.disconnect();
        }
        return true;
      })()
    "#).unwrap(), "true");
}
