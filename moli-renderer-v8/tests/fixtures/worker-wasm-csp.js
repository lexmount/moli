(async () => {
  const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
  const events = [];
  let microtaskRan = false;
  let reportsComplete;
  const reportsDone = new Promise(resolve => { reportsComplete = resolve; });
  const listener = event => { events.push({
    blocked: event.blockedURI,
    directive: event.effectiveDirective,
    policy: event.originalPolicy,
    disposition: event.disposition,
    document: event.documentURI,
    sample: event.sample,
    native: event instanceof SecurityPolicyViolationEvent,
    afterMicrotask: microtaskRan,
  });
    if (events.length === globalThis.__expectedWasmCspReports) reportsComplete();
  };
  addEventListener('securitypolicyviolation', listener);
  queueMicrotask(() => { microtaskRan = true; });
  const response = () => new Response(bytes, { headers: { 'Content-Type': 'application/wasm' } });
  const results = [];
  let synchronousEvents;
  for (const [name, run] of [
    ['Module', () => new WebAssembly.Module(bytes)],
    ['compile', () => WebAssembly.compile(bytes)],
    ['instantiate', () => WebAssembly.instantiate(bytes)],
    ['compileStreaming', () => WebAssembly.compileStreaming(response())],
    ['instantiateStreaming', () => WebAssembly.instantiateStreaming(Promise.resolve(response()))],
  ]) {
    try {
      const value = run();
      if (name === 'Module') synchronousEvents = events.length;
      await value;
      results.push([name, 'allowed']);
    } catch (error) {
      if (name === 'Module') synchronousEvents = events.length;
      results.push([name, error instanceof WebAssembly.CompileError ? 'CompileError' : error.name]);
    }
  }
  const valid = WebAssembly.validate(bytes);
  const observation = new Promise(resolve => setTimeout(resolve, 30));
  if (globalThis.__expectedWasmCspReports > 0) await reportsDone;
  await observation;
  removeEventListener('securitypolicyviolation', listener);
  return { results, synchronousEvents, valid, events };
})()
