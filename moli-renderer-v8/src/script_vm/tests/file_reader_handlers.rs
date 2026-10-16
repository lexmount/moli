use super::*;

fn new_native_proxy_file_reader_test_vm(
    url: &str,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut vm = new_storage_page_task_executor_test_vm(url);
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
    vm
}

#[test]
fn file_reader_prototype_handlers_share_native_proxy_state_and_listener_order() {
    let mut vm = new_native_proxy_file_reader_test_vm("https://file-reader-handlers.test/");
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

#[test]
fn file_reader_native_proxy_abort_dispatches_on_backing_reader() {
    let mut vm = new_native_proxy_file_reader_test_vm("https://file-reader-proxy-abort.test/");
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name);};
      const calls = [];
      const record = label => function(event) {
        calls.push([label, this === reader, event.target === reader,
          event.currentTarget === reader, reader.readyState === FileReader.DONE,
          nativeReaderProxy.result === null, nativeReaderProxy.error === null].join(':'));
      };
      reader.addEventListener('abort', record('abort'));
      nativeReaderProxy.onabort = record('abort-handler');
      nativeReaderProxy.addEventListener('loadend', record('loadend'));
      reader.onloadend = record('loadend-handler');
      const expected = ['abort', 'abort-handler', 'loadend', 'loadend-handler']
        .map(label => label + ':true:true:true:true:true:true').join();
      for (const abort of [() => nativeReaderProxy.abort(),
          () => FileReader.prototype.abort.call(nativeReaderProxy),
          () => foreign.FileReader.prototype.abort.call(nativeReaderProxy)]) {
        nativeReaderProxy.readAsText(new Blob(['test']));
        assert(reader.readyState === FileReader.LOADING, 'proxy starts reading');
        calls.length = 0;
        abort();
        assert(calls.join() === expected, 'proxy abort dispatch: ' + calls.join());
        assert(reader.readyState === FileReader.DONE && nativeReaderProxy.readyState === FileReader.DONE,
          'shared aborted state');
        abort();
        assert(calls.join() === expected, 'second abort does not dispatch');
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn file_reader_native_proxy_inherited_event_target_methods_share_listener_registry() {
    let mut vm = new_native_proxy_file_reader_test_vm("https://file-reader-proxy-dispatch.test/");
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name);};
      const calls = [];
      const listener = function(event) {
        calls.push([this === reader, event.target === reader, event.currentTarget === reader].join(':'));
        event.preventDefault();
      };
      for (const callee of [window, foreign]) {
        const prototype = callee.EventTarget.prototype;
        for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
          assert(!Object.hasOwn(callee.FileReader.prototype, name) &&
            callee.FileReader.prototype[name] === prototype[name], name + '/inherited');
        }
        prototype.addEventListener.call(nativeReaderProxy, 'custom', listener);
        reader.addEventListener('custom', listener);
        calls.length = 0;
        const event = new callee.Event('custom', {cancelable: true});
        assert(prototype.dispatchEvent.call(nativeReaderProxy, event) === false, 'canceled proxy dispatch');
        assert(calls.join() === 'true:true:true', 'shared listener identity and duplicate suppression');
        assert(event.target === reader && event.currentTarget === null && event.defaultPrevented,
          'event target and dispatch cleanup');
        prototype.removeEventListener.call(nativeReaderProxy, 'custom', listener);
        calls.length = 0;
        assert(reader.dispatchEvent(new Event('custom')) === true && calls.length === 0,
          'inherited proxy removal updates reader');
        reader.addEventListener('custom', listener);
        assert(nativeReaderProxy.dispatchEvent(new Event('custom', {cancelable: true})) === false,
          'inherited dispatchEvent through proxy');
        assert(calls.join() === 'true:true:true', 'proxy dispatch reaches reader listener');
        nativeReaderProxy.removeEventListener('custom', listener);
        let traps = 0;
        const author = new Proxy(nativeReaderProxy, {
          get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
        });
        const revoked = Proxy.revocable(reader, {}); revoked.revoke();
        for (const receiver of [{}, Object.create(reader), author, revoked.proxy]) {
          for (const [method, args] of [
              ['addEventListener', ['custom', listener]],
              ['removeEventListener', ['custom', listener]],
              ['dispatchEvent', [new Event('custom')]]]) {
            let error; try {prototype[method].call(receiver, ...args);} catch (caught) {error = caught;}
            assert(error && Object.getPrototypeOf(error) === callee.TypeError.prototype && traps === 0,
              method + '/receiver');
          }
        }
      }
      return true;
    })()"#).unwrap(), "true");
}
