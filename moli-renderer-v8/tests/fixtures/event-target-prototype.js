(() => {
  const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
  const typeError = (body, realm) => {
    try { body(); return false; } catch (error) { return error instanceof realm.TypeError; }
  };
  globalThis.eventTargetPrototypeProbe = (realm = globalThis, peers = [['self', realm]]) => {
    const rows = [], observations = {};
    const check = (name, body) => {
      try { rows.push({name, passed: body() === true}); }
      catch (error) { rows.push({name, passed: false, error: String(error)}); }
    };
    const C = realm.EventTarget, prototype = C.prototype;
    const names = ['addEventListener', 'removeEventListener', 'dispatchEvent'];
    const descriptors = Object.fromEntries(names.map(name => [name, Object.getOwnPropertyDescriptor(prototype, name)]));
    const add = prototype.addEventListener, remove = prototype.removeEventListener, dispatch = prototype.dispatchEvent;
    const event = type => new realm.Event(type, {cancelable: true});
    const make = () => new C();
    for (const [name, length] of [['addEventListener', 2], ['removeEventListener', 2], ['dispatchEvent', 1]]) {
      check(`${name} prototype descriptor`, () => {
        const d = descriptors[name];
        return d.writable && d.enumerable && d.configurable && d.value.name === name &&
          d.value.length === length && Object.getPrototypeOf(d.value) === realm.Function.prototype;
      });
      check(`${name} is inherited with canonical identity`, () => {
        const a = make(), b = make();
        return !own(a, name) && !own(b, name) && a[name] === descriptors[name].value && a[name] === b[name];
      });
      check(`${name} follows author prototype replacement`, () => {
        const before = make(), replacement = () => 'author';
        try {
          Object.defineProperty(prototype, name, {...descriptors[name], value: replacement});
          const after = make();
          return before[name] === replacement && after[name] === replacement && !own(after, name);
        } finally { Object.defineProperty(prototype, name, descriptors[name]); }
      });
      check(`${name} deletion does not materialize own methods`, () => {
        try {
          delete prototype[name];
          const target = make();
          let calls = 0; const callback = () => {calls++;};
          add.call(target, 'x', callback); dispatch.call(target, event('x')); remove.call(target, 'x', callback);
          return !(name in target) && !own(target, name) && calls === 1;
        } finally { Object.defineProperty(prototype, name, descriptors[name]); }
      });
    }
    check('constructor requirements and ignored arguments', () => {
      let conversions = 0;
      const ignored = {toString() {conversions++; throw 1;}};
      return C.length === 0 && typeError(() => C(), realm) &&
        new C(ignored) instanceof C && conversions === 0;
    });
    check('native state is private and per-instance', () => {
      const a = make(), b = make(); let calls = 0;
      add.call(a, 'x', () => {calls++;});
      dispatch.call(b, event('x')); dispatch.call(a, event('x'));
      return Reflect.ownKeys(a).length === 0 && Reflect.ownKeys(b).length === 0 && calls === 1;
    });
    check('listener call has target identity and native event state', () => {
      const target = make(), e = event('x'); let observed = false;
      add.call(target, 'x', function(received) {
        observed = received === e && this === target && e.target === target &&
          e.currentTarget === target && e.eventPhase === 2 && e.composedPath().length === 1 &&
          e.composedPath()[0] === target && !e.isTrusted;
      });
      const result = dispatch.call(target, e);
      return observed && result && e.target === target && e.currentTarget === null &&
        e.eventPhase === 0 && e.composedPath().length === 0;
    });
    check('frozen target retains mutable native listener state', () => {
      const target = Object.freeze(make()); let calls = 0;
      const callback = () => {calls++;};
      add.call(target, 'x', callback); dispatch.call(target, event('x'));
      remove.call(target, 'x', callback); dispatch.call(target, event('x'));
      return calls === 1 && Reflect.ownKeys(target).length === 0;
    });
    check('prototype removal preserves native receiver and state', () => {
      const target = make(); Object.setPrototypeOf(target, null); let calls = 0;
      add.call(target, 'x', function(e) {if (e.target === target && this === target) calls++;});
      return dispatch.call(target, event('x')) && calls === 1;
    });
    check('subclass overrides are not shadowed by native own methods', () => {
      let registrations = 0, calls = 0;
      class Derived extends C {
        addEventListener(...args) {registrations++; return add.apply(this, args);}
      }
      const target = new Derived();
      target.addEventListener('x', () => {calls++;});
      return target instanceof Derived && !own(target, 'addEventListener') &&
        target.dispatchEvent(event('x')) && registrations === 1 && calls === 1;
    });
    check('custom newTarget prototype preserves branding and listener state', () => {
      function NewTarget() {}
      const custom = {marker: 1}; NewTarget.prototype = custom;
      const target = Reflect.construct(C, [], NewTarget); let calls = 0;
      add.call(target, 'x', function(e) {if (this === target && e.target === target) calls++;});
      dispatch.call(target, event('x'));
      return Object.getPrototypeOf(target) === custom && target.marker === 1 &&
        names.every(name => !own(target, name)) && calls === 1;
    });
    check('constructor does not consult prototype method getters', () => {
      let reads = 0;
      try {
        for (const name of names) Object.defineProperty(prototype, name, {configurable: true, get() {reads++; throw 1;}});
        const target = make(); let calls = 0;
        add.call(target, 'x', () => {calls++;}); dispatch.call(target, event('x'));
        return reads === 0 && calls === 1 && names.every(name => !own(target, name));
      } finally { for (const name of names) Object.defineProperty(prototype, name, descriptors[name]); }
    });
    check('once and duplicate identity are preserved', () => {
      const target = make(); let calls = 0; const callback = () => {calls++;};
      add.call(target, 'x', callback, {once: true}); add.call(target, 'x', callback, {once: false});
      dispatch.call(target, event('x')); dispatch.call(target, event('x'));
      return calls === 1;
    });
    check('capture order and matching removal are preserved', () => {
      const target = make(), trace = [], capture = () => trace.push('capture');
      add.call(target, 'x', () => trace.push('bubble')); add.call(target, 'x', capture, true);
      dispatch.call(target, event('x')); remove.call(target, 'x', capture, true);
      dispatch.call(target, event('x'));
      observations.captureOrder = trace.join(',');
      return observations.captureOrder === 'bubble,capture,bubble';
    });
    check('cancellation and passive listeners are preserved', () => {
      const a = make(), b = make(), ea = event('x'), eb = event('x');
      add.call(a, 'x', e => e.preventDefault());
      add.call(b, 'x', e => e.preventDefault(), {passive: true});
      return dispatch.call(a, ea) === false && ea.defaultPrevented &&
        dispatch.call(b, eb) === true && !eb.defaultPrevented;
    });
    check('AbortSignal removes listeners from native state', () => {
      const target = make(), controller = new realm.AbortController(); let calls = 0;
      add.call(target, 'x', () => {calls++;}, {signal: controller.signal});
      dispatch.call(target, event('x')); controller.abort(); dispatch.call(target, event('x'));
      return calls === 1;
    });
    check('callback interface lookup remains dynamic', () => {
      const target = make(), trace = [], listener = {handleEvent() {if (this === listener) trace.push('first');}};
      add.call(target, 'x', listener); dispatch.call(target, event('x'));
      listener.handleEvent = function() {if (this === listener) trace.push('second');};
      dispatch.call(target, event('x')); remove.call(target, 'x', listener); dispatch.call(target, event('x'));
      return trace.join(',') === 'first,second';
    });
    check('listener Proxy is callable without making a Proxy receiver valid', () => {
      const target = make(); let calls = 0;
      const callback = new Proxy(() => {}, {apply(_fn, receiver, args) {
        if (receiver === target && args[0].target === target) calls++;
      }});
      add.call(target, 'x', callback); dispatch.call(target, event('x'));
      return calls === 1;
    });
    check('stopImmediatePropagation stops later target listeners', () => {
      const target = make(), trace = [];
      add.call(target, 'x', e => {trace.push('first'); e.stopImmediatePropagation();});
      add.call(target, 'x', () => trace.push('second')); dispatch.call(target, event('x'));
      return trace.join(',') === 'first';
    });
    check('listener removal during dispatch preserves live registration state', () => {
      const target = make(), trace = [], later = () => trace.push('later');
      add.call(target, 'x', () => {trace.push('first'); remove.call(target, 'x', later);});
      add.call(target, 'x', later); dispatch.call(target, event('x'));
      return trace.join(',') === 'first';
    });
    for (const name of names) {
      check(`${name} enforces required argument count in callee realm`, () =>
        typeError(() => descriptors[name].value.call(make()), realm));
    }
    for (const [label, receiver] of [['null', null], ['undefined', undefined]]) {
      check(`${label} receiver maps to the callee global EventTarget`, () => {
        const type = 'event-target-global-' + label;
        let calls = 0;
        const callback = function(e) {if (this === realm && e.target === realm) calls++;};
        try {
          add.call(receiver, type, callback);
          const result = dispatch.call(receiver, new realm.Event(type));
          remove.call(receiver, type, callback);
          dispatch.call(receiver, new realm.Event(type));
          return result === true && calls === 1;
        } finally { remove.call(realm, type, callback); }
      });
    }
    for (const [ownerName, owner] of peers) {
      const target = new owner.EventTarget(); let traps = 0;
      const proxy = new Proxy(target, {get() {traps++;}, getPrototypeOf() {traps++;}, has() {traps++;}});
      const revoked = Proxy.revocable(target, {}); revoked.revoke();
      const invalid = [['plain', {}], ['prototype', owner.EventTarget.prototype],
        ['forged', Object.create(owner.EventTarget.prototype)], ['inherited instance', Object.create(target)],
        ['author Proxy', proxy], ['revoked Proxy', revoked.proxy], ['number', 1]];
      for (const [label, receiver] of invalid) {
        for (const name of names) {
          check(`${ownerName} ${name} rejects ${label} before conversion in callee realm`, () => {
            let conversions = 0;
            const type = {toString() {conversions++; return 'x';}};
            const args = name === 'dispatchEvent' ? [event('x')] : [type, null];
            return typeError(() => descriptors[name].value.call(receiver, ...args), realm) && conversions === 0;
          });
        }
      }
      check(`${ownerName} receiver brand check does not execute traps`, () => traps === 0);
      check(`${ownerName} borrowed methods preserve target identity and dispatch`, () => {
        const e = new owner.Event('borrowed', {cancelable: true}); let calls = 0;
        function listener(received) {if (this === target && received === e && e.target === target) calls++;}
        add.call(target, 'borrowed', listener); const result = dispatch.call(target, e);
        remove.call(target, 'borrowed', listener); dispatch.call(target, new owner.Event('borrowed'));
        return result && calls === 1;
      });
    }
    return {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows, observations};
  };
  const realms = typeof document === 'undefined' ? [['worker', globalThis]] :
    [['top', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const probes = realms.map(([name, realm]) => [name, eventTargetPrototypeProbe(realm, realms)]);
  const rows = probes.flatMap(([name, probe]) => probe.rows.map(row => ({...row, name: `${name}: ${row.name}`})));
  const observations = Object.fromEntries(probes.map(([name, probe]) => [name, probe.observations]));
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows, observations};
  return rows.every(row => row.passed);
})()
