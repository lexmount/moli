(async () => {
  const checks = [];
  const check = (name, passed, observed) => checks.push({name, passed, observed});
  const modes = ['cors', 'no-cors', 'same-origin'];
  async function read(url, mode) {
    try {
      const r = await fetch(url, {mode});
      return {type: r.type, status: r.status, text: await r.text()};
    } catch (e) { return {error: e.name}; }
  }
  function xhr(url) {
    return new Promise(resolve => {
      const x = new XMLHttpRequest(); x.open('GET', url);
      x.onload = () => resolve({status: x.status, text: x.responseText});
      x.onerror = () => resolve({error: 'NetworkError', status: x.status}); x.send();
    });
  }
  const peerSource = `const read=${read.toString()}; const xhr=${xhr.toString()};
    const own=URL.createObjectURL(new Blob(['peer'],{type:'text/plain'}));
    const tell=x=>${'typeof parent !== "undefined" ? parent.postMessage(x,"*") : postMessage(x)'};
    tell({kind:'ready',url:own});
    onmessage=async e=>{if(e.data.kind!=='audit')return;
      const rows=[];for(const [name,url] of [['own',own],['parent',e.data.parent],['foreign',e.data.foreign]]){
        for(const mode of ['cors','no-cors','same-origin'])rows.push({name,mode,...await read(url,mode)});
        rows.push({name,mode:'xhr',...await xhr(url)});
      }tell({kind:'done',rows});};`;
  const wait = (source, kind) => new Promise(resolve => {
    const listener = e => {if(e.source===source && e.data?.kind===kind){removeEventListener('message',listener);resolve(e.data);}};
    addEventListener('message',listener);
  });
  const opaque = document.createElement('iframe'); opaque.sandbox='allow-scripts';
  opaque.srcdoc='<script>'+peerSource+'<\/script>';
  document.body.append(opaque);
  const opaqueReady=await wait(opaque.contentWindow,'ready');
  const parentUrl=URL.createObjectURL(new Blob(['parent'],{type:'text/plain'}));
  const opaqueDone=wait(opaque.contentWindow,'done');
  opaque.contentWindow.postMessage({kind:'audit',parent:parentUrl,foreign:parentUrl},'*');
  for(const row of (await opaqueDone).rows){
    const own=row.name==='own';
    check('opaque '+row.name+' '+row.mode, own
      ? row.status===200 && row.text==='peer' && (row.mode==='xhr'||row.type==='basic')
      : !!row.error && (row.mode==='xhr'?row.status===0:row.error==='TypeError'), row);
  }
  const same = document.createElement('iframe'); same.srcdoc='<script>'+peerSource+'<\/script>';
  document.body.append(same);
  const sameReady=await wait(same.contentWindow,'ready');
  const sameDone=wait(same.contentWindow,'done');
  same.contentWindow.postMessage({kind:'audit',parent:parentUrl,foreign:opaqueReady.url},'*');
  for(const row of (await sameDone).rows){
    const allowed=row.name!=='foreign';
    check('same origin '+row.name+' '+row.mode, allowed
      ? row.status===200 && row.text===(row.name==='own'?'peer':'parent') && (row.mode==='xhr'||row.type==='basic')
      : !!row.error && (row.mode==='xhr'?row.status===0:row.error==='TypeError'), row);
  }
  const script=URL.createObjectURL(new Blob([peerSource],{type:'text/javascript'}));
  const worker=new Worker(script);
  const workerReady=await new Promise(resolve=>worker.onmessage=e=>resolve(e.data));
  const workerDone=new Promise(resolve=>worker.onmessage=e=>resolve(e.data));
  worker.postMessage({kind:'audit',parent:parentUrl,foreign:opaqueReady.url});
  for(const row of (await workerDone).rows){
    const allowed=row.name!=='foreign';
    check('worker '+row.name+' '+row.mode, allowed
      ? row.status===200 && row.text===(row.name==='own'?'peer':'parent') && (row.mode==='xhr'||row.type==='basic')
      : !!row.error && (row.mode==='xhr'?row.status===0:row.error==='TypeError'), row);
  }
  for(const [name,url] of [['same origin',sameReady.url],['worker',workerReady.url]]){
    for(const mode of modes){const r=await read(url,mode);check('parent reads '+name+' '+mode,r.status===200&&r.text==='peer'&&r.type==='basic',r);}
    const r=await xhr(url);check('parent reads '+name+' xhr',r.status===200&&r.text==='peer',r);
  }
  // The fetch receiver determines the environment when a method is borrowed.
  const foreignRequest=new same.contentWindow.Request(opaqueReady.url);
  try {await same.contentWindow.fetch.call(window,foreignRequest);check('borrowed fetch rejects foreign Request',false);}
  catch(e){check('borrowed fetch rejects foreign Request',e.name==='TypeError',e.name);}
  const r=await same.contentWindow.fetch.call(window,new Request(parentUrl,{mode:'no-cors'}));
  check('borrowed fetch reads own Request',r.type==='basic'&&await r.text()==='parent',r.type);
  worker.terminate();opaque.remove();same.remove();URL.revokeObjectURL(script);URL.revokeObjectURL(parentUrl);
  globalThis.__uiEventResults={checks,total:checks.length,passed:checks.filter(r=>r.passed).length,complete:true};
  return checks.every(r=>r.passed);
})()
