async function () {
return (async()=>{
 const controlled=navigator.serviceWorker.controller?Promise.resolve():new Promise(resolve=>navigator.serviceWorker.addEventListener('controllerchange',resolve,{once:true}));
 await navigator.serviceWorker.register('/probe-worker.js');await navigator.serviceWorker.ready;await controlled;
 const rows=[];
 for(const destination of ['script','fetch']) for(const source of ['basic','cors','default','opaque']) for(const bad of [false,true]){
  const token='sw'+rows.length,url=new URL('/probe-worker-asset',location.href);Object.entries({source,token,as:'script'}).forEach(([k,v])=>url.searchParams.set(k,v));
  await fetch('/probe-release?token='+token);
  const link=document.createElement('link');Object.assign(link,{rel:'preload',as:destination,href:url.href,integrity:bad?'sha384-AAAA':''});
  const linkEvent=await new Promise(resolve=>{link.onload=link.onerror=e=>resolve(e.type);document.head.append(link)});
  let consumer;
  if(destination==='fetch'){
   try{const response=await fetch(url,{mode:'no-cors',credentials:'include'});consumer={event:'load',type:response.type,text:await response.text(),status:response.status};}
   catch(e){consumer={event:'error',name:e.name};}
  }else{
   const script=document.createElement('script');script.src=url;
   consumer=await new Promise(resolve=>{script.onload=script.onerror=e=>resolve({event:e.type});document.body.append(script)});script.remove();
  }
  const counts=await(await fetch('/probe-worker-counts')).json();
  rows.push({destination,source,bad,linkEvent,consumer,events:counts[url.href],timing:performance.getEntriesByName(url.href).map(e=>e.initiatorType)});link.remove();
 }
 return {rows,failures:rows.filter(r=>r.events!==1||r.linkEvent!==(r.bad?'error':'load')||r.consumer.event!==(r.bad?'error':'load')||r.timing.length!==1 ||(!r.bad&&r.destination==='fetch'&&r.source==='opaque'&&(r.consumer.type!=='opaque'||r.consumer.text!==''||r.consumer.status!==0)))};
})();
}
