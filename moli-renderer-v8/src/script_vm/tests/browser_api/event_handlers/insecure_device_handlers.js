(() => {
  const names=['ondevicemotion','ondeviceorientation','ondeviceorientationabsolute'];
  const other=document.getElementById('child').contentWindow, rows=[];
  function check(name,expected,run) {
    let actual;try{actual=run();}catch(error){actual={error:error.name,message:error.message};}
    rows.push({name,expected,actual,pass:JSON.stringify(actual)===JSON.stringify(expected)});
  }
  function openFrom(owner, base) {
    owner.document.open();
    owner.document.write((base ? '<base href="'+base+'">' : '') + '<script>window.createdPopup = window.open();<\/script>');
    owner.document.close();
    const child = owner.createdPopup;
    delete owner.createdPopup;
    return child;
  }
  const popup=open(), childPopup=openFrom(other), nestedPopup=openFrom(popup);
  for(const [owner,realm] of [['main',globalThis],['child',other],['popup',popup],['childPopup',childPopup],['nestedPopup',nestedPopup]]) {
    if(owner==='main'||owner==='child')check(owner+'/secure',false,()=>realm.isSecureContext);
    for(const name of names)check(owner+'/'+name+'/absent',false,()=>name in realm);
  }
  for(const name of names)check(name+'/child-document-open-expando',[true,0,0],()=>{
    const frame=document.createElement('iframe');document.body.appendChild(frame);
    try {
      const w=frame.contentWindow;let nativeCalls=0,authorCalls=0;
      const expando=()=>authorCalls++;w[name]=expando;w.onload=()=>nativeCalls++;
      w.document.open();w.document.close();
      w.dispatchEvent(new w.Event(name.slice(2)));w.dispatchEvent(new w.Event('load'));
      return [w[name]===expando,authorCalls,nativeCalls];
    } finally {frame.remove();}
  });
  nestedPopup.close();childPopup.close();popup.close();
  check('root-document-open-expando',[true,true,true,0,0],()=>{
    let authorCalls=0,nativeCalls=0;const expando=()=>authorCalls++;
    for(const name of names)globalThis[name]=expando;onload=()=>nativeCalls++;
    document.open();document.close();
    for(const name of names)dispatchEvent(new Event(name.slice(2)));
    dispatchEvent(new Event('load'));
    return [...names.map(name=>globalThis[name]===expando),authorCalls,nativeCalls];
  });
  check('insecure-state-ignores-author-property-and-base', [false,false,false], () => {
    const descriptor=Object.getOwnPropertyDescriptor(window,'isSecureContext');
    Object.defineProperty(window,'isSecureContext',{configurable:true,value:true});
    let target;
    try {
      target=openFrom(window,'https://secure-device-base.test/');
      return names.map(name=>name in target);
    } finally {
      if(target)target.close();
      Object.defineProperty(window,'isSecureContext',descriptor);
    }
  });
  const failures=rows.filter(r=>!r.pass);
  globalThis.__nodeReplacementResults={total:rows.length,passed:rows.length-failures.length,failures,rows};
  return failures.length===0;
})()
