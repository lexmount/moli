(() => {
  const child = document.getElementById('child').contentWindow;
  const frame = child.document.createElement('iframe');
  child.document.body.appendChild(frame);
  const realms = [['root', window], ['child', child], ['nested', frame.contentWindow]];
  const descriptors = realms.map(([label, realm]) => ({label, realm,
    descriptor: Object.getOwnPropertyDescriptor(realm, 'event')}));
  const rows = [];
  for (const {label, realm, descriptor} of descriptors) {
    for (const mode of ['ordinary', 'overwrite', 'accessor', 'prototype']) {
      const checks = [];
      const check = (name, pass) => checks.push({name, pass: !!pass});
      const oldName = '__moliWindowEvent';
      const previousSlot = Object.getOwnPropertyDescriptor(realm, oldName);
      const prototype = Object.getPrototypeOf(realm);
      const previousPrototypeSlot = Object.getOwnPropertyDescriptor(prototype, oldName);
      const previousError = realm.onerror;
      const authorValue = {};
      const publicValue = {};
      let reads = 0, writes = 0, caught = 0;
      const trace = [];
      const read = () => descriptor.get.call(realm);
      check('no internal own name', !Object.getOwnPropertyNames(realm).includes(oldName));
      check('no internal reflected key', !Reflect.ownKeys(realm).includes(oldName));
      check('public accessor', typeof descriptor.get === 'function' && typeof descriptor.set === 'function');
      try {
        const map = new realm.WeakMap();
        for (const name of Object.getOwnPropertyNames(realm)) {
          if (/[A-Z][A-Za-z0-9]+Event$/.test(name)) map.set(realm[name], name);
        }
        check('EventRecorder constructor scan', true);
      } catch (error) { check('EventRecorder constructor scan', false); }
      if (mode !== 'ordinary') {
        delete realm[oldName];
        if (mode === 'overwrite') Object.defineProperty(realm, oldName,
          {value: authorValue, writable: true, configurable: true});
        else Object.defineProperty(mode === 'prototype' ? prototype : realm, oldName,
          {configurable: true, get() { reads++; return authorValue; }, set() { writes++; }});
      }
      const target = new realm.EventTarget();
      const state = {read, check, target, trace, outer: null, replaced: false,
        visible: () => realm.event, publicValue,
        otherWindowsClear: () => descriptors.filter(d => d.realm !== realm)
          .every(d => d.descriptor.get.call(d.realm) === undefined)};
      const callbacks = realm.Function('s', `return {
        outer(event) {
          s.trace.push('outer'); s.outer = event;
          s.check('outer current', s.read() === event);
          s.check('other windows clear', s.otherWindowsClear());
          s.check('outer public value', s.visible() === (s.replaced ? s.publicValue : event));
          s.target.dispatchEvent(new Event('inner'));
          s.check('outer restored', s.read() === event);
          s.trace.push('restored');
        },
        inner(event) {
          s.trace.push('inner');
          s.check('inner current', s.read() === event && event !== s.outer);
          s.check('inner other windows clear', s.otherWindowsClear());
        },
        object: { get handleEvent() {
          s.trace.push('lookup'); s.check('current before handleEvent lookup', s.read()?.type === 'lookup');
          return event => s.check('object callback current', s.read() === event);
        } },
        fail() { throw new Error('expected current-event probe failure'); },
        report() {
          s.trace.push('error');
          s.check('exception report current', s.read()?.type === 'error');
          return true;
        }
      }`)(state);
      target.addEventListener('outer', callbacks.outer);
      target.addEventListener('inner', callbacks.inner);
      target.addEventListener('lookup', callbacks.object);
      target.addEventListener('fail', callbacks.fail);
      realm.onerror = callbacks.report;
      try {
        check('undefined before dispatch', read() === undefined);
        target.dispatchEvent(new realm.Event('outer'));
        check('undefined after nested dispatch', read() === undefined);
        target.dispatchEvent(new realm.Event('lookup'));
        check('undefined after object callback', read() === undefined);
        target.dispatchEvent(new realm.Event('fail'));
        check('undefined after exception reporting', read() === undefined);
        realm.event = publicValue;
        check('public Replaceable value', realm.event === publicValue);
        check('replacement does not change native current event', read() === undefined);
        const replacement = Object.getOwnPropertyDescriptor(realm, 'event');
        check('Replaceable data descriptor', replacement.value === publicValue &&
          replacement.writable && replacement.enumerable && replacement.configurable);
        state.replaced = true;
        target.dispatchEvent(new realm.Event('outer'));
        check('replacement survives callback', realm.event === publicValue);
        check('native current cleared with replacement', read() === undefined);
        check('trace', trace.join(',') === 'outer,inner,restored,lookup,error,outer,inner,restored');
        check('author getter ignored', reads === 0);
        check('author setter ignored', writes === 0);
        if (mode === 'overwrite') check('author slot value unchanged', realm[oldName] === authorValue);
        if (mode === 'ordinary') check('dispatch does not publish internal state', !Object.hasOwn(realm, oldName));
      } catch (error) { caught++; check('dispatch completed without uncaught exception', false); }
      finally {
        Object.defineProperty(realm, 'event', descriptor);
        realm.onerror = previousError;
        delete realm[oldName];
        if (previousSlot) Object.defineProperty(realm, oldName, previousSlot);
        if (mode === 'prototype') {
          delete prototype[oldName];
          if (previousPrototypeSlot) Object.defineProperty(prototype, oldName, previousPrototypeSlot);
        }
      }
      rows.push({label, mode, complete: caught === 0 && checks.every(c => c.pass),
        passed: checks.filter(c => c.pass).length, total: checks.length, checks, trace, reads, writes, caught});
    }
  }
  globalThis.__uiEventResults = {rows, complete: rows.every(r => r.complete),
    passed: rows.reduce((n, r) => n + r.passed, 0), total: rows.reduce((n, r) => n + r.total, 0)};
  return __uiEventResults.complete;
})();
