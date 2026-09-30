(async()=>{
 const rows=[],assert=(ok,message)=>{if(!ok)throw Error(message);},check=async(name,fn)=>{try{await fn();rows.push({name,pass:true});}catch(error){rows.push({name,pass:false,message:String(error)});}};
 const buffer=await new OfflineAudioContext(1,8,8000).startRendering();
 for(const kind of ['constant','buffer','buffer-cleared','borrowed-start','analyser'])await check(kind+'/unsupported render rejects Promise',async()=>{
  const context=new OfflineAudioContext(1,32,8000),node=kind==='constant'?context.createConstantSource():context.createBufferSource();
  const analyser=context.createAnalyser();
  if(kind!=='constant')node.buffer=buffer;
  if(kind==='analyser')node.connect(analyser);else {node.connect(analyser);analyser.connect(context.destination);}
  if(kind==='borrowed-start')AudioScheduledSourceNode.prototype.start.call(node);else node.start();
  if(kind==='buffer-cleared'||kind==='borrowed-start')node.buffer=null;
  const rendering=context.startRendering();assert(rendering instanceof Promise,'native rejected Promise');let error;try{await rendering;}catch(e){error=e;}
  assert(error instanceof DOMException&&error.name==='NotSupportedError','unimplemented PCM backend');assert(context.state==='suspended','failed render preserves context');
  const bins=new Float32Array(analyser.frequencyBinCount);analyser.getFloatFrequencyData(bins);assert(bins.every(x=>x===-Infinity),'failed render does not commit synthetic input');
 });
 for(const kind of ['null-buffer','future-constant','stopped-constant','disconnected-constant','zero-duration-buffer'])await check(kind+'/silence',async()=>{
  const context=new OfflineAudioContext(1,32,8000),node=kind.includes('buffer')?context.createBufferSource():context.createConstantSource();
  if(kind==='zero-duration-buffer'){node.buffer=buffer;node.start(0,0,0);}else node.start(kind==='future-constant'?1:0);
  if(kind==='stopped-constant')node.stop();
  if(kind!=='disconnected-constant')node.connect(context.destination);
  node.__moliAudioSourceStartTime=0;node.__moliAudioSourceStopTime=Infinity;context.__moliAudioContextDestination={};
  const result=await context.startRendering();assert(result.getChannelData(0).every(x=>x===0),'no fabricated oscillator fingerprint');
 });
 await check('buffer setter private state',()=>{
  const context=new AudioContext();try{const node=context.createBufferSource();node.buffer=buffer;node.__moliBufferSourceBufferSet=false;node.__moliBufferSourceBuffer=null;let error;try{node.buffer=buffer;}catch(e){error=e;}assert(error?.name==='InvalidStateError'&&node.buffer===buffer,'public spoof cannot reset native buffer state');}finally{context.close();}
 });
 globalThis.__nodeReplacementResults={rows,passed:rows.filter(r=>r.pass).length,total:rows.length,failures:rows.filter(r=>!r.pass)};return rows.every(r=>r.pass);
})()
