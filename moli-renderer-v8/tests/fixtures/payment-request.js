(async () => {
  const checks=[];
  const record=(name,actual)=>checks.push({name,passed:actual===true,actual});
  const check=(name,action)=>{try{record(name,action());}catch(error){record(name,{name:error.name,message:error.message});}};
  const asyncCheck=async(name,action)=>{try{record(name,await action());}catch(error){record(name,{name:error.name,message:error.message});}};
  const realms=[window,document.querySelector('iframe').contentWindow];
  const methods=()=>[{supportedMethods:'not-available'}];
  const item=(value='1',currency='USD')=>({amount:{currency,value},label:'Total'});
  const details=()=>({total:item()});
  const raises=(action,C)=>{try{action();return false;}catch(error){return error instanceof C;}};
  for (const [index,w] of realms.entries()) {
    const C=w.PaymentRequest,p=C.prototype;
    const make=(d=details(),o={})=>new C(methods(),d,o);
    const c=(name,action)=>check(index+':'+name,action);
    const a=(name,action)=>asyncCheck(index+':'+name,action);
    c('constructor metadata',()=>C.length===2&&C.name==='PaymentRequest'&&Object.getPrototypeOf(C)===w.EventTarget&&Object.getPrototypeOf(p)===w.EventTarget.prototype);
    c('constructor requires new',()=>raises(()=>C(methods(),details()),w.TypeError));
    c('constructor requires methods and details',()=>raises(()=>new C(),w.TypeError)&&raises(()=>new C(methods()),w.TypeError));
    c('UUID version and uniqueness',()=>{const ids=Array.from({length:32},()=>make().id);return new Set(ids).size===ids.length&&ids.every(id=>/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id));});
    for (const id of ['',null,0,'\uD800','\uDC00','\uD83D\uDE00','x'.repeat(1024)]) c('provided id '+JSON.stringify(id),()=>make({...details(),id}).id===String(id));
    c('undefined id generates UUID',()=>make({...details(),id:undefined}).id.length===36);
    c('readonly defaults',()=>{const q=make();return q.shippingAddress===null&&q.shippingOption===null&&q.shippingType===null;});
    for(const type of ['shipping','delivery','pickup']) c('shipping type '+type,()=>make(details(),{requestShipping:true,shippingType:type}).shippingType===type);
    c('shipping default type',()=>make(details(),{requestShipping:true}).shippingType==='shipping');
    c('shipping options last selected including empty id',()=>{const shippingOptions=[{...item(),id:'first',selected:true},{...item(),id:'',selected:true}];return make({...details(),shippingOptions},{requestShipping:true}).shippingOption===''&&make({...details(),shippingOptions}).shippingOption===null;});
    c('unrequested shipping ignores semantic validation',()=>make({...details(),shippingOptions:[{...item('bad','bad!'),id:'x'},{...item(),id:'x'}]}).shippingOption===null);
    c('unrequested shipping still converts dictionary',()=>raises(()=>make({...details(),shippingOptions:[{}]}),w.TypeError));
    c('shipping duplicate ids',()=>raises(()=>make({...details(),shippingOptions:[{...item(),id:'x'},{...item(),id:'x'}]},{requestShipping:true}),w.TypeError));
    c('shipping types validate even if unrequested',()=>raises(()=>make(details(),{shippingType:'mail'}),w.TypeError));
    c('method list cannot be empty',()=>raises(()=>new C([],details()),w.TypeError));
    c('method normalized duplicate',()=>raises(()=>new C([{supportedMethods:'https://EXAMPLE.test:443/pay'},{supportedMethods:'https://example.test/pay'}],details()),w.RangeError));
    for(const method of ['a','a0-b1','https://example.test/path?q#hash',' \thttps://example.test/\n','https://:@example.test/']) c('valid PMI '+method,()=>new C([{supportedMethods:method}],details()).shippingAddress===null);
    for(const method of ['','a--b','a-0','A','a-','-a','visa,mastercard','http://example.test/','https://u@example.test/','https://:p@example.test/','https://','not-https://example.test/','\uD800']) c('invalid PMI '+JSON.stringify(method),()=>raises(()=>new C([{supportedMethods:method}],details()),w.RangeError));
    for(const value of ['0','1.25','000.010','1'.repeat(600)+'.'+'2'.repeat(600)]) c('valid total '+(value.length>50?'high precision':value),()=>make({total:item(value)}).shippingAddress===null);
    for(const value of ['','-0','-0.00','-1','+1','.1','1.','1e3','1 2','NaN','Infinity',' 1 ','\u0661']) c('invalid total '+JSON.stringify(value),()=>raises(()=>make({total:item(value)}),w.TypeError));
    for(const currency of ['usd','XxX','EUR']) c('valid currency '+currency,()=>make({total:item('1',currency)}).shippingAddress===null);
    for(const currency of ['','EU','USDD','123','\u0131nr','\uD800US']) c('invalid currency '+JSON.stringify(currency),()=>raises(()=>make({total:item('1',currency)}),w.RangeError));
    c('negative display and shipping amounts',()=>make({...details(),displayItems:[item('-10')],shippingOptions:[{...item('-1'),id:'discount',selected:true}]},{requestShipping:true}).shippingOption==='discount');
    c('display amounts validated',()=>raises(()=>make({...details(),displayItems:[item('wrong')]}),w.TypeError));
    c('modifier total must be nonnegative',()=>raises(()=>make({...details(),modifiers:[{supportedMethods:'anything',total:item('-1')}]}),w.TypeError));
    c('modifier currency validated',()=>raises(()=>make({...details(),modifiers:[{supportedMethods:'anything',additionalDisplayItems:[item('1','bad!')]}]}),w.RangeError));
    c('method data IDL object',()=>[null,1,'data',true].every(data=>raises(()=>new C([{supportedMethods:'not-available',data}],details()),w.TypeError)));
    c('method JSON cyclic',()=>{const data={};data.self=data;return raises(()=>new C([{supportedMethods:'not-available',data}],details()),w.TypeError);});
    c('method JSON undefined',()=>raises(()=>new C([{supportedMethods:'not-available',data:{toJSON(){}}}],details()),w.TypeError));
    c('method JSON bigint',()=>raises(()=>new C([{supportedMethods:'not-available',data:{x:1n}}],details()),w.TypeError));
    c('method JSON exception identity',()=>{const sentinel={};try{new C([{supportedMethods:'not-available',data:{toJSON(){throw sentinel;}}}],details());return false;}catch(error){return error===sentinel;}});
    c('modifier JSON exception identity',()=>{const sentinel={};try{make({...details(),modifiers:[{supportedMethods:'not-available',data:{toJSON(){throw sentinel;}}}]});return false;}catch(error){return error===sentinel;}});
    c('dictionary inheritance and serialization order',()=>{const log=[];const capture=(prefix,value)=>new Proxy(value,{get(t,k,r){if(typeof k==='string')log.push(prefix+k);return Reflect.get(t,k,r);}});const data={toJSON(){log.push('serialize');return {};}};new C([capture('m.',{supportedMethods:'not-available',data})],capture('d.',{...details(),id:'id',displayItems:[],modifiers:[],shippingOptions:[]}),capture('o.',{}));return JSON.stringify(log)===JSON.stringify(['m.data','m.supportedMethods','d.displayItems','d.modifiers','d.shippingOptions','d.id','d.total','o.requestBillingAddress','o.requestPayerEmail','o.requestPayerName','o.requestPayerPhone','o.requestShipping','o.shippingType','serialize']);});
    c('all IDL conversion precedes PMI validation',()=>{const sentinel={};try{new C([{supportedMethods:'INVALID'}],{get total(){throw sentinel;}});return false;}catch(error){return error===sentinel;}});
    c('getter exception identity',()=>{const sentinel={};try{make(details(),{get requestShipping(){throw sentinel;}});return false;}catch(error){return error===sentinel;}});
    c('constructor snapshots readonly values',()=>{const d={...details(),id:'original',shippingOptions:[{...item(),id:'selected',selected:true}]};const options={requestShipping:true,shippingType:'delivery'};const q=make(d,options);d.id='mutated';d.shippingOptions[0].id='changed';options.shippingType='pickup';return q.id==='original'&&q.shippingOption==='selected'&&q.shippingType==='delivery';});
    c('internal snapshots use own data properties and native JSON',()=>{const A=w.Array.prototype,O=w.Object.prototype,J=w.JSON;const iterator=Object.getOwnPropertyDescriptor(A,Symbol.iterator),zero=Object.getOwnPropertyDescriptor(A,'0'),id=Object.getOwnPropertyDescriptor(O,'id'),stringify=J.stringify;let calls=0,q;const poison=()=>{calls++;throw Error('poison');};try{q=make({...details(),id:'safe'},{get shippingType(){Object.defineProperty(A,Symbol.iterator,{value:poison,configurable:true,writable:true});Object.defineProperty(A,'0',{set:poison,configurable:true});Object.defineProperty(O,'id',{set:poison,configurable:true});J.stringify=poison;return 'shipping';}});}finally{Object.defineProperty(A,Symbol.iterator,iterator);if(zero)Object.defineProperty(A,'0',zero);else delete A[0];if(id)Object.defineProperty(O,'id',id);else delete O.id;J.stringify=stringify;}return q.id==='safe'&&calls===0;});
    for(const name of ['id','shippingAddress','shippingOption','shippingType']) c('readonly descriptor '+name,()=>{const d=Object.getOwnPropertyDescriptor(p,name);const q=make();return d.enumerable&&d.configurable&&typeof d.get==='function'&&d.set===undefined&&!(name in Object.getOwnPropertyDescriptors(q));});
    for(const name of ['show','abort','canMakePayment']) c('method descriptor '+name,()=>{const d=Object.getOwnPropertyDescriptor(p,name);return d.enumerable&&d.configurable&&d.writable&&d.value.length===0;});
    for(const name of ['shippingaddresschange','shippingoptionchange','paymentmethodchange']) {
      c('handler default and nonobject '+name,()=>{const q=make(),key='on'+name;q[key]=1;return q[key]===null;});
      c('handler ordered replacement and reactivation '+name,()=>{const q=make(),key='on'+name,log=[];q.addEventListener(name,()=>log.push('first'));q[key]=()=>log.push('old');q.addEventListener(name,()=>log.push('last'));q[key]=function(e){log.push(this===q&&e.type===name?'handler':'wrong');return false;};const event=new w.Event(name,{cancelable:true});const dispatched=q.dispatchEvent(event);if(dispatched||!event.defaultPrevented||log.join()!=='first,handler,last')return false;q[key]=null;q[key]=()=>log.push('new');log.length=0;q.dispatchEvent(new w.Event(name));return log.join()==='first,last,new';});
    }
    for(const [calleeIndex,callee] of realms.entries()) {
      for(const name of ['id','shippingAddress','shippingOption','shippingType','onshippingaddresschange','onshippingoptionchange','onpaymentmethodchange']) {
        const descriptor=Object.getOwnPropertyDescriptor(callee.PaymentRequest.prototype,name);
        c('cross realm getter '+calleeIndex+':'+name,()=>{const q=make({...details(),id:'cross'});return descriptor.get.call(q)===q[name];});
        c('brand getter '+calleeIndex+':'+name,()=>{const q=make();const revoked=Proxy.revocable(q,{});revoked.revoke();return [{},Object.create(p),Object.create(q),new Proxy(q,{}),revoked.proxy,new w.EventTarget()].every(value=>raises(()=>descriptor.get.call(value),callee.TypeError));});
      }
      for(const method of ['show','abort','canMakePayment']) await a('promise receiver brand '+calleeIndex+':'+method,async()=>{const q=make();let conversions=0;const fn=callee.PaymentRequest.prototype[method];let promise;try{promise=fn.call(new Proxy(q,{}),{get then(){conversions++;return undefined;}});}catch{return false;}if(!(promise instanceof callee.Promise)||conversions)return false;try{await promise;return false;}catch(error){return error instanceof callee.TypeError&&(callee===w||!(error instanceof w.TypeError));}});
    }
    await a('no payment handler capability and fresh promises',async()=>{const q=make();const one=q.canMakePayment(),two=q.canMakePayment();return one!==two&&one instanceof w.Promise&&await one===false&&await two===false;});
    await a('abort rejects outside interactive state',async()=>{const q=make();try{await q.abort();return false;}catch(error){return error instanceof w.DOMException&&error.name==='InvalidStateError'&&await q.canMakePayment()===false;}});
    await a('show converts thenable before presentation checks',async()=>{let reads=0;const sentinel={},q=make();try{await q.show({get then(){reads++;throw sentinel;}});return false;}catch(error){return reads===1&&error!==sentinel&&error instanceof w.DOMException&&['SecurityError','NotSupportedError'].includes(error.name);}});
  }
  if(new Set(checks.map(c=>c.name)).size!==checks.length)throw Error('duplicate check names');
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(c=>c.passed).length,checks};
  return checks.every(c=>c.passed);
})()
