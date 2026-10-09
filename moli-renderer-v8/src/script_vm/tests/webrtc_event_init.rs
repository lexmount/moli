use super::*;

#[test]
fn webrtc_event_constructors_use_shared_dictionary_brands_and_deferred_prototypes() {
    for url in ["https://rtc-events.test/", "http://rtc-events.test/"] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval(include_str!("../../../tests/fixtures/webrtc-event-init.js"))
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]"
        );
        assert_eq!(vm.eval("__uiEventResults.total >= 140").unwrap(), "true");
    }
}

#[test]
fn native_rtc_track_events_preserve_identity_frozen_snapshots_realms_and_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-rtc-events.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    media_streams::install_inert_media_tracks(&mut vm);
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (interface, name, proxy_name) in [
            ("RTCRtpReceiver", "rtpReceiver", "nativeReceiver"),
            ("RTCRtpTransceiver", "rtpTransceiver", "nativeTransceiver"),
        ] {
            let object = v8::Object::new(scope);
            let prototype =
                crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, interface)?;
            assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
            crate::web_api_interfaces::initialize(scope, object, interface)?;
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
            for (key, value) in [(name, object.into()), (proxy_name, proxy.into())] {
                assert_eq!(
                    global.create_data_property(
                        scope,
                        crate::util::v8str(scope, key).into(),
                        value
                    ),
                    Some(true)
                );
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(include_str!(
            "../../../tests/fixtures/webrtc-event-native.js"
        ))
        .unwrap(),
        "true"
    );
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("trackEvent", "nativeTrackEvent"),
            ("toneEvent", "nativeToneEvent"),
        ] {
            let value = global
                .get(scope, crate::util::v8str(scope, name).into())
                .unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, proxy_name).into(),
                    proxy.into()
                ),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const other=document.querySelector('iframe').contentWindow;
      for(const w of [window,other]) {
        const tone=Object.getOwnPropertyDescriptor(w.RTCDTMFToneChangeEvent.prototype,'tone').get;
        if(tone.call(nativeToneEvent)!=='\ud800')throw Error('registered native event Proxy');
        for(const property of ['receiver','track','streams','transceiver']) {
          const get=Object.getOwnPropertyDescriptor(w.RTCTrackEvent.prototype,property).get;
          if(get.call(nativeTrackEvent)!==trackEvent[property])throw Error('registered native track event Proxy');
          let error;try{get.call(new Proxy(nativeTrackEvent,{}))}catch(caught){error=caught}
          if(!(error instanceof w.TypeError))throw Error('author Proxy around native event');
        }
      }
      return true;
    })()"#).unwrap(), "true");
}
