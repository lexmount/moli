(() => {
  const failures = [];
  const context = new OfflineAudioContext(1, 128, 8192);
  const source = context.createOscillator();
  const sink = context.createAnalyser();
  source.connect(sink);
  try {
    source.disconnect(sink, {valueOf() { source.disconnect(sink); return 0; }});
    failures.push('accepted an edge removed during conversion');
  } catch (error) {
    if (error.name !== 'InvalidAccessError') failures.push('removed edge: ' + error.name);
  }
  try {
    source.disconnect(sink, {valueOf() { source.connect(sink); return 0; }});
  } catch (error) {
    failures.push('new edge during conversion: ' + error.name);
  }
  let traps = 0;
  const previous = Object.getOwnPropertyDescriptor(Array.prototype, '0');
  Object.defineProperty(Array.prototype, '0', {
    configurable: true,
    set() { traps++; }
  });
  try {
    source.connect(sink);
    sink.connect(context.destination);
    source.disconnect(sink);
    sink.disconnect();
  } finally {
    if (previous) Object.defineProperty(Array.prototype, '0', previous);
    else delete Array.prototype[0];
  }
  if (traps) failures.push('graph storage called page setters: ' + traps);
  return failures;
})()
