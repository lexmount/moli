use super::*;

#[tokio::test(flavor = "current_thread")]
async fn webrtc_signaling_state_events_and_descriptions_follow_native_operations() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://signaling.test/",
        "http://signaling.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/webrtc-signaling-state.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm, &loader, "String(globalThis.__uiEventResults?.complete)", "true",
            "signaling events and reentrant operations must finish through the production selected Page dispatcher",
        ).await;
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
async fn webrtc_signaling_state_registered_native_proxy_uses_canonical_slots() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://signaling.test/proxy", &loader);
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.trace=[]")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "pc").unwrap();
        let object =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, object, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8_string(scope, "nativePC").unwrap();
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval(r#"nativePC.onsignalingstatechange=()=>trace.push(pc.signalingState);
      if(pc.onsignalingstatechange!==nativePC.onsignalingstatechange||nativePC.remoteDescription!==null)throw Error('native proxy slots');
      globalThis.proxyDone=false;(async()=>{
        await nativePC.setLocalDescription();
        if(nativePC.localDescription!==pc.localDescription)throw Error('native proxy description');
        await nativePC.setLocalDescription({type:'rollback'});proxyDone=true;
      })().catch(e=>globalThis.proxyError=String(e));"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(proxyDone||!!globalThis.proxyError)",
        "true",
        "native proxy signaling operations must complete",
    )
    .await;
    assert_eq!(vm.eval("String(globalThis.proxyError??'')").unwrap(), "");
    assert_eq!(
        vm.eval("trace.join(',')").unwrap(),
        "have-local-offer,stable"
    );
}

#[test]
fn webrtc_signaling_descriptions_prefer_pending_and_fall_back_to_current() {
    let mut vm = new_parsed_test_vm("https://signaling.test/current", "<!doctype html><body>");
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.cl=new RTCSessionDescription({type:'answer',sdp:'current local'});globalThis.pl=new RTCSessionDescription({type:'offer',sdp:'pending local'});globalThis.cr=new RTCSessionDescription({type:'answer',sdp:'current remote'});globalThis.pr=new RTCSessionDescription({type:'offer',sdp:'pending remote'})").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "pc").unwrap();
        let pc = v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        // Simulate committed and pending backend snapshots to cover getter
        // invariants before remote SDP negotiation can populate these slots.
        for (slot, value) in [
            ("__moliRtcPeerConnectionCurrentLocalDescription", "cl"),
            ("__moliRtcPeerConnectionPendingLocalDescription", "pl"),
            ("__moliRtcPeerConnectionCurrentRemoteDescription", "cr"),
            ("__moliRtcPeerConnectionPendingRemoteDescription", "pr"),
        ] {
            let key = crate::util::v8_string(scope, value).unwrap();
            let value = global.get(scope, key.into()).unwrap();
            crate::util::set_private_value(scope, pc, slot, value);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("pc.localDescription===pl&&pc.currentLocalDescription===cl&&pc.pendingLocalDescription===pl&&pc.remoteDescription===pr&&pc.currentRemoteDescription===cr&&pc.pendingRemoteDescription===pr").unwrap(), "true");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "pc").unwrap();
        let pc = v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        for slot in [
            "__moliRtcPeerConnectionPendingLocalDescription",
            "__moliRtcPeerConnectionPendingRemoteDescription",
        ] {
            crate::util::set_private_value(scope, pc, slot, v8::null(scope).into());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("pc.localDescription===cl&&pc.remoteDescription===cr&&pc.pendingLocalDescription===null&&pc.pendingRemoteDescription===null").unwrap(), "true");
    assert_eq!(vm.eval("(()=>{try{pc.setConfiguration({iceCandidatePoolSize:1})}catch(e){return e.name==='InvalidModificationError'}return false})()").unwrap(), "true");
}
