use super::*;

#[test]
fn offscreen_canvas_transfer_owns_pixels_and_preserves_context_state_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://offscreen-transfer.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "globalThis.probe={};",
        include_str!("../../../tests/fixtures/canvas-offscreen-transfer-checks.js")
    ))
    .unwrap();
    let result:serde_json::Value=serde_json::from_str(&vm.eval(r#"JSON.stringify((() => {
      const other=document.querySelector('iframe').contentWindow;
      return [globalThis,other].flatMap(owner=>[globalThis,other].flatMap(callee=>probe(owner,callee,'window').checks));
    })())"#).unwrap()).unwrap();
    let checks = result.as_array().unwrap();
    assert_eq!(checks.len(), 116);
    let failures: Vec<_> = checks.iter().filter(|row| row["passed"] != true).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn offscreen_canvas_native_proxy_transfer_preserves_identity_and_pixel_ownership() {
    let mut vm = new_storage_page_task_executor_test_vm("https://offscreen-proxy.test/");
    vm.eval(r#"document.body.innerHTML='<iframe></iframe>';
      globalThis.nativeOffscreen=new OffscreenCanvas(2,1);
      const context=nativeOffscreen.getContext('2d');context.fillStyle='#00ff00';context.fillRect(0,0,2,1);
    "#).unwrap();
    let ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let target = global
            .get(scope, crate::util::v8str(scope, "nativeOffscreen").into())
            .unwrap();
        let target = v8::Local::<v8::Object>::try_from(target).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
        global.create_data_property(
            scope,
            crate::util::v8str(scope, "offscreenProxy").into(),
            proxy.into(),
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const transfer=document.querySelector('iframe').contentWindow.OffscreenCanvas.prototype.transferToImageBitmap;
      const first=transfer.call(offscreenProxy);
      const context=nativeOffscreen.getContext('2d');context.fillStyle='#0000ff';context.fillRect(0,0,2,1);
      const second=transfer.call(offscreenProxy), read=new OffscreenCanvas(2,1).getContext('2d');
      read.drawImage(first,0,0);if(Array.from(read.getImageData(0,0,1,1).data).join()!=='0,255,0,255')throw Error('old storage changed');
      read.drawImage(second,0,0);if(Array.from(read.getImageData(0,0,1,1).data).join()!=='0,0,255,255')throw Error('replacement storage');
      if(Object.getPrototypeOf(first)!==ImageBitmap.prototype)throw Error('owner realm');
      first.close();second.close();return true;
    })()"#).unwrap(),"true");
}
