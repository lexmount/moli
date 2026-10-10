use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtp_transceivers_native_graph_conversion_preferences_and_lifecycle() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://rtp.test/",
        "http://rtp.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!("../../../tests/fixtures/rtp-transceivers.js"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "RTP fixture must settle through the production selected Page task dispatcher",
        )
        .await;
        assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===100 && new Set(__uiEventResults.checks.map(row=>row.name)).size===100").unwrap(), "true");
    }
}

#[test]
fn rtp_transceivers_accept_registered_native_proxies_and_reject_author_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtp-native-proxy.test/");
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.transceiver=pc.addTransceiver('audio');globalThis.sender=transceiver.sender;globalThis.receiver=transceiver.receiver;globalThis.track=receiver.track").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (original, native) in [
            ("pc", "nativePC"),
            ("transceiver", "nativeTransceiver"),
            ("sender", "nativeSender"),
            ("receiver", "nativeReceiver"),
            ("track", "nativeTrack"),
        ] {
            let key = crate::util::v8_string(scope, original).unwrap();
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let name = crate::util::v8_string(scope, native).unwrap();
            assert_eq!(
                global.create_data_property(scope, name.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      if(nativePC.getTransceivers()[0]!==transceiver||nativeSender.track!==null||nativeReceiver.track!==track)return false;
      if(nativePC.onnegotiationneeded!==null)return false;
      const handler=()=>{};nativePC.onnegotiationneeded=handler;if(pc.onnegotiationneeded!==handler)return false;
      nativePC.onnegotiationneeded=42;if(pc.onnegotiationneeded!==null)return false;
      const descriptor=Object.getOwnPropertyDescriptor(RTCPeerConnection.prototype,'onnegotiationneeded');
      try{descriptor.set.call(new Proxy(nativePC,{}),handler);return false;}catch(error){if(!(error instanceof TypeError))return false;}
      nativeTransceiver.direction='sendonly';nativeTransceiver.setCodecPreferences(RTCRtpSender.getCapabilities('audio').codecs);
      if(transceiver.direction!=='sendonly'||nativePC.addTransceiver(nativeTrack).sender.track!==nativeTrack)return false;
      let reads=0;try{RTCRtpTransceiver.prototype.setCodecPreferences.call(new Proxy(nativeTransceiver,{}),{[Symbol.iterator](){reads++;return [][Symbol.iterator]();}});return false;}catch(error){if(!(error instanceof TypeError)||reads!==0)return false;}
      nativeTransceiver.stop();if(transceiver.direction!=='stopped')return false;pc.close();return true;
    })()"#).unwrap(), "true");
}
