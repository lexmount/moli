function facts() {
 const result={secure:isSecureContext,isolated:crossOriginIsolated,constructor:typeof SharedArrayBuffer};
 try {
  const memory=new WebAssembly.Memory({shared:true,initial:1,maximum:1});
  const buffer=memory.buffer;
  const clone=structuredClone(buffer);
  new Uint8Array(buffer)[0]=37;
  result.clone={distinct:clone!==buffer,shared:new Uint8Array(clone)[0]===37};
 } catch(error) {result.clone={error:error.name};}
 return result;
}
if ('registration' in self) {
 self.oninstall=event=>event.waitUntil(skipWaiting());
 self.onmessage=event=>event.ports[0].postMessage(facts());
} else {
 self.onconnect=event=>{
  const port=event.ports[0];port.onmessage=()=>port.postMessage(facts());port.start();
 };
}