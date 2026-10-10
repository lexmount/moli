use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_local_transports_association_snapshots_brands_realms_rollback_and_close() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://local-transports.test/",
        "http://local-transports.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/rtc-local-transports.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "local transport fixture must finish through the selected Page dispatcher",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===54 && new Set(__uiEventResults.checks.map(row=>row.name)).size===54").unwrap(), "true");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_local_transports_registered_native_proxies_keep_native_identity_without_traps() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://local-transports.test/proxy",
        &loader,
    );
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.t=pc.addTransceiver('audio');globalThis.ready=false;pc.setLocalDescription().then(()=>{globalThis.dtls=t.sender.transport;globalThis.ice=dtls.iceTransport;globalThis.ready=true;});globalThis.traps=0;globalThis.trap=()=>{traps++;throw Error('native Proxy trap')};").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(ready)",
        "true",
        "associate the native transports",
    )
    .await;
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let trap_key = crate::util::v8_string(scope, "trap").unwrap();
        let trap = global.get(scope, trap_key.into()).unwrap();
        for (name, alias) in [("dtls", "nativeDtls"), ("ice", "nativeIce")] {
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
      for(const [name,proxy,real] of [['RTCDtlsTransport',nativeDtls,dtls],['RTCIceTransport',nativeIce,ice]]){
        for(const key of Object.getOwnPropertyNames(globalThis[name].prototype)){
          if(key==='constructor')continue;const d=Object.getOwnPropertyDescriptor(globalThis[name].prototype,key);
          if(d.get){const value=d.get.call(proxy);if(value!==d.get.call(real))throw Error('native getter');if(d.set){const fn=()=>{};d.set.call(proxy,fn);if(d.get.call(real)!==fn)throw Error('native setter');d.set.call(real,null);}}
          else if(typeof d.value==='function'){const result=d.value.call(proxy);if(result!==null&&typeof result!=='object')throw Error('native method');}
        }
        EventTarget.prototype.addEventListener.call(proxy,'custom',()=>{});EventTarget.prototype.dispatchEvent.call(proxy,new Event('custom'));
        let error;try{Object.getOwnPropertyDescriptor(globalThis[name].prototype,'state').get.call(new Proxy(proxy,{}))}catch(e){error=e}if(!(error instanceof TypeError))throw Error('author Proxy');
      }
      pc.close();return dtls.state==='closed'&&ice.state==='closed'&&traps===0;
    })()"#).unwrap(),"true");
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_local_transports_connection_close_suppresses_queued_rollback_statechange() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://local-transports.test/queued",
        &loader,
    );
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.t=pc.addTransceiver('audio');globalThis.ready=false;pc.setLocalDescription().then(()=>{globalThis.dtls=t.sender.transport;globalThis.ice=dtls.iceTransport;globalThis.ready=true;});globalThis.trace=[];").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(ready)",
        "true",
        "associate the local transports",
    )
    .await;
    vm.eval("dtls.onstatechange=()=>trace.push('unexpected');ice.onstatechange=()=>trace.push('unexpected');pc.setLocalDescription({type:'rollback'}).then(()=>{trace.push('rolled back');pc.close();if(dtls.state!=='closed'||ice.state!=='closed')throw Error('synchronous transport shutdown');pc.getStats().then(()=>trace.push('stats'));});").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "trace.join('|')",
        "rolled back|stats",
        "closing the owner must suppress its queued DTLS event",
    )
    .await;
    assert_eq!(
        vm.eval("dtls.state==='closed' && ice.state==='closed' && t.sender.transport===null")
            .unwrap(),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_local_transports_failed_offer_planning_rejects_and_advances_operations() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://local-transports.test/failure",
        &loader,
    );
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.t=pc.addTransceiver('audio');globalThis.reasons=[];").unwrap();
    let set_next_mid = |vm: &mut ScriptVm, value: u32| {
        let context_ptr = &vm.page_default_runtime.context as *const _;
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8_string(scope, "pc").unwrap();
            let pc =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            crate::util::set_private_value(
                scope,
                pc,
                "__moliRtcNextMid",
                v8::Integer::new_from_unsigned(scope, value).into(),
            );
            Ok(())
        })
        .unwrap();
    };
    // Exhaust the native allocator without producing billions of sections.
    // Both explicit offer creation and implicit local-description planning fail.
    set_next_mid(&mut vm, u32::MAX);
    vm.eval("for(const implicit of [false,true,false]){const p=implicit?pc.setLocalDescription():pc.createOffer();p.then(()=>reasons.push('unexpected success'),error=>reasons.push(error));}").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(reasons.length)",
        "3",
        "every failed operation must reject and release the next operation",
    )
    .await;
    assert_eq!(vm.eval("reasons.every(error=>error instanceof DOMException&&error.name==='OperationError'&&error.code===0)&&pc.signalingState==='stable'&&pc.localDescription===null&&t.mid===null&&t.sender.transport===null&&t.receiver.transport===null").unwrap(), "true");
    set_next_mid(&mut vm, 0);
    vm.eval("globalThis.recovered=false;pc.setLocalDescription().then(()=>{recovered=t.mid==='0'&&t.sender.transport!==null&&t.sender.transport===t.receiver.transport;pc.close();});").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(recovered)",
        "true",
        "the same connection must recover after allocation is available",
    )
    .await;
    assert_eq!(
        vm.eval(
            "t.sender.transport.state==='closed'&&t.sender.transport.iceTransport.state==='closed'"
        )
        .unwrap(),
        "true"
    );
}
