(async () => {
  const checks = [];
  globalThis.__blobConstructorProgress = {stage: 'setup', checks};
  async function runMatrix(owner, callee, prefix, checks) {
    const capture = action => {
      try {return {value: action()};} catch (error) {return {error};}
    };
    for (const kind of ['Blob', 'File']) {
      const label = `${prefix}/${kind}`;
      if (globalThis.__blobConstructorProgress) __blobConstructorProgress.stage = label;
      const C = callee[kind];
      const create = (parts, options, name = 'file.txt') => kind === 'Blob'
        ? new C(parts, options) : new C(parts, name, options);
      const check = (name, passed, detail) => checks.push({name: `${label}/${name}`, passed, detail});
      const events = [];
      const parts = {
        get [Symbol.iterator]() {
          events.push('iterator.get');
          return () => {
            events.push('iterator.call');
            let next = 0;
            return {next() {
              events.push('next');
              return next++ === 0 ? {value: {toString() {events.push('part.string'); return 'a';}}, done: false} : {done: true};
            }};
          };
        }
      };
      const result = create(parts, {
        get endings() {events.push('endings.get'); return {toString() {events.push('endings.string'); return 'transparent';}};},
        get type() {events.push('type.get'); return {toString() {events.push('type.string'); return 'TEXT/PLAIN';}};},
        get lastModified() {events.push('modified.get'); return {valueOf() {events.push('modified.number'); return 17;}};}
      }, {toString() {events.push('name.string'); return 'file.txt';}});
      const expected = ['iterator.get', 'iterator.call', 'next', 'part.string', 'next'];
      if (kind === 'File') expected.push('name.string');
      expected.push('endings.get', 'endings.string', 'type.get', 'type.string');
      if (kind === 'File') expected.push('modified.get', 'modified.number');
      check('argument and inherited dictionary order', events.join() === expected.join(), {events, expected});
      check('successful metadata and bytes', await result.text() === 'a' && result.type === 'text/plain' && (kind === 'Blob' || result.name === 'file.txt' && result.lastModified === 17));

      const partError = new callee.Error('part sentinel');
      const optionError = new callee.Error('option sentinel');
      let gets = 0;
      let outcome = capture(() => create([{toString() {throw partError;}}], {get endings() {gets++; throw optionError;}}));
      check('part exception precedes dictionary getters', outcome.error === partError && gets === 0, {error: outcome.error?.message, gets});
      outcome = capture(() => create([{toString() {throw partError;}}], 17));
      check('part conversion precedes invalid dictionary', outcome.error === partError, outcome.error?.name);
      let conversions = 0;
      outcome = capture(() => create([{toString() {conversions++; return 'a';}}], {endings: 'invalid'}));
      check('part conversion precedes enum validation', conversions === 1 && outcome.error instanceof callee.TypeError, {conversions, error: outcome.error?.name});

      const mutableParts = ['before'];
      const changed = create(mutableParts, {get endings() {mutableParts[0] = 'after'; return 'transparent';}});
      check('dictionary cannot replace already converted strings', await changed.text() === 'before');
      const appendParts = ['a'];
      const appended = create(appendParts, {get endings() {appendParts.push('extra'); return 'transparent';}});
      check('dictionary cannot append to converted sequence', await appended.text() === 'a');

      const data = new owner.Uint8Array([65]);
      const delayed = create([data], {get type() {data[0] = 66; return 'text/plain';}});
      check('buffer copying follows dictionary conversion', await delayed.text() === 'B');
      const earlier = new owner.Uint8Array([65]);
      const modified = create([earlier, {toString() {earlier[0] = 66; return 'S';}}]);
      check('buffer copying follows all part conversions', await modified.text() === 'BS');

      for (const [name, value] of [
        ['resizable buffer', new owner.ArrayBuffer(4, {maxByteLength: 8})],
        ['resizable view', new owner.Uint8Array(new owner.ArrayBuffer(4, {maxByteLength: 8}))]
      ]) {
        outcome = capture(() => create([value]));
        check(name + ' is rejected', outcome.error instanceof callee.TypeError, outcome.error?.name);
      }
      if (typeof owner.SharedArrayBuffer === 'function') {
        for (const [name, value] of [
          ['shared buffer', new owner.SharedArrayBuffer(4)],
          ['shared view', new owner.Uint8Array(new owner.SharedArrayBuffer(4))]
        ]) {
          outcome = capture(() => create([value]));
          check(name + ' is rejected', outcome.error instanceof callee.TypeError, outcome.error?.name);
        }
      }

      const item = new owner.Blob(['blob']);
      item.toString = () => {throw partError;};
      check('native Blob union ignores string coercion', await create([item]).text() === 'blob');
      const noClose = {count: 0};
      outcome = capture(() => create({[Symbol.iterator]() {
        return {next() {return {value: {toString() {throw partError;}}, done: false};}, return() {noClose.count++; return {done: true};}};
      }}));
      check('sequence failure propagates without IteratorClose', outcome.error === partError && noClose.count === 0);

      if (kind === 'File') {
        let names = 0;
        outcome = capture(() => create([{toString() {throw partError;}}], undefined, {toString() {names++; throw optionError;}}));
        check('fileBits precede fileName conversion', outcome.error === partError && names === 0, {names, error: outcome.error?.message});
        for (const [name, value, expected] of [
          ['NaN', NaN, 0], ['infinity', Infinity, 0], ['negative infinity', -Infinity, 0],
          ['signed wrap', 2 ** 63, -(2 ** 63)], ['unsigned wrap', 2 ** 64 + 8192, 8192]
        ]) {
          const file = create([], {lastModified: value});
          check('lastModified ' + name, file.lastModified === expected, {actual: file.lastModified, expected});
        }
      }
    }
    if (owner.document) {
      const list = owner.document.querySelectorAll('__missing');
      const iteratorEvents = [];
      Object.defineProperty(list, Symbol.iterator, {configurable: true, get() {
        iteratorEvents.push('get');
        return function* () {iteratorEvents.push('run'); yield 'custom';};
      }});
      const value = new callee.Blob(list);
      checks.push({name: `${prefix}/platform custom iterator`, passed: await value.text() === 'custom' && iteratorEvents.join() === 'get,run', detail: iteratorEvents});
    }
  }
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const realms = [['top', window], ['child', frame.contentWindow]];
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms) {
    await runMatrix(owner, callee, `${ownerName}/${calleeName}`, checks);
  }
  __blobConstructorProgress.stage = 'worker constructors';
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
  const checks = globalThis.__blobConstructorProgress.checks;
  checks.push({name: 'fixture completion', passed: false, detail: String(error.stack || error)});
  globalThis.__uiEventResults = {checks};
});
