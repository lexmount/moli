function nativeErrorBrandProbe(realm) {
  const rows = [], failures = [];
  const check = (name, actual, expected = true) => {
    rows.push({name, actual});
    if (actual !== expected) failures.push({name, actual, expected});
  };
  const isError = realm.Error.isError, DOM = realm.DOMException;
  check('intrinsic metadata', isError.name === 'isError' && isError.length === 1 && /\[native code\]/.test(realm.Function.prototype.toString.call(isError)));
  const entries = [
    ['DOMException', ['message','AbortError']],
    ['QuotaExceededError', ['full',{quota:0,requested:12.5}]],
    ['WebSocketError', ['closed']],
    ['WebTransportError', []],
  ];
  if (realm.document) entries.push(['RTCError',[{errorDetail:'dtls-failure'}]],['OverconstrainedError',['width']]);
  let authorReads = 0;
  const poison = () => { ++authorReads; throw new Error('author lookup'); };
  for (const [name,args] of entries) {
    const Constructor = realm[name];
    check(name+': exposed constructor',typeof Constructor === 'function');
    if (typeof Constructor !== 'function') continue;
    const value = Reflect.construct(Constructor,args);
    check(name+': script-created native brand',isError(value));
    const Derived = class extends Constructor {};
    check(name+': genuine subclass',isError(Reflect.construct(Constructor,args,Derived)));
    check(name+': forged prototype rejected',isError(Object.create(Constructor.prototype)),false);
    check(name+': inherited native identity rejected',isError(Object.create(value)),false);
    const proxy = new Proxy(value,{get:poison,getPrototypeOf:poison});
    check(name+': author Proxy rejected without traps',isError(proxy),false);
    const revoked = Proxy.revocable(value,{}); revoked.revoke();
    check(name+': revoked Proxy rejected',isError(revoked.proxy),false);
    Object.setPrototypeOf(value,null);
    for (const property of ['name','message','stack','constructor']) Object.defineProperty(value,property,{get:poison});
    Object.defineProperty(value,Symbol.toStringTag,{get:poison});
    check(name+': classification uses only native identity',isError(value));
    check(name+': this value is irrelevant',isError.call(null,value));
  }
  for (const [name,make] of [
    ['Error',()=>new realm.Error('ordinary')],
    ['TypeError',()=>new realm.TypeError('ordinary')],
    ['AggregateError',()=>new realm.AggregateError([])],
    ['WebAssembly.CompileError',()=>new realm.WebAssembly.CompileError('ordinary')],
  ]) {
    const value=make();
    check(name+': V8 ordinary Error brand retained',isError(value));
    check(name+': Proxy does not acquire ErrorData',isError(new Proxy(value,{})),false);
  }
  for (const [name,value] of [
    ['global',realm],
    ['null',null],['undefined',undefined],['number',1],['string','error'],
    ['Object',{}],['Error.prototype',realm.Error.prototype],
    ['DOMException.prototype',DOM.prototype],['Event',new realm.Event('test')],
    ['URL',new realm.URL('https://native-error.test/')],['Blob',new realm.Blob([])],
  ]) check(name+': not a native error',isError(value),false);
  if (realm.document) {
    check('Document is not an error',isError(realm.document),false);
    check('Element is not an error',isError(realm.document.createElement('div')),false);
  }
  let native;
  try {realm.structuredClone(Symbol());} catch(error) {native=error;}
  check('API-created DOMException',isError(native));
  const copy = realm.structuredClone(native);
  check('cloned API-created exception',isError(copy));
  check('cloned API-created exception has data without a stack',copy instanceof DOM && copy.name === 'DataCloneError' && !Object.hasOwn(copy,'stack'));
  const source = new DOM('message','AbortError');
  const cloned = realm.structuredClone([source,source]);
  check('data clone preserves native brand and alias identity',isError(cloned[0]) && cloned[0] === cloned[1] && cloned[0] instanceof DOM);
  const quota = realm.structuredClone(new realm.QuotaExceededError('full',{quota:0,requested:12.5}));
  check('quota clone preserves native brand and data',isError(quota) && quota instanceof realm.QuotaExceededError && quota.quota === 0 && quota.requested === 12.5 && !Object.hasOwn(quota,'stack'));
  const descriptor = Object.getOwnPropertyDescriptor(realm,'DOMException');
  try {
    Object.defineProperty(realm,'DOMException',{configurable:true,get:poison});
    check('public constructor does not supply the brand',isError(source));
    check('public constructor does not brand forged objects',isError(Object.create(DOM.prototype)),false);
  } finally {Object.defineProperty(realm,'DOMException',descriptor);}
  check('author getters and traps were not observed',authorReads,0);
  return {rows,failures};
}
