(async () => {
  const checks=[];
  const progress=globalThis.__mediaResourceEndProgress={stage:'setup',checks};
  const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
  const assert=(value,message)=>{if(!value)throw Error(message);};
  const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  const set=(callee,media,name,value)=>Object.getOwnPropertyDescriptor(callee.HTMLMediaElement.prototype,name).set.call(media,value);
  async function until(condition,message) {
    const deadline=Date.now()+4000;
    while(!condition()){assert(Date.now()<deadline,message);await wait(5);}
  }
  async function ready(media,source) {
    let loaded=false;media.addEventListener('canplaythrough',()=>loaded=true,{once:true});
    media.src=source;media.load();await until(()=>loaded,'metadata did not become ready');
    assert(Number.isFinite(media.duration) && media.duration>0 && media.readyState>=3,'a real finite resource is required');
  }
  async function seek(callee,media,time) {
    let done=false;media.addEventListener('seeked',()=>done=true,{once:true});
    set(callee,media,'currentTime',time);await until(()=>done,'seek did not finish');
  }
  async function finish(owner,callee,media) {
    let ended=false;media.addEventListener('ended',()=>ended=true,{once:true});
    await seek(callee,media,media.duration-0.05);
    await callee.HTMLMediaElement.prototype.play.call(media);
    await until(()=>ended,'normal playback did not end');
    assert(media.ended && media.paused && media.currentTime===media.duration,'natural end state');
  }
  async function restart(callee,media,loop) {
    await seek(callee,media,media.duration);
    assert(media.ended && media.paused,'a paused seek establishes an ended position');
    media.loop=loop;
    assert(media.ended===!loop,'loop affects the ended getter immediately');
    const order=[];
    media.addEventListener('seeked',()=>order.push('seeked'),{once:true});
    media.addEventListener('playing',()=>order.push('playing'),{once:true});
    const promise=callee.HTMLMediaElement.prototype.play.call(media);
    assert(!media.paused && !media.ended && media.seeking && media.currentTime<media.duration,'play restarts from the beginning');
    await promise.then(()=>order.push('resolved'));
    await until(()=>order.includes('seeked'),'the restart must seek');
    assert(order.indexOf('playing')>=0 && order.indexOf('playing')<order.indexOf('resolved'),'playing precedes promise fulfillment');
  }
  const cases=[
    ['ended follows finite position, seeking, loop and resource reset',async(owner,callee,media,source)=>{
      assert(!media.ended && media.readyState===0,'a cold element is not ended');
      await ready(media,source);assert(!media.ended && media.paused,'a fresh loaded element is not ended');
      let done=false;media.addEventListener('seeked',()=>done=true,{once:true});
      set(callee,media,'currentTime',media.duration);
      assert(media.seeking && !media.ended,'an incomplete seek is not ended');
      await until(()=>done,'the end seek did not complete');
      assert(media.ended && media.paused && media.currentTime===media.duration,'a completed end seek exposes ended');
      media.loop=true;assert(!media.ended,'loop suppresses the ended getter');
      media.loop=false;assert(media.ended,'removing loop restores the ended getter');
      const clone=media.cloneNode();assert(!clone.ended && clone.readyState===0,'clones do not inherit an ended resource');
      media.removeAttribute('src');media.load();assert(!media.ended && Number.isNaN(media.duration),'load discards ended state');
    }],
    ['natural completion publishes bounded time and ordered trusted events',async(owner,callee,media,source)=>{
      await ready(media,source);
      const events=[];
      for(const type of ['timeupdate','pause','ended'])media.addEventListener(type,event=>events.push({type,time:media.currentTime,paused:media.paused,ended:media.ended,trusted:event.isTrusted,bubbles:event.bubbles,cancelable:event.cancelable,realm:event instanceof owner.Event}));
      await finish(owner,callee,media);
      const end=events.findIndex(event=>event.type==='ended'),pause=events.findIndex(event=>event.type==='pause');
      assert(pause>=0 && pause<end && events.slice(0,pause).some(event=>event.type==='timeupdate'),'timeupdate, pause, ended order');
      assert(events[pause].paused && events[end].ended && events[end].time===media.duration,'end callbacks observe the final state');
      assert(events.every(event=>event.trusted && !event.bubbles && !event.cancelable && event.realm),'native event identity and flags');
      const position=media.currentTime,count=events.length;await wait(30);
      assert(media.currentTime===position && events.length===count,'an ended clock remains stopped');
    }],
    ['play restarts an ended resource without loop',async(owner,callee,media,source)=>{
      await ready(media,source);await restart(callee,media,false);
    }],
    ['play restarts an ended resource after loop is enabled',async(owner,callee,media,source)=>{
      await ready(media,source);await restart(callee,media,true);
    }],
    ['playing loops perform a real seek at positive and zero rate',async(owner,callee,media,source)=>{
      await ready(media,source);media.loop=true;
      let notifications=0,ended=0;media.addEventListener('seeked',()=>notifications++);media.addEventListener('ended',()=>ended++);
      for(const rate of [1,0]) {
        set(callee,media,'playbackRate',rate);await callee.HTMLMediaElement.prototype.play.call(media);
        const before=notifications;set(callee,media,'currentTime',media.duration);
        await until(()=>notifications>=before+2 && !media.seeking,'seek-to-end must be followed by a loop seek');
        assert(!media.paused && !media.ended && media.currentTime<media.duration && ended===0,'loop remains playing from the beginning');
        if(rate===0){const time=media.currentTime;await wait(30);assert(time===0 && media.currentTime===time,'zero rate rewinds and then remains stationary');}
      }
    }],
    ['HTML end steps also rewind a paused looping seek',async(owner,callee,media,source)=>{
      await ready(media,source);media.loop=true;
      let notifications=0;media.addEventListener('seeked',()=>notifications++);
      set(callee,media,'currentTime',media.duration);
      await until(()=>notifications===2 && !media.seeking,'the HTML end-of-resource loop step must seek to the beginning');
      assert(media.currentTime===0 && media.paused && !media.ended,'looping a seek does not start playback');
    }],
    ['zero rate still ends an explicit non-looping seek',async(owner,callee,media,source)=>{
      await ready(media,source);set(callee,media,'playbackRate',0);
      await callee.HTMLMediaElement.prototype.play.call(media);
      let ended=false;media.addEventListener('ended',()=>ended=true,{once:true});
      await seek(callee,media,media.duration);
      assert(media.ended,'zero playbackRate has forward direction');
      await until(()=>ended,'an explicit end seek must end even at zero rate');
      assert(media.paused && media.currentTime===media.duration,'the zero-rate end task pauses at EOF');
    }],
    ['resource replacement from the final timeupdate retires old end work',async(owner,callee,media,source)=>{
      await ready(media,source);let replaced=false,ended=0;
      media.addEventListener('ended',()=>ended++);
      media.addEventListener('timeupdate',()=>{
        if(!replaced && !media.seeking && media.currentTime>=media.duration){
          replaced=true;media.removeAttribute('src');callee.HTMLMediaElement.prototype.load.call(media);
        }
      });
      await seek(callee,media,media.duration-0.05);await callee.HTMLMediaElement.prototype.play.call(media);
      await until(()=>replaced,'the final timeupdate must be observable');await wait(30);
      assert(ended===0 && media.readyState===0 && Number.isNaN(media.duration),'the old end task cannot publish into a replacement resource');
    }],
  ];
  try {
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const kind of ['audio','video'])for(const transport of ['data','http']) {
        for(const [name,body] of cases) {
          const label=`${ownerName}/${calleeName}/${kind}/${transport}/${name}`;progress.stage=label;
          const media=owner.document.createElement(kind);owner.document.body.appendChild(media);
          try{await body(owner,callee,media,__mediaEndSources[transport][kind]);checks.push({name:label,passed:true});}
          catch(error){checks.push({name:label,passed:false,detail:String(error)});}
          finally{media.pause();media.removeAttribute('src');callee.HTMLMediaElement.prototype.load.call(media);media.remove();}
        }
      }
  }catch(error){checks.push({name:'end fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
