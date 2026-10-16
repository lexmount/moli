(async () => {
  const checks = [], rows = [];
  const check = (name, passed, detail) => checks.push({name, passed, detail});
  const iframe = document.createElement('iframe');
  document.body.appendChild(iframe);
  const realms = [['top', window], ['child', iframe.contentWindow]];
  const methods = ['readAsText', 'readAsBinaryString', 'readAsDataURL', 'readAsArrayBuffer'];
  globalThis.__fileReaderTaskProgress = {stage:'setup', checks, rows};
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms) for (const method of methods) {
    const prefix = `${ownerName}/${calleeName}/${method}`;
    __fileReaderTaskProgress.stage = prefix;
    const reader = new owner.FileReader(), events = [], results = [];
    let checkpoints = 0;
    for (let round = 0; round < 2; round++) {
      let depth = 0;
      reader.onloadstart = () => events.push(`${round}:start`);
      reader.onprogress = () => {
        events.push(`${round}:progress`);
        Promise.resolve().then(() => depth++).then(() => depth++).then(() => depth++);
      };
      reader.onload = () => {
        if (depth === 3) checkpoints++;
        events.push(`${round}:load`);
        results.push(reader.result);
      };
      const done = new Promise(resolve => reader.onloadend = () => {events.push(`${round}:end`); resolve();});
      callee.FileReader.prototype[method].call(reader, new callee.Blob(['read'], {type:'text/plain'}));
      check(prefix + `/${round}/synchronous loading`, reader.readyState === 1 && reader.result === null);
      await Promise.resolve();
      await Promise.resolve();
      check(prefix + `/${round}/reading is not a microtask`, events.length === round * 4, events.slice());
      await done;
    }
    check(prefix + '/progress checkpoints finish before load', checkpoints === 2, checkpoints);
    check(prefix + '/event order', events.join() === '0:start,0:progress,0:load,0:end,1:start,1:progress,1:load,1:end', events);
    check(prefix + '/result realm and bytes', results.every(value => method === 'readAsArrayBuffer'
      ? value instanceof owner.ArrayBuffer && Array.from(new owner.Uint8Array(value)).join() === '114,101,97,100'
      : method === 'readAsDataURL' ? typeof value === 'string' && value.endsWith('cmVhZA==') : value === 'read'));
    const before = events.length;
    reader.abort();
    check(prefix + '/abort after done clears result without events', reader.readyState === 2 && reader.result === null && events.length === before);
    rows.push({prefix, events});
  }
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms) {
    const prefix = `${ownerName}/${calleeName}/abort-restart`;
    __fileReaderTaskProgress.stage = prefix;
    const reader = new owner.FileReader(), events = [];
    const read = value => callee.FileReader.prototype.readAsText.call(reader, new callee.Blob([value]));
    reader.onloadstart = () => events.push('start');
    reader.onabort = () => {events.push('abort'); read('new');};
    reader.onload = () => events.push('load:' + reader.result);
    const done = new Promise(resolve => reader.onloadend = () => {events.push('end'); resolve();});
    read('old');
    callee.FileReader.prototype.abort.call(reader);
    check(prefix + '/abort is synchronous and restart remains loading', events.join() === 'abort' && reader.readyState === 1 && reader.result === null, events.slice());
    await done;
    check(prefix + '/stale request does not dispatch', events.join() === 'abort,start,load:new,end', events);
    rows.push({prefix, events});
  }
  __fileReaderTaskProgress.stage = 'worker FIFO';
  const workerBody = () => {
    const probe = new FileReader(), blob = new Blob(['probe']);
    let captures = 0;
    probe.onloadend = () => {
      const first = new FileReader(), second = new FileReader(), starts = [];
      let complete = 0;
      const end = () => {if (++complete === 2) {postMessage({starts, captures}); close();}};
      first.onloadstart = () => starts.push('first-new');
      second.onloadstart = () => starts.push('second');
      first.onloadend = second.onloadend = end;
      first.onabort = () => first.readAsText(new Blob(['new']));
      first.readAsText(new Blob(['old']));
      second.readAsText(new Blob(['second']));
      first.abort();
    };
    const descriptor = Object.getOwnPropertyDescriptor(Array.prototype, '0');
    Object.defineProperty(Array.prototype, '0', {
      configurable: true,
      set(value) {
        captures++;
        Object.defineProperty(this, '0', {value, writable:true, enumerable:true, configurable:true});
      }
    });
    try {probe.readAsText(blob);}
    finally {
      if (descriptor) Object.defineProperty(Array.prototype, '0', descriptor);
      else delete Array.prototype[0];
    }
  };
  const url = URL.createObjectURL(new Blob(['(' + workerBody.toString() + ')()'], {type:'text/javascript'}));
  const worker = new Worker(url);
  try {
    const {starts, captures} = await new Promise((resolve, reject) => {worker.onmessage = event => resolve(event.data); worker.onerror = event => reject(Error(event.message));});
    check('worker/aborted ticket preserves FIFO of new requests', starts.join() === 'second,first-new', starts);
    check('worker/internal request queue ignores Array prototype setters', captures === 0, captures);
    rows.push({prefix:'worker FIFO', starts, captures});
  } finally {worker.terminate(); URL.revokeObjectURL(url);}
  iframe.remove();
  globalThis.__uiEventResults = {checks, rows};
})().catch(error => {
  globalThis.__uiEventResults = {checks:[{name:'fixture completion', passed:false, detail:String(error.stack || error)}], rows:[]};
});
