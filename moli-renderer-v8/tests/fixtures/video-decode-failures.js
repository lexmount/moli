(async () => {
  const checks=[];
  const progress=globalThis.__videoDecodeFailureProgress={stage:'setup',checks};
  const assert=(ok,message)=>{if(!ok)throw Error(message);};
  for(const windowless of [false,true]) {
    const media=[];
    const name=(windowless?'windowless':'live')+'/failed decoders release admission for valid video';
    let timeout;
    try {
      await Promise.race([(async()=>{
        const doc=windowless?document.implementation.createHTMLDocument(''):document;
        progress.stage=name+'/invalid resources';
        await Promise.all(Array.from({length:100},()=>new Promise((resolve,reject)=>{
          const item=doc.createElement('video');media.push(item);doc.body.appendChild(item);
          item.onloadeddata=()=>reject(Error('malformed WebM must not produce a decoded frame'));
          item.onerror=event=>{
            try {
              assert(event.isTrusted && event.target===item,'native media error identity');
              assert(item.error.code===4 && item.readyState===0 && item.videoWidth===0,'malformed media error state');
              resolve();
            }catch(error){reject(error);}
          };
          item.preload='auto';item.src='data:video/webm;base64,GkXfow==';
        })));
        progress.stage=name+'/valid resource after errors';
        const good=globalThis.failureProbeMedia=doc.createElement('video');
        media.push(good);doc.body.appendChild(good);
        await new Promise((resolve,reject)=>{
          good.onerror=()=>reject(Error('a valid video was starved after decode errors: '+good.error.code));
          good.oncanplaythrough=()=>{
            try {
              assert(good.videoWidth===20 && good.videoHeight===20 && good.readyState===4,'valid decoded video after errors');
              resolve();
            }catch(error){reject(error);}
          };
          good.preload='auto';good.src=__videoFrameSources.data.pattern;
        });
      })(),new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('video failure recovery timeout: '+progress.stage)),10000);})]);
      checks.push({name,category:windowless?'windowless':'live',passed:true});
    }catch(error){checks.push({name,category:windowless?'windowless':'live',passed:false,detail:String(error)});}
    finally {
      clearTimeout(timeout);
      for(const item of media){item.removeAttribute('src');item.load();item.remove();}
      await new Promise(resolve=>setTimeout(resolve,0));
    }
  }
  globalThis.__uiEventResults={checks,total:checks.length};
})()
