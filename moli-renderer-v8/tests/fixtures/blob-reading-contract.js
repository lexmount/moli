(async () => {
  const checks = [];
  globalThis.__blobReadingProgress = {stage: 'setup', checks};

  async function runSuite(owner, callee, prefix, checks) {
    const check = (name, passed, detail) => checks.push({name: `${prefix}/${name}`, passed, detail});
    const bytes = [0xef, 0xbb, 0xbf, 0x41, 0xc0, 0x80];
    const text = 'A\ufffd\ufffd';
    const source = kind => kind === 'Blob'
      ? new owner.Blob([new owner.Uint8Array(bytes)])
      : new owner.File([new owner.Uint8Array(bytes)], 'sample.txt');
    for (const kind of ['Blob', 'File']) for (const method of ['text', 'arrayBuffer', 'bytes', 'stream']) {
      const blob = source(kind);
      const first = callee.Blob.prototype[method].call(blob);
      const second = callee.Blob.prototype[method].call(blob);
      const firstReader = method === 'stream' ? first.getReader() : null;
      const secondReader = method === 'stream' ? second.getReader() : null;
      const firstValue = firstReader ? (await firstReader.read()).value : await first;
      const secondValue = secondReader ? (await secondReader.read()).value : await second;
      const expectedPrototype = method === 'stream' ? owner.ReadableStream.prototype : owner.Promise.prototype;
      check(`${kind}/${method}/return belongs to Blob realm`,
        Object.getPrototypeOf(first) === expectedPrototype && Object.getPrototypeOf(second) === expectedPrototype);
      if (method === 'text') {
        check(`${kind}/${method}/UTF-8 decoding`, firstValue === text && secondValue === text, [firstValue, secondValue]);
        check(`${kind}/${method}/independent promises`, first !== second);
      } else {
        const firstBytes = method === 'arrayBuffer' ? new owner.Uint8Array(firstValue) : firstValue;
        const secondBytes = method === 'arrayBuffer' ? new owner.Uint8Array(secondValue) : secondValue;
        const expected = method === 'arrayBuffer' ? owner.ArrayBuffer.prototype : owner.Uint8Array.prototype;
        check(`${kind}/${method}/result bytes and realm`, Object.getPrototypeOf(firstValue) === expected
          && Object.getPrototypeOf(secondValue) === expected && firstBytes.buffer instanceof owner.ArrayBuffer
          && Array.from(firstBytes).join() === bytes.join(), Array.from(firstBytes));
        firstBytes[0] = 0;
        check(`${kind}/${method}/independent byte storage`, first !== second && firstValue !== secondValue
          && firstBytes.buffer !== secondBytes.buffer && Array.from(secondBytes).join() === bytes.join(), Array.from(secondBytes));
      }
      if (firstReader) {
        const firstEnd = await firstReader.read();
        const secondEnd = await secondReader.read();
        if (!firstEnd.done || !secondEnd.done) throw Error('Blob stream did not close after its bytes');
        firstReader.releaseLock();
        secondReader.releaseLock();
      }
    }

    for (const kind of ['Blob', 'File']) {
      const blob = kind === 'Blob' ? new owner.Blob([new owner.Uint8Array([1, 2, 3, 4, 5])])
        : new owner.File([new owner.Uint8Array([1, 2, 3, 4, 5])], 'byob');
      const stream = callee.Blob.prototype.stream.call(blob);
      const reader = stream.getReader({mode: 'byob'});
      const input = new owner.Uint8Array(2);
      const pending = reader.read(input);
      let settled = false;
      pending.then(() => {settled = true;}, () => {settled = true;});
      const atMicrotask = await new owner.Promise(resolve => owner.queueMicrotask(() => resolve(settled)));
      check(`${kind}/BYOB/transfers the input buffer`, input.byteLength === 0);
      check(`${kind}/BYOB/read waits for a file-reading task`, atMicrotask === false, atMicrotask);
      const first = await pending;
      const second = await reader.read(new owner.Uint8Array(8));
      const last = await reader.read(new owner.Uint8Array(1));
      check(`${kind}/BYOB/fills buffers and preserves the relevant realm`, !first.done && !second.done
        && Object.getPrototypeOf(first.value) === owner.Uint8Array.prototype
        && Object.getPrototypeOf(second.value) === owner.Uint8Array.prototype
        && Array.from(first.value).join() === '1,2' && Array.from(second.value).join() === '3,4,5');
      await reader.closed;
      check(`${kind}/BYOB/closes with an empty view`, last.done && last.value.byteLength === 0);
      reader.releaseLock();

      const canceled = callee.Blob.prototype.stream.call(source(kind)).getReader();
      const beforeCancel = canceled.read();
      await canceled.cancel({reason: 'before file reading'});
      const result = await beforeCancel;
      await canceled.closed;
      check(`${kind}/cancel/settles a pending read without delivering data`, result.done && result.value === undefined);
      const after = await new owner.Blob(['after cancellation']).text();
      check(`${kind}/cancel/queued byte delivery does not disrupt later reads`, after === 'after cancellation');
      canceled.releaseLock();
    }

    const native = {Promise: owner.Promise, ArrayBuffer: owner.ArrayBuffer, Uint8Array: owner.Uint8Array,
      ReadableStream: owner.ReadableStream};
    const methods = Object.fromEntries(['text', 'arrayBuffer', 'bytes', 'stream'].map(name => [name, callee.Blob.prototype[name]]));
    const sources = [['Blob', source('Blob')], ['File', source('File')]];
    const globals = ['Blob', 'File', 'Promise', 'ArrayBuffer', 'Uint8Array', 'ReadableStream', 'queueMicrotask', 'setTimeout'];
    const realms = Array.from(new Set([owner, callee]));
    const saved = realms.map(realm => [realm, globals.map(name => [name, realm[name]])]);
    let called = 0;
    const poison = () => {called++; throw Error('author replacement was called');};
    try {
      for (const realm of realms) for (const name of globals) realm[name] = poison;
      for (const [kind, blob] of sources) for (const method of ['text', 'arrayBuffer', 'bytes', 'stream']) {
        const value = methods[method].call(blob);
        const reader = method === 'stream' ? value.getReader() : null;
        const result = reader ? (await reader.read()).value : await value;
        const resultBytes = method === 'arrayBuffer' ? new native.Uint8Array(result) : result;
        const dataCorrect = method === 'text' ? result === text : Array.from(resultBytes).join() === bytes.join();
        check(`${kind}/${method}/native construction ignores replaced globals`, called === 0 && dataCorrect
          && (reader ? value instanceof native.ReadableStream : value instanceof native.Promise));
        if (reader) {
          if (!(await reader.read()).done) throw Error('poisoned-global Blob stream did not close');
          reader.releaseLock();
        }
      }
    } finally {
      for (const [realm, values] of saved) for (const [name, value] of values) realm[name] = value;
    }
  }

  const iframe = document.createElement('iframe');
  document.body.appendChild(iframe);
  const realms = [['top', window], ['child', iframe.contentWindow]];
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms) {
    __blobReadingProgress.stage = `${ownerName}/${calleeName}`;
    await runSuite(owner, callee, `${ownerName}/${calleeName}`, checks);
  }
  iframe.remove();

  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const methods = Object.fromEntries(['text', 'arrayBuffer', 'bytes', 'stream'].map(name => [name, frame.contentWindow.Blob.prototype[name]]));
  frame.remove();
  for (const kind of ['Blob', 'File']) for (const method of ['text', 'arrayBuffer', 'bytes', 'stream']) {
    const source = kind === 'Blob' ? new Blob(['live']) : new File(['live'], 'live.txt');
    const value = methods[method].call(source);
    const reader = method === 'stream' ? value.getReader() : null;
    const result = reader ? (await reader.read()).value : await value;
    checks.push({name: `detached-callee/${kind}/${method}/return belongs to the live Blob realm`, passed:
      Object.getPrototypeOf(value) === (reader ? ReadableStream.prototype : Promise.prototype)});
    const text = method === 'text' ? result : new TextDecoder().decode(result);
    checks.push({name: `detached-callee/${kind}/${method}/read completes`, passed: text === 'live', detail: text});
    if (reader) reader.releaseLock();
  }

  __blobReadingProgress.stage = 'worker';
  const workerBody = async runSuite => {
    const checks = [];
    try {
      await runSuite(globalThis, globalThis, 'worker', checks);
      postMessage({checks});
    } catch (error) {
      checks.push({name: 'worker fixture completion', passed: false, detail: String(error.stack || error)});
      postMessage({checks});
    }
    close();
  };
  const url = URL.createObjectURL(new Blob([`(${workerBody})(${runSuite})`], {type: 'text/javascript'}));
  const worker = new Worker(url);
  try {
    const facts = await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(Error(event.message));
    });
    checks.push(...facts.checks);
  } finally {
    worker.terminate();
    URL.revokeObjectURL(url);
  }
  globalThis.__uiEventResults = {checks};
})().catch(error => {
  __blobReadingProgress.checks.push({name: 'fixture completion', passed: false, detail: String(error.stack || error)});
  globalThis.__uiEventResults = {checks: __blobReadingProgress.checks};
});
