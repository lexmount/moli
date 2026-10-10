(() => {
  const checks = [];
  function check(name, predicate) {
    try { checks.push({name, passed: predicate() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  }
  const other = document.querySelector('iframe').contentWindow;
  for (const [label, realm] of [['main', window], ['iframe', other]]) {
    for (const name of ['RTCEncodedAudioFrame', 'RTCEncodedVideoFrame']) {
      const C = realm[name];
      check(`${label} ${name} constructor length`, () => C.length === 1);
      for (const [index, value] of [undefined, null, false, 1, 1n, '', Symbol(), {}, Object.create(C.prototype), Object.create(null), new Proxy({}, {})].entries()) {
        let reads = 0, error;
        const options = {get metadata() {reads++; throw Error('options were read');}};
        try { new C(value, options); } catch (caught) { error = caught; }
        check(`${label} ${name} invalid original ${index}`, () => error instanceof realm.TypeError && reads === 0);
      }
      let error;
      try { new C(); } catch (caught) { error = caught; }
      check(`${label} ${name} required original`, () => error instanceof realm.TypeError);
      error = undefined;
      try { C({}); } catch (caught) { error = caught; }
      check(`${label} ${name} requires new`, () => error instanceof realm.TypeError);
      const members = name === 'RTCEncodedVideoFrame' ? ['type', 'data', 'getMetadata'] : ['data', 'getMetadata'];
      for (const key of members) {
        const d = realm.Object.getOwnPropertyDescriptor(C.prototype, key);
        check(`${label} ${name}.${key} descriptor`, () => !!d && d.enumerable && d.configurable &&
          (key === 'getMetadata' ? typeof d.value === 'function' && d.value.length === 0 && d.writable : typeof d.get === 'function' && d.get.length === 0 &&
          (key === 'data' ? typeof d.set === 'function' && d.set.length === 1 : d.set === undefined)));
        const trap = () => { throw Error('author trap ran'); };
        const revoked = Proxy.revocable({}, {}); revoked.revoke();
        for (const [index, receiver] of [{}, Object.create(C.prototype), new Proxy({}, {get: trap, getPrototypeOf: trap}), revoked.proxy, C.prototype].entries()) {
          for (const [role, callback] of key === 'getMetadata' ? [['method', d?.value]] : key === 'data' ? [['get', d?.get], ['set', d?.set]] : [['get', d?.get]]) {
            let error;
            try { callback.call(receiver, {get buffer() {throw Error('conversion ran');}}); } catch (caught) { error = caught; }
            check(`${label} ${name}.${key} ${role} receiver ${index}`, () => typeof callback === 'function' && error instanceof realm.TypeError);
          }
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(x => x.passed).length, checks};
  return checks.every(x => x.passed);
})()
