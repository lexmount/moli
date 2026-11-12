(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  if (typeof window === 'undefined') {
    assert(typeof External === 'undefined' && typeof external === 'undefined', 'Window-only exposure');
    return 'worker-ok';
  }
  const object = window.external, prototype = External.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(window, 'external');
  assert(object === window.external, 'SameObject');
  assert(object instanceof External && Object.getPrototypeOf(object) === prototype, 'native interface');
  assert(Object.prototype.toString.call(object) === '[object External]', 'toStringTag');
  assert(Object.getOwnPropertyNames(object).length === 0, 'no own implementation fields');
  assert(Object.getOwnPropertyNames(prototype).join(',') === 'AddSearchProvider,IsSearchProviderInstalled,constructor', 'prototype surface');
  assert(descriptor.enumerable && descriptor.configurable && typeof descriptor.get === 'function' && typeof descriptor.set === 'function', 'replaceable accessor');
  const throwsTypeError = (callback, message) => {
    let caught;
    try { callback(); } catch (error) { caught = error; }
    assert(caught instanceof TypeError, message);
  };
  throwsTypeError(() => External(), 'illegal call');
  throwsTypeError(() => new External(), 'illegal constructor');
  throwsTypeError(() => descriptor.get.call({}), 'illegal Window receiver');
  let coercions = 0;
  const extra = {[Symbol.toPrimitive]() { coercions++; return 'unused'; }};
  for (const name of ['AddSearchProvider', 'IsSearchProviderInstalled']) {
    const method = prototype[name], flags = Object.getOwnPropertyDescriptor(prototype, name);
    assert(method.name === name && method.length === 0, 'method signature');
    assert(flags.enumerable && flags.configurable && flags.writable, 'method descriptor');
    assert(Function.prototype.toString.call(method).includes('[native code]'), 'native method');
    assert(method.call(object, extra) === undefined, 'void no-op');
    for (const receiver of [null, {}, prototype, Object.create(prototype), Object.create(object), new Proxy(object, {})]) {
      throwsTypeError(() => method.call(receiver, extra), 'illegal External receiver');
    }
  }
  assert(coercions === 0, 'IDL declares no arguments');
  window.external = 42;
  const replaced = Object.getOwnPropertyDescriptor(window, 'external');
  assert(replaced.value === 42 && replaced.writable && replaced.enumerable && replaced.configurable, 'replacement data property');
  assert(descriptor.get.call(window) === object, 'replacement must not overwrite native cache');
  assert(delete window.external, 'deletable replacement');
  assert(typeof window.external === 'undefined', 'delete must not resurrect accessor');
  Object.defineProperty(window, 'external', descriptor);
  assert(window.external === object, 'restored accessor retains SameObject');
  return 'window-ok';
})()
