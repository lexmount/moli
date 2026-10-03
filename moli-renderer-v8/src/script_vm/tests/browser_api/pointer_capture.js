globalThis.__pointerCaptureProbe = realms => {
  const rows = [], errors = [];
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const run = (label, work) => { try { work(); } catch (error) { errors.push({label, error: String(error.stack || error)}); } };
  const names = ['setPointerCapture', 'releasePointerCapture', 'hasPointerCapture'];
  const outcome = work => { try { return {value: work()}; } catch (error) { return {error}; } };
  for (const [r, realm] of realms.entries()) for (const name of names) {
    const prefix = `${r}/${name}`, method = realm.Element.prototype[name];
    run(prefix + '/metadata', () => {
      const d = Object.getOwnPropertyDescriptor(realm.Element.prototype, name);
      record(prefix + '/metadata', {name: method.name === name, length: method.length === 1,
        writable: d.writable, enumerable: d.enumerable, configurable: d.configurable});
    });
    for (const [owner, window] of realms.entries()) {
      const document = window.document;
      const detachedDocument = document.implementation.createHTMLDocument('');
      const element = document.createElement('div');
      document.body.append(element);
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      const receivers = [['live', element], ['detached', document.createElement('div')],
        ['svg', svg], ['windowless', detachedDocument.createElement('div')],
        ['native-proxy', detachedDocument.createElement('select')]];
      for (const [kind, receiver] of receivers) {
        const label = `${prefix}/${owner}/${kind}`;
        run(label + '/required', () => {
          const result = outcome(() => method.call(receiver));
          record(label + '/required', {typeError: result.error instanceof realm.TypeError}, result.error?.name || 'returned');
        });
        for (const [n, value] of [undefined, null, false, 0, -0, NaN, Infinity, -Infinity, 2 ** 32, '0'].entries()) run(label + '/number-' + n, () => {
          const result = outcome(() => method.call(receiver, value));
          const noFrame = kind === 'windowless' || kind === 'native-proxy';
          const expectedValue = name === 'hasPointerCapture' ? false : undefined;
          record(label + '/number-' + n, {result: noFrame || name === 'hasPointerCapture'
            ? !result.error && result.value === expectedValue
            : result.error instanceof realm.DOMException && result.error.name === 'NotFoundError' && result.error.code === 8}, result.error?.name || String(result.value));
        });
        for (const [n, value] of [Symbol('id'), 1n, Object(1n)].entries()) run(label + '/type-' + n, () => {
          const result = outcome(() => method.call(receiver, value));
          record(label + '/type-' + n, {typeError: result.error instanceof realm.TypeError}, result.error?.name || 'returned');
        });
        for (const hook of ['symbol', 'valueOf', 'toString']) run(label + '/throw-' + hook, () => {
          const sentinel = {}, trace = [];
          const value = hook === 'symbol' ? {[Symbol.toPrimitive](hint) {trace.push(hint); throw sentinel;}}
            : hook === 'valueOf' ? {valueOf() {trace.push('valueOf'); throw sentinel;}}
            : {valueOf() {trace.push('valueOf'); return {};}, toString() {trace.push('toString'); throw sentinel;}};
          const result = outcome(() => method.call(receiver, value));
          record(label + '/throw-' + hook, {identity: result.error === sentinel,
            order: JSON.stringify(trace) === JSON.stringify(hook === 'symbol' ? ['number'] : hook === 'valueOf' ? ['valueOf'] : ['valueOf', 'toString'])}, trace);
        });
        run(label + '/author-number-proxy', () => {
          const trace = [];
          const value = new Proxy({}, {get(_, key) {trace.push(String(key)); return key === Symbol.toPrimitive ? hint => {trace.push(hint); return 0;} : undefined;}});
          const result = outcome(() => method.call(receiver, value));
          record(label + '/author-number-proxy', {converted: JSON.stringify(trace) === JSON.stringify(['Symbol(Symbol.toPrimitive)', 'number']),
            result: name === 'hasPointerCapture' ? !result.error && result.value === false : (kind === 'windowless' || kind === 'native-proxy') ? !result.error && result.value === undefined : result.error?.name === 'NotFoundError'}, trace);
        });
      }
      const revoked = Proxy.revocable(element, {}); revoked.revoke();
      const invalid = [['plain', {}], ['prototype', Object.create(window.Element.prototype)],
        ['inherited-native', Object.create(element)], ['author-proxy', new Proxy(element, {})],
        ['revoked', revoked.proxy], ['text', document.createTextNode('x')], ['document', document],
        ['window', window], ['null', null], ['undefined', undefined], ['number', 1]];
      for (const [kind, receiver] of invalid) run(`${prefix}/${owner}/invalid-${kind}`, () => {
        let conversions = 0, traps = 0;
        const value = {[Symbol.toPrimitive]() {conversions++; return 0;}};
        const result = outcome(() => method.call(receiver, value));
        const missing = outcome(() => method.call(receiver));
        const proxy = new Proxy(element, {get() {traps++;}, getPrototypeOf() {traps++;}});
        const proxyResult = outcome(() => method.call(proxy, value));
        record(`${prefix}/${owner}/invalid-${kind}`, {typeError: result.error instanceof realm.TypeError,
          missing: missing.error instanceof realm.TypeError, conversion: conversions === 0,
          proxy: proxyResult.error instanceof realm.TypeError, traps: traps === 0});
      });
      element.remove();
    }
  }
  const checks = rows.flatMap(row => Object.values(row.checks));
  return {complete: errors.length === 0 && checks.every(value => value === true),
    passed: checks.filter(value => value === true).length, total: checks.length, rows, errors};
};

globalThis.__installPointerCaptureStateProbe = (first, second, realm = globalThis) => {
  const rows = [], events = [], errors = [], state = {phase: 0, pointerId: null};
  const set = realm.Element.prototype.setPointerCapture, release = realm.Element.prototype.releasePointerCapture;
  const has = realm.Element.prototype.hasPointerCapture;
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const snapshot = id => [has.call(first, id), has.call(second, id)];
  const protect = (label, work) => {try {work();} catch(error) {errors.push({label, error: String(error.stack || error)});}};
  const failureChecks = id => {
    for (const [name, method] of [['set', set], ['release', release], ['has', has]]) {
      let missing; try {method.call(first);} catch(error) {missing = error;}
      record(name + '/required-active', {typeError: missing instanceof realm.TypeError, state: has.call(first, id)});
      for (const [n, value] of [Symbol('id'), 1n].entries()) {
        let error; try {method.call(first, value);} catch(e) {error = e;}
        record(name + '/type-active-' + n, {typeError: error instanceof realm.TypeError, state: has.call(first, id)});
      }
      const sentinel = {}; let error, conversions = 0;
      try {method.call(first, {[Symbol.toPrimitive](hint) {conversions++; if(hint !== 'number') throw new Error('hint'); throw sentinel;}});} catch(e) {error = e;}
      record(name + '/throw-active', {identity: error === sentinel, conversions: conversions === 1, state: has.call(first, id)});
    }
  };
  for (const [label, element] of [['first', first], ['second', second]]) {
    for (const type of ['pointerdown', 'gotpointercapture', 'pointermove', 'pointerup', 'lostpointercapture']) {
      element.addEventListener(type, event => events.push(`${type}@${label}`));
    }
  }
  first.addEventListener('pointerdown', event => protect('down', () => {
    state.pointerId = event.pointerId;
    set.call(first, event.pointerId + 2 ** 32);
    record('set/long-wrap', {first: has.call(first, event.pointerId), second: !has.call(second, event.pointerId)});
    let conversions = 0;
    const value = {[Symbol.toPrimitive](hint) {conversions++; return String(event.pointerId + 2 ** 32 + 0.75);}};
    record('has/long-wrap', {has: has.call(first, value), conversions: conversions === 1});
    failureChecks(event.pointerId);
    state.phase = 1;
  }));
  first.addEventListener('pointermove', event => protect('switch', () => {
    if (state.phase !== 1) return;
    state.phase = 2;
    const id = event.pointerId;
    record('before-switch', {first: has.call(first, id), second: !has.call(second, id)});
    set.call(second, id);
    const after = snapshot(id);
    record('pending-switch', {old: after[0] === false, next: after[1] === true}, after);
    release.call(first, id);
    record('release-other', {old: !has.call(first, id), next: has.call(second, id)}, snapshot(id));
  }));
  second.addEventListener('pointermove', event => protect('release', () => {
    if (state.phase !== 2) return;
    state.phase = 3;
    release.call(second, event.pointerId + 2 ** 32 + 0.5);
    const after = snapshot(event.pointerId);
    record('pending-release', {old: after[0] === false, next: after[1] === false}, after);
  }));
  first.addEventListener('pointerup', event => protect('up', () => {
    record('finished', {phase: state.phase === 3, first: !has.call(first, event.pointerId), second: !has.call(second, event.pointerId)});
  }));
  return {rows, events, errors, state};
};
