use super::*;

#[test]
fn canvas_fill_rect_uses_native_receivers_transforms_and_alpha_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-fill-rect.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(() => {{
            const probe = {};
            const realms = [globalThis, document.getElementById('child').contentWindow];
            const checks = [];
            for(let owner=0;owner<2;owner++) for(let callee=0;callee<2;callee++)
                for(const kind of ['element','offscreen'])
                    checks.push(...probe(realms[owner],realms[callee],kind,`window-${{owner}}-${{callee}}-${{kind}}`));
            globalThis.__fillRectChecks = checks;
            return checks.length;
        }})()"#,
        include_str!("canvas_fill_rect.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "328");
    assert_eq!(
        vm.eval("JSON.stringify(__fillRectChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn canvas_fill_rect_registered_native_proxy_keeps_context_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-fill-rect-proxy.test/");
    vm.eval(
        "globalThis.ctx = new OffscreenCanvas(64,64).getContext('2d');\
         ctx.setTransform(1,0,0,1,20,10); ctx.fillStyle='red'; ctx.globalAlpha=.5;",
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "ctx");
        let value = global.get(scope, key.into()).unwrap();
        let target = v8::Local::<v8::Object>::try_from(value).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
                const fillRect = OffscreenCanvasRenderingContext2D.prototype.fillRect;
                fillRect.call(nativeProxy,2,2,8,8);
                const hit = Array.from(ctx.getImageData(24,14,1,1).data);
                if(JSON.stringify(hit)!=='[255,0,0,128]') throw Error('native proxy state');
                let conversions=0, caught;
                try {fillRect.call(new Proxy(nativeProxy,{}),{valueOf(){conversions++;return 2;}},2,8,8);}
                catch(error){caught=error;}
                if(!(caught instanceof TypeError) || conversions!==0) throw Error('author proxy branding');
                return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
