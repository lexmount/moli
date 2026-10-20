async function navigationTransitionState(base, targetKind) {
  const checks = [];
  const check = (name, run) => {
    try { checks.push({name, pass: !!run()}); }
    catch (error) { checks.push({name, pass: false, error: error.name}); }
  };
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  const frame = document.createElement('iframe');
  frame.src = base + '/common/blank.html';
  await new Promise(resolve => { frame.onload = resolve; document.body.appendChild(frame); });
  await tick();
  const win = targetKind === 'child' ? frame.contentWindow : window;
  const other = targetKind === 'child' ? window : frame.contentWindow;
  const nav = win.navigation;
  const activation = frame.contentWindow.navigation.activation;
  const activationOwner = frame.contentWindow;
  check('network navigation creates an activation', () => activation !== null);
  let release;
  const pending = new win.Promise(resolve => { release = resolve; });
  let destination;
  nav.addEventListener('navigate', event => {
    destination = event.destination;
    event.intercept({handler: () => pending});
  }, {once: true});
  const from = nav.currentEntry;
  const result = nav.navigate('#native-transition-state', {history: 'push'});
  const transition = nav.transition;
  check('navigation exposes its current transition', () => transition !== null && transition === nav.transition);
  check('transition keeps source and destination identities', () => transition.from === from && transition.to === destination);
  const original = {};
  for (const name of ['from','to','navigationType','committed','finished']) original[name] = transition[name];
  check('transition promises use the Navigation realm', () => original.committed instanceof win.Promise && original.finished instanceof win.Promise);
  check('transition finished differs from the navigate result promise', () => original.finished !== result.finished);
  const rows = [
    ['NavigationTransition',transition,win,other,['navigationType','from','to','committed','finished']],
    ['NavigationActivation',activation,activationOwner,window,['from','entry','navigationType']]
  ];
  for (const [className, object, owner, foreign, names] of rows) {
    const Constructor = owner[className], proto = Constructor.prototype;
    check(className+' prototype attributes follow IDL order',()=>Object.keys(proto).filter(name=>names.includes(name)).join(',')===names.join(','));
    const isError = (run, ErrorCtor) => {try {run();return false;}catch(error){return error instanceof ErrorCtor;}};
    const rejectedGetter = async (getter, value, realm) => {
      if(typeof getter!=='function')return false;
      let promise;
      try {promise=getter.call(value);}catch(error){return false;}
      if(!(promise instanceof realm.Promise))return false;
      return await promise.then(()=>false,error=>error instanceof realm.TypeError);
    };
    check(className+' has the owner prototype and tag', () => Object.getPrototypeOf(object)===proto && Object.prototype.toString.call(object)==='[object '+className+']');
    check(className+' has no own IDL fields or tag', () => !Object.hasOwn(object,Symbol.toStringTag) && Object.getOwnPropertyNames(object).length===0);
    check(className+' prototype has the standard tag descriptor', () => {const d=Object.getOwnPropertyDescriptor(proto,Symbol.toStringTag);return d.value===className && !d.writable && !d.enumerable && d.configurable;});
    check(className+' is an illegal constructor', () => isError(()=>new Constructor(),owner.TypeError));
    for (const name of names) {
      const label=className+'.'+name, d=Object.getOwnPropertyDescriptor(proto,name);
      const getter=d?.get, saved=object?.[name];
      check(label+' is a readonly prototype accessor',()=>typeof getter==='function' && d.set===undefined && d.enumerable && d.configurable && getter.name==='get '+name && getter.length===0 && !Object.hasOwn(object,name));
      const accepted=Reflect.set(object,name,null);
      if(accepted)object[name]=saved;
      check(label+' rejects assignment',()=>!accepted && object[name]===saved);
      const strict=isError(()=>{'use strict';object[name]=null;},TypeError);
      if(!strict)object[name]=saved;
      check(label+' rejects strict assignment',()=>strict);
      const revoked=Proxy.revocable(object,{});revoked.revoke();let traps=0;
      const proxy=new Proxy(object,{get(){traps++;throw 1;},getPrototypeOf(){traps++;throw 2;}});
      const invalid=[{},Object.create(proto),Object.create(object),proxy,revoked.proxy,null,undefined,1];
      const promiseAttribute=className==='NavigationTransition' && ['committed','finished'].includes(name);
      for(let i=0;i<invalid.length;i++) {
        const rejected=promiseAttribute ? await rejectedGetter(getter,invalid[i],owner) : typeof getter==='function' && isError(()=>getter.call(invalid[i]),owner.TypeError);
        check(label+' rejects receiver '+i,()=>rejected);
      }
      check(label+' brand check avoids author traps',()=>traps===0);
      check(label+' getter returns its associated value',()=>typeof getter==='function' && getter.call(object)===saved);
      const foreignGetter=Object.getOwnPropertyDescriptor(foreign[className].prototype,name)?.get;
      check(label+' genuine cross-realm receiver keeps identity',()=>typeof foreignGetter==='function' && foreignGetter.call(object)===saved);
      const foreignRejection=promiseAttribute ? await rejectedGetter(foreignGetter,proxy,foreign) : typeof foreignGetter==='function' && isError(()=>foreignGetter.call(proxy),foreign.TypeError);
      check(label+' errors belong to the borrowed getter realm',()=>foreignRejection);
    }
  }
  let publicReads=0;
  const descriptors={};
  for(const name of ['from','to','navigationType','committed','finished']) {
    descriptors[name]=Object.getOwnPropertyDescriptor(transition,name);
    Object.defineProperty(transition,name,{configurable:true,get(){publicReads++;return original[name];}});
    const getter=Object.getOwnPropertyDescriptor(win.NavigationTransition.prototype,name)?.get;
    check('native '+name+' getter ignores own shadow',()=>typeof getter==='function' && getter.call(transition)===original[name]);
  }
  release();
  await Promise.all([result.finished,original.finished]);
  check('settlement clears transition without reading author fields',()=>nav.transition===null && publicReads===0);
  for(const name of Object.keys(descriptors)) {
    if(descriptors[name])Object.defineProperty(transition,name,descriptors[name]);
    else delete transition[name];
  }
  check('retained transition preserves settled promises',()=>transition.finished===original.finished && transition.committed===original.committed);
  check('same-document navigation preserves activation',()=>activationOwner.navigation.activation===activation);
  const prior=Object.getOwnPropertyDescriptor(win,'NavigationTransition');let constructorReads=0;
  Object.defineProperty(win,'NavigationTransition',{configurable:true,get(){constructorReads++;throw new Error('public constructor');}});
  let second;
  nav.addEventListener('navigate',event=>event.intercept({handler(){second=nav.transition;Object.freeze(second);}}),{once:true});
  try {
    await nav.navigate('#native-transition-frozen',{history:'push'}).finished;
    check('factory uses intrinsic prototype after global tampering',()=>Object.getPrototypeOf(second)===Object.getPrototypeOf(transition) && constructorReads===0);
    check('frozen transition still settles and clears',()=>Object.isFrozen(second) && nav.transition===null && second.finished!==original.finished);
  } finally {
    if(prior)Object.defineProperty(win,'NavigationTransition',prior);else delete win.NavigationTransition;
    frame.remove();
  }
  return {checks,passed:checks.filter(row=>row.pass).length,total:checks.length};
}
