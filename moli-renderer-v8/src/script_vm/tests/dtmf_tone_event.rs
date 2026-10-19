use super::*;

#[test]
fn dtmf_tone_event_preserves_dom_strings_and_dictionary_conversion_order() {
    let mut vm = new_storage_test_vm("https://dtmf-event.test/");
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => { if (!ok) throw Error(name) };
      const C = RTCDTMFToneChangeEvent;
      assert(C.name === 'RTCDTMFToneChangeEvent' && C.length === 1, 'constructor metadata');
      for (const init of [undefined, null, {}]) {
        const event = new C('tonechange', init);
        assert(event instanceof C && event instanceof Event, 'inheritance');
        assert(event.tone === '' && event.type === 'tonechange', 'defaults');
        assert(!event.bubbles && !event.cancelable && !event.composed && !event.isTrusted, 'event defaults');
      }
      for (const [tone, expected] of [[null, 'null'], [123, '123'], ['\uD800\0\uDC00', '\uD800\0\uDC00']]) {
        const event = new C('\uDC00\0', {tone, bubbles:true, cancelable:true, composed:true});
        assert(event.tone === expected && event.type === '\uDC00\0', 'UTF-16 DOMString');
        assert(event.bubbles && event.cancelable && event.composed, 'inherited flags');
      }
      const order = [];
      const init = {};
      for (const name of ['bubbles', 'cancelable', 'composed', 'tone']) {
        Object.defineProperty(init, name, {get() { order.push(name); return name === 'tone' ? {toString() {order.push('tone conversion'); return 'A'}} : true }});
      }
      const event = new C({toString() {order.push('type'); return 'tonechange'}}, init);
      assert(event.tone === 'A' && order.join() === 'type,bubbles,cancelable,composed,tone,tone conversion', 'conversion order');
      const sentinel = {};
      let error;
      try {new C('x', {get tone() {throw sentinel}})} catch (caught) {error = caught}
      assert(error === sentinel, 'original getter exception');
      for (const action of [() => C('x'), () => new C(), () => new C(Symbol()), () => new C('x', 1), () => new C('x', {tone:Symbol()})]) {
        error = undefined; try {action()} catch (caught) {error = caught}
        assert(error instanceof TypeError, 'invalid arguments');
      }
      const descriptor = Object.getOwnPropertyDescriptor(C.prototype, 'tone');
      assert(descriptor.enumerable && descriptor.configurable && !descriptor.set, 'readonly descriptor');
      class Derived extends C {}
      assert(new Derived('x', {tone:'B'}).tone === 'B', 'subclass');
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn dtmf_tone_event_native_identity_survives_cross_realm_getters_and_native_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://dtmf-event-realms.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'; globalThis.nativeEvent = new RTCDTMFToneChangeEvent('tonechange', {tone:'C'})").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeEvent");
        let event =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, event, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativeProxy").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => { if (!ok) throw Error(name) };
      const other = document.querySelector('iframe').contentWindow;
      const get = Object.getOwnPropertyDescriptor(other.RTCDTMFToneChangeEvent.prototype, 'tone').get;
      assert(get.call(nativeEvent) === 'C' && get.call(nativeProxy) === 'C', 'native identity across realms');
      const foreign = new other.RTCDTMFToneChangeEvent('tonechange', {tone:'D'});
      assert(foreign instanceof other.Event && !(foreign instanceof Event) && foreign.tone === 'D', 'constructor realm');
      const revoked = Proxy.revocable(nativeEvent, {}); revoked.revoke();
      let traps = 0;
      const author = new Proxy(nativeEvent, {get() {traps++; throw Error('get')}, getPrototypeOf() {traps++; throw Error('prototype')}});
      for (const bad of [{}, Object.create(nativeEvent), author, revoked.proxy, new Proxy(nativeProxy, {})]) {
        let error; try {get.call(bad)} catch (caught) {error = caught}
        assert(error instanceof other.TypeError && !(error instanceof TypeError), 'callee realm receiver rejection');
      }
      assert(traps === 0, 'receiver checks do not invoke author traps');
      Object.defineProperty(nativeEvent, 'tone', {value:'expando'});
      Object.setPrototypeOf(nativeEvent, null);
      assert(get.call(nativeEvent) === 'C' && get.call(nativeProxy) === 'C', 'private payload survives author mutation');
      return true;
    })()"#).unwrap(), "true");
}
