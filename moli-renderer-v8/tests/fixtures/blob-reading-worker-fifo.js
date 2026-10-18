(async () => {
  globalThis.__blobReadingProgress = {stage: 'worker file-reading FIFO'};
  const body = async () => {
    const checks = [];
    try {
      for (const method of ['text', 'arrayBuffer', 'bytes', 'stream']) {
        const events = [];
        const reader = new FileReader();
        let depth = 0;
        const done = new Promise(resolve => reader.onloadend = resolve);
        reader.onloadstart = () => {
          events.push('start');
          queueMicrotask(() => events.push('start-microtask'));
        };
        reader.onprogress = () => events.push(`progress:${depth}`);
        reader.onload = () => events.push(`load:${reader.result}`);
        reader.readAsText(new Blob(['reader']));
        const value = new Blob(['blob'])[method]();
        const streamReader = method === 'stream' ? value.getReader() : null;
        const pending = streamReader ? streamReader.read() : value;
        pending.then(result => {
          const bytes = streamReader ? result.value : result;
          const text = method === 'text' ? bytes : new TextDecoder().decode(bytes);
          events.push(text);
          Promise.resolve().then(() => depth++).then(() => depth++).then(() => depth++);
          if (streamReader) streamReader.releaseLock();
        });
        checks.push({name: `${method}/not synchronous or microtask completion`, passed: events.length === 0 && reader.readyState === 1});
        await done;
        checks.push({name: `${method}/same source FIFO`, passed: events.join() === 'start,start-microtask,blob,progress:3,load:reader', detail: events});
        checks.push({name: `${method}/complete checkpoint before next source turn`, passed: depth === 3 && reader.readyState === 2});
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
    worker.terminate();
    URL.revokeObjectURL(url);
  }
})().catch(error => {
  globalThis.__uiEventResults = {checks: [{name: 'fixture completion', passed: false, detail: String(error.stack || error)}]};
});
