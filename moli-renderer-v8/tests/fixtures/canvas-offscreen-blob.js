(async () => {
  const checks = [];
  const add = (name, passed, detail) => checks.push({name,passed:!!passed,...(passed?{}:{detail:String(detail??'')})});
  const other = document.querySelector('iframe').contentWindow;
  async function pixels(name, blob, expected, width=2, height=1) {
    try {
      const bitmap = await createImageBitmap(await blob);
      const context = new OffscreenCanvas(width,height).getContext('2d');
      context.drawImage(bitmap,0,0);
      add(name, bitmap.width === width && bitmap.height === height && JSON.stringify(Array.from(context.getImageData(0,0,1,1).data)) === JSON.stringify(expected));
      bitmap.close();
    } catch(error) {add(name,false,error);}
  }
  for (const [ownerName,owner] of [['main',globalThis],['iframe',other]]) {
    for (const [calleeName,callee] of [['main',globalThis],['iframe',other]]) {
      const label = ownerName+' canvas / '+calleeName+' method';
      const result = await __offscreenBlobChecks(owner,callee,label);
      checks.push(...result.checks);
      await pixels(label+' snapshot pixels', result.snapshot, [0,255,0,255]);
      await pixels(label+' converted pixels', result.converted, [0,0,255,255]);
    }
  }
  const bitmapCanvas = new OffscreenCanvas(0,0);
  const renderer = bitmapCanvas.getContext('bitmaprenderer');
  renderer.transferFromImageBitmap(await createImageBitmap(new ImageData(new Uint8ClampedArray([255,0,0,255]),1,1)));
  await pixels('bitmap natural dimensions despite zero attributes', bitmapCanvas.convertToBlob(), [255,0,0,255],1,1);
  renderer.transferFromImageBitmap(null);
  let emptyError;
  try {await bitmapCanvas.convertToBlob();} catch(error) {emptyError=error;}
  add('null bitmap restores empty dimensions',emptyError instanceof DOMException && emptyError.name==='IndexSizeError');

  const workerURL = URL.createObjectURL(new Blob([
    'const checkExports = '+__offscreenBlobChecks.toString()+';\n'+
    'checkExports(globalThis,globalThis,"worker").then(result=>{postMessage(result);close();},error=>postMessage({error:String(error)}));'
  ], {type:'text/javascript'}));
  const worker = new Worker(workerURL);
  try {
    const result = await new Promise((resolve,reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(new Error(event.message));
    });
    add('worker exports complete', Array.isArray(result.checks), result.error);
    if (Array.isArray(result.checks)) checks.push(...result.checks);
    await pixels('worker snapshot pixels through Blob clone',result.snapshot,[0,255,0,255]);
    await pixels('worker converted pixels through Blob clone',result.converted,[0,0,255,255]);
  } finally {
    worker.terminate();
    URL.revokeObjectURL(workerURL);
  }
  globalThis.__uiEventResults = {complete:true,total:checks.length,passed:checks.filter(check=>check.passed).length,checks};
  return checks.every(check=>check.passed);
})()
