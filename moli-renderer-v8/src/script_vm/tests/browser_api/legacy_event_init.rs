use super::*;

#[test]
fn legacy_event_initializers_preserve_creation_state_and_convert_before_dispatch_guard() {
    let mut vm = new_parsed_test_vm(
        "https://legacy-event-init.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm.eval(include_str!("legacy_event_init.js")).expect(
        "legacy event initializers should preserve native Event state and WebIDL conversion",
    );
    assert_eq!(result, "true");
}

#[test]
fn legacy_mouse_initializer_converts_nullable_interfaces_and_short_before_mutation() {
    let mut vm = new_parsed_test_vm(
        "https://legacy-mouse-init.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        "globalThis.legacyMouseNativeRelatedTarget = document.implementation.createHTMLDocument('').createElement('select')",
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "legacyMouseNativeRelatedTarget").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        assert!(value.is_proxy(), "fixture must exercise a native Proxy");
        let object = v8::Local::<v8::Object>::try_from(value).unwrap();
        assert!(crate::web_api_interfaces::EventTarget::is_instance(
            scope, object
        ));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            "(() => { const event = new MouseEvent('before'); event.initMouseEvent('after', false, false, null, 0, 0, 0, 0, 0, false, false, false, false, 0, legacyMouseNativeRelatedTarget); return event.relatedTarget === legacyMouseNativeRelatedTarget; })()",
        )
        .unwrap(),
        "true",
    );
    let result = vm
        .eval(include_str!("legacy_mouse_event_init.js"))
        .expect("legacy MouseEvent conversion matrix should execute");
    if result != "true" {
        let failures = vm
            .eval("JSON.stringify(__uiEventResults.rows.filter(row => !row.passed))")
            .expect("legacy MouseEvent failures should be available");
        panic!("legacy MouseEvent conversion failures: {failures}");
    }
}

#[test]
fn legacy_mouse_initializer_synchronizes_flags_and_resets_extended_modifiers() {
    let mut vm = new_parsed_test_vm(
        "https://legacy-mouse-modifiers.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    assert_eq!(vm.eval(r#"
(() => {
  const frame = document.body.appendChild(document.createElement('iframe'));
  const extra = ['AltGraph', 'CapsLock', 'Fn', 'FnLock', 'Hyper', 'NumLock',
    'ScrollLock', 'Super', 'Symbol', 'SymbolLock'];
  for (const w of [window, frame.contentWindow]) {
    for (const C of [w.MouseEvent, w.WheelEvent, w.PointerEvent, w.DragEvent]) {
      const event = new C('before', {ctrlKey: true, modifierAltGraph: true, modifierCapsLock: true});
      if (!event.getModifierState('AltGraph') || !event.getModifierState('Control')) throw Error('initial modifier state');
      w.MouseEvent.prototype.initMouseEvent.call(event, 'after', true, false, w,
        0, 0, 0, 0, 0, false, true, true, false, 0, null);
      if (event.ctrlKey || !event.altKey || !event.shiftKey || event.metaKey) throw Error('legacy modifier flags');
      for (const key of ['Control', 'Alt', 'Shift', 'Meta', ...extra]) {
        if (event.getModifierState(key) !== (key === 'Alt' || key === 'Shift')) throw Error(C.name + ': stale ' + key);
      }
      w.MouseEvent.prototype.initMouseEvent.call(event, 'reset');
      for (const key of ['Control', 'Alt', 'Shift', 'Meta', ...extra]) {
        if (event.getModifierState(key)) throw Error('default reinitialization left ' + key);
      }
      const dispatch = new C('probe', {ctrlKey: true, modifierAltGraph: true});
      const target = w.document.createElement('div');
      let dispatchPreserved = false;
      target.addEventListener('probe', () => {
        w.MouseEvent.prototype.initMouseEvent.call(dispatch, 'ignored');
        dispatchPreserved = dispatch.type === 'probe' && dispatch.ctrlKey && dispatch.getModifierState('AltGraph');
      });
      target.dispatchEvent(dispatch);
      if (!dispatchPreserved) throw Error('reinitialized during dispatch');
    }
  }
  frame.remove();
  return 'ok';
})()
"#).expect("legacy MouseEvent modifier initialization probe"), "ok");
}
