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

#[test]
fn indexed_db_image_bitmap_pixels_survive_sqlite_profile_restart() {
    let root = indexed_db_test_root("image-bitmap-profile-restart");
    for write in [true, false] {
        let manager = crate::new_indexed_db_manager(Some(root.clone())).unwrap();
        let mut vm = new_storage_page_task_executor_test_vm(
            "https://indexeddb-image-bitmap-profile-restart.test/",
        );
        vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&manager)));
        vm.set_storage_bucket_store(
            crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager),
        );
        vm.eval(&format!("globalThis.writeBitmap = {write};"))
            .unwrap();
        vm.eval(
            r#"
globalThis.bitmapProfile = 'pending';
(async () => {
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const complete = transaction => new Promise((resolve, reject) => {
    transaction.oncomplete = resolve;
    transaction.onabort = () => reject(transaction.error);
  });
  const open = indexedDB.open('bitmaps', 1);
  open.onupgradeneeded = () => open.result.createObjectStore('records');
  const db = await result(open);
  if (writeBitmap) {
    const canvas = new OffscreenCanvas(2, 1);
    canvas.getContext('2d').putImageData(
      new ImageData(new Uint8ClampedArray([11,22,33,255,51,102,0,85]), 2, 1), 0, 0);
    const bitmap = canvas.transferToImageBitmap();
    Object.defineProperty(bitmap, 'width', {get() {throw Error('author width');}});
    const graph = {bitmap, alias:bitmap};
    graph.self = graph;
    const transaction = db.transaction('records', 'readwrite');
    const done = complete(transaction);
    transaction.objectStore('records').put(graph, 1);
    bitmap.close();
    await done;
  }
  const transaction = db.transaction('records');
  const done = complete(transaction);
  const graph = await result(transaction.objectStore('records').get(1));
  await done;
  db.close();
  globalThis.persistedBitmap = graph.bitmap;
  return [graph.bitmap instanceof ImageBitmap, graph.bitmap.width, graph.bitmap.height,
    graph.bitmap === graph.alias, graph.self === graph];
})().then(result => bitmapProfile = JSON.stringify(result),
          error => bitmapProfile = String(error));
"#,
        )
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("bitmapProfile").unwrap(),
            "[true,2,1,true,true]",
            "profile write: {write}"
        );
        let context_ptr = &vm.page_default_runtime.context as *const _;
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let bitmap = global
                .get(scope, crate::util::v8str(scope, "persistedBitmap").into())
                .unwrap();
            let bitmap = v8::Local::<v8::Object>::try_from(bitmap).unwrap();
            assert_eq!(
                crate::context_bootstrap::image_bitmap_pixels_for_test(scope, bitmap),
                Some(vec![11, 22, 33, 255, 51, 102, 0, 85]),
                "native RGBA pixels, profile write: {write}"
            );
            Ok(())
        })
        .unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}
