use super::*;

#[test]
fn canvas_shadow_color_uses_native_document_connection() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-color-membership.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(() => {{
        const probe = {};
        const realms = [globalThis, document.getElementById('child').contentWindow];
        const checks = [];
        for(let owner=0;owner<2;owner++) for(let callee=0;callee<2;callee++)
            checks.push(...probe(realms[owner],realms[callee],owner+'-'+callee));
        globalThis.__canvasMembershipChecks=checks;
        return checks.length;
    }})()"#,
        include_str!("canvas_currentcolor_membership.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "44");
    assert_eq!(
        vm.eval("JSON.stringify(__canvasMembershipChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn canvas_style_attributes_validate_convert_and_restore_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-style-state.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(() => {{
        const probe = {};
        const realms = [globalThis, document.getElementById('child').contentWindow];
        const checks = [];
        for (let owner = 0; owner < 2; owner++) for (let callee = 0; callee < 2; callee++)
            for (const kind of ['element', 'offscreen'])
                checks.push(...probe(realms[owner], realms[callee], kind, owner + '-' + callee + '-' + kind));
        globalThis.__canvasStyleChecks = checks;
        return checks.length;
    }})()"#,
        include_str!("canvas_style_state.js")
    );
    let count: usize = vm.eval(&source).unwrap().parse().unwrap();
    assert_eq!(count, 2_136, "the complete realm matrix must execute");
    assert_eq!(
        vm.eval("JSON.stringify(__canvasStyleChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn canvas_shadow_color_resolves_after_conversion_without_author_accessors() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://canvas-shadow-current-color.test/");
    assert_eq!(vm.eval(r#"(() => {
        const canvas = document.createElement('canvas'); document.body.appendChild(canvas);
        canvas.style.color = 'rgb(1, 2, 3)';
        const context = canvas.getContext('2d');
        let hooks = 0;
        Object.defineProperty(context, 'canvas', { get() { hooks++; throw Error('author canvas'); } });
        Object.defineProperty(canvas, 'ownerDocument', { get() { hooks++; throw Error('author document'); } });
        context.shadowColor = 'currentcolor';
        if (context.shadowColor !== '#010203') throw Error('currentcolor');
        context.save();
        context.shadowColor = { toString() { canvas.style.color = 'rgb(4, 5, 6)'; return 'currentcolor'; } };
        if (context.shadowColor !== '#040506') throw Error('conversion ordering');
        context.restore();
        return hooks === 0 && context.shadowColor === '#010203';
    })()"#).unwrap(), "true");
}

#[test]
fn canvas_style_attributes_share_registered_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-style-native-proxy.test/");
    vm.eval("globalThis.styleContexts = [document.createElement('canvas').getContext('2d'), new OffscreenCanvas(8,8).getContext('2d')]").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "styleContexts");
        let contexts =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let mut proxies = Vec::new();
        for index in 0..contexts.length() {
            let context =
                v8::Local::<v8::Object>::try_from(contexts.get_index(scope, index).unwrap())
                    .unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, context, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            proxies.push(proxy.into());
        }
        let proxies = v8::Array::new_with_elements(scope, &proxies);
        let key = crate::util::v8str(scope, "styleProxies");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxies.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        for (let index=0; index<2; index++) {
            const real=styleContexts[index], proxy=styleProxies[index];
            const proto=(index===0?CanvasRenderingContext2D:OffscreenCanvasRenderingContext2D).prototype;
            for (const [name,value] of [['fillStyle','#abcdef'],['strokeStyle','#abcdef'],['shadowBlur',2],['shadowOffsetX',-2],['shadowOffsetY',3],['shadowColor','#abcdef'],['textAlign','end'],['textBaseline','bottom']]) {
                const d=Object.getOwnPropertyDescriptor(proto,name);
                d.set.call(proxy,value); if(d.get.call(real)!==value) throw Error(name+' target');
                d.set.call(real); if(d.get.call(real)!==value) throw Error(name+' missing value');
                proto.save.call(proxy); proto.reset.call(proxy); d.set.call(real,value);
                if(d.get.call(proxy)!==value) throw Error(name+' proxy');
                let calls=0, rejected=false;
                try { d.set.call(new Proxy(proxy,{}), { [Symbol.toPrimitive]() {calls++; return value;} }); }
                catch(error) {rejected=error instanceof TypeError;}
                if(!rejected || calls!==0) throw Error(name+' author proxy');
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
