(() => {
  const rows = [];
  const check = (name, passed) => rows.push({name, passed: !!passed});
  const throws = (fn, Expected = TypeError) => {
    try { fn(); return false; } catch (error) { return error instanceof Expected; }
  };
  const throwsExactly = (fn, expected) => {
    try { fn(); return false; } catch (error) { return error === expected; }
  };
  const raw = 'new\ud800\0\udc00';
  const flags = {bubbles: false, cancelable: true, composed: true};
  const detail = {payload: 'detail'};
  const cases = [
    {name: 'Event', method: 'initEvent', make: w => new w.Event('before', flags), tail: () => []},
    {name: 'CustomEvent', method: 'initCustomEvent',
      make: w => new w.CustomEvent('before', {...flags, detail: 'old'}), tail: () => [detail]},
    {name: 'UIEvent', method: 'initUIEvent',
      make: w => new w.UIEvent('before', flags), tail: w => [w, 7]},
    {name: 'MouseEvent', method: 'initMouseEvent',
      make: w => new w.MouseEvent('before', flags),
      tail: w => [w, 7, 1, 2, 3, 4, true, false, true, false, 1, null]},
    {name: 'KeyboardEvent', method: 'initKeyboardEvent',
      make: w => new w.KeyboardEvent('before', flags), tail: w => [w]},
    {name: 'CompositionEvent', method: 'initCompositionEvent',
      make: w => new w.CompositionEvent('before', flags), tail: w => [w, 'data']},
    {name: 'StorageEvent', method: 'initStorageEvent',
      make: w => new w.StorageEvent('before', flags), tail: () => ['key', 'old', 'new', 'https://event.test/', null]},
    {name: 'TextEvent', method: 'initTextEvent', make: w => {
      const event = w.document.createEvent('TextEvent');
      event.initEvent('before', false, true);
      return event;
    }, tail: w => [w, 'data']}
  ];
  for (const entry of cases) {
    const event = entry.make(window);
    const method = event[entry.method];
    const initialize = (receiver, type, bubbles = true, cancelable = false) =>
      method.call(receiver, type, bubbles, cancelable, ...entry.tail(window));
    const stamp = event.timeStamp;
    const composed = event.composed;
    event.preventDefault();
    event.stopImmediatePropagation();
    const started = Date.now();
    while (Date.now() - started < 3) {}
    check(entry.name + ' method metadata', method.name === entry.method && method.length === 1);
    check(entry.name + ' returns undefined', initialize(event, raw) === undefined);
    check(entry.name + ' preserves DOMString units', event.type === raw);
    check(entry.name + ' preserves composed', event.composed === composed);
    check(entry.name + ' preserves timestamp', event.timeStamp === stamp);
    check(entry.name + ' reinitializes flags', event.bubbles && !event.cancelable &&
      !event.defaultPrevented && event.returnValue && !event.cancelBubble && !event.isTrusted);
    let called = 0;
    const target = new EventTarget();
    target.addEventListener(raw, received => { if (received === event) called++; });
    check(entry.name + ' propagation flags cleared', target.dispatchEvent(event) && called === 1);
    check(entry.name + ' dispatched target', event.target === target && event.srcElement === target);
    initialize(event, raw);
    check(entry.name + ' target cleared', event.target === null && event.srcElement === null &&
      event.currentTarget === null && event.eventPhase === Event.NONE && event.composedPath().length === 0);
    check(entry.name + ' repeated timestamp', event.timeStamp === stamp);
    check(entry.name + ' repeated composed', event.composed === composed);
    check(entry.name + ' Symbol rejects', throws(() => initialize(event, Symbol())) && event.type === raw);
    check(entry.name + ' missing type rejects', throws(() => method.call(event)) && event.type === raw);
    let conversions = 0;
    const boolean = {valueOf() { throw new Error('boolean conversion must not coerce'); }};
    check(entry.name + ' conversion order', initialize(event, {toString() {
      conversions++; return raw;
    }}, boolean, boolean) === undefined && conversions === 1 && event.type === raw &&
      event.bubbles && event.cancelable);
    event.preventDefault();
    const sentinel = new RangeError('type conversion');
    check(entry.name + ' conversion failure preserves state', throwsExactly(() => initialize(event, {
      toString() { throw sentinel; }
    }), sentinel) && event.defaultPrevented && event.type === raw && event.timeStamp === stamp);
    let traps = 0;
    const proxy = new Proxy(event, {get() { traps++; throw sentinel; }, getPrototypeOf() { traps++; throw sentinel; }});
    const revoked = Proxy.revocable(event, {});
    revoked.revoke();
    for (const receiver of [null, undefined, {}, Object.getPrototypeOf(event), Object.create(event), proxy, revoked.proxy]) {
      conversions = 0;
      check(entry.name + ' invalid receiver', throws(() => initialize(receiver, {toString() {
        conversions++; return raw;
      }})) && conversions === 0);
    }
    check(entry.name + ' author Proxy traps skipped', traps === 0);
    const dispatched = entry.make(window);
    const beforeStamp = dispatched.timeStamp;
    const beforeComposed = dispatched.composed;
    const dispatchTarget = new EventTarget();
    let dispatchConversions = 0;
    let dispatchChecks = false;
    dispatchTarget.addEventListener('before', current => {
      current.preventDefault();
      const type = {toString() { dispatchConversions++; return raw; }};
      initialize(current, type);
      const failure = throwsExactly(() => initialize(current, {toString() { throw sentinel; }}), sentinel);
      const missing = throws(() => method.call(current));
      const symbol = throws(() => initialize(current, Symbol()));
      dispatchChecks = failure && missing && symbol && current.type === 'before' &&
        !current.bubbles && current.cancelable && current.defaultPrevented &&
        current.target === dispatchTarget && current.currentTarget === dispatchTarget &&
        current.eventPhase === Event.AT_TARGET && current.composed === beforeComposed &&
        current.timeStamp === beforeStamp;
    });
    check(entry.name + ' dispatch remains canceled', dispatchTarget.dispatchEvent(dispatched) === false);
    check(entry.name + ' converts during dispatch', dispatchConversions === 1);
    check(entry.name + ' dispatch guard follows conversion', dispatchChecks);
    initialize(dispatched, raw);
    check(entry.name + ' reusable after dispatch', dispatched.type === raw && !dispatched.defaultPrevented &&
      dispatched.target === null && dispatched.timeStamp === beforeStamp && dispatched.composed === beforeComposed);
  }
  const payloadCases = [
    ['CustomEvent', () => new CustomEvent('before', {...flags, detail}), e => e.detail],
    ['UIEvent', () => new UIEvent('before', {...flags, detail: 9, view: window}), e => e.detail],
    ['MouseEvent', () => new MouseEvent('before', {...flags, clientX: 9}), e => e.clientX],
    ['KeyboardEvent', () => new KeyboardEvent('before', {...flags, key: 'key'}), e => e.key],
    ['CompositionEvent', () => new CompositionEvent('before', {...flags, data: 'data'}), e => e.data],
    ['StorageEvent', () => new StorageEvent('before', {...flags, key: 'key'}), e => e.key],
    ['AnimationEvent', () => new AnimationEvent('before', {...flags, animationName: 'animation'}), e => e.animationName],
    ['TransitionEvent', () => new TransitionEvent('before', {...flags, propertyName: 'color'}), e => e.propertyName]
  ];
  for (const [name, make, payload] of payloadCases) {
    const event = make();
    const value = payload(event);
    const stamp = event.timeStamp;
    Event.prototype.initEvent.call(event, raw);
    check(name + ' base initializer DOMString', event.type === raw);
    check(name + ' base initializer retains payload', payload(event) === value);
    check(name + ' base initializer retains composed', event.composed);
    check(name + ' base initializer retains timestamp', event.timeStamp === stamp);
  }
  const custom = new CustomEvent('before', {...flags, detail});
  const customStamp = custom.timeStamp;
  custom.initCustomEvent(raw);
  check('CustomEvent default detail', custom.detail === null && custom.composed && custom.timeStamp === customStamp);
  custom.initCustomEvent(raw, false, false, undefined);
  check('CustomEvent explicit undefined detail', custom.detail === null);
  custom.initCustomEvent(raw, false, false, detail);
  check('CustomEvent detail identity', custom.detail === detail);
  for (const value of [undefined, null, 7, 1n]) {
    const event = new Event('before');
    event.initEvent(value);
    check('Event DOMString ' + String(value), event.type === String(value));
  }
  const child = document.querySelector('#child').contentWindow;
  for (const entry of cases) {
    const event = entry.make(child);
    const method = event[entry.method];
    const stamp = event.timeStamp;
    const composed = event.composed;
    const local = entry.make(window);
    local[entry.method].call(event, raw, true, false, ...entry.tail(window));
    check(entry.name + ' cross realm receiver', event.type === raw && event.timeStamp === stamp &&
      event.composed === composed && event instanceof child.Event);
    let conversions = 0;
    check(entry.name + ' callee realm receiver error', throws(() => method.call({}, {toString() {
      conversions++; return raw;
    }}), child.TypeError) && conversions === 0);
    check(entry.name + ' callee realm conversion error', throws(() => method.call(event, Symbol()), child.TypeError));
  }
  const frozen = Object.freeze(new CustomEvent('before', {...flags, detail}));
  const frozenStamp = frozen.timeStamp;
  frozen.initCustomEvent(raw, true, false, 17);
  check('frozen wrapper reinitialization', frozen.type === raw && frozen.detail === 17 &&
    frozen.composed && frozen.timeStamp === frozenStamp);
  globalThis.__legacyEventInitResults = {rows, passed: rows.filter(row => row.passed).length, total: rows.length};
  return rows.every(row => row.passed) || JSON.stringify(rows.filter(row => !row.passed));
})()
