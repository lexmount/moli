use super::*;

#[test]
fn svg_smil_path_sampling_preserves_base_values_and_webidl_contracts() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-smil-path-timing.test/");
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_smil_path_timing.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__svgSmilPathTimingResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__svgSmilPathTimingResults.complete && __svgSmilPathTimingResults.total === 358 && __svgSmilPathTimingResults.passed === 358").unwrap(), "true");
}

#[test]
fn svg_time_controls_use_registered_native_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-smil-native-proxy.test/");
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    vm.eval("document.body.innerHTML = '<iframe></iframe>'; globalThis.svgReceiver = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); document.body.append(svgReceiver)").unwrap();
    let context = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let target = global
            .get(scope, crate::util::v8str(scope, "svgReceiver").into())
            .unwrap();
        let target = v8::Local::<v8::Object>::try_from(target).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::SVGSVGElement::is_instance(
            scope,
            proxy.into()
        ));
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "svgNativeProxy").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const realm = document.querySelector('iframe').contentWindow;
        const P = realm.SVGSVGElement.prototype;
        P.pauseAnimations.call(svgNativeProxy);
        P.setCurrentTime.call(svgNativeProxy, 9);
        if (P.getCurrentTime.call(svgReceiver) !== 9 || !P.animationsPaused.call(svgNativeProxy)) return false;
        let conversions = 0, traps = 0;
        const author = new Proxy(svgNativeProxy, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
        const revoked = Proxy.revocable(svgNativeProxy, {}); revoked.revoke();
        for (const receiver of [author, revoked.proxy, Object.create(svgReceiver)]) {
            try { P.setCurrentTime.call(receiver, {valueOf() { conversions++; return 3; }}); return false; }
            catch (error) { if (Object.getPrototypeOf(error) !== realm.TypeError.prototype) return false; }
        }
        return conversions === 0 && traps === 0 && P.getCurrentTime.call(svgReceiver) === 9;
    })()"#).unwrap(), "true");
}

#[test]
fn svg_fragment_clocks_preserve_pending_seek_and_document_begin() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-smil-document-begin.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<svg id="root"><svg id="nested"></svg></svg>';
        globalThis.rootSvg = document.getElementById('root');
        globalThis.nestedSvg = document.getElementById('nested');
        rootSvg.pauseAnimations(); rootSvg.setCurrentTime(7);
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval("rootSvg.getCurrentTime()").unwrap(), "0");
    vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)
        .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
        if (rootSvg.getCurrentTime() !== 7 || nestedSvg.getCurrentTime() !== 7) return false;
        nestedSvg.setCurrentTime(99); nestedSvg.unpauseAnimations();
        if (rootSvg.getCurrentTime() !== 7 || !nestedSvg.animationsPaused()) return false;
        const windowless = document.implementation.createHTMLDocument('');
        const svg = windowless.createElementNS('http://www.w3.org/2000/svg', 'svg');
        windowless.body.append(svg); svg.pauseAnimations(); svg.setCurrentTime(9);
        if (svg.getCurrentTime() !== 0) return false;
        document.body.append(svg);
        return svg.getCurrentTime() === 9 && svg.animationsPaused();
    })()"#
        )
        .unwrap(),
        "true"
    );
}
