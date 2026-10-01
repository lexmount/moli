(() => {
  const rows = [];
  function assert(value, message) { if (!value) throw new Error(message); }
  function check(name, callback) {
    try { callback(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  }
  function throwsTypeError(w, callback, message) {
    let error;
    try { callback(); } catch (value) { error = value; }
    assert(error instanceof w.TypeError, message + ': callee TypeError');
    if (w !== window) assert(!(error instanceof TypeError), message + ': distinct realm');
  }
  const names = [
    'onblur', 'onerror', 'onfocus', 'onload', 'onresize', 'onscroll',
    'onafterprint', 'onbeforeprint', 'onbeforeunload', 'ongamepadconnected',
    'ongamepaddisconnected', 'onhashchange', 'onlanguagechange', 'onmessage',
    'onmessageerror', 'onoffline', 'ononline', 'onpagehide', 'onpagereveal',
    'onpageshow', 'onpageswap', 'onpopstate', 'onrejectionhandled', 'onstorage',
    'onunhandledrejection', 'onunload'
  ];
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = open();
  assert(popup !== null, 'popup fixture');
  try {
    const realms = [window, frame.contentWindow, popup];
    for (const [realmIndex, w] of realms.entries()) {
      for (const tag of ['body', 'frameset']) {
        const proto = (tag === 'body' ? w.HTMLBodyElement : w.HTMLFrameSetElement).prototype;
        const otherTag = tag === 'body' ? 'frameset' : 'body';
        const real = w.document.createElement(tag);
        const detached = w.document.implementation.createHTMLDocument('').createElement(tag);
        for (const name of names) {
          const key = realmIndex + ':' + tag + ':' + name;
          const descriptor = Object.getOwnPropertyDescriptor(proto, name);
          check(key + ':descriptor', () => {
            assert(descriptor && descriptor.enumerable && descriptor.configurable, 'own accessor');
            assert(descriptor.get.length === 0 && descriptor.set.length === 1, 'accessor arity');
            assert(descriptor.get.name === 'get ' + name && descriptor.set.name === 'set ' + name, 'accessor name');
            throwsTypeError(window, () => Reflect.construct(descriptor.get, []), 'getter constructor');
            throwsTypeError(window, () => Reflect.construct(descriptor.set, [null]), 'setter constructor');
          });
          check(key + ':invalid-receivers', () => {
            let traps = 0;
            const handler = { get() { traps++; throw new Error('get trap'); },
              getPrototypeOf() { traps++; throw new Error('prototype trap'); },
              has() { traps++; throw new Error('has trap'); } };
            const revoked = Proxy.revocable(real, handler); revoked.revoke();
            const invalid = [proto, {}, Object.create(proto), Object.create(real),
              w.document.createElement(otherTag), w.document.createElement('div'),
              w.document, w, null, undefined, 7, 'body', Symbol('body'),
              new Proxy(real, handler), revoked.proxy];
            const value = new Proxy(function() {}, handler);
            const saved = w[name];
            try {
              for (const receiver of invalid) {
                throwsTypeError(w, () => descriptor.get.call(receiver), 'getter receiver');
                throwsTypeError(w, () => descriptor.set.call(receiver, value), 'setter receiver');
              }
              assert(traps === 0, 'validation executes no author traps');
              assert(w[name] === saved, 'invalid setter keeps Window handler');
            } finally { w[name] = saved; }
          });
          check(key + ':native-receivers', () => {
            const saved = w[name];
            const handler = function() {};
            const originalPrototype = Object.getPrototypeOf(real);
            try {
              assert(descriptor.set.call(real, handler) === undefined, 'setter result');
              assert(descriptor.get.call(real) === handler && w[name] === handler, 'native Window forwarding');
              Object.setPrototypeOf(real, null);
              descriptor.set.call(real, null);
              assert(descriptor.get.call(real) === null && w[name] === null, 'native identity survives prototype change');
              for (const other of realms) {
                const cross = other.document.createElement(tag);
                const otherSaved = other[name];
                try {
                  descriptor.set.call(cross, handler);
                  assert(descriptor.get.call(cross) === handler && other[name] === handler, 'cross-realm receiver owner');
                } finally { other[name] = otherSaved; }
              }
              descriptor.set.call(detached, handler);
              assert(descriptor.get.call(detached) === null, 'windowless native receiver has no target');
              assert(w[name] === null, 'windowless setter keeps live Window');
            } finally {
              Object.setPrototypeOf(real, originalPrototype);
              w[name] = saved;
            }
          });
        }
      }
      check(realmIndex + ':lenient-global-handlers', () => {
        for (const name of ['onmouseenter', 'onmouseleave']) {
          const descriptor = Object.getOwnPropertyDescriptor(w.HTMLElement.prototype, name);
          assert(descriptor.get.call({}) === undefined, 'lenient getter');
          assert(descriptor.set.call({}, function() {}) === undefined, 'lenient setter');
        }
      });
    }
    const failures = rows.filter(row => !row.passed);
    globalThis.__nodeReplacementResults = {total: rows.length, passed: rows.length - failures.length, failures, rows};
    return failures.length === 0;
  } finally {
    popup.close();
    frame.remove();
  }
})()
