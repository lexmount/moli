use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtp_sender_parameters_tracks_streams_and_operations_use_native_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://rtp-sender.test/",
        "http://rtp-sender.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!("../../../tests/fixtures/rtp-sender.js"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "sender promises must finish through the production selected Page dispatcher",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===164 && new Set(__uiEventResults.checks.map(row=>row.name)).size===164").unwrap(), "true");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rtp_sender_registered_native_proxies_keep_identity_before_argument_conversion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://rtp-proxy.test/", &loader);
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.sender=pc.addTransceiver('audio').sender;globalThis.track=pc.addTransceiver('audio').receiver.track;globalThis.stream=new MediaStream()").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (original, native) in [
            ("pc", "nativePC"),
            ("sender", "nativeSender"),
            ("track", "nativeTrack"),
            ("stream", "nativeStream"),
        ] {
            let key = crate::util::v8_string(scope, original).unwrap();
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8_string(scope, native).unwrap();
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    vm.eval(r#"globalThis.nativeProxyDone=false;(async()=>{
      const p=nativeSender.getParameters();p.encodings[0].active=false;await nativeSender.setParameters(p);
      if(sender.getParameters().encodings[0].active)throw Error('native sender update');
      await nativeSender.replaceTrack(nativeTrack);if(sender.track!==nativeTrack)throw Error('native track identity');
      nativeSender.setStreams(nativeStream,nativeStream);
      const offer=await nativePC.createOffer();if(offer.sdp.split('a=msid:'+stream.id+' ').length!==2)throw Error('native stream ID');
      let reads=0;try{await RTCRtpSender.prototype.setParameters.call(new Proxy(nativeSender,{}),{get codecs(){reads++;return [];}});throw Error('author proxy accepted');}catch(e){if(!(e instanceof TypeError)||reads!==0)throw e;}
      nativePC.close();if(pc.signalingState!=='closed')throw Error('native PC state');nativeProxyDone=true;
    })().catch(e=>globalThis.nativeProxyError=String(e));"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(nativeProxyDone||!!globalThis.nativeProxyError)",
        "true",
        "native proxy operations must complete through networking tasks",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.nativeProxyError??'')").unwrap(),
        ""
    );
    assert_eq!(vm.eval("nativeProxyDone").unwrap(), "true");
}
