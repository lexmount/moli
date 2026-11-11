
globalThis.policyRows=[];globalThis.policyError=null;globalThis.policyDone=false;
(async()=>{
 const childSource='<script>onmessage=event=>event.source.postMessage({enabled:document.fullscreenEnabled,origin:origin,url:location.href},"*")</script>';
 for(const source of ['data','tuple']) {
 for(const allow of [null, "fullscreen", "fullscreen 'src'", "fullscreen 'self'", "fullscreen 'none'", "fullscreen *", "payment;fullscreen"]) {
  const frame=document.createElement('iframe');
  if(allow!==null)frame.setAttribute('allow',allow);
  const result=new Promise((resolve,reject)=>{
   const listener=event=>{
    if(event.source!==frame.contentWindow)return;
    removeEventListener('message',listener);
    resolve({source,allow,fullscreenAttribute:false,observed:event.data,sourceMatches:true});
   };
   addEventListener('message',listener);
   frame.onerror=()=>reject(Error('frame loading failed'));
  });
  frame.onload=()=>frame.contentWindow.postMessage('inspect','*');
  frame.src=source==='tuple'?location.origin.replace('127.0.0.1','localhost')+'/child':'data:text/html,'+encodeURIComponent(childSource);
  document.body.appendChild(frame);
  policyRows.push(await result);frame.remove();
 }
 const frame=document.createElement('iframe');frame.setAttribute('allowfullscreen','');
 const result=new Promise(resolve=>{
  const listener=event=>{
   if(event.source!==frame.contentWindow)return;
   removeEventListener('message',listener);
   resolve({source,allow:null,fullscreenAttribute:true,observed:event.data,sourceMatches:true});
  };addEventListener('message',listener);
 });
 frame.onload=()=>frame.contentWindow.postMessage('inspect','*');
 frame.src=source==='tuple'?location.origin.replace('127.0.0.1','localhost')+'/child':'data:text/html,'+encodeURIComponent(childSource);document.body.appendChild(frame);
 policyRows.push(await result);frame.remove();}
 policyDone=true;
})().catch(error=>{policyError=String(error);policyDone=true;});
