(() => {
  const rows = [], errors = [];
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const names = ['registerProtocolHandler', 'unregisterProtocolHandler'];
  try {
    const child = document.getElementById('child').contentWindow;
    const realms = [window, child];
    const cases = [
      ['relative', ['web+sample', '%s'], null],
      ['case-fold', ['WeB+SaMpLe', './path/%s'], null],
      ['safe-case-fold', ['MAILTO', '?url=%s'], null],
      ['surrogate-url', ['web+sample', 'path\uD800?url=%s'], null],
      ['two-placeholders', ['tel', '%s/foo/%s'], null],
      ['credentials', ['tel', location.origin.replace('://', '://user:pass@') + '/%s'], null],
      ['unknown-scheme', ['http', '%s'], 'SecurityError'],
      ['scheme-colon', ['mailto:', '%s'], 'SecurityError'],
      ['empty-custom', ['web+', '%s'], 'SecurityError'],
      ['custom-digit', ['web+abc1', '%s'], 'SecurityError'],
      ['unicode-case', ['web+\u212Aelvin', '%s'], 'SecurityError'],
      ['custom-surrogate', ['web+\uD800', '%s'], 'SecurityError'],
      ['null-scheme', [null, '%s'], 'SecurityError'],
      ['missing-token', ['tel', 'path'], 'SyntaxError'],
      ['upper-token', ['tel', '%S'], 'SyntaxError'],
      ['null-url', ['tel', null], 'SyntaxError'],
      ['undefined-url', ['tel', undefined], 'SyntaxError'],
      ['host-token', ['tel', 'https://%s.invalid/'], 'SyntaxError'],
      ['port-token', ['tel', 'https://example.test:%s/'], 'SyntaxError'],
      ['other-origin', ['tel', 'https://elsewhere.test/%s'], 'SecurityError'],
      ['non-http-url', ['tel', 'mailto:%s'], 'SecurityError'],
      ['scheme-before-url-syntax', ['http', 'https://test:test/'], 'SecurityError'],
      ['scheme-before-url-origin', ['http', 'https://elsewhere.test/%s'], 'SecurityError'],
      ['scheme-symbol', [Symbol('scheme'), '%s'], 'TypeError'],
      ['url-symbol-before-scheme-validation', ['http', Symbol('url')], 'TypeError'],
      ['missing-arguments', [], 'TypeError'],
      ['missing-url', ['tel'], 'TypeError'],
    ];
    function invoke(fn, receiver, args, realm, expected) {
      if (typeof fn !== 'function') return {checks: {present: false, result: false, realm: false}, observed: 'missing'};
      try {
        const result = fn.call(receiver, ...args);
        return {checks: {present: true, result: expected === null && result === undefined, realm: expected === null}, observed: 'returned'};
      } catch (error) {
        return {checks: {present: true, result: error.name === expected,
          realm: expected === 'TypeError' ? error instanceof realm.TypeError : error instanceof realm.DOMException}, observed: error.name};
      }
    }
    for (const [r, realm] of realms.entries()) {
      for (const name of names) {
        const descriptor = Object.getOwnPropertyDescriptor(realm.Navigator.prototype, name);
        record(`${r}/${name}/exposure`, realm.isSecureContext ? {
          present: !!descriptor, ownPrototype: typeof descriptor?.value === 'function',
          metadata: descriptor?.value.name === name && descriptor?.value.length === 2,
          flags: descriptor?.writable === true && descriptor?.enumerable === true && descriptor?.configurable === true,
          inherited: !Object.hasOwn(realm.navigator, name),
        } : {absentPrototype: !descriptor, absentInstance: !(name in realm.navigator)});
      }
    }
    if (isSecureContext) {
      for (const [f, realm] of realms.entries()) for (const [r, receiverRealm] of realms.entries()) {
        for (const name of names) for (const [label, args, expected] of cases) {
          const result = invoke(realm.navigator[name], receiverRealm.navigator, args, realm, expected);
          record(`${f}/${r}/${name}/${label}`, result.checks, result.observed);
        }
      }
      for (const [f, realm] of realms.entries()) for (const name of names) {
        const fn = realm.navigator[name];
        for (const [r, real] of realms.entries()) {
          const revoked = Proxy.revocable(real.navigator, {}); revoked.revoke();
          const receivers = [null, undefined, {}, real.Navigator.prototype, Object.create(real.navigator), new Proxy(real.navigator, {}), revoked.proxy];
          for (const [i, receiver] of receivers.entries()) {
            let conversions = 0, traps = 0;
            const tracked = i === 5 ? new Proxy(real.navigator, {get() {traps++; throw new Error('author trap');}}) : receiver;
            const value = {toString() {conversions++; return '%s';}};
            const result = invoke(fn, tracked, [value, value], realm, 'TypeError');
            record(`${f}/${r}/${name}/receiver-${i}`, {...result.checks, noConversions: conversions === 0, noTraps: traps === 0}, result.observed);
          }
        }
        for (const behavior of ['scheme-throws', 'url-throws', 'invalid-scheme-converts-url', 'missing-url-no-conversion', 'extra-argument-ignored']) {
          const order = [], sentinel = {};
          const scheme = {toString() {order.push('scheme'); if (behavior === 'scheme-throws') throw sentinel; return behavior === 'invalid-scheme-converts-url' ? 'http' : 'tel';}};
          const url = {toString() {order.push('url'); if (behavior === 'url-throws') throw sentinel; return '%s';}};
          const ignored = {toString() {order.push('extra'); throw sentinel;}};
          const args = behavior === 'missing-url-no-conversion' ? [scheme] : [scheme, url, ignored];
          let caught, returned = false;
          if (typeof fn === 'function') {try {returned = fn.call(realm.navigator, ...args) === undefined;} catch (error) {caught = error;}}
          const expectedOrder = behavior === 'scheme-throws' ? ['scheme'] : behavior === 'missing-url-no-conversion' ? [] : ['scheme', 'url'];
          const result = behavior.endsWith('-throws') ? caught === sentinel : behavior === 'missing-url-no-conversion' ? caught instanceof realm.TypeError : behavior === 'invalid-scheme-converts-url' ? caught?.name === 'SecurityError' && caught instanceof realm.DOMException : returned;
          record(`${f}/${name}/${behavior}`, {present: typeof fn === 'function', result, order: JSON.stringify(order) === JSON.stringify(expectedOrder)}, order);
        }
      }
      // A foreign base belongs to the receiver Document, independently of the callee realm.
      const base = child.document.createElement('base'); base.href = 'https://elsewhere.test/'; child.document.head.appendChild(base);
      const descriptor = Object.getOwnPropertyDescriptor(child.document, 'baseURI');
      Object.defineProperty(child.document, 'baseURI', {configurable: true, get() {throw new Error('public baseURI');}});
      try {
        for (const [f, realm] of realms.entries()) for (const [r, receiverRealm] of realms.entries()) for (const name of names) {
          const result = invoke(realm.navigator[name], receiverRealm.navigator, ['web+sample', '%s'], realm, r ? 'SecurityError' : null);
          record(`${f}/${r}/${name}/receiver-base`, result.checks, result.observed);
        }
      } finally {
        base.remove(); if (descriptor) Object.defineProperty(child.document, 'baseURI', descriptor); else delete child.document.baseURI;
      }
    }
  } catch (error) {errors.push(String(error.stack || error));}
  globalThis.__uiEventResults = {rows, errors};
  return errors.length === 0 && rows.every(row => Object.values(row.checks).every(value => value === true));
})()
