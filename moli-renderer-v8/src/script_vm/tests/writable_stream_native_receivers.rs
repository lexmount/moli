use super::*;

#[test]
fn writable_stream_native_proxy_state_and_error_realms_follow_generated_bindings() {
    let mut vm = new_storage_page_task_executor_test_vm("https://writable-receivers.test/");
    vm.eval(
        r#"
document.body.innerHTML = '<iframe></iframe>';
globalThis.stream = new WritableStream();
globalThis.closing = new WritableStream();
globalThis.aborting = new WritableStream();
"#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, alias) in [
            ("stream", "streamProxy"),
            ("closing", "closingProxy"),
            ("aborting", "abortingProxy"),
        ] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let alias = crate::util::v8str(scope, alias);
            assert_eq!(
                global.create_data_property(scope, alias.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    vm.eval(r#"
      globalThis.receiverResult = 'pending';
      (async () => {
        const assert = (ok, name) => {if (!ok) throw Error(name)};
        const other = document.querySelector('iframe').contentWindow;
        let traps = 0;
        const author = new Proxy(streamProxy, {get(){traps++;throw 42}, getPrototypeOf(){traps++;throw 42}});
        const revoked = Proxy.revocable(streamProxy, {}); revoked.revoke();
        for (const realm of [window, other]) {
          const p = realm.WritableStream.prototype;
          const locked = Object.getOwnPropertyDescriptor(p, 'locked').get;
          const writer = p.getWriter.call(streamProxy);
          assert(locked.call(stream) && locked.call(streamProxy), 'native Proxy shares writer lock');
          writer.releaseLock();
          assert(!locked.call(streamProxy), 'released lock reaches native target');
          const nativeWriter = new realm.WritableStreamDefaultWriter(streamProxy);
          assert(locked.call(stream) && locked.call(streamProxy), 'constructor locks native target');
          nativeWriter.releaseLock();
          assert(!locked.call(stream), 'constructed writer releases native target');
          for (const receiver of [{}, Object.create(stream), author, revoked.proxy]) {
            let constructorError;
            try { new realm.WritableStreamDefaultWriter(receiver); } catch (error) { constructorError = error; }
            assert(constructorError && Object.getPrototypeOf(constructorError) === realm.TypeError.prototype, 'constructor interface conversion uses callee TypeError');
            for (const fn of [p.getWriter, locked]) {
              let error;try {fn.call(receiver)} catch(caught) {error=caught}
              assert(error && Object.getPrototypeOf(error)===realm.TypeError.prototype, 'synchronous callee TypeError');
            }
            for (const fn of [p.abort, p.close]) {
              const promise=fn.call(receiver);
              assert(Object.getPrototypeOf(promise)===realm.Promise.prototype, 'callee rejection Promise');
              let error;try {await promise} catch(caught) {error=caught}
              assert(error && Object.getPrototypeOf(error)===realm.TypeError.prototype, 'asynchronous callee TypeError');
            }
          }
        }
        await other.WritableStream.prototype.close.call(closingProxy);
        await other.WritableStream.prototype.abort.call(abortingProxy, {reason:true});
        assert(!closing.locked && !aborting.locked && traps===0, 'native operations and no author traps');
        receiverResult='ok';
      })().catch(error => {receiverResult=String(error)});
    "#).unwrap();
    assert_eq!(vm.eval("receiverResult").unwrap(), "ok");
}
