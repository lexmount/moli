use super::*;

#[test]
fn push_interfaces_share_native_prototypes_and_receiver_policy() {
    let mut vm = new_storage_page_task_executor_test_vm("https://push-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "({}).then(value => globalThis.__pushInterfaceDone = value, error => globalThis.__pushInterfaceDone = String(error));",
        include_str!("push_interfaces.js")
    )).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__pushInterfaceDone").unwrap(), "true");
}

#[test]
fn push_interface_globals_require_secure_contexts() {
    let mut vm = new_storage_page_task_executor_test_vm("http://push-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        "({}).then(value => globalThis.__pushInterfaceDone = value);",
        include_str!("push_interfaces.js")
    ))
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__pushInterfaceDone").unwrap(), "true");
}

#[test]
fn push_subscription_factories_keep_native_state_and_intrinsic_prototypes() {
    let mut vm = new_storage_page_task_executor_test_vm("https://push-interfaces.test/");
    vm.eval(
        r#"
      globalThis.PushTypes = [PushManager, PushSubscription, PushSubscriptionOptions];
      globalThis.constructorReads = 0;
      for (const name of ['PushManager', 'PushSubscription', 'PushSubscriptionOptions']) {
        Object.defineProperty(globalThis, name, { configurable: true, get() {
          constructorReads++; throw Error('author constructor');
        } });
      }
    "#,
    )
    .unwrap();
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let object = crate::context_bootstrap::push_interfaces::build_subscription(
                scope,
                &crate::service_worker_runtime::ServiceWorkerPushSubscriptionSnapshot {
                    endpoint: "https://push-interfaces.test/local-subscription".to_owned(),
                    user_visible_only: true,
                },
            )
            .unwrap();
            let global = context.global(scope);
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, "nativeSubscription").into(),
                    object.into()
                ),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const [Manager, Subscription, Options] = PushTypes, s = nativeSubscription;
      const assert = (value, message) => { if (!value) throw Error(message); };
      const throws = (run, expected) => { let error; try { run(); } catch (e) { error = e; }
        assert(expected ? error === expected : error instanceof TypeError, 'expected exception'); };
      assert(constructorReads === 0, 'intrinsic factories');
      assert(Object.getPrototypeOf(s) === Subscription.prototype, 'subscription prototype');
      assert(Object.getPrototypeOf(s.options) === Options.prototype && s.options === s.options, 'options identity and prototype');
      assert(!Object.hasOwn(s, 'endpoint') && !Object.hasOwn(s, 'getKey') && !Object.hasOwn(s.options, 'userVisibleOnly'), 'shared members');
      assert(s.options.userVisibleOnly && s.options.applicationServerKey === null && s.expirationTime === null, 'native state');
      assert(s.getKey('auth') === null && s.getKey('p256dh') === null, 'unavailable key material');
      for (const args of [[], ['other'], [Symbol()]]) throws(() => s.getKey(...args));
      const sentinel = {};
      throws(() => s.getKey({ toString() { throw sentinel; } }), sentinel);
      let conversions = 0;
      const revoked = Proxy.revocable(s, {}); revoked.revoke();
      for (const receiver of [Object.create(s), new Proxy(s, {}), revoked.proxy]) {
        throws(() => Subscription.prototype.getKey.call(receiver, { toString() { conversions++; return 'auth'; } }));
      }
      assert(conversions === 0, 'brand before conversion');
      const endpoint = s.endpoint;
      let getterCalls = 0, setterCalls = 0;
      for (const name of ['endpoint', 'expirationTime', 'options']) {
        Object.defineProperty(s, name, { configurable: true, get() { getterCalls++; throw Error('author getter'); } });
      }
      for (const name of ['endpoint', 'expirationTime', 'keys']) {
        Object.defineProperty(Object.prototype, name, { configurable: true, set() { setterCalls++; } });
      }
      let json;
      try { json = Subscription.prototype.toJSON.call(s); }
      finally { for (const name of ['endpoint', 'expirationTime', 'keys']) delete Object.prototype[name]; }
      assert(getterCalls === 0 && setterCalls === 0, 'serialization uses internal state and own data properties');
      assert(Object.keys(json).join() === 'endpoint,expirationTime,keys' && json.endpoint === endpoint && json.expirationTime === null, 'JSON fields');
      assert(Object.getPrototypeOf(json.keys) === Object.prototype && Object.keys(json.keys).length === 0, 'empty native key record');
      const d = Object.getOwnPropertyDescriptor(json, 'endpoint');
      assert(d.writable && d.enumerable && d.configurable, 'JSON dictionary descriptor');
      const saved = s.getKey; Object.setPrototypeOf(s, null);
      assert(saved.call(s, 'auth') === null && Subscription.prototype.toJSON.call(s).endpoint === endpoint, 'brand independent of author prototype');
      return 'ok';
    })()"#).unwrap(), "ok");
}
