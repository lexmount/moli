(async () => {
  const urls=[];
  const makeURL=source=>{const url=URL.createObjectURL(new Blob([source],{type:'text/javascript'}));urls.push(url);return url;};
  const makeTree=(depth,name,scriptType='classic')=>{
    let url=makeURL(`navigator.locks.request(${JSON.stringify(name)},()=>{postMessage({kind:'held'});return new Promise(()=>{});});`);
    for(let i=0;i<depth;i++)url=makeURL(`const child=new Worker(${JSON.stringify(url)},{type:${JSON.stringify(scriptType)}});child.onmessage=event=>postMessage(event.data);child.onerror=event=>postMessage({kind:'error',message:event.message});onmessage=event=>{if(event.data==='close')close();if(event.data==='terminate-child'){child.terminate();postMessage({kind:'terminated'});}};`);
    return new Worker(url,{type:scriptType});
  };
  const message=(worker,kind)=>new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{cleanup();reject(new Error('missing worker '+kind));},5000);
    const onmessage=event=>{if(event.data.kind==='error'){cleanup();reject(new Error(event.data.message));}else if(event.data.kind===kind){cleanup();resolve();}};
    const onerror=event=>{cleanup();reject(new Error(event.message||'worker failed'));};
    function cleanup(){clearTimeout(timer);worker.removeEventListener('message',onmessage);worker.removeEventListener('error',onerror);}
    worker.addEventListener('message',onmessage);worker.addEventListener('error',onerror);
  });
  const snapshot=async()=>await navigator.locks.query();
  const released=async name=>{
    const deadline=performance.now()+3000;
    do{const value=await snapshot();if(!value.held.some(lock=>lock.name===name))return {released:true,snapshot:value};await new Promise(resolve=>setTimeout(resolve,10));}while(performance.now()<deadline);
    return {released:false,snapshot:await snapshot()};
  };
  const controlName='C58-unrelated-live-worker';
  const control=makeTree(0,controlName);await message(control,'held');
  const cases=[];
  try{
    for(const scriptType of ['classic','module']) for(const [depth,mode] of [[0,'terminate'],[1,'terminate'],[2,'terminate'],[1,'close'],[1,'terminate-child']]){
      const name=`C58-owned-worker-${scriptType}-${depth}-${mode}`;
      const worker=makeTree(depth,name,scriptType);await message(worker,'held');
      const before=await snapshot();
      if(mode==='terminate')worker.terminate();
      else if(mode==='close')worker.postMessage('close');
      else {const done=message(worker,'terminated');worker.postMessage('terminate-child');await done;}
      const after=await released(name);
      cases.push({scriptType,depth,mode,heldBefore:before.held.some(lock=>lock.name===name),released:after.released,controlRetained:after.snapshot.held.some(lock=>lock.name===controlName),snapshotAfter:after.snapshot});
      worker.terminate();
    }
  }finally{control.terminate();for(const url of urls)URL.revokeObjectURL(url);}
  const controlAfter=await released(controlName);
  return {cases,controlReleased:controlAfter.released,remainingHeld:controlAfter.snapshot.held};
})()
