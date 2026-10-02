(() => {
  const rows = [], defaultWhich = {};
  const check = (name, run) => {
    try {
      if (run() !== true) throw new Error('assertion failed');
      rows.push({name, passed: true});
    } catch (error) {
      rows.push({name, passed: false, error: String(error), stack: error.stack});
    }
  };
  const other = document.getElementById('child').contentWindow;
  const typeError = (run, realm = globalThis) => {
    try { run(); } catch (error) {
      return error instanceof realm.TypeError &&
        (realm === globalThis || !(error instanceof TypeError));
    }
    return false;
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const make = (name, realm = globalThis) => {
    if (name === 'TextEvent') {
      const event = realm.document.createEvent(name);
      event.initTextEvent('event', false, false, null, 'text');
      return event;
    }
    return new realm[name]('event', {
      view: realm, detail: 3, data: 'text', inputType: 'insertText', isComposing: true,
      key: 'K', code: 'KeyK', location: 2, ctrlKey: true, shiftKey: true,
      altKey: true, metaKey: true, repeat: true, charCode: 75, keyCode: 75
    });
  };
  const declarations = [
    ['Event', ['type', 'target', 'currentTarget', 'eventPhase', 'bubbles', 'cancelable',
      'defaultPrevented', 'composed', 'srcElement', 'timeStamp', 'cancelBubble', 'returnValue']],
    ['UIEvent', ['view', 'detail', 'which']],
    ['FocusEvent', ['relatedTarget']],
    ['InputEvent', ['data', 'inputType', 'isComposing']],
    ['KeyboardEvent', ['key', 'code', 'location', 'ctrlKey', 'shiftKey', 'altKey',
      'metaKey', 'repeat', 'isComposing', 'charCode', 'keyCode']],
    ['CompositionEvent', ['data']],
    ['TextEvent', ['data']]
  ];
  for (const [name, attributes] of declarations) {
    for (const attribute of attributes) {
      const descriptor = Object.getOwnPropertyDescriptor(globalThis[name].prototype, attribute);
      check(`${name}.${attribute} prototype descriptor`, () => {
        const mutable = name === 'Event' && ['cancelBubble', 'returnValue'].includes(attribute);
        return !!descriptor && descriptor.enumerable && descriptor.configurable &&
          typeof descriptor.get === 'function' && descriptor.get.length === 0 &&
          descriptor.get.name === `get ${attribute}` &&
          (mutable ? typeof descriptor.set === 'function' : descriptor.set === undefined);
      });
      check(`${name}.${attribute} instance inherits accessor`, () => {
        const event = make(name);
        return !own(event, attribute) && attribute in event &&
          Object.is(descriptor.get.call(event), event[attribute]);
      });
      check(`${name}.${attribute} genuine receiver ignores public prototype`, () => {
        const event = make(name), value = event[attribute];
        Object.setPrototypeOf(event, null);
        return Object.is(descriptor.get.call(event), value);
      });
      check(`${name}.${attribute} borrowed foreign receiver`, () => {
        const event = make(name, other);
        return Object.is(descriptor.get.call(event), event[attribute]);
      });
      check(`${name}.${attribute} rejects forged and proxy receivers without traps`, () => {
        const event = make(name), revocable = Proxy.revocable(event, {});
        let traps = 0;
        const proxy = new Proxy(event, {
          get() {traps++;}, getPrototypeOf() {traps++;}, has() {traps++;}
        });
        revocable.revoke();
        return [{}, globalThis[name].prototype, Object.create(globalThis[name].prototype),
          Object.create(event), proxy, revocable.proxy].every(
          value => typeError(() => descriptor.get.call(value))) && traps === 0;
      });
      check(`${name}.${attribute} foreign getter throws in callee realm`, () => {
        const getter = Object.getOwnPropertyDescriptor(other[name].prototype, attribute).get;
        return typeError(() => getter.call({}), other) &&
          typeError(() => getter.call(new Proxy(make(name), {})), other);
      });
    }
  }
  for (const name of ['UIEvent', 'FocusEvent', 'InputEvent', 'KeyboardEvent',
    'CompositionEvent', 'TextEvent', 'MouseEvent', 'WheelEvent', 'PointerEvent', 'TouchEvent']) {
    check(`${name} inherits base and UI attributes`, () => {
      const event = name === 'TextEvent' ? make(name) : new globalThis[name]('event');
      defaultWhich[name] = event.which;
      return ['type', 'view', 'detail', 'which'].every(attribute => !own(event, attribute)) &&
        typeof event.which === 'number' &&
        Object.getOwnPropertyDescriptor(UIEvent.prototype, 'which').get.call(event) === event.which;
    });
  }
  check('non-mouse UI event classes default which to zero', () =>
    ['UIEvent', 'FocusEvent', 'InputEvent', 'KeyboardEvent', 'CompositionEvent', 'TextEvent']
      .every(name => defaultWhich[name] === 0));
  for (const [suffix, value] of [['STANDARD', 0], ['LEFT', 1], ['RIGHT', 2], ['NUMPAD', 3]]) {
    const name = `DOM_KEY_LOCATION_${suffix}`;
    for (const object of [KeyboardEvent, KeyboardEvent.prototype]) {
      check(`${object === KeyboardEvent ? 'KeyboardEvent' : 'KeyboardEvent.prototype'}.${name}`, () => {
        const d = Object.getOwnPropertyDescriptor(object, name);
        return !!d && d.value === value && d.enumerable && !d.writable && !d.configurable;
      });
    }
    check(`KeyboardEvent instance inherits ${name}`, () => {
      const event = make('KeyboardEvent');
      return event[name] === value && !own(event, name);
    });
  }
  for (const [interfaceName, attribute, wrong] of [
    ['UIEvent', 'view', new Event('event')],
    ['KeyboardEvent', 'key', new UIEvent('event')],
    ['CompositionEvent', 'data', new InputEvent('event')],
    ['TextEvent', 'data', new CompositionEvent('event')],
    ['InputEvent', 'isComposing', new KeyboardEvent('event')],
    ['FocusEvent', 'relatedTarget', new MouseEvent('event')]
  ]) {
    check(`${interfaceName}.${attribute} rejects unrelated genuine interface`, () =>
      typeError(() => Object.getOwnPropertyDescriptor(globalThis[interfaceName].prototype, attribute).get.call(wrong)));
  }
  check('UI view retains original foreign Window identity', () => {
    const event = new UIEvent('event', {view: other});
    return event.view === other &&
      Object.getOwnPropertyDescriptor(other.UIEvent.prototype, 'view').get.call(event) === other;
  });
  check('isTrusted remains an own unforgeable accessor', () => {
    const event = make('KeyboardEvent'), d = Object.getOwnPropertyDescriptor(event, 'isTrusted');
    return !!d && typeof d.get === 'function' && d.set === undefined && d.enumerable &&
      !d.configurable && !own(Event.prototype, 'isTrusted') && event.isTrusted === false;
  });
  check('InputEvent payload and composing state remain unchanged', () => {
    const event = make('InputEvent');
    return event.data === 'text' && event.inputType === 'insertText' && event.isComposing === true;
  });
  check('KeyboardEvent values remain unchanged', () => {
    const event = make('KeyboardEvent');
    return event.key === 'K' && event.code === 'KeyK' && event.location === 2 &&
      event.ctrlKey && event.shiftKey && event.altKey && event.metaKey && event.repeat &&
      event.isComposing && event.charCode === 75 && event.keyCode === 75;
  });
  check('frozen wrapper still exposes legacy initialization from private state', () => {
    const event = Object.freeze(new UIEvent('before', {view: window, detail: 7}));
    event.initUIEvent('after', true, true, null, 11);
    return event.type === 'after' && event.bubbles && event.cancelable && event.view === null && event.detail === 11;
  });
  check('prototype replacement is observable without running author code during construction', () => {
    const prototype = KeyboardEvent.prototype;
    const d = Object.getOwnPropertyDescriptor(prototype, 'key');
    let reads = 0, writes = 0;
    try {
      Object.defineProperty(prototype, 'key', {
        get() {reads++; return 'public';}, set() {writes++;}, configurable: true
      });
      const event = new KeyboardEvent('event', {key: 'native'});
      if (reads || writes || own(event, 'key') || d.get.call(event) !== 'native') return false;
      return event.key === 'public' && reads === 1 && writes === 0;
    } finally { Object.defineProperty(prototype, 'key', d); }
  });
  check('deleted prototype attribute is not replaced with an own getter', () => {
    const d = Object.getOwnPropertyDescriptor(CompositionEvent.prototype, 'data');
    try {
      delete CompositionEvent.prototype.data;
      const event = new CompositionEvent('event', {data: 'native'});
      return !own(event, 'data') && event.data === undefined && d.get.call(event) === 'native';
    } finally { Object.defineProperty(CompositionEvent.prototype, 'data', d); }
  });
  check('private event state ignores Object.prototype pollution', () => {
    const d = Object.getOwnPropertyDescriptor(Object.prototype, 'which');
    try {
      Object.defineProperty(Object.prototype, 'which', {get() {throw new Error('polluted');}, configurable: true});
      return new FocusEvent('event').which === 0 && new InputEvent('event').which === 0;
    } finally {
      if (d) Object.defineProperty(Object.prototype, 'which', d);
      else delete Object.prototype.which;
    }
  });
  check('dispatch ignores author shadows of type and defaultPrevented', () => {
    const target = document.createElement('div'), event = new UIEvent('native', {cancelable: true});
    let calls = 0;
    target.addEventListener('native', e => {calls++; e.preventDefault();});
    Object.defineProperty(event, 'type', {get() {throw new Error('public type');}});
    Object.defineProperty(event, 'defaultPrevented', {value: false});
    return target.dispatchEvent(event) === false && calls === 1 && event.defaultPrevented === false &&
      Object.getOwnPropertyDescriptor(Event.prototype, 'defaultPrevented').get.call(event) === true;
  });
  globalThis.__uiEventResults = {
    rows, passed: rows.filter(row => row.passed).length, total: rows.length,
    isSecureContext, origin: location.origin, defaultWhich
  };
  return rows.every(row => row.passed);
})()
