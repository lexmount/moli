function setup(port) {
  let child;
  port.onmessage = event => {
    child = new Worker(event.data.url, {type: event.data.type, credentials: event.data.type === 'module' ? 'include' : 'omit'});
    child.onmessage = event => port.postMessage(event.data);
    child.onerror = event => port.postMessage({kind: 'child-error', eventType: event.type});
    child.postMessage('queued-before-load');
  };
  if (port.start) port.start();
}
