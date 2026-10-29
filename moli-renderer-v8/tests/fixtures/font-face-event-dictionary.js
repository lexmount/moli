(() => {
  const assert = (ok,label) => {if (!ok) throw Error(label);};
  const sentinel={};
  const members=['bubbles','cancelable','composed','fontfaces'];
  const reads=[];
  new FontFaceSetLoadEvent('audit',new Proxy({}, {get(_target,key){reads.push(key);}}));
  assert(reads.join()===members.join(),'FontFaceSetLoadEvent inherited member order');
  for (const stop of members) {
    reads.length=0;
    let error;
    try {new FontFaceSetLoadEvent('audit',new Proxy({}, {get(_target,key) {
      reads.push(key);if (key===stop) throw sentinel;
    }}));} catch(caught) {error=caught;}
    assert(error===sentinel && reads.join()===members.slice(0,members.indexOf(stop)+1).join(),
      stop+' original exception and short circuit');
  }
  let next=0,error;
  try {new FontFaceSetLoadEvent('audit',{fontfaces:{[Symbol.iterator](){return {next(){
    if (next++) throw sentinel;
    return {done:false,value:{}};
  }};}}});}catch(caught){error=caught;}
  assert(error instanceof TypeError && next===1,'invalid interface rejected before the next iterator step');
  const face=new FontFace('audit','url(/not-loaded.ttf)');
  const source=new Set([face]);
  const event=new FontFaceSetLoadEvent('audit',{bubbles:true,composed:true,fontfaces:source});
  source.clear();
  assert(event.bubbles && event.composed && event.fontfaces.length===1 && event.fontfaces[0]===face &&
    Object.isFrozen(event.fontfaces),'frozen native identity snapshot');
  for(const value of [1,'init',true]) {
    let error;
    try{new FontFaceSetLoadEvent('audit',value);}catch(caught){error=caught;}
    assert(error instanceof TypeError,'primitive dictionary');
  }
  for(const value of [{},Object.create(FontFace.prototype),new Proxy(face,{})]) {
    let error;
    try{new FontFaceSetLoadEvent('audit',{fontfaces:[value]});}catch(caught){error=caught;}
    assert(error instanceof TypeError,'native interface identity');
  }
  // Snapshot array construction must not run author indexed setters.
  const previous=Object.getOwnPropertyDescriptor(Array.prototype,'0');
  let writes=0,snapshot;
  Object.defineProperty(Array.prototype,'0',{configurable:true,set(){writes++;throw sentinel;}});
  try{snapshot=new FontFaceSetLoadEvent('audit',{fontfaces:new Set([face])}).fontfaces;}
  finally {if(previous) Object.defineProperty(Array.prototype,'0',previous);else delete Array.prototype[0];}
  assert(writes===0 && snapshot[0]===face && Object.isFrozen(snapshot),'snapshot bypasses array prototype setters');
  return true;
})()
