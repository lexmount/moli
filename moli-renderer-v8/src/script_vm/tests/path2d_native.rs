use super::*;

#[test]
fn path2d_native_geometry_copy_add_path_and_drawing_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://path2d-native.test/");
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
            globalThis.__path2dChecks = checks;
            return checks.length;
        }})()"#,
        include_str!("path2d_native.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "1280");
    assert_eq!(
        vm.eval("JSON.stringify(__path2dChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn path2d_registered_native_proxies_share_state_and_reject_author_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://path2d-proxy.test/");
    vm.eval(
        r#"globalThis.path = new Path2D(); path.rect(10,10,20,20);
        globalThis.ctx = new OffscreenCanvas(96,96).getContext('2d');"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("path", "pathProxy"), ("ctx", "ctxProxy")] {
            let key = crate::util::v8str(scope, name);
            let value = global.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(value).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, proxy_name);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(r#"(() => {
            const p = new Path2D(pathProxy);
            Path2D.prototype.addPath.call(pathProxy, pathProxy, {e:40});
            const fill = OffscreenCanvasRenderingContext2D.prototype.fill;
            fill.call(ctxProxy,pathProxy);
            if(ctx.getImageData(20,20,1,1).data[3] !== 255 || ctx.getImageData(60,20,1,1).data[3] !== 255)
                throw Error('native proxy identity/state');
            ctx.clearRect(0,0,96,96); fill.call(ctxProxy,p);
            if(ctx.getImageData(60,20,1,1).data[3] !== 0) throw Error('copy independence');
            let reads=0;
            const bad=new Proxy(pathProxy, {get(){reads++;throw Error('trap');}});
            for(const run of [() => Path2D.prototype.addPath.call(pathProxy,bad,{get a(){reads++;return 1;}}),
                () => Path2D.prototype.rect.call(bad, {valueOf(){reads++;return 1;}}, 1,2,3),
                () => fill.call(new Proxy(ctxProxy,{}),p)]) {
                let rejected=false; try {run();} catch(e) {rejected=e instanceof TypeError;}
                if(!rejected) throw Error('author proxy accepted');
            }
            if(reads!==0) throw Error('brand failure ran author code');
            return true;
        })()"#).unwrap(),
        "true"
    );
}
