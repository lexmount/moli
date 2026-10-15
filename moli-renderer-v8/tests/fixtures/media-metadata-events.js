(async () => {
  const checks=[];
  const progress=globalThis.__mediaMetadataProgress={stage:'setup',checks};
  const assert=(value,message)=>{if(!value)throw Error(message);};
  const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
  const expected={audio:110255/22050,video:9.8};
  const types=['loadstart','durationchange','resize','loadedmetadata','loadeddata','canplay','canplaythrough'];
  const load=(callee,media)=>callee.HTMLMediaElement.prototype.load.call(media);
  const duration=(callee,media)=>Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype,'duration').get.call(media);
  async function until(condition,message) {
    const deadline=Date.now()+5000;
    while(!condition()) {assert(Date.now()<deadline,message);await wait(5);}
  }
  function values(media,callee) {
    assert(Math.abs(duration(callee,media)-expected[media.localName])<1e-6,'duration must come from the resource timeline');
    assert(duration(callee,media)===media.duration,'cross-realm getters must share metadata');
    if(media.localName==='video')assert(media.videoWidth===352 && media.videoHeight===288,'natural dimensions must come from the decoded resource');
  }
  function empty(media,callee) {
    assert(Number.isNaN(duration(callee,media)) && media.readyState===0,'no resource metadata means NaN and HAVE_NOTHING');
    if(media.localName==='video')assert(media.videoWidth===0 && media.videoHeight===0,'natural dimensions reset with metadata');
  }
  function record(media,callee) {
    const trace=[],records=[];
    let complete=false;
    for(const type of types)media.addEventListener(type,event=>{
      trace.push(type);
      records.push({type,duration:duration(callee,media),readyState:media.readyState,
        width:media.videoWidth,height:media.videoHeight,bubbles:event.bubbles,cancelable:event.cancelable,
        exactTarget:event.target===media});
      if(['durationchange','resize','loadedmetadata'].includes(type)) {
        Promise.resolve().then(()=>trace.push(type+' microtask'));
      }
      if(type==='canplaythrough')complete=true;
    });
    return {trace,records,complete:()=>complete};
  }
  function begin(media,callee,source) {media.src=source;load(callee,media);}
  async function ready(media,state) {
    await until(state.complete,'resource events did not complete');
    assert(media.error===null && media.readyState>=3,'valid media must become ready');
  }
  async function replaceDuring(media,callee,source,type,microtask) {
    const state=record(media,callee);
    let replaced=false;
    media.addEventListener(type,()=>{
      if(replaced)return;
      replaced=true;
      const replace=()=>{values(media,callee);load(callee,media);empty(media,callee);};
      if(microtask)Promise.resolve().then(replace);else replace();
    });
    begin(media,callee,source);await ready(media,state);values(media,callee);
    const events=state.records.map(record=>record.type);
    assert(events.filter(value=>value==='durationchange').length===2,'each usable selection gets one durationchange');
    assert(events.filter(value=>value==='loadedmetadata').length===1,'the old loadedmetadata task must be cancelled');
    assert(events.filter(value=>value==='resize').length===(media.localName==='audio'?0:type==='resize'?2:1),'resize must belong to the current selection');
    return {trace:state.trace.slice()};
  }
  const cases=[
    ['initial state and receiver brands',async(media,callee)=>{
      empty(media,callee);
      let traps=0;
      const proxy=new Proxy(media,{get(){traps++;throw Error('get trap');},getPrototypeOf(){traps++;throw Error('prototype trap');}});
      const revoked=Proxy.revocable(media,{});revoked.revoke();
      for(const receiver of [{},Object.create(media),proxy,revoked.proxy,callee.document.createElement('div')]) {
        let caught;try {duration(callee,receiver);}catch(error){caught=error;}
        assert(caught instanceof callee.TypeError,'illegal receivers must throw in the getter realm');
      }
      assert(traps===0,'author Proxy traps must not participate in receiver validation');
    }],
    ['metadata event order and values',async(media,callee,source)=>{
      const state=record(media,callee);begin(media,callee,source);await ready(media,state);values(media,callee);
      const metadata=media.localName==='video'?['durationchange','resize','loadedmetadata']:['durationchange','loadedmetadata'];
      const events=state.records.map(record=>record.type);
      assert(JSON.stringify(events)===JSON.stringify(['loadstart',...metadata,'loadeddata','canplay','canplaythrough']),'metadata events precede readiness events exactly once');
      for(const type of metadata) {
        const event=state.records.find(record=>record.type===type);
        assert(event.readyState>=1 && Math.abs(event.duration-expected[media.localName])<1e-6,'metadata callbacks must see initialized duration and readyState');
        if(media.localName==='video')assert(event.width===352 && event.height===288,'durationchange and resize must see natural dimensions');
        const next=events[events.indexOf(type)+1];
        assert(state.trace.indexOf(type+' microtask')<state.trace.indexOf(next),'each metadata event uses a separate task');
      }
      return {duration:media.duration,width:media.videoWidth,height:media.videoHeight,trace:state.trace.slice()};
    }],
    ['metadata event target and flags',async(media,callee,source)=>{
      const state=record(media,callee);begin(media,callee,source);await ready(media,state);values(media,callee);
      const events=state.records.filter(record=>['durationchange','resize','loadedmetadata'].includes(record.type));
      for(const event of events)assert(!event.bubbles && !event.cancelable && event.exactTarget,'metadata event target and flags: '+JSON.stringify(event));
      return {events};
    }],
    ['load resets metadata without durationchange',async(media,callee,source)=>{
      const state=record(media,callee);begin(media,callee,source);await ready(media,state);values(media,callee);
      const before=state.records.filter(record=>record.type==='durationchange').length;
      media.removeAttribute('src');load(callee,media);empty(media,callee);
      await wait(40);empty(media,callee);
      assert(state.records.filter(record=>record.type==='durationchange').length===before,'resetting duration to NaN must not fire durationchange');
    }],
    ['durationchange replaces selection',(media,callee,source)=>replaceDuring(media,callee,source,'durationchange',false)],
    ['durationchange microtask replaces selection',(media,callee,source)=>replaceDuring(media,callee,source,'durationchange',true)],
    ['clones start without resource metadata',async(media,callee,source)=>{
      const state=record(media,callee);begin(media,callee,source);await ready(media,state);values(media,callee);
      const clone=media.cloneNode(true);empty(clone,callee);
      assert(clone.getAttribute('src')===media.getAttribute('src'),'cloning preserves the source attribute');
      clone.removeAttribute('src');load(callee,clone);
    }],
  ];
  try {
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const kind of ['audio','video'])for(const transport of ['data','http']) {
        const group=kind==='video'?[...cases,['resize replaces selection',(media,callee,source)=>replaceDuring(media,callee,source,'resize',false)]]:cases;
        for(const [name,body] of group) {
          const label=`${ownerName}/${calleeName}/${kind}/${transport}/${name}`;
          progress.stage=label;
          const media=owner.document.createElement(kind);owner.document.body.appendChild(media);
          try {const facts=await body(media,callee,__mediaMetadataSources[transport][kind]);checks.push({name:label,passed:true,...facts});}
          catch(error){checks.push({name:label,passed:false,detail:String(error)});}
          finally {media.pause();media.removeAttribute('src');load(callee,media);media.remove();}
        }
      }
  } catch(error) {checks.push({name:'metadata fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
