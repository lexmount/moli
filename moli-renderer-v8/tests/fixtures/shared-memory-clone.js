(() => {
  const facts = {};
  const probe = (name, operation) => {
    try { facts[name] = operation(); }
    catch (error) { facts[name] = {error: error.name}; }
  };
  probe('graph', () => {
    const buffer = new SharedArrayBuffer(16);
    const view = new Int32Array(buffer, 4, 2);
    Atomics.store(view, 0, 17);
    const graph = {buffer, again: buffer, view, data: new DataView(buffer, 8, 4)};
    graph.self = graph;
    graph.map = new Map([[buffer, graph]]);
    const copy = structuredClone(graph);
    Atomics.add(copy.view, 0, 2);
    copy.data.setUint8(0, 23);
    return [copy !== graph, copy.self === copy, copy.buffer !== buffer,
      copy.buffer === copy.again, copy.view.buffer === copy.buffer,
      copy.data.buffer === copy.buffer, copy.map.get(copy.buffer) === copy,
      copy.view.byteOffset, copy.view.length, Atomics.load(view, 0),
      new Uint8Array(buffer)[8], buffer.byteLength];
  });
  probe('distinctWrappers', () => {
    const first = new SharedArrayBuffer(4);
    const second = structuredClone(first);
    const copy = structuredClone([first, second, first]);
    new Uint8Array(copy[1])[0] = 31;
    return [first !== second, copy[0] !== first, copy[0] !== copy[1],
      copy[0] === copy[2], new Uint8Array(first)[0], new Uint8Array(copy[0])[0]];
  });
  probe('growable', () => {
    const buffer = new SharedArrayBuffer(4, {maxByteLength: 16});
    const copy = structuredClone({buffer, tracking: new Uint8Array(buffer),
      fixed: new Uint8Array(buffer, 0, 4)});
    buffer.grow(8);
    copy.tracking[7] = 41;
    copy.buffer.grow(12);
    return [copy.buffer !== buffer, copy.buffer.growable, copy.buffer.maxByteLength,
      buffer.byteLength, copy.buffer.byteLength, copy.tracking.length,
      copy.fixed.length, new Uint8Array(buffer)[7]];
  });
  probe('wasmMemory', () => {
    const memory = new WebAssembly.Memory({shared: true, initial: 1, maximum: 2});
    const copy = structuredClone({memory, buffer: memory.buffer});
    new Uint8Array(copy.memory.buffer)[0] = 53;
    const sameBuffer = copy.memory.buffer === copy.buffer;
    copy.memory.grow(1);
    return [copy.memory !== memory, copy.buffer !== memory.buffer,
      sameBuffer, new Uint8Array(memory.buffer)[0], memory.buffer.byteLength,
      copy.memory.buffer.byteLength];
  });
  probe('prototype', () => {
    const buffer = new SharedArrayBuffer(2);
    let getterReads = 0;
    Object.setPrototypeOf(buffer, null);
    Object.defineProperty(buffer, 'expando', {enumerable: true,
      get() { getterReads++; throw Error('must not inspect SAB expandos'); }});
    const copy = structuredClone(buffer);
    return [Object.getPrototypeOf(copy) === SharedArrayBuffer.prototype,
      copy.byteLength, getterReads, copy.expando === undefined];
  });
  probe('transfers', () => {
    const shared = new SharedArrayBuffer(4);
    const regular = new ArrayBuffer(2);
    let error;
    try { structuredClone({shared, regular}, {transfer: [regular, shared]}); }
    catch (caught) { error = caught.name; }
    const copy = structuredClone({shared, regular}, {transfer: [regular]});
    return [error, regular.byteLength, copy.regular.byteLength, shared.byteLength,
      copy.shared !== shared];
  });
  if (typeof history !== 'undefined') {
    probe('history', () => ['pushState', 'replaceState'].map(method => {
      try { history[method]({nested: [new SharedArrayBuffer(1)]}, ''); return 'accepted'; }
      catch (error) { return error.name; }
    }));
  }
  probe('nativeIsolation', () => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'crossOriginIsolated');
    try {
      Object.defineProperty(globalThis, 'crossOriginIsolated', {configurable: true, value: false});
      const original = new SharedArrayBuffer(1);
      const copy = structuredClone(original);
      return [globalThis.crossOriginIsolated, copy !== original, copy.byteLength];
    } finally {
      if (descriptor) Object.defineProperty(globalThis, 'crossOriginIsolated', descriptor);
      else delete globalThis.crossOriginIsolated;
    }
  });
  return facts;
})()
