(async () => {
  const checks = [], diagnostics = [];
  const add = result => { checks.push(...result.checks); diagnostics.push(...result.diagnostics); };
  const other = document.querySelector('iframe').contentWindow;
  for (const [ownerName, owner] of [['main', globalThis], ['iframe', other]]) {
    for (const [calleeName, callee] of [['main', globalThis], ['iframe', other]]) {
      add(__offscreenTransferChecks(owner, callee, ownerName + ' canvas / ' + calleeName + ' method'));
    }
  }
  for (const [name, realm] of [['main', globalThis], ['iframe', other]]) {
    try {
      const canvas = realm.document.createElement('canvas'); canvas.width = 3; canvas.height = 2;
      const offscreen = canvas.transferControlToOffscreen();
      const context = offscreen.getContext('2d'); context.fillStyle = '#00ff00'; context.fillRect(0, 0, 3, 2);
      const bitmap = offscreen.transferToImageBitmap();
      const read = new OffscreenCanvas(3, 2).getContext('2d'); read.drawImage(bitmap, 0, 0);
      checks.push({name: name + ': placeholder-owned bitmap transfers real pixels', passed: bitmap.width === 3 && bitmap.height === 2 && Array.from(read.getImageData(0, 0, 1, 1).data).join(',') === '0,255,0,255'});
      bitmap.close();
    } catch (error) { checks.push({name: name + ': placeholder-owned bitmap transfers real pixels', passed: false, error: String(error)}); }
  }
  const workerURL = URL.createObjectURL(new Blob([
    'const probe = ' + __offscreenTransferChecks.toString() + ';\n' +
    'postMessage(probe(globalThis, globalThis, "worker")); close();'
  ], {type: 'text/javascript'}));
  const worker = new Worker(workerURL);
  try {
    const result = await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(new Error(event.message));
    });
    add(result);
  } finally { worker.terminate(); URL.revokeObjectURL(workerURL); }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks, diagnostics};
  return checks.every(row => row.passed);
})()
