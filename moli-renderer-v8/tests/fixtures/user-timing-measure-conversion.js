(owner = globalThis, callee = owner) => {
  const checks = [];
  const performance = owner.performance;
  const method = callee.Performance.prototype.measure;
  const mark = owner.Performance.prototype.mark.bind(performance);
  const entries = owner.Performance.prototype.getEntriesByType.bind(performance);
  const prefix = '__measureConversion:';
  const names = new Set();
  let next = 0;
  const measure = (...args) => {
    if (typeof args[0] === 'string') names.add(args[0]);
    return Reflect.apply(method, performance, args);
  };
  const name = () => prefix + next++;
  const assert = (passed, message) => { if (!passed) throw Error(message); };
  const equal = (actual, expected) => assert(JSON.stringify(actual) === JSON.stringify(expected),
    JSON.stringify({actual, expected}));
  const capture = callback => { try { callback(); } catch (error) { return error; } };
  const test = (label, callback) => {
    try { callback(); checks.push({name: label, passed: true}); }
    catch (error) { checks.push({name: label, passed: false, details: String(error.stack || error)}); }
  };
  const options = (trace, overrides = {}) => Object.defineProperties({}, {
    detail: {configurable: true, get() { trace.push('detail'); return undefined; }},
    duration: {configurable: true, get() { trace.push('duration'); return undefined; }},
    end: {configurable: true, get() { trace.push('end'); return overrides.end; }},
    start: {configurable: true, get() { trace.push('start'); return overrides.start; }},
  });
  const startName = prefix + 'start';
  const endName = prefix + 'end';
  const markNames = [startName, endName, 'null', 'true', '5', '0'];
  for (const [value, time] of [[startName, 2], [endName, 9], ['null', 777.125],
    ['true', 161], ['5', 162], ['0', 163]]) mark(value, {startTime: time});
  try {
    test('required name uses callee TypeError', () => {
      assert(capture(() => Reflect.apply(method, performance, [])) instanceof callee.TypeError, 'arity');
    });
    test('receiver validation precedes argument conversion', () => {
      let reads = 0;
      const error = capture(() => Reflect.apply(method, {}, [{toString() { reads++; return name(); }}]));
      assert(error instanceof callee.TypeError && reads === 0, 'receiver conversion order');
    });
    test('name conversion precedes dictionary reads', () => {
      const trace = []; const sentinel = {};
      const error = capture(() => measure({toString() { trace.push('name'); throw sentinel; }},
        options(trace)));
      assert(error === sentinel, 'name exception identity'); equal(trace, ['name']);
    });
    test('dictionary converts each member before reading the next', () => {
      const trace = [];
      const result = measure(name(), options(trace, {
        end: {[Symbol.toPrimitive](hint) { trace.push('convert-end:' + hint); return endName; }},
        start: {[Symbol.toPrimitive](hint) { trace.push('convert-start:' + hint); return startName; }},
      }));
      equal(trace, ['detail', 'duration', 'end', 'convert-end:string', 'start', 'convert-start:string']);
      assert(result.startTime === 2 && result.duration === 7, 'measure values');
    });
    test('end conversion exception precedes start getter and third argument', () => {
      const trace = []; const endError = {}; const startError = {};
      const value = options(trace, {end: {toString() { trace.push('convert-end'); throw endError; }}});
      Object.defineProperty(value, 'start', {get() { trace.push('start'); throw startError; }});
      const error = capture(() => measure(name(), value, {toString() { trace.push('third'); return endName; }}));
      assert(error === endError, 'end exception identity');
      equal(trace, ['detail', 'duration', 'end', 'convert-end']);
    });
    for (const [label, end] of [['infinity', Infinity], ['NaN', NaN], ['symbol', Symbol()]]) {
      test(label + ' end conversion stops later getters in callee realm', () => {
        const trace = []; const sentinel = {};
        const value = options(trace, {end});
        Object.defineProperty(value, 'start', {get() { trace.push('start'); throw sentinel; }});
        const error = capture(() => measure(name(), value));
        assert(error instanceof callee.TypeError, 'callee TypeError');
        equal(trace, ['detail', 'duration', 'end']);
      });
    }
    test('duration conversion precedes end getter', () => {
      const trace = []; const sentinel = {};
      const value = options(trace);
      Object.defineProperty(value, 'duration', {get() {
        trace.push('duration'); return {valueOf() { trace.push('convert-duration'); throw sentinel; }};
      }});
      assert(capture(() => measure(name(), value)) === sentinel, 'duration identity');
      equal(trace, ['detail', 'duration', 'convert-duration']);
    });
    test('nonfinite duration stops end getter', () => {
      const trace = []; const value = options(trace);
      Object.defineProperty(value, 'duration', {get() { trace.push('duration'); return Infinity; }});
      assert(capture(() => measure(name(), value)) instanceof callee.TypeError, 'duration type');
      equal(trace, ['detail', 'duration']);
    });
    test('start conversion exception precedes legacy argument', () => {
      const trace = []; const sentinel = {};
      const value = options(trace, {end: 9, start: {toString() { trace.push('convert-start'); throw sentinel; }}});
      assert(capture(() => measure(name(), value, {toString() { trace.push('third'); return endName; }})) === sentinel, 'start identity');
      equal(trace, ['detail', 'duration', 'end', 'start', 'convert-start']);
    });
    test('end conversion can update the later start member', () => {
      const value = {start: 2, end: {toString() { value.start = 4; return endName; }}};
      const result = measure(name(), value);
      assert(result.startTime === 4 && result.duration === 5, 'updated start');
    });
    test('end conversion can remove the later start member', () => {
      const value = {start: 2, end: {toString() { delete value.start; return endName; }}};
      const result = measure(name(), value);
      assert(result.startTime === 0 && result.duration === 9, 'removed start');
    });
    test('end conversion cannot change an already converted duration', () => {
      let conversions = 0;
      const value = {duration: {valueOf() { conversions++; return 1; }},
        end: {toString() { value.duration = 3; return endName; }}};
      const result = measure(name(), value);
      assert(result.startTime === 8 && result.duration === 1 && conversions === 1, 'stored duration');
    });
    test('third argument conversion follows the dictionary conversions', () => {
      const trace = [];
      const value = options(trace, {
        end: {toString() { trace.push('convert-end'); return endName; }},
        start: {toString() { trace.push('convert-start'); return startName; }},
      });
      const error = capture(() => measure(name(), value, {toString() { trace.push('third'); return endName; }}));
      assert(error instanceof callee.TypeError, 'nonempty options and third argument');
      equal(trace, ['detail', 'duration', 'end', 'convert-end', 'start', 'convert-start', 'third']);
    });
    for (const [label, second] of [['empty', {}], ['null', null], ['undefined', undefined]]) {
      test(label + ' second argument preserves a null legacy end mark', () => {
        const result = measure(name(), second, null);
        assert(result.startTime === 0 && result.duration === 777.125, 'null is a DOMString mark');
      });
    }
    test('null legacy end counts as given with nonempty options', () => {
      assert(capture(() => measure(name(), {start: 0}, null)) instanceof callee.TypeError, 'third argument present');
    });
    test('third argument exception precedes semantic validation', () => {
      const sentinel = {};
      assert(capture(() => measure(name(), {detail: {}}, {toString() { throw sentinel; }})) === sentinel, 'third exception');
    });
    test('third argument exception precedes detail serialization', () => {
      const sentinel = {};
      assert(capture(() => measure(name(), {start: 0, detail: () => {}}, {toString() { throw sentinel; }})) === sentinel, 'third before clone');
    });
    test('object numeric primitive is converted to a mark string', () => {
      const trace = [];
      const result = measure(name(), {start: 0, end: {[Symbol.toPrimitive](hint) { trace.push(hint); return 5; }}});
      assert(result.duration === 162, 'object selects DOMString'); equal(trace, ['string']);
    });
    test('boxed Number is converted as a string rather than a timestamp', () => {
      const value = new Number(5); let valueOfCalls = 0;
      value.valueOf = () => { valueOfCalls++; throw Error('number hint'); };
      const result = measure(name(), {start: 0, end: value});
      assert(result.duration === 162 && valueOfCalls === 0, 'boxed number');
    });
    for (const [label, end, expected] of [['boolean', true, 161], ['bigint', 5n, 162], ['null', null, 777.125], ['number', 5, 5]]) {
      test(label + ' end preserves union selection', () => {
        assert(measure(name(), {start: 0, end}).duration === expected, 'union result');
      });
    }
    test('undefined end stays omitted', () => {
      const result = measure(name(), {start: 2, duration: 1, end: undefined});
      assert(result.startTime === 2 && result.duration === 1, 'undefined end');
    });
    test('invalid primitive result throws before start getter', () => {
      const trace = []; const value = options(trace, {end: {[Symbol.toPrimitive]() { trace.push('convert-end'); return {}; }}});
      Object.defineProperty(value, 'start', {get() { trace.push('start'); throw {}; }});
      assert(capture(() => measure(name(), value)) instanceof callee.TypeError, 'primitive result error');
      equal(trace, ['detail', 'duration', 'end', 'convert-end']);
    });
    test('inherited dictionary members retain conversion order', () => {
      const trace = [];
      const prototype = options(trace, {end: {toString() { trace.push('convert-end'); return endName; }}, start: 2});
      const result = measure(name(), Object.create(prototype));
      assert(result.startTime === 2 && result.duration === 7, 'inherited values');
      equal(trace, ['detail', 'duration', 'end', 'convert-end', 'start']);
    });
    test('proxy dictionary reads preserve conversion boundaries', () => {
      const trace = [];
      const target = {end: {toString() { trace.push('convert-end'); return endName; }}, start: 2};
      const result = measure(name(), new Proxy(target, {get(object, key, receiver) {
        trace.push(key); return Reflect.get(object, key, receiver);
      }}));
      assert(result.startTime === 2 && result.duration === 7, 'proxy values');
      equal(trace, ['detail', 'duration', 'end', 'convert-end', 'start']);
    });
    test('conversion failure does not enqueue a measure', () => {
      const before = entries('measure').length;
      capture(() => measure(name(), {end: Symbol()}));
      assert(entries('measure').length === before, 'failed measure queued');
    });
  } finally {
    for (const value of names) owner.Performance.prototype.clearMeasures.call(performance, value);
    for (const value of markNames) owner.Performance.prototype.clearMarks.call(performance, value);
  }
  return checks;
}
