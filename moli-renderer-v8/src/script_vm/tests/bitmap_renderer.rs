use super::*;

#[test]
fn bitmap_renderer_surface_preserves_realms_conversion_and_native_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://bitmap-renderer.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "globalThis.probe={};",
        include_str!("../../../tests/fixtures/canvas-bitmap-renderer-surface.js")
    ))
    .unwrap();
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval(
            r#"JSON.stringify((() => {
        const other=document.querySelector('iframe').contentWindow;
        return [globalThis,other].flatMap(owner => [globalThis,other].flatMap(callee =>
            ['element','offscreen'].flatMap(kind => probe(owner,callee,kind,kind))));
    })())"#,
        )
        .unwrap(),
    )
    .unwrap();
    let checks = result.as_array().unwrap();
    assert_eq!(checks.len(), 248);
    let failures: Vec<_> = checks.iter().filter(|row| row["passed"] != true).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn bitmap_renderer_moves_pixels_detaches_sources_and_clears_to_native_blank_surfaces() {
    let mut vm = new_storage_page_task_executor_test_vm("https://bitmap-pixels.test/");
    vm.eval(
        r#"
      document.body.innerHTML='<iframe></iframe>';
      const source=new OffscreenCanvas(2,1), ctx=source.getContext('2d');
      ctx.fillStyle='rgb(255,0,0)'; ctx.fillRect(0,0,2,1);
      Promise.all(Array.from({length:8},()=>createImageBitmap(source)))
        .then(bitmaps=>globalThis.inputBitmaps=bitmaps);
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,name)=>{if(!ok)throw Error(name)};
      assert(inputBitmaps.length===8,'all bitmap copies settled');
      const other=document.querySelector('iframe').contentWindow;
      let index=0;
      for(const owner of [globalThis,other]) for(const alpha of [true,false]) {
        for(const kind of ['element','offscreen']) {
          const destination=kind==='element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(7,5);
          const renderer=destination.getContext('bitmaprenderer',{alpha});
          const input=inputBitmaps[index++];
          renderer.transferFromImageBitmap(input);
          assert(input.width===0 && input.height===0,'source detached');
          const output=new OffscreenCanvas(2,1).getContext('2d');
          output.drawImage(destination,0,0);
          assert(JSON.stringify(Array.from(output.getImageData(0,0,2,1).data))==='[255,0,0,255,255,0,0,255]','transferred red pixels');
          if(kind==='element') assert(destination.toDataURL().startsWith('data:image/png;base64,'),'PNG export');
          let error;try{renderer.transferFromImageBitmap(input)}catch(caught){error=caught}
          assert(error instanceof owner.DOMException && error.name==='InvalidStateError','closed source in callback realm');
          renderer.transferFromImageBitmap(null);
          output.clearRect(0,0,2,1);output.drawImage(destination,0,0);
          const blank=Array.from(output.getImageData(0,0,2,1).data);
          assert(blank.every((value,i)=>value===(!alpha && i%4===3 ? 255 : 0)),'alpha-aware blank output');
        }
      }
      return true;
    })()"#).unwrap(),"true");
}

#[test]
fn bitmap_renderer_transfers_registered_native_proxies_and_shares_source_detachment() {
    let mut vm = new_storage_page_task_executor_test_vm("https://bitmap-proxy.test/");
    vm.eval(
        r#"
      globalThis.nativeRenderer=new OffscreenCanvas(2,1).getContext('bitmaprenderer');
      const source=new OffscreenCanvas(1,1);source.getContext('2d').fillRect(0,0,1,1);
      createImageBitmap(source).then(bitmap=>globalThis.nativeBitmap=bitmap);
    "#,
    )
    .unwrap();
    vm.register_bitmap_renderer_proxies_for_test().unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const transfer=ImageBitmapRenderingContext.prototype.transferFromImageBitmap;
      const canvas=Object.getOwnPropertyDescriptor(ImageBitmapRenderingContext.prototype,'canvas').get;
      const width=Object.getOwnPropertyDescriptor(ImageBitmap.prototype,'width').get;
      if(canvas.call(rendererProxy)!==nativeRenderer.canvas || width.call(bitmapProxy)!==1) throw Error('native proxy brand');
      let traps=0,error;
      try{transfer.call(rendererProxy,new Proxy(bitmapProxy,{get(){traps++},getPrototypeOf(){traps++}}))}catch(caught){error=caught}
      if(!(error instanceof TypeError) || traps!==0 || nativeBitmap.width!==1) throw Error('author proxy rejection');
      transfer.call(rendererProxy,bitmapProxy);
      if(nativeBitmap.width!==0 || width.call(bitmapProxy)!==0) throw Error('shared detachment');
      const read=new OffscreenCanvas(1,1).getContext('2d');read.drawImage(canvas.call(rendererProxy),0,0);
      if(JSON.stringify(Array.from(read.getImageData(0,0,1,1).data))!=='[0,0,0,255]') throw Error('pixel ownership');
      return true;
    })()"#).unwrap(),"true");
}

impl ScriptVm {
    // Register wrappers around objects produced by the real page task executor.
    // Author-created Proxy objects in the fixtures remain unregistered.
    pub(crate) fn register_bitmap_renderer_proxies_for_test(&mut self) -> anyhow::Result<()> {
        let context_ptr = &self.page_default_runtime.context as *const _;
        self.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            for (target_name, proxy_name) in [
                ("nativeRenderer", "rendererProxy"),
                ("nativeBitmap", "bitmapProxy"),
            ] {
                let key = crate::util::v8str(scope, target_name);
                let target =
                    v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap())
                        .unwrap();
                let handler = crate::util::new_null_prototype_object(scope);
                let proxy = v8::Proxy::new(scope, target, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
                let key = crate::util::v8str(scope, proxy_name);
                assert_eq!(
                    global.create_data_property(scope, key.into(), proxy.into()),
                    Some(true)
                );
            }
            Ok(())
        })
    }
}
