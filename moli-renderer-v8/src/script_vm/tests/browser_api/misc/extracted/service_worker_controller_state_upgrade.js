let release;
oninstall=e=>e.waitUntil(skipWaiting());
onactivate=e=>e.waitUntil((async()=>{
 const gate=new Promise(resolve=>{release=resolve;});
 const windows=await clients.matchAll({type:'window',includeUncontrolled:true});
 for(const c of windows)c.postMessage({kind:'activate-entered',ownState:registration.active&&registration.active.state});
 await gate;
})());
onmessage=e=>{
 if(e.data==='release')release();
 if(e.data==='broadcast-after')e.waitUntil((async()=>{
  for(const c of await clients.matchAll({type:'all',includeUncontrolled:true}))c.postMessage({kind:'after-release'});
  e.source.postMessage({kind:'broadcast-done'});
 })());
};
