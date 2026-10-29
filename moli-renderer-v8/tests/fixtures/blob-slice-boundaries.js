(async () => {
  const checks = [];
  globalThis.__blobSliceProgress = {stage: 'setup', checks};
  async function runMatrix(owner, callee, prefix, checks) {
    const edges = [
      ['negative infinity', -Infinity, 'data', ''],
      ['negative finite limit', -Number.MAX_VALUE, 'data', ''],
      ['signed minimum', -(2 ** 63), 'data', ''],
      ['below signed minimum', -(2 ** 63) - 2048, 'data', ''],
      ['positive finite limit', Number.MAX_VALUE, '', 'data'],
      ['positive infinity', Infinity, '', 'data'],
      ['above signed maximum', 2 ** 63, '', 'data'],
      ['zero', 0, 'data', ''],
      ['negative zero', -0, 'data', ''],
      ['negative one', -1, 'a', 'dat'],
      ['negative half', -0.5, 'data', ''],
      ['negative tie', -1.5, 'ta', 'da'],
      ['positive tie', 1.5, 'ta', 'da'],
      ['positive half', 0.5, 'data', ''],
      ['NaN', NaN, 'data', '']
    ];
    for (const kind of ['Blob', 'File']) {
      const source = kind === 'Blob' ? new owner.Blob(['data']) : new owner.File(['data'], 'file.txt');
      const empty = kind === 'Blob' ? new owner.Blob() : new owner.File([], 'empty.txt');
      const slice = callee.Blob.prototype.slice;
      for (const [name, value, startText, endText] of edges) {
        for (const [position, args, expected] of [['start', [value], startText], ['end', [0, value], endText]]) {
          const label = `${prefix}/${kind}/${name}/${position}`;
          if (globalThis.__blobSliceProgress) globalThis.__blobSliceProgress.stage = label;
          const result = slice.call(source, ...args);
          const text = await result.text();
          checks.push({name: label, passed: text === expected && result.size === expected.length && result.type === '', detail: {text, size: result.size, expected}});
        }
      }
      for (const value of [-Infinity, 0, Infinity]) {
        for (const [position, args] of [['start', [value]], ['end', [0, value]]]) {
          const label = `${prefix}/${kind}/empty/${value}/${position}`;
          const result = slice.call(empty, ...args);
          const text = await result.text();
          checks.push({name: label, passed: result.size === 0 && text === '', detail: {text, size: result.size}});
        }
      }
    }
  }
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const realms = [['top', window], ['child', frame.contentWindow]];
  for (const [ownerName, owner] of realms) {
    for (const [calleeName, callee] of realms) {
      await runMatrix(owner, callee, `${ownerName}/${calleeName}`, checks);
    }
  }
  __blobSliceProgress.stage = 'worker boundaries';
  const workerBody = async runMatrix => {
    const checks = [];
    await runMatrix(globalThis, globalThis, 'worker', checks);
    postMessage(checks);
    close();
  };
  const url = URL.createObjectURL(new Blob([
    `(${workerBody.toString()})(${runMatrix.toString()})`
  ], {type: 'text/javascript'}));
  const worker = new Worker(url);
  try {
    const workerChecks = await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(Error(event.message));
    });
    checks.push(...workerChecks);
  } finally {
    worker.terminate();
    URL.revokeObjectURL(url);
    frame.remove();
  }
  globalThis.__uiEventResults = {checks};
})().catch(error => {
  const checks = globalThis.__blobSliceProgress.checks;
  checks.push({name: 'fixture completion', passed: false, detail: String(error.stack || error)});
  globalThis.__uiEventResults = {checks};
});
