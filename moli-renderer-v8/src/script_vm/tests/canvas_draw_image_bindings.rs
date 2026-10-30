use super::*;

#[test]
fn draw_image_native_bindings_validate_arguments_and_snapshot_after_conversion_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://draw-image-bindings.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "globalThis.probe={};",
        include_str!("../../../tests/fixtures/canvas-draw-image-bindings.js")
    ))
    .unwrap();
    let result:serde_json::Value=serde_json::from_str(&vm.eval(r#"JSON.stringify((()=>{
      const realms=[globalThis,document.querySelector('iframe').contentWindow],checks=[];
      let matrices=0;for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)for(const kind of ['element','offscreen']){
        matrices++;checks.push(...probe(realms[owner],realms[callee],kind,owner+'-'+callee+'-'+kind));
      }
      return {matrices,checks};
    })())"#).unwrap()).unwrap();
    assert_eq!(result["matrices"], 8);
    let checks = result["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 104);
    let failures: Vec<_> = checks.iter().filter(|row| row["passed"] != true).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn draw_image_accepts_registered_native_context_canvas_and_bitmap_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://draw-image-proxies.test/");
    vm.eval(r#"globalThis.nativeContext=new OffscreenCanvas(2,1).getContext('2d');
      globalThis.nativeSource=new OffscreenCanvas(2,1);nativeSource.getContext('2d').fillRect(0,0,2,1);
      globalThis.nativeBitmap=nativeSource.transferToImageBitmap();
      nativeSource.getContext('2d').fillStyle='#00ff00';nativeSource.getContext('2d').fillRect(0,0,2,1);"#).unwrap();
    let ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [
            ("nativeContext", "contextProxy"),
            ("nativeSource", "sourceProxy"),
            ("nativeBitmap", "bitmapProxy"),
        ] {
            let object = global
                .get(scope, crate::util::v8str(scope, name).into())
                .unwrap();
            let object = v8::Local::<v8::Object>::try_from(object).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, alias).into(),
                    proxy.into()
                ),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(()=>{
      const draw=OffscreenCanvasRenderingContext2D.prototype.drawImage;
      draw.call(contextProxy,sourceProxy,0,0);
      if(Array.from(nativeContext.getImageData(0,0,1,1).data).join()!=='0,255,0,255')throw Error('native canvas proxy');
      draw.call(contextProxy,bitmapProxy,0,0);
      if(Array.from(nativeContext.getImageData(0,0,1,1).data).join()!=='0,0,0,255')throw Error('native bitmap proxy');
      return true;
    })()"#).unwrap(),"true");
}
