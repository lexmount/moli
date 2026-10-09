use super::*;

#[test]
fn picture_in_picture_interfaces_validate_descriptors_receivers_and_required_dictionary_members() {
    let mut vm = new_storage_page_task_executor_test_vm("https://picture-in-picture.test/");
    assert_eq!(
        vm.eval(include_str!("picture_in_picture_interfaces.js"))
            .unwrap(),
        "true"
    );
}

fn install_closed_window(vm: &mut ScriptVm) {
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let window = crate::context_bootstrap::closed_picture_in_picture_window_for_test(scope);
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, window, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        for (name, value) in [
            ("nativeWindow", window.into()),
            ("nativeProxy", proxy.into()),
        ] {
            assert_eq!(
                global.create_data_property(scope, crate::util::v8str(scope, name).into(), value),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn picture_in_picture_events_retain_native_window_identity_across_realms_and_author_mutations() {
    let mut vm = new_storage_page_task_executor_test_vm("https://picture-in-picture-events.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    install_closed_window(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name)};
      const other = document.querySelector('iframe').contentWindow;
      const payload = Object.getOwnPropertyDescriptor(other.PictureInPictureEvent.prototype, 'pictureInPictureWindow').get;
      const width = Object.getOwnPropertyDescriptor(other.PictureInPictureWindow.prototype, 'width').get;
      assert(nativeWindow.width === 0 && nativeWindow.height === 0 && nativeProxy.width === 0, 'closed dimensions');
      for (const window of [nativeWindow, nativeProxy]) {
        const event = new other.PictureInPictureEvent('enterpictureinpicture', {pictureInPictureWindow: window, bubbles:true, cancelable:true, composed:true});
        assert(event instanceof other.PictureInPictureEvent && event instanceof other.Event && !(event instanceof PictureInPictureEvent), 'constructor realm');
        assert(event.pictureInPictureWindow === window && event.pictureInPictureWindow === event.pictureInPictureWindow, 'SameObject preserves original argument');
        assert(event.type === 'enterpictureinpicture' && event.bubbles && event.cancelable && event.composed && !event.isTrusted, 'inherited EventInit');
        Object.defineProperty(event, 'pictureInPictureWindow', {value: {}, configurable:true});
        assert(payload.call(event) === window, 'author payload expando does not alter native state');
        const proxy = Proxy.revocable(window, {}); proxy.revoke();
        let traps = 0;
        const author = new Proxy(window, {get() {traps++; throw Error('get')}, getPrototypeOf() {traps++; throw Error('prototype')}});
        for (const bad of [Object.create(window), author, proxy.proxy]) {
          let error; try {new other.PictureInPictureEvent('x', {pictureInPictureWindow:bad})} catch (caught) {error=caught}
          assert(Object.getPrototypeOf(error) === other.TypeError.prototype, 'author wrapper rejected in callee realm');
        }
        assert(traps === 0, 'native brand does not invoke author traps');
      }
      class Derived extends other.PictureInPictureEvent {}
      const derived = new Derived('x', {pictureInPictureWindow:nativeWindow});
      assert(derived instanceof Derived && payload.call(derived) === nativeWindow, 'subclass native brand');
      Object.defineProperty(nativeWindow, 'width', {get() {throw Error('public dimension read')}});
      Object.setPrototypeOf(nativeWindow, null);
      assert(width.call(nativeWindow) === 0 && payload.call(derived) === nativeWindow, 'private identity survives prototype and expando changes');
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn picture_in_picture_resize_handlers_share_listener_order_and_cancellation_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://picture-in-picture-resize.test/");
    install_closed_window(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name)};
      const target = nativeWindow, order = [];
      const descriptor = Object.getOwnPropertyDescriptor(PictureInPictureWindow.prototype, 'onresize');
      assert(target.onresize === null, 'initial handler');
      target.addEventListener('resize', event => {order.push('before'); assert(event.target === target, 'native target')});
      target.onresize = function(event) {order.push('original'); assert(this === target && event.currentTarget === target, 'callback this and currentTarget')};
      target.addEventListener('resize', () => order.push('after'));
      nativeProxy.onresize = function(event) {order.push('replacement'); assert(this === target, 'native proxy updates same handler'); return false};
      const event = new Event('resize', {cancelable:true});
      assert(target.dispatchEvent(event) === false && event.defaultPrevented, 'handler cancellation');
      assert(order.join() === 'before,replacement,after' && target.onresize === nativeProxy.onresize, 'replacement preserves listener position');
      for (const value of [undefined, null, false, 3, 'x', 1n, Symbol('handler')]) {
        descriptor.set.call(target, value);
        assert(target.onresize === null, 'primitive handler becomes null');
      }
      order.length = 0;
      descriptor.set.call(nativeProxy, () => order.push('last'));
      assert(target.dispatchEvent(new Event('resize')) && order.join() === 'before,after,last', 'cleared handler is appended when set again');
      assert(event.currentTarget === null && target.width === 0 && target.height === 0, 'dispatch does not open a window');
      return true;
    })()"#).unwrap(), "true");
}
