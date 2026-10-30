(owner, callee, kind, label) => {
  const checks=[];
  const assert=(ok,name)=>{if(!ok)throw Error(name)};
  const check=(name,run)=>{try{run();checks.push({name:label+': '+name,passed:true})}catch(error){checks.push({name:label+': '+name,passed:false,error:String(error),stack:error.stack})}};
  const C=kind==='element'?callee.CanvasRenderingContext2D:callee.OffscreenCanvasRenderingContext2D;
  const canvas=kind==='element'?owner.document.createElement('canvas'):new owner.OffscreenCanvas(4,2);
  canvas.width=4;canvas.height=2;
  const ctx=canvas.getContext('2d'), draw=C.prototype.drawImage;
  const source=new owner.OffscreenCanvas(2,1),sourceContext=source.getContext('2d');
  sourceContext.fillStyle='#00ff00';sourceContext.fillRect(0,0,2,1);
  const pixel=(x=0)=>Array.from(ctx.getImageData(x,0,1,1).data).join();
  const throws=(Ctor,run,name)=>{let error;try{run()}catch(caught){error=caught}assert(error instanceof Ctor && (!name || error.name===name),'unexpected error: '+error);return error;};
  check('native operation descriptor',()=>{
    const d=Object.getOwnPropertyDescriptor(C.prototype,'drawImage');
    assert(d.enumerable && d.configurable && d.writable && draw.length===3 && !Object.hasOwn(draw,'prototype') && String(draw).includes('[native code]'),'descriptor');
  });
  check('natural scaled and cropped overloads convert once in argument order',()=>{
    for(const values of [[0,0],[0,0,2,1],[0,0,2,1,0,0,2,1]]) {
      const log=[];ctx.clearRect(0,0,4,2);
      const coordinates=values.map((value,i)=>({valueOf(){log.push(i);return value}}));
      draw.call(ctx,source,...coordinates);
      assert(log.join()===values.map((_,i)=>i).join() && pixel()==='0,255,0,255','argument order and green pixel');
    }
  });
  check('invalid overload counts throw in the callee realm',()=>{
    for(const count of [0,1,2,4,6,7,8]) {const args=Array(count).fill(0);if(count)args[0]=source;throws(callee.TypeError,()=>draw.call(ctx,...args));}
  });
  check('extra arguments are ignored after the cropped overload',()=>{
    const poison={valueOf(){throw Error('extra value conversion')},toString(){throw Error('extra string conversion')}};
    draw.call(ctx,source,0,0,2,1,0,0,2,1,poison,Symbol(),1n);
    assert(pixel()==='0,255,0,255','cropped extra arguments');
  });
  check('illegal receivers fail before source or coordinate author code',()=>{
    let reads=0;const trap=()=>{reads++;throw Error('author trap')};
    const proxy=new Proxy(ctx,{get:trap,getPrototypeOf:trap}),revoked=Proxy.revocable(ctx,{});revoked.revoke();
    for(const receiver of [{},Object.create(C.prototype),Object.create(ctx),proxy,revoked.proxy]) {
      throws(callee.TypeError,()=>draw.call(receiver,new Proxy(source,{get:trap,getPrototypeOf:trap}),{valueOf:trap},0));
    }
    assert(reads===0,'receiver validation order');
  });
  check('source union rejects forged author and revoked proxy values without traps',()=>{
    let reads=0;const trap=()=>{reads++;throw Error('source trap')};
    const proxy=new Proxy(source,{get:trap,getPrototypeOf:trap}),revoked=Proxy.revocable(source,{});revoked.revoke();
    const invalid=[null,undefined,1,{},Object.create(source),Object.create(owner.OffscreenCanvas.prototype),proxy,revoked.proxy];
    if(owner.document)invalid.push(owner.document.createElement('div'));
    for(const value of invalid)throws(callee.TypeError,()=>draw.call(ctx,value,{valueOf:trap},0));
    assert(reads===0,'source validation order');
  });
  check('coordinate errors preserve exception identity',()=>{
    const marker={};let error;
    try{draw.call(ctx,source,{valueOf(){throw marker}},0)}catch(caught){error=caught}
    assert(error===marker,'coordinate exception');throws(callee.TypeError,()=>draw.call(ctx,source,Symbol(),0));
  });
  check('snapshot and natural dimensions are read after coordinate conversion',()=>{
    const image=new owner.OffscreenCanvas(2,1),context=image.getContext('2d');
    context.fillStyle='#00ff00';context.fillRect(0,0,2,1);ctx.clearRect(0,0,4,2);
    draw.call(ctx,image,{valueOf(){image.width=1;context.fillStyle='#0000ff';context.fillRect(0,0,1,1);return 0}},0);
    assert(pixel()==='0,0,255,255' && pixel(1)==='0,0,0,0','updated source snapshot');
  });
  check('live ImageBitmap pixels are accepted',()=>{
    const bitmap=source.transferToImageBitmap();ctx.clearRect(0,0,4,2);
    try{draw.call(ctx,bitmap,0,0);assert(pixel()==='0,255,0,255','bitmap pixels')}finally{bitmap.close()}
    sourceContext.fillStyle='#00ff00';sourceContext.fillRect(0,0,2,1);
  });
  check('all coordinate conversion precedes detached bitmap errors',()=>{
    const bitmap=source.transferToImageBitmap();bitmap.close();const log=[];
    throws(callee.DOMException,()=>draw.call(ctx,bitmap,{valueOf(){log.push('x');return 0}},{valueOf(){log.push('y');return 0}}),'InvalidStateError');
    assert(log.join()==='x,y','conversion before usability');
    sourceContext.fillStyle='#00ff00';sourceContext.fillRect(0,0,2,1);
  });
  check('coordinate getters can close a source before snapshot validation',()=>{
    const bitmap=source.transferToImageBitmap();const log=[];
    throws(callee.DOMException,()=>draw.call(ctx,bitmap,{valueOf(){log.push('x');bitmap.close();return 0}},{valueOf(){log.push('y');return 0}}),'InvalidStateError');
    assert(log.join()==='x,y','reentrant close');sourceContext.fillStyle='#00ff00';sourceContext.fillRect(0,0,2,1);
  });
  check('nonfinite coordinates are no-ops after normal conversion',()=>{
    ctx.clearRect(0,0,4,2);for(const value of [NaN,Infinity,-Infinity])draw.call(ctx,source,value,0);
    assert(pixel()==='0,0,0,0','nonfinite draw');
  });
  check('source public dimension getters are not consulted',()=>{
    for(const name of ['width','height'])Object.defineProperty(source,name,{get(){throw Error('public source '+name)}});
    draw.call(ctx,source,0,0);assert(pixel()==='0,255,0,255','native source dimensions');
  });
  return checks;
}
