(async () => {
  const checks = [];
  globalThis.__blobSliceConversionProgress = {stage: 'setup', checks};
  async function runMatrix(owner, callee, prefix, checks) {
    const slice = callee.Blob.prototype.slice;
    const capture = action => {try {return {value: action()};} catch (error) {return {error};}};
    for (const kind of ['Blob', 'File']) {
      const source = kind === 'Blob' ? new owner.Blob(['data']) : new owner.File(['data'], 'file.txt');
      const label = `${prefix}/${kind}`;
      if (globalThis.__blobSliceConversionProgress) __blobSliceConversionProgress.stage = label;
      const check = (name, passed, detail) => checks.push({name: `${label}/${name}`, passed, detail});
      const invoke = (...args) => slice.call(source, ...args);
      let events = [];
      const index = (name, value) => ({[Symbol.toPrimitive](hint) {events.push(name + ':' + hint); return value;}});
      const mime = {toString() {events.push('type:string'); return 'TEXT/PLAIN';}};
      const result = invoke(index('start', 1.5), index('end', 3.5), mime);
      check('successful conversion order and hints', events.join() === 'start:number,end:number,type:string', events);
      check('rounded bounds and normalized type', await result.text() === 'ta' && result.type === 'text/plain');
      check('result belongs to receiver realm', result instanceof owner.Blob && Object.getPrototypeOf(result) === owner.Blob.prototype && !(result instanceof owner.File));

      for (const stage of ['start', 'end', 'type']) {
        events = [];
        const sentinel = {stage};
        const nextError = {later: true};
        const first = {valueOf() {events.push('start'); if (stage === 'start') throw sentinel; return 1;}};
        const last = {valueOf() {events.push('end'); if (stage === 'end') throw sentinel; if (stage === 'start') throw nextError; return 3;}};
        const type = {toString() {events.push('type'); if (stage === 'type') throw sentinel; if (stage !== 'type') throw nextError; return 'text/plain';}};
        const outcome = capture(() => invoke(first, last, type));
        const expected = stage === 'start' ? 'start' : stage === 'end' ? 'start,end' : 'start,end,type';
        check(stage + ' exception identity and short circuit', outcome.error === sentinel && events.join() === expected,
              {events, expected, originalError: outcome.error === sentinel, laterError: outcome.error === nextError});
      }

      for (const position of ['start', 'end']) {
        for (const [name, value] of [['BigInt', 1n], ['Symbol', Symbol('index')],
             ['object returning BigInt', {valueOf() {return 1n;}}],
             ['non-primitive @@toPrimitive', {[Symbol.toPrimitive]() {return {};}}]]) {
          let later = 0;
          const after = {valueOf() {later++; return 3;}};
          const type = {toString() {later++; return 'text/plain';}};
          const outcome = capture(() => position === 'start' ? invoke(value, after, type) : invoke(0, value, type));
          check(position + ' ' + name + ' rejects before later conversions', outcome.error instanceof callee.TypeError && later === 0,
                {later, error: outcome.error?.name, calleeRealm: outcome.error instanceof callee.TypeError});
        }
      }

      const revoked = owner.Proxy.revocable(source, {});
      revoked.revoke();
      for (const [name, receiver] of [['plain object', {}], ['prototype forgery', Object.create(owner.Blob.prototype)],
                                    ['object inheriting instance', Object.create(source)], ['author Proxy', new owner.Proxy(source, {})],
                                    ['revoked Proxy', revoked.proxy]]) {
        let conversions = 0;
        const value = {valueOf() {conversions++; return 0;}, toString() {conversions++; return '';}};
        const outcome = capture(() => slice.call(receiver, value, value, value));
        check(name + ' rejects receiver before conversions', outcome.error instanceof callee.TypeError && conversions === 0,
              {conversions, error: outcome.error?.name, calleeRealm: outcome.error instanceof callee.TypeError});
      }

      for (const [name, args, expected] of [
        ['omitted arguments', [], 'data'], ['omitted end', [1], 'ata'],
        ['undefined start and end', [undefined, undefined], 'data'], ['undefined end', [1, undefined], 'ata'],
        ['null end', [1, null], ''], ['zero end', [undefined, 0], ''],
        ['negative zero', [-0], 'data'], ['NaN start', [NaN], 'data']
      ]) {
        const value = invoke(...args);
        check(name, await value.text() === expected && value.type === '', {expected, size: value.size});
      }
      for (const [name, type, expected] of [['undefined', undefined, ''], ['null', null, 'null'], ['BigInt', 7n, '7'],
                                          ['non-ASCII', 'text/é', ''], ['control character', 'text/\u0000', '']]) {
        check('contentType ' + name, invoke(0, undefined, type).type === expected);
      }
      const symbolType = capture(() => invoke(0, undefined, Symbol('type')));
      check('contentType Symbol uses callee TypeError', symbolType.error instanceof callee.TypeError);
      events = [];
      const extra = {toString() {events.push('extra'); throw {};}, valueOf() {events.push('extra'); throw {};}};
      check('extra arguments are not converted', (await invoke(1, 3, '', extra).text()) === 'at' && events.length === 0);
    }
  }
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const realms = [['top', window], ['child', frame.contentWindow]];
  for (const [sourceName, owner] of realms) for (const [calleeName, callee] of realms) {
    await runMatrix(owner, callee, `${sourceName}/${calleeName}`, checks);
  }
  __blobSliceConversionProgress.stage = 'worker conversions';
  const workerBody = async runMatrix => {
    const checks = [];
    await runMatrix(globalThis, globalThis, 'worker', checks);
    postMessage(checks); close();
  };
  const url = URL.createObjectURL(new Blob([`(${workerBody})(${runMatrix})`], {type: 'text/javascript'}));
  const worker = new Worker(url);
  try {
    checks.push(...await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(Error(event.message));
    }));
  } finally {worker.terminate(); URL.revokeObjectURL(url); frame.remove();}
  globalThis.__uiEventResults = {checks};
})().catch(error => {
  const checks = globalThis.__blobSliceConversionProgress.checks;
  checks.push({name: 'fixture completion', passed: false, detail: String(error.stack || error)});
  globalThis.__uiEventResults = {checks};
});
