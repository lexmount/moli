(async () => {
  const checks = [];
  const progress = globalThis.__svgImageProgress = {stage:'setup',checks};
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const assert = (ok, detail) => { if (!ok) throw Error(detail); };
  const red = [255, 0, 0, 255], green = [0, 255, 0, 255];
  const blue = [0, 0, 255, 255], clear = [0, 0, 0, 0];
  const pixel = (context, x=0, y=0) => Array.from(context.getImageData(x,y,1,1).data);
  const expectPixel = (context, x, y, expected) => {
    const actual = pixel(context,x,y);
    assert(same(actual,expected), `pixel ${x},${y}: ${actual}, expected ${expected}`);
  };
  const run = async (name, body) => {
    progress.stage=name;
    try { await body(); checks.push({name, passed:true}); }
    catch (error) { checks.push({name, passed:false, detail:String(error)}); }
  };
  const rejected = async (promise, callee, name) => {
    let caught;
    try { await promise; } catch (error) { caught=error; }
    assert(caught instanceof callee.DOMException && caught.name===name, 'rejection '+caught);
  };
  const security = (body, callee) => {
    let caught;
    try { body(); } catch (error) { caught=error; }
    assert(caught instanceof callee.DOMException && caught.name==='SecurityError', 'security '+caught);
  };
  const svg = (attributes, body) => `<svg xmlns="http://www.w3.org/2000/svg" ${attributes}>${body}</svg>`;
  const palette = '<rect width="100%" height="100%" fill="lime"/>'+
    '<rect width="1" height="1" fill="red"/><rect y="1" width="1" height="1" fill="blue"/>';
  function load(owner, url) {
    return new Promise((resolve, reject) => {
      const image = new owner.Image();
      image.onload = () => resolve(image);
      image.onerror = () => reject(Error('SVG load failed: '+url));
      image.src = url;
    });
  }
  async function image(owner, text) {
    progress.stage='SVG load '+text.slice(0,120);
    const url = owner.URL.createObjectURL(new owner.Blob([text], {type:'image/svg+xml'}));
    try { return await load(owner,url); }
    finally { owner.URL.revokeObjectURL(url); }
  }
  function context(callee, offscreen, width=12, height=8) {
    const canvas = offscreen ? new callee.OffscreenCanvas(width,height) : callee.document.createElement('canvas');
    canvas.width=width; canvas.height=height;
    return canvas.getContext('2d');
  }
  try {
    const realms = [['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
    assert(typeof __svgCrossOriginURL==='string', 'cross-origin fixture URL required');
    for (const [ownerName,owner] of realms) {
      const source = await image(owner,svg('width="4" height="2"',palette));
      const stripe = await image(owner,svg('width="4" height="2"',
        '<rect width="4" height="2" fill="lime"/><rect x="0.5" width="0.5" height="2" fill="red"/>'));
      const naturalLess = await image(owner,svg('',palette));
      const ratioOnly = await image(owner,svg('viewBox="0 0 4 2"',palette));
      const fractional = await image(owner,svg('width="4.5" height="2.5"',palette));
      const overflow = await image(owner,svg('width="4" height="2" overflow="visible"',
        '<rect x="-2" y="-2" width="8" height="6" fill="lime"/>'));
      const zero = await image(owner,svg('width="0" height="2"',palette));
      progress.stage='cross-origin load '+ownerName;
      const cross = await load(owner,__svgCrossOriginURL+'/cross-'+ownerName+'.svg');
      for (const [calleeName,callee] of realms) {
        const prefix = ownerName+' source / '+calleeName;
        for (const offscreen of [false,true]) {
          const label = prefix+' '+(offscreen?'offscreen':'html');
          await run(label+' natural SVG survives Blob URL revocation', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(source,0,0);
            expectPixel(ctx,0,0,red);expectPixel(ctx,0,1,blue);expectPixel(ctx,3,1,green);
            expectPixel(ctx,4,1,clear);
          });
          await run(label+' scaled SVG retains subpixel vector detail', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(stripe,0,0,12,6);
            expectPixel(ctx,2,2,red);expectPixel(ctx,4,2,green);
          });
          await run(label+' cropped SVG retains subpixel vector detail', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(stripe,0.5,0,0.5,2,0,0,12,8);
            expectPixel(ctx,6,4,red);
          });
          await run(label+' negative extents do not mirror SVG', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(source,4,2,-4,-2,12,6,-12,-6);
            expectPixel(ctx,1,1,red);expectPixel(ctx,1,4,blue);expectPixel(ctx,10,4,green);
          });
          await run(label+' source clipping maps destination proportionally', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(source,-1,0,2,1,0,0,8,4);
            expectPixel(ctx,1,1,clear);expectPixel(ctx,6,1,red);
          });
          await run(label+' outside source is a no-op with copy', () => {
            const ctx=context(callee,offscreen);ctx.fillStyle='blue';ctx.fillRect(0,0,12,8);
            ctx.globalCompositeOperation='copy';ctx.drawImage(source,9,0,1,1,0,0,12,8);
            expectPixel(ctx,0,0,blue);
          });
          await run(label+' CTM preserves vector detail', () => {
            const ctx=context(callee,offscreen);ctx.scale(3,3);ctx.drawImage(stripe,0,0);
            expectPixel(ctx,2,2,red);expectPixel(ctx,4,2,green);
          });
          await run(label+' numeric conversions precede SVG snapshot', () => {
            const ctx=context(callee,offscreen), order=[];
            ctx.drawImage(source,...[0,0,12,6].map((value,index)=>({valueOf(){order.push(index);return value;}})));
            assert(same(order,[0,1,2,3]), 'conversion order');expectPixel(ctx,1,1,red);
          });
          await run(label+' missing natural size uses destination bitmap', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(naturalLess,0,0);
            expectPixel(ctx,0,0,red);expectPixel(ctx,1,0,green);expectPixel(ctx,11,7,green);
          });
          await run(label+' missing size resolves in a one-pixel viewport', () => {
            const ctx=context(callee,offscreen,1,1);ctx.drawImage(naturalLess,0,0);
            expectPixel(ctx,0,0,red);
          });
          await run(label+' viewBox-only source uses contained default size', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(ratioOnly,0,0);
            expectPixel(ctx,1,1,red);expectPixel(ctx,10,5,green);expectPixel(ctx,10,6,clear);
          });
          await run(label+' root viewport clips SVG overflow', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(overflow,2,2);
            expectPixel(ctx,1,2,clear);expectPixel(ctx,2,2,green);expectPixel(ctx,6,2,clear);
          });
          await run(label+' fractional natural dimensions stay fractional', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(fractional,0,0);
            expectPixel(ctx,4,1,[0,255,0,128]);
            const corner=pixel(ctx,4,2);
            assert(same(corner.slice(0,3),green.slice(0,3)) && corner[3]>0 && corner[3]<255,
              'fractional corner coverage '+corner);
          });
          await run(label+' zero-sized SVG is a drawing no-op', () => {
            const ctx=context(callee,offscreen);ctx.fillStyle='blue';ctx.fillRect(0,0,12,8);
            ctx.drawImage(zero,0,0);expectPixel(ctx,0,0,blue);
          });
          await run(label+' SVG pattern snapshots loaded source', () => {
            const ctx=context(callee,offscreen);ctx.fillStyle=ctx.createPattern(source,'repeat');ctx.fillRect(0,0,12,8);
            expectPixel(ctx,4,0,red);expectPixel(ctx,4,1,blue);expectPixel(ctx,7,1,green);
          });
          await run(label+' SVG draw taints destination', () => {
            const ctx=context(callee,offscreen);ctx.drawImage(cross,0,0);
            security(()=>ctx.getImageData(0,0,1,1),callee);
          });
          await run(label+' SVG pattern taints destination', () => {
            const ctx=context(callee,offscreen);ctx.fillStyle=ctx.createPattern(cross,'repeat');ctx.fillRect(0,0,12,8);
            security(()=>ctx.getImageData(0,0,1,1),callee);
          });
        }
        await run(prefix+' SVG ImageBitmap renders at requested resolution', async () => {
          const bitmap=await callee.createImageBitmap(stripe,{resizeWidth:24,resizeHeight:12});
          assert(bitmap instanceof callee.ImageBitmap && bitmap.width===24 && bitmap.height===12,'bitmap realm or dimensions');
          const ctx=context(callee,true,24,12);ctx.drawImage(bitmap,0,0);
          expectPixel(ctx,4,4,red);expectPixel(ctx,8,4,green);bitmap.close();
        });
        await run(prefix+' cropped SVG ImageBitmap pads outside source', async () => {
          const bitmap=await callee.createImageBitmap(source,-1,0,2,1,{resizeWidth:8,resizeHeight:4});
          const ctx=context(callee,true,8,4);ctx.drawImage(bitmap,0,0);
          expectPixel(ctx,0,1,clear);expectPixel(ctx,6,1,red);bitmap.close();
        });
        await run(prefix+' negative crop and flipY use common bitmap semantics', async () => {
          const bitmap=await callee.createImageBitmap(source,1,2,-1,-2,{imageOrientation:'flipY',premultiplyAlpha:'none'});
          const ctx=context(callee,true,1,2);ctx.drawImage(bitmap,0,0);
          expectPixel(ctx,0,0,blue);expectPixel(ctx,0,1,red);bitmap.close();
        });
        await run(prefix+' fractional SVG bitmap rounds media dimensions before cropping', async () => {
          const bitmap=await callee.createImageBitmap(fractional,1,0,2,2,{resizeWidth:24,resizeHeight:16});
          const ctx=context(callee,true,24,16);ctx.drawImage(bitmap,0,0);
          expectPixel(ctx,0,0,red);expectPixel(ctx,5,0,green);bitmap.close();
        });
        for (const options of [{},{resizeWidth:12},{resizeHeight:8}]) {
          await run(prefix+' no natural dimensions rejects '+JSON.stringify(options), () =>
            rejected(callee.createImageBitmap(naturalLess,options),callee,'InvalidStateError'));
        }
        await run(prefix+' both resize dimensions provide SVG viewport', async () => {
          const bitmap=await callee.createImageBitmap(naturalLess,{resizeWidth:12,resizeHeight:8});
          const ctx=context(callee,true);ctx.drawImage(bitmap,0,0);
          expectPixel(ctx,0,0,red);expectPixel(ctx,1,0,green);expectPixel(ctx,11,7,green);bitmap.close();
        });
        await run(prefix+' explicit SVG bitmap viewport preserves viewBox ratio', async () => {
          const bitmap=await callee.createImageBitmap(ratioOnly,{resizeWidth:12,resizeHeight:8});
          const ctx=context(callee,true);ctx.drawImage(bitmap,0,0);
          expectPixel(ctx,0,0,clear);expectPixel(ctx,1,2,red);expectPixel(ctx,10,6,green);expectPixel(ctx,10,7,clear);bitmap.close();
        });
        await run(prefix+' zero-sized SVG bitmap rejects', () =>
          rejected(callee.createImageBitmap(zero),callee,'InvalidStateError'));
        await run(prefix+' SVG bitmap carries source taint', async () => {
          const bitmap=await callee.createImageBitmap(cross);
          const ctx=context(callee,true);ctx.drawImage(bitmap,0,0);
          security(()=>ctx.getImageData(0,0,1,1),callee);bitmap.close();
        });
        await run(prefix+' SVG-derived bitmap transfers through worker', async () => {
          const bitmap=await callee.createImageBitmap(source);
          const code='onmessage=async e=>{try{const bitmap=await createImageBitmap(e.data);e.data.close();postMessage(bitmap,[bitmap]);}catch(error){postMessage({error:String(error)});}close();};';
          const url=callee.URL.createObjectURL(new callee.Blob([code],{type:'text/javascript'}));
          const worker=new callee.Worker(url);
          try {
            const output=await new Promise((resolve,reject)=>{
              worker.onmessage=e=>resolve(e.data);worker.onerror=e=>reject(Error(e.message));
              worker.postMessage(bitmap,[bitmap]);
            });
            assert(bitmap.width===0 && output instanceof callee.ImageBitmap,'transfer ownership or receiving realm');
            const ctx=context(callee,true);ctx.drawImage(output,0,0);
            expectPixel(ctx,0,0,red);expectPixel(ctx,0,1,blue);output.close();
          } finally { worker.terminate();callee.URL.revokeObjectURL(url); }
        });
      }
      await run(ownerName+' SVG source ignores public metadata and prototype replacement', async () => {
        const poisoned=await image(owner,svg('width="4" height="2"',palette));
        for (const key of ['width','height','naturalWidth','naturalHeight','src','complete']) {
          Object.defineProperty(poisoned,key,{get(){throw Error('public '+key);},configurable:true});
        }
        Object.setPrototypeOf(poisoned,null);
        const ctx=context(globalThis,true);ctx.drawImage(poisoned,0,0);expectPixel(ctx,0,0,red);
        const bitmap=await createImageBitmap(poisoned);ctx.clearRect(0,0,12,8);ctx.drawImage(bitmap,0,0);
        expectPixel(ctx,0,1,blue);bitmap.close();
      });
    }
  } catch (error) { checks.push({name:'SVG fixture setup',passed:false,detail:String(error)}); }
  globalThis.__uiEventResults={checks,total:checks.length};
})()
