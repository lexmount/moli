use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_dtmf_sender_identity_conversion_events_and_lifecycle() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://dtmf.test/",
        "http://dtmf.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!("../../../tests/fixtures/rtc-dtmf-sender.js"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "DTMF sender fixture must finish through the production selected Page dispatcher",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===64 && new Set(__uiEventResults.checks.map(row=>row.name)).size===64").unwrap(), "true");
    }
}

#[test]
fn rtc_dtmf_sender_registered_native_proxies_keep_identity_and_event_state() {
    let mut vm = new_parsed_test_vm(
        "https://dtmf.test/proxy",
        "<!doctype html><body><iframe></iframe>",
    );
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.sender=pc.addTransceiver('audio').sender;globalThis.dtmf=sender.dtmf").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [("sender", "nativeSender"), ("dtmf", "nativeDTMF")] {
            let key = crate::util::v8_string(scope, name).unwrap();
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8_string(scope, alias).unwrap();
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const other=document.querySelector('iframe').contentWindow;
      const assert=ok=>{if(!ok)throw Error('native DTMF proxy')};
      assert(nativeSender.dtmf===dtmf && nativeDTMF.toneBuffer==='' && nativeDTMF.canInsertDTMF===false);
      let count=0; const callback=()=>count++;
      Object.getOwnPropertyDescriptor(other.RTCDTMFSender.prototype,'ontonechange').set.call(nativeDTMF,callback);
      assert(dtmf.ontonechange===callback);
      EventTarget.prototype.dispatchEvent.call(nativeDTMF,new RTCDTMFToneChangeEvent('tonechange',{tone:'1'}));
      assert(count===1);
      let conversions=0,error;
      try{other.RTCDTMFSender.prototype.insertDTMF.call(nativeDTMF,{toString(){conversions++;return '1'}})}catch(e){error=e}
      assert(error instanceof other.DOMException && error.name==='InvalidStateError' && conversions===1);
      conversions=0;try{other.RTCDTMFSender.prototype.insertDTMF.call(new Proxy(nativeDTMF,{}),{toString(){conversions++;return '1'}})}catch(e){error=e}
      assert(error instanceof other.TypeError && conversions===0);
      pc.close(); assert(nativeSender.dtmf===dtmf && nativeDTMF.toneBuffer==='');return true;
    })()"#).unwrap(), "true");
}
