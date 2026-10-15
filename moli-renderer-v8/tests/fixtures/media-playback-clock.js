(async () => {
  const checks=[];
  const progress=globalThis.__mediaClockProgress={stage:'setup',checks};
  const assert=(condition,message)=>{if(!condition)throw Error(message);};
  const set=(callee,media,name,value)=>Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype,name).set.call(media,value);
  const play=(callee,media)=>callee.HTMLMediaElement.prototype.play.call(media);
  const pause=(callee,media)=>callee.HTMLMediaElement.prototype.pause.call(media);
  const wait=milliseconds=>new Promise(resolve=>setTimeout(resolve,milliseconds));
  function once(media,type,action,owner,predicate=()=>true) {
    return new Promise((resolve,reject)=>{
      const listener=event=>{
        try{
          assert(event.target===media && event.currentTarget===media && event.isTrusted,'native event target and trust');
          assert(event instanceof owner.Event,'native event realm');
          if(!predicate(event))return;
          media.removeEventListener(type,listener);resolve(event);
        }catch(error){media.removeEventListener(type,listener);reject(error);}
      };
      media.addEventListener(type,listener);
      if(action)action();
    });
  }
  async function readyPlay(owner,callee,media,kind) {
    set(callee,media,'src',__mediaPlaybackSources[kind]);
    const promise=play(callee,media);
    assert(promise instanceof owner.Promise,'play promise uses the media object realm');
    await promise;
    assert(!media.paused && media.readyState>=3,'loaded media must become playable');
  }
  const cases=[
    ['playback advances through native media tasks',async(owner,callee,media,kind)=>{
      await readyPlay(owner,callee,media,kind);
      const start=media.currentTime;
      await once(media,'timeupdate',null,owner,()=>media.currentTime>start);
      assert(!media.paused && media.currentTime>start,'playing media must advance');
      const official=media.currentTime;
      for(let i=0;i<1000;i++)assert(media.currentTime===official,'the official position stays stable within a JavaScript turn');
    }],
    ['pause seek and playbackRate preserve the clock',async(owner,callee,media,kind)=>{
      await readyPlay(owner,callee,media,kind);
      await once(media,'timeupdate',null,owner,()=>media.currentTime>0);
      await once(media,'pause',()=>pause(callee,media),owner);
      const stopped=media.currentTime;
      await wait(40);assert(media.currentTime===stopped && media.paused,'pause freezes playback position');
      set(callee,media,'playbackRate',0);
      await play(callee,media);
      const zeroRatePosition=media.currentTime;
      await wait(40);assert(!media.paused && media.currentTime===zeroRatePosition,'zero rate keeps an unpaused clock stationary');
      await once(media,'seeked',()=>set(callee,media,'currentTime',1),owner);
      assert(media.currentTime===1 && !media.seeking,'seek establishes the new anchor');
      const began=performance.now();
      await once(media,'timeupdate',()=>set(callee,media,'playbackRate',2),owner,()=>media.currentTime>1);
      const elapsed=(performance.now()-began)/1000;
      assert(Math.abs((media.currentTime-1)-2*elapsed)<0.2,'playbackRate must scale elapsed monotonic time');
    }],
    ['reentrant pause stops periodic notifications',async(owner,callee,media,kind)=>{
      await readyPlay(owner,callee,media,kind);
      const paused=once(media,'pause',null,owner);
      const first=event=>{
        if(media.currentTime===0)return;
        media.removeEventListener('timeupdate',first);
        pause(callee,media);
      };
      media.addEventListener('timeupdate',first);
      await paused;
      let later=0;media.ontimeupdate=()=>later++;
      const stopped=media.currentTime;
      await wait(300);
      assert(later===0 && media.currentTime===stopped && media.paused,'a paused clock must cancel its scheduled wakeup');
    }],
    ['empty resource settles in the relevant global',async(owner,callee,media)=>{
      media.defaultMuted=true;set(callee,media,'muted',false);set(callee,media,'volume',0.25);
      const order=[];
      media.addEventListener('error',()=>order.push('error'));
      const failed=once(media,'error',()=>set(callee,media,'src','data:,'),owner);
      const promise=play(callee,media);
      assert(promise instanceof owner.Promise,'failed request retains its promise realm');
      let rejected;
      await promise.then(()=>{throw Error('empty resource fulfilled');},error=>{rejected=error;order.push('rejected');});
      await failed;
      assert(rejected instanceof owner.DOMException && rejected.name==='NotSupportedError','failure uses the media object realm');
      assert(order.join('|')==='error|rejected','error precedes the play rejection');
      assert(media.error.code===4 && media.readyState===0 && media.volume===0.25 && !media.muted && media.defaultMuted,'failure preserves controls');
    }],
    ['HTTP resource loads under its relevant global',async(owner,callee,media,kind)=>{
      const ready=once(media,'canplaythrough',()=>set(callee,media,'src',__mediaClockHttpSources[kind]),owner);
      const promise=play(callee,media);
      await promise;await ready;
      assert(media.readyState===4 && media.error===null && !media.paused,'a real network response must complete for the media object');
    }],
  ];
  try{
    const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const kind of ['audio','video'])for(const windowless of [false,true]){
        const doc=windowless?owner.document.implementation.createHTMLDocument(''):owner.document;
        for(const [name,body] of cases){
          const media=doc.createElement(kind);doc.body.appendChild(media);
          const label=`${ownerName}/${calleeName}/${kind}/${windowless?'windowless':'live'}/${name}`;
          progress.stage=label;let timeout;
          try{
            await Promise.race([body(owner,callee,media,kind),new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('clock case timeout')),2000);})]);
            if(windowless)assert(doc.defaultView===null && doc.hidden,'media tasks must keep inert Documents windowless and hidden');
            checks.push({name:label,category:windowless?'windowless':'live',passed:true});
          }catch(error){checks.push({name:label,category:windowless?'windowless':'live',passed:false,detail:String(error)});}
          finally{clearTimeout(timeout);media.remove();media.removeAttribute('src');media.load();}
        }
      }
  }catch(error){checks.push({name:'clock fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
