function domExceptionStackBoundariesProbe(realm) {
  const rows = [], failures = [];
  const check = (name, actual, expected) => {
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  const entries = [
    ['DOMException', ['message', 'AbortError']],
    ['QuotaExceededError', ['full', {quota: 10, requested: 12}]],
    ['WebSocketError', ['closed']],
    ['WebTransportError', []],
  ];
  if (realm.document) entries.push(['RTCError', [{errorDetail: 'dtls-failure'}, 'rtc']], ['OverconstrainedError', ['width', 'constraint']]);
  const descriptor = Object.getOwnPropertyDescriptor(realm.Error, 'captureStackTrace');
  let captureReads = 0;
  Object.defineProperty(realm.Error, 'captureStackTrace', {configurable: true, get() {captureReads++; throw new Error('author capture lookup');}});
  try {
    for (const [name, args] of entries) {
      check(name + ':exposure', typeof realm[name], 'function');
      if (typeof realm[name] !== 'function') continue;
      const error = Reflect.construct(realm[name], args);
      check(name + ':script-stack', [Object.hasOwn(error, 'stack'), typeof error.stack], [false, 'undefined']);
      if (name === 'DOMException' || name === 'QuotaExceededError') {
        const copy = realm.structuredClone(error);
        check(name + ':clone-fields', [copy.name, copy.message, copy.code, Object.getPrototypeOf(copy) === realm[name].prototype], [error.name, error.message, error.code, true]);
        check(name + ':clone-stack', [Object.hasOwn(copy, 'stack'), typeof copy.stack], [false, 'undefined']);
        if (name === 'QuotaExceededError') check(name + ':clone-options', [copy.quota, copy.requested], [10, 12]);
      }
      const Derived = class extends realm[name] {};
      let stackReads = 0;
      Object.defineProperty(Derived.prototype, 'stack', {configurable: true, get() {stackReads++; return 'inherited stack';}});
      const derived = Reflect.construct(realm[name], args, Derived);
      check(name + ':subclass-initialization', [Object.getPrototypeOf(derived) === Derived.prototype, Object.hasOwn(derived, 'stack'), stackReads], [true, false, 0]);
      check(name + ':inherited-stack', [derived.stack, stackReads], ['inherited stack', 1]);
    }
    let cloneError;
    try {realm.structuredClone(Symbol('not cloneable'));} catch (error) {cloneError = error;}
    const stackDescriptor = Object.getOwnPropertyDescriptor(cloneError, 'stack');
    check('native-clone-error', [cloneError instanceof realm.DOMException, cloneError.name, typeof cloneError.stack], [true, 'DataCloneError', 'string']);
    check('native-stack-descriptor', [!!stackDescriptor, stackDescriptor?.enumerable, stackDescriptor?.configurable], [true, false, true]);
    const copiedFailure = realm.structuredClone(cloneError);
    check('cloned-native-error-data', [copiedFailure.name, Object.hasOwn(copiedFailure, 'stack'), typeof copiedFailure.stack], ['DataCloneError', false, 'undefined']);
    if (realm.document) {
      let selectorError;
      try {realm.document.querySelector('[');} catch (error) {selectorError = error;}
      check('native-selector-error', [selectorError instanceof realm.DOMException, selectorError.name, typeof selectorError.stack], [true, 'SyntaxError', 'string']);
    }
    const source = new realm.DOMException('original message', 'AbortError');
    let authorReads = 0;
    for (const name of ['name', 'message', 'stack']) Object.defineProperty(source, name, {enumerable: true, get() {authorReads++; throw new Error('author error field');}});
    const copies = realm.structuredClone([source, source]);
    check('clone-native-fields-only', [copies[0].name, copies[0].message, copies[0].code, authorReads], ['AbortError', 'original message', 20, 0]);
    check('clone-reference-identity', [copies[0] === copies[1], Object.hasOwn(copies[0], 'stack')], [true, false]);
    const jsError = new realm.TypeError('ordinary JS error');
    const jsCopy = realm.structuredClone(jsError);
    check('builtin-error-stack-retention', [jsCopy instanceof realm.TypeError, jsCopy.message, jsCopy.stack === jsError.stack], [true, 'ordinary JS error', true]);
    check('author-capture-reads', captureReads, 0);
  } finally {
    if (descriptor) Object.defineProperty(realm.Error, 'captureStackTrace', descriptor);
    else delete realm.Error.captureStackTrace;
  }
  return {rows, failures};
}
