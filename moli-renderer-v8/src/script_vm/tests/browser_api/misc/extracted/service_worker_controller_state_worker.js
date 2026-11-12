const type=new URL(location.href).searchParams.get('type');
(async()=>{
 const sw=navigator.serviceWorker;
 if(!sw){postMessage({kind:'unsupported-worker',type});postMessage({kind:'ready'});return;}
 await sw.getRegistration(location.href);
 let controllerRef=null,events=[];
 sw.oncontrollerchange=async()=>{
  const reg=await sw.getRegistration(location.href);const before=reg.active&&reg.active.state;
  const c=sw.controller;controllerRef=c;c.addEventListener('statechange',()=>events.push(c.state));
  const row={label:'worker-'+type,before,first:c.state,repeated:sw.controller.state,same:c===sw.controller,alias:c===reg.active,registrationAfter:reg.active.state,url:c.scriptURL};
  queueMicrotask(()=>{row.microtask=c.state;postMessage({kind:'held',row});});
 };
 sw.addEventListener('message',e=>{
  if(e.data&&e.data.kind==='after-release'){
   const before=controllerRef&&controllerRef.state;const current=sw.controller;
   postMessage({kind:'after',row:{label:'worker-'+type,before,after:current&&current.state,same:current===controllerRef,events}});
  }
 });
 postMessage({kind:'ready'});
})().catch(e=>postMessage({kind:'probe-error',error:String(e)}));
