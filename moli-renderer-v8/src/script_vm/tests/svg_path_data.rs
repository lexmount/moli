use super::*;

#[test]
fn svg_path_data_converts_dictionaries_and_preserves_base_path_mutations() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-path-data.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_path_data.js")).unwrap();
    assert_eq!(vm.eval("__svgPathDataResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__svgPathDataResults.total").unwrap(), "1264");
    assert_eq!(
        vm.eval("JSON.stringify(__svgPathDataResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_path_data_accepts_registered_native_proxies_and_updates_geometry() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-path-data-proxy.test/");
    vm.eval(
        r#"
        document.body.innerHTML='<svg><path id="path" d="M0 0L3 4"/></svg><iframe></iframe>';
        globalThis.child=document.querySelector('iframe').contentWindow;
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "path");
        let path =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, path, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const path=document.querySelector('#path');
        for (const realm of [window, child]) {
            const get=realm.SVGPathElement.prototype.getPathData;
            const set=realm.SVGPathElement.prototype.setPathData;
            path.setAttribute('d','M0 0L3 4');
            if (path.getTotalLength()!==5 || get.call(nativeProxy)[1].type!=='L') throw Error('proxy read');
            const input=[{type:'M',values:[0,0]},{type:'L',values:[6,8]}];
            set.call(nativeProxy,input);
            input[1].values[0]=900;
            if (path.getTotalLength()!==10 || get.call(nativeProxy)[1].values[0]!==6) throw Error('path mutation');
            let reads=0, traps=0;
            const author=new Proxy(nativeProxy,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});
            const revoked=Proxy.revocable(nativeProxy,{});revoked.revoke();
            for (const receiver of [author,revoked.proxy,Object.create(nativeProxy)]) {
                for (const method of [get,set]) {
                    let error;
                    const poison=new Proxy({}, {get(){reads++;throw 42;}});
                    try {method.call(receiver,poison);} catch(e) {error=e;}
                    if (!error || Object.getPrototypeOf(error)!==realm.TypeError.prototype || reads || traps) throw Error('author receiver accepted');
                }
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
