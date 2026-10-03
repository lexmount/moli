(() => new Promise((resolve, reject) => {
  const source = 'self.postMessage((' + globalThis.__errorEventProbe.toString() + ')([self]));';
  const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
  const worker = new Worker(url);
  const finish = () => {worker.terminate(); URL.revokeObjectURL(url);};
  worker.onmessage = event => {
    globalThis.__uiEventResults = event.data;
    finish();
    resolve(event.data.errors.length === 0 && event.data.rows.every(row => Object.values(row.checks).every(value => value === true)));
  };
  worker.onerror = event => {finish(); reject(new Error(event.message));};
}))()
