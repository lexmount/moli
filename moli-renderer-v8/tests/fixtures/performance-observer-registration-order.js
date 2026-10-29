(async () => {
  const events = [];
  let acknowledge;
  let remaining;
  const callback = label => list => {
    events.push(...list.getEntries().map(entry => label + ':' + entry.name));
    if (--remaining === 0) acknowledge();
  };
  const first = new PerformanceObserver(callback('first'));
  const second = new PerformanceObserver(callback('second'));
  const batch = async (name, observers) => {
    first.disconnect();
    second.disconnect();
    const delivered = new Promise(resolve => acknowledge = resolve);
    remaining = observers.length;
    for (const observer of observers) observer.observe({ type: 'mark' });
    // Updating an existing registration must preserve its position.
    observers[0].observe({ type: 'mark' });
    performance.mark(name);
    await delivered;
  };
  await batch('initial', [second, first]);
  await batch('reobserved', [first, second]);
  first.disconnect();
  second.disconnect();
  return events;
})()
