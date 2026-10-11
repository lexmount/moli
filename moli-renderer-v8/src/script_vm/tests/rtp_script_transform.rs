use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtp_script_transform_clones_options_and_owns_worker_streams_across_window_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://rtp-transform.test/",
        "http://rtp-transform.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/rtp-script-transform.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm, &loader, "String(globalThis.__uiEventResults?.complete)", "true",
            "RTP transform creation, messages and cancellation must complete on their owning Worker and Page queues",
        ).await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===344 && new Set(__uiEventResults.checks.map(row=>row.name)).size===344").unwrap(), "true", "{url}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rtp_script_transform_native_proxies_keep_worker_and_endpoint_identity() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://rtp-transform-proxy.test/",
        &loader,
    );
    vm.eval("globalThis.worker=new Worker('data:text/javascript,onrtctransform=()=>{}');globalThis.pc=new RTCPeerConnection();globalThis.sender=pc.addTransceiver('audio').sender;globalThis.transform=new RTCRtpScriptTransform(worker)").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (original, native) in [
            ("worker", "nativeWorker"),
            ("sender", "nativeSender"),
            ("transform", "nativeTransform"),
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
    assert_eq!(vm.eval(r#"(() => {
      const other=new RTCRtpScriptTransform(nativeWorker);
      nativeSender.transform=nativeTransform;
      if(sender.transform!==nativeTransform)return false;
      nativeSender.transform=nativeTransform;
      nativeSender.transform=other;
      if(sender.transform!==other)return false;
      let traps=0;
      try {nativeSender.transform=new Proxy(nativeTransform,{get(){traps++;}});return false;} catch(e){if(!(e instanceof TypeError)||traps!==0)return false;}
      nativeSender.transform=null;worker.terminate();pc.close();return sender.transform===null;
    })()"#).unwrap(), "true");
}
