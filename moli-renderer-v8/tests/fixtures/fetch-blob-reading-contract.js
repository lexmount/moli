(async () => {
  const checks = [], rows = [];
  globalThis.__blobReadingProgress = {stage: 'setup', checks, rows};
  const check = (name, passed, detail = '') => checks.push({name, passed: !!passed, detail});
  const fence = async () => { for (let i = 0; i < 16; i++) await Promise.resolve(); };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const realms = [['top', window], ['child', frame.contentWindow]];
  const decode = value => new TextDecoder().decode(value);
  for (const [sourceName, source] of realms) for (const [constructorName, constructor] of realms)
    for (const kind of ['Blob', 'File']) for (const type of ['Request', 'Response']) {
      const prefix = `${sourceName}/${constructorName}/${kind}/${type}`;
      const blob = (text = 'data', mime = 'text/plain') => {
        const result = kind === 'Blob' ? new source.Blob([text], {type: mime})
          : new source.File([text], 'body.txt', {type: mime});
        for (const key of ['stream', 'text', 'arrayBuffer', 'bytes', 'size', 'type'])
          Object.defineProperty(result, key, {get() { throw new Error(`author Blob.${key} accessed`); }});
        return result;
      };
      const create = (value = blob(), options = {}) => type === 'Response'
        ? new constructor.Response(value, options)
        : new constructor.Request(location.href, {method: 'POST', body: value, ...options});
      for (const method of ['text', 'json', 'arrayBuffer', 'bytes', 'blob', 'formData', 'stream']) {
        __blobReadingProgress.stage = `${prefix}/${method}`;
        const text = method === 'json' ? '{"ok":"data"}' : method === 'formData' ? 'a=data&b=two' : 'data';
        const mime = method === 'json' ? 'application/json' : method === 'formData'
          ? 'application/x-www-form-urlencoded' : 'text/plain';
        const object = create(blob(text, mime));
        check(`${prefix}/${method}/native MIME`, object.headers.get('content-type') === mime);
        check(`${prefix}/${method}/unused before reading`, !object.bodyUsed);
        const stream = object.body;
        const reader = method === 'stream' ? stream.getReader() : null;
        const promise = reader ? reader.read() : object[method]();
        let settled = false;
        promise.then(() => { settled = true; }, () => { settled = true; });
        await fence();
        check(`${prefix}/${method}/waits beyond pure microtasks`, !settled);
        const result = await promise;
        let valid;
        if (method === 'stream') valid = !result.done && decode(result.value) === text;
        else if (method === 'json') valid = result.ok === 'data';
        else if (method === 'formData') valid = result.get('a') === 'data' && result.get('b') === 'two';
        else if (method === 'blob') valid = result.type === mime && await result.text() === text;
        else if (method === 'text') valid = result === text;
        else valid = decode(result) === text;
        check(`${prefix}/${method}/complete original bytes`, valid);
        check(`${prefix}/${method}/used and locked`, object.bodyUsed && stream.locked);
        rows.push({prefix, method, streamInBlobRealm: Object.getPrototypeOf(stream) === source.ReadableStream.prototype,
          streamInConstructorRealm: Object.getPrototypeOf(stream) === constructor.ReadableStream.prototype,
          promiseInConstructorRealm: Object.getPrototypeOf(promise) === constructor.Promise.prototype});
        if (reader) {
          check(`${prefix}/${method}/end of stream`, (await reader.read()).done);
          reader.releaseLock();
        }
      }
      __blobReadingProgress.stage = `${prefix}/clone`;
      {
        const object = create();
        const original = object.body;
        const clone = object.clone();
        check(`${prefix}/clone/locks original stream`, original.locked);
        check(`${prefix}/clone/replaces original body`, original !== object.body && object.body !== clone.body);
        const reader = object.body.getReader(), other = clone.body.getReader();
        const first = reader.read(), second = other.read();
        let settled = false;
        Promise.all([first, second]).then(() => {settled = true;});
        await fence();
        check(`${prefix}/clone/both branches wait for native source`, !settled);
        const [a, b] = await Promise.all([first, second]);
        check(`${prefix}/clone/copies branch data`, decode(a.value) === 'data' && decode(b.value) === 'data' && a.value.buffer !== b.value.buffer);
        check(`${prefix}/clone/both branches close`, (await reader.read()).done && (await other.read()).done);
        reader.releaseLock();other.releaseLock();
      }
      __blobReadingProgress.stage = `${prefix}/cancel-clone`;
      {
        const object = create(), clone = object.clone();
        let cancelled = false;
        const cancellation = object.body.cancel('branch').then(() => {cancelled = true;});
        await fence();
        check(`${prefix}/cancel-clone/waits for other branch`, !cancelled);
        check(`${prefix}/cancel-clone/other branch retains data`, await clone.text() === 'data');
        await cancellation;
        check(`${prefix}/cancel-clone/cancellation completes`, cancelled && object.bodyUsed && clone.bodyUsed);
      }
      __blobReadingProgress.stage = `${prefix}/BYOB`;
      {
        const object = create(), reader = object.body.getReader({mode: 'byob'});
        const target = new constructor.Uint8Array(8);
        const pending = reader.read(target);
        let settled = false;
        pending.then(() => {settled = true;});
        await fence();
        check(`${prefix}/BYOB/waits for file reading`, !settled);
        const result = await pending;
        check(`${prefix}/BYOB/original buffer transferred`, target.buffer.byteLength === 0);
        check(`${prefix}/BYOB/receives native bytes`, !result.done && decode(result.value) === 'data');
        check(`${prefix}/BYOB/closes`, (await reader.read(new constructor.Uint8Array(1))).done);
        reader.releaseLock();
      }
      __blobReadingProgress.stage = `${prefix}/cancel`;
      {
        const object = create(), reader = object.body.getReader();
        const read = reader.read();
        await reader.cancel('cancel');
        check(`${prefix}/cancel/pending read closes without bytes`, (await read).done);
        check(`${prefix}/cancel/consumer marked used`, object.bodyUsed);
        reader.releaseLock();
      }
      __blobReadingProgress.stage = `${prefix}/empty`;
      {
        const object = create(blob('')), reader = object.body.getReader();
        const pending = reader.read();
        let settled = false;
        pending.then(() => {settled = true;});
        await fence();
        check(`${prefix}/empty/waits for file reading`, !settled);
        check(`${prefix}/empty/closes without a chunk`, (await pending).done);
        reader.releaseLock();
      }
      if (type === 'Request') {
        __blobReadingProgress.stage = `${prefix}/transfer`;
        const object = create(blob('data'), {mode: 'no-cors', keepalive: true});
        const original = object.body;
        const transferred = new constructor.Request(object);
        check(`${prefix}/transfer/locks source stream`, original.locked && object.bodyUsed);
        check(`${prefix}/transfer/preserves Blob permissions`, transferred.mode === 'no-cors' && transferred.keepalive);
        check(`${prefix}/transfer/preserves data`, await transferred.text() === 'data');
      }
    }
  frame.remove();
  globalThis.__uiEventResults = {checks, rows};
})().catch(error => {
  __blobReadingProgress.checks.push({name: 'fixture completion', passed: false, detail: String(error.stack || error)});
  globalThis.__uiEventResults = {checks: __blobReadingProgress.checks, rows: __blobReadingProgress.rows};
});
