(async () => {
  const rows = [];
  for (const mode of ['undefined', 'deleted', 'replacement', 'getter', 'unmaterialized', 'poison-init', 'shadow', 'forward']) {
    const source = `
      const mode = ${JSON.stringify(mode)};
      const NativeErrorEvent = mode === 'unmaterialized' ? null : ErrorEvent;
      const marker = new TypeError('native error marker');
      let calls = 0, gets = 0, fieldReads = 0, handler, after;
      self.addEventListener('error', event => {
        if (mode === 'shadow') {
          for (const field of ['message', 'filename', 'lineno', 'colno', 'error']) {
            Object.defineProperty(event, field, {get() { fieldReads++; throw new Error('public getter'); }});
          }
        }
      });
      self.onerror = function(message, filename, line, column, error) {
        handler = {
          count: arguments.length,
          receiver: this === self,
          message: typeof message === 'string' && message.includes('native error marker'),
          filename: typeof filename === 'string' && filename.length > 0,
          location: line > 0 && column > 0,
          identity: error === marker
        };
        return mode !== 'forward';
      };
      self.addEventListener('error', event => {
        after = {
          errorEvent: NativeErrorEvent ? event instanceof NativeErrorEvent : Object.prototype.toString.call(event) === '[object ErrorEvent]',
          cancelable: event.cancelable,
          trusted: event.isTrusted,
          prevented: event.defaultPrevented
        };
      });
      if (mode === 'deleted') delete self.ErrorEvent;
      if (['undefined', 'unmaterialized'].includes(mode)) self.ErrorEvent = undefined;
      if (mode === 'replacement') self.ErrorEvent = function() { calls++; throw new Error('public constructor'); };
      if (mode === 'getter') Object.defineProperty(self, 'ErrorEvent', {configurable: true, get() { gets++; throw new Error('public constructor getter'); }});
      if (mode === 'poison-init') Object.defineProperty(Object.prototype, 'bubbles', {configurable: true, get() { gets++; throw new Error('dictionary getter'); }});
      setTimeout(() => {
        setTimeout(() => {
          if (mode === 'poison-init') delete Object.prototype.bubbles;
          postMessage({handler, after, calls, gets, fieldReads});
        }, 0);
        throw marker;
      }, 0);
    `;
    rows.push(await new Promise(resolve => {
      const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
      const worker = new Worker(url);
      let parentErrors = 0;
      worker.onerror = event => { parentErrors++; event.preventDefault(); };
      worker.onmessage = event => {
        worker.terminate();
        URL.revokeObjectURL(url);
        resolve({mode, parentErrors, ...event.data});
      };
    }));
  }
  return rows;
})()
