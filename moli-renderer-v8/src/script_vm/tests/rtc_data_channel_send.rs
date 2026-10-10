use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_send_overloads_conversion_order_and_callee_errors() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://channel-send.test/",
        "http://channel-send.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        for (fixture, count) in [
            (
                include_str!("../../../tests/fixtures/rtc-data-channel-send.js"),
                62,
            ),
            (
                include_str!("../../../tests/fixtures/rtc-data-channel-send-shared.js"),
                12,
            ),
        ] {
            vm.eval(fixture).unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(globalThis.__uiEventResults?.complete)",
                "true",
                "send conversion fixture must complete",
            )
            .await;
            assert_eq!(
                vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                    .unwrap(),
                "[]",
                "{url}"
            );
            assert_eq!(vm.eval(&format!("__uiEventResults.total==={count} && new Set(__uiEventResults.checks.map(row=>row.name)).size==={count}")).unwrap(), "true");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_send_registered_native_proxy_preserves_channel_and_blob_identity() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://channel-send.test/proxy",
        &loader,
    );
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.channel=pc.createDataChannel('native');globalThis.blob=new Blob(['native']);globalThis.nativeTraps=0;globalThis.nativeTrap=()=>{nativeTraps++;throw Error('native Proxy trap')};").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "nativeTrap").unwrap();
        let trap = global.get(scope, key.into()).unwrap();
        for (name, alias) in [("channel", "nativeChannel"), ("blob", "nativeBlob")] {
            // Registration binds the private handler identity to one Proxy.
            let handler = crate::util::new_null_prototype_object(scope);
            let key = crate::util::v8_string(scope, "get").unwrap();
            assert_eq!(
                handler.create_data_property(scope, key.into(), trap),
                Some(true)
            );
            let key = crate::util::v8_string(scope, name).unwrap();
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
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
      const send=RTCDataChannel.prototype.send;
      for(const receiver of [channel,nativeChannel])for(const value of ['message',blob,nativeBlob,new Uint8Array(8)]) {
        let error;try{send.call(receiver,value)}catch(e){error=e}
        if(error?.name!=='InvalidStateError'||!(error instanceof DOMException))throw Error(`native proxy state: ${error?.name}: ${error?.message}`);
      }
      if(nativeTraps!==0||channel.bufferedAmount!==0)throw Error('native proxy trap or buffering');
      let conversions=0,error;try{send.call(new Proxy(nativeChannel,{}),{toString(){conversions++;return 'x'}})}catch(e){error=e}
      if(!(error instanceof TypeError)||conversions!==0)throw Error('author receiver proxy');
      const data=new Proxy(nativeBlob,{get(target,key){if(key!==Symbol.toPrimitive)throw Error('wrong lookup');return()=>{conversions++;return 'proxy text'}}});
      try{send.call(channel,data)}catch(e){error=e}
      if(error?.name!=='InvalidStateError'||conversions!==1||nativeTraps!==0)throw Error('author data proxy string overload');
      pc.close();return true;
    })()"#).unwrap(), "true");
}
