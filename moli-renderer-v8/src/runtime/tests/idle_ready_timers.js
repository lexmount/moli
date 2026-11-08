function runIdleReadyTimerProbe(variant, onDone = () => {}) {
  const row = globalThis.__idleReadyTimerProbe = {
    variant, done: false, order: [], didTimeout: null, futureRan: false, startedAt: performance.now(), callbacks: []
  };
  const required = ['ready', 'source', 'child-ready', 'expired'].includes(variant) ? 2 : 1;
  const note = label => {
    row.order.push(label);
    row.callbacks.push({label, time:performance.now()});
    if (row.order.length === required) {
      row.done = true;
      onDone();
    }
  };
  globalThis.__idleReadyTimerNote = note;
  const block = () => {
    const start = performance.now();
    while (performance.now() - start < 5) {}
  };
  let future;
  row.idleRegisteredAt = performance.now();
  requestIdleCallback(deadline => {
    row.didTimeout = deadline.didTimeout;
    if (future !== undefined) clearTimeout(future);
    note('idle');
  }, variant === 'expired' ? {timeout: 1} : undefined);
  // Let the idle transport deadline become earlier than the ordinary timer.
  // The second synchronous block ensures that both are due before task return.
  block();
  row.ordinaryRegisteredAt = performance.now();
  if (variant === 'ready' || variant === 'expired') {
    setTimeout(() => note('timer'), 0);
  } else if (variant === 'source') {
    setTimeout("__idleReadyTimerNote('timer')", 0);
  } else if (variant === 'child-ready') {
    const frame = document.createElement('iframe');
    document.body.appendChild(frame);
    frame.contentWindow.setTimeout(() => note('timer'), 0);
  } else if (variant === 'cancelled') {
    const timer = setTimeout(() => note('cancelled timer'), 0);
    clearTimeout(timer);
  } else if (variant === 'future') {
    future = setTimeout(() => { row.futureRan = true; }, 1000);
  } else {
    throw new Error('Unknown idle timer scenario: ' + variant);
  }
  block();
  row.returnedAt = performance.now();
}
