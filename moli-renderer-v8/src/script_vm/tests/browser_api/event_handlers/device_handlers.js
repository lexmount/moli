(() => {
  const names = ['ondevicemotion', 'ondeviceorientation', 'ondeviceorientationabsolute'];
  const other = document.getElementById('child').contentWindow;
  function openFrom(owner, base) {
    owner.document.open();
    owner.document.write((base ? '<base href="'+base+'">' : '') + '<script>window.createdPopup = window.open();<\/script>');
    owner.document.close();
    const child = owner.createdPopup;
    delete owner.createdPopup;
    return child;
  }
  const popup = open();
  const childPopup = openFrom(other);
  const nestedPopup = openFrom(popup);
  const owners = [['main', globalThis], ['child', other], ['popup', popup], ['childPopup', childPopup], ['nestedPopup', nestedPopup]];
  const rows = [];
  function check(name, expected, run) {
    let actual;
    try {actual=run();} catch(error) {actual={error:error.name,message:error.message};}
    rows.push({name,expected,actual,pass:JSON.stringify(actual)===JSON.stringify(expected)});
  }
  for (const [owner, realm] of owners) for (const name of names) {
    const type=name.slice(2), label=owner+'/'+name;
    check(label+'/descriptor', ['function','function',true,true,null], () => {
      const d=Object.getOwnPropertyDescriptor(realm,name);
      return [typeof d.get,typeof d.set,d.enumerable,d.configurable,realm[name]];
    });
    check(label+'/only-window', [false,false,false], () =>
      ['body','frameset','div'].map(tag => name in realm.document.createElement(tag)));
    check(label+'/registration-order', [['before','replacement','after'],['before','after'],['before','after','first']], () => {
      const log=[], first=function(){log.push('first');}, replacement=function(){log.push('replacement');};
      const before=()=>log.push('before'), after=()=>log.push('after');
      realm.addEventListener(type,before); realm[name]=first; realm.addEventListener(type,after); realm[name]=replacement;
      try {
        realm.dispatchEvent(new Event(type)); const one=log.splice(0);
        realm[name]=null; realm.dispatchEvent(new Event(type)); const two=log.splice(0);
        realm[name]=first; realm.dispatchEvent(new Event(type)); return [one,two,log.splice(0)];
      } finally {realm[name]=null;realm.removeEventListener(type,before);realm.removeEventListener(type,after);}
    });
    check(label+'/callback-this-and-cancel', [true,true,true], () => {
      let receiver,seen;const event=new Event(type,{cancelable:true});
      const callback=function(e){receiver=this;seen=e;return false;};realm[name]=callback;
      try {const accepted=realm.dispatchEvent(event);return [!accepted,receiver===realm,seen===event];}
      finally {realm[name]=null;}
    });
  }
  for (const [sourceName, source] of owners) for (const [targetName, target] of owners) for (const name of names) {
    check(sourceName+'->'+targetName+'/'+name+'/borrow', true, () => {
      const descriptor=Object.getOwnPropertyDescriptor(source,name), callback=()=>{};
      try {
        descriptor.set.call(target,callback);
        return descriptor.get.call(target)===callback && target[name]===callback;
      } finally {target[name]=null;}
    });
  }
  for (const [owner, realm] of owners) for (const name of names) {
    check(owner+'/'+name+'/native-receiver',true,()=>{
      const descriptor=Object.getOwnPropertyDescriptor(realm,name);
      const TypeErrorCtor=descriptor.get.constructor('return TypeError')();
      let traps=0;
      const trap=()=>{++traps;throw new Error('proxy trap');};
      const revoked=Proxy.revocable(realm,{});revoked.revoke();
      for(const receiver of [{},Object.create(realm),new Proxy(realm,{get:trap,getPrototypeOf:trap}),revoked.proxy]) {
        for(const op of ['get','set']) {
          try {descriptor[op].call(receiver,()=>{});return false;}
          catch(error){if(!(error instanceof TypeErrorCtor))return false;}
        }
      }
      return traps===0;
    });
  }
  for (const name of names) for (const mode of ['native','deleted','replaced']) {
    check(name+'/document-open/'+mode, [true,true,0,0], () => {
      const frame=document.createElement('iframe');document.body.appendChild(frame);
      try {
        const w=frame.contentWindow,d=w.document,get=Object.getOwnPropertyDescriptor(w,name).get;
        let callbacks=0,writes=0;
        w[name]=()=>callbacks++;w.addEventListener(name.slice(2),()=>callbacks++);
        const setter=()=>writes++;
        if(mode==='deleted')delete w[name];
        if(mode==='replaced')Object.defineProperty(w,name,{configurable:true,set:setter});
        d.open();d.close();w.dispatchEvent(new w.Event(name.slice(2)));
        const preserved=mode==='deleted' ? !Object.hasOwn(w,name) : mode==='replaced' ? Object.getOwnPropertyDescriptor(w,name).set===setter : w[name]===null;
        return [get.call(w)===null,preserved,callbacks,writes];
      } finally {frame.remove();}
    });
  }
  nestedPopup.close(); childPopup.close(); popup.close();
  check('secure-state-ignores-author-property-and-base', [true,true,true], () => {
    const descriptor=Object.getOwnPropertyDescriptor(window,'isSecureContext');
    Object.defineProperty(window,'isSecureContext',{configurable:true,value:false});
    let target;
    try {
      target=openFrom(window,'http://insecure-device-base.test/');
      return names.map(name=>typeof Object.getOwnPropertyDescriptor(target,name)?.get==='function');
    } finally {
      if(target)target.close();
      Object.defineProperty(window,'isSecureContext',descriptor);
    }
  });
  const failures=rows.filter(r=>!r.pass);
  globalThis.__nodeReplacementResults={total:rows.length,passed:rows.length-failures.length,failures,rows};
  return failures.length===0;
})()
