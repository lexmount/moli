(async () => {
  const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
  const events = [];
  let microtaskRan = false;
  const listener = event => events.push({
    blocked: event.blockedURI,
    directive: event.effectiveDirective,
    policy: event.originalPolicy,
    disposition: event.disposition,
    document: event.documentURI,
    sample: event.sample,
    native: event instanceof SecurityPolicyViolationEvent,
    afterMicrotask: microtaskRan,
  });
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
  await new Promise(resolve => setTimeout(resolve, 30));
  removeEventListener('securitypolicyviolation', listener);
  return { results, synchronousEvents, valid, events };
})()
