(scopeName => {
  const failures = [];
  let checks = 0;
  const check = (value, label) => {
    checks++;
    if (!value) failures.push(label);
  };
  const test = (label, callback) => {
    try { callback(); } catch (error) { check(false, label + ': ' + error); }
  };
  const throwsTypeError = callback => {
    try { callback(); } catch (error) { return error instanceof TypeError; }
    return false;
  };
  const scopeConstructor = self[scopeName];
  const eventTarget = EventTarget;
  const chain = [self, scopeConstructor.prototype, WorkerGlobalScope.prototype,
    eventTarget.prototype, Object.prototype];
  for (let i = 0; i < chain.length; i++) {
    const value = chain[i];
    const parent = chain[i + 1] || null;
    const original = Object.getPrototypeOf(value);
    check(original === parent, 'prototype parent ' + i);
    check(Object.isExtensible(value), 'extensible ' + i);
    check(!Reflect.setPrototypeOf(value, {}), 'Reflect rejects different prototype ' + i);
    // Restore the prototype in a broken implementation so other checks remain useful.
    if (Object.getPrototypeOf(value) !== original) Reflect.setPrototypeOf(value, original);
    check(throwsTypeError(() => Object.setPrototypeOf(value, {})), 'Object rejects different prototype ' + i);
    if (Object.getPrototypeOf(value) !== original) Reflect.setPrototypeOf(value, original);
    check(Reflect.setPrototypeOf(value, original), 'same prototype allowed ' + i);
  }
  check(Object.getPrototypeOf(scopeConstructor) === WorkerGlobalScope, 'specific constructor inherits WorkerGlobalScope');
  check(Object.getPrototypeOf(WorkerGlobalScope) === eventTarget, 'WorkerGlobalScope constructor inherits EventTarget');
  check(self instanceof scopeConstructor, 'specific instanceof');
  check(self instanceof WorkerGlobalScope, 'WorkerGlobalScope instanceof');
  check(self instanceof eventTarget, 'EventTarget instanceof');
  check(Object.prototype.toString.call(self) === '[object ' + scopeName + ']', 'global tag');
  check(Object.prototype.toString.call(WorkerGlobalScope.prototype) === '[object WorkerGlobalScope]', 'worker tag');
  check(throwsTypeError(() => new scopeConstructor()), 'specific illegal constructor');
  check(throwsTypeError(() => new WorkerGlobalScope()), 'worker illegal constructor');
  check(typeof self.when === 'function', 'inherited when exposed');
  check(self.when === eventTarget.prototype.when, 'inherited when identity');
  for (const value of chain.slice(0, 3)) {
    check(!Object.hasOwn(value, 'when'), 'when is inherited');
  }
  test('prototype extension', () => {
    const key = Symbol('EventTarget extension');
    eventTarget.prototype[key] = 17;
    try { check(self[key] === 17, 'EventTarget extensions reach global'); }
    finally { delete eventTarget.prototype[key]; }
  });
  test('global event delivery', () => {
    const values = [];
    const listener = event => values.push(event);
    self.addEventListener('worker-prototype-probe', listener);
    const first = new Event('worker-prototype-probe');
    self.dispatchEvent(first);
    check(values.length === 1 && values[0] === first, 'global dispatch reaches listener');
    check(first.target === self, 'dispatched event target is global');
    self.removeEventListener('worker-prototype-probe', listener);
    self.dispatchEvent(new Event('worker-prototype-probe'));
    check(values.length === 1, 'global listener removal still works');
  });
  test('inherited observable events', () => {
    const controller = new AbortController();
    const values = [];
    const observable = self.when('worker-prototype-probe');
    check(observable instanceof Observable, 'when returns Observable');
    observable.subscribe(event => values.push(event), {signal: controller.signal});
    const first = new Event('worker-prototype-probe');
    self.dispatchEvent(first);
    check(values.length === 1 && values[0] === first, 'when receives global dispatch');
    check(first.target === self, 'dispatched event target is global');
    controller.abort();
    self.dispatchEvent(new Event('worker-prototype-probe'));
    check(values.length === 1, 'abort removes global listener');
  });
  test('global error reporting', () => {
    const errors = [];
    const marker = new Error('inspector abort');
    const controller = new AbortController();
    self.when('error').take(1).subscribe(event => {
      errors.push(event.error);
      event.preventDefault();
    });
    new Observable(subscriber => subscriber.next(1))
      .inspect({abort() { throw marker; }})
      .subscribe(() => controller.abort(), {signal: controller.signal});
    check(errors.length === 1 && errors[0] === marker, 'when receives reported callback error');
  });
  test('receiver branding', () => {
    let conversions = 0;
    let traps = 0;
    const type = {toString() { conversions++; return 'probe'; }};
    const revoked = Proxy.revocable(self, {});
    revoked.revoke();
    for (const receiver of [{}, Object.create(self), new Proxy(self, {
      get() { traps++; throw new Error('author proxy trap'); },
      getPrototypeOf() { traps++; throw new Error('author proxy trap'); }
    }), revoked.proxy]) {
      check(throwsTypeError(() => eventTarget.prototype.addEventListener.call(receiver, type, () => {})), 'addEventListener rejects forged/proxy receiver');
      check(throwsTypeError(() => eventTarget.prototype.when.call(receiver, type)), 'when rejects forged/proxy receiver');
    }
    check(conversions === 0, 'brand checked before conversion');
    check(traps === 0, 'brand checked without proxy traps');
  });
  test('global constructor replacement', () => {
    self.EventTarget = function ReplacedEventTarget() {};
    try {
      check(Object.getPrototypeOf(WorkerGlobalScope.prototype) === eventTarget.prototype, 'prototype keeps intrinsic EventTarget');
      check(Object.getPrototypeOf(WorkerGlobalScope) === eventTarget, 'constructor keeps intrinsic EventTarget');
      check(self.when === eventTarget.prototype.when, 'when keeps intrinsic method');
    } finally { self.EventTarget = eventTarget; }
  });
  return {checks, failures};
})
