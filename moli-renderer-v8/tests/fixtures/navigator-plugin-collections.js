(() => {
  const checks = [];
  const check = (name, run) => {
    try { checks.push({name, pass: !!run()}); }
    catch (error) { checks.push({name, pass: false, error: error.name}); }
  };
  const typeError = run => { try { run(); return false; } catch (error) { return error instanceof TypeError; } };
  if (typeof document === 'undefined') {
    for (const name of ['Plugin', 'PluginArray', 'MimeType', 'MimeTypeArray'])
      check(`${name} is Window-only`, () => !(name in self));
    check('worker navigator has no plugin collections', () => !('plugins' in navigator) && !('mimeTypes' in navigator));
    return {checks, passed: checks.filter(c => c.pass).length, total: checks.length};
  }
  const plugins = navigator.plugins, mimeTypes = navigator.mimeTypes;
  const plugin = plugins[0], mime = mimeTypes[0];
  check('navigator collections are SameObject', () => plugins === navigator.plugins && mimeTypes === navigator.mimeTypes);
  check('one shared MIME list belongs to the Window', () => Array.from(plugins).every(p => p[0] === mimeTypes[0] && p[1] === mimeTypes[1]));
  check('every MIME has the generic enabledPlugin', () => Array.from(plugins).every(p => Array.from(p).every(m => m.enabledPlugin === plugin)));
  const attributes = [[Plugin, plugin, ['name','description','filename','length']], [MimeType, mime, ['type','description','suffixes','enabledPlugin']], [PluginArray, plugins, ['length']], [MimeTypeArray, mimeTypes, ['length']]];
  for (const [Constructor, receiver, names] of attributes) {
    for (const name of names) {
      const prefix = `${Constructor.name}.${name}`;
      const descriptor = Object.getOwnPropertyDescriptor(Constructor.prototype, name);
      check(prefix+' is a readonly prototype accessor', () => typeof descriptor?.get === 'function' && descriptor.set === undefined && descriptor.enumerable && descriptor.configurable && !Object.hasOwn(receiver, name) && descriptor.get.name === 'get '+name && descriptor.get.length === 0);
      const before = receiver[name];
      const accepted = Reflect.set(receiver, name, null);
      if (accepted) receiver[name] = before;
      check(prefix+' rejects assignment', () => !accepted);
      const strictRejected=typeError(() => { 'use strict'; receiver[name] = null; });
      if (!strictRejected) receiver[name]=before;
      check(prefix+' rejects strict assignment', () => strictRejected);
      const revoked = Proxy.revocable(receiver, {});revoked.revoke();
      for (const [kind, fake] of [['plain',{}],['forged',Object.create(Constructor.prototype)],['inherited',Object.create(receiver)],['proxy',new Proxy(receiver,{})],['revoked',revoked.proxy]]) {
        check(prefix+' rejects '+kind, () => typeof descriptor?.get === 'function' && typeError(() => descriptor.get.call(fake)));
      }
    }
  }
  for (const [name, collection, supported] of [['PluginArray',plugins,'PDF Viewer'], ['MimeTypeArray',mimeTypes,'application/pdf'], ['Plugin',plugin,'application/pdf']]) {
    const value = collection.item(0), proto = Object.getPrototypeOf(collection);
    const indexDescriptor=Object.getOwnPropertyDescriptor(collection,'0'), nameDescriptor=Object.getOwnPropertyDescriptor(collection,supported);
    check(name+' native item ignores ordinary names', () => collection.namedItem('length') === null && collection.namedItem('item') === null && collection.namedItem('toString') === null && collection.namedItem('0') === null);
    check(name+' applies unsigned-long index conversion', () => collection.item(2**32) === value && collection.item(-1) === null);
    check(name+' exposes readonly configurable indexed descriptors', () => {const d=Object.getOwnPropertyDescriptor(collection,'0');return d.value===value && !d.writable && d.enumerable && d.configurable;});
    check(name+' exposes unenumerable readonly named descriptors', () => {const d=Object.getOwnPropertyDescriptor(collection,supported);return d.value===value && !d.writable && !d.enumerable && d.configurable;});
    const setIndex=Reflect.set(collection,'0',null);
    if(setIndex)Reflect.set(collection,'0',value);
    check(name+' rejects supported index writes', () => !setIndex && collection[0]===value);
    const strictIndex=typeError(() => { 'use strict'; collection[0]=null; });
    if(!strictIndex)Reflect.set(collection,'0',value);
    check(name+' rejects strict index writes', () => strictIndex);
    const setMissing=Reflect.set(collection,'99',null);
    if(setMissing)Reflect.deleteProperty(collection,'99');
    check(name+' rejects unsupported index writes', () => !setMissing && !Object.hasOwn(collection,'99'));
    const defineIndex=Reflect.defineProperty(collection,'0',{value}), defineMissing=Reflect.defineProperty(collection,'99',{value,configurable:true});
    if(defineMissing)Reflect.deleteProperty(collection,'99');
    check(name+' rejects indexed definitions', () => !defineIndex && !defineMissing);
    const deleteIndex=Reflect.deleteProperty(collection,'0');
    if(deleteIndex)Reflect.defineProperty(collection,'0',indexDescriptor);
    check(name+' rejects index deletion', () => !deleteIndex && collection[0]===value);
    check(name+' accepts absent-index deletion', () => Reflect.deleteProperty(collection,'99'));
    const setName=Reflect.set(collection,supported,null);
    if(setName)Reflect.set(collection,supported,value);
    check(name+' rejects supported name writes', () => !setName && collection.namedItem(supported)===value);
    check(name+' rejects supported name definition', () => !Reflect.defineProperty(collection,supported,{value}));
    const deleteName=Reflect.deleteProperty(collection,supported);
    if(deleteName)Reflect.defineProperty(collection,supported,nameDescriptor);
    check(name+' rejects supported name deletion', () => !deleteName && collection[supported]===value);
    const prior=Object.getOwnPropertyDescriptor(proto,supported);const token={};
    try {
      Object.defineProperty(proto,supported,{value:token,configurable:true});
      check(name+' prototype properties hide named entries', () => collection[supported]===token && Object.getOwnPropertyDescriptor(collection,supported)===undefined && !Object.getOwnPropertyNames(collection).includes(supported));
      check(name+' namedItem ignores prototype shadowing', () => collection.namedItem(supported)===value);
    } finally { if(prior)Object.defineProperty(proto,supported,prior);else delete proto[supported]; }
    check(name+' deleting prototype shadow reveals native entry', () => collection[supported]===value);
    let getterCalls=0;
    const priorIndex=Object.getOwnPropertyDescriptor(proto,'99');
    try {
      Object.defineProperty(proto,'99',{get(){getterCalls++;return token;},configurable:true});
      check(name+' item ignores out-of-range prototype getters', () => collection.item(99)===null && getterCalls===0);
    } finally { if(priorIndex)Object.defineProperty(proto,'99',priorIndex);else delete proto[99]; }
    let conversions=0;const input={toString(){conversions++;return supported;},valueOf(){conversions++;return 0;}};
    check(name+' validates receivers before argument conversion', () => typeError(()=>proto.item.call({},input)) && typeError(()=>proto.namedItem.call(new Proxy(collection,{}),input)) && conversions===0);
    check(name+' methods support genuine inherited-function calls', () => proto.item.call(collection,0)===value && proto.namedItem.call(collection,supported)===value);
    const symbol=Symbol('author');
    check(name+' allows ordinary string and symbol expandos', () => Reflect.defineProperty(collection,'00',{value:token,configurable:true}) && Reflect.defineProperty(collection,symbol,{value:token,configurable:true}) && collection['00']===token && collection[symbol]===token);
    delete collection['00'];delete collection[symbol];
    const derived = Object.create(collection);
    check(name+' permits a derived receiver to shadow a supported name', () => Reflect.set(collection,supported,token,derived) && Object.hasOwn(derived,supported) && derived[supported]===token && collection.namedItem(supported)===value);
    check(name+' rejects indexed writes even with a derived receiver', () => !Reflect.set(collection,'0',token,derived) && !Object.hasOwn(derived,'0'));
    let writes=0, receiver;
    let receivedValue;const inheritedSetter={set(v){writes++;receiver=this;receivedValue=v;},configurable:true};
    try {
      Object.defineProperty(proto,supported,inheritedSetter);
      check(name+' uses inherited setters for a supported name', () => Reflect.set(collection,supported,token) && writes===1 && receiver===collection && receivedValue===token && collection.namedItem(supported)===value);
      check(name+' rejects supported definitions while hidden by a setter', () => !Reflect.defineProperty(collection,supported,{value:token}));
      check(name+' deleting a hidden supported name succeeds', () => Reflect.deleteProperty(collection,supported));
    } finally { if(prior)Object.defineProperty(proto,supported,prior);else delete proto[supported]; }
    try {
      Object.defineProperty(proto,'99',inheritedSetter);
      check(name+' permits an inherited setter at an unsupported index', () => Reflect.set(collection,'99',token) && writes===2 && receiver===collection && receivedValue===token && !Object.hasOwn(collection,'99'));
    } finally { if(priorIndex)Object.defineProperty(proto,'99',priorIndex);else delete proto[99]; }
    check(name+' treats the uint32 maximum as an ordinary name', () => Reflect.defineProperty(collection,'4294967295',{value:token,configurable:true}) && collection['4294967295']===token && Reflect.deleteProperty(collection,'4294967295'));
    const oldPrototype=Object.getPrototypeOf(collection);
    let proxySets=0;
    try {
      Object.setPrototypeOf(collection,new Proxy(proto,{set(target,key,v,receiver){proxySets++;return key===supported && v===token && receiver===collection;}}));
      check(name+' preserves Proxy prototype Set behavior',()=>Reflect.set(collection,supported,token) && proxySets===1);
    } finally { Object.setPrototypeOf(collection,oldPrototype); }
    const beforeZero=Object.getOwnPropertyDescriptor(proto,'0');
    try {
      Object.defineProperty(proto,'0',{value:token,configurable:true});
      check(name+' native indices mask prototype properties',()=>collection[0]===value && collection.item(0)===value);
    } finally { if(beforeZero)Object.defineProperty(proto,'0',beforeZero);else delete proto[0]; }
    try {
      Object.setPrototypeOf(collection,null);
      check(name+' readonly indices survive a null prototype', () => !Reflect.set(collection,'0',token) && !Reflect.set(collection,'99',token));
      check(name+' allows ordinary expandos with a null prototype', () => Reflect.set(collection,'author',token) && collection.author===token && Reflect.deleteProperty(collection,'author'));
    } finally { Object.setPrototypeOf(collection,oldPrototype); }
    check(name+' rejects preventExtensions', () => !Reflect.preventExtensions(collection) && Object.isExtensible(collection));
  }
  const iframe=document.createElement('iframe');document.body.appendChild(iframe);
  try {
    const other=iframe.contentWindow;
    let trapCalls=0;
    const trap=()=>{trapCalls++;return null;};
    try {
      Object.defineProperty(other.Object.prototype,'get',{value:trap,configurable:true});
      Object.defineProperty(other.Object.prototype,'set',{value:trap,configurable:true});
      const foreignPlugins=other.navigator.plugins, foreignMimeTypes=other.navigator.mimeTypes;
      check('lazy native graph ignores author Proxy traps on Object.prototype',()=>foreignPlugins[0].name==='PDF Viewer' && foreignMimeTypes[0].type==='application/pdf' && trapCalls===0);
      check('internal property descriptors ignore author get/set pollution',()=>{const d=Object.getOwnPropertyDescriptor(foreignPlugins,'0');return d.value===foreignPlugins[0] && !d.writable && trapCalls===0;});
    } finally { delete other.Object.prototype.get;delete other.Object.prototype.set; }
    for (const [name,local,foreign] of [['PluginArray',plugins,other.navigator.plugins],['MimeTypeArray',mimeTypes,other.navigator.mimeTypes],['Plugin',plugin,other.navigator.plugins[0]],['MimeType',mime,other.navigator.mimeTypes[0]]]) {
      const attribute=name==='MimeType'?'type':'length';
      const localGetter=Object.getOwnPropertyDescriptor(globalThis[name].prototype,attribute)?.get;
      const foreignGetter=Object.getOwnPropertyDescriptor(other[name].prototype,attribute)?.get;
      check(name+' cross-realm genuine receivers work both ways',()=>localGetter.call(foreign)===foreign[attribute] && foreignGetter.call(local)===local[attribute]);
      check(name+' illegal receiver errors belong to the callee realm',()=>{try{foreignGetter.call(new Proxy(local,{}));return false;}catch(error){return error instanceof other.TypeError && !(error instanceof TypeError);}});
      check(name+' objects stay in their own Window',()=>Object.getPrototypeOf(foreign)===other[name].prototype && Object.getPrototypeOf(local)===globalThis[name].prototype && local!==foreign);
    }
    check('different Windows have separate MIME object graphs',()=>other.navigator.plugins[0][0]===other.navigator.mimeTypes[0] && other.navigator.mimeTypes[0]!==mime);
  } finally { iframe.remove(); }
  return {checks, passed: checks.filter(c => c.pass).length, total: checks.length};
})()
