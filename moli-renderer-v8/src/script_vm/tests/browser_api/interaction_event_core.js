(realms) => {
  const rows = [], errors = [];
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const attempt = (label, run) => { try { run(); } catch (error) { errors.push({label, error: String(error.stack || error)}); } };
  const units = text => Array.from({length: text.length}, (_, i) => text.charCodeAt(i));
  for (const [r, realm] of realms.entries()) {
    const doc = realm.document;
    for (const name of ['ToggleEvent', 'CommandEvent']) {
      const Ctor = realm[name], prefix = `${r}/${name}`;
      const textFields = name === 'ToggleEvent' ? ['oldState', 'newState'] : ['command'];
      const fields = [...textFields, 'source'];
      const keys = ['bubbles', 'cancelable', 'composed', ...(name === 'ToggleEvent' ? ['newState', 'oldState'] : ['command']), 'source'];
      attempt(prefix + '/metadata', () => {
        const event = new Ctor('interaction');
        record(prefix + '/constructor', {name: Ctor.name === name, length: Ctor.length === 1, parent: Object.getPrototypeOf(Ctor.prototype) === realm.Event.prototype});
        for (const field of fields) {
          const d = Object.getOwnPropertyDescriptor(Ctor.prototype, field);
          record(prefix + '/descriptor-' + field, {getter: typeof d?.get === 'function', name: d?.get?.name === 'get ' + field, length: d?.get?.length === 0, enumerable: d?.enumerable === true, configurable: d?.configurable === true, setter: d?.set === undefined, own: !Object.hasOwn(event, field)});
        }
      });
      for (const [i, init] of [undefined, null, {}, [], function() {}, new Date(0)].entries()) attempt(prefix + '/default-' + i, () => {
        const e = new Ctor('x', init);
        record(prefix + '/default-' + i, {texts: textFields.every(f => e[f] === ''), source: e.source === null, bubbles: e.bubbles === false, cancelable: e.cancelable === false, composed: e.composed === false});
      });
      for (const [i, value] of [1, 'init', false, Symbol('init'), 1n].entries()) attempt(prefix + '/dictionary-' + i, () => {
        let error; try { new Ctor('x', value); } catch (e) { error = e; }
        record(prefix + '/dictionary-' + i, {typeError: error instanceof realm.TypeError}, error?.name || 'returned');
      });
      const strings = [undefined, null, '', 'plain', '\uD800', '\uDC00', '\uD83D\uDE80', 'x\uD800y', '\0', true, false, -3, 3.5, NaN, Infinity, 1n, ['a', 'b']];
      for (const field of textFields) {
        for (const [i, value] of strings.entries()) attempt(prefix + '/string-' + field + '-' + i, () => {
          const expected = value === undefined ? '' : String(value), actual = new Ctor('x', {[field]: value})[field];
          record(prefix + '/string-' + field + '-' + i, {converted: actual === expected}, units(actual));
        });
        attempt(prefix + '/coercion-' + field, () => {
          const trace = [], value = {[Symbol.toPrimitive](hint) { trace.push(hint); return '\uD800'; }};
          const actual = new Ctor('x', {[field]: value})[field];
          record(prefix + '/coercion-' + field, {value: actual === '\uD800', hint: trace.length === 1 && trace[0] === 'string'}, trace);
          let error; try { new Ctor('x', {[field]: Symbol('text')}); } catch (e) { error = e; }
          record(prefix + '/symbol-' + field, {typeError: error instanceof realm.TypeError});
        });
      }
      attempt(prefix + '/order', () => {
        const trace = [];
        new Ctor('x', new Proxy({}, {get(_, key) { trace.push(key); }}));
        record(prefix + '/order', {order: JSON.stringify(trace) === JSON.stringify(keys)}, trace);
        for (const [i, key] of keys.entries()) {
          const sentinel = {}, observed = [];
          let error;
          try { new Ctor('x', new Proxy({}, {get(_, k) { observed.push(k); if (k === key) throw sentinel; }})); } catch (e) { error = e; }
          record(prefix + '/throw-get-' + key, {identity: error === sentinel, order: JSON.stringify(observed) === JSON.stringify(keys.slice(0, i + 1))}, observed);
        }
        for (const field of textFields) {
          const sentinel = {}, observed = [];
          let error;
          try { new Ctor('x', new Proxy({}, {get(_, key) { observed.push(key); return key === field ? {[Symbol.toPrimitive]() { throw sentinel; }} : undefined; }})); } catch (e) { error = e; }
          record(prefix + '/throw-convert-' + field, {identity: error === sentinel, order: JSON.stringify(observed) === JSON.stringify(keys.slice(0, keys.indexOf(field) + 1))}, observed);
        }
        const init = Object.create(Object.fromEntries(textFields.map(f => [f, '\uD800'])));
        init.bubbles = 1; init.cancelable = {}; init.composed = 'yes';
        const event = new Ctor('x', init);
        record(prefix + '/inherited', {texts: textFields.every(f => event[f] === '\uD800'), flags: event.bubbles && event.cancelable && event.composed});
      });
      attempt(prefix + '/construct', () => {
        let call, missing; try { Ctor('x'); } catch (e) { call = e; } try { new Ctor(); } catch (e) { missing = e; }
        record(prefix + '/required', {call: call instanceof realm.TypeError, missing: missing instanceof realm.TypeError});
        const trace = [], type = {[Symbol.toPrimitive](hint) { trace.push('type:' + hint); return '\uD800'; }};
        const e = new Ctor(type, new Proxy({}, {get(_, key) { trace.push(key); }}));
        record(prefix + '/type-first', {type: e.type === '\uD800', order: JSON.stringify(trace) === JSON.stringify(['type:string', ...keys])}, trace);
        const sentinel = {}, observed = []; let error;
        try { new Ctor({toString() { throw sentinel; }}, new Proxy({}, {get(_, key) { observed.push(key); }})); } catch (e) { error = e; }
        record(prefix + '/type-throw', {identity: error === sentinel, laterReads: observed.length === 0});
        class Derived extends Ctor {}
        const derived = new Derived('x', Object.fromEntries(textFields.map(f => [f, '\uD800'])));
        record(prefix + '/subclass', {identity: derived instanceof Derived && derived instanceof Ctor, payload: textFields.every(f => derived[f] === '\uD800')});
      });
      attempt(prefix + '/sources', () => {
        const foreign = realms[(r + 1) % realms.length].document.createElement('b');
        const detached = doc.implementation.createHTMLDocument('').createElement('select');
        const real = doc.createElement('div');
        for (const [i, value] of [null, undefined, real, foreign, doc.createElementNS('http://www.w3.org/2000/svg', 'svg'), detached].entries()) {
          const event = new Ctor('x', {source: value});
          record(prefix + '/valid-source-' + i, {identity: event.source === (value ?? null)});
        }
        const revoked = Proxy.revocable(real, {}); revoked.revoke();
        const invalid = [1, false, '', 1n, Symbol('source'), {}, doc, doc.createTextNode('x'), doc.createAttribute('x'), realm, new realm.EventTarget(), Object.create(realm.Element.prototype), Object.create(real), new Proxy(real, {}), revoked.proxy];
        for (const [i, value] of invalid.entries()) {
          let error; try { new Ctor('x', {source: value}); } catch (e) { error = e; }
          record(prefix + '/invalid-source-' + i, {typeError: error instanceof realm.TypeError}, error?.name || 'returned');
        }
        let traps = 0;
        const proxy = new Proxy(real, {get() { traps++; throw 'get'; }, getPrototypeOf() { traps++; throw 'prototype'; }});
        let error; try { new Ctor('x', {source: proxy}); } catch (e) { error = e; }
        record(prefix + '/source-proxy-traps', {typeError: error instanceof realm.TypeError, traps: traps === 0});
      });
      for (const field of fields) attempt(prefix + '/receiver-' + field, () => {
        const getter = Object.getOwnPropertyDescriptor(Ctor.prototype, field)?.get;
        if (!getter) { record(prefix + '/receiver-' + field, {getter: false}); return; }
        const genuine = new Ctor('x');
        const revoked = Proxy.revocable(genuine, {}); revoked.revoke();
        for (const [i, value] of [null, undefined, {}, Object.create(Ctor.prototype), Object.create(genuine), new realm.Event('x'), new Proxy(genuine, {}), revoked.proxy].entries()) {
          let error; try { getter.call(value); } catch (e) { error = e; }
          record(prefix + '/receiver-' + field + '-' + i, {typeError: error instanceof realm.TypeError});
        }
        for (const [j, other] of realms.entries()) {
          const event = new other[name]('x', Object.fromEntries(textFields.map(f => [f, '\uD800'])));
          record(prefix + '/foreign-getter-' + field + '-' + j, {value: getter.call(event) === (field === 'source' ? null : '\uD800')});
        }
      });
      for (const mode of ['open', 'closed']) attempt(prefix + '/retarget-' + mode, () => {
        const outer = doc.body.appendChild(doc.createElement('div')), outerRoot = outer.attachShadow({mode});
        const inner = outerRoot.appendChild(doc.createElement('div')), innerRoot = inner.attachShadow({mode});
        const source = innerRoot.appendChild(doc.createElement('span')), inside = innerRoot.appendChild(doc.createElement('b'));
        const middle = outerRoot.appendChild(doc.createElement('p')), outside = doc.body.appendChild(doc.createElement('p'));
        const tag = value => value === source ? 'source' : value === inner ? 'inner' : value === outer ? 'outer' : value === null ? 'null' : 'other';
        const event = new Ctor('interaction', {source, bubbles: true, composed: true});
        record(prefix + '/' + mode + '/before', {outer: event.source === outer}, tag(event.source));
        for (const [i, target, expected] of [[0, outside, outer], [1, inside, source], [2, middle, inner], [3, doc, outer], [4, realm, outer], [5, new realm.EventTarget(), outer], [6, innerRoot, source], [7, outerRoot, inner]]) {
          let during;
          target.addEventListener('interaction', e => { during = e.source; }, {once: true});
          target.dispatchEvent(event);
          record(prefix + '/' + mode + '/redispatch-' + i, {during: during === expected, after: event.source === outer}, {during: tag(during), after: tag(event.source)});
        }
        const seen = [];
        for (const [target, expected] of [[inside, source], [innerRoot, source], [inner, inner], [outerRoot, inner], [outer, outer], [doc, outer], [realm, outer]]) {
          for (const capture of [true, false]) target.addEventListener('phases', e => { seen.push(e.source === expected); }, {capture, once: true});
        }
        inside.dispatchEvent(new Ctor('phases', {source, bubbles: true, composed: true}));
        record(prefix + '/' + mode + '/phases', {count: seen.length === 14, values: seen.every(Boolean)}, seen);
        outside.appendChild(source);
        record(prefix + '/' + mode + '/move-out', {source: event.source === source}, tag(event.source));
        innerRoot.appendChild(source);
        record(prefix + '/' + mode + '/move-back', {outer: event.source === outer}, tag(event.source));
        let beforeMove, afterMove, afterRestore;
        outside.addEventListener('mutation', e => { beforeMove = e.source; outside.appendChild(source); afterMove = e.source; innerRoot.appendChild(source); afterRestore = e.source; }, {once: true});
        const moved = new Ctor('mutation', {source}); outside.dispatchEvent(moved);
        record(prefix + '/' + mode + '/during-move', {before: beforeMove === outer, after: afterMove === source, restored: afterRestore === outer}, [beforeMove, afterMove, afterRestore].map(tag));
        const originalGetter = Object.getOwnPropertyDescriptor(Ctor.prototype, 'source').get;
        Object.defineProperty(event, 'currentTarget', {get() { throw 'author currentTarget'; }});
        Object.defineProperty(source, 'getRootNode', {value() { throw 'author getRootNode'; }});
        Object.defineProperty(source, 'parentNode', {get() { throw 'author parentNode'; }});
        record(prefix + '/' + mode + '/private-state', {native: originalGetter.call(event) === outer});
        const light = outer.appendChild(doc.createElement('i')); outerRoot.appendChild(doc.createElement('slot'));
        const slotted = new Ctor('slotted', {source: light}); let slottedSource;
        inside.addEventListener('slotted', e => { slottedSource = e.source; }, {once: true}); inside.dispatchEvent(slotted);
        record(prefix + '/' + mode + '/slotted-light', {before: slotted.source === light, during: slottedSource === light});
        outer.remove(); outside.remove();
      });
    }
  }
  return {rows, errors};
}
