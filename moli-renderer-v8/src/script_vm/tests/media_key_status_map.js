(() => {
  const checks = [];
  const check = (name, action) => {
    try { checks.push({name, passed: action() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = window.open();
  const realms = [['main', window], ['iframe', frame.contentWindow], ['popup', popup]];
  const methods = [['has', 1], ['get', 1], ['entries', 0], ['keys', 0], ['values', 0], ['forEach', 1]];
  try {
    for (const [realm, w] of realms) {
      const C = w?.MediaKeyStatusMap, P = C?.prototype;
      const prefix = realm + ': ';
      const throws = action => {
        try { action(); } catch (error) { return error instanceof w.TypeError; }
        return false;
      };
      check(prefix + 'interface object', () => typeof C === 'function' && C.name === 'MediaKeyStatusMap' && C.length === 0);
      check(prefix + 'prototype inheritance', () => Object.getPrototypeOf(P) === w.Object.prototype);
      check(prefix + 'illegal constructor', () => throws(() => new C()));
      check(prefix + 'illegal function call', () => throws(() => C()));
      check(prefix + 'prototype tag', () => {
        const d = Object.getOwnPropertyDescriptor(P, Symbol.toStringTag);
        return d.value === 'MediaKeyStatusMap' && !d.writable && !d.enumerable && d.configurable;
      });
      const size = Object.getOwnPropertyDescriptor(P ?? {}, 'size');
      check(prefix + 'size descriptor', () => typeof size.get === 'function' && size.set === undefined && size.enumerable && size.configurable && size.get.length === 0);
      check(prefix + 'iterator alias', () => typeof P.entries === 'function' && P[Symbol.iterator] === P.entries);
      check(prefix + 'iterator descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(P, Symbol.iterator);
        return d.writable && !d.enumerable && d.configurable;
      });
      for (const [name, length] of methods) {
        check(prefix + name + ' descriptor and length', () => {
          const d = Object.getOwnPropertyDescriptor(P, name);
          return typeof d.value === 'function' && d.value.name === name && d.value.length === length && d.writable && d.enumerable && d.configurable;
        });
      }
      let traps = 0, conversions = 0;
      const argument = new Proxy({}, {get() {traps++; throw Error('argument trap');}});
      const fake = Object.create(P ?? null);
      const revoked = Proxy.revocable({}, {}); revoked.revoke();
      const receivers = [{}, fake, Object.create(fake), new Proxy(fake, {get() {traps++;}}), revoked.proxy, new w.EventTarget()];
      for (const [index, receiver] of receivers.entries()) {
        check(prefix + 'size rejects receiver ' + index, () => throws(() => w.Reflect.apply(size?.get, receiver, [])));
        for (const [name] of methods) {
          check(prefix + name + ' rejects receiver ' + index, () => throws(() => w.Reflect.apply(P?.[name], receiver, [argument, {valueOf() {conversions++;}}])));
        }
      }
      check(prefix + 'brand checks precede conversion and traps', () => traps === 0 && conversions === 0);
    }
  } finally {
    popup?.close(); frame.remove();
  }
  globalThis.__uiEventResults = {complete: true, checks, passed: checks.filter(row => row.passed).length, total: checks.length};
  return __uiEventResults;
})()
