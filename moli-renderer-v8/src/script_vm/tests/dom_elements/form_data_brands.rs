use super::*;

#[tokio::test]
async fn form_data_constructor_accepts_native_forms_across_document_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://form-data-brands.test/",
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
    assert_eq!(vm.eval(include_str!("form_data_brands.js")).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}

#[test]
fn form_data_internal_construction_ignores_author_constructors() {
    let mut vm = new_storage_page_task_executor_test_vm("https://form-data-intrinsics.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
      const Data = FormData, DataEvent = FormDataEvent;
      document.body.innerHTML = '<form><input name="original" value="value"></form>';
      const form = document.querySelector('form');
      let event, reads = 0;
      form.addEventListener('formdata', value => {
        event = value;
        value.formData.append('listener', 'retained');
      });
      for (const name of ['FormData', 'FormDataEvent']) {
        Object.defineProperty(globalThis, name, {configurable:true, get() {
          reads++;
          throw Error('public constructor must not supply an internal object');
        }});
      }
      const data = new Data(form);
      return reads === 0 && event instanceof DataEvent && event.isTrusted &&
        event.formData instanceof Data && data.get('original') === 'value' &&
        data.get('listener') === 'retained';
    })()"#
        )
        .unwrap(),
        "true"
    );
}
