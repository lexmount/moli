async base => {
 const checks=[],observations={}; const p=performance; const Entry=PerformanceResourceTiming;
 const test=(name,callback)=>{try{if(!callback())throw Error('assertion failed');checks.push({name,passed:true});}catch(error){checks.push({name,passed:false,details:String(error)});}};
 const tick=()=>new Promise(resolve=>setTimeout(resolve,0));
 const rows=url=>p.getEntriesByName(url,'resource');
 // Opaque text() reads a null public body; its network terminal is a
 // separate task. Observe that task instead of treating one timer as a barrier.
 const waitForEntry=url=>rows(url).length?Promise.resolve():new Promise((resolve,reject)=>{
  const observer=new PerformanceObserver(list=>{
   if(list.getEntries().some(entry=>entry.name===url)){clearTimeout(timer);observer.disconnect();resolve();}
  });
  observer.observe({type:'resource',buffered:true});
  const timer=setTimeout(()=>{observer.disconnect();reject(Error('Missing resource entry: '+url));},2000);
 });
 const snapshot=url=>rows(url).map(e=>({name:e.name,initiatorType:e.initiatorType,startTime:e.startTime,duration:e.duration,responseStart:e.responseStart,responseEnd:e.responseEnd,responseStatus:e.responseStatus,transferSize:e.transferSize,encodedBodySize:e.encodedBodySize,decodedBodySize:e.decodedBodySize,nextHopProtocol:e.nextHopProtocol}));
 const verify=(label,url,initiator,status)=>{
  const e=rows(url)[0]; observations[label]=snapshot(url);
  test(label+': one entry by completion',()=>rows(url).length===1);
  test(label+': native resource entry',()=>e instanceof Entry&&e.entryType==='resource'&&e.initiatorType===initiator);
  test(label+': requested URL',()=>e.name===url);
  test(label+': status',()=>e.responseStatus===status);
  test(label+': monotonic timing',()=>e.startTime>=0&&e.duration>=0&&e.responseEnd>=e.startTime&&e.responseEnd<=p.now()+.1);
 };
 for(const [label,path] of [['fetch','/timing-data#fragment'],['fetch 404','/timing-404'],['fetch redirect','/timing-redirect']]){
  p.clearResourceTimings();const url=base+path;const r=await fetch(url);await r.text();verify(label,url,'fetch',label==='fetch 404'?404:200);
 }
 p.clearResourceTimings();{
  const url=base+'/timing-delay';const response=await fetch(url);const clone=response.clone();
  test('streaming fetch waits for EOF',()=>rows(url).length===0);
  await response.text();verify('streaming fetch',url,'fetch',200);await clone.text();test('cloned response does not duplicate timing',()=>rows(url).length===1);
 }
 const xhr=(url,sync)=>sync?new Promise((resolve,reject)=>{try{const x=new XMLHttpRequest();x.open('GET',url,false);x.send();resolve(x);}catch(e){reject(e);}}):new Promise((resolve,reject)=>{const x=new XMLHttpRequest();x.open('GET',url);x.onload=()=>resolve(x);x.onerror=()=>reject(Error('XHR error'));x.send();});
 for(const sync of [false,true]){
  p.clearResourceTimings();const url=base+'/timing-data?xhr='+sync;await xhr(url,sync);verify('XHR '+sync,url,'xmlhttprequest',200);
 }
 for(const tao of [false,true]){
  p.clearResourceTimings();const url=base.replace('127.0.0.1','localhost')+'/timing-data?cors=1'+(tao?'&tao=1':'');await(await fetch(url)).text();verify('CORS '+tao,url,'fetch',200);
  test('CORS '+tao+': readable body sizes',()=>rows(url)[0].decodedBodySize>0);
  test('CORS '+tao+': TAO connection fields',()=>tao?rows(url)[0].responseStart>0&&rows(url)[0].transferSize>0:rows(url)[0].responseStart===0&&rows(url)[0].transferSize===0);
 }
 for(const tao of [false,true]){
  p.clearResourceTimings();const url=base.replace('127.0.0.1','localhost')+'/timing-data?opaque=1'+(tao?'&tao=1':'');await(await fetch(url,{mode:'no-cors'})).text();await waitForEntry(url);observations['opaque '+tao]=snapshot(url);
 }
 for(const [label,path]of [['broken body','/timing-broken'],['CORS rejection','/timing-cors-deny']]){
  p.clearResourceTimings();const url=(label==='CORS rejection'?base.replace('127.0.0.1','localhost'):base)+path;try{await(await fetch(url)).text();}catch{}await tick();observations[label]=snapshot(url);
 }
 for(const afterHeaders of [false,true]){
  p.clearResourceTimings();const url=base+'/timing-hang?abort='+afterHeaders;const c=new AbortController();const promise=fetch(url,{signal:c.signal});
  if(afterHeaders){const r=await promise;c.abort();try{await r.text();}catch{}}
  else{c.abort();try{await promise;}catch{}}
  await tick();observations['abort '+afterHeaders]=snapshot(url);
 }
 p.clearResourceTimings();await(await fetch('data:text/plain,local')).text();await tick();observations.local=p.getEntriesByType('resource').map(e=>e.name);
 const descriptor=Object.getOwnPropertyDescriptor(globalThis,'performance');const url=base+'/timing-data?shadow=1';
 try{Object.defineProperty(globalThis,'performance',{value:{},configurable:true});await(await fetch(url)).text();}
 finally{if(descriptor)Object.defineProperty(globalThis,'performance',descriptor);else delete globalThis.performance;}
 test('native Performance survives author shadowing',()=>rows(url).length===1);

 test('opaque body information remains hidden',()=>[false,true].every(tao=>{
  const entries=observations['opaque '+tao];
  return entries.length===1&&entries[0].responseStatus===0&&entries[0].encodedBodySize===0&&entries[0].decodedBodySize===0;
 }));
 test('broken body retains status and received bytes',()=>{
  const e=observations['broken body'][0];return e&&e.responseStatus===200&&e.encodedBodySize===27&&e.decodedBodySize===27;
 });
 test('CORS rejection hides response metadata',()=>{
  const e=observations['CORS rejection'][0];return e&&e.responseStatus===0&&e.responseStart===0&&e.transferSize===0&&e.decodedBodySize===0;
 });
 test('early abort reports one failed entry',()=>{
  const entries=observations['abort false'];const e=entries[0];return entries.length===1&&e.responseStatus===0&&e.responseStart===0&&e.transferSize===0;
 });
 test('abort after headers retains allowed connection timing',()=>{
  const entries=observations['abort true'];const e=entries[0];return entries.length===1&&e.responseStatus===0&&e.responseStart>0&&e.transferSize===300&&e.decodedBodySize===0;
 });
 test('data URL does not create a resource entry',()=>observations.local.length===0);
 for(const sync of [false,true]){
  p.clearResourceTimings();const url=base.replace('127.0.0.1','localhost')+'/timing-data?corsXhr='+sync;
  await xhr(url,sync);verify('CORS XHR '+sync,url,'xmlhttprequest',200);
  test('CORS XHR '+sync+': readable sizes without TAO',()=>rows(url)[0].decodedBodySize===27&&rows(url)[0].transferSize===0);
  p.clearResourceTimings();const broken=base+'/timing-broken?xhr='+sync;
  try{await xhr(broken,sync);}catch{}
  const entries=rows(broken);const e=entries[0];observations['broken XHR '+sync]=snapshot(broken);
  test('broken XHR '+sync+': retains received bytes',()=>entries.length===1&&e.responseStatus===200&&e.decodedBodySize===27);
 }
 for(const kind of ['abort','timeout']){
  p.clearResourceTimings();const url=base+'/timing-hang?xhr='+kind;const x=new XMLHttpRequest();
  await new Promise(resolve=>{
   x.open('GET',url);x.onloadend=resolve;if(kind==='timeout')x.timeout=20;
   x.send();if(kind==='abort')x.abort();
  });
  await tick();const entries=rows(url);const e=entries[0];observations['XHR '+kind]=snapshot(url);
  test('XHR '+kind+': one failure before loadend',()=>entries.length===1&&e.responseStatus===0&&e.decodedBodySize===0);
 }
 p.clearResourceTimings();{
  const old=base+'/timing-hang?xhr=replaced';const url=base+'/timing-data?xhr=replacement';const x=new XMLHttpRequest();let aborts=0;
  x.onabort=()=>aborts++;
  await new Promise((resolve,reject)=>{
   x.open('GET',old);x.send();x.open('GET',url);x.onload=resolve;x.onerror=reject;x.send();
  });
  verify('reopened XHR',url,'xmlhttprequest',200);await tick();const entries=rows(old);
  test('XHR open retires the old timing owner',()=>entries.length===1&&entries[0].responseStatus===0);
  test('XHR open does not dispatch abort',()=>aborts===0);
 }
 p.clearResourceTimings();{
  const url=base+'/timing-data?fromImportedScript=1';
  importScripts(base.replace('127.0.0.1','localhost')+'/timing-origin-import.js');
  await globalThis.__timingImportedFetch;
  verify('fetch from imported script',url,'fetch',200);
  test('imported script keeps the Worker origin',()=>rows(url)[0].responseStart>0&&rows(url)[0].transferSize>0);
  const body=await globalThis.__timingImportedNoCorsFetch;
  const noCors=rows(base+'/timing-html?fromImportedScript=1');
  test('imported script no-cors HTML stays readable for its Worker origin',()=>body==='/* worker transport body */');
  test('imported script no-cors timing remains same-origin',()=>noCors.length===1&&noCors[0].responseStatus===200&&noCors[0].responseStart>0);

 }
 p.clearResourceTimings();{
  const url=base+'/timing-delay?lateAbort=1';const controller=new AbortController();
  const response=await fetch(url,{signal:controller.signal});
  const blockStart=p.now();while(p.now()-blockStart<450){}
  const blockEnd=p.now();controller.abort();try{await response.text();}catch{}
  await waitForEntry(url);const entries=rows(url);const e=entries[0];observations.lateAbort=snapshot(url);
  test('late JS abort preserves completed bytes and reports abort status',()=>entries.length===1&&e.responseStatus===0&&e.decodedBodySize===27);
  test('response end precedes delayed Worker delivery',()=>e.responseEnd<blockEnd-50);
 }
 return {checks,observations};
}
