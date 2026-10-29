(async () => {
  const checks = [];
  globalThis.__blobConstructorProgress = {stage: 'detachment', checks};
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  try {
    const realms = [['top', window], ['child', frame.contentWindow]];
    for (const [sourceName, owner] of realms) for (const [calleeName, callee] of realms) {
      for (const kind of ['Blob', 'File']) for (const source of ['buffer', 'typed view', 'DataView']) {
        for (const phase of ['before conversion', 'dictionary conversion', 'later part conversion']) {
          const buffer = new owner.ArrayBuffer(4);
          new owner.Uint8Array(buffer).set([65, 66, 67, 68]);
          const value = source === 'buffer' ? buffer : source === 'typed view'
            ? new owner.Uint8Array(buffer, 1, 2) : new owner.DataView(buffer, 1, 2);
          const detach = () => buffer.transfer();
          if (phase === 'before conversion') detach();
          const parts = [value];
          if (phase === 'later part conversion') parts.push({toString() {detach(); return 'S';}});
          const options = phase === 'dictionary conversion' ? {get type() {detach(); return 'text/plain';}} : {};
          const expected = phase === 'later part conversion' ? 'S' : '';
          const name = `${sourceName}/${calleeName}/${kind}/${source}/${phase}`;
          try {
            const result = kind === 'Blob' ? new callee.Blob(parts, options) : new callee.File(parts, 'name', options);
            const actual = await result.text();
            checks.push({name, passed: actual === expected && buffer.byteLength === 0, detail: {actual, expected}});
          } catch (error) {checks.push({name, passed: false, detail: String(error.stack || error)});}
        }
      }
    }
  } finally {frame.remove();}
  globalThis.__uiEventResults = {checks};
})().catch(error => {globalThis.__uiEventResults = {checks: [{name: 'fixture completion', passed: false, detail: String(error.stack || error)}]};});
