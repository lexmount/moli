use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_init_conversion_attributes_and_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://channels.test/",
        "http://channels.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/rtc-data-channel-init.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "channel initialization fixture must finish through the Page dispatcher",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===52 && new Set(__uiEventResults.checks.map(row=>row.name)).size===52").unwrap(), "true");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_init_registered_native_proxies_share_pc_and_channel_slots() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://channels.test/proxy", &loader);
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.channel=pc.createDataChannel('native',{negotiated:true,id:31,maxRetransmits:8});").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [("pc", "nativePC"), ("channel", "nativeChannel")] {
            let key = crate::util::v8_string(scope, name).unwrap();
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
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
    assert_eq!(vm.eval(r#"(()=>{
      if(nativeChannel.label!=='native'||nativeChannel.id!==31||nativeChannel.maxRetransmits!==8)throw Error('proxy slots');
      nativeChannel.binaryType='blob';nativeChannel.bufferedAmountLowThreshold=39;
      if(channel.binaryType!=='blob'||channel.bufferedAmountLowThreshold!==39)throw Error('proxy mutation');
      let error;try{nativePC.createDataChannel('duplicate',{negotiated:true,id:31})}catch(e){error=e}
      if(error?.name!=='OperationError')throw Error('proxy PC identity');
      const second=nativePC.createDataChannel('second',{negotiated:true,id:32});
      if(second.id!==32||second.label!=='second')throw Error('proxy PC creation');
      let conversions=0;try{Reflect.getOwnPropertyDescriptor(RTCDataChannel.prototype,'binaryType').set.call(new Proxy(nativeChannel,{}),{toString(){conversions++;return 'blob'}})}catch(e){error=e}
      if(!(error instanceof TypeError)||conversions!==0)throw Error('author proxy conversion');
      nativeChannel.close();if(channel.readyState!=='closed')throw Error('proxy close');
      pc.close();return true;
    })()"#).unwrap(), "true");
}
