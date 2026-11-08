function runMessageSourceProbe(config) {
  const state = globalThis.__probeState = {
    config, complete: false, settled: false, messages: [],
    cancels: 0, reason: null, sourceDesiredSize: null,
    consumer: null, reentry: null, unhandled: []
  };
  let controller;
  let finished = false;
  let grace = null;
  const channel = new MessageChannel();
  const describe = reason => typeof reason === 'number'
    ? {kind: 'number', value: reason}
    : {kind: 'exception', name: reason.name, realm: reason instanceof DOMException};
  addEventListener('unhandledrejection', event => {
    state.unhandled.push({name: event.reason.name, message: event.reason.message});
    event.preventDefault();
  });
  const source = new ReadableStream({
    start(value) {
      controller = value;
      if (config.mode === 'bad-chunk') value.enqueue(() => {});
      else if (config.mode === 'source-error') value.error(new Error('source-failure'));
      else if (config.mode === 'source-close') value.close();
    },
    cancel(reason) {
      state.cancels++;
      state.reason = describe(reason);
      if (config.cancel === 'reject') return Promise.reject(new TypeError('cancel-reject'));
      if (config.cancel === 'throw') throw new TypeError('cancel-throw');
      if (config.cancel === 'pending') return new Promise(() => {});
      if (config.cancel === 'reenter') {
        try {controller.enqueue('unexpected'); state.reentry = {status: 'enqueued'};}
        catch (error) {state.reentry = {status: error.name, realm: error instanceof TypeError};}
      }
    }
  });
  function consume(stream, mode, report) {
    const reader = stream.getReader();
    let operation = null, closed = null, operationReason, closedReason;
    const describe = error => {
      const result = {status: 'rejected', name: error.name, realm: error instanceof DOMException};
      if (error.name === 'Error') result.message = error.message;
      return result;
    };
    const maybe = () => {
      if (operation === null || closed === null) return;
      report({operation, closed, sameError: operation.status === 'rejected' && closed.status === 'rejected'
        ? operationReason === closedReason : null});
    };
    reader.closed.then(
      () => {closed = {status: 'fulfilled'}; maybe();},
      error => {closedReason = error; closed = describe(error); maybe();}
    );
    const promise = mode === 'blocked-cancel'
      ? reader.cancel(new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0])))
      : mode === 'normal-cancel' ? reader.cancel(37) : reader.read();
    promise.then(
      result => {
        operation = {status: 'fulfilled'};
        if (mode === 'source-close') operation.done = result.done;
        maybe();
      },
      error => {operationReason = error; operation = describe(error); maybe();}
    );
  }
  const report = result => {state.consumer = result; maybeFinish();};
  channel.port1.onmessage = event => {
    if (event.data === 'ready') channel.port1.postMessage(source, [source]);
  };
  if (config.mode === 'blocked-cancel') {
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', 'allow-scripts');
    addEventListener('message', event => {
      if (event.source !== frame.contentWindow) return;
      if (event.data.kind === 'ready') {
        frame.contentWindow.postMessage({kind: 'port'}, '*', [channel.port2]);
      } else if (event.data.kind === 'result') report(event.data.result);
    });
    frame.srcdoc = '<!doctype html><script>' + consume.toString() + `
      onmessage = event => {
        const port = event.ports[0];
        port.onmessage = message => {
          consume(message.data, 'blocked-cancel', result => {
            parent.postMessage({kind: 'result', result}, '*');
            port.close();
          });
        };
        port.start();
        port.postMessage('ready');
      };
      parent.postMessage({kind: 'ready'}, '*');
    <\/script>`;
    document.body.appendChild(frame);
  } else {
    channel.port2.onmessage = event => {
      consume(event.data, config.mode, result => {report(result); channel.port2.close();});
    };
    channel.port1.postMessage(source, [source]);
  }
  function finish() {
    if (finished) return;
    finished = true;
    clearTimeout(fence);
    if (grace !== null) clearTimeout(grace);
    state.sourceDesiredSize = controller.desiredSize;
    state.settled = state.consumer !== null &&
      (config.mode === 'source-error' || config.mode === 'source-close' || state.cancels > 0);
    state.messages.push({cancels: state.cancels, reason: state.reason, consumer: state.consumer});
    state.complete = true;
    channel.port1.close();
    globalThis.probeResult = state;
    console.log('probe-done:' + config.name);
  }
  function maybeFinish() {
    if (finished || grace !== null || state.consumer === null) return;
    if (config.mode !== 'source-error' && config.mode !== 'source-close' && state.cancels === 0) return;
    grace = setTimeout(finish, 100);
  }
  // Underlying cancel may run after the consumer promise has already settled.
  const tick = setInterval(() => {maybeFinish(); if (finished) clearInterval(tick);}, 5);
  const fence = setTimeout(finish, 3000);
}
