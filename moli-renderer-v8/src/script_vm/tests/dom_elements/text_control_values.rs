use super::*;

#[tokio::test]
async fn text_control_values_preserve_utf16_across_document_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://text-control-values.test/",
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
    assert_eq!(vm.eval(include_str!("text_control_values.js")).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}

#[test]
fn form_data_converts_utf16_control_values_at_the_submission_boundary() {
    let mut vm = new_storage_html_test_vm("https://form-value-utf16.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
      const form=document.body.appendChild(document.createElement('form'));
      for (const tag of ['input','textarea']) {
        const control=form.appendChild(document.createElement(tag));
        const name=tag+'-\uDC01',value='value-\uD800';
        control.name=name; control.value=value;
        if (control.name!==name || control.value!==value) return false;
      }
      const entries=Array.from(new FormData(form));
      return entries.length===2 && entries.every(([name,value],index)=>
        name===['input','textarea'][index]+'-\uFFFD' && value==='value-\uFFFD');
    })()"#
        )
        .unwrap(),
        "true"
    );
}
