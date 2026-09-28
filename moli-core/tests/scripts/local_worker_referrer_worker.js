function connect(channel) {
  channel.onmessage = async ({data}) => {
    if (data.kind === 'close') {
      self.close();
      return;
    }
    try {
      let value;
      if (data.kind === 'fetch') {
        const options = {referrer: data.referrer};
        if (data.policy !== undefined) options.referrerPolicy = data.policy;
        value = await (await fetch(data.url, options)).json();
      } else if (data.kind === 'blob') {
        value = URL.createObjectURL(new Blob([data.source], {type: 'text/javascript'}));
      } else if (data.kind === 'entry-referrer') {
        value = globalThis.workerEntryReferrer ?? null;
      } else if (data.kind === 'nested') {
        const url = data.url || URL.createObjectURL(new Blob([data.source], {type: 'text/javascript'}));
        const worker = new Worker(url, {type: data.type});
        try {
          value = await new Promise((resolve, reject) => {
            worker.onmessage = event => event.data.error ? reject(new Error(event.data.error)) : resolve(event.data.value);
            worker.onerror = event => reject(new Error(event.message));
            worker.postMessage({...data.command, id: 1});
          });
        } finally {
          worker.terminate();
          if (!data.url) URL.revokeObjectURL(url);
        }
      } else {
        throw new Error('Unknown command: ' + data.kind);
      }
      channel.postMessage({id: data.id, value});
    } catch (error) {
      channel.postMessage({id: data.id, error: error.name + ': ' + error.message});
    }
  };
  if (channel !== self) channel.start();
}

if ('onconnect' in self) {
  self.onconnect = event => connect(event.ports[0]);
} else {
  connect(self);
}
