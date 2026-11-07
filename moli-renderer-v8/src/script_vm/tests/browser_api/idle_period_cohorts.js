globalThis.runIdleCohortProbe = scenario => {
  const row = globalThis.__idleCohortProbe = { scenario, done: false };
  let first;
  let marker;
  let markerRan = false;
  const second = deadline => {
    row.previous = first.timeRemaining();
    row.current = deadline.timeRemaining();
    row.didTimeout = deadline.didTimeout;
    row.markerBeforeCallback = markerRan;
    if (marker !== undefined) clearTimeout(marker);
    row.done = true;
  };
  requestIdleCallback(deadline => {
    first = deadline;
    row.initial = deadline.timeRemaining();
    switch (scenario) {
      case 'queued': {
        const start = performance.now();
        while (performance.now() - start < 5) {}
        break;
      }
      case 'callback': requestIdleCallback(second); break;
      case 'microtask': Promise.resolve().then(() => requestIdleCallback(second)); break;
      case 'timer': setTimeout(() => requestIdleCallback(second), 0); break;
      case 'child': document.querySelector('iframe').contentWindow.requestIdleCallback(second); break;
      case 'timeout':
        requestIdleCallback(second, { timeout: 10 });
        marker = setTimeout(() => { markerRan = true; }, 20);
        break;
      default: throw new Error(scenario);
    }
  });
  if (scenario === 'queued') requestIdleCallback(second);
  return 'scheduled';
};
