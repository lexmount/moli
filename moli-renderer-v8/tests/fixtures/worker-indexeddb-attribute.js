function workerIndexedDBAttributeProbe() {
  const checks = [];
  const check = (label, pass, actual = '') => checks.push({label, pass, actual: String(actual)});
  const typeError = callback => {
    try { callback(); } catch (error) { return Object.getPrototypeOf(error) === TypeError.prototype; }
    return false;
  };
  const prototype = WorkerGlobalScope.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(prototype, 'indexedDB');
  const getter = descriptor?.get;
  const native = receiver => typeof getter === 'function' ? getter.call(receiver) : undefined;
  check('readonly prototype accessor', typeof getter === 'function' && descriptor.set === undefined);
  check('enumerable configurable prototype accessor', descriptor?.enumerable === true && descriptor.configurable === true);
  check('getter metadata', getter?.name === 'get indexedDB' && getter.length === 0);
  check('getter cannot construct', typeof getter === 'function' && typeError(() => new getter()));
  check('no initial own attribute', !Object.hasOwn(self, 'indexedDB'));
  check('invalid receiver before first factory access', typeError(() => native({})));

  const factory = self.indexedDB;
  const originalOwn = Object.getOwnPropertyDescriptor(self, 'indexedDB');
  const restoreOwn = () => {
    delete self.indexedDB;
    if (originalOwn) Object.defineProperty(self, 'indexedDB', originalOwn);
  };
  check('SameObject', self.indexedDB === factory);
  check('factory brand', factory instanceof IDBFactory);
  check('factory operation', factory.cmp(1, 2) === -1);
  check('read does not materialize own attribute', !Object.hasOwn(self, 'indexedDB'));
  for (const [label, receiver] of [['self', self], ['undefined', undefined], ['null', null]]) {
    check('getter receiver ' + label, native(receiver) === factory);
  }

  const revoked = Proxy.revocable(self, {}); revoked.revoke();
  let traps = 0;
  const proxy = new Proxy(self, {
    get() { ++traps; throw new Error('get trap'); },
    getPrototypeOf() { ++traps; throw new Error('prototype trap'); }
  });
  const invalid = [{}, prototype, Object.create(prototype), Object.create(self), 0, 'text',
    true, Symbol('receiver'), 1n, factory, IDBFactory.prototype, new Proxy(self, {}), revoked.proxy, proxy];
  for (let i = 0; i < invalid.length; ++i) {
    check('reject invalid receiver ' + i, typeError(() => native(invalid[i])));
  }
  check('receiver check does not invoke Proxy traps', traps === 0, traps);
  check('inherited attribute validates receiver', typeError(() => Object.create(self).indexedDB));
  check('author proxy property read validates receiver', typeError(() => new Proxy(self, {}).indexedDB));

  let conversions = 0;
  const replacement = {[Symbol.toPrimitive]() { ++conversions; throw new Error('conversion'); }};
  check('getter ignores extra arguments', typeof getter === 'function' && getter.call(self, replacement) === factory);
  check('Reflect.set rejects assignment', Reflect.set(self, 'indexedDB', replacement) === false);
  check('assignment preserves factory', self.indexedDB === factory);
  restoreOwn();
  check('strict assignment throws TypeError', typeError(() => { 'use strict'; self.indexedDB = replacement; }));
  check('strict assignment preserves factory', self.indexedDB === factory);
  restoreOwn();
  check('assignment does not convert value', conversions === 0, conversions);
  check('deleting absent own attribute succeeds', delete self.indexedDB);
  check('deleting absent own attribute preserves factory', self.indexedDB === factory);
  restoreOwn();

  Object.defineProperty(self, 'indexedDB', {value: 42, writable: true, enumerable: true, configurable: true});
  check('author data shadow is visible', self.indexedDB === 42);
  check('native getter ignores data shadow', native(self) === factory);
  check('author data shadow can be deleted', delete self.indexedDB);
  check('deleting data shadow restores same factory', self.indexedDB === factory);
  restoreOwn();
  let reads = 0;
  const marker = new Error('author getter');
  Object.defineProperty(self, 'indexedDB', {configurable: true, get() { ++reads; throw marker; }});
  check('native getter ignores accessor shadow', native(self) === factory && reads === 0, reads);
  let error;
  try { self.indexedDB; } catch (caught) { error = caught; }
  check('author accessor remains observable', error === marker && reads === 1, reads);
  delete self.indexedDB;
  check('deleting accessor shadow restores same factory', self.indexedDB === factory);
  restoreOwn();

  // Changing the exposed prototype property must not recreate the factory.
  Object.defineProperty(prototype, 'indexedDB', {value: 'prototype shadow', configurable: true});
  check('prototype property is configurable', self.indexedDB === 'prototype shadow');
  check('saved getter survives prototype replacement', native(self) === factory);
  delete prototype.indexedDB;
  check('saved getter survives prototype deletion', native(self) === factory);
  if (descriptor) Object.defineProperty(prototype, 'indexedDB', descriptor);
  check('restoring prototype restores same factory', self.indexedDB === factory);

  let constructorReads = 0;
  const constructors = ['WorkerGlobalScope', 'IDBFactory'];
  const originalConstructors = constructors.map(name => Object.getOwnPropertyDescriptor(self, name));
  try {
    for (const name of constructors) {
      Object.defineProperty(self, name, {configurable: true, get() { ++constructorReads; throw marker; }});
    }
    check('getter ignores replaced constructors', native(self) === factory && constructorReads === 0, constructorReads);
  } finally {
    constructors.forEach((name, index) => Object.defineProperty(self, name, originalConstructors[index]));
  }
  check('factory remains usable', factory.cmp('a', 'b') === -1);
  Object.defineProperty(self, 'indexedDB', {value: 'locked', writable: false, configurable: false});
  check('locked author shadow is visible', self.indexedDB === 'locked');
  check('native getter ignores locked shadow', native(self) === factory);
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
