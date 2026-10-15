(async () => {
  const checks=[];
  const progress=globalThis.__mediaVolumeProgress={stage:'setup',checks};
  const assert=(condition,message)=>{if(!condition)throw Error(message);};
  const setter=(realm,name)=>Object.getOwnPropertyDescriptor(realm.HTMLMediaElement.prototype,name).set;
  const set=(realm,media,name,value)=>setter(realm,name).call(media,value);
  const wait=milliseconds=>new Promise(resolve=>setTimeout(resolve,milliseconds));
  function nextEvents(media,count,action,owner) {
    const events=[];
    return new Promise((resolve,reject)=>{
      const listener=event=>{
        try{
          assert(event.target===media && event.currentTarget===media,'native event target');
          assert(!event.bubbles && !event.composed && event.isTrusted,'native event flags');
          assert(event instanceof owner.Event,'native event realm');
          events.push([media.volume,media.muted]);
          if(events.length===count){media.removeEventListener('volumechange',listener);resolve(events);}
        }catch(error){media.removeEventListener('volumechange',listener);reject(error);}
      };
      media.addEventListener('volumechange',listener);
      action();
      assert(events.length===0,'volumechange must be asynchronous');
    });
  }
  const cases=[
    ['initial equal volume writes stay silent',async(owner,callee,media)=>{
      let count=0,conversions=0;media.onvolumechange=()=>count++;
      assert(media.volume===1,'the initial playback volume is one');
      set(callee,media,'volume',media.volume);
      set(callee,media,'volume',1);
      set(callee,media,'volume',{valueOf(){conversions++;return 1;}});
      assert(conversions===1 && media.volume===1 && count===0,'equal values still convert synchronously');
      await wait(20);
      assert(count===0,'equal initial playback volume must not queue an event');
    }],
    ['conversion compares against the current volume',async(owner,callee,media)=>{
      let count=0,conversions=0;media.addEventListener('volumechange',()=>count++);
      await nextEvents(media,1,()=>set(callee,media,'volume',{
        valueOf(){conversions++;set(callee,media,'volume',0.7);return 0.7;}
      }),owner);
      await wait(20);
      assert(conversions===1 && media.volume===0.7 && count===1,'the outer setter must compare after conversion changed the volume');
    }],
    ['volume notifies asynchronously',async(owner,callee,media)=>{
      set(callee,media,'volume',0.25);
      assert(media.volume===0.25,'volume changes synchronously');
      const events=await nextEvents(media,1,()=>{},owner);
      assert(events.length===1 && events[0][0]===0.25,'listener installed after the setter must observe the event');
    }],
    ['mute notifies asynchronously',async(owner,callee,media)=>{
      set(callee,media,'muted',true);
      assert(media.muted,'mute changes synchronously');
      const events=await nextEvents(media,1,()=>{},owner);
      assert(events.length===1 && events[0][1],'muted must queue volumechange');
    }],
    ['equal values stay silent',async(owner,callee,media)=>{
      await nextEvents(media,1,()=>set(callee,media,'muted',true),owner);
      await nextEvents(media,1,()=>set(callee,media,'muted',false),owner);
      let count=0;media.onvolumechange=()=>count++;
      set(callee,media,'volume',1);set(callee,media,'volume',1);
      set(callee,media,'muted',false);set(callee,media,'muted',false);
      await wait(20);assert(count===0,'equal explicit values must not notify');
      media.defaultMuted=true;
      assert(!media.muted,'an equal-value muted setter still records an explicit preference');
    }],
    ['batched changes do not coalesce',async(owner,callee,media)=>{
      const events=await nextEvents(media,4,()=>{
        set(callee,media,'volume',0.25);set(callee,media,'muted',true);
        set(callee,media,'volume',0.75);set(callee,media,'muted',false);
      },owner);
      assert(events.length===4 && events.every(([volume,muted])=>volume===0.75 && !muted),'each real change gets its own task observing current state');
    }],
    ['reentrant change gets a later task',async(owner,callee,media)=>{
      const order=[];
      await new Promise(resolve=>{
        media.onvolumechange=()=>{
          if(order.length===0){
            order.push('first');set(callee,media,'volume',0.25);
            Promise.resolve().then(()=>order.push('microtask'));
          }else{order.push('second');resolve();}
        };
        set(callee,media,'volume',0.5);
        assert(order.length===0,'the first event must be asynchronous');
      });
      assert(order.join('|')==='first|microtask|second','reentrant notification must wait for another task: '+order);
    }],
    ['invalid volume preserves state and error realm',async(owner,callee,media)=>{
      const valid=nextEvents(media,1,()=>set(callee,media,'volume',0.25),owner);
      for(const value of [-1,-Number.MIN_VALUE,1+Number.EPSILON,2]){
        let error;try{set(callee,media,'volume',value);}catch(value){error=value;}
        assert(error instanceof callee.DOMException && error.name==='IndexSizeError' && error.code===1,'range errors must use the setter realm');
        assert(media.volume===0.25,'an invalid value must not change volume');
      }
      for(const value of [NaN,Infinity,-Infinity,Symbol('volume')]){
        let error;try{set(callee,media,'volume',value);}catch(value){error=value;}
        assert(error instanceof callee.TypeError,'non-finite or symbol conversion must fail in the setter realm');
      }
      let conversions=0,error;
      try{set(callee,media,'volume',{valueOf(){conversions++;return 2;}});}catch(value){error=value;}
      assert(conversions===1 && error instanceof callee.DOMException && media.volume===0.25,'convert once before range validation');
      const sentinel=Error('conversion sentinel');error=undefined;
      try{set(callee,media,'volume',{valueOf(){throw sentinel;}});}catch(value){error=value;}
      assert(error===sentinel,'conversion must propagate the original exception');
      await valid;
    }],
    ['receiver brands precede conversion',async(owner,callee,media)=>{
      let conversions=0,traps=0;
      const converted={valueOf(){conversions++;return 0.25;}};
      const revoked=Proxy.revocable(media,{});revoked.revoke();
      const receivers=[{},Object.create(media),Object.create(callee.HTMLMediaElement.prototype),
        new Proxy(media,{get(){traps++;throw Error('author proxy trap');}}),revoked.proxy];
      for(const receiver of receivers){
        let error;try{set(callee,receiver,'volume',converted);}catch(value){error=value;}
        assert(error instanceof callee.TypeError,'forged and author proxy receivers must reject');
      }
      assert(conversions===0 && traps===0 && media.volume===1,'branding must happen before conversion or proxy traps');
    }],
    ['default muted preference and explicit override',async(owner,callee,media)=>{
      media.defaultMuted=true;assert(media.muted,'the default muted state must consult the content attribute');
      set(callee,media,'muted',true);media.removeAttribute('muted');
      assert(media.muted,'the explicit preference must survive attribute removal');
      set(callee,media,'muted',false);media.defaultMuted=true;
      assert(!media.muted,'the explicit false preference must override the content attribute');
    },'default-muted'],
    ['default to explicit mute state notifies once',async(owner,callee,media)=>{
      let count=0;media.addEventListener('volumechange',()=>count++);
      await nextEvents(media,1,()=>set(callee,media,'muted',false),owner);
      assert(!media.muted && count===1,'default to explicit false must notify even when effective mute is unchanged');
      set(callee,media,'muted',false);
      await wait(20);
      assert(count===1,'a repeated explicit false must not notify');
      media.defaultMuted=true;
      assert(!media.muted,'the explicit false preference must override later content attributes');
    },'mute-state-transition'],
    ['clones reset mutable controls',async(owner,callee,media)=>{
      set(callee,media,'volume',0.25);set(callee,media,'muted',true);
      media.currentTime=7;media.playbackRate=2;
      const copy=media.cloneNode(false);
      assert(copy.volume===1 && !copy.muted && copy.paused && !copy.seeking && copy.currentTime===0 && copy.playbackRate===1,'a clone must start from fresh media control state');
      assert(copy.readyState===0 && copy.networkState===0 && copy.error===null,'a clone must not share loaded state or errors');
      media.defaultMuted=true;set(callee,media,'muted',false);
      const withDefault=media.cloneNode(true);
      assert(withDefault.muted && withDefault.defaultMuted,'a clone must apply copied default attributes');
      assert(media.volume===0.25 && !media.muted && media.currentTime===7,'cloning must preserve the source');
    }],
    ['native volumechange is not cancelable',async(owner,callee,media)=>{
      const event=await new Promise(resolve=>{
        media.onvolumechange=resolve;
        set(callee,media,'volume',0.25);
      });
      event.preventDefault();
      assert(!event.cancelable && !event.defaultPrevented,'volumechange must use the default noncancelable EventInit');
    },'event-cancelability'],
  ];
  try{
    const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const kind of ['audio','video'])for(const windowless of [false,true]){
        const doc=windowless?owner.document.implementation.createHTMLDocument(''):owner.document;
        for(const [name,body,category] of cases){
          const media=doc.createElement(kind);doc.body.appendChild(media);
          const label=`${ownerName}/${calleeName}/${kind}/${windowless?'windowless':'live'}/${name}`;
          progress.stage=label;let timeout;
          try{
            await Promise.race([body(owner,callee,media),new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('control case timeout')),900);})]);
            if(windowless)assert(doc.defaultView===null && doc.hidden,'media control must not associate a Window with an inert Document');
            checks.push({name:label,category:category||'common',passed:true});
          }catch(error){checks.push({name:label,category:category||'common',passed:false,detail:String(error)});}
          finally{clearTimeout(timeout);media.remove();media.load();}
        }
      }
  }catch(error){checks.push({name:'volume control setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
