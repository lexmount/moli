globalThis.factoryReceiverChecks = [];
async function factoryReceiverProbe(prefix = 'factory-receivers') {
  const checks = factoryReceiverChecks;
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const throws = (label, action, prototype) => {
    let caught;
    try { action(); } catch (error) { caught = error; }
    check(label, caught !== undefined && Object.getPrototypeOf(caught) === prototype, caught);
  };
  const result = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const exercise = async (label, Callee, Receiver) => {
    const prototype = Callee.IDBFactory.prototype;
    const factory = Receiver.indexedDB;
    const marker = new Error('argument must not be evaluated');
    let reads = 0;
    const name = {[Symbol.toPrimitive]() { ++reads; throw marker; }};
    const version = {valueOf() { ++reads; throw marker; }};
    const key = [1];
    Object.defineProperty(key, '0', {get() { ++reads; throw marker; }});
    const revoked = Proxy.revocable(factory, {}); revoked.revoke();
    let traps = 0;
    const trapped = new Proxy(factory, {
      get() { ++traps; throw marker; },
      getPrototypeOf() { ++traps; throw marker; },
      has() { ++traps; throw marker; }
    });
    const invalid = [
      ['null', null], ['undefined', undefined], ['number', 1], ['string', 'factory'],
      ['symbol', Symbol('factory')], ['ordinary', {}], ['prototype', Receiver.IDBFactory.prototype],
      ['forged', Object.create(Receiver.IDBFactory.prototype)], ['inherited', Object.create(factory)],
      ['proxy', new Proxy(factory, {})], ['revoked proxy', revoked.proxy], ['trapped proxy', trapped]
    ];
    const methods = [['open', [name, version]], ['deleteDatabase', [name]], ['cmp', [key, 1]]];
    for (const [kind, receiver] of invalid) {
      for (const [method, args] of methods) {
        reads = 0;
        throws(label + ': ' + method + ' rejects ' + kind,
          () => Reflect.apply(prototype[method], receiver, args), Callee.TypeError.prototype);
        check(label + ': ' + method + ' checks ' + kind + ' before conversion', reads === 0, reads);
      }
      throws(label + ': cmp rejects ' + kind + ' with valid keys',
        () => prototype.cmp.call(receiver, 1, 2), Callee.TypeError.prototype);
      let promise;
      let synchronousError;
      try { promise = prototype.databases.call(receiver, name); } catch (error) { synchronousError = error; }
      check(label + ': databases does not throw for ' + kind, synchronousError === undefined, synchronousError);
      check(label + ': databases rejection Promise realm for ' + kind,
        promise !== undefined && Object.getPrototypeOf(promise) === Callee.Promise.prototype);
      let synchronous = true;
      const settled = Promise.resolve(promise).then(
        () => ({error: undefined, synchronous}), error => ({error, synchronous}));
      synchronous = false;
      const rejection = await settled;
      check(label + ': databases rejects ' + kind + ' in callee realm',
        rejection.error !== undefined && Object.getPrototypeOf(rejection.error) === Callee.TypeError.prototype,
        rejection.error);
      check(label + ': databases rejection is asynchronous for ' + kind, !rejection.synchronous);
    }
    check(label + ': brand checks do not invoke Proxy traps', traps === 0, traps);
    for (const [method, args] of methods) {
      reads = 0;
      let caught;
      try { Reflect.apply(prototype[method], factory, args); } catch (error) { caught = error; }
      check(label + ': valid ' + method + ' preserves conversion exception', caught === marker, caught);
      check(label + ': valid ' + method + ' evaluates argument once', reads === 1, reads);
    }
    for (const method of ['open', 'deleteDatabase', 'cmp']) {
      throws(label + ': ' + method + ' requires arguments',
        () => prototype[method].call(factory), Callee.TypeError.prototype);
    }
    throws(label + ': open rejects zero version in callee realm',
      () => prototype.open.call(factory, prefix, 0), Callee.TypeError.prototype);
    throws(label + ': cmp rejects invalid keys in callee realm',
      () => prototype.cmp.call(factory, null, 1), Callee.DOMException.prototype);
    check(label + ': genuine receiver compares keys', prototype.cmp.call(factory, [1], [2]) === -1);
    for (const [method, length] of [['open', 1], ['deleteDatabase', 1], ['databases', 0], ['cmp', 2]]) {
      check(label + ': ' + method + ' length', prototype[method].length === length, prototype[method].length);
    }

    // Changing an author's prototype cannot remove native identity. Preserve the
    // genuine receiver's storage ownership while borrowing another realm's API.
    const originalPrototype = Object.getPrototypeOf(factory);
    let request;
    const conversionOrder = [];
    const databaseName = prefix + '-' + label;
    try {
      Object.setPrototypeOf(factory, null);
      check(label + ': native brand survives prototype removal', prototype.cmp.call(factory, 1, 2) === -1);
      request = prototype.open.call(factory,
        {toString() { conversionOrder.push('name'); return databaseName; }},
        {valueOf() { conversionOrder.push('version'); return 1; }});
    } finally { Object.setPrototypeOf(factory, originalPrototype); }
    check(label + ': valid open conversion order', conversionOrder.join(',') === 'name,version', conversionOrder);
    check(label + ': open request belongs to receiver', Object.getPrototypeOf(request) === Receiver.IDBOpenDBRequest.prototype);
    check(label + ': open request source is null', request.source === null);
    check(label + ': open request starts pending', request.readyState === 'pending');
    const db = await result(request);
    check(label + ': open preserves database name', db.name === databaseName, db.name);
    check(label + ': open preserves version', db.version === 1, db.version);
    const ignored = new Proxy({}, {get() { throw new Error('databases ignores arguments'); }});
    const databases = prototype.databases.call(factory, ignored);
    check(label + ': successful databases Promise belongs to receiver', Object.getPrototypeOf(databases) === Receiver.Promise.prototype);
    const infos = await databases;
    check(label + ': databases uses receiver storage', infos.some(info => info.name === databaseName && info.version === 1));
    db.close();
    const deletion = prototype.deleteDatabase.call(factory, databaseName);
    check(label + ': delete request belongs to receiver', Object.getPrototypeOf(deletion) === Receiver.IDBOpenDBRequest.prototype);
    check(label + ': delete request source is null', deletion.source === null);
    check(label + ': delete succeeds', await result(deletion) === undefined);
    check(label + ': database was deleted', !(await prototype.databases.call(factory)).some(info => info.name === databaseName));
  };
  await exercise('local', globalThis, globalThis);
  if (typeof document !== 'undefined') {
    const frame = document.createElement('iframe');
    const loaded = new Promise(resolve => { frame.onload = resolve; });
    frame.srcdoc = '<!doctype html>'; document.documentElement.appendChild(frame); await loaded;
    try {
      await exercise('foreign-method', frame.contentWindow, globalThis);
      await exercise('foreign-receiver', globalThis, frame.contentWindow);
    } finally { frame.remove(); }
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
