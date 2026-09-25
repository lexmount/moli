use super::*;

fn close_watcher_input_vm() -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://close-watcher-input.test/");
    setup_close_watcher_input_document(&mut vm);
    vm
}

fn setup_close_watcher_input_document(vm: &mut ScriptVm) {
    vm.eval(r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        globalThis.events = [];
        globalThis.record = name => {
          const watcher = new CloseWatcher();
          watcher.addEventListener('cancel', e => events.push(name + ':cancel:' + e.cancelable + ':' + e.isTrusted));
          watcher.addEventListener('close', e => events.push(name + ':close:' + e.isTrusted));
          return watcher;
        };
    "#).expect("prepare CloseWatcher input document");
}

fn close_watcher_key(vm: &mut ScriptVm, event: &str, key: &str) {
    vm.dispatch_key_event(event, key, key, "", 0, false, false)
        .expect("native key input");
}

fn close_watcher_protocol(
    vm: &mut ScriptVm,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({"id": 1001, "method": method, "params": params}).to_string(),
        )
        .expect("CloseWatcher protocol command");
    let response = messages
        .into_iter()
        .find(|message| message["id"] == 1001)
        .expect("protocol response");
    assert!(response.get("error").is_none(), "{response}");
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
    response["result"].clone()
}

#[test]
fn close_watcher_protocol_activation_survives_evaluate_and_call_function_on() {
    let mut vm = close_watcher_input_vm();
    close_watcher_protocol(&mut vm, "Runtime.enable", serde_json::json!({}));
    for use_object_id in [false, true] {
        vm.eval("events.length = 0; globalThis.watcher = record('watcher'); watcher.oncancel = e => e.preventDefault()").unwrap();
        if use_object_id {
            let object = close_watcher_protocol(
                &mut vm,
                "Runtime.evaluate",
                serde_json::json!({"expression":"({})"}),
            );
            close_watcher_protocol(
                &mut vm,
                "Runtime.callFunctionOn",
                serde_json::json!({"objectId":object["result"]["objectId"],"functionDeclaration":"function() {}","userGesture":true}),
            );
        } else {
            close_watcher_protocol(
                &mut vm,
                "Runtime.evaluate",
                serde_json::json!({"expression":"0","userGesture":true}),
            );
        }
        close_watcher_key(&mut vm, "keydown", "Escape");
        close_watcher_key(&mut vm, "keydown", "Escape");
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "watcher:cancel:true:true|watcher:cancel:false:true|watcher:close:true"
        );
    }
}

#[test]
fn close_watcher_groups_are_shared_by_isolated_realms_and_release_retired_watchers() {
    let mut vm = close_watcher_input_vm();
    close_watcher_protocol(&mut vm, "Runtime.enable", serde_json::json!({}));
    vm.eval("record('main')").unwrap();
    let isolated = vm
        .create_isolated_world("close-watcher", true)
        .expect("isolated world");
    vm.exec_in_execution_context(
        isolated,
        "globalThis.watcherClosed = false; new CloseWatcher().onclose = () => globalThis.watcherClosed = true",
    )
    .unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "main:cancel:false:true|main:close:true"
    );
    let result = close_watcher_protocol(
        &mut vm,
        "Runtime.evaluate",
        serde_json::json!({"expression":"globalThis.watcherClosed","contextId":isolated,"returnByValue":true}),
    );
    assert_eq!(result["result"]["value"], true);

    vm.eval("events.length = 0; record('main')").unwrap();
    close_watcher_key(&mut vm, "keydown", "x");
    vm.exec_in_execution_context(isolated, "new CloseWatcher()")
        .unwrap();
    vm.destroy_isolated_world_context(isolated);
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "main:cancel:true:true|main:close:true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn close_watcher_activation_and_consumption_follow_the_focused_frame_tree() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    for sandboxed in [false, true] {
        let mut vm = new_storage_test_vm_with_loader("https://close-watcher-input.test/", &loader);
        setup_close_watcher_input_document(&mut vm);
        close_watcher_protocol(&mut vm, "Runtime.enable", serde_json::json!({}));
        vm.eval(&format!(
            r#"
            globalThis.watcher = record('parent');
            watcher.oncancel = e => e.preventDefault();
            globalThis.frame = document.createElement('iframe');
            if ({sandboxed}) frame.sandbox = 'allow-scripts';
            frame.srcdoc = '<!doctype html><body>child</body>';
            document.body.appendChild(frame);
        "#
        ))
        .unwrap();
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "CloseWatcher input frame")
            .await;
        let realms = vm.live_child_default_runtime_realm_inventory();
        assert_eq!(realms.len(), 1);
        let child = realms[0].context_id;
        let unique_id = realms[0]
            .realm_id
            .as_ref()
            .expect("child unique context ID");
        vm.exec_in_execution_context(child, r#"
            if (!document.body) document.documentElement.appendChild(document.createElement('body'));
            document.body.tabIndex = -1;
            globalThis.events = [];
            globalThis.record = () => {
                const watcher = new CloseWatcher();
                watcher.oncancel = e => { events.push('cancel:' + e.cancelable); e.preventDefault(); };
                watcher.onclose = () => events.push('close');
            };
            record();
        "#).unwrap();

        // Parent input reaches same-origin descendants, but not an opaque child.
        close_watcher_key(&mut vm, "keydown", "x");
        vm.exec_in_execution_context(child, "document.body.focus()")
            .unwrap();
        close_watcher_key(&mut vm, "keydown", "Escape");
        let child_events = close_watcher_protocol(
            &mut vm,
            "Runtime.evaluate",
            serde_json::json!({
                "contextId":child, "expression":"events.join('|')", "returnByValue":true,
            }),
        );
        assert_eq!(
            child_events["result"]["value"],
            if sandboxed {
                "cancel:false|close"
            } else {
                "cancel:true"
            }
        );
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "",
            "Esc must target the focused child"
        );

        // A canceled child request consumes activation in the entire tree.
        vm.eval("document.body.tabIndex = -1; document.body.focus()")
            .unwrap();
        close_watcher_key(&mut vm, "keydown", "Escape");
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            if sandboxed {
                "parent:cancel:true:true"
            } else {
                "parent:cancel:false:true|parent:close:true"
            }
        );
        vm.eval("watcher.destroy(); events.length = 0; globalThis.watcher = record('parent'); watcher.oncancel = e => e.preventDefault()").unwrap();

        // Protocol activation in a child reaches its ancestors regardless of origin.
        close_watcher_protocol(
            &mut vm,
            "Runtime.evaluate",
            serde_json::json!({
                "uniqueContextId":unique_id, "expression":"0", "userGesture":true,
            }),
        );
        close_watcher_key(&mut vm, "keydown", "Escape");
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "parent:cancel:true:true"
        );
    }
}

#[test]
fn close_watcher_native_escape_honors_keydown_cancellation_and_ignores_synthetic_input() {
    let mut vm = close_watcher_input_vm();
    vm.eval(
        r#"
        record('first'); record('second');
        dispatchEvent(new KeyboardEvent('keydown', {key: 'x'}));
        dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape'}));
        globalThis.stopEscape = e => { if (e.key === 'Escape') e.preventDefault(); };
        addEventListener('keydown', stopEscape);
    "#,
    )
    .expect("watchers and canceling key listener");
    close_watcher_key(&mut vm, "keydown", "Escape");
    vm.eval("removeEventListener('keydown', stopEscape)")
        .expect("remove key listener");
    close_watcher_key(&mut vm, "keyup", "Escape");
    close_watcher_key(&mut vm, "keypress", "Escape");
    assert_eq!(vm.eval("events.join('|')").unwrap(), "");
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "second:cancel:false:true|second:close:true|first:cancel:false:true|first:close:true"
    );
}

#[test]
fn close_watcher_native_requests_close_only_the_last_activation_group() {
    let mut vm = close_watcher_input_vm();
    vm.eval("record('first')").unwrap();
    close_watcher_key(&mut vm, "keydown", "x");
    vm.eval("record('second'); record('third')").unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "third:cancel:false:true|third:close:true|second:cancel:false:true|second:close:true"
    );
    vm.eval("events.length = 0").unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "first:cancel:false:true|first:close:true"
    );
}

#[test]
fn close_watcher_canceling_a_request_consumes_activation_until_the_next_input() {
    let mut vm = close_watcher_input_vm();
    vm.eval("globalThis.watcher = record('watcher'); watcher.oncancel = e => e.preventDefault()")
        .unwrap();
    close_watcher_key(&mut vm, "keydown", "x");
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "watcher:cancel:true:true"
    );
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "watcher:cancel:true:true|watcher:cancel:false:true|watcher:close:true"
    );
    vm.eval("events.length = 0; globalThis.next = record('next'); next.oncancel = e => e.preventDefault()").unwrap();
    close_watcher_key(&mut vm, "keydown", "x");
    vm.eval("next.requestClose()").unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "next:cancel:true:true|next:cancel:false:true|next:close:true"
    );
}

#[test]
fn close_watcher_user_activation_cannot_bank_unlimited_groups() {
    let mut vm = close_watcher_input_vm();
    for _ in 0..4 {
        close_watcher_key(&mut vm, "keydown", "x");
    }
    vm.eval("record('first'); record('second'); record('third')")
        .unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "third:cancel:false:true|third:close:true|second:cancel:false:true|second:close:true"
    );
    vm.eval("events.length = 0").unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "first:cancel:false:true|first:close:true"
    );
}

#[test]
fn close_watcher_native_group_snapshot_survives_mutation_inside_cancel() {
    let mut vm = close_watcher_input_vm();
    vm.eval(
        r#"
        const first = record('first'); record('second');
        const third = record('third');
        third.oncancel = () => { first.destroy(); record('new'); };
    "#,
    )
    .unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "third:cancel:false:true|third:close:true|second:cancel:false:true|second:close:true"
    );
    vm.eval("events.length = 0").unwrap();
    close_watcher_key(&mut vm, "keydown", "Escape");
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "new:cancel:false:true|new:close:true"
    );
}

#[test]
fn close_watcher_mouse_and_touch_activate_at_their_respective_input_phases() {
    for touch in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://close-watcher-input.test/",
            "<html><body><div>input target</div></body></html>",
        );
        setup_close_watcher_input_document(&mut vm);
        vm.eval("record('first').oncancel = e => e.preventDefault()")
            .unwrap();
        vm.publish_layout_for_test().unwrap();
        if touch {
            vm.dispatch_touch_event_at_point(10.0, 11.0, "touchstart", false)
                .unwrap();
        } else {
            vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousemove", 0, Some(0), 0.0, 0.0)
                .unwrap();
        }
        close_watcher_key(&mut vm, "keydown", "Escape");
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "first:cancel:false:true|first:close:true"
        );
        vm.eval("events.length = 0; record('next').oncancel = e => e.preventDefault()")
            .unwrap();
        if touch {
            vm.dispatch_touch_event_at_point(10.0, 11.0, "touchend", false)
                .unwrap();
        } else {
            // Canceling pointerdown suppresses the compatibility mousedown,
            // but the trusted pointer input still activates this Window.
            vm.eval("addEventListener('pointerdown', e => e.preventDefault())")
                .unwrap();
            vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, Some(1), 0.0, 0.0)
                .unwrap();
        }
        close_watcher_key(&mut vm, "keydown", "Escape");
        close_watcher_key(&mut vm, "keydown", "Escape");
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "next:cancel:true:true|next:cancel:false:true|next:close:true"
        );
    }
}

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
