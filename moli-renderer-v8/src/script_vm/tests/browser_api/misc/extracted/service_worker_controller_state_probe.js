globalThis.probeDone=false;globalThis.results={held:[],after:[],unsupported:[],activationEntered:null};
let ready=new Set(),frames=[],enteredResolve;
const entered=new Promise(resolve=>{enteredResolve=resolve;});
function receiveFixtureMessage(e){
 const d=e.data;if(!d)return;
 if(d.kind==='client-ready')ready.add(d.mode);
 else if(d.kind==='held')results.held.push(d.row);
 else if(d.kind==='after')results.after.push(d.row);
 else if(d.kind==='unsupported-worker')results.unsupported.push(d.type);
 else if(d.kind==='probe-error'){results.error=d.error;probeDone=true;}
 else if(d.kind==='activate-entered'){results.activationEntered=d;enteredResolve();}
}
addEventListener('message',receiveFixtureMessage);
navigator.serviceWorker.addEventListener('message',receiveFixtureMessage);
navigator.serviceWorker.startMessages();
async function until(predicate){while(!predicate())await new Promise(resolve=>setTimeout(resolve,5));}
async function active(reg){const w=reg.installing||reg.waiting||reg.active;if(w.state!=='activated')await new Promise(resolve=>w.addEventListener('statechange',()=>{if(w.state==='activated')resolve();}));return w;}
(async()=>{
 const one=await navigator.serviceWorker.register('/control/first.js',{scope:'/control/'});await active(one);
 for(const mode of ['eager','lazy']){const f=document.createElement('iframe');f.src='/control/client.html?mode='+mode;document.body.appendChild(f);frames.push(f);await until(()=>ready.has(mode));}
 await until(()=>ready.size===2);
 const two=await navigator.serviceWorker.register('/control/second.js',{scope:'/control/'});
 await entered;
 frames[1].contentWindow.postMessage('inspect','*');
 const expected=4-results.unsupported.length;
 await until(()=>results.held.length===expected);
 results.rootRegistrationBeforeRelease=two.active.state;
 two.active.postMessage('release');
 await active(two);
 for(const f of frames)f.contentWindow.postMessage('after','*');
 const targets=await (new Promise(resolve=>{const handler=e=>{if(e.data&&e.data.kind==='broadcast-done'){navigator.serviceWorker.removeEventListener('message',handler);resolve(true);}};navigator.serviceWorker.addEventListener('message',handler);two.active.postMessage('broadcast-after');}));
 await until(()=>results.after.length===expected);
 probeDone=true;
})().catch(e=>{results.error=String(e);probeDone=true;});
