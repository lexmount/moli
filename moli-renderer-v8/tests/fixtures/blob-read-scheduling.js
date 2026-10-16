(async () => {
  const checks=[];
  globalThis.__blobReadProgress={stage:'setup',checks};
  async function runMatrix(owner,callee,prefix,checks) {
    for(const kind of ['Blob','File']) for(const name of ['text','arrayBuffer','bytes','stream']) {
      const source=kind==='Blob'?new owner.Blob(['data']):new owner.File(['data'],'file');
      const value=callee.Blob.prototype[name].call(source);
      const reader=name==='stream'?value.getReader():null;
      const promise=reader?reader.read():value;
      let settled=false;
      promise.then(()=>{settled=true;},()=>{settled=true;});
      const beforeTask=await new owner.Promise(resolve=>owner.queueMicrotask(()=>resolve(settled)));
      const result=await promise;
      checks.push({name:`${prefix}/${kind}/${name}/file-reading-task-before-settlement`,passed:beforeTask===false,detail:{settledDuringFirstMicrotask:beforeTask}});
      if(reader)reader.releaseLock();
    }
  }
  const frame=document.createElement('iframe');document.body.appendChild(frame);
  const realms=[['top',window],['child',frame.contentWindow]];
  for(const [ownerName,owner]of realms)for(const [calleeName,callee]of realms)await runMatrix(owner,callee,`${ownerName}/${calleeName}`,checks);
  const body=async runMatrix=>{const checks=[];await runMatrix(globalThis,globalThis,'worker',checks);postMessage(checks);close();};
  const url=URL.createObjectURL(new Blob([`(${body})(${runMatrix})`],{type:'text/javascript'}));const worker=new Worker(url);
  try{checks.push(...await new Promise((resolve,reject)=>{worker.onmessage=e=>resolve(e.data);worker.onerror=e=>reject(Error(e.message));}));}
  finally{worker.terminate();URL.revokeObjectURL(url);frame.remove();}
  globalThis.__uiEventResults={checks};
})().catch(error=>{const checks=__blobReadProgress.checks;checks.push({name:'fixture completion',passed:false,detail:String(error.stack||error)});globalThis.__uiEventResults={checks};});
