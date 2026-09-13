use super::*;

#[test]
fn event_modifier_state_constructor_keeps_each_modifier_independent() {
    let mut vm = new_storage_test_vm("https://event-modifiers.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const modifiers = [
    ['Control', 'ctrlKey'], ['Shift', 'shiftKey'], ['Alt', 'altKey'], ['Meta', 'metaKey'],
    ...['AltGraph', 'CapsLock', 'Fn', 'FnLock', 'Hyper', 'NumLock', 'ScrollLock',
        'Super', 'Symbol', 'SymbolLock'].map(name => [name, 'modifier' + name])
  ];
  for (const Ctor of [KeyboardEvent, MouseEvent, WheelEvent, PointerEvent, DragEvent]) {
    const empty = new Ctor('event');
    for (const [key, member] of modifiers) {
      if (empty.getModifierState(key)) return Ctor.name + ': default ' + key;
      const event = new Ctor('event', {[member]: true});
      for (const [queried] of modifiers) {
        if (event.getModifierState(queried) !== (queried === key)) {
          return Ctor.name + ': ' + key + ' changed ' + queried;
        }
      }
      for (const value of [false, 0, '', null, undefined]) {
        if (new Ctor('event', {[member]: value}).getModifierState(key)) {
          return Ctor.name + ': boolean conversion ' + key;
        }
      }
      const truthy = {valueOf() { throw new Error('boolean conversion called valueOf'); }};
      if (!new Ctor('event', {[member]: truthy}).getModifierState(key)) return 'truthy ' + key;
      if (member.startsWith('modifier') && member in event) return 'exposed init member ' + member;
    }
    for (const key of ['control', 'Unknown', '', 'Alt\0Graph']) {
      if (empty.getModifierState(key)) return 'unknown modifier ' + key;
    }
  }
  return 'ok';
})()
"#,
        )
        .expect("event modifier constructor probe should run");
    assert_eq!(result, "ok");
}

#[test]
fn event_modifier_state_ignores_shadow_properties_and_checks_receivers() {
    let mut vm = new_storage_test_vm("https://event-modifiers.test/");
    let result = vm.eval(r#"
(() => {
  for (const Ctor of [KeyboardEvent, MouseEvent, WheelEvent, PointerEvent, DragEvent]) {
    const event = new Ctor('event', {ctrlKey: true, modifierAltGraph: true});
    const method = event.getModifierState;
    Object.defineProperty(event, 'ctrlKey', {get() {throw Error('shadow property'); }});
    if (!method.call(event, 'Control') || !method.call(event, 'AltGraph')) return 'shadow ' + Ctor.name;
    Object.setPrototypeOf(event, null);
    if (!method.call(event, 'Control')) return 'changed prototype ' + Ctor.name;
    for (const receiver of [{ctrlKey: true}, Object.create(Ctor.prototype), new Proxy(event, {}), new Event('event')]) {
      let converted = false;
      const key = {toString() {converted = true; return 'Control'; }};
      try {method.call(receiver, key); return 'accepted receiver ' + Ctor.name;}
      catch (error) {if (!(error instanceof TypeError) || converted) return 'receiver order ' + Ctor.name;}
    }
  }
  for (const [method, receiver] of [
    [KeyboardEvent.prototype.getModifierState, new MouseEvent('event')],
    [MouseEvent.prototype.getModifierState, new KeyboardEvent('event')],
  ]) {
    try {method.call(receiver, 'Control'); return 'accepted unrelated interface';}
    catch (error) {if (!(error instanceof TypeError)) return 'wrong receiver error';}
  }
  for (const Ctor of [MouseEvent, KeyboardEvent]) {
    const descriptor = Object.getOwnPropertyDescriptor(Ctor.prototype, 'getModifierState');
    if (!descriptor.writable || !descriptor.enumerable || !descriptor.configurable || descriptor.value.length !== 1) {
      return 'method descriptor ' + Ctor.name;
    }
  }
  return 'ok';
})()
"#).expect("event modifier receiver probe should run");
    assert_eq!(result, "ok");
}

#[test]
fn event_modifier_state_arguments_preserve_webidl_conversion_errors() {
    let mut vm = new_storage_test_vm("https://event-modifiers.test/");
    let result = vm.eval(r#"
(() => {
  for (const Ctor of [KeyboardEvent, MouseEvent, WheelEvent, PointerEvent, DragEvent]) {
    const event = new Ctor('event', {ctrlKey: true});
    for (const args of [[], [Symbol('Control')]]) {
      try {event.getModifierState(...args); return 'accepted argument ' + Ctor.name;}
      catch (error) {if (!(error instanceof TypeError)) return 'wrong argument error';}
    }
    if (event.getModifierState(undefined) || event.getModifierState(null)) return 'nullable conversion';
    let reads = 0;
    if (!event.getModifierState({toString() {reads++; return 'Control'; }}) || reads !== 1) return 'string conversion';
    const sentinel = new RangeError('key conversion');
    try {event.getModifierState({toString() {throw sentinel; }}); return 'swallowed argument exception';}
    catch (error) {if (error !== sentinel) return 'replaced argument exception';}
  }
  return 'ok';
})()
"#).expect("event modifier argument probe should run");
    assert_eq!(result, "ok");
}

#[test]
fn event_modifier_state_dictionary_reads_once_and_propagates_getter_exceptions() {
    let mut vm = new_storage_test_vm("https://event-modifiers.test/");
    let result = vm.eval(r#"
(() => {
  const members = ['ctrlKey', 'altKey', 'metaKey', 'shiftKey',
    ...['AltGraph', 'CapsLock', 'Fn', 'FnLock', 'Hyper', 'NumLock', 'ScrollLock', 'Super', 'Symbol', 'SymbolLock'].map(name => 'modifier' + name)
  ].sort();
  for (const Ctor of [KeyboardEvent, MouseEvent, WheelEvent, PointerEvent, DragEvent]) {
    const reads = [];
    const init = new Proxy({}, {get(target, key) {
      if (members.includes(key)) {reads.push(key); return true;}
    }});
    const event = new Ctor('event', init);
    if (reads.join() !== members.join()) return Ctor.name + ': read order ' + reads;
    if (!event.getModifierState('AltGraph') || reads.length !== members.length) return 'state rereads init';
    const sentinel = new RangeError('modifier getter');
    let laterRead = false;
    try {
      new Ctor('event', {
        get modifierCapsLock() {throw sentinel;},
        get modifierFn() {laterRead = true; return true;},
      });
      return 'swallowed getter exception ' + Ctor.name;
    } catch (error) {
      if (error !== sentinel || laterRead) return 'getter exception order ' + Ctor.name;
    }
  }
  return 'ok';
})()
"#).expect("event modifier dictionary probe should run");
    assert_eq!(result, "ok");
}

#[test]
fn event_modifier_state_legacy_initializers_reset_only_when_allowed() {
    let mut vm = new_storage_test_vm("https://event-modifiers.test/");
    let result = vm.eval(r#"
(() => {
  const init = {ctrlKey: true, modifierAltGraph: true, modifierCapsLock: true};
  for (const Ctor of [MouseEvent, WheelEvent, PointerEvent, DragEvent]) {
    const event = new Ctor('event', init);
    event.initEvent('again');
    if (!event.getModifierState('Control') || !event.getModifierState('AltGraph')) return 'initEvent reset ' + Ctor.name;
    const target = document.createElement('div');
    target.addEventListener('again', () => event.initMouseEvent('ignored'));
    target.dispatchEvent(event);
    if (event.type !== 'again' || !event.getModifierState('Control')) return 'reset during dispatch';
    event.initMouseEvent('changed', false, false, null, 0, 0, 0, 0, 0, false, true, true, false);
    if (event.getModifierState('Control') || event.getModifierState('AltGraph') || event.getModifierState('CapsLock')) return 'retained old modifier';
    if (!event.getModifierState('Alt') || !event.getModifierState('Shift')) return 'legacy modifier arguments';
    event.initMouseEvent('empty');
    if (event.getModifierState('Alt') || event.getModifierState('Shift')) return 'legacy default modifiers';
  }
  const keyboard = new KeyboardEvent('event', init);
  keyboard.initKeyboardEvent('empty');
  if (keyboard.getModifierState('Control') || keyboard.getModifierState('AltGraph') || keyboard.getModifierState('CapsLock')) return 'legacy keyboard reset';
  const created = document.createEvent('MouseEvents');
  created.initMouseEvent('click', false, false, null, 0, 0, 0, 0, 0, true);
  if (!created.getModifierState('Control')) return 'createEvent modifiers';
  return 'ok';
})()
"#).expect("event modifier legacy initializer probe should run");
    assert_eq!(result, "ok");
}

#[test]
fn event_modifier_state_accepts_cross_realm_native_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://event-modifiers.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm.eval(r#"
(() => {
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const child = frame.contentWindow;
  for (const name of ['MouseEvent', 'KeyboardEvent']) {
    const local = new window[name]('event', {shiftKey: true});
    const foreign = new child[name]('event', {ctrlKey: true});
    const localMethod = window[name].prototype.getModifierState;
    const foreignMethod = child[name].prototype.getModifierState;
    if (!localMethod.call(foreign, 'Control') || !foreignMethod.call(local, 'Shift')) return 'cross-realm ' + name;
    try {foreignMethod.call({}, 'Control'); return 'accepted forged receiver';}
    catch (error) {if (!(error instanceof child.TypeError)) return 'wrong exception realm';}
  }
  frame.remove();
  return 'ok';
})()
"#).expect("cross-realm event modifier probe should run");
    assert_eq!(result, "ok");
}
