(async () => {
  globalThis.__blobReadingProgress = {stage: 'worker Blob bodies'};
  const body = async () => {
    const checks = [];
    const check = (name, passed) => checks.push({name, passed: !!passed});
    const fence = async () => {for (let i = 0; i < 16; i++) await Promise.resolve();};
    const decode = value => new TextDecoder().decode(value);
    try {
      for (const kind of ['Blob', 'File']) for (const type of ['Request', 'Response']) {
        const prefix = `${kind}/${type}`;
        const create = () => {
          const blob = kind === 'Blob' ? new Blob(['data']) : new File(['data'], 'body.txt');
          Object.defineProperty(blob, 'stream', {get() { throw Error('public Blob.stream read'); }});
          return type === 'Response' ? new Response(blob)
            : new Request(location.href, {method: 'POST', body: blob});
        };
        for (const method of ['text', 'bytes', 'stream']) {
          const object = create(), stream = object.body;
          check(`${prefix}/${method}/unused`, !object.bodyUsed);
          const reader = method === 'stream' ? stream.getReader() : null;
          const pending = reader ? reader.read() : object[method]();
          let settled = false;
          pending.then(() => {settled = true;}, () => {settled = true;});
          await fence();
          check(`${prefix}/${method}/file-reading boundary`, !settled);
          const result = await pending;
          check(`${prefix}/${method}/native data`, (reader ? decode(result.value)
            : method === 'text' ? result : decode(result)) === 'data');
          check(`${prefix}/${method}/used and locked`, object.bodyUsed && stream.locked);
          if (reader) reader.releaseLock();
        }
        {
          const object = create(), original = object.body, clone = object.clone();
          check(`${prefix}/clone/original locked`, original.locked);
          const a = object.body.getReader(), b = clone.body.getReader();
          const pending = Promise.all([a.read(), b.read()]);
          let settled = false;
          pending.then(() => {settled = true;});
          await fence();
          check(`${prefix}/clone/native source task`, !settled);
          const [first, second] = await pending;
          check(`${prefix}/clone/independent byte copies`, decode(first.value) === 'data'
            && decode(second.value) === 'data' && first.value.buffer !== second.value.buffer);
          check(`${prefix}/clone/both close`, (await a.read()).done && (await b.read()).done);
          a.releaseLock();b.releaseLock();
        }
        {
          const reader = create().body.getReader({mode: 'byob'}), target = new Uint8Array(8);
          const pending = reader.read(target);
          let settled = false;
          pending.then(() => {settled = true;});
          await fence();
          check(`${prefix}/BYOB/native source task`, !settled);
          const result = await pending;
          check(`${prefix}/BYOB/transfer and data`, target.buffer.byteLength === 0 && decode(result.value) === 'data');
          check(`${prefix}/BYOB/closes`, (await reader.read(new Uint8Array(1))).done);
          reader.releaseLock();
        }
      }
    } catch (error) {
      checks.push({name: 'worker completion', passed: false, detail: String(error.stack || error)});
    }
    postMessage({checks});
    close();
  };
  const url = URL.createObjectURL(new Blob([`(${body})()`], {type: 'text/javascript'}));
  const worker = new Worker(url);
  try {
    globalThis.__uiEventResults = await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(Error(event.message));
    });
  } finally {
    worker.terminate();URL.revokeObjectURL(url);
  }
})().catch(error => {
  globalThis.__uiEventResults = {checks: [{name: 'fixture completion', passed: false, detail: String(error.stack || error)}]};
});
