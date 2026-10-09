(async () => {
  const checks = [];
  const diagnostics = [];
  const other = document.querySelector('iframe').contentWindow;
  const realms = [['main', globalThis], ['iframe', other]];
  const red = [255, 0, 0, 255], green = [0, 255, 0, 255];
  const blue = [0, 0, 255, 255], white = [255, 255, 255, 255];
  const same = (actual, expected) => JSON.stringify(actual) === JSON.stringify(expected);
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const run = (name, body) => {
    try { body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, detail: String(error)}); }
  };
  const errorFrom = body => { try { body(); } catch (error) { return error; } };
  function load(image, url) {
    return new Promise((resolve, reject) => {
      image.onload = () => resolve(image);
      image.onerror = () => reject(Error('image failed: ' + url));
      image.src = url;
    });
  }
  function sourceCanvas(owner) {
    const canvas = owner.document.createElement('canvas');
    canvas.width = canvas.height = 2;
    const context = canvas.getContext('2d');
    for (const [color, x, y] of [['red',0,0], ['lime',1,0], ['blue',0,1], ['white',1,1]]) {
      context.fillStyle = color; context.fillRect(x, y, 1, 1);
    }
    return canvas;
  }
  function destination(owner, offscreen) {
    const canvas = offscreen ? new owner.OffscreenCanvas(4,4) : owner.document.createElement('canvas');
    canvas.width = canvas.height = 4;
    const context = canvas.getContext('2d');
    context.imageSmoothingEnabled = false;
    return context;
  }
  const pixel = (context, x=0, y=0) => Array.from(context.getImageData(x,y,1,1).data);
  const blob = await new Promise(resolve => sourceCanvas(globalThis).toBlob(resolve));
  const url = URL.createObjectURL(blob);
  const images = await Promise.all(realms.map(([,owner]) => load(new owner.Image(), url)));
  URL.revokeObjectURL(url);
  const broken = new Image();
  await new Promise(resolve => { broken.onerror = resolve; broken.src = 'data:image/png;base64,YmFk'; });
  for (let ownerIndex=0; ownerIndex<realms.length; ownerIndex++) {
    const [ownerName, owner] = realms[ownerIndex], image = images[ownerIndex];
    for (const [calleeName, callee] of realms) for (const offscreen of [false,true]) {
      const label = ownerName+' image / '+calleeName+' '+(offscreen?'offscreen':'html');
      const context = destination(callee, offscreen);
      const draw = context.drawImage;
      const clear = () => context.clearRect(0,0,4,4);
      run(label+' loaded image survives Blob URL revocation', () => {
        draw.call(context,image,0,0);
        assert(same(pixel(context),red) && same(pixel(context,1,0),green) &&
          same(pixel(context,0,1),blue) && same(pixel(context,1,1),white), 'natural pixels');
      });
      run(label+' scaled pixels', () => {
        clear(); draw.call(context,image,0,0,4,4);
        assert(same(pixel(context,3,0),green) && same(pixel(context,3,3),white), 'scaled pixels');
      });
      run(label+' cropped pixels', () => {
        clear(); draw.call(context,image,1,0,1,1,0,0,2,2);
        assert(same(pixel(context,1,1),green), 'cropped pixels');
      });
      for (const count of [3,5,9]) run(label+' '+count+' argument conversion order', () => {
        const values = count===9 ? [0,0,2,2,0,0,2,2] : count===5 ? [0,0,2,2] : [0,0];
        const calls = [];
        draw.call(context,image,...values.map((value,index) => ({valueOf(){calls.push(index);return value;}})));
        assert(same(calls,values.map((_,index)=>index)), 'conversion order');
      });
      for (const count of [4,6,7,8]) run(label+' rejects unsupported arity '+count+' before conversion', () => {
        let conversions=0;
        const coordinate={valueOf(){conversions++;return 0;}};
        const error=errorFrom(()=>draw.call(context,image,...Array(count-1).fill(coordinate)));
        assert(error instanceof callee.TypeError && conversions===0,'overload resolution before conversion');
      });
      run(label+' ignores argument beyond longest overload', () => draw.call(context,image,0,0,2,2,0,0,2,2,Symbol('ignored')));
      for (const count of [0,1,2]) run(label+' requires minimum arguments '+count, () => {
        const args = [image,0].slice(0,count);
        assert(errorFrom(()=>draw.call(context,...args)) instanceof callee.TypeError, 'callee TypeError');
      });
      run(label+' numeric getter error identity', () => {
        const sentinel = {};
        assert(errorFrom(()=>draw.call(context,image,{valueOf(){throw sentinel;}},0))===sentinel,'exception identity');
      });
      run(label+' nonfinite no-op', () => {
        clear(); draw.call(context,image,Infinity,0);
        assert(same(pixel(context),[0,0,0,0]), 'nonfinite draws nothing');
      });
      run(label+' converts all arguments before nonfinite no-op', () => {
        assert(errorFrom(()=>draw.call(context,image,Infinity,Symbol('dy'))) instanceof callee.TypeError,'convert dy');
      });
      run(label+' unloaded image no-op', () => {
        clear(); draw.call(context,new owner.Image(),0,0);
        assert(same(pixel(context),[0,0,0,0]), 'unloaded draws nothing');
      });
      run(label+' broken image throws after conversion', () => {
        let conversions=0;
        const error = errorFrom(()=>draw.call(context,broken,{valueOf(){conversions++;return 0;}},0));
        assert(conversions===1 && error instanceof callee.DOMException && error.name==='InvalidStateError','broken state');
      });
      run(label+' nonfinite precedes broken image usability', () => draw.call(context,broken,NaN,0));
      const revocable = Proxy.revocable(image,{}); revocable.revoke();
      let traps=0;
      const invalidSources = [
        ['plain',{}], ['forged',Object.create(owner.HTMLImageElement.prototype)],
        ['inherited real',Object.create(image)], ['author proxy',new Proxy(image,{get(){traps++;throw Error('trap');}})],
        ['revoked proxy',revocable.proxy], ['null',null], ['number',1]
      ];
      for (const [kind,source] of invalidSources) run(label+' rejects '+kind+' source before conversion', () => {
        let conversions=0;
        const error=errorFrom(()=>draw.call(context,source,{valueOf(){conversions++;return 0;}},0));
        assert(error instanceof callee.TypeError && conversions===0 && traps===0, 'native source brand');
      });
      const receiverRevocable=Proxy.revocable(context,{}); receiverRevocable.revoke();
      for (const [kind,receiver] of [['plain',{}],['inherited real',Object.create(context)],['author proxy',new Proxy(context,{})],['revoked proxy',receiverRevocable.proxy]]) {
        run(label+' rejects '+kind+' receiver before conversion', () => {
          let conversions=0;
          const error=errorFrom(()=>draw.call(receiver,image,{valueOf(){conversions++;return 0;}},0));
          assert(error instanceof callee.TypeError && conversions===0, 'native receiver brand');
        });
      }
      run(label+' rejects the other 2d interface before conversion', () => {
        let conversions=0;
        const error=errorFrom(()=>draw.call(destination(callee,!offscreen),image,{valueOf(){conversions++;return 0;}},0));
        assert(error instanceof callee.TypeError && conversions===0, 'specific interface receiver brand');
      });
      run(label+' accepts genuine receiver from source realm', () => {
        const receiver=destination(owner,offscreen);
        draw.call(receiver,image,0,0);
        assert(same(pixel(receiver),red),'cross-realm receiver');
      });
      run(label+' snapshots canvas after numeric getter', () => {
        const canvas=sourceCanvas(owner);
        clear(); draw.call(context,canvas,{valueOf(){canvas.width=1;const c=canvas.getContext('2d');c.fillStyle='lime';c.fillRect(0,0,1,2);return 0;}},0);
        assert(same(pixel(context),green) && same(pixel(context,1,0),[0,0,0,0]), 'new size and pixels');
      });
      run(label+' empty source canvas throws', () => {
        const canvas=owner.document.createElement('canvas');canvas.width=0;
        const error=errorFrom(()=>draw.call(context,canvas,0,0));
        assert(error instanceof callee.DOMException && error.name==='InvalidStateError','empty canvas');
      });
      const bitmap=await createImageBitmap(blob);
      run(label+' closes bitmap during numeric conversion', () => {
        const error=errorFrom(()=>draw.call(context,bitmap,{valueOf(){bitmap.close();return 0;}},0));
        assert(error instanceof callee.DOMException && error.name==='InvalidStateError','closed after conversion');
      });
      run(label+' nonfinite precedes detached bitmap usability', () => draw.call(context,bitmap,NaN,0));
    }
    run(ownerName+' ignores image own property getters', () => {
      for(const name of ['src','currentSrc','naturalWidth','naturalHeight','width','height']) {
        Object.defineProperty(image,name,{configurable:true,get(){throw Error('read '+name);}});
      }
      const context=destination(globalThis,false);
      context.drawImage(image,0,0);
      assert(same(pixel(context),red), 'native accepted content');
    });
    run(ownerName+' genuine image with changed prototype', () => {
      Object.setPrototypeOf(image,null);
      const context=destination(globalThis,false);
      context.drawImage(image,0,0);
      assert(same(pixel(context),red), 'native identity survives prototype change');
    });
  }
  const replacementCanvas=sourceCanvas(globalThis);
  const replacementContext=replacementCanvas.getContext('2d');
  replacementContext.fillStyle='blue';replacementContext.fillRect(0,0,2,2);
  const replacementBlob=await new Promise(resolve=>replacementCanvas.toBlob(resolve));
  const firstURL=URL.createObjectURL(blob), nextURL=URL.createObjectURL(replacementBlob);
  const replacing=await load(new Image(),firstURL), read=destination(globalThis,false);
  const next=load(replacing,nextURL);
  await Promise.resolve();
  // Current/pending request presentation is a separate image lifecycle gap.
  // Retain the actual observation without treating it as loaded-image coverage.
  read.drawImage(replacing,0,0);
  diagnostics.push({name:'current image retained during pending request',expected:red,
    pixels:pixel(read),currentSrc:replacing.currentSrc,naturalWidth:replacing.naturalWidth});
  await next;
  URL.revokeObjectURL(firstURL);URL.revokeObjectURL(nextURL);
  run('accepted replacement becomes current image',()=>{
    read.clearRect(0,0,4,4);read.drawImage(replacing,0,0);assert(same(pixel(read),blue),'replacement image');
  });
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(check=>check.passed).length,checks,diagnostics};
  return checks.every(check=>check.passed);
})()
