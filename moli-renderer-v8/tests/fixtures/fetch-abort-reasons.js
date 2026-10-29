globalThis.runFetchAbortReasonContract = async function(base, prefix) {
  const checks = [];
  const check = (name, passed) => checks.push({name:`${prefix}/${name}`,passed:!!passed});
  const reasonFor = kind => {
    switch (kind) {
      case 'default': return undefined;
      case 'object': return {abort:'identity'};
      case 'Error': return new Error('abort identity');
      case 'DOMException': return new DOMException('abort identity','DataError');
      case 'string': return 'abort identity';
      case 'number': return 42;
      case 'null': return null;
      case 'Symbol': return Symbol('abort identity');
      case 'function': return function abortReason() {};
    }
  };
  const kinds = ['default','object','Error','DOMException','string','number','null','Symbol','function'];
  for (const kind of kinds) {
    for (const method of ['text','arrayBuffer','bytes','blob','json','formData']) {
      for (const phase of ['pending','after-abort']) {
        const name = `${kind}/${method}/${phase}`;
        const id = `${prefix}-${kind}-${method}-${phase}`;
        const endpoint = path => new URL(path+'?id='+encodeURIComponent(id),base).href;
        const controller = new AbortController(), reason = reasonFor(kind);
        try {
          const response = await fetch(endpoint('/stream'),{signal:controller.signal});
          check(`${name}/initial body unused`,!response.bodyUsed);
          let pending, settled = false;
          if (phase === 'pending') {
            pending = response[method]();
            pending.then(() => {settled = true;},() => {settled = true;});
            for (let i = 0; i < 16; i++) await Promise.resolve();
            check(`${name}/body waits for remaining network bytes`,!settled);
          }
          controller.abort(reason);
          const expected = controller.signal.reason;
          check(`${name}/signal keeps reason`,kind === 'default'
            ? expected instanceof DOMException && expected.name === 'AbortError'
            : expected === reason);
          if (phase === 'after-abort') pending = response[method]();
          const [result] = await Promise.allSettled([pending]);
          check(`${name}/body rejects with exact signal reason`,result.status === 'rejected' && result.reason === expected);
          check(`${name}/body is disturbed`,response.bodyUsed);
        } finally {
          await fetch(endpoint('/release')).then(response => response.text());
        }
      }
    }
  }
  return {checks};
};
