const mode=new URL(location.href).searchParams.get('mode');
let controllerRef=null,events=[];
function inspect(label,registration){
 const before=registration&&registration.active&&registration.active.state;
 const controller=navigator.serviceWorker.controller;controllerRef=controller;
 if(controller)controller.addEventListener('statechange',()=>events.push(controller.state));
 const first=controller&&controller.state;
 const again=navigator.serviceWorker.controller;
 const row={label,before,first,repeated:again&&again.state,same:controller===again,alias:registration?controller===registration.active:null,registrationAfter:registration&&registration.active&&registration.active.state,url:controller&&controller.scriptURL};
 queueMicrotask(()=>{row.microtask=controller&&controller.state;parent.postMessage({kind:'held',row},'*');});
}
if(mode==='eager'){
 const sw=navigator.serviceWorker;const first=sw.controller;
 sw.oncontrollerchange=async()=>{inspect('window-cached',await sw.getRegistration(location.href));};
 let ready=0;
 (async()=>{for(const type of ['classic','module']){
  await new Promise(resolve=>{const w=new Worker('/control/client-worker.js?type='+type,{type});
  w.onmessage=e=>{
   if(e.data.kind==='ready'){resolve();}
   else parent.postMessage(e.data,'*');
  };});
 }parent.postMessage({kind:'client-ready',mode},'*');})();
}else parent.postMessage({kind:'client-ready',mode},'*');
addEventListener('message',e=>{
 if(e.data==='inspect')inspect('window-first-read',null);
 if(e.data==='after'){
  const before=controllerRef&&controllerRef.state;const current=navigator.serviceWorker.controller;
  parent.postMessage({kind:'after',row:{label:mode==='eager'?'window-cached':'window-first-read',before,after:current&&current.state,same:current===controllerRef,events}},'*');
 }
});
