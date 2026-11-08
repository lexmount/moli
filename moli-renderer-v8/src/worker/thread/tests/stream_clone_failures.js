function probeStreamCloneFailures(poisonBridge) {
  let bridgeReads = 0;
  if (poisonBridge) {
    Object.defineProperty(globalThis, '__moliNativeBridge', {
      configurable: true,
      get() { ++bridgeReads; return undefined; }
    });
  }
  const results = [];
  for (const order of ['tr', 'rt', 'tw', 'wt']) {
    const transform = new TransformStream();
    const members = {t: transform, r: transform.readable, w: transform.writable};
    const transfer = [...order].map(key => members[key]);
    let caught = null;
    try {
      structuredClone(transfer, {transfer});
    } catch (error) {
      caught = [error.name, error instanceof DOMException];
    }
    results.push(caught);
  }
  for (const kind of ['readable', 'writable']) {
    const readable = new ReadableStream();
    const writable = new WritableStream();
    const lock = kind === 'readable' ? readable.getReader() : writable.getWriter();
    const transfer = kind === 'readable' ? [writable, readable] : [readable, writable];
    let caught = null;
    try {
      structuredClone(transfer, {transfer});
    } catch (error) {
      caught = [error.name, error instanceof DOMException];
    }
    results.push(caught);
    lock.releaseLock();
  }
  const marker = {};
  let sameMarker = false;
  try {
    structuredClone(null, {get transfer() { throw marker; }});
  } catch (error) {
    sameMarker = error === marker;
  }
  let functionError = null;
  try {
    structuredClone(() => {});
  } catch (error) {
    functionError = [error.name, error instanceof DOMException];
  }
  if (poisonBridge) delete globalThis.__moliNativeBridge;
  return {results, sameMarker, functionError, bridgeReads};
}
