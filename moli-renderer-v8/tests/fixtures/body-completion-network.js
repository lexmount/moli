(async () => {
  const failures = [];
  let total = 0;
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const run = async (name, action) => {
    total++;
    try { await action(); } catch (error) { failures.push(name + ': ' + error); }
  };
  const checkpoint = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  };
  const origin = __fetchOrigins[0];
  for (const method of ['bytes', 'arrayBuffer', 'blob', 'text', 'json', 'formData']) {
    await run('network EOF ' + method, async () => {
      const key = 'body-eof-' + method;
      const response = await fetch(origin + '/image?gate=body&key=' + key);
      const reader = response.clone().body.getReader();
      let settled = false;
      const consumed = response[method]().then(
        value => { settled = true; return {value}; },
        error => { settled = true; return {error}; },
      );
      // The clone observes the same network EOF. Its closed promise checkpoint
      // must finish before Body's queued conversion can settle its promise.
      const observed = reader.closed.then(async () => { await checkpoint(); return settled; });
      const drain = (async () => {
        const chunks = [];
        while (true) {
          const {done, value} = await reader.read();
          if (done) return new Uint8Array(chunks);
          chunks.push(...value);
        }
      })();
      await checkpoint();
      check(response.bodyUsed && response.body.locked && !settled, 'native pending read locks and waits for EOF');
      await fetch(origin + '/release?key=' + key);
      const [output, early, expected] = await Promise.all([consumed, observed, drain]);
      check(!early, 'EOF conversion waits for a fetch task');
      if (method === 'json' || method === 'formData') {
        check(output.error instanceof (method === 'json' ? SyntaxError : TypeError), 'conversion error propagates');
      } else {
        check(!('error' in output), 'network read succeeds: ' + output.error);
        if (method === 'text') check(output.value === new TextDecoder().decode(expected), 'UTF-8 bytes preserved');
        else {
          const actual = method === 'blob' ? new Uint8Array(await output.value.arrayBuffer()) : new Uint8Array(output.value);
          check(actual.length === expected.length && actual.every((value, i) => value === expected[i]), 'all streamed bytes preserved');
        }
      }
    });

    await run('network abort ' + method, async () => {
      const key = 'body-abort-' + method;
      const controller = new AbortController();
      const response = await fetch(origin + '/image?gate=body&key=' + key, {signal: controller.signal});
      let settled = false;
      const consumed = response[method]().then(
        () => { settled = true; return {success: true}; },
        error => { settled = true; return {error}; },
      );
      const reason = {aborted: method};
      controller.abort(reason);
      await checkpoint();
      const early = settled;
      const output = await consumed;
      await fetch(origin + '/release?key=' + key);
      check(!early, 'network read error waits for a fetch task');
      check(!output.success && output.error === reason, 'abort reason identity is preserved');
    });
  }
  return JSON.stringify({total, failures});
})();
