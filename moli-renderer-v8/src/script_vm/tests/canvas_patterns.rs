use super::*;

#[test]
fn native_canvas_patterns_snapshot_sources_and_share_transforms_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-patterns.test/");
    vm.eval(
        r#"
        document.body.innerHTML = '<iframe></iframe>';
        globalThis.patternRealms = [globalThis, document.querySelector('iframe').contentWindow];
    "#,
    )
    .unwrap();
    vm.eval(&format!(r#"
        const patternProbe = {};
        globalThis.patternChecks = [];
        for (let owner = 0; owner < 2; owner++)
            for (let callee = 0; callee < 2; callee++)
                for (const kind of ['element', 'offscreen'])
                    patternChecks.push(...patternProbe(patternRealms[owner], patternRealms[callee], kind, owner + '-' + callee + '-' + kind));
    "#, include_str!("canvas_patterns.js"))).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(patternChecks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("patternChecks.length").unwrap(), "304");
}

#[test]
fn native_pattern_proxies_preserve_identity_and_origin_clean_propagation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-pattern-proxy.test/");
    vm.eval(
        r#"
        globalThis.source = new OffscreenCanvas(2, 2);
        const sourceContext = source.getContext('2d');
        sourceContext.fillStyle = 'red'; sourceContext.fillRect(0, 0, 2, 2);
        globalThis.target = new OffscreenCanvas(4, 4);
        globalThis.context = target.getContext('2d');
    "#,
    )
    .unwrap();
    let ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, destination) in [("source", "nativeSource"), ("context", "nativeContext")] {
            let key = crate::util::v8str(scope, name);
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
            let key = crate::util::v8str(scope, destination);
            global
                .create_data_property(scope, key.into(), proxy.into())
                .unwrap();
        }
        Ok(())
    })
    .unwrap();
    vm.eval("globalThis.pattern = context.createPattern(nativeSource, 'repeat');")
        .unwrap();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "pattern");
        let object =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, object, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
        let key = crate::util::v8str(scope, "nativePattern");
        global
            .create_data_property(scope, key.into(), proxy.into())
            .unwrap();
        // Inject the native response flag independently of HTTP loading. The
        // original cross-origin WPTs exercise the network provenance separately.
        let key = crate::util::v8str(scope, "source");
        let source =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let flag = v8::Boolean::new(scope, false);
        crate::util::set_private_value(scope, source, "__moliCanvasOriginClean", flag.into());
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"
        context.fillStyle = nativePattern;
        CanvasPattern.prototype.setTransform.call(nativePattern, {e: 2});
        OffscreenCanvasRenderingContext2D.prototype.fillRect.call(nativeContext, 0, 0, 4, 4);
        context.fillStyle === nativePattern && Array.from(context.getImageData(2, 0, 1, 1).data).join() === '255,0,0,255'
    "#).unwrap(), "true");
    assert_eq!(vm.eval(r#"
        globalThis.dirtyPattern = context.createPattern(nativeSource, 'repeat');
        context.getImageData(0, 0, 1, 1); // Creating the pattern does not taint its context.
        context.fillStyle = dirtyPattern;
        globalThis.securityError = f => { try {f();} catch(e) {return e.name === 'SecurityError';} return false; };
        securityError(() => context.getImageData(0, 0, 1, 1))
    "#).unwrap(), "true");
    assert_eq!(
        vm.eval(
            r#"
        context.reset();
        securityError(() => context.getImageData(0, 0, 1, 1))
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"
        target.width = target.width;
        context.getImageData(0, 0, 1, 1);
        const bitmap = source.transferToImageBitmap();
        context.drawImage(bitmap, 0, 0);
        securityError(() => context.getImageData(0, 0, 1, 1)) &&
        securityError(() => source.getContext('2d').getImageData(0, 0, 1, 1)) &&
        securityError(() => new VideoFrame(source, {timestamp: 0}))
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"
        const html = document.createElement('canvas');
        html.getContext('2d').strokeStyle = dirtyPattern;
        html.getContext('2d').strokeStyle = 'blue';
        securityError(() => html.toDataURL()) && securityError(() => html.toBlob(() => {}))
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"
        globalThis.blobError = '';
        const result = target.convertToBlob();
        result.catch(e => {blobError = e.name;});
        result instanceof Promise
    "#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(vm.eval("blobError").unwrap(), "SecurityError");
}
