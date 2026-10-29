use super::*;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn message_port_transfer_checks_live_state_after_serialization_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://port-transfer.test/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        r#"(async()=>{{
        const probe={},realms=[globalThis,document.getElementById('child').contentWindow],rows=[];
        for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)
            rows.push(...await probe(realms[owner],realms[callee],owner+'-'+callee));
        globalThis.__portTransfers=rows;
    }})().catch(error=>globalThis.__portTransfers=String(error));"#,
        include_str!("message_port_transfer.js")
    ))
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while vm.eval("typeof __portTransfers").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(
            Instant::now() < deadline,
            "MessagePort transfer contract timed out"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        vm.eval("Array.isArray(__portTransfers)").unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__portTransfers)").unwrap()
    );
    assert_eq!(vm.eval("__portTransfers.length").unwrap(), "160");
    assert_eq!(
        vm.eval("JSON.stringify(__portTransfers.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn message_port_transfer_respects_registered_native_proxy_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://port-transfer.test/");
    vm.eval("globalThis.channel=new MessageChannel();globalThis.original=channel.port1")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "original");
        let target =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval("channel.port2.onmessage=event=>globalThis.__nativePortMessage=event.data;nativeProxy.postMessage('native proxy');").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("__nativePortMessage")
            .unwrap(),
        "native proxy"
    );
    assert_eq!(vm.eval(r#"(() => {
        for(const list of [[original,nativeProxy],[nativeProxy,original]]) {
            let reads=0,error;
            try{structuredClone({get value(){reads++;}},{transfer:list});}catch(caught){error=caught;}
            if(!(error instanceof DOMException)||error.name!=='DataCloneError'||reads!==0)return false;
        }
        const copy=structuredClone({port:original},{transfer:[nativeProxy]});
        nativeProxy.close();
        let error;try{structuredClone(1,{transfer:[original]});}catch(caught){error=caught;}
        copy.port.close();channel.port2.close();
        return copy.port instanceof MessagePort&&error instanceof DOMException&&error.name==='DataCloneError';
    })()"#).unwrap(),"true");
}
