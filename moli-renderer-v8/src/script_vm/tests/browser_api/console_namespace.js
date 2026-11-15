(realm) => {
  const {Object, Reflect, Symbol, console} = realm;
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const descriptor = Object.getOwnPropertyDescriptor(realm, 'console');
  assert(descriptor.value === console && descriptor.writable &&
         descriptor.configurable && !descriptor.enumerable, 'global descriptor');
  const prototype = Object.getPrototypeOf(console);
  assert(Reflect.ownKeys(prototype).length === 0, 'empty console prototype');
  assert(Object.getPrototypeOf(prototype) === Object.prototype, 'realm intrinsic prototype');
  assert(!('Console' in realm), 'no interface constructor');
  for (const name of ['assert', 'clear', 'debug', 'error', 'info', 'log', 'table',
                       'trace', 'warn', 'dir', 'dirxml', 'count', 'countReset',
                       'group', 'groupCollapsed', 'groupEnd', 'time', 'timeLog', 'timeEnd']) {
    const property = Object.getOwnPropertyDescriptor(console, name);
    assert(property && typeof property.value === 'function' && property.writable &&
           property.configurable && property.enumerable, `${name} descriptor`);
    assert(property.value.name === name && property.value.length === 0, `${name} metadata`);
    assert(Object.getPrototypeOf(property.value) === realm.Function.prototype, `${name} realm`);
  }
  const tag = Object.getOwnPropertyDescriptor(console, Symbol.toStringTag);
  assert(tag && tag.value === 'console' && !tag.writable &&
         tag.configurable && !tag.enumerable, 'namespace class string descriptor');
  assert(Object.prototype.toString.call(console) === '[object console]', 'class string');
  try {
    Object.defineProperty(console, Symbol.toStringTag, {value: 'custom'});
    assert(Object.prototype.toString.call(console) === '[object custom]', 'replaceable class string');
    assert(Reflect.deleteProperty(console, Symbol.toStringTag), 'deletable class string');
    assert(Object.prototype.toString.call(console) === '[object Object]', 'deleted class string');
  } finally {
    Object.defineProperty(console, Symbol.toStringTag, tag);
  }
  return true;
}
