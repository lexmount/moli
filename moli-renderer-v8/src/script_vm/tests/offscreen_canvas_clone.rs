use super::*;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn offscreen_canvas_transfer_preserves_native_state_and_receiving_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://offscreen-transfer.test/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        r#"(async()=>{{
        const probe={},realms=[globalThis,document.getElementById('child').contentWindow],rows=[];
        for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)
            rows.push(...await probe(realms[owner],realms[callee],owner+'-'+callee));
        globalThis.__offscreenTransfers=rows;
    }})().catch(error=>globalThis.__offscreenTransfers=String(error));"#,
        include_str!("offscreen_canvas_clone.js")
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while vm.eval("typeof __offscreenTransfers").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(
            Instant::now() < deadline,
            "OffscreenCanvas transfer contract timed out"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        vm.eval("Array.isArray(__offscreenTransfers)").unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__offscreenTransfers)").unwrap()
    );
    assert_eq!(vm.eval("__offscreenTransfers.length").unwrap(), "208");
    assert_eq!(
        vm.eval("JSON.stringify(__offscreenTransfers.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn offscreen_canvas_transfer_respects_registered_native_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://offscreen-transfer.test/");
    vm.eval("globalThis.canvas=new OffscreenCanvas(3,2)")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "canvas");
        let canvas =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, canvas, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
        if(nativeProxy.width!==3||nativeProxy.height!==2)return false;
        nativeProxy.width=4;nativeProxy.height=5;
        return canvas.width===4&&canvas.height===5;
    })()"#
        )
        .unwrap(),
        "true"
    );
    assert_eq!(vm.eval(r#"(() => {
        for(const list of [[canvas,nativeProxy],[nativeProxy,canvas]]) {
            let reads=0,error;
            try{structuredClone({get value(){reads++;}},{transfer:list});}catch(caught){error=caught;}
            if(!(error instanceof DOMException)||error.name!=='DataCloneError'||reads!==0)return false;
        }
        const copy=structuredClone({canvas,alias:canvas},{transfer:[nativeProxy]});
        return copy.canvas===copy.alias&&copy.canvas instanceof OffscreenCanvas&&copy.canvas.width===4&&copy.canvas.height===5&&canvas.width===0&&nativeProxy.width===0;
    })()"#).unwrap(), "true");
    assert_eq!(vm.eval(r#"(() => {
        const get=OffscreenCanvas.prototype.getContext;
        try{get.call(nativeProxy,'2d');}catch(error){return error instanceof DOMException&&error.name==='InvalidStateError';}
        return false;
    })()"#).unwrap(), "true");
}
