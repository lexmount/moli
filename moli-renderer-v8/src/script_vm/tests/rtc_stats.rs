use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_stats_native_reports_maplike_conversion_and_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://stats.test/",
        "http://stats.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!("../../../tests/fixtures/rtc-stats.js"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "statistics fixture must finish through the selected Page dispatcher",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total===71 && new Set(__uiEventResults.checks.map(row=>row.name)).size===71").unwrap(), "true");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_stats_registered_native_proxies_preserve_track_and_report_identity() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://stats.test/proxy", &loader);
    vm.eval("globalThis.pc=new RTCPeerConnection();globalThis.t=pc.addTransceiver('audio');globalThis.sender=t.sender;globalThis.receiver=t.receiver;globalThis.track=receiver.track").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [
            ("pc", "nativePC"),
            ("sender", "nativeSender"),
            ("receiver", "nativeReceiver"),
            ("track", "nativeTrack"),
        ] {
            let key = crate::util::v8_string(scope, name).unwrap();
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
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
    vm.eval(r#"globalThis.done=false;globalThis.failure=null;
      (async()=>{
        const report=await nativePC.getStats();globalThis.report=report;
        if(!(report instanceof RTCStatsReport))throw Error('PC proxy report');
        for(const value of [await nativePC.getStats(nativeTrack),await nativeSender.getStats(),await nativeReceiver.getStats()])
          if(value.size!==0)throw Error('RTP proxy selection');
      })().then(()=>done=true,e=>{failure=String(e);done=true});'queued'"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "native stats proxy promises",
    )
    .await;
    assert_eq!(vm.eval("failure===null").unwrap(), "true");
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8_string(scope, "report").unwrap();
        let report =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, report, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8_string(scope, "nativeReport").unwrap();
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(()=>{
      const stats=[...report.values()][0],id=stats.id;
      if(nativeReport.size!==1||nativeReport.get(id)!==stats||!nativeReport.has(id))throw Error('native report proxy');
      let count=0;nativeReport.forEach((v,k,owner)=>{if(owner!==nativeReport||v!==stats||k!==id)throw Error('proxy owner');count++});
      if(count!==1||[...nativeReport][0][1]!==stats)throw Error('proxy iteration');
      let conversions=0,error;
      try{RTCStatsReport.prototype.get.call(new Proxy(nativeReport,{}),{toString(){conversions++;return id}})}catch(e){error=e}
      if(!(error instanceof TypeError)||conversions!==0)throw Error('author proxy brand');
      pc.close();return true;
    })()"#).unwrap(), "true");
}
