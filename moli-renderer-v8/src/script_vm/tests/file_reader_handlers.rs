use super::*;

#[test]
fn file_reader_prototype_handlers_share_native_proxy_state_and_listener_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://file-reader-handlers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'; globalThis.foreign = document.querySelector('iframe').contentWindow; globalThis.reader = new foreign.FileReader();").unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "reader");
        let reader = global.get(scope, key.into()).unwrap();
        let reader = v8::Local::<v8::Object>::try_from(reader).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, reader, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeReaderProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name);};
      const names = ['onloadstart','onprogress','onload','onabort','onerror','onloadend'];
      for (const callee of [window, foreign]) {
        for (const name of names) {
          const d = Object.getOwnPropertyDescriptor(callee.FileReader.prototype, name);
          assert(d.enumerable && d.configurable && d.get.length === 0 && d.set.length === 1, name + '/descriptor');
          assert(!Object.hasOwn(reader, name) && d.get.call(nativeReaderProxy) === null, name + '/prototype');
          const handler = () => {};
          d.set.call(nativeReaderProxy, handler);
          assert(reader[name] === handler && d.get.call(nativeReaderProxy) === handler, name + '/native-state');
          let objectHandlerCalls = 0;
          const nonCallable = {handleEvent() {objectHandlerCalls++;}};
          d.set.call(reader, nonCallable);
          assert(d.get.call(nativeReaderProxy) === nonCallable, name + '/object-handler');
          reader.dispatchEvent(new Event(name.slice(2)));
          assert(objectHandlerCalls === 0, name + '/object-is-not-event-listener');
          d.set.call(reader, 42);
          assert(d.get.call(nativeReaderProxy) === null, name + '/primitive-handler');
          let traps = 0;
          const author = new Proxy(reader, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}});
          const revoked = Proxy.revocable(reader, {}); revoked.revoke();
          for (const receiver of [{}, Object.create(reader), author, revoked.proxy]) {
            for (const callback of [() => d.get.call(receiver), () => d.set.call(receiver, handler)]) {
              let error; try {callback();} catch (caught) {error = caught;}
              assert(error && Object.getPrototypeOf(error) === callee.TypeError.prototype && traps === 0, name + '/receiver');
            }
          }
        }
      }
      assert(nativeReaderProxy.readyState === 0 && nativeReaderProxy.result === null && nativeReaderProxy.error === null, 'native getter state');
      const calls = [], listener = () => calls.push('listener');
      FileReader.prototype.addEventListener.call(nativeReaderProxy, 'load', listener);
      nativeReaderProxy.onload = () => calls.push('handler');
      reader.addEventListener('load', () => calls.push('tail'));
      reader.dispatchEvent(new Event('load'));
      assert(calls.join() === 'listener,handler,tail', 'ordered handler insertion');
      nativeReaderProxy.onload = () => calls.push('replacement');
      FileReader.prototype.removeEventListener.call(nativeReaderProxy, 'load', listener);
      calls.length = 0; reader.dispatchEvent(new Event('load'));
      assert(calls.join() === 'replacement,tail', 'replacement retains order and native removal');
      nativeReaderProxy.onload = null;
      nativeReaderProxy.onload = () => calls.push('readded');
      calls.length = 0; reader.dispatchEvent(new Event('load'));
      assert(calls.join() === 'tail,readded', 'readding follows later listeners');
      return true;
    })()"#).unwrap(), "true");
}
