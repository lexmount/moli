function connect(port) {
  let child;
  port.onmessage = event => {
    if (event.data === 'ping') {
      port.postMessage({kind: 'pong'});
      return;
    }
    if (event.data === 'terminate') {
      child.terminate();
      port.postMessage({kind: 'terminated'});
      return;
    }
    port.postMessage({kind: 'before-new'});
    child = new Worker(event.data.url, {
      type: event.data.type,
      credentials: event.data.type === 'module' ? 'include' : 'omit',
    });
    port.postMessage({kind: 'after-new'});
    child.onmessage = event => port.postMessage(event.data);
    child.onerror = event => {
      event.preventDefault();
      port.postMessage({kind: 'child-error', eventType: event.type});
    };
    const bytes = new Uint8Array([13, 21, 34]);
    child.postMessage({value: 'queued-before-load', bytes: bytes.buffer}, [bytes.buffer]);
    port.postMessage({kind: 'transferred', byteLength: bytes.byteLength});
  };
  if (port.start) port.start();
}
