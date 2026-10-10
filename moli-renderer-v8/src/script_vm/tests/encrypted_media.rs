use super::*;

#[tokio::test(flavor = "current_thread")]
async fn encrypted_media_native_interfaces_convert_requests_and_validate_receivers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "http://localhost/",
        "https://localhost/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
            .unwrap();
        vm.eval(include_str!("../../../tests/fixtures/encrypted-media.js"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "EME fixture must finish through native Page tasks",
        )
        .await;
        assert_eq!(
            vm.eval("!!globalThis.__uiEventResults?.complete").unwrap(),
            "true",
            "{url}"
        );
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn encrypted_media_requests_use_exact_receiver_owners_and_callee_error_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://eme-owner.test/", &loader);
    vm.eval("document.body.innerHTML='<iframe></iframe>'; globalThis.other=document.querySelector('iframe').contentWindow").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let navigator = v8::Local::<v8::Object>::try_from(
            global
                .get(scope, crate::util::v8str(scope, "navigator").into())
                .unwrap(),
        )
        .unwrap();
        let handler = v8::Object::new(scope);
        handler
            .set_prototype(scope, v8::null(scope).into())
            .unwrap();
        let proxy = v8::Proxy::new(scope, navigator, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::Navigator::is_instance(
            scope,
            proxy.into()
        ));
        global.create_data_property(
            scope,
            crate::util::v8str(scope, "nativeNavigator").into(),
            proxy.into(),
        );
        Ok(())
    })
    .unwrap();
    vm.eval(r#"globalThis.trace=[]; globalThis.done=false;
      other.Navigator.prototype.requestMediaKeySystemAccess.call(nativeNavigator,'com.example.unsupported',[{}]).catch(error=>{
        if(!(error instanceof other.DOMException)||error instanceof DOMException||error.name!=='NotSupportedError')throw Error('error realm');
        trace.push('rejected');done=true;
      }); Promise.resolve().then(()=>trace.push('microtask'));
    "#).unwrap();
    assert_eq!(vm.eval("JSON.stringify(trace)").unwrap(), "[\"microtask\"]");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "EME backend rejection needs an owner task",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(trace)").unwrap(),
        "[\"microtask\",\"rejected\"]"
    );

    vm.eval(r#"globalThis.retired=false;
      Navigator.prototype.requestMediaKeySystemAccess.call(other.navigator,'com.example.unsupported',[{}]).then(()=>retired=true,()=>retired=true);
      document.querySelector('iframe').remove();
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("String(retired)").unwrap(),
        "false",
        "a removed receiver owner must not dispatch in the borrowed callee"
    );
}

#[test]
fn encrypted_media_session_handlers_use_native_event_targets_and_registered_proxies() {
    use moli_webapi_declare::WebApiObject;

    #[derive(WebApiObject)]
    #[webapi(interface = crate::web_api_interfaces::MediaKeySession, require_prototype)]
    struct TestSession {
        #[webapi(slot = crate::context_bootstrap::SIMPLE_EVENT_TARGET_SLOT, value = "__moliMediaKeySessionListeners")]
        target_slot: (),
        #[webapi(slot = crate::context_bootstrap::SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
        ordered_handlers: (),
    }

    let mut vm = new_storage_page_task_executor_test_vm("https://eme-events.test/");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let session = TestSession::new().bind(scope).unwrap();
        let handler = v8::Object::new(scope);
        handler
            .set_prototype(scope, v8::null(scope).into())
            .unwrap();
        let proxy = v8::Proxy::new(scope, session, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let global = scope.get_current_context().global(scope);
        global.create_data_property(
            scope,
            crate::util::v8str(scope, "session").into(),
            proxy.into(),
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,message)=>{if(!ok)throw Error(message)};
      assert(session instanceof MediaKeySession&&session instanceof EventTarget,'native interface chain');
      const log=[];
      assert(session.onmessage===null&&session.onkeystatuseschange===null,'initial handlers');
      session.addEventListener('message',()=>log.push('before'));
      session.onmessage=function(event){assert(this===session,'native receiver');assert(event.type==='message','event');log.push('handler')};
      session.addEventListener('message',()=>log.push('after'));
      session.dispatchEvent(new Event('message'));
      assert(log.join()==='before,handler,after','ordered native handler');
      session.onmessage=()=>log.push('replacement');log.length=0;session.dispatchEvent(new Event('message'));
      assert(log.join()==='before,replacement,after','replacement retains listener position');
      session.onmessage=null;log.length=0;session.dispatchEvent(new Event('message'));
      assert(log.join()==='before,after'&&session.onmessage===null,'handler removal');
      session.onkeystatuseschange=()=>log.push('status');session.dispatchEvent(new Event('keystatuseschange'));
      assert(log[log.length-1]==='status','independent status handler');
      const get=Object.getOwnPropertyDescriptor(MediaKeySession.prototype,'onmessage').get;
      try {get.call(new Proxy(session,{}));throw Error('accepted author Proxy')} catch(error){assert(error instanceof TypeError,'author Proxy brand')}
      return true;
    })()"#).unwrap(), "true");
}
