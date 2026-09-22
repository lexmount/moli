(mode => {
  const check = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  let constructorReads = 0;
  const replacement = mode === 'function' ? function Fake() {} : 123;
  const poison = () => {
    constructorReads++;
    throw new Error('public IDBFactory getter must not run');
  };
  // Do not read IDBFactory, its descriptor, or indexedDB before the override:
  // doing so could materialize the intrinsic and hide a first-use regression.
  if (mode === 'getter') {
    Object.defineProperty(globalThis, 'IDBFactory', {
      get: poison,
      configurable: false
    });
  } else if (mode === 'delete') {
    check(delete globalThis.IDBFactory, 'constructor deletion');
  } else {
    globalThis.IDBFactory = replacement;
  }

  const factory = globalThis.indexedDB;
  check(typeof factory === 'object' && factory !== null, 'factory exists');
  const prototype = Object.getPrototypeOf(factory);
  const intrinsic = prototype.constructor;
  check(intrinsic !== replacement && intrinsic.name === 'IDBFactory', 'intrinsic constructor');
  check(prototype === intrinsic.prototype, 'intrinsic prototype');
  check(factory instanceof intrinsic, 'factory instance');
  check(Object.prototype.toString.call(factory) === '[object IDBFactory]', 'factory tag');
  check(factory.cmp(1, 2) === -1 && factory.cmp(2, 2) === 0, 'native cmp works');
  for (const name of ['open', 'deleteDatabase', 'databases']) {
    check(typeof factory[name] === 'function', name + ' method');
  }
  check(globalThis.indexedDB === factory, 'SameObject');
  check(constructorReads === 0, 'no public constructor reads');

  const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'IDBFactory');
  if (mode === 'getter') {
    check(descriptor.get === poison && !descriptor.configurable, 'getter preserved');
  } else if (mode === 'delete') {
    check(descriptor === undefined, 'deleted constructor stays absent');
  } else {
    check(descriptor.value === replacement, 'replacement preserved');
  }
  return 'ok';
})
