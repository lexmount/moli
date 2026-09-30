(() => {
  const names=['ondevicemotion','ondeviceorientation','ondeviceorientationabsolute'];
  const realms=[window,document.getElementById('child').contentWindow];
  for(const w of realms) for(const name of names) {
    const d=Object.getOwnPropertyDescriptor(w,name);
    if(!w.isSecureContext){if(d)throw Error(name+': insecure exposure');continue;}
    if(!d || !d.enumerable || !d.configurable || typeof d.get!=='function' || typeof d.set!=='function' || w[name]!==null)throw Error(name+': descriptor');
    if(name in w.document || name in w.document.createElement('body'))throw Error(name+': Window only');
    let calls=0;w[name]=function(e){if(this!==w || e.type!==name.slice(2))throw Error(name+': receiver');calls++;return false;};
    if(w.dispatchEvent(new Event(name.slice(2),{cancelable:true})) || calls!==1)throw Error(name+': dispatch');
    const callback=()=>{};for(const other of realms){d.set.call(other,callback);if(d.get.call(other)!==callback || other[name]!==callback)throw Error(name+': borrowed accessor');other[name]=null;}
    for(const value of [{},Object.create(w),new Proxy(w,{})])for(const op of ['get','set']){let error;try{d[op].call(value,callback);}catch(e){error=e;}if(!error || error.name!=='TypeError')throw Error(name+': receiver brand');}
    w[name]=null;
  }
  return true;
})()
