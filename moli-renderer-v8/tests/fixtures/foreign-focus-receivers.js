(() => {
  const failures = [];
  const namespaces = ['http://www.w3.org/1999/xhtml', 'http://www.w3.org/2000/svg', 'http://www.w3.org/1998/Math/MathML'];
  const tags = ['div', 'rect', 'mrow'];
  const names = ['HTMLElement', 'SVGElement', 'MathMLElement'];
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const frame = body.appendChild(document.createElement('iframe'));
  const child = frame.contentWindow;
  let reads = 0;
  let traps = 0;
  const options = {get preventScroll() { reads++; return true; }};
  for (const realm of [window, child]) {
    for (let owner = 0; owner < names.length; ++owner) {
      const proto = realm[names[owner]].prototype;
      const real = document.createElementNS(namespaces[owner], tags[owner]);
      const revoked = Proxy.revocable(real, {});
      revoked.revoke();
      const receivers = [null, {}, proto, Object.create(proto), Object.create(real),
        new Proxy(real, {get() { traps++; throw new Error('proxy trap'); }}), revoked.proxy];
      for (let other = 0; other < names.length; ++other) {
        if (other !== owner) receivers.push(document.createElementNS(namespaces[other], tags[other]));
      }
      for (const name of ['focus', 'blur']) {
        const descriptor = Object.getOwnPropertyDescriptor(proto, name);
        if (!descriptor || typeof descriptor.value !== 'function') {
          failures.push(`${names[owner]}.${name} missing`);
          continue;
        }
        const method = descriptor.value;
        if (method.length !== 0 || !descriptor.enumerable || !descriptor.writable || !descriptor.configurable) {
          failures.push(`${names[owner]}.${name} descriptor`);
        }
        for (const receiver of receivers) {
          const before = reads;
          try { Reflect.apply(method, receiver, [options]); failures.push(`${names[owner]}.${name} accepted invalid receiver`); }
          catch (error) {
            if (!(error instanceof realm.TypeError)) failures.push(`${names[owner]}.${name} wrong error realm`);
          }
          if (reads !== before) failures.push(`${names[owner]}.${name} converted before receiver check`);
        }
        for (const documentInRealm of [document, child.document]) {
          const valid = documentInRealm.createElementNS(namespaces[owner], tags[owner]);
          Object.setPrototypeOf(valid, null);
          const before = reads;
          try { Reflect.apply(method, valid, [options]); }
          catch (error) { failures.push(`${names[owner]}.${name} rejected genuine object: ${error.name}`); }
          if (reads !== before + (name === 'focus' ? 1 : 0)) failures.push(`${names[owner]}.${name} options reads`);
        }
        if (name === 'focus') {
          const sentinel = {};
          try { method.call(real, {get preventScroll() { throw sentinel; }}); failures.push('swallowed options exception'); }
          catch (error) { if (error !== sentinel) failures.push('replaced options exception'); }
        }
      }
    }
  }
  if (traps !== 0) failures.push(`receiver validation invoked ${traps} proxy traps`);
  frame.remove();
  return failures;
})()
