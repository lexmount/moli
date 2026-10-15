use super::*;

#[test]
fn native_canvas_shadows_preserve_unclipped_sources_and_ordered_compositing() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-shadows.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>';globalThis.shadowRealms=[globalThis,document.querySelector('iframe').contentWindow];").unwrap();
    vm.eval(&format!(
        "const probe={};globalThis.shadowChecks=[];for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)for(const kind of ['element','offscreen'])shadowChecks.push(...probe(shadowRealms[owner],shadowRealms[callee],kind,owner+'-'+callee+'-'+kind));",
        include_str!("canvas_shadows.js")
    )).unwrap();
    assert_eq!(vm.eval("shadowChecks.length").unwrap(), "200");
    assert_eq!(
        vm.eval("JSON.stringify(shadowChecks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn native_canvas_compositing_applies_operators_across_drawing_consumers_and_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-compositing.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>';globalThis.compositeRealms=[globalThis,document.querySelector('iframe').contentWindow];").unwrap();
    let vectors = serde_json::to_string(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-canvas/src/composite_chromium_145.tsv"
    )))
    .unwrap();
    vm.eval(&format!(
        "const probe={};globalThis.compositeChecks=[];for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)for(const kind of ['element','offscreen'])compositeChecks.push(...probe(compositeRealms[owner],compositeRealms[callee],kind,owner+'-'+callee+'-'+kind,{}));",
        include_str!("canvas_compositing.js"),vectors
    )).unwrap();
    assert_eq!(vm.eval("compositeChecks.length").unwrap(), "5248");
    assert_eq!(
        vm.eval("JSON.stringify(compositeChecks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn native_canvas_compositing_uses_registered_proxy_state_without_public_getters() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-compositing.test/");
    vm.eval("globalThis.context=new OffscreenCanvas(4,4).getContext('2d');context.fillStyle='red';context.fillRect(0,0,4,4);").unwrap();
    let ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "context");
        let target =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeContext");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const prototype=OffscreenCanvasRenderingContext2D.prototype;
        Object.getOwnPropertyDescriptor(prototype,'globalCompositeOperation').set.call(nativeContext,'copy');
        Object.getOwnPropertyDescriptor(prototype,'globalAlpha').set.call(nativeContext,0);
        Object.defineProperty(context,'globalCompositeOperation',{get(){throw Error('public operator getter');}});
        prototype.strokeRect.call(nativeContext,0,0,2,2);
        return Array.from(context.getImageData(3,3,1,1).data).join()==='0,0,0,0';
    })()"#).unwrap(),"true");
}
