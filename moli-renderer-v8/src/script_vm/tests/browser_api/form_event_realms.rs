use super::*;

#[tokio::test]
async fn native_form_events_use_target_realms_and_bypass_author_construction_hooks() {
    let mut vm = native_ui_test_vm().await;
    assert_eq!(
        vm.eval(include_str!("native_form_event_realms.js"))
            .unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults)").unwrap()
    );
}

#[test]
fn native_form_data_events_preserve_unread_author_constructor_bindings() {
    let mut vm = new_storage_html_test_vm("https://native-form-data-lazy.test/");
    let result = vm
        .eval(
            r#"(() => {
        const Data=FormData;
        const form=document.body.appendChild(document.createElement('form'));
        form.innerHTML='<input name=field value=value>';
        let reads=0, captured;
        const getter=()=>{reads++;throw Error('author FormDataEvent');};
        Object.defineProperty(globalThis,'FormDataEvent',{configurable:false,get:getter});
        form.addEventListener('formdata',event=>{
          captured=event;
          event.formData.append('listener','updated');
        });
        const result=new Data(form);
        const binding=Object.getOwnPropertyDescriptor(globalThis,'FormDataEvent');
        return reads===0 && binding.get===getter && !binding.configurable &&
          Object.prototype.toString.call(captured)==='[object FormDataEvent]' &&
          captured.isTrusted && captured.target===form && captured.formData instanceof Data &&
          captured.formData!==result && result.get('listener')==='updated';
    })()"#,
        )
        .unwrap();
    assert_eq!(result, "true");
}

#[test]
fn native_form_events_share_mutations_with_isolated_listener_worlds() {
    let mut vm = new_storage_html_test_vm("https://native-form-worlds.test/");
    vm.eval("document.body.innerHTML='<form id=form><input id=input name=field value=initial><button id=submit type=submit></button></form>'; 'ready'")
        .unwrap();
    let isolated = vm.create_isolated_world("native-form", false).unwrap();
    let observe = r#"
        globalThis.nativeFormRows=[];
        const E=Event, S=SubmitEvent, D=FormDataEvent, F=FormData;
        const form=document.getElementById('form'), input=document.getElementById('input'),
          submit=document.getElementById('submit');
        for (const type of ['submit','reset','invalid','formdata']) {
          const target=type==='invalid'?input:form;
          target.addEventListener(type,event=> {
            const Constructor=type==='submit'?S:type==='formdata'?D:E;
            const checks={realm:event instanceof E,typed:event instanceof Constructor,
              prototype:Object.getPrototypeOf(event)===Constructor.prototype,
              constructor:event.constructor===Constructor,
              target:event.target===target,current:event.currentTarget===target,trusted:event.isTrusted};
            if (type==='submit') checks.submitter=event.submitter===submit;
            if (type==='formdata') {
              checks.payload=event.formData instanceof F;
              checks.payloadIdentity=event.formData===event.formData;
              event.formData.append(globalThis.formWorld,'updated');
            } else event.preventDefault();
            nativeFormRows.push(checks);
          });
        }
        'ready'
    "#;
    vm.eval("globalThis.formWorld='main'; 'ready'").unwrap();
    vm.eval_in_isolated_context(isolated, "globalThis.formWorld='isolated'; 'ready'")
        .unwrap();
    vm.eval(observe).unwrap();
    vm.eval_in_isolated_context(isolated, observe).unwrap();
    let actions = r#"(() => {
        input.value='edited'; form.requestSubmit(submit); form.reset();
        const resetCanceled=input.value==='edited';
        input.required=true; input.value=''; const valid=input.checkValidity();
        input.required=false; input.value='edited'; const result=new FormData(form);
        return resetCanceled && !valid && result instanceof FormData &&
          result.get('field')==='edited' && result.get('main')==='updated' && result.get('isolated')==='updated';
    })()"#;
    assert_eq!(vm.eval(actions).unwrap(), "true");
    assert_eq!(
        vm.eval_in_isolated_context(isolated, actions).unwrap(),
        "true"
    );
    for result in [
        vm.eval("JSON.stringify(nativeFormRows)").unwrap(),
        vm.eval_in_isolated_context(isolated, "JSON.stringify(nativeFormRows)")
            .unwrap(),
    ] {
        let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 8, "{rows}");
        for row in rows.as_array().unwrap() {
            for (field, value) in row.as_object().unwrap() {
                assert_eq!(value, true, "{field}: {row}");
            }
        }
    }
}

async fn native_ui_test_vm() -> crate::runtime::PageVmTaskExecutorTestHarness {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://native-ui-target-window.test/",
        &loader,
    );
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame=document.body.appendChild(document.createElement('iframe'));
        frame.id='child';
        frame.srcdoc='<body></body>';
        void frame.contentWindow;
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
    vm
}
