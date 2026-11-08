(async () => {
  const assert=(ok,label)=>{if(!ok) throw Error(label);};
  const members=['body','cache','credentials','duplex','headers','integrity','keepalive',
    'method','mode','priority','redirect','referrer','referrerPolicy','signal'];
  const sentinel={original:true};
  const trace=[];
  const options={method:'POST',body:'value'};
  for(const key of members) Object.defineProperty(options,key,{configurable:true,
    get(){trace.push(key);return key==='method'?'POST':key==='body'?'value':undefined;}});
  new Request('/index.html',options);
  assert(trace.join()===members.join(),'RequestInit complete lexical order: '+trace);
  for(const stop of members) {
    trace.length=0;
    const bad=Object.create(options);
    Object.defineProperty(bad,stop,{get(){trace.push(stop);throw sentinel;}});
    let error;
    try{new Request('/index.html',bad);}catch(caught){error=caught;}
    assert(error===sentinel && trace.join()===members.slice(0,members.indexOf(stop)+1).join(),
      stop+' original exception and short circuit');
    trace.length=0;
    let promise;
    try{promise=fetch('/index.html',bad);}catch(error){throw Error('fetch conversion threw synchronously');}
    error=undefined;try{await promise;}catch(caught){error=caught;}
    assert(error===sentinel && trace.join()===members.slice(0,members.indexOf(stop)+1).join(),
      'fetch/'+stop+' original rejection and single conversion');
  }
  for(const value of [1,true,'init',Symbol('init'),1n]) {
    let error;
    try{new Request('/index.html',value);}catch(caught){error=caught;}
    assert(error instanceof TypeError,'Request primitive dictionary');
    error=undefined;try{await fetch('/index.html',value);}catch(caught){error=caught;}
    assert(error instanceof TypeError,'fetch primitive dictionary');
  }
  const controller=new AbortController();controller.abort(sentinel);
  let gets=0;
  const hidden=new Proxy({signal:controller.signal},{has(){return false;},get(t,key){
    if(key==='signal') gets++;return Reflect.get(t,key);
  }});
  const request=new Request('/index.html',hidden);
  assert(gets===1 && request.signal!==controller.signal && request.signal.aborted &&
    request.signal.reason===sentinel,'signal uses Get once and preserves aborted state');
  let error;
  const noHas=new Proxy({signal:controller.signal},{has(t,key){
    if(['method','body','headers','signal'].includes(key)) throw sentinel;return Reflect.has(t,key);
  }});
  assert(new Request('/index.html',noHas).signal.aborted,'RequestInit does not use HasProperty');
  error=undefined;try{await fetch('/index.html',noHas);}catch(caught){error=caught;}
  assert(error===sentinel,'fetch ignores HasProperty traps and rejects with the abort reason');
  gets=0;try{await fetch('/index.html',hidden);}catch(caught){error=caught;}
  assert(gets===1 && error===sentinel,'fetch reads the same signal once and rejects with its reason');
  const inherited=new Request(request,{signal:undefined});
  const cleared=new Request(request,{signal:null});
  assert(inherited.signal.aborted && !cleared.signal.aborted,'signal missing/undefined/null distinction');
  let late=0;
  try{new Request('/index.html',{signal:{},get window(){late++;throw sentinel;}});}catch(caught){error=caught;}
  assert(error instanceof TypeError && late===0,'signal brand checked before later fields');
  try{new Request('http://%',{get method(){throw sentinel;}});}catch(caught){error=caught;}
  assert(error===sentinel,'dictionary conversion precedes URL parsing');
  const input=new Request('/index.html',{headers:{'x-snapshot':'before'}});
  const copy=new Request(input,{get signal(){input.headers.set('x-snapshot','after');return null;}});
  assert(copy.headers.get('x-snapshot')==='after','native Request state read after argument conversion');
  const bytes=new Uint8Array([65]);
  const body=new Request('/index.html',{body:bytes,get method(){bytes[0]=66;return 'POST';}});
  assert(await body.text()==='B','body bytes extracted after later getters');
  const params=new URLSearchParams('a=1');
  const form=new Request('/index.html',{body:params,get method(){params.append('b','2');return 'POST';}});
  assert(await form.text()==='a=1&b=2','native BodyInit objects retained until extraction');
  const bodyOrder=[];
  new Request('/index.html',{body:{toString(){bodyOrder.push('string');return 'text';}},
    get cache(){bodyOrder.push('cache');},method:'POST'});
  assert(bodyOrder.join()==='cache,string','BodyInit extraction waits for complete dictionary conversion');
  for(const method of ['GET','HEAD']) for(const body of ['',new Uint8Array()]) {
    let error;
    try{new Request('/index.html',{method,body});}catch(caught){error=caught;}
    assert(error instanceof TypeError,'empty non-null body is still a body');
  }
  const source=new Request('/index.html',{method:'POST',body:'source'});
  try{new Request(source,{get signal(){throw sentinel;}});}catch(caught){error=caught;}
  assert(error===sentinel && !source.bodyUsed && await source.text()==='source',
    'failed conversion does not consume the input body');
  return true;
})()
