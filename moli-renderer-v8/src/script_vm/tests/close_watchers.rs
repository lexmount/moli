use super::*;

#[tokio::test(flavor = "current_thread")]
async fn close_watcher_respects_its_document_and_event_realm_after_frame_removal() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://close-watcher-frame.test/", &loader);
    for phase in ["before", "cancel", "close"] {
        vm.eval(
            r#"
            globalThis.frame = document.createElement('iframe');
            (document.body || document.documentElement || document).appendChild(frame);
            void frame.contentWindow;
        "#,
        )
        .expect("expose child Window");
        assert!(
            vm.run_one_child_frame_task_executor_turn(
                ChildFrameSemanticTurnKind::RealmMaterialization,
                &loader,
            )
            .await
            .expect("materialize child realm")
        );
        vm.eval(&format!("globalThis.phase = {phase:?};"))
            .expect("set removal phase");
        let result = vm.eval(r#"
        (() => {
          const check = (condition, label) => { if (!condition) throw new Error(label); };
          const child = frame.contentWindow;
          const ChildWatcher = child.CloseWatcher;
          const ChildException = child.DOMException;
          const ChildEvent = child.Event;
          const controller = new AbortController();
          const watcher = new ChildWatcher({signal: controller.signal});
          const events = [];
          watcher.oncancel = event => {
            events.push('cancel:' + (Object.getPrototypeOf(event) === ChildEvent.prototype));
            if (phase === 'cancel') frame.remove();
          };
          watcher.onclose = event => {
            events.push('close:' + (Object.getPrototypeOf(event) === ChildEvent.prototype));
            if (phase === 'close') frame.remove();
          };
          if (phase === 'before') frame.remove();
          // Borrowing a parent method must still use the watcher's document and realm.
          CloseWatcher.prototype.requestClose.call(watcher);
          for (const method of ['requestClose', 'close', 'destroy']) watcher[method]();
          controller.abort();
          check(events.join() === ({before: '', cancel: 'cancel:true', close: 'cancel:true,close:true'})[phase],
                phase + ': ' + events);
          let caught;
          try { new ChildWatcher(); } catch (error) { caught = error; }
          check(caught instanceof ChildException && caught.name === 'InvalidStateError',
                'retained constructor must throw in the child realm');
          const sentinel = {};
          caught = undefined;
          try { new ChildWatcher({get signal() { throw sentinel; }}); } catch (error) { caught = error; }
          check(caught === sentinel, 'dictionary conversion precedes inactive document check');
          return 'ok';
        })()
        "#).expect("CloseWatcher frame lifecycle");
        assert_eq!(result, "ok", "removal during {phase}");
    }
}

#[test]
fn close_watcher_methods_handle_cancellation_destruction_and_reentrancy() {
    let mut vm = new_storage_test_vm("https://close-watcher.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  for (const method of ['requestClose', 'close', 'destroy']) {
    const watcher = new CloseWatcher();
    const events = [];
    watcher.addEventListener('cancel', () => events.push('cancel'));
    watcher.addEventListener('close', () => events.push('close'));
    check(watcher[method]() === undefined, method + ' return value');
    for (const later of ['requestClose', 'close', 'destroy']) watcher[later]();
    check(events.join() === ({requestClose: 'cancel,close', close: 'close', destroy: ''})[method],
          method + ' must deactivate exactly once: ' + events);
  }
  for (const eventType of ['cancel', 'close']) {
    for (const method of ['requestClose', 'close', 'destroy']) {
      const watcher = new CloseWatcher();
      const events = [];
      watcher.addEventListener('cancel', () => events.push('cancel'));
      watcher.addEventListener('close', () => events.push('close'));
      watcher.addEventListener(eventType, () => watcher[method]());
      watcher.requestClose();
      check(events.join() === (eventType === 'cancel' && method === 'destroy' ? 'cancel' : 'cancel,close'),
            eventType + '/' + method + ': ' + events);
    }
  }
  const watcher = new CloseWatcher();
  const events = [];
  watcher.oncancel = e => { events.push(e.cancelable); e.preventDefault(); };
  watcher.onclose = () => events.push('close');
  watcher.requestClose();
  watcher.requestClose();
  check(events.join() === 'true,true', 'programmatic cancellation does not need user activation');
  watcher.close();
  check(events.join() === 'true,true,close', 'close bypasses cancel');
  return 'ok';
})()
"#).expect("CloseWatcher programmatic lifecycle");
    assert_eq!(result, "ok");
}

#[test]
fn close_watcher_abort_algorithms_precede_listeners_and_ignore_public_signal_properties() {
    let mut vm = new_storage_test_vm("https://close-watcher-abort.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  const aborted = new AbortController();
  aborted.abort();
  const inactive = new CloseWatcher({signal: aborted.signal});
  let inactiveEvents = 0;
  inactive.oncancel = inactive.onclose = () => ++inactiveEvents;
  inactive.requestClose();
  inactive.close();
  check(inactiveEvents === 0, 'already aborted watcher must remain inactive');
  for (const phase of ['before', 'cancel', 'close']) {
    const controller = new AbortController();
    let watcher;
    const events = [];
    controller.signal.addEventListener('abort', () => watcher.requestClose());
    Object.defineProperty(controller.signal, 'aborted', {get() { throw new Error('public aborted getter'); }});
    controller.signal.addEventListener = () => { throw new Error('public addEventListener'); };
    watcher = new CloseWatcher({signal: controller.signal});
    watcher.oncancel = () => {
      events.push('cancel');
      if (phase === 'cancel') controller.abort();
    };
    watcher.onclose = () => {
      events.push('close');
      if (phase === 'close') controller.abort();
    };
    if (phase === 'before') controller.abort();
    watcher.requestClose();
    watcher.close();
    check(events.join() === ({before: '', cancel: 'cancel', close: 'cancel,close'})[phase],
          phase + ': ' + events);
  }
  for (const signal of [null, false, 0, {}, Object.create(AbortSignal.prototype)]) {
    let caught;
    try { new CloseWatcher({signal}); } catch (error) { caught = error; }
    check(caught instanceof TypeError, 'signal must be a branded AbortSignal');
  }
  const sentinel = {};
  let caught;
  try { new CloseWatcher({get signal() { throw sentinel; }}); } catch (error) { caught = error; }
  check(caught === sentinel, 'propagate dictionary getter exception');
  let reads = 0;
  new CloseWatcher(new Proxy({}, {
    has() { throw new Error('dictionary conversion must not use HasProperty'); },
    get(target, key) { check(key === 'signal', 'only read signal'); ++reads; }
  })).destroy();
  check(reads === 1, 'read the dictionary member once');
  for (const options of [false, 1, 'x', Symbol(), 1n]) {
    let caught;
    try { new CloseWatcher(options); } catch (error) { caught = error; }
    check(caught instanceof TypeError, 'non-object options must throw');
  }
  for (const options of [undefined, null, {}, {signal: undefined}]) new CloseWatcher(options).destroy();
  return 'ok';
})()
"#).expect("CloseWatcher AbortSignal integration");
    assert_eq!(result, "ok");
}

#[test]
fn close_watcher_handlers_keep_listener_order_and_support_return_false() {
    let mut vm = new_storage_test_vm("https://close-watcher-handlers.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  const watcher = new CloseWatcher();
  const events = [];
  check(watcher.oncancel === null && watcher.onclose === null, 'initial handlers');
  watcher.addEventListener('cancel', () => events.push('first'));
  watcher.oncancel = () => events.push('old');
  watcher.addEventListener('cancel', () => events.push('last'));
  watcher.oncancel = function(event) {
    check(this === watcher && event.currentTarget === watcher, 'handler receiver');
    events.push('handler');
    return false;
  };
  watcher.onclose = () => events.push('close');
  watcher.requestClose();
  check(events.join() === 'first,handler,last', 'replacement keeps listener position and cancels');
  watcher.oncancel = null;
  watcher.oncancel = () => { events.push('new'); return false; };
  watcher.requestClose();
  check(events.join() === 'first,handler,last,first,last,new', 'reactivated handler moves to end');
  watcher.oncancel = 7;
  check(watcher.oncancel === null, 'non-object clears handler');
  watcher.requestClose();
  check(events.slice(-3).join() === 'first,last,close', 'cleared handler no longer cancels');
  return 'ok';
})()
"#,
        )
        .expect("CloseWatcher event handler semantics");
    assert_eq!(result, "ok");
}

#[test]
fn close_watcher_uses_intrinsic_events_and_branded_prototype_members() {
    let mut vm = new_storage_test_vm("https://close-watcher-interface.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  check(CloseWatcher.length === 0 && CloseWatcher.name === 'CloseWatcher', 'constructor metadata');
  check(Object.getPrototypeOf(CloseWatcher.prototype) === EventTarget.prototype, 'prototype inheritance');
  class Derived extends CloseWatcher {}
  const watcher = new Derived();
  check(watcher instanceof Derived && watcher instanceof CloseWatcher && watcher instanceof EventTarget, 'brands');
  check(Object.prototype.toString.call(watcher) === '[object CloseWatcher]', 'toStringTag');
  for (const member of ['requestClose', 'close', 'destroy', 'oncancel', 'onclose']) {
    const descriptor = Object.getOwnPropertyDescriptor(CloseWatcher.prototype, member);
    check(descriptor.enumerable && descriptor.configurable, member + ' descriptor');
    const functions = descriptor.value ? [descriptor.value] : [descriptor.get, descriptor.set];
    for (const fn of functions) {
      for (const receiver of [{}, new EventTarget(), Object.create(CloseWatcher.prototype)]) {
        let caught;
        try { fn.call(receiver); } catch (error) { caught = error; }
        check(caught instanceof TypeError, member + ' must reject unbranded receivers');
      }
    }
  }
  const OriginalEvent = Event;
  const events = [];
  for (const type of ['cancel', 'close']) {
    watcher.addEventListener(type, event => {
      check(Object.getPrototypeOf(event) === OriginalEvent.prototype, 'intrinsic Event prototype');
      check(event.type === type && event.isTrusted, 'trusted event type');
      check(!event.bubbles && !event.composed && event.cancelable === (type === 'cancel'), 'event flags');
      check(event.target === watcher && event.currentTarget === watcher, 'event targets');
      events.push(event);
    });
  }
  globalThis.Event = function() { throw new Error('replaced Event constructor'); };
  watcher.requestClose();
  check(events.length === 2, 'cancel and close fired');
  check(events.every(event => event.currentTarget === null), 'dispatch cleanup');
  return 'ok';
})()
"#).expect("CloseWatcher interface and intrinsic events");
    assert_eq!(result, "ok");
}
