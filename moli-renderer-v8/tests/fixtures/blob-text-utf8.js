(async () => {
  const checks = [];
  globalThis.__blobTextProgress = {stage: 'setup', checks};
  async function runMatrix(owner, callee, prefix, checks) {
    const cases = [
      ['empty', [], ''], ['BOM only', [239,187,191], ''],
      ['leading BOM', [239,187,191,65], 'A'],
      ['repeated BOM', [239,187,191,239,187,191,65], '\uFEFFA'],
      ['embedded BOM', [65,239,187,191], 'A\uFEFF'],
      ['incomplete BOM', [239,187], '\uFFFD'],
      ['UTF-16LE BOM is not sniffed', [255,254,65,0], '\uFFFD\uFFFDA\u0000'],
      ['invalid UTF-8', [192,128,245,65], '\uFFFD\uFFFD\uFFFDA']
    ];
    for (const kind of ['Blob','File']) for (const type of ['', 'text/plain;charset=utf-16le', 'application/octet-stream']) {
      for (const [name, bytes, expected] of cases) {
        const source = kind === 'Blob' ? new owner.Blob([new owner.Uint8Array(bytes)], {type})
          : new owner.File([new owner.Uint8Array(bytes)], 'text', {type});
        const actual = await callee.Blob.prototype.text.call(source);
        checks.push({name: `${prefix}/${kind}/${type}/${name}`, passed: actual === expected, detail: {actual, expected}});
      }
    }
  }
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  const realms = [['top', window], ['child', frame.contentWindow]];
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms) {
    await runMatrix(owner, callee, `${ownerName}/${calleeName}`, checks);
  }
  const body = async runMatrix => {const checks=[];await runMatrix(globalThis,globalThis,'worker',checks);postMessage(checks);close();};
  const url=URL.createObjectURL(new Blob([`(${body})(${runMatrix})`],{type:'text/javascript'}));
  const worker=new Worker(url);
  try {checks.push(...await new Promise((resolve,reject)=>{worker.onmessage=e=>resolve(e.data);worker.onerror=e=>reject(Error(e.message));}));}
  finally {worker.terminate(); URL.revokeObjectURL(url); frame.remove();}
  globalThis.__uiEventResults={checks};
})().catch(error=>{const checks=__blobTextProgress.checks;checks.push({name:'fixture completion',passed:false,detail:String(error.stack||error)});globalThis.__uiEventResults={checks};});
