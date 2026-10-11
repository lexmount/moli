use super::*;

#[tokio::test(flavor = "current_thread")]
async fn webtransport_frontend_preserves_conversion_streams_failure_and_worker_ownership() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for (url, expected) in [
        ("https://webtransport.test/", 42),
        ("http://webtransport.test/", 1),
        ("data:text/html,<body>", 1),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval(include_str!(
            "../../../tests/fixtures/webtransport-frontend.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "WebTransport fixture must finish through the selected Page dispatcher",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval(&format!("__uiEventResults.total==={expected} && __uiEventResults.passed==={expected} && new Set(__uiEventResults.checks.map(row=>row.name)).size==={expected}")).unwrap(), "true", "{url}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn webtransport_frontend_registered_native_proxies_preserve_brands_and_stream_ancestry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://webtransport.test/proxy",
        &loader,
    );
    vm.eval("globalThis.wt=new WebTransport('https://example.test:0/');globalThis.duplex=wt.datagrams;globalThis.group=wt.createSendGroup();globalThis.writable=duplex.createWritable();globalThis.traps=0;globalThis.trap=()=>{traps++;throw Error('native Proxy trap')};").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "trap").unwrap();
        let trap = global.get(scope, key.into()).unwrap();
        for (name, alias) in [
            ("wt", "nativeWt"),
            ("duplex", "nativeDuplex"),
            ("group", "nativeGroup"),
            ("writable", "nativeWritable"),
        ] {
            let handler = crate::util::new_null_prototype_object(scope);
            for property in ["get", "getPrototypeOf"] {
                let key = crate::util::v8_string(scope, property).unwrap();
                assert_eq!(
                    handler.create_data_property(scope, key.into(), trap),
                    Some(true)
                );
            }
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
    assert_eq!(vm.eval(r#"(() => {
      const descriptor=(C,key)=>Object.getOwnPropertyDescriptor(C.prototype,key);
      if(descriptor(WebTransport,'datagrams').get.call(nativeWt)!==duplex)throw Error('native transport');
      descriptor(WebTransport,'anticipatedConcurrentIncomingBidirectionalStreams').set.call(nativeWt,7);
      if(wt.anticipatedConcurrentIncomingBidirectionalStreams!==7)throw Error('native transport setter');
      if(descriptor(WebTransportDatagramDuplexStream,'readable').get.call(nativeDuplex)!==duplex.readable)throw Error('native duplex');
      descriptor(WebTransportDatagramsWritable,'sendGroup').set.call(nativeWritable,nativeGroup);
      if(descriptor(WebTransportDatagramsWritable,'sendGroup').get.call(nativeWritable)!==nativeGroup)throw Error('native group identity');
      if(!(WebTransportSendGroup.prototype.getStats.call(nativeGroup) instanceof Promise))throw Error('native group stats');
      if(descriptor(WritableStream,'locked').get.call(nativeWritable)!==false)throw Error('base stream brand');
      const writer=WritableStream.prototype.getWriter.call(nativeWritable);
      if(descriptor(WritableStream,'locked').get.call(writable)!==true)throw Error('base stream state');
      writer.closed.catch(()=>{});
      writer.releaseLock();
      const directWriter=new WritableStreamDefaultWriter(nativeWritable);
      directWriter.closed.catch(()=>{});
      if(descriptor(WritableStream,'locked').get.call(writable)!==true)throw Error('constructor stream identity');
      let conversions=0;
      for(const [C,key,receiver] of [[WebTransport,'anticipatedConcurrentIncomingBidirectionalStreams',nativeWt],[WebTransportDatagramDuplexStream,'outgoingMaxAge',nativeDuplex],[WebTransportDatagramsWritable,'sendOrder',nativeWritable]]){
        let error;try{descriptor(C,key).set.call(new Proxy(receiver,{}),{valueOf(){conversions++;return 1}})}catch(e){error=e}
        if(!(error instanceof TypeError))throw Error('author Proxy '+key);
      }
      WebTransport.prototype.close.call(nativeWt);
      return traps===0&&conversions===0;
    })()"#).unwrap(), "true");
}
