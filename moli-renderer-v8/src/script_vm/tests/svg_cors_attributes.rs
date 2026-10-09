use super::*;

#[test]
fn svg_cors_attributes_reflect_native_utf16_and_validate_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-cors-attributes.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_cors_attributes.js")).unwrap();
    assert_eq!(vm.eval("__svgCorsResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__svgCorsResults.total").unwrap(), "1824");
    assert_eq!(
        vm.eval("JSON.stringify(__svgCorsResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
}

#[test]
fn svg_cors_attributes_accept_native_proxies_and_preserve_owner_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-cors-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.child = document.querySelector('iframe').contentWindow;
        const doc = child.document.implementation.createHTMLDocument('');
        for (const tag of ['image','script','feImage']) {
            globalThis[tag] = doc.createElementNS('http://www.w3.org/2000/svg',tag);
        }
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for tag in ["image", "script", "feImage"] {
            let key = crate::util::v8str(scope, tag);
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8_string(scope, &format!("{tag}Proxy")).unwrap();
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const first = Object.getOwnPropertyDescriptor(SVGFEImageElement.prototype,'crossOrigin').get.call(feImageProxy);
        if (Object.getPrototypeOf(first) !== child.SVGAnimatedString.prototype || first !== feImage.crossOrigin) throw Error('first access owner realm');
        first.baseVal='USE-CREDENTIALS';
        if (feImage.getAttribute('crossorigin') !== 'USE-CREDENTIALS' || first.baseVal !== 'use-credentials') throw Error('animated writeback');
        for (const realm of [window,child]) {
            for (const [tag,iface] of [['image','SVGImageElement'],['script','SVGScriptElement']]) {
                const descriptor=Object.getOwnPropertyDescriptor(realm[iface].prototype,'crossOrigin');
                let conversions=0;
                descriptor.set.call(globalThis[tag+'Proxy'],{toString(){conversions++;return '\ud800';}});
                if (conversions!==1 || globalThis[tag].getAttribute('crossorigin')!=='\ud800' || descriptor.get.call(globalThis[tag+'Proxy'])!=='anonymous') throw Error('native UTF16 writeback');
                const author=new Proxy(globalThis[tag+'Proxy'],{get(){throw 42;},getPrototypeOf(){throw 42;}});
                let error;
                try {descriptor.set.call(author,{toString(){conversions++;return 'anonymous';}});} catch(caught) {error=caught;}
                if (!error || Object.getPrototypeOf(error)!==realm.TypeError.prototype || conversions!==1) throw Error('author proxy brand');
            }
            const get=Object.getOwnPropertyDescriptor(realm.SVGFEImageElement.prototype,'crossOrigin').get;
            if (get.call(feImageProxy)!==first) throw Error('native animated identity');
        }
        return true;
    })()"#).unwrap(), "true");
}
