(async () => {
  const checks=[];
  const progress=globalThis.__mediaPlaybackProgress={stage:'setup',checks};
  const assert=(ok,message)=>{if(!ok)throw Error(message);};
  function play(callee,media) {
    const promise=callee.HTMLMediaElement.prototype.play.call(media);
    promise.catch(()=>{});
    return promise;
  }
  const pause=(callee,media)=>callee.HTMLMediaElement.prototype.pause.call(media);
  const source=kind=>__mediaPlaybackSources[kind];
  const once=(media,event,action)=>new Promise((resolve,reject)=>{
    media.addEventListener(event,resolve,{once:true});
    if(event!=='error')media.addEventListener('error',()=>reject(Error('unexpected media error')),{once:true});
    action();
  });
  async function abort(owner,promise) {
    let error;try{await promise;}catch(value){error=value;}
    assert(error instanceof owner.DOMException && error.name==='AbortError','expected media-realm AbortError, got '+error);
  }
  async function ready(media,kind) {
    await once(media,'canplaythrough',()=>{media.src=source(kind);});
    assert(media.readyState===4,'loaded media must have enough data');
  }
  const cases=[
    ['same-task pause',async(owner,callee,media,kind)=>{
      const events=[];for(const name of ['play','playing','pause'])media.addEventListener(name,()=>events.push(name));
      media.src=source(kind);
      const promise=play(callee,media);
      assert(promise instanceof owner.Promise,'media Promise realm');
      assert(!media.paused && media.readyState<3,'play must change paused without inventing ready data');
      assert(events.length===0,'play events must be asynchronous');
      pause(owner,media);
      assert(media.paused && events.length===0,'pause state is synchronous, its event is not');
      await abort(owner,promise);
      assert(events.join('|')==='play|pause','no playing event before aborted playback: '+events);
    }],
    ['pause from play listener',async(owner,callee,media)=>{
      const events=[];
      media.addEventListener('play',()=>{events.push('play');pause(owner,media);});
      media.addEventListener('pause',()=>events.push('pause'));
      await abort(owner,play(callee,media));
      assert(events.join('|')==='play|pause','pause from play callback must abort the request');
    }],
    ['load aborts pending promise',async(owner,callee,media,kind)=>{
      media.src=source(kind);const promise=play(callee,media);media.load();
      await abort(owner,promise);assert(media.paused,'load must pause the element');
    }],
    ['source replacement aborts pending promise',async(owner,callee,media,kind)=>{
      media.src=source(kind);const promise=play(callee,media);media.src=source(kind)+'#replacement';
      await abort(owner,promise);assert(media.paused,'source replacement must pause the element');
    }],
    ['loaded play resolves after playing',async(owner,callee,media,kind)=>{
      await ready(media,kind);const events=[];
      for(const name of ['play','playing'])media.addEventListener(name,()=>events.push(name));
      const promise=play(callee,media);
      assert(events.length===0,'loaded play events must still be queued');
      const result=await promise;events.push('resolved');
      assert(result===undefined && events.join('|')==='play|playing|resolved','playing must precede fulfillment: '+events);
    }],
    ['multiple loaded play requests',async(owner,callee,media,kind)=>{
      await ready(media,kind);let plays=0,playing=0;
      media.onplay=()=>plays++;media.onplaying=()=>playing++;
      const first=play(callee,media),second=play(callee,media);
      assert(first!==second && first instanceof owner.Promise && second instanceof owner.Promise,'each call returns its own media-realm Promise');
      const results=await Promise.all([first,second]);
      assert(results.every(value=>value===undefined) && plays===1 && playing===1,'already playing must not duplicate events');
    }],
    ['pause from playing preserves captured promise',async(owner,callee,media,kind)=>{
      await ready(media,kind);
      const paused=once(media,'pause',()=>{});
      media.onplaying=()=>pause(owner,media);
      const result=await play(callee,media);
      assert(result===undefined && media.paused,'pause must not reject a request already captured by playing');
      await paused;
    }],
    ['load from play fulfills captured playing promise',async(owner,callee,media,kind)=>{
      await ready(media,kind);let playing=0;
      media.onplay=()=>media.load();media.onplaying=()=>playing++;
      const result=await play(callee,media);
      assert(result===undefined && playing===0 && media.paused,'load must fulfill an already captured request before discarding its playing event');
    }],
    ['removal pauses at stable state',async(owner,callee,media,kind)=>{
      await ready(media,kind);await play(callee,media);
      const paused=once(media,'pause',()=>media.remove());
      assert(!media.paused,'removal must not pause synchronously');
      await paused;assert(media.paused,'detached media must pause');
    }],
    ['move within document keeps playing',async(owner,callee,media,kind)=>{
      await ready(media,kind);await play(callee,media);let pauses=0;media.onpause=()=>pauses++;
      const container=owner.document.createElement('div');owner.document.body.appendChild(container);
      container.appendChild(media);
      await Promise.resolve();await Promise.resolve();
      assert(!media.paused && pauses===0,'reinserted media must remain playing');
      owner.document.body.appendChild(media);container.remove();
    }],
    ['adoption restarts loaded media for the new document',async(owner,callee,media,kind,ownerName)=>{
      await ready(media,kind);await play(callee,media);
      const destination=ownerName==='main'?document.querySelector('iframe').contentWindow:globalThis;
      destination.document.adoptNode(media);
      assert(media.ownerDocument===destination.document && media.paused && media.readyState===0,'adoption must reset playback before returning');
      await once(media,'canplaythrough',()=>destination.document.body.appendChild(media));
      const promise=play(callee,media);
      assert(promise instanceof owner.Promise,'adoption must retain the media object realm');
      assert(await promise===undefined && !media.paused,'playback must restart under the new document owner');
    }],
    ['shadow tree adoption resets media playback',async(owner,callee,media,kind,ownerName)=>{
      const host=owner.document.createElement('div');owner.document.body.appendChild(host);
      host.attachShadow({mode:'open'}).appendChild(media);
      try{
        await ready(media,kind);await play(callee,media);
        const destination=ownerName==='main'?document.querySelector('iframe').contentWindow:globalThis;
        destination.document.adoptNode(host);
        assert(media.ownerDocument===destination.document && media.paused && media.readyState===0,'adoption must reset media in shadow trees before returning');
        await once(media,'canplaythrough',()=>destination.document.body.appendChild(host));
        assert(await play(callee,media)===undefined && !media.paused,'shadow media must load under its new owner');
      }finally{host.remove();}
    }],
    ['windowless media keeps its relevant global',async(owner,callee,media,kind)=>{
      const detached=owner.document.implementation.createHTMLDocument('');
      const inactive=detached.createElement(kind);detached.body.appendChild(inactive);
      const eventRealms=[];
      for(const name of ['play','pause'])inactive.addEventListener(name,event=>eventRealms.push(event instanceof owner.Event));
      const promise=play(callee,inactive);let settled=false;promise.then(()=>settled=true,()=>settled=true);
      await Promise.resolve();
      assert(!settled && !inactive.paused && inactive.readyState===0,'play without a resource must remain pending without inventing readiness');
      assert(detached.defaultView===null && detached.hidden,'play must not associate a synthetic visible Window');
      pause(callee,inactive);await abort(owner,promise);
      assert(eventRealms.length===2 && eventRealms.every(Boolean),'windowless events must use the media object global');
    }],
    ['unsupported source rejects play',async(owner,callee,media,kind,ownerName,calleeName)=>{
      await once(media,'error',()=>{media.src=__mediaPlaybackUnsupportedURL+'/'+ownerName+'-'+calleeName+'-'+kind;});
      assert(media.error && media.error.code===4,'unsupported source must expose its native media error');
      let error;try{await play(callee,media);}catch(value){error=value;}
      assert(error instanceof owner.DOMException && error.name==='NotSupportedError','unsupported source must reject in media realm');
      assert(media.paused,'rejected unsupported play must retain paused state');
    }],
  ];
  try {
    const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)for(const kind of ['audio','video'])
      for(const [name,body] of cases){
        const media=owner.document.createElement(kind);media.muted=true;owner.document.body.appendChild(media);
        const label=`${ownerName}/${calleeName}/${kind}/${name}`;progress.stage=label;
        let timeout;
        try{await Promise.race([body(owner,callee,media,kind,ownerName,calleeName),new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('playback case timeout')),2500);})]);checks.push({name:label,passed:true});}
        catch(error){checks.push({name:label,passed:false,detail:String(error)});}
        finally{clearTimeout(timeout);media.remove();media.removeAttribute('src');media.load();}
      }
  }catch(error){checks.push({name:'media playback setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
