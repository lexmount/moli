globalThis.versionChangeChecks = [];
async function versionChangeProbe(prefix = 'version-change') {
  const checks = versionChangeChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const same = (label, actual, expected) => check(label, JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify(actual));
  const throws = (label, action, Realm) => {
    let caught;
    try { action(); } catch (error) { caught = error; }
    check(label, caught !== undefined && Object.getPrototypeOf(caught) === Realm.TypeError.prototype, caught);
  };
  const exercise = async (label, Realm, GetterRealm) => {
    const Ctor = Realm.IDBVersionChangeEvent;
    const getters = {};
    for (const name of ['oldVersion', 'newVersion']) {
      const descriptor = Object.getOwnPropertyDescriptor(GetterRealm.IDBVersionChangeEvent.prototype, name);
      getters[name] = descriptor?.get;
      check(label + ': ' + name + ' descriptor', typeof descriptor?.get === 'function' &&
        descriptor.set === undefined && descriptor.enumerable && descriptor.configurable);
      check(label + ': ' + name + ' getter metadata', descriptor?.get?.name === 'get ' + name && descriptor.get.length === 0);
    }
    const get = (name, receiver) => typeof getters[name] === 'function' ? getters[name].call(receiver) : undefined;
    check(label + ': constructor length', Ctor.length === 1, Ctor.length);
    throws(label + ': new is required', () => Ctor('type'), Realm);
    throws(label + ': type is required', () => new Ctor(), Realm);
    throws(label + ': symbol type is rejected', () => new Ctor(Symbol()), Realm);
    for (const invalid of [1, 'init', Symbol(), 1n]) {
      throws(label + ': primitive dictionary is rejected', () => new Ctor('type', invalid), Realm);
    }
    for (const init of [undefined, null, {}]) {
      const event = new Ctor('default', init);
      same(label + ': default versions', [event.oldVersion, event.newVersion], [0, null]);
      same(label + ': default flags', [event.bubbles, event.cancelable, event.composed, event.isTrusted], [false, false, false, false]);
    }
    const order = [];
    const init = {};
    for (const [name, value] of [['bubbles', true], ['cancelable', true], ['composed', true],
      ['newVersion', {valueOf() { order.push('convert newVersion'); return 9; }}],
      ['oldVersion', {valueOf() { order.push('convert oldVersion'); return 7; }}]]) {
      Object.defineProperty(init, name, {get() { order.push(name); return value; }});
    }
    const event = new Ctor({toString() { order.push('type'); return 'flags'; }}, Object.create(init));
    same(label + ': conversion order', order,
      ['type', 'bubbles', 'cancelable', 'composed', 'newVersion', 'convert newVersion', 'oldVersion', 'convert oldVersion']);
    for (const name of ['bubbles', 'cancelable', 'composed']) {
      check(label + ': EventInit ' + name, event[name] === true, event[name]);
    }
    const target = new EventTarget();
    target.addEventListener('flags', value => value.preventDefault());
    check(label + ': EventInit dispatch cancellation', target.dispatchEvent(event) === false);
    check(label + ': EventInit defaultPrevented', event.defaultPrevented === true);
    for (const [name, expected] of [['oldVersion', 7], ['newVersion', 9]]) {
      check(label + ': ' + name + ' inherited', !Object.hasOwn(event, name));
      check(label + ': ' + name + ' getter value', get(name, event) === expected, get(name, event));
      check(label + ': ' + name + ' readonly', Reflect.set(event, name, 99) === false);
      check(label + ': ' + name + ' unchanged', event[name] === expected, event[name]);
      Object.defineProperty(event, name, {configurable: true, get() { throw new Error('author shadow'); }});
      check(label + ': ' + name + ' native value under author shadow', get(name, event) === expected, get(name, event));
      delete event[name];
      check(label + ': ' + name + ' restored getter', event[name] === expected, event[name]);
    }
    const revoked = Proxy.revocable(event, {}); revoked.revoke();
    let traps = 0;
    const trapped = new Proxy(event, {get() { ++traps; throw new Error('trap'); }, getPrototypeOf() { ++traps; throw new Error('trap'); }});
    for (const receiver of [null, undefined, 1, {}, Ctor.prototype, Object.create(Ctor.prototype),
      Object.create(event), new Realm.Event('base'), new Proxy(event, {}), revoked.proxy, trapped]) {
      for (const name of ['oldVersion', 'newVersion']) {
        throws(label + ': ' + name + ' invalid receiver', () => get(name, receiver), GetterRealm);
      }
    }
    check(label + ': no Proxy traps', traps === 0, traps);
    Object.setPrototypeOf(event, null);
    same(label + ': native identity survives prototype removal', [get('oldVersion', event), get('newVersion', event)], [7, 9]);
    class Subclass extends Ctor {}
    const sub = new Subclass('sub', {oldVersion: 3, newVersion: 4});
    check(label + ': subclass prototype is preserved', Object.getPrototypeOf(sub) === Subclass.prototype);
    same(label + ': subclass native identity', [get('oldVersion', sub), get('newVersion', sub)], [3, 4]);

    const members = ['bubbles', 'cancelable', 'composed', 'newVersion', 'oldVersion'];
    for (let stop = 0; stop < members.length; ++stop) {
      const reads = [];
      const marker = new Error('dictionary getter');
      const options = new Proxy({}, {get(_target, key) {
        reads.push(key);
        if (key === members[stop]) throw marker;
        return undefined;
      }});
      let caught;
      try { new Ctor('throws', options); } catch (error) { caught = error; }
      check(label + ': getter exception identity ' + members[stop], caught === marker, caught);
      same(label + ': stop conversion at ' + members[stop], reads, members.slice(0, stop + 1));
    }
    const conversions = [[undefined, 0, null], [null, 0, null], [NaN, 0, 0], [Infinity, 0, 0],
      [-Infinity, 0, 0], [1.9, 1, 1], [-1, 2 ** 64, 2 ** 64], [2 ** 64, 0, 0], ['12', 12, 12]];
    for (const [value, oldVersion, newVersion] of conversions) {
      const converted = new Ctor('number', {oldVersion: value, newVersion: value});
      same(label + ': numeric conversion ' + String(value), [converted.oldVersion, converted.newVersion], [oldVersion, newVersion]);
    }
    for (const name of ['oldVersion', 'newVersion']) {
      for (const value of [Symbol(), 1n]) {
        throws(label + ': reject invalid ' + name, () => new Ctor('invalid', {[name]: value}), Realm);
      }
    }
    const marker = new Error('type conversion');
    let dictionaryReads = 0;
    let caught;
    try {
      new Ctor({toString() { throw marker; }}, new Proxy({}, {get() { ++dictionaryReads; }}));
    } catch (error) { caught = error; }
    check(label + ': type exception identity', caught === marker);
    check(label + ': type exception stops dictionary conversion', dictionaryReads === 0, dictionaryReads);

    const nativeEvents = [];
    const seen = [];
    const inspectNative = (name, value, oldVersion, newVersion) => {
      seen.push(name);
      same(label + ': native ' + name + ' versions', [value.oldVersion, value.newVersion], [oldVersion, newVersion]);
      same(label + ': native ' + name + ' flags', [value.bubbles, value.cancelable, value.composed, value.isTrusted], [false, false, false, true]);
      check(label + ': native ' + name + ' prototype', Object.getPrototypeOf(value) === Ctor.prototype);
      check(label + ': native ' + name + ' inherited attributes', !Object.hasOwn(value, 'oldVersion') && !Object.hasOwn(value, 'newVersion'));
      same(label + ': native ' + name + ' borrowed getters', [get('oldVersion', value), get('newVersion', value)], [oldVersion, newVersion]);
      check(label + ': native ' + name + ' readonly oldVersion', Reflect.set(value, 'oldVersion', 99) === false);
      check(label + ': native ' + name + ' readonly newVersion', Reflect.set(value, 'newVersion', 99) === false);
      nativeEvents.push([name, value, oldVersion, newVersion]);
    };
    const requestResult = request => new Promise((resolve, reject) => {
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const factory = Realm.indexedDB;
    const name = prefix + '-' + label;
    const savedCtor = Realm.IDBVersionChangeEvent;
    Realm.IDBVersionChangeEvent = function() { throw new Error('author constructor'); };
    try {
      const first = factory.open(name, 1);
      first.onupgradeneeded = value => inspectNative('create', value, 0, 1);
      const oldDb = await requestResult(first);
      oldDb.onversionchange = value => inspectNative('versionchange', value, 1, 2);
      const second = factory.open(name, 2);
      second.onblocked = value => { inspectNative('blocked', value, 1, 2); oldDb.close(); };
      second.onupgradeneeded = value => inspectNative('upgrade', value, 1, 2);
      const newDb = await requestResult(second);
      newDb.onversionchange = value => { inspectNative('delete notification', value, 2, null); newDb.close(); };
      const deletion = factory.deleteDatabase(name);
      await new Promise((resolve, reject) => {
        deletion.onsuccess = value => { inspectNative('delete success', value, 2, null); resolve(); };
        deletion.onerror = () => reject(deletion.error);
      });
      const absent = factory.deleteDatabase(name);
      await new Promise((resolve, reject) => {
        absent.onsuccess = value => { inspectNative('absent delete', value, 0, null); resolve(); };
        absent.onerror = () => reject(absent.error);
      });
    } finally { Realm.IDBVersionChangeEvent = savedCtor; }
    same(label + ': native event order', seen,
      ['create', 'versionchange', 'blocked', 'upgrade', 'delete notification', 'delete success', 'absent delete']);
    for (const [name, value, oldVersion, newVersion] of nativeEvents) {
      same(label + ': retained native ' + name, [get('oldVersion', value), get('newVersion', value)], [oldVersion, newVersion]);
    }
  };
  await exercise('local', globalThis, globalThis);
  if (typeof document !== 'undefined') {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    frame.srcdoc = '<!doctype html>'; document.documentElement.appendChild(frame); await loaded;
    try {
      await exercise('foreign-getter', globalThis, frame.contentWindow);
      await exercise('foreign-event', frame.contentWindow, globalThis);
    } finally { frame.remove(); }
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
