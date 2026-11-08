(async () => {
  const checks = [];
  const progress = globalThis.__fileReaderBindingProgress = {stage: 'setup', checks};
  const check = (name, passed, detail) => checks.push({name, passed, detail});
  const thrown = action => {
    try {action();} catch (error) {return error;}
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const realms = [['top', window], ['child', frame.contentWindow]];
  const methods = ['readAsText', 'readAsDataURL', 'readAsArrayBuffer', 'readAsBinaryString'];

  for (const [ownerName, owner] of realms) {
    for (const [calleeName, callee] of realms) {
      for (const method of methods) {
        const prefix = `${ownerName}/${calleeName}/${method}`;
        progress.stage = prefix;
        const reader = new owner.FileReader();
        const read = (...args) => callee.FileReader.prototype[method].call(reader, ...args);
        const blob = new callee.Blob(['pending']);
        read(blob);
        const missing = thrown(() => read());
        check(prefix + '/missing Blob precedes loading check', missing instanceof callee.TypeError, missing?.name);
        let traps = 0;
        const revoked = Proxy.revocable(blob, {});
        revoked.revoke();
        const invalid = [
          null, undefined, {}, Object.create(blob), Object.create(callee.Blob.prototype),
          new Proxy(blob, {get() {traps++; throw Error('Blob proxy trap');}}), revoked.proxy
        ];
        invalid.forEach((value, index) => {
          const error = thrown(() => read(value));
          check(prefix + `/invalid Blob ${index} precedes loading check`, error instanceof callee.TypeError, error?.name);
        });
        check(prefix + '/Blob conversion does not invoke proxy traps', traps === 0, traps);
        const loading = thrown(() => read(blob));
        check(prefix + '/valid Blob reaches loading check', loading instanceof callee.DOMException && loading.name === 'InvalidStateError', loading?.name);
        check(prefix + '/failed calls preserve pending read', reader.readyState === 1 && reader.result === null);
        reader.abort();

        let conversions = 0;
        const badReceiver = Object.create(reader);
        const error = thrown(() => callee.FileReader.prototype[method].call(
          badReceiver, blob, {toString() {conversions++; return 'utf-8';}}
        ));
        check(prefix + '/receiver check precedes conversion', error instanceof callee.TypeError && conversions === 0);
        const file = new callee.File(['file'], 'file.txt');
        read(file);
        await new Promise(resolve => reader.onloadend = resolve);
        check(prefix + '/cross-realm File remains a valid Blob', method === 'readAsArrayBuffer'
          ? reader.result instanceof owner.ArrayBuffer && Array.from(new owner.Uint8Array(reader.result)).join() === '102,105,108,101'
          : method === 'readAsDataURL' ? reader.result.endsWith('ZmlsZQ==') : reader.result === 'file');
      }

      const prefix = `${ownerName}/${calleeName}/encoding`;
      progress.stage = prefix;
      const reader = new owner.FileReader();
      const blob = new callee.Blob(['outer']);
      const read = (...args) => callee.FileReader.prototype.readAsText.call(reader, ...args);
      read(new callee.Blob(['pending']));
      let conversions = 0;
      let error = thrown(() => read(blob, {toString() {conversions++; return 'utf-8';}}));
      check(prefix + '/encoding converts before loading check', conversions === 1 && error instanceof callee.DOMException && error.name === 'InvalidStateError', error?.name);
      const sentinel = new callee.Error('encoding sentinel');
      error = thrown(() => read(blob, {toString() {throw sentinel;}}));
      check(prefix + '/encoding exception precedes loading check', error === sentinel, error?.name);
      conversions = 0;
      error = thrown(() => read({}, {toString() {conversions++; throw sentinel;}}));
      check(prefix + '/Blob conversion precedes encoding conversion', error instanceof callee.TypeError && conversions === 0, {error: error?.name, conversions});
      error = thrown(() => read(blob, {toString() {reader.abort(); return 'utf-8';}}));
      check(prefix + '/encoding can abort the previous read', error === undefined && reader.readyState === 1, error?.name);
      await new Promise(resolve => reader.onloadend = resolve);
      check(prefix + '/read after conversion abort completes', reader.result === 'outer', reader.result);

      error = thrown(() => read(blob, {toString() {read(new callee.Blob(['inner'])); return 'utf-8';}}));
      check(prefix + '/encoding can make the reader loading', error instanceof callee.DOMException && error.name === 'InvalidStateError', error?.name);
      await new Promise(resolve => reader.onloadend = resolve);
      check(prefix + '/conversion-started read keeps its result', reader.result === 'inner', reader.result);
    }
  }

  progress.stage = 'worker conversions';
  const workerBody = () => {
    const checks = [];
    const check = (name, passed, detail) => checks.push({name, passed, detail});
    const thrown = action => {
      try {action();} catch (error) {return error;}
    };
    const methods = ['readAsText', 'readAsDataURL', 'readAsArrayBuffer', 'readAsBinaryString'];
    const blob = new Blob(['worker']);
    const reader = new FileReader();
    for (const method of methods) {
      reader[method](blob);
      let error = thrown(() => reader[method]());
      check(`worker/${method}/missing Blob precedes loading`, error instanceof TypeError, error?.name);
      error = thrown(() => reader[method]({}));
      check(`worker/${method}/invalid Blob precedes loading`, error instanceof TypeError, error?.name);
      reader.abort();
    }
    reader.readAsText(blob);
    let conversions = 0;
    let error = thrown(() => reader.readAsText(blob, {toString() {conversions++; return 'utf-8';}}));
    check('worker/encoding converts before loading', conversions === 1 && error?.name === 'InvalidStateError');
    const sentinel = Error('worker encoding sentinel');
    error = thrown(() => reader.readAsText(blob, {toString() {throw sentinel;}}));
    check('worker/encoding exception precedes loading', error === sentinel);
    reader.abort();

    const sync = new FileReaderSync();
    for (const method of methods) {
      const revoked = Proxy.revocable(sync, {});
      revoked.revoke();
      let traps = 0;
      const invalid = [null, {}, Object.create(sync), Object.create(FileReaderSync.prototype),
        new Proxy(sync, {get() {traps++; throw Error('receiver trap');}}), revoked.proxy];
      invalid.forEach((receiver, index) => {
        let conversions = 0;
        const error = thrown(() => FileReaderSync.prototype[method].call(
          receiver, blob, {toString() {conversions++; return 'utf-8';}}
        ));
        check(`worker/sync/${method}/receiver ${index} precedes conversion`, error instanceof TypeError && conversions === 0, {error: error?.name, conversions});
      });
      check(`worker/sync/${method}/receiver checks invoke no traps`, traps === 0, traps);
      const value = sync[method](new File(['file'], 'worker.txt'));
      check(`worker/sync/${method}/File inherits Blob`, method === 'readAsArrayBuffer'
        ? value instanceof ArrayBuffer && Array.from(new Uint8Array(value)).join() === '102,105,108,101'
        : method === 'readAsDataURL' ? value.endsWith('ZmlsZQ==') : value === 'file');
    }
    conversions = 0;
    error = thrown(() => sync.readAsText({}, {toString() {conversions++; throw sentinel;}}));
    check('worker/sync/Blob converts before encoding', error instanceof TypeError && conversions === 0, {error: error?.name, conversions});
    error = thrown(() => sync.readAsText(blob, {toString() {throw sentinel;}}));
    check('worker/sync/encoding exception identity', error === sentinel);
    postMessage(checks);
    close();
  };
  const url = URL.createObjectURL(new Blob(['(' + workerBody.toString() + ')()'], {type: 'text/javascript'}));
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
  const checks = globalThis.__fileReaderBindingProgress.checks;
  checks.push({name: 'fixture completion', passed: false, detail: String(error.stack || error)});
  globalThis.__uiEventResults = {checks};
});
