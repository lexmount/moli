use super::*;

#[test]
fn canvas_transform_snapshots_copy_native_state_with_brands_and_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-transform.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(() => {{
            const probe = {};
            const realms = [globalThis, document.getElementById('child').contentWindow];
            const checks = [];
            for (let owner = 0; owner < 2; owner++) for (let callee = 0; callee < 2; callee++)
                for (const kind of ['element', 'offscreen'])
                    checks.push(...probe(realms[owner], realms[callee], kind, 'window-' + owner + '-' + callee + '-' + kind));
            globalThis.__canvasTransformChecks = checks;
            return checks.length;
        }})()"#,
        include_str!("canvas_transform_snapshots.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "600");
    assert_eq!(
        vm.eval("JSON.stringify(__canvasTransformChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn canvas_transform_snapshots_accept_registered_native_proxies_and_share_paint_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-transform-native.test/");
    vm.eval(
        r#"document.body.innerHTML = '<iframe id=child></iframe>';
        globalThis.nativeCanvasContexts = [document.createElement('canvas'), new OffscreenCanvas(96,96)]
            .map(canvas => canvas.getContext('2d'));
        for (const ctx of nativeCanvasContexts) ctx.setTransform(1,0,0,1,20,0);"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeCanvasContexts");
        let value = global.get(scope, key.into()).unwrap();
        let contexts = v8::Local::<v8::Array>::try_from(value).unwrap();
        let proxies = v8::Array::new(scope, 0);
        for index in 0..contexts.length() {
            let value = contexts.get_index(scope, index).unwrap();
            let context = v8::Local::<v8::Object>::try_from(value).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, context, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(proxies.set_index(scope, index, proxy.into()), Some(true));
        }
        let key = crate::util::v8str(scope, "nativeCanvasContextProxies");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxies.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
            const realms = [globalThis, document.getElementById('child').contentWindow];
            for (let index=0;index<2;index++) {
                const ctx=nativeCanvasContexts[index], proxy=nativeCanvasContextProxies[index];
                const name=index===0?'CanvasRenderingContext2D':'OffscreenCanvasRenderingContext2D';
                for (const realm of realms) {
                    const get=realm[name].prototype.getTransform;
                    const matrix=get.call(proxy);
                    if(matrix.e!==20 || !(matrix instanceof realm.DOMMatrix)) throw Error('native proxy snapshot');
                    matrix.e=80;
                    if(get.call(ctx).e!==20) throw Error('snapshot mutated paint state');
                    let rejected=false;
                    try {get.call(new Proxy(proxy,{}));} catch(error) {rejected=error instanceof realm.TypeError;}
                    if(!rejected) throw Error('author proxy accepted');
                }
                ctx.beginPath(); ctx.rect(0,0,10,10); ctx.fill();
                if(ctx.getImageData(25,5,1,1).data[3]!==255 || ctx.getImageData(5,5,1,1).data[3]!==0)
                    throw Error('snapshot transform differs from native painting');
                ctx.__moliCanvasPathState=0;
                Object.setPrototypeOf(ctx,null); Object.freeze(ctx);
                if(globalThis[name].prototype.getTransform.call(ctx).e!==20) throw Error('native identity lost');
            }
            return true;
        })()"#,
        )
        .unwrap(),
        "true"
    );
}
