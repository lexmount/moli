(async () => {
  const popup = open('about:blank', 'registry-owner');
  const rows = [];
  try {
    for (const [label, owner, borrowed] of [
      ['main', window, popup],
      ['popup', popup, window],
    ]) {
      for (const [kind, registry] of [
        ['global', owner.customElements],
        ['scoped', new owner.CustomElementRegistry()],
      ]) {
        const own = owner.CustomElementRegistry.prototype;
        const other = borrowed.CustomElementRegistry.prototype;
        const name = `registry-${label}-${kind}`;
        const constructor = class extends owner.HTMLElement {};
        const pending = own.whenDefined.call(registry, name);
        const borrowedPending = other.whenDefined.call(registry, name);
        let ownResult;
        let borrowedResult;
        pending.then(value => { ownResult = value; });
        borrowedPending.then(value => { borrowedResult = value; });
        other.define.call(registry, name, constructor);
        await Promise.resolve();
        rows.push({
          case: `${label}-${kind}`,
          distinctConstructors: owner.CustomElementRegistry !== borrowed.CustomElementRegistry,
          get: own.get.call(registry, name) === constructor,
          borrowedGet: other.get.call(registry, name) === constructor,
          getName: own.getName.call(registry, constructor) === name,
          borrowedGetName: other.getName.call(registry, constructor) === name,
          samePendingPromise: pending === borrowedPending,
          whenDefined: ownResult === constructor,
          borrowedWhenDefined: borrowedResult === constructor,
          independent: borrowed.customElements.get(name) === undefined,
          scopedIndependent: kind === 'global' || owner.customElements.get(name) === undefined,
        });
      }
    }

    // V8 may allocate the receiver in NewTarget's realm. Its native registry
    // still belongs to the CustomElementRegistry constructor being invoked.
    for (const [label, owner, foreign] of [
      ['main', window, popup],
      ['popup', popup, window],
    ]) {
      const target = foreign.eval('(function RegistryNewTarget() {})');
      const registry = Reflect.construct(owner.CustomElementRegistry, [], target);
      const own = owner.CustomElementRegistry.prototype;
      const other = foreign.CustomElementRegistry.prototype;
      const name = `registry-${label}-foreign-new-target`;
      const constructor = class extends owner.HTMLElement {};
      own.define.call(registry, name, constructor);
      let result;
      other.whenDefined.call(registry, name).then(value => { result = value; });
      await Promise.resolve();
      rows.push({
        case: `${label}-foreign-new-target`,
        prototype: Object.getPrototypeOf(registry) === target.prototype,
        get: own.get.call(registry, name) === constructor,
        borrowedGet: other.get.call(registry, name) === constructor,
        getName: other.getName.call(registry, constructor) === name,
        whenDefined: result === constructor,
        independent: own.get.call(owner.customElements, name) === undefined &&
          other.get.call(foreign.customElements, name) === undefined,
      });
    }

    for (const [label, realm] of [['main', window], ['popup', popup]]) {
      const registry = new realm.CustomElementRegistry();
      const revoked = Proxy.revocable(registry, {});
      revoked.revoke();
      const invalid = [{}, Object.create(registry),
        Object.create(realm.CustomElementRegistry.prototype),
        new Proxy(registry, {}), revoked.proxy];
      let conversions = 0;
      const name = {toString() { ++conversions; return 'invalid-registry-receiver'; }};
      let correctErrors = 0;
      for (const receiver of invalid) {
        for (const [method, args] of [
          ['define', [name, class extends realm.HTMLElement {}]],
          ['get', [name]],
          ['getName', [function () {}]],
          ['whenDefined', [name]],
          ['upgrade', [document]],
          ['initialize', [document]],
        ]) {
          let promiseReturned = false;
          try {
            const result = realm.CustomElementRegistry.prototype[method].apply(receiver, args);
            if (method === 'whenDefined') {
              promiseReturned = result instanceof realm.Promise;
              await result;
            }
          } catch (error) {
            if (error instanceof realm.TypeError &&
                (method !== 'whenDefined' || promiseReturned)) ++correctErrors;
          }
        }
      }
      rows.push({
        case: `${label}-invalid-receivers`,
        brandErrors: correctErrors === invalid.length * 6,
        beforeConversion: conversions === 0,
        independent: registry.get('invalid-registry-receiver') === undefined &&
          realm.customElements.get('invalid-registry-receiver') === undefined,
      });
    }
    return JSON.stringify({rows});
  } finally {
    popup.close();
  }
})()
