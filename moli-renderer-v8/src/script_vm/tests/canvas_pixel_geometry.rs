use super::*;

#[test]
fn canvas_pixel_methods_preserve_rectangles_and_webidl_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-pixel-geometry.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    let script = format!(
        r#"(() => {{
        const probe = {};
        const realms = [globalThis, document.getElementById('child').contentWindow];
        const checks = [];
        for(let owner=0;owner<2;owner++) for(let callee=0;callee<2;callee++)
            for(const kind of ['element','offscreen'])
                checks.push(...probe(realms[owner],realms[callee],kind,owner+'-'+callee+'-'+kind));
        globalThis.__pixelGeometryChecks=checks;
        return checks.length;
    }})()"#,
        include_str!("canvas_pixel_geometry.js")
    );
    assert_eq!(vm.eval(&script).unwrap(), "3456");
    assert_eq!(
        vm.eval("JSON.stringify(__pixelGeometryChecks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn canvas_pixel_methods_share_registered_native_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-pixel-proxy.test/");
    vm.eval(r#"
        globalThis.pixelContexts=[document.createElement('canvas').getContext('2d'),new OffscreenCanvas(4,4).getContext('2d')];
        globalThis.pixelSource=new ImageData(new Uint8ClampedArray([255,0,0,255,0,255,0,255]),2,1);
        globalThis.pixelObjects=[...pixelContexts,pixelSource];
    "#).unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "pixelObjects");
        let objects =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let mut proxies = Vec::new();
        for index in 0..objects.length() {
            let target =
                v8::Local::<v8::Object>::try_from(objects.get_index(scope, index).unwrap())
                    .unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            proxies.push(proxy.into());
        }
        let proxies = v8::Array::new_with_elements(scope, &proxies);
        let key = crate::util::v8str(scope, "pixelProxies");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxies.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        for(let index=0;index<2;index++) {
            const proto=(index===0?CanvasRenderingContext2D:OffscreenCanvasRenderingContext2D).prototype;
            const context=pixelProxies[index], source=pixelProxies[2];
            proto.putImageData.call(context,source,0,0,1,0,1,1);
            const actual=Array.from(proto.getImageData.call(context,2,1,-2,-1).data).join(',');
            if(actual!=='0,0,0,0,0,255,0,255')throw Error('shared native rectangle '+actual);
            const blank=proto.createImageData.call(context,source);
            if(blank.width!==2||blank.height!==1||blank.data.some(value=>value!==0))throw Error('native source dimensions');
            let calls=0,rejected=false;
            try{proto.putImageData.call(context,new Proxy(source,{}),{valueOf(){calls++;return 0;}},0);}
            catch(error){rejected=error instanceof TypeError;}
            if(!rejected||calls!==0)throw Error('author source proxy');
        }
        return true;
    })()"#).unwrap(), "true");
}
