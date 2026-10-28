(async () => {
  const rows = [];
  const methods = CustomElementRegistry.prototype;
  for (const kind of ['global', 'scoped']) {
    const popup = open('about:blank');
    try {
      const registry = kind === 'global' ? popup.customElements : new popup.CustomElementRegistry();
      const name = `retirement-${kind}-before`;
      const lateName = `retirement-${kind}-after`;
      const Constructor = class extends popup.HTMLElement {};
      let fulfilled;
      let lateFulfilled = false;
      const pending = methods.whenDefined.call(registry, lateName);
      pending.then(() => { lateFulfilled = true; });
      methods.define.call(registry, name, Constructor);
      methods.whenDefined.call(registry, name).then(value => { fulfilled = value; });
      await Promise.resolve();
      rows.push({case: `${kind}-before-close`,
        get: methods.get.call(registry, name) === Constructor,
        getName: methods.getName.call(registry, Constructor) === name,
        whenDefined: fulfilled === Constructor,
        samePending: methods.whenDefined.call(registry, lateName) === pending,
      });

      popup.close();
      await new Promise(resolve => setTimeout(resolve, 500));
      let conversions = 0;
      let prototypeReads = 0;
      const LateConstructor = new Proxy(class {}, {get(target, key, receiver) {
        if (key === 'prototype') ++prototypeReads;
        return Reflect.get(target, key, receiver);
      }});
      methods.define.call(registry, {toString() { ++conversions; return lateName; }}, LateConstructor);
      fulfilled = undefined;
      methods.whenDefined.call(registry, name).then(value => { fulfilled = value; });
      await Promise.resolve();
      rows.push({case: `${kind}-after-close`,
        get: methods.get.call(registry, name) === Constructor,
        getName: methods.getName.call(registry, Constructor) === name,
        whenDefined: fulfilled === Constructor,
        samePending: methods.whenDefined.call(registry, lateName) === pending,
        lateDefinitionAbsent: methods.get.call(registry, lateName) === undefined,
        latePromisePending: !lateFulfilled,
        argumentsConverted: conversions === 1,
        constructorReadOnce: prototypeReads === 1,
      });
    } finally { popup.close(); }
  }
  return JSON.stringify({rows});
})()
