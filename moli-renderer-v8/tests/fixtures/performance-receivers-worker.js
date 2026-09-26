(async () => {
  const source = `postMessage((${__runPerformanceReceiverChecks.toString()})());`;
  const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
  const worker = new Worker(url);
  try {
    return await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(new Error(event.message));
    });
  } finally {
    worker.terminate();
    URL.revokeObjectURL(url);
  }
})();
