// Idle/capability contract only; this does not test queued-input reporting.
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  if (typeof window === 'undefined') {
    assert(typeof Scheduling === 'undefined', 'Scheduling must not be exposed in workers');
    assert(!('scheduling' in navigator), 'WorkerNavigator must not expose scheduling');
    return 'worker-ok';
  }
  const scheduling = navigator.scheduling;
  assert(typeof Scheduling === 'function', 'Scheduling interface');
  assert(scheduling instanceof Scheduling, 'Scheduling brand');
  assert(Object.getPrototypeOf(scheduling) === Scheduling.prototype, 'Scheduling prototype');
  assert(Object.prototype.toString.call(scheduling) === '[object Scheduling]', 'Scheduling tag');
  assert(navigator.scheduling === scheduling, 'cached per-Navigator object');
  assert(Object.getOwnPropertyNames(scheduling).length === 0, 'no own implementation fields');
  assert(!Object.hasOwn(navigator, 'scheduling'), 'Navigator prototype accessor');
  const descriptor = Object.getOwnPropertyDescriptor(Navigator.prototype, 'scheduling');
  assert(descriptor.enumerable && descriptor.configurable && !descriptor.set, 'readonly accessor flags');
  assert(descriptor.get.call(navigator) === scheduling, 'borrowed getter');
  const method = Scheduling.prototype.isInputPending;
  const methodDescriptor = Object.getOwnPropertyDescriptor(Scheduling.prototype, 'isInputPending');
  assert(methodDescriptor.enumerable && methodDescriptor.configurable && methodDescriptor.writable, 'method flags');
  assert(method.length === 0 && method.name === 'isInputPending', 'method signature');
  assert(Function.prototype.toString.call(method).includes('[native code]'), 'native method');
  const throwsTypeError = (callback, message) => {
    let caught;
    try { callback(); } catch (error) { caught = error; }
    assert(caught instanceof TypeError, message);
  };
  throwsTypeError(() => new Scheduling(), 'illegal constructor');
  throwsTypeError(() => Scheduling(), 'illegal function call');
  for (const receiver of [null, {}, Scheduling.prototype, Object.create(Scheduling.prototype), new Proxy(scheduling, {})]) {
    throwsTypeError(() => method.call(receiver), 'illegal method receiver');
  }
  throwsTypeError(() => descriptor.get.call({}), 'illegal Navigator receiver');
  for (const options of [undefined, null, {}, {includeContinuous:false}, {includeContinuous:true}, () => {}]) {
    assert(method.call(scheduling, options) === false, 'idle result');
  }
  for (const options of [1, true, 'x', Symbol('x')]) {
    throwsTypeError(() => method.call(scheduling, options), 'dictionary conversion');
  }
  const accesses = [];
  const options = new Proxy({}, {get(_target, key) { accesses.push(key); return {valueOf() { throw new Error('no coercion'); }}; }});
  assert(method.call(scheduling, options) === false, 'truthy includeContinuous');
  assert(accesses.join(',') === 'includeContinuous', 'only read the declared dictionary member');
  const sentinel = new Error('options getter');
  let caught;
  try { method.call(scheduling, {get includeContinuous() { throw sentinel; }}); } catch (error) { caught = error; }
  assert(caught === sentinel, 'preserve dictionary getter exception');
  let reads = 0;
  throwsTypeError(() => method.call({}, {get includeContinuous() { reads++; }}), 'brand precedes conversion');
  assert(reads === 0, 'invalid receiver must not read options');
  return 'window-ok';
})()
