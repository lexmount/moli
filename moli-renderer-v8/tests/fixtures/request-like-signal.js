(async () => {
  const assert=(value,label)=>{if(!value)throw Error(label);};
  const url='data:text/plain,ok';
  const reject=async (input,init)=>{
    let promise;try{promise=fetch(input,init);}catch(error){throw Error('fetch threw synchronously');}
    assert(promise instanceof Promise,'fetch Promise');let caught=Symbol('not rejected');
    await promise.then(()=>{throw Error('expected rejection');},error=>{caught=error;});return caught;
  };
  const controller=new AbortController();let trapCalls=0;
  const proxy=new Proxy(controller.signal,{get(){trapCalls++;throw Error('public signal read');},getPrototypeOf(){trapCalls++;throw Error('public signal prototype');}});
  for(const invalid of [{},Object.create(AbortSignal.prototype),proxy]){
    let reads=0;const input={url,get signal(){reads++;return invalid;}};
    assert(await reject(input) instanceof TypeError,'inherited signal must have native AbortSignal identity');
    assert(reads===1,'legacy signal getter read once');
  }
  assert(trapCalls===0,'signal brand validation must not invoke author code');
  for(const init of [{signal:null},{signal:controller.signal}]){
    let reads=0;const response=await fetch({url,get signal(){reads++;return {};}},init);
    assert(await response.text()==='ok' && reads===1,'explicit init signal overrides malformed inherited object');
  }
  let reads=0;
  assert(await reject({url,get signal(){reads++;return {};}},{signal:undefined}) instanceof TypeError && reads===1,'undefined init inherits and validates');
  for(const sentinel of [{},null,undefined,7]){
    let reads=0;
    const caught=await reject({url,get signal(){reads++;return {};}},{get signal(){throw sentinel;}});
    assert(Object.is(caught,sentinel) && reads===1,'full init conversion precedes inherited signal validation');
    assert(Object.is(await reject({url,get signal(){throw sentinel;}}),sentinel),'source getter exception preserved');
  }
  const signal=controller.signal;Object.setPrototypeOf(signal,null);
  Object.defineProperty(signal,'aborted',{get(){throw Error('public aborted');}});
  let validReads=0;
  const valid=await fetch({url,get signal(){validReads++;return signal;}});
  assert(await valid.text()==='ok' && validReads===1,'native identity survives prototype and public property changes');
  const reason={},aborted=new AbortController();aborted.abort(reason);
  assert(await reject({url,signal:aborted.signal})===reason,'inherited native abort reason');
  assert(await reject({url,signal:{}},{signal:aborted.signal})===reason,'explicit aborted signal wins over malformed inherited object');
  const lateReason={},late=new AbortController();
  assert(await reject({url,signal:late.signal},{get signal(){late.abort(lateReason);return undefined;}})===lateReason,'read native state after init getters');
  return true;
})()
