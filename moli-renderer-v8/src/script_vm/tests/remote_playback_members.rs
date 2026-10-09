use super::*;

#[tokio::test]
async fn remote_playback_members_preserve_receivers_realms_and_unavailable_callback_order() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://remote-members.test/", &loader);
    vm.eval(include_str!("remote_playback_members.js")).unwrap();
    for realm in ["main", "iframe", "popup"] {
        run_next_page_media_element_event_for_test(&mut vm, &loader, realm).await;
    }
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.watches.every(row => row.resolvedType === 'undefined' && row.available === false && row.count === 1)").unwrap(), "true");
}

#[test]
fn remote_playback_registered_native_proxies_share_media_and_handler_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://remote-native-proxy.test/");
    vm.eval("globalThis.media = document.createElement('video'); globalThis.remote = media.remote")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (original, alias) in [("media", "mediaProxy"), ("remote", "remoteProxy")] {
            let key = crate::util::v8str(scope, original);
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, alias);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name)};
      assert(mediaProxy.remote === remote && remoteProxy.state === 'disconnected', 'native identity');
      mediaProxy.disableRemotePlayback = true;
      assert(media.hasAttribute('disableremoteplayback') && mediaProxy.disableRemotePlayback, 'native proxy boolean reflection');
      const log = [];
      remote.addEventListener('connect', () => log.push('before'));
      remoteProxy.onconnect = () => log.push('original');
      remote.addEventListener('connect', () => log.push('after'));
      remoteProxy.onconnect = () => {log.push('replacement'); return false};
      assert(remote.onconnect === remoteProxy.onconnect, 'handler native identity');
      assert(!remote.dispatchEvent(new Event('connect', {cancelable:true})), 'EventHandler cancellation');
      assert(log.join() === 'before,replacement,after' && remote.state === 'disconnected', 'ordered handler without a connection');
      Object.setPrototypeOf(remote, null);
      assert(Object.getOwnPropertyDescriptor(RemotePlayback.prototype, 'state').get.call(remoteProxy) === 'disconnected', 'brand survives prototype replacement');
      let error; try {Object.getOwnPropertyDescriptor(RemotePlayback.prototype, 'state').get.call(new Proxy(remoteProxy, {}))} catch (caught) {error=caught}
      assert(error instanceof TypeError, 'author wrapping native Proxy is rejected');
      return true;
    })()"#).unwrap(), "true");
}

#[tokio::test]
async fn remote_playback_callback_exceptions_do_not_reject_watches_or_stop_media_tasks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://remote-errors.test/", &loader);
    vm.eval(r#"
      globalThis.order = []; globalThis.errors = 0;
      window.onerror = () => {errors++; return true};
      const remote = document.createElement('video').remote;
      remote.watchAvailability(() => {order.push('throwing'); throw Error('availability callback')}).then(value => order.push(String(value)));
      remote.watchAvailability(available => order.push(String(available))).then(() => order.push('resolved'));
    "#).unwrap();
    assert_eq!(vm.eval("order.join()").unwrap(), "undefined,resolved");
    for _ in 0..2 {
        run_next_page_media_element_event_for_test(&mut vm, &loader, "availability notification")
            .await;
    }
    assert_eq!(
        vm.eval("order.join()").unwrap(),
        "undefined,resolved,throwing,false"
    );
    assert_eq!(vm.eval("errors").unwrap(), "1");
}

#[tokio::test]
async fn remote_playback_retired_iframe_discards_callbacks_and_rejects_prompt() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://remote-retirement.test/",
        &loader,
    );
    vm.eval(r#"
      globalThis.called = 0; globalThis.promptResult = '';
      const frame = document.createElement('iframe'); document.body.appendChild(frame);
      const other = frame.contentWindow;
      const media = other.document.createElement('video'); other.document.body.appendChild(media);
      const remote = media.remote;
      remote.watchAvailability(() => {called++});
      frame.remove();
      remote.prompt().catch(error => {promptResult = error.name + ':' + (error instanceof other.DOMException)});
    "#).unwrap();
    vm.run_one_media_element_event_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(vm.eval("called").unwrap(), "0");
    assert_eq!(vm.eval("promptResult").unwrap(), "InvalidAccessError:true");
}
