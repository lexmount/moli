(() => {
  const results = globalThis.__idleOptionsProbe = [];
  globalThis.__idleOptionsDone = false;
  const marker = {};
  const cases = [
    ['omitted', () => undefined, true],
    ['undefined', () => undefined],
    ['null-options', () => null],
    ['zero', () => ({ timeout: 0 })],
    ['negative-zero', () => ({ timeout: -0 })],
    ['undefined-member', () => ({ timeout: undefined })],
    ['null-member', () => ({ timeout: null })],
    ['nan', () => ({ timeout: NaN })],
    ['infinity', () => ({ timeout: Infinity })],
    ['negative-one', () => ({ timeout: -1 })],
    ['wrapped-zero', () => ({ timeout: 2 ** 32 })],
    ['wrapped-positive', () => ({ timeout: 2 ** 32 + 1 })],
    ['fractional-zero', () => ({ timeout: 0.9 })],
    ['positive', () => ({ timeout: 1 })],
    ['coerced', row => ({
      get timeout() {
        row.gets++;
        return { valueOf() { row.coercions++; return 2 ** 32 + 1; } };
      }
    })],
    ['boolean-options', () => false],
    ['number-options', () => 0],
    ['string-options', () => ''],
    ['symbol-member', () => ({ timeout: Symbol('timeout') })],
    ['bigint-member', () => ({ timeout: 1n })],
    ['throwing-getter', row => ({ get timeout() { row.gets++; throw marker; } })],
    ['throwing-coercion', row => ({
      get timeout() {
        row.gets++;
        return { valueOf() { row.coercions++; throw marker; } };
      }
    })],
  ];
  for (const [name, makeOptions, omitted] of cases) {
    const row = { name, outcome: 'ok', calls: 0, didTimeout: null,
      timeCorrect: null, positiveHandle: false, gets: 0, coercions: 0 };
    results.push(row);
    const callback = deadline => {
      row.calls++;
      row.didTimeout = deadline.didTimeout;
      row.timeCorrect = deadline.didTimeout ? deadline.timeRemaining() === 0 : deadline.timeRemaining() > 0;
    };
    try {
      const handle = omitted ? requestIdleCallback(callback) : requestIdleCallback(callback, makeOptions(row));
      row.positiveHandle = typeof handle === 'number' && handle > 0;
    } catch (error) {
      row.outcome = error === marker ? 'marker' : error.name;
    }
  }
  // Every positive one-millisecond timeout is already due before dispatch.
  const start = performance.now();
  while (performance.now() - start < 10) {}
  requestIdleCallback(() => requestIdleCallback(() => { globalThis.__idleOptionsDone = true; }));
  return 'scheduled';
})()
