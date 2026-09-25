globalThis.windowScrollCase = (() => {
  const frame = document.createElement('iframe');
  document.body.append(frame);
  const child = frame.contentWindow;
  const popup = open('', 'window-scroll-receiver');
  const windows = {root: window, child, popup};
  for (const w of Object.values(windows)) {
    w.document.body.style.cssText = 'margin:0;width:10000px;height:10000px';
  }
  return {
    ready() {
      return Object.values(windows).every(w => w.document.readyState === 'complete');
    },
    async run() {
      let checks = 0;
      const failures = [];
      const equal = (actual, expected, label) => {
        ++checks;
        if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
      };
      const position = w => [w.scrollX, w.scrollY];
      const caught = async fn => { try { await fn(); return null; } catch (error) { return error; } };
      for (const method of ['scroll', 'scrollTo', 'scrollBy']) {
        for (const [calleeName, callee] of Object.entries(windows)) {
          const fn = callee[method];
          for (const [receiverName, receiver] of Object.entries(windows)) {
            const label = method + '/' + calleeName + '/' + receiverName;
            for (const w of Object.values(windows)) w.scrollTo(11, 19);
            const promise = fn.call(receiver, 23, 31);
            // Lightweight popups currently share their creator's V8 realm.
            const promiseConstructor = receiver.Promise || window.Promise;
            equal(!!promise && Object.getPrototypeOf(promise) === promiseConstructor.prototype, true, label + ' Promise realm');
            const result = await promise;
            equal(result, undefined, label + ' fulfilled scroll result');
            for (const [name, w] of Object.entries(windows)) {
              equal(position(w), name === receiverName
                ? (method === 'scrollBy' ? [34, 50] : [23, 31]) : [11, 19], label + ' targets ' + name);
            }
          }
        }
        // Conversion errors belong to the function's realm, even with a
        // valid receiver from another realm. No scroll may precede conversion.
        for (const [calleeName, callee] of [['root', window], ['child', child]]) {
          const fn = callee[method];
          const label = method + '/' + calleeName;
          const call = (...args) => fn.call(child, ...args);
          const reset = () => child.scrollTo(11, 19);
          const expected = (x, y) => method === 'scrollBy' ? [11 + x, 19 + y] : [x, y];
          for (const args of [[], [undefined], [null], [{}]]) {
            reset();
            call(...args);
            equal(position(child), [11, 19], label + ' empty dictionary preserves position');
          }
          for (const value of [1, true, 'x', Symbol('x'), 1n]) {
            reset();
            const error = await caught(() => call(value));
            equal(error instanceof callee.TypeError, true, label + ' primitive dictionary error');
            equal(position(child), [11, 19], label + ' primitive dictionary has no effect');
          }
          let order = [];
          const options = {
            get behavior() { order.push('behavior'); return {toString() { order.push('behavior string'); return 'instant'; }}; },
            get left() { order.push('left'); return {valueOf() { order.push('left number'); return 23; }}; },
            get top() { order.push('top'); return {valueOf() { order.push('top number'); return 31; }}; }
          };
          reset();
          call(options);
          equal(order, ['behavior', 'behavior string', 'left', 'left number', 'top', 'top number'], label + ' dictionary order');
          equal(position(child), expected(23, 31), label + ' dictionary values');
          const functionOptions = Object.assign(function() { throw new Error('must not call options'); }, {left: 23, top: 31});
          reset();
          call(functionOptions);
          equal(position(child), expected(23, 31), label + ' callable dictionary');
          for (const behavior of ['', 'invalid', null, 5]) {
            reset();
            order = [];
            const error = await caught(() => call({behavior, get left() { order.push('left'); return 23; }}));
            equal([error instanceof callee.TypeError, order], [true, []], label + ' enum fails before coordinates');
            equal(position(child), [11, 19], label + ' invalid enum has no effect');
          }
          for (const name of ['behavior', 'left', 'top']) {
            reset();
            order = [];
            const sentinel = {};
            const error = await caught(() => call(new Proxy({}, {get(_target, key) {
              order.push(key);
              if (key === name) throw sentinel;
              return key === 'behavior' ? 'instant' : 23;
            }})));
            equal(error === sentinel, true, label + ' propagates ' + name + ' getter');
            equal(order, ['behavior', 'left', 'top'].slice(0, ['behavior', 'left', 'top'].indexOf(name) + 1), label + ' aborts dictionary conversion');
            equal(position(child), [11, 19], label + ' throwing dictionary has no effect');
          }
          for (const value of [NaN, Infinity, -Infinity]) {
            reset();
            call(value, value);
            equal(position(child), expected(0, 0), label + ' normalizes numeric non-finite');
            reset();
            call({left: value, top: value});
            equal(position(child), expected(0, 0), label + ' normalizes dictionary non-finite');
          }
          reset();
          call({left: undefined, top: null});
          equal(position(child), method === 'scrollBy' ? [11, 19] : [11, 0], label + ' missing vs null');
          reset();
          order = [];
          call({get left() { throw new Error('numeric overload must not read dictionary'); }, valueOf() { order.push('x'); return 23; }},
            {valueOf() { order.push('y'); return 31; }}, {valueOf() { throw new Error('ignored extra argument'); }});
          equal(order, ['x', 'y'], label + ' numeric overload order');
          equal(position(child), expected(23, 31), label + ' numeric overload ignores dictionary');
          for (const index of [0, 1]) {
            reset();
            order = [];
            const sentinel = {};
            const args = [0, 1].map(i => ({valueOf() { order.push(i); if (index === i) throw sentinel; return 23; }}));
            equal(await caught(() => call(...args)) === sentinel, true, label + ' propagates numeric conversion');
            equal(order, index === 0 ? [0] : [0, 1], label + ' aborts numeric conversion');
            equal(position(child), [11, 19], label + ' throwing numbers have no effect');
          }
          reset();
          call({get left() { child.scrollTo(61, 67); return 23; }});
          equal(position(child), method === 'scrollBy' ? [84, 67] : [23, 67], label + ' reads position after conversion');
          for (const receiver of [null, undefined]) {
            callee.scrollTo(11, 19);
            const promise = fn.call(receiver, 23, 31);
            equal(!!promise && Object.getPrototypeOf(promise) === callee.Promise.prototype, true, label + ' Promise realm');
            const result = await promise;
            equal(result, undefined, label + ' fulfilled scroll result');
            equal(position(callee), expected(23, 31), label + ' nullish receiver uses function global');
          }
        }
        for (const numeric of [false, true]) {
          const removedFrame = document.createElement('iframe');
          document.body.append(removedFrame);
          const removed = removedFrame.contentWindow;
          let conversions = 0;
          let callerAfterRemoval;
          window.scrollTo(11, 19);
          const input = {valueOf() {
            ++conversions;
            removedFrame.remove();
            // Tree removal can itself clamp the caller's viewport. Only the
            // outer scroll operation must leave this resulting state alone.
            callerAfterRemoval = position(window);
            return 23;
          }};
          const args = numeric ? [input, {valueOf() { ++conversions; return 31; }}]
            : [{left: input, get top() { ++conversions; return 31; }}];
          equal(await caught(() => window[method].apply(removed, args)), null, method + ' removed receiver remains valid');
          equal(conversions, 2, method + ' completes conversion after removal');
          equal(position(window), callerAfterRemoval, method + ' removed receiver does not scroll caller');
        }
      }
      return {checks, failures};
    },
    close() {
      frame.remove();
      popup.close();
    }
  };
})();
