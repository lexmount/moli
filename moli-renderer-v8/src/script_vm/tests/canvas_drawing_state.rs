use super::*;

#[test]
fn canvas_drawing_stack_can_reset_unattached_native_contexts() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-state-unattached.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
                for (const Constructor of [CanvasRenderingContext2D, OffscreenCanvasRenderingContext2D]) {
                    const ctx = new Constructor();
                    ctx.lineWidth = 2; ctx.translate(3,4); ctx.save();
                    ctx.lineWidth = 3; ctx.resetTransform(); ctx.restore();
                    if(ctx.lineWidth!==2 || ctx.getTransform().e!==3) throw Error('unattached restore');
                    ctx.save(); ctx.reset(); ctx.restore();
                    if(ctx.lineWidth!==1 || ctx.getTransform().e!==0) throw Error('unattached reset retained state');
                }
                return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn canvas_drawing_stack_restores_native_attributes_transform_and_dash_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-state.test/");
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
            globalThis.__canvasStateChecks = checks;
            return checks.length;
        }})()"#,
        include_str!("canvas_drawing_state.js"),
    );
    let count: usize = vm.eval(&source).unwrap().parse().unwrap();
    assert_eq!(count, 1_340, "the complete realm matrix must execute");
    assert_eq!(
        vm.eval("JSON.stringify(__canvasStateChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn canvas_drawing_stack_accepts_registered_native_proxies_without_author_hooks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-state-proxy.test/");
    vm.eval(
        r#"document.body.innerHTML = '<iframe id=child></iframe>';
        globalThis.canvasStateContexts = [document.createElement('canvas'), new OffscreenCanvas(8,8)]
            .map(canvas => canvas.getContext('2d'));"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "canvasStateContexts");
        let contexts =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let mut proxies = Vec::new();
        for index in 0..contexts.length() {
            let value = contexts.get_index(scope, index).unwrap();
            let context = v8::Local::<v8::Object>::try_from(value).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, context, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            proxies.push(proxy.into());
        }
        let proxies = v8::Array::new_with_elements(scope, &proxies);
        let key = crate::util::v8str(scope, "canvasStateProxies");
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
                for (let i=0;i<2;i++) {
                    const real=canvasStateContexts[i], proxy=canvasStateProxies[i];
                    const name=i===0?'CanvasRenderingContext2D':'OffscreenCanvasRenderingContext2D';
                    for (const realm of realms) {
                        const p=realm[name].prototype;
                        real.lineWidth=2; real.setTransform(1,0,0,1,3,4); real.setLineDash([1,2]);
                        p.save.call(proxy);
                        real.lineWidth=5; real.resetTransform(); real.setLineDash([4,5]);
                        p.restore.call(proxy);
                        if(real.lineWidth!==2 || real.getTransform().e!==3 || real.getLineDash()[0]!==1)
                            throw Error('native proxy snapshot differs from native target');
                        for(const member of ['save','restore','reset']) {
                            let rejected=false;
                            try {p[member].call(new Proxy(proxy,{}));} catch(error) {rejected=error instanceof realm.TypeError;}
                            if(!rejected) throw Error('author proxy accepted');
                        }
                        p.save.call(proxy); p.reset.call(proxy); p.restore.call(proxy);
                        if(real.lineWidth!==1 || real.getTransform().e!==0 || real.getLineDash().length!==0)
                            throw Error('native proxy reset retained stack');
                    }
                }
                return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
