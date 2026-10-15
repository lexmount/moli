(async () => {
  const checks=[];
  const progress=globalThis.__mediaDefaultStartProgress={stage:'setup',checks};
  const assert=(value,message)=>{if(!value)throw Error(message);};
  const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
  const set=(callee,media,value)=>Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype,'currentTime').set.call(media,value);
  const get=(callee,media)=>Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype,'currentTime').get.call(media);
  const load=(callee,media)=>callee.HTMLMediaElement.prototype.load.call(media);
  async function until(condition,message) {
    const deadline=Date.now()+5000;
    while(!condition()) {assert(Date.now()<deadline,message);await wait(5);}
  }
  async function ready(media,callee,source,waitForInitialSeek=false) {
    let seekComplete=!waitForInitialSeek;
    if(waitForInitialSeek)media.addEventListener('seeked',()=>seekComplete=true,{once:true});
    let done=false;media.addEventListener('canplaythrough',()=>done=true,{once:true});
    media.src=source;load(callee,media);
    await until(()=>done && !media.seeking && seekComplete,'resource did not become ready and finish seeking');
    assert(media.error===null && media.readyState>=3,'valid resource readiness');
  }
  const cases=[
    ['cold assignments preserve raw finite values without seeking',async(media,callee)=>{
      let events=0;
      for(const type of ['seeking','timeupdate','seeked'])media.addEventListener(type,()=>events++);
      for(const value of [1,-1,Number.MAX_VALUE,0,-0,2]) {
        set(callee,media,value);
        assert(Object.is(get(callee,media),value===0?0:value),'HAVE_NOTHING exposes the default start position');
        assert(media.readyState===0 && !media.seeking,'a cold assignment does not seek');
      }
      await wait(20);assert(events===0,'a default start position must not dispatch seek events');
    }],
    ['load preserves and applies the last positive default',async(media,callee,source)=>{
      const order=[];
      for(const type of ['durationchange','resize','loadedmetadata','seeking','timeupdate','seeked'])media.addEventListener(type,()=>order.push(type));
      media.addEventListener('timeupdate',()=>queueMicrotask(()=>order.push('timeupdate microtask')));
      set(callee,media,2);set(callee,media,3);set(callee,media,1);
      load(callee,media);
      assert(get(callee,media)===1 && !media.seeking,'load preserves the pending start preference');
      await ready(media,callee,source,true);
      assert(Math.abs(get(callee,media)-1)<1e-6 && media.paused,'metadata applies the default without starting playback');
      assert(order.indexOf('loadedmetadata')<order.indexOf('seeking'),'metadata notifications precede the initial seeking event');
      const seeking=order.indexOf('seeking'),updated=order.indexOf('timeupdate',seeking);
      assert(seeking>=0 && seeking<updated && updated<order.indexOf('seeked'),'initial seek completion event order');
      const checkpoint=order.indexOf('timeupdate microtask',updated);
      assert(updated<checkpoint && checkpoint<order.indexOf('seeked'),'a microtask checkpoint separates the completion notifications');
      return {position:media.currentTime,order:order.slice()};
    }],
    ['a reentrant seek retains the completed seek notification',async(media,callee,source)=>{
      await ready(media,callee,source);
      let restarted=false,notifications=0;
      media.addEventListener('timeupdate',()=>{
        if(!restarted && !media.seeking && get(callee,media)===1) {
          restarted=true;set(callee,media,2);
        }
      });
      media.addEventListener('seeked',()=>notifications++);
      set(callee,media,1);
      await until(()=>restarted && notifications===2 && !media.seeking,'both completed seeks must keep their notification');
      assert(get(callee,media)===2,'the reentrant seek reaches its requested position');
    }],
    ['negative defaults are cleared when metadata becomes available',async(media,callee,source)=>{
      let seeks=0;media.addEventListener('seeking',()=>seeks++);
      set(callee,media,-2);assert(get(callee,media)===-2,'negative default is observable before metadata');
      await ready(media,callee,source);
      assert(get(callee,media)===0 && seeks===0,'only a positive default causes an initial seek');
    }],
    ['finite resource bounds clamp default and explicit seeks',async(media,callee,source)=>{
      set(callee,media,Number.MAX_VALUE);await ready(media,callee,source,true);
      assert(get(callee,media)===media.duration,'initial seek is clamped to the resource end');
      let done=false;media.addEventListener('seeked',()=>done=true,{once:true});
      set(callee,media,-1);await until(()=>done,'lower-bound seek did not finish');
      assert(get(callee,media)===0,'explicit seek is clamped to the earliest position');
      done=false;media.addEventListener('seeked',()=>done=true,{once:true});
      set(callee,media,Number.MAX_VALUE);await until(()=>done,'upper-bound seek did not finish');
      assert(get(callee,media)===media.duration,'explicit seek is clamped to the finite duration');
      const snapshot=media.seekable;assert(snapshot.length===1 && snapshot.start(0)===0 && snapshot.end(0)===media.duration,'the complete resource has a finite seekable range');
      media.removeAttribute('src');load(callee,media);
      assert(media.seekable.length===0 && snapshot.length===1,'TimeRanges reads are independent resource snapshots');
    }],
    ['conversion observes a reentrant load before choosing seek semantics',async(media,callee,source)=>{
      await ready(media,callee,source);
      let conversions=0;
      set(callee,media,{valueOf(){conversions++;media.removeAttribute('src');load(callee,media);return 2;}});
      assert(conversions===1 && media.readyState===0 && !media.seeking && get(callee,media)===2,'conversion can return the receiver to HAVE_NOTHING');
      await ready(media,callee,source,true);assert(Math.abs(get(callee,media)-2)<1e-6,'the converted default survives reselection');
    }],
    ['clones and imports discard the default start preference',async(media,callee)=>{
      set(callee,media,2);
      for(const clone of [media.cloneNode(true),callee.document.importNode(media,true)]) {
        assert(get(callee,clone)===0 && !clone.seeking && clone.readyState===0,'clones start with fresh media state');
      }
      assert(get(callee,media)===2,'cloning does not mutate the source preference');
    }],
    ['receiver checks precede conversion for author and revoked Proxies',async(media,callee)=>{
      let conversions=0,traps=0;
      const value={valueOf(){conversions++;return 2;}};
      const proxy=new Proxy(media,{get(){traps++;throw Error('get trap');},getPrototypeOf(){traps++;throw Error('prototype trap');}});
      const revoked=Proxy.revocable(media,{});revoked.revoke();
      for(const receiver of [{},Object.create(media),proxy,revoked.proxy]) {
        let caught;try {set(callee,receiver,value);}catch(error){caught=error;}
        assert(caught instanceof callee.TypeError,'illegal receiver error belongs to the setter realm');
      }
      assert(conversions===0 && traps===0,'invalid receivers must not execute author code');
    }],
  ];
  try {
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const kind of ['audio','video'])for(const transport of ['data','http']) {
        for(const [name,body] of cases) {
          const label=`${ownerName}/${calleeName}/${kind}/${transport}/${name}`;progress.stage=label;
          const media=owner.document.createElement(kind);owner.document.body.appendChild(media);
          try {const facts=await body(media,callee,__mediaTimelineSources[transport][kind]);checks.push({name:label,passed:true,...facts});}
          catch(error){checks.push({name:label,passed:false,detail:String(error)});}
          finally {media.pause();media.removeAttribute('src');load(callee,media);media.remove();}
        }
      }
  }catch(error){checks.push({name:'default start fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
