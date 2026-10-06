use super::*;

#[test]
fn svg_filter_enumerations_reflect_native_attributes_with_webidl_receiver_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-enums.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_filter_enumerations.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "5744");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_filter_enumerations_accept_registered_native_proxies_for_values_and_elements() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-filter-enum-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.childWindow = document.querySelector('iframe').contentWindow;
        const doc = childWindow.document.implementation.createHTMLDocument('');
        globalThis.definitions = [
            ['feComposite', 'SVGFECompositeElement', 'operator', 'arithmetic', 6],
            ['feGaussianBlur', 'SVGFEGaussianBlurElement', 'edgeMode', 'wrap', 2],
            ['feConvolveMatrix', 'SVGFEConvolveMatrixElement', 'edgeMode', 'none', 3],
            ['feMorphology', 'SVGFEMorphologyElement', 'operator', 'dilate', 2],
            ['feDisplacementMap', 'SVGFEDisplacementMapElement', 'xChannelSelector', 'A', 4],
        ];
        for (const [tag, , property, keyword] of definitions) {
            const element = doc.createElementNS('http://www.w3.org/2000/svg', tag);
            element.setAttribute(property, keyword);
            globalThis[tag] = element;
            globalThis[tag + 'Value'] = element[property];
        }
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for tag in [
            "feComposite",
            "feGaussianBlur",
            "feConvolveMatrix",
            "feMorphology",
            "feDisplacementMap",
        ] {
            for suffix in ["", "Value"] {
                let name = format!("{tag}{suffix}");
                let key = crate::util::v8_string(scope, &name).unwrap();
                let object = global.get(scope, key.into()).unwrap();
                let object = v8::Local::<v8::Object>::try_from(object).unwrap();
                let handler = crate::util::new_null_prototype_object(scope);
                let proxy = v8::Proxy::new(scope, object, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                let key = crate::util::v8_string(scope, &format!("{name}Proxy")).unwrap();
                assert_eq!(
                    global.create_data_property(scope, key.into(), proxy.into()),
                    Some(true)
                );
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
            for (const realm of [window, childWindow]) {
                const prototype = realm.SVGAnimatedEnumeration.prototype;
                const base = Object.getOwnPropertyDescriptor(prototype, 'baseVal');
                const anim = Object.getOwnPropertyDescriptor(prototype, 'animVal');
                for (const [tag, iface, property, keyword, expected] of definitions) {
                    const element = globalThis[tag];
                    const value = globalThis[tag + 'Value'];
                    const nativeValue = globalThis[tag + 'ValueProxy'];
                    const getter = Object.getOwnPropertyDescriptor(realm[iface].prototype, property).get;
                    if (getter.call(globalThis[tag + 'Proxy']) !== value ||
                        base.get.call(nativeValue) !== expected || anim.get.call(nativeValue) !== expected) {
                        throw Error('registered proxy lost identity or reflected value');
                    }
                    let conversions = 0;
                    base.set.call(nativeValue, {valueOf() {conversions++; return expected + 65536;}});
                    if (conversions !== 1 || element.getAttribute(property) !== keyword ||
                        value.baseVal !== expected || value.animVal !== expected) throw Error('native proxy writeback');
                    let traps = 0;
                    const author = new Proxy(nativeValue, {
                        get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
                    });
                    const revoked = Proxy.revocable(nativeValue, {}); revoked.revoke();
                    for (const receiver of [author, revoked.proxy, Object.create(nativeValue)]) {
                        for (const fn of [base.get, anim.get, base.set]) {
                            let error;
                            try {fn.call(receiver, {valueOf() {conversions++; return expected;}});}
                            catch (caught) {error = caught;}
                            if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype ||
                                conversions !== 1 || traps !== 0) throw Error('invalid proxy receiver');
                        }
                    }
                }
            }
            return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
