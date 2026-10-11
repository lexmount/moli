use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_sctp_transports_associate_native_snapshots_brands_realms_rollback_and_close() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://sctp.test/",
        "http://sctp.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/rtc-sctp-transports.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "SCTP fixture must finish through the selected Page dispatcher",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===35 && __uiEventResults.passed===35 && new Set(__uiEventResults.checks.map(row=>row.name)).size===35").unwrap(), "true");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_sctp_transports_registered_native_proxies_keep_identity_without_author_traps() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://sctp.test/proxy", &loader);
    vm.eval("globalThis.pc=new RTCPeerConnection();pc.createDataChannel('data');globalThis.ready=false;pc.setLocalDescription().then(()=>{globalThis.sctp=pc.sctp;globalThis.ready=true;});globalThis.traps=0;globalThis.trap=()=>{traps++;throw Error('native Proxy trap')};").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(ready)",
        "true",
        "associate SCTP",
    )
    .await;
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "trap").unwrap();
        let trap = global.get(scope, key.into()).unwrap();
        for (name, alias) in [("pc", "nativePc"), ("sctp", "nativeSctp")] {
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
      const pcGetter=Object.getOwnPropertyDescriptor(RTCPeerConnection.prototype,'sctp').get;
      if(pcGetter.call(nativePc)!==sctp)throw Error('native PC association');
      for(const key of ['transport','state','maxMessageSize','maxChannels','onstatechange']){
        const d=Object.getOwnPropertyDescriptor(RTCSctpTransport.prototype,key);
        if(d.get.call(nativeSctp)!==d.get.call(sctp))throw Error('native getter '+key);
        if(d.set){const fn=()=>{};d.set.call(nativeSctp,fn);if(d.get.call(sctp)!==fn)throw Error('native handler');d.set.call(sctp,null);}
        let error;try{d.get.call(new Proxy(nativeSctp,{}))}catch(e){error=e}
        if(!(error instanceof TypeError))throw Error('author Proxy '+key);
      }
      let count=0;EventTarget.prototype.addEventListener.call(nativeSctp,'statechange',()=>count++);
      RTCPeerConnection.prototype.close.call(nativePc);
      return count===1&&sctp.state==='closed'&&traps===0;
    })()"#).unwrap(), "true");
}
