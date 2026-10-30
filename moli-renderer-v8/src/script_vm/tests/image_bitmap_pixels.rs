use super::*;

#[test]
fn image_bitmap_owns_native_pixels_independently_of_source_and_globals() {
    let mut vm = new_storage_page_task_executor_test_vm("https://bitmap-pixels.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.originalBitmap = ImageBitmap;
        globalThis.pixelSource = new OffscreenCanvas(2, 1);
        globalThis.pixelContext = pixelSource.getContext('2d');
        pixelContext.fillStyle = '#00ff00'; pixelContext.fillRect(0, 0, 2, 1);
        globalThis.ImageBitmap = function () { throw Error('author constructor'); };
        createImageBitmap(pixelSource).then(value => globalThis.pixelBitmap = value);
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval("pixelBitmap instanceof originalBitmap && pixelBitmap.width === 2 && pixelBitmap.height === 1").unwrap(), "true");
    vm.eval("pixelContext.clearRect(0,0,2,1); pixelSource.width = 9;")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let bitmap = global
            .get(scope, crate::util::v8str(scope, "pixelBitmap").into())
            .unwrap();
        let bitmap = v8::Local::<v8::Object>::try_from(bitmap).unwrap();
        assert_eq!(
            crate::context_bootstrap::image_bitmap_pixels_for_test(scope, bitmap),
            Some(vec![0, 255, 0, 255, 0, 255, 0, 255])
        );
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, bitmap, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativePixelBitmap").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const other=document.querySelector('iframe').contentWindow;
        for (const w of [window,other]) {
            const p=w === window ? originalBitmap.prototype : w.ImageBitmap.prototype;
            const width=Object.getOwnPropertyDescriptor(p,'width').get;
            if (width.call(nativePixelBitmap)!==2) throw Error('native proxy width');
            let conversions=0;
            for (const bad of [{},Object.create(pixelBitmap),new Proxy(pixelBitmap,{}),new Proxy(nativePixelBitmap,{})]) {
                for (const method of [width,p.close]) {
                    let error;try{method.call(bad)}catch(caught){error=caught}
                    if (!(error instanceof w.TypeError)) throw Error('callee brand error');
                }
            }
        }
        other.ImageBitmap.prototype.close.call(nativePixelBitmap);
        originalBitmap.prototype.close.call(pixelBitmap);
        return pixelBitmap.width === 0 && nativePixelBitmap.height === 0;
    })()"#).unwrap(),"true");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let bitmap = global
            .get(scope, crate::util::v8str(scope, "pixelBitmap").into())
            .unwrap();
        let bitmap = v8::Local::<v8::Object>::try_from(bitmap).unwrap();
        assert_eq!(
            crate::context_bootstrap::image_bitmap_pixels_for_test(scope, bitmap),
            None
        );
        Ok(())
    })
    .unwrap();
}
