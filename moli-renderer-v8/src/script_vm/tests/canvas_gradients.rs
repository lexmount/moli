use super::*;

#[test]
fn native_canvas_gradients_apply_mutable_styles_geometry_and_colors_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-gradients.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>';globalThis.gradientRealms=[globalThis,document.querySelector('iframe').contentWindow];").unwrap();
    vm.eval(&format!("const gradientProbe={};globalThis.gradientChecks=[];for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)for(const kind of ['element','offscreen'])gradientChecks.push(...gradientProbe(gradientRealms[owner],gradientRealms[callee],kind,owner+'-'+callee+'-'+kind));",include_str!("canvas_gradients.js"))).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(gradientChecks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("gradientChecks.length").unwrap(), "360");
}

#[test]
fn native_gradient_proxies_share_stops_and_keep_assigned_style_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://gradient-native-proxy.test/");
    vm.eval("globalThis.context=new OffscreenCanvas(4,4).getContext('2d');globalThis.gradient=context.createLinearGradient(0,0,4,0);").unwrap();
    let ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (source, destination) in [("gradient", "nativeGradient"), ("context", "nativeContext")]
        {
            let key = crate::util::v8str(scope, source);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
            let key = crate::util::v8str(scope, destination);
            global
                .create_data_property(scope, key.into(), proxy.into())
                .unwrap();
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("CanvasGradient.prototype.addColorStop.call(nativeGradient,0,'blue');context.fillStyle=nativeGradient;context.fillRect(0,0,4,4);context.fillStyle===nativeGradient&&Array.from(context.getImageData(1,1,1,1).data).join()==='0,0,255,255'").unwrap(),"true");
    assert_eq!(vm.eval("context.reset();context.globalAlpha=0.5;context.strokeStyle=nativeGradient;context.lineWidth=2;OffscreenCanvasRenderingContext2D.prototype.strokeRect.call(nativeContext,0,0,4,4);Array.from(context.getImageData(0,0,1,1).data).join()==='0,0,255,128'").unwrap(),"true");
    assert_eq!(vm.eval("context.clearRect(0,0,4,4);context.globalAlpha=1;context.fillStyle='red';context.strokeStyle=nativeGradient;OffscreenCanvasRenderingContext2D.prototype.strokeText.call(nativeContext,'MM',0,4);const textPixels=context.getImageData(0,0,4,4).data;let painted=false;for(let i=0;i<textPixels.length;i+=4){if(textPixels[i+3]){painted=true;if(textPixels[i]!==0||textPixels[i+1]!==0||textPixels[i+2]!==255)throw Error('stroke paint');}}painted").unwrap(),"true");
}
