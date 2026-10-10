(() => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const interfaces = [["ReportingObserver", null, 1]];
  for (const realm of [window, document.getElementById('child').contentWindow]) {
    for (const [name, parentName, length] of interfaces) {
      const constructor = realm[name], parent = realm[parentName ?? 'Object'];
      assert(typeof constructor === 'function' && constructor.name === name && constructor.length === length,
        name + ' interface object');
      const descriptor = Object.getOwnPropertyDescriptor(realm, name);
      assert(descriptor.writable && descriptor.configurable && !descriptor.enumerable, name + ' global descriptor');
      assert(Object.getPrototypeOf(constructor.prototype) === parent.prototype, name + ' prototype inheritance');
      assert(Object.getPrototypeOf(constructor) === (parentName ? parent : realm.Function.prototype), name + ' interface inheritance');
      assert(constructor.prototype.constructor === constructor, name + ' prototype constructor');
      const tag = Object.getOwnPropertyDescriptor(constructor.prototype, Symbol.toStringTag);
      assert(tag.value === name && !tag.writable && !tag.enumerable && tag.configurable, name + ' prototype tag');
      let callError;
      try { constructor(); } catch (error) { callError = error; }
      assert(callError instanceof realm.TypeError, name + ' call requires new in the callee realm');
      if (realm !== window) assert(!(callError instanceof TypeError), name + ' foreign TypeError');
      for (const invalid of [undefined, null, {}, 1]) {
        let error;
        try { new constructor(invalid); } catch (caught) { error = caught; }
        assert(error instanceof realm.TypeError, name + ' requires a callable callback');
      }
      let calls = 0;
      const callback = new Proxy(() => calls++, {apply() { calls++; }});
      const reads = [];
      const observer = new constructor(callback, {
        get buffered() { reads.push('buffered'); return true; },
        get types() { reads.push('types'); return ['deprecation', '\ud800']; }
      });
      assert(reads.join(',') === 'buffered,types', 'dictionary conversion order');
      assert(observer instanceof constructor && Object.prototype.toString.call(observer) === '[object ReportingObserver]', 'native observer identity');
      for (const method of ['observe', 'disconnect', 'takeRecords']) {
        const descriptor = Object.getOwnPropertyDescriptor(constructor.prototype, method);
        assert(descriptor.value.length === 0 && descriptor.enumerable && descriptor.writable && descriptor.configurable, method + ' descriptor');
        for (const forged of [{}, Object.create(observer), new Proxy(observer, {})]) {
          let error;
          try { descriptor.value.call(forged); } catch (caught) { error = caught; }
          assert(error instanceof realm.TypeError, method + ' rejects forged receivers in its realm');
        }
      }
      observer.disconnect(); observer.observe(); observer.observe();
      const first = observer.takeRecords();
      first.push('author record');
      const second = observer.takeRecords();
      assert(first !== second && second instanceof realm.Array && second.length === 0, 'fresh native empty record queue');
      observer.disconnect(); observer.disconnect(); observer.observe(); observer.disconnect();
      assert(calls === 0, 'empty reporting backend never invents reports');
      const sentinel = {};
      let caught;
      try { new constructor(() => {}, { get buffered() { throw sentinel; } }); } catch (error) { caught = error; }
      assert(caught === sentinel, 'dictionary getter exception is preserved');
      let optionsRead = false;
      try { new constructor({}, {get buffered() { optionsRead = true; }}); } catch (_) {}
      assert(!optionsRead, 'callback validation precedes options');
      assert(realm[name] === constructor, name + ' materialization remains stable');
    }
  }
  return 'ok';
})()
