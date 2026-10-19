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
        #[webapi(slot = crate::context_bootstrap::SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
        target_slot: (),
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

#[test]
fn encrypted_media_element_handlers_use_native_bindings_and_ordered_dispatch() {
    for url in [
        "http://eme-handlers.test/",
        "https://eme-handlers.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_test_vm(url);
        vm.eval(include_str!("encrypted_media_handlers.js"))
            .unwrap();
        assert_eq!(
            vm.eval("__uiEventResults.complete").unwrap(),
            "true",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "168", "{url}");
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
    }
}

#[test]
fn encrypted_media_element_content_handlers_compile_from_parser_attributes() {
    let mut vm = new_parsed_test_vm(
        "https://eme-handler-parser.test/",
        r#"<!doctype html><body>
          <audio id="audio"
            onencrypted="__handlerTrace.push('audio:' + event.type)"
            onwaitingforkey="__handlerTrace.push('audio:' + event.type)"></audio>
          <video id="video"
            onencrypted="__handlerTrace.push('video:' + event.type)"
            onwaitingforkey="__handlerTrace.push('video:' + event.type)"></video>
          <button id="empty" onclick=""></button>
        "#,
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
              globalThis.__handlerTrace = [];
              for (const id of ['audio', 'video']) {
                const media = document.getElementById(id);
                for (const type of ['encrypted', 'waitingforkey']) {
                  const name = 'on' + type;
                  const handler = media[name];
                  if (typeof handler !== 'function' || handler !== media[name]) {
                    throw Error('parser handler identity: ' + id + '/' + name);
                  }
                  media.dispatchEvent(new Event(type));
                }
              }
              const empty = document.getElementById('empty');
              const handler = empty.onclick;
              if (typeof handler !== 'function' || empty.onclick !== handler ||
                  !empty.dispatchEvent(new Event('click', {cancelable:true}))) {
                throw Error('empty global event-handler body');
              }
              return __handlerTrace.join();
            })()"#,
        )
        .unwrap(),
        "audio:encrypted,audio:waitingforkey,video:encrypted,video:waitingforkey"
    );
}

#[test]
fn encrypted_media_element_handlers_accept_registered_native_proxies() {
    let mut vm = new_parsed_test_vm("https://eme-handler-proxy.test/", "<!doctype html><body>");
    vm.eval(
        r#"document.body.innerHTML = '<iframe></iframe>';
        globalThis.other = document.querySelector('iframe').contentWindow;
        globalThis.audioTarget = document.createElement('audio');
        globalThis.videoTarget = document.implementation.createHTMLDocument('').createElement('video');
        globalThis.nativeProxyReads = 0;
        globalThis.nativeProxyGet = function() { nativeProxyReads++; throw Error('native get trap'); };"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("audioTarget", "audioProxy"), ("videoTarget", "videoProxy")] {
            let value = global
                .get(scope, crate::util::v8str(scope, name).into())
                .unwrap();
            let target = v8::Local::<v8::Object>::try_from(value).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let get_trap = global
                .get(scope, crate::util::v8str(scope, "nativeProxyGet").into())
                .unwrap();
            assert_eq!(
                handler.create_data_property(
                    scope,
                    crate::util::v8str(scope, "get").into(),
                    get_trap
                ),
                Some(true)
            );
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert!(crate::web_api_interfaces::HTMLMediaElement::is_instance(
                scope,
                proxy.into()
            ));
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
    assert_eq!(
        vm.eval(
            r#"(() => {
              for (const realm of [window, other]) {
                for (const [media, nativeProxy] of [[audioTarget, audioProxy], [videoTarget, videoProxy]]) {
                  for (const name of ['onencrypted', 'onwaitingforkey', 'onclick']) {
                    const prototype = name === 'onclick' ? realm.HTMLElement.prototype : realm.HTMLMediaElement.prototype;
                    const {get, set} = Object.getOwnPropertyDescriptor(prototype, name);
                    let calls = 0;
                    const callback = function(event) {
                      if (this !== media || event.currentTarget !== media) throw Error('native event target');
                      calls++;
                    };
                    set.call(nativeProxy, callback);
                    if (get.call(nativeProxy) !== callback || media[name] !== callback) throw Error('native proxy handler identity');
                    media.dispatchEvent(new Event(name.slice(2)));
                    if (calls !== 1) throw Error('native proxy dispatch');
                    let traps = 0;
                    const author = new Proxy(nativeProxy, {
                      get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
                    });
                    const revoked = Proxy.revocable(nativeProxy, {}); revoked.revoke();
                    for (const receiver of [author, revoked.proxy, Object.create(nativeProxy)]) {
                      for (const operation of [get, set]) {
                        let error;
                        try { operation.call(receiver, callback); } catch (caught) { error = caught; }
                        if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype || traps !== 0) {
                          throw Error('invalid receiver accepted');
                        }
                      }
                    }
                    set.call(nativeProxy, null);
                    if (get.call(nativeProxy) !== null || media[name] !== null) throw Error('native proxy clear');
                    if (media.ownerDocument.defaultView) {
                      media.setAttribute(name, 'this.setAttribute("data-proxy-fired", event.type)');
                      const content = get.call(nativeProxy);
                      if (typeof content !== 'function' || content !== media[name]) throw Error('native proxy content handler');
                      media.dispatchEvent(new Event(name.slice(2)));
                      if (media.getAttribute('data-proxy-fired') !== name.slice(2)) throw Error('native proxy content dispatch');
                      media.removeAttribute(name);
                    }
                  }
                }
              }
              if (nativeProxyReads !== 0) throw Error('native identity must bypass get traps');
              return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
