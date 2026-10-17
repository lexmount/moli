(owner = globalThis) => {
  const checks = [];
  const check = (name, passed, details = '') => checks.push({name, passed: !!passed, details});
  const performance = owner.performance;
  const prototype = owner.Performance.prototype;
  const now = prototype.now.bind(performance);
  const mark = prototype.mark.bind(performance);
  const measure = prototype.measure.bind(performance);
  const clearMarks = prototype.clearMarks.bind(performance);
  const clearMeasures = prototype.clearMeasures.bind(performance);
  const prefix = '__userTimingClock:';
  const names = [];
  const name = suffix => {
    const value = prefix + suffix;
    names.push(value);
    return value;
  };
  const rounded = value => Number(value.toFixed(2));
  try {
    const boundaryName = name('boundary');
    mark(boundaryName, {startTime: 123.456789123});
    const samples = [
      ['mark', suffix => mark(name(suffix)), entry => entry.startTime],
      ['constructor', suffix => new owner.PerformanceMark(name(suffix)), entry => entry.startTime],
      ['measure', suffix => measure(name(suffix)), entry => entry.startTime + entry.duration],
      ['empty-options', suffix => measure(name(suffix), {}), entry => entry.startTime + entry.duration],
      ['legacy-start', suffix => measure(name(suffix), boundaryName), entry => entry.startTime + entry.duration],
      ['options-start', suffix => measure(name(suffix), {start: boundaryName}), entry => entry.startTime + entry.duration],
    ];
    for (let index = 0; index < 32; index++) {
      for (const [kind, create, timestamp] of samples) {
        const before = rounded(now());
        const entry = create(`${kind}:${index}`);
        const after = rounded(now());
        const value = rounded(timestamp(entry));
        check(`${kind}:${index}:within-now`, before <= value && value <= after,
          JSON.stringify({before, value, after}));
      }
    }

    const exact = 123.456789123;
    check('explicit-mark-keeps-precision', mark(name('explicit'), {startTime: exact}).startTime === exact);
    check('explicit-constructor-keeps-precision',
      new owner.PerformanceMark(name('explicit-constructor'), {startTime: exact}).startTime === exact);
    const end = 321.987654321;
    const explicit = measure(name('explicit-measure'), {start: exact, end});
    check('explicit-measure-keeps-precision', explicit.startTime === exact && explicit.duration === end - exact);

    let authorNowCalls = 0;
    const ownNow = Object.getOwnPropertyDescriptor(performance, 'now');
    Object.defineProperty(performance, 'now', {configurable: true, value() {
      authorNowCalls++;
      throw new Error('User Timing must use its native clock');
    }});
    try {
      mark(name('native-mark'));
      new owner.PerformanceMark(name('native-constructor'));
      measure(name('native-measure'));
      check('implicit-timestamps-ignore-author-now', authorNowCalls === 0);
    } finally {
      if (ownNow) Object.defineProperty(performance, 'now', ownNow);
      else delete performance.now;
    }
  } finally {
    for (const value of names) {
      clearMarks(value);
      clearMeasures(value);
    }
  }
  return checks;
}
