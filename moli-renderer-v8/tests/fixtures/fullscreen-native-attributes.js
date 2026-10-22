(() => {
  const rows = [], jobs = [];
  let traps = 0;
  const check = (name, body) => {
    const index = rows.length;
    rows.push({name, passed: false});
    jobs.push(Promise.resolve().then(body).then(value => {
      if (!value) throw Error('false');
      rows[index] = {name, passed: true};
    }).catch(error => {
      rows[index] = {name, passed: false, error: String(error)};
    }));
  };
  const other = document.querySelector('#child').contentWindow;
  for (const [ci, callee] of [globalThis, other].entries()) {
    for (const [oi, owner] of [globalThis, other].entries()) {
      for (const flavor of ['live', 'html', 'xml', 'parsed', 'new']) {
        const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
          owner.document.implementation.createHTMLDocument('fullscreen') : flavor === 'xml' ?
          owner.document.implementation.createDocument(null, 'r') : flavor === 'new' ?
          new owner.Document() : new owner.DOMParser().parseFromString('<r/>', 'application/xml');
        const element = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
        const root = element.attachShadow({mode: 'open'});
        const prefix = `${ci}/${oi}/${flavor}`;
        const typeError = error => Object.getPrototypeOf(error) === callee.TypeError.prototype;
        const poisonHandler = {
          get() { traps++; throw Error('get'); },
          getPrototypeOf() { traps++; throw Error('getPrototypeOf'); }
        };
        for (const [kind, target, members] of [
          ['Document', doc, ['fullscreen', 'fullscreenElement', 'fullscreenEnabled']],
          ['ShadowRoot', root, ['fullscreenElement']]
        ]) {
          const proto = callee[kind].prototype;
          const revoked = Proxy.revocable(target, poisonHandler);
          revoked.revoke();
          const invalid = [null, undefined, {}, Object.create(proto), Object.create(target),
            new Proxy(target, poisonHandler), revoked.proxy, element];
          for (const member of members) {
            const descriptor = Object.getOwnPropertyDescriptor(proto, member);
            check(`${prefix}/${kind}/${member}/shape`, () => descriptor &&
              descriptor.enumerable && descriptor.configurable &&
              descriptor.get.length === 0 && descriptor.set.length === 1);
            check(`${prefix}/${kind}/${member}/value`, () => {
              const value = descriptor.get.call(target);
              return member === 'fullscreenEnabled' ? typeof value === 'boolean' :
                value === (member === 'fullscreen' ? false : null);
            });
            check(`${prefix}/${kind}/${member}/lenient`, () => {
              let converted = 0;
              const before = descriptor.get.call(target);
              const value = {
                valueOf() { converted++; throw Error('value'); },
                toString() { converted++; throw Error('string'); }
              };
              return descriptor.set.call(target, value) === undefined &&
                descriptor.set.call(target, Symbol()) === undefined &&
                descriptor.set.call(target) === undefined &&
                descriptor.get.call(target) === before && converted === 0;
            });
            for (const [ri, receiver] of invalid.entries()) {
              check(`${prefix}/${kind}/${member}/receiver/${ri}`, () => {
                // A missing descriptor must not be mistaken for a receiver rejection.
                if (!descriptor) return false;
                let count = 0, getError, setError;
                try { descriptor.get.call(receiver); } catch (error) { getError = error; }
                try {
                  descriptor.set.call(receiver, {toString() { count++; throw Error('conversion'); }});
                } catch (error) { setError = error; }
                return getError && setError && typeError(getError) && typeError(setError) &&
                  count === 0 && traps === 0;
              });
            }
          }
        }
      }
    }
  }
  return Promise.all(jobs).then(() => {
    rows.push({name: 'no-proxy-traps', passed: traps === 0});
    globalThis.__fullscreenAttributeResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
    return rows.every(row => row.passed);
  });
})()
