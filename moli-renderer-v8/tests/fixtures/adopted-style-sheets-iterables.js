(() => {
  const rows = [];
  const realms = [['top', globalThis], ['child', document.querySelector('#child').contentWindow]];
  function check(name, body) {
    try {rows.push({name, passed: body() === true});}
    catch (error) {rows.push({name, passed: false, error: String(error)});}
  }
  function throws(realm, name, body) {
    try {body(); return false;}
    catch (error) {return name === 'TypeError' ? Object.getPrototypeOf(error) === realm.TypeError.prototype :
      Object.getPrototypeOf(error) === realm.DOMException.prototype && error.name === name;}
  }
  for (const [calleeName, callee] of realms) for (const [ownerName, owner] of realms)
    for (const kind of ['Document', 'ShadowRoot']) {
      const doc = owner.document, host = doc.body.appendChild(doc.createElement('div'));
      const receiver = kind === 'Document' ? doc : host.attachShadow({mode: 'open'});
      const descriptor = Object.getOwnPropertyDescriptor(callee[kind].prototype, 'adoptedStyleSheets');
      const get = () => descriptor.get.call(receiver), set = value => descriptor.set.call(receiver, value);
      const a = new owner.CSSStyleSheet(), b = new owner.CSSStyleSheet();
      const label = `${calleeName} setter ${ownerName} ${kind}`;
      try {
        check(`${label} accepts a Set preserving native sheet order`, () => {
          set(new Set([a, b]));return get().length === 2 && get()[0] === a && get()[1] === b;
        });
        check(`${label} accepts a generator`, () => {
          set((function*() {yield b; yield a;})());return get().length === 2 && get()[0] === b && get()[1] === a;
        });
        check(`${label} reads an array's iterator instead of its indexed elements`, () => {
          const input = [a]; let calls = 0;
          Object.defineProperty(input, 0, {get() {throw Error('ignored array index');}});
          input[Symbol.iterator] = function*() {calls++; yield b;};
          set(input);return calls === 1 && get().length === 1 && get()[0] === b;
        });
        check(`${label} owns an array copy without installing methods on the input`, () => {
          const input = new owner.Array(a);set(input);
          input.push(b);
          return get() !== input && get().length === 1 && get()[0] === a &&
            input.push === owner.Array.prototype.push && !Object.hasOwn(input, 'push');
        });
        for (const [name, value] of [['undefined', undefined], ['null', null], ['number', 3],
          ['string', 'x'], ['array-like', {0: a, length: 1}]])
          check(`${label} rejects ${name}`, () => throws(callee, 'TypeError', () => set(value)));
        for (const part of ['iterator getter', 'iterator call', 'next', 'done', 'value'])
          check(`${label} preserves ${part} exception identity and previous state`, () => {
            set([a]); const previous = get(), sentinel = {};let calls = 0;
            const input = {get [Symbol.iterator]() {
              if (part === 'iterator getter') {calls++; throw sentinel;}
              return function() {
                if (part === 'iterator call') {calls++; throw sentinel;}
                return {next() {
                  if (part === 'next') {calls++; throw sentinel;}
                  return {get done() {if (part === 'done') {calls++; throw sentinel;}return false;},
                    get value() {calls++; throw sentinel;}};
                }};
              };
            }};
            try {set(input);return false;}
            catch (error) {return error === sentinel && calls === 1 && get() === previous && get()[0] === a;}
          });
        for (const badKind of ['object', 'prototype', 'inherited', 'proxy', 'revoked'])
          check(`${label} rejects ${badKind} sheet during conversion without traps or IteratorClose`, () => {
            let traps = 0, steps = 0, closes = 0;
            const revoked = Proxy.revocable(a, {});revoked.revoke();
            const bad = badKind === 'object' ? {} : badKind === 'prototype' ? Object.create(owner.CSSStyleSheet.prototype) :
              badKind === 'inherited' ? Object.create(a) : badKind === 'revoked' ? revoked.proxy :
              new Proxy(a, {get() {traps++;throw 1;}, getPrototypeOf() {traps++;throw 1;}});
            const input = {[Symbol.iterator]() {return {next() {steps++;return {value: bad, done: false};},
              return() {closes++;return {};}};}};
            return throws(callee, 'TypeError', () => set(input)) && traps === 0 && steps === 1 && closes === 0;
          });
        check(`${label} rejects a non-constructed sheet`, () => {
          const style = doc.head.appendChild(doc.createElement('style'));
          try {return throws(callee, 'NotAllowedError', () => set([style.sheet]));}
          finally {style.remove();}
        });
        check(`${label} rejects a constructed sheet from another document`, () => {
          const other = owner === globalThis ? realms[1][1] : globalThis;
          return throws(callee, 'NotAllowedError', () => set([new other.CSSStyleSheet()]));
        });
      } finally {set([]);host.remove();}
    }
  // The genuine-receiver conversion cases moved here from the ShadowRoot matrix.
  for (const [calleeName, callee] of realms) for (const [ownerName, owner] of realms)
    for (const flavor of ['live', 'html', 'xml']) {
      const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
        owner.document.implementation.createHTMLDocument('shadow') :
        owner.document.implementation.createDocument(null, 'r');
      const host = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
      (doc.body || doc.documentElement).appendChild(host);
      const root = host.attachShadow({mode: 'open'});
      check(`${calleeName} binding ${ownerName} ${flavor} adoptedStyleSheets setter preserves iterator exception identity`, () => {
        const sentinel = {};let conversions = 0;
        try {Object.getOwnPropertyDescriptor(callee.ShadowRoot.prototype, 'adoptedStyleSheets').set.call(root,
          {get [Symbol.iterator]() {conversions++;throw sentinel;}});return false;}
        catch (error) {return error === sentinel && conversions === 1;}
      });
      host.remove();
    }
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
