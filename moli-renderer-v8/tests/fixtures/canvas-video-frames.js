(async () => {
  const checks=[];
  const progress=globalThis.__videoFrameProgress={stage:'setup',checks};
  const assert=(condition,message)=>{if(!condition)throw Error(message);};
  const set=(realm,media,name,value)=>Object.getOwnPropertyDescriptor(realm.HTMLMediaElement.prototype,name).set.call(media,value);
  // Read frames in the completion event's Promise reaction, before another
  // timer task can run. The native event must already expose the decoded frame.
  function ready(owner,callee,video,url) {
    return new Promise((resolve,reject)=>{
      video.oncanplaythrough=event=>{
        video.oncanplaythrough=video.onerror=null;
        try{assert(event.isTrusted && event.target===video && event instanceof owner.Event,'native media event identity');resolve();}catch(error){reject(error);}
      };
      video.onerror=()=>reject(Error('video resource failed '+url.slice(0,80)));
      set(callee,video,'preload','auto');set(callee,video,'src',url);
    });
  }
  function context(realm,offscreen,width=20,height=20) {
    const canvas=offscreen?new realm.OffscreenCanvas(width,height):realm.document.createElement('canvas');
    canvas.width=width;canvas.height=height;return canvas.getContext('2d');
  }
  const colors=[[255,0,0,255],[0,255,3,255],[0,0,255,255],[11,1,7,255]];
  function corners(ctx,expected=colors,size=20) {
    for(let i=0;i<4;i++) {
      const x=(i%2?3:1)*size/4,y=(i<2?1:3)*size/4;
      const actual=ctx.getImageData(x,y,1,1).data;
      for(let c=0;c<4;c++)assert(Math.abs(actual[c]-expected[i][c])<=3,`corner ${i} channel ${c}: ${Array.from(actual)} expected ${expected[i]}`);
    }
  }
  function bytes(ctx,source) {
    ctx.canvas.width=source.width;ctx.canvas.height=source.height;ctx.drawImage(source,0,0);
    return ctx.getImageData(0,0,source.width,source.height).data;
  }
  function sameBytes(actual,expected) {
    if(actual.length!==expected.length)return false;
    for(let i=0;i<actual.length;i++)if(actual[i]!==expected[i])return false;
    return true;
  }
  function seek(callee,video,time) {
    return new Promise((resolve,reject)=>{
      video.onseeked=()=>{progress.operation='seeked/'+time;video.onseeked=null;try{assert(video.currentTime===time && !video.seeking,'seek position '+video.currentTime+' expected '+time+' seeking '+video.seeking);resolve();}catch(error){reject(error);}};
      set(callee,video,'currentTime',time);
    });
  }
  const cases=[
    ['decoded dimensions and Canvas pixels',async(owner,callee,video,url,ctx)=>{
      await ready(owner,callee,video,url);
      assert(video.videoWidth===20 && video.videoHeight===20 && video.readyState===4,'decoded natural size and availability');
      ctx.drawImage(video,0,0);corners(ctx);
    }],
    ['ImageBitmap crop resize flip and native metadata',async(owner,callee,video,url,ctx,transport,offscreen)=>{
      await ready(owner,callee,video,url);
      Object.defineProperty(video,'videoWidth',{value:999,configurable:true});
      Object.defineProperty(video,'videoHeight',{value:999,configurable:true});
      const promise=callee.createImageBitmap(video,{imageOrientation:'flipY'});
      assert(promise instanceof callee.Promise,'bitmap promise realm');
      const bitmap=await promise;
      assert(bitmap instanceof callee.ImageBitmap && bitmap.width===20 && bitmap.height===20,'native snapshot dimensions and realm');
      ctx.drawImage(bitmap,0,0);corners(ctx,[colors[2],colors[3],colors[0],colors[1]]);
      bitmap.close();
      const expected=context(callee,offscreen);expected.imageSmoothingEnabled=false;
      expected.drawImage(video,10,0,10,10,0,0,20,20);
      const crop=await callee.createImageBitmap(video,10,0,10,10,{resizeWidth:20,resizeHeight:20,resizeQuality:'pixelated'});
      ctx.clearRect(0,0,20,20);ctx.drawImage(crop,0,0);
      const actual=ctx.getImageData(0,0,20,20).data,reference=expected.getImageData(0,0,20,20).data;
      assert(actual.every((value,index)=>value===reference[index]),'pixelated bitmap crop matches the Canvas source crop');crop.close();
    }],
    ['VideoFrame shares the decoded Canvas source',async(owner,callee,video,url,ctx)=>{
      await ready(owner,callee,video,url);
      const frame=new callee.VideoFrame(video,{timestamp:123});
      assert(frame.displayWidth===20 && frame.displayHeight===20 && frame.timestamp===123,'VideoFrame image-source metadata');
      ctx.drawImage(frame,0,0);corners(ctx);frame.close();
    }],
    ['reload and clone discard resource state but retain snapshots',async(owner,callee,video,url,ctx)=>{
      await ready(owner,callee,video,url);
      const bitmap=await callee.createImageBitmap(video);
      const clone=video.cloneNode();
      assert(clone.readyState===0 && clone.videoWidth===0 && clone.videoHeight===0,'clones do not inherit decoded resources');
      video.removeAttribute('src');callee.HTMLMediaElement.prototype.load.call(video);
      assert(video.readyState===0 && video.videoWidth===0 && video.videoHeight===0,'load resets resource dimensions');
      let error;await callee.createImageBitmap(video).catch(value=>error=value);
      assert(error instanceof callee.DOMException && error.name==='InvalidStateError','an empty video cannot be snapshotted');
      ctx.drawImage(bitmap,0,0);corners(ctx);bitmap.close();
    }],
    ['seek selects new frames and rewinds reference state',async(owner,callee,video,url,ctx,transport)=>{
      progress.operation='counting-ready';
      await ready(owner,callee,video,__videoFrameSources[transport].counting);
      progress.operation='first-bitmap';
      const first=await callee.createImageBitmap(video),initial=bytes(ctx,first);
      progress.operation='forward-seek';
      await seek(callee,video,2.5);
      progress.operation='later-bitmap';
      const later=await callee.createImageBitmap(video);
      assert(!sameBytes(bytes(ctx,later),initial),'seek must produce a genuinely different decoded frame');
      assert(sameBytes(bytes(ctx,first),initial),'an earlier bitmap remains immutable');
      progress.operation='rewind-seek';
      await seek(callee,video,0);
      progress.operation='rewound-bitmap';
      const rewound=await callee.createImageBitmap(video);
      assert(sameBytes(bytes(ctx,rewound),initial),'rewinding replays codec references');
      first.close();later.close();rewound.close();
    }],
    ['VP9 frames share native allocation limits and timestamps',async(owner,callee,video,url,ctx,transport)=>{
      await ready(owner,callee,video,__videoFrameSources[transport].vp9);
      assert(video.videoWidth===16 && video.videoHeight===16,'VP9 dimensions');
      const initial=await callee.createImageBitmap(video);
      for(const [time,value] of [[0,0],[1,130],[2,255]]) {
        if(time)await seek(callee,video,time);
        ctx.clearRect(0,0,20,20);ctx.drawImage(video,0,0);
        const pixel=ctx.getImageData(8,8,1,1).data;
        for(let c=0;c<3;c++)assert(Math.abs(pixel[c]-value)<=1,'VP9 sample '+Array.from(pixel)+' expected '+value);
        assert(pixel[3]===255,'VP9 opaque alpha');
      }
      ctx.drawImage(initial,0,0);assert(ctx.getImageData(8,8,1,1).data[0]===0,'VP9 earlier snapshot remains immutable');initial.close();
    }],
  ];
  try {
    const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const transport of ['data','http'])for(const windowless of [false,true])for(const offscreen of [false,true]) {
        const scenario=`${ownerName}/${calleeName}/${transport}/${windowless?'windowless':'live'}/${offscreen?'offscreen':'html'}`;
        if(globalThis.__videoFrameScenario && globalThis.__videoFrameScenario !== scenario)continue;
        const doc=windowless?owner.document.implementation.createHTMLDocument(''):owner.document;
        for(const [name,body] of cases) {
          const video=doc.createElement('video');doc.body.appendChild(video);
          const label=`${ownerName}/${calleeName}/${transport}/${windowless?'windowless':'live'}/${offscreen?'offscreen':'html'}/${name}`;
          progress.stage=label;let timeout;
          try {
            await Promise.race([body(owner,callee,video,__videoFrameSources[transport].pattern,context(callee,offscreen),transport,offscreen),new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('video case timeout '+JSON.stringify({operation:progress.operation,currentTime:video.currentTime,seeking:video.seeking,readyState:video.readyState,error:video.error?.code,ownerClosed:owner.closed,calleeClosed:callee.closed,native:globalThis.__videoFrameNativeState?.(video)}))),2500);})]);
            if(windowless)assert(doc.defaultView===null && doc.hidden,'video resources do not create a synthetic Window');
            checks.push({name:label,category:windowless?'windowless':'live',passed:true});
          }catch(error){checks.push({name:label,category:windowless?'windowless':'live',passed:false,detail:String(error)});}
          finally{clearTimeout(timeout);video.remove();video.removeAttribute('src');callee.HTMLMediaElement.prototype.load.call(video);}
        }
      }
  }catch(error){checks.push({name:'video fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
