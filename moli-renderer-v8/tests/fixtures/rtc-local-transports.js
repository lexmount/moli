(async()=>{
 const checks=[],child=document.querySelector('iframe').contentWindow;
 const assert=(ok,message='assertion failed')=>{if(!ok)throw Error(message);};
 const caught=body=>{try{body();}catch(error){return error;}};
 const tick=()=>new Promise(resolve=>setTimeout(resolve,40));
 const check=async(name,body)=>{try{await body();checks.push({name,passed:true});}catch(error){checks.push({name,passed:false,error:String(error),stack:error.stack});}};
 const parts=sdp=>sdp.split('m=').slice(1);
 const mids=sdp=>parts(sdp).map(part=>part.match(/a=mid:(\S+)/)[1]);
 const credentials=part=>({usernameFragment:part.match(/a=ice-ufrag:(\S+)/)[1],password:part.match(/a=ice-pwd:(\S+)/)[1]});
 const iceAttributes=['component','role','state','gatheringState','ongatheringstatechange','onstatechange','onselectedcandidatepairchange'];
 const iceMethods=['getLocalCandidates','getRemoteCandidates','getSelectedCandidatePair','getLocalParameters','getRemoteParameters'];
 const dtlsAttributes=['iceTransport','state','onstatechange','onerror'];
 const dtlsMethods=['getRemoteCertificates'];
 const ready=async pc=>{const transceiver=pc.addTransceiver('audio');await pc.setLocalDescription(await pc.createOffer());assert(transceiver.sender.transport,'missing DTLS association');return transceiver;};
 for(const [realm,w] of [['main',window],['iframe',child]]){
  const run=(name,body,config)=>check(`${realm}: ${name}`,async()=>{const pc=new w.RTCPeerConnection(config);try{await body(pc);}finally{w.RTCPeerConnection.prototype.close.call(pc);}});
  const get=(name,property,object)=>w.Object.getOwnPropertyDescriptor(w[name].prototype,property).get.call(object);
  await run('transport constructors are illegal and core declarations have native descriptors',pc=>{
   for(const [name,attributes,methods] of [['RTCIceTransport',iceAttributes,iceMethods],['RTCDtlsTransport',dtlsAttributes,dtlsMethods]]){
    assert(caught(()=>new w[name]()) instanceof w.TypeError);
    for(const property of attributes){const d=w.Object.getOwnPropertyDescriptor(w[name].prototype,property);assert(d&&d.enumerable&&d.configurable&&typeof d.get==='function');assert(d.get.name==='get '+property&&d.get.length===0);assert(property.startsWith('on')?typeof d.set==='function'&&d.set.length===1:d.set===undefined);}
    for(const method of methods){const d=w.Object.getOwnPropertyDescriptor(w[name].prototype,method);assert(d&&d.writable&&d.enumerable&&d.configurable&&d.value.name===method&&d.value.length===0);}
   }
  });
  await run('new transceivers have no MID or endpoint transport',pc=>{const t=pc.addTransceiver('audio');assert(t.mid===null&&t.sender.transport===null&&t.receiver.transport===null);});
  await run('offer creation reserves SDP credentials without exposing transport or MID',async pc=>{
   const t=pc.addTransceiver('audio'),offer=await pc.createOffer();assert(t.mid===null&&t.sender.transport===null&&t.receiver.transport===null);const c=credentials(parts(offer.sdp)[0]);assert(c.usernameFragment.length>=4&&c.password.length>=22&&parts(offer.sdp)[0].includes('a=setup:actpass'));
  });
  await run('local description associates sender and receiver with one native DTLS and ICE',async pc=>{
   const t=await ready(pc),dtls=t.sender.transport,ice=dtls.iceTransport;
   assert(dtls===t.receiver.transport&&dtls===t.sender.transport&&ice===dtls.iceTransport);
   assert(w.Object.getPrototypeOf(dtls)===w.RTCDtlsTransport.prototype&&w.Object.getPrototypeOf(ice)===w.RTCIceTransport.prototype);
   assert(dtls instanceof w.EventTarget&&ice instanceof w.EventTarget&&dtls.state==='new'&&ice.state==='new'&&t.mid===mids(pc.localDescription.sdp)[0]);
  });
  await run('ungathered ICE exposes the normative component and unknown role',async pc=>{
   const ice=(await ready(pc)).sender.transport.iceTransport;assert(ice.component==='rtp'&&ice.role==='unknown'&&ice.gatheringState==='new'&&ice.getSelectedCandidatePair()===null&&ice.getRemoteParameters()===null);
  });
  await run('candidate and certificate results are fresh sequences',async pc=>{
   const dtls=(await ready(pc)).sender.transport,ice=dtls.iceTransport;
   for(const [object,name] of [[ice,'getLocalCandidates'],[ice,'getRemoteCandidates'],[dtls,'getRemoteCertificates']]){const a=object[name](),b=object[name]();assert(w.Array.isArray(a)&&a!==b&&a.length===0&&b.length===0);a.push('author');assert(object[name]().length===0);}
  });
  await run('local parameters are fresh dictionaries matching the applied SDP',async pc=>{
   const t=await ready(pc),ice=t.sender.transport.iceTransport,a=ice.getLocalParameters(),b=ice.getLocalParameters(),c=credentials(parts(pc.localDescription.sdp)[0]);
   assert(a&&b&&a!==b&&w.Object.getPrototypeOf(a)===w.Object.prototype&&a.usernameFragment===c.usernameFragment&&a.password===c.password);
   a.usernameFragment='author';a.password='author';assert(ice.getLocalParameters().usernameFragment===c.usernameFragment&&ice.getLocalParameters().password===c.password);
  });
  for(const policy of ['balanced','max-compat','max-bundle'])await run(`${policy} shares only the prescribed local transport and credentials`,async pc=>{
   const ts=['audio','audio','video'].map(kind=>pc.addTransceiver(kind));pc.createDataChannel('data');await pc.setLocalDescription(await pc.createOffer());
   const transports=ts.map(t=>t.sender.transport);assert(transports.every((d,i)=>d&&d===ts[i].receiver.transport));
   const cs=parts(pc.localDescription.sdp).map(credentials),same=policy==='max-bundle';assert(new w.Set(transports).size===(same?1:3));
   assert(cs.every(c=>c.usernameFragment.length>=4&&c.password.length>=22));if(same)assert(new w.Set(cs.map(c=>c.usernameFragment)).size===1&&new w.Set(cs.map(c=>c.password)).size===1);
  },{bundlePolicy:policy});
  await run('repeated local offers retain associated MID and transport identities',async pc=>{
   const t=await ready(pc),dtls=t.sender.transport,ice=dtls.iceTransport,mid=t.mid;await pc.setLocalDescription(await pc.createOffer());assert(t.mid===mid&&t.sender.transport===dtls&&t.receiver.transport===dtls&&dtls.iceTransport===ice);
  });
  await run('applying an older offer does not associate a later transceiver',async pc=>{
   const a=pc.addTransceiver('audio'),offer=await pc.createOffer(),b=pc.addTransceiver('video');await pc.setLocalDescription(offer);
   assert(a.mid===mids(offer.sdp)[0]&&a.sender.transport&&b.mid===null&&b.sender.transport===null&&b.receiver.transport===null);
   await pc.setLocalDescription(await pc.createOffer());assert(b.mid!==null&&b.mid!==a.mid&&b.sender.transport);
  });
  await run('new transceivers preserve an already offered data media section index',async pc=>{
   pc.createDataChannel('first');const a=pc.addTransceiver('audio');await pc.setLocalDescription();const first=parts(pc.localDescription.sdp),old=mids(pc.localDescription.sdp);const b=pc.addTransceiver('video');await pc.setLocalDescription();
   const next=parts(pc.localDescription.sdp);assert(next.length===3&&next[0].split(' ')[0]===first[0].split(' ')[0]&&next[1].split(' ')[0]===first[1].split(' ')[0]&&mids(pc.localDescription.sdp).slice(0,2).join('|')===old.join('|')&&b.sender.transport&&a.sender.transport);
  });
  await run('stopping an unassociated transceiver does not consume an SDP section',async pc=>{
   const stopped=pc.addTransceiver('audio'),live=pc.addTransceiver('video');stopped.stop();const offer=await pc.createOffer();await pc.setLocalDescription(offer);
   assert(parts(offer.sdp).length===1&&stopped.mid===null&&stopped.sender.transport===null&&live.mid===mids(offer.sdp)[0]&&live.sender.transport);
  });
  await run('an associated stopped section keeps its MID and is rejected',async pc=>{
   const a=await ready(pc),old=a.mid,b=pc.addTransceiver('video');a.stop();const offer=await pc.createOffer();await pc.setLocalDescription(offer);
   const sections=parts(offer.sdp);assert(sections.length===2&&sections[0].startsWith('audio 0 ')&&sections[0].includes('a=inactive')&&a.mid===old&&b.mid!==old&&b.sender.transport);
  });
  await run('rollback detaches endpoints and closes only the old DTLS object',async pc=>{
   const t=await ready(pc),dtls=t.sender.transport,ice=dtls.iceTransport;await pc.setLocalDescription({type:'rollback'});
   assert(pc.localDescription===null&&t.mid===null&&t.sender.transport===null&&t.receiver.transport===null&&dtls.state==='closed'&&ice.state==='new'&&ice.getLocalParameters()===null);
  });
  await run('rollback emits one trusted DTLS statechange in listener order',async pc=>{
   const dtls=(await ready(pc)).sender.transport,ice=dtls.iceTransport,trace=[],events=[];let iceEvents=0;
   dtls.addEventListener('statechange',e=>{trace.push('first');events.push(e);assert(e.target===dtls&&e.currentTarget===dtls&&dtls.state==='closed');});
   dtls.onstatechange=()=>trace.push('handler');dtls.addEventListener('statechange',()=>trace.push('last'));ice.onstatechange=()=>iceEvents++;
   await pc.setLocalDescription({type:'rollback'});await tick();assert(trace.join('|')==='first|handler|last'&&events.length===1&&iceEvents===0);
   const e=events[0];assert(e instanceof w.Event&&w.Object.getPrototypeOf(e)===w.Event.prototype&&e.isTrusted&&!e.bubbles&&!e.cancelable&&!e.composed&&e.currentTarget===null);
  });
  await run('a new offer after rollback uses fresh transports and unused MIDs',async pc=>{
   const t=await ready(pc),old=t.sender.transport,mid=t.mid;await pc.setLocalDescription({type:'rollback'});await pc.setLocalDescription();
   assert(t.sender.transport&&t.sender.transport!==old&&t.sender.transport===t.receiver.transport&&t.mid!==mid&&old.state==='closed'&&old.iceTransport.state==='new');
  });
  await run('reapplying a saved generated offer creates a new association after rollback',async pc=>{
   const t=pc.addTransceiver('audio'),offer=await pc.createOffer();await pc.setLocalDescription(offer);const old=t.sender.transport;assert(old);await pc.setLocalDescription({type:'rollback'});await pc.setLocalDescription(offer);
   assert(t.mid===mids(offer.sdp)[0]&&t.sender.transport&&t.sender.transport!==old&&old.state==='closed');
  });
  await run('connection close closes both transports synchronously without state events',async pc=>{
   const t=await ready(pc),dtls=t.sender.transport,ice=dtls.iceTransport;let calls=0;dtls.onstatechange=()=>calls++;ice.onstatechange=()=>calls++;
   pc.close();assert(dtls.state==='closed'&&ice.state==='closed'&&t.sender.transport===dtls&&t.receiver.transport===dtls);await tick();assert(calls===0&&ice.getSelectedCandidatePair()===null);
  });
  await run('connection close also reaches ICE retained after rollback',async pc=>{
   const t=await ready(pc),old=t.sender.transport;await pc.setLocalDescription({type:'rollback'});await pc.setLocalDescription();const current=t.sender.transport;
   pc.close();assert(old.state==='closed'&&old.iceTransport.state==='closed'&&current.state==='closed'&&current.iceTransport.state==='closed');
  });
  await run('connection shutdown ignores author transport properties and methods',async pc=>{
   const dtls=(await ready(pc)).sender.transport,ice=dtls.iceTransport;let reads=0;
   for(const [object,properties] of [[dtls,['state','iceTransport','onstatechange']],[ice,['state','onstatechange']]])for(const name of properties)w.Object.defineProperty(object,name,{get(){reads++;throw Error('author getter');},configurable:true});
   ice.stop=()=>{reads++;throw Error('author method');};pc.close();assert(get('RTCDtlsTransport','state',dtls)==='closed'&&get('RTCIceTransport','state',ice)==='closed'&&reads===0);
  });
  await run('transport allocation and registries use intrinsic constructors and collections',async pc=>{
   const t=pc.addTransceiver('audio'),originals=w.Object.fromEntries(['RTCDtlsTransport','RTCIceTransport','Set','Map'].map(name=>[name,w[name]]));
   const methods=[];for(const name of ['Set','Map'])for(const key of ['add','set','get','has','clear','values'])if(typeof originals[name].prototype[key]==='function')methods.push([originals[name].prototype,key,originals[name].prototype[key]]);
   let calls=0;try{
    for(const name of w.Object.keys(originals))w[name]=function(){calls++;throw Error('author constructor');};for(const [prototype,key] of methods)prototype[key]=function(){calls++;throw Error('author collection');};
    await pc.setLocalDescription(await pc.createOffer());const dtls=t.sender.transport,ice=dtls.iceTransport;assert(w.Object.getPrototypeOf(dtls)===originals.RTCDtlsTransport.prototype&&w.Object.getPrototypeOf(ice)===originals.RTCIceTransport.prototype);pc.close();assert(calls===0&&dtls.state==='closed'&&ice.state==='closed');
   }finally{for(const [name,value] of w.Object.entries(originals))w[name]=value;for(const [prototype,key,value] of methods)prototype[key]=value;}
  });
  await run('transport EventHandlers retain objects and clear primitives without conversion',async pc=>{
   const dtls=(await ready(pc)).sender.transport,ice=dtls.iceTransport;let reads=0,calls=0;const object={get handleEvent(){reads++;throw Error('handleEvent');},toString(){reads++;throw Error('conversion');}};
   for(const [target,names] of [[dtls,['onstatechange','onerror']],[ice,['onstatechange','ongatheringstatechange','onselectedcandidatepairchange']]])for(const name of names){
    const fn=function(e){assert(this===target&&e.type===name.slice(2));calls++;};target[name]=fn;assert(target[name]===fn);target.dispatchEvent(new w.Event(name.slice(2)));target[name]=object;assert(target[name]===object);target.dispatchEvent(new w.Event(name.slice(2)));
    for(const value of [undefined,null,false,1,1n,'text',Symbol()]){target[name]=value;assert(target[name]===null);}
   }assert(calls===5&&reads===0);
  });
  await run('all transport bindings reject forged and author Proxy receivers before traps',async pc=>{
   const dtls=(await ready(pc)).sender.transport;let reads=0,conversions=0;const extra={toString(){conversions++;throw Error('conversion');}};
   for(const [name,real,attributes,methods] of [['RTCDtlsTransport',dtls,dtlsAttributes,dtlsMethods],['RTCIceTransport',dtls.iceTransport,iceAttributes,iceMethods]]){
    const proxy=new w.Proxy(real,{get(){reads++;throw Error('get');},getPrototypeOf(){reads++;throw Error('prototype');}}),revoked=w.Proxy.revocable(real,{});revoked.revoke();
    for(const receiver of [{},w.Object.create(w[name].prototype),w.Object.create(real),proxy,revoked.proxy,null,undefined]){
     for(const property of attributes){const d=w.Object.getOwnPropertyDescriptor(w[name].prototype,property);if(!d&&property==='component')continue;assert(caught(()=>d.get.call(receiver)) instanceof w.TypeError,`${name}.${property} getter brand`);if(d.set)assert(caught(()=>d.set.call(receiver,extra)) instanceof w.TypeError,`${name}.${property} setter brand`);}
     for(const method of methods)assert(caught(()=>w[name].prototype[method].call(receiver,extra)) instanceof w.TypeError);
    }
   }assert(reads===0&&conversions===0);
  });
 }
 for(const [name,callee,owner] of [['child callee',child,window],['main callee',window,child]]){
  await check(`${name}: borrowed local description creates transports in the connection realm`,async()=>{
   const pc=new owner.RTCPeerConnection(),t=pc.addTransceiver('audio');try{const offer=await callee.RTCPeerConnection.prototype.createOffer.call(pc);await callee.RTCPeerConnection.prototype.setLocalDescription.call(pc,offer);const dtls=t.sender.transport;assert(dtls&&owner.Object.getPrototypeOf(dtls)===owner.RTCDtlsTransport.prototype&&owner.Object.getPrototypeOf(dtls.iceTransport)===owner.RTCIceTransport.prototype);}finally{pc.close();}
  });
  await check(`${name}: borrowed transport accessors validate brands in the callee realm`,async()=>{
   const pc=new owner.RTCPeerConnection();try{const dtls=(await ready(pc)).sender.transport;
    for(const [name,real,properties,methods] of [['RTCDtlsTransport',dtls,dtlsAttributes,dtlsMethods],['RTCIceTransport',dtls.iceTransport,iceAttributes,iceMethods]]){
     for(const property of properties){const d=callee.Object.getOwnPropertyDescriptor(callee[name].prototype,property);if(!d&&property==='component')continue;assert(d.get.call(real)===owner.Object.getOwnPropertyDescriptor(owner[name].prototype,property).get.call(real));const error=caught(()=>d.get.call(new owner.Proxy(real,{})));assert(error instanceof callee.TypeError&&!(error instanceof owner.TypeError));}
     for(const method of methods){const fn=callee[name].prototype[method],error=caught(()=>fn.call(new owner.Proxy(real,{})));assert(error instanceof callee.TypeError&&!(error instanceof owner.TypeError));const result=fn.call(real);assert(result===null||typeof result==='object');}
    }
   }finally{pc.close();}
  });
 }
 globalThis.__uiEventResults={complete:true,passed:checks.filter(row=>row.passed).length,total:checks.length,checks};return checks.every(row=>row.passed);
})()
