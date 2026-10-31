(async () => {
  const facts = {};
  const leafSource = `onmessage = event => {
    const { graph, port } = event.data;
    const original = graph.buffer;
    const copy = structuredClone(graph);
    Atomics.add(copy.view, 0, 7);
    const sameGraph = copy.buffer === copy.again && copy.view.buffer === copy.buffer;
    const nativeIsolated = self.crossOriginIsolated;
    Object.defineProperty(self, 'crossOriginIsolated', {configurable: true, value: false});
    let nativeClone = false;
    try { nativeClone = structuredClone(original) !== original; }
    catch (error) { nativeClone = error.name; }
    const result = {graph: copy, sameGraph, nativeIsolated, nativeClone};
    if (port) port.postMessage(result);
    else postMessage(result);
    close();
  };`;
  for (const type of ['classic', 'module']) {
    for (const nested of [false, true]) {
      for (const viaPort of [false, true]) {
        const key = [type, nested ? 'nested' : 'direct', viaPort ? 'port' : 'worker'].join('/');
        const urls = [];
        const channel = viaPort ? new MessageChannel() : null;
        let worker;
        let timer;
        try {
          const leafURL = URL.createObjectURL(new Blob([leafSource], {type: 'text/javascript'}));
          urls.push(leafURL);
          let sourceURL = leafURL;
          if (nested) {
            const parentSource = `const child = new Worker(${JSON.stringify(leafURL)}, {type: ${JSON.stringify(type)}});
              child.onmessage = event => { postMessage(event.data); child.terminate(); close(); };
              child.onerror = event => { throw Error(event.message); };
              onmessage = event => {
                child.postMessage(event.data, event.data.port ? [event.data.port] : []);
              };`;
            sourceURL = URL.createObjectURL(new Blob([parentSource], {type: 'text/javascript'}));
            urls.push(sourceURL);
          }
          worker = new Worker(sourceURL, {type});
          const buffer = new SharedArrayBuffer(16);
          const view = new Int32Array(buffer, 4, 2);
          Atomics.store(view, 0, 11);
          const graph = {buffer, again: buffer, view};
          const reply = new Promise((resolve, reject) => {
            timer = setTimeout(() => reject(Error('shared-memory reply deadline')), 3500);
            const receiver = channel ? channel.port1 : worker;
            receiver.onmessage = event => resolve(event.data);
            receiver.onmessageerror = () => reject(Error('messageerror'));
            worker.onerror = event => reject(Error(event.message));
          });
          worker.postMessage({graph, port: channel ? channel.port2 : null}, channel ? [channel.port2] : []);
          const result = await reply;
          worker.terminate();
          Atomics.add(result.graph.view, 0, 2);
          facts[key] = [result.sameGraph, result.nativeIsolated, result.nativeClone,
            result.graph.buffer !== buffer, result.graph.view.buffer === result.graph.buffer,
            result.graph.buffer === result.graph.again, Atomics.load(view, 0),
            result.graph.view.byteOffset, result.graph.view.length, buffer.byteLength];
        } catch (error) {
          facts[key] = {error: error.name, message: error.message};
        } finally {
          clearTimeout(timer);
          if (worker) worker.terminate();
          if (channel) { channel.port1.close(); channel.port2.close(); }
          for (const url of urls) URL.revokeObjectURL(url);
        }
      }
    }
  }
  return facts;
})()
