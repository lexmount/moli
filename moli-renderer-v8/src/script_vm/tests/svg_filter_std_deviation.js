(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const invoke = (method, receiver, args) => {
    if (typeof method !== 'function') throw new Error('setStdDeviation is missing');
    return Reflect.apply(method, receiver, args);
  };
  const typeError = (method, receiver, args, realm) => {
    if (typeof method !== 'function') throw new Error('setStdDeviation is missing');
    try { Reflect.apply(method, receiver, args); }
    catch (error) { return error instanceof realm.TypeError; }
    return false;
  };
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.querySelector('iframe').contentWindow;
  const definitions = [['feGaussianBlur', 'SVGFEGaussianBlurElement', 0], ['feDropShadow', 'SVGFEDropShadowElement', 2]];
  const values = [
    ['ordinary', 2.5, 3.25], ['equal', 2, 2], ['zero', 0, 0], ['negative', -1, -2],
    ['negative-zero', -0, -0], ['rounded', 0.1, 0.2], ['large-integer', 16777217, 16777219],
    ['float-limits', Math.fround(3.4028234663852886e38), Math.fround(1e-45)],
    ['underflow', Number.MIN_VALUE, -Number.MIN_VALUE], ['coercion', '2.5', '3.25'],
    ['null-bool', null, true], ['empty-false', '', false],
  ];
  for (const [world, realm] of [['main', globalThis], ['child', child]]) {
    for (const [documentKind, doc] of [
      ['live', realm.document], ['windowless-html', realm.document.implementation.createHTMLDocument('')],
      ['windowless-xml', new realm.DOMParser().parseFromString('<root/>', 'application/xml')],
    ]) {
      for (const [tag, name, initial] of definitions) {
        const prefix = `${world}/${documentKind}/${tag}`;
        const proto = realm[name].prototype;
        const method = proto.setStdDeviation;
        const make = () => doc.createElementNS(ns, tag);
        const fresh = make();
        check(prefix + '/own-method', () => Object.hasOwn(proto, 'setStdDeviation') && typeof method === 'function');
        check(prefix + '/descriptor', () => {
          const d = Object.getOwnPropertyDescriptor(proto, 'setStdDeviation');
          return d.writable && d.enumerable && d.configurable && d.value === method;
        });
        check(prefix + '/function-metadata', () => method.name === 'setStdDeviation' && method.length === 2);
        check(prefix + '/function-realm', () => Object.getPrototypeOf(method) === realm.Function.prototype);
        check(prefix + '/nonconstructor', () => {
          if (typeof method !== 'function') return false;
          try { new method(1, 2); } catch (error) { return error instanceof TypeError; }
          return false;
        });
        check(prefix + '/defaults', () => fresh.stdDeviationX.baseVal === initial && fresh.stdDeviationY.baseVal === initial);
        for (const [label, first, second] of values) {
          const element = make(), x = element.stdDeviationX, y = element.stdDeviationY;
          check(prefix + '/value/' + label, () => {
            const result = invoke(method, element, [first, second]);
            return result === undefined && x === element.stdDeviationX && y === element.stdDeviationY &&
              [x.baseVal, x.animVal].every(v => Object.is(v, Math.fround(Number(first)))) &&
              [y.baseVal, y.animVal].every(v => Object.is(v, Math.fround(Number(second))));
          });
          check(prefix + '/attribute/' + label, () => {
            if (typeof method !== 'function') return false;
            const raw = element.getAttribute('stdDeviation');
            if (raw === null) return false;
            const pair = raw.trim().split(/[ ,]+/).map(Number);
            return pair.length === 2 && Object.is(Math.fround(pair[0]), Math.fround(Number(first))) &&
              Object.is(Math.fround(pair[1]), Math.fround(Number(second)));
          });
        }
        check(prefix + '/reflection-after-attribute-write', () => {
          const element = make(), x = element.stdDeviationX, y = element.stdDeviationY;
          invoke(method, element, [2.5, 3.25]);
          element.setAttribute('stdDeviation', '6');
          return x.baseVal === 6 && y.baseVal === 6 && x.animVal === 6 && y.animVal === 6;
        });
        check(prefix + '/reflection-after-removal', () => {
          const element = make(), x = element.stdDeviationX, y = element.stdDeviationY;
          invoke(method, element, [2.5, 3.25]); element.removeAttribute('stdDeviation');
          return x.baseVal === initial && y.baseVal === initial && x.animVal === initial && y.animVal === initial;
        });
        check(prefix + '/reflection-after-base-setter', () => {
          const element = make(), x = element.stdDeviationX, y = element.stdDeviationY;
          invoke(method, element, [2.5, 3.25]); x.baseVal = 7;
          return x.baseVal === 7 && x.animVal === 7 && y.baseVal === 3.25 && y.animVal === 3.25;
        });
        check(prefix + '/single-native-mutation', () => {
          const element = make(); element.setAttribute('stdDeviation', '8 9');
          const observer = new realm.MutationObserver(() => {});
          observer.observe(element, {attributes: true, attributeOldValue: true});
          try {
            invoke(method, element, [2.5, 3.25]);
            const records = observer.takeRecords();
            return records.length === 1 && records[0].type === 'attributes' && records[0].target === element &&
              records[0].attributeName === 'stdDeviation' && records[0].attributeNamespace === null && records[0].oldValue === '8 9';
          } finally { observer.disconnect(); }
        });
        check(prefix + '/bypass-author-methods-and-reflection-properties', () => {
          const element = make(); let calls = 0;
          element.setAttribute = () => { calls++; throw new Error('author setAttribute'); };
          Object.defineProperty(element, 'stdDeviationX', {get() { calls++; throw new Error('author x'); }});
          Object.defineProperty(element, 'stdDeviationY', {get() { calls++; throw new Error('author y'); }});
          const getX = Object.getOwnPropertyDescriptor(proto, 'stdDeviationX').get;
          const getY = Object.getOwnPropertyDescriptor(proto, 'stdDeviationY').get;
          invoke(method, element, [2.5, 3.25]);
          return calls === 0 && getX.call(element).baseVal === 2.5 && getY.call(element).baseVal === 3.25;
        });
        const invalids = [NaN, Infinity, -Infinity, 1e39, -1e39, undefined, Symbol('number'), 1n];
        for (const [index, invalid] of invalids.entries()) {
          for (const position of [0, 1]) {
            check(prefix + `/invalid/${index}/${position}`, () => {
              const element = make(); element.setAttribute('stdDeviation', '8 9');
              const observer = new realm.MutationObserver(() => {});
              observer.observe(element, {attributes: true});
              try {
                const args = position === 0 ? [invalid, 4] : [3, invalid];
                return typeError(method, element, args, realm) && element.getAttribute('stdDeviation') === '8 9' &&
                  element.stdDeviationX.baseVal === 8 && element.stdDeviationY.baseVal === 9 && observer.takeRecords().length === 0;
              } finally { observer.disconnect(); }
            });
          }
        }
        for (const count of [0, 1]) {
          check(prefix + '/arity/' + count, () => {
            const element = make(); let conversions = 0;
            const value = {valueOf() { conversions++; return 3; }};
            return typeError(method, element, count ? [value] : [], realm) && conversions === 0 && element.getAttribute('stdDeviation') === null;
          });
        }
        check(prefix + '/left-to-right-and-no-partial-write', () => {
          const element = make(); element.setAttribute('stdDeviation', '8 9');
          const order = [];
          const first = {[Symbol.toPrimitive](hint) { order.push('first:' + hint); return 3; }};
          const second = {[Symbol.toPrimitive](hint) { order.push('second:' + hint); return Infinity; }};
          return typeError(method, element, [first, second], realm) && order.join(',') === 'first:number,second:number' &&
            element.getAttribute('stdDeviation') === '8 9';
        });
        check(prefix + '/first-conversion-failure-stops-second', () => {
          const element = make(); let second = 0;
          const value = {valueOf() { second++; return 3; }};
          return typeError(method, element, [Infinity, value], realm) && second === 0;
        });
        for (const position of [0, 1]) {
          check(prefix + '/original-exception/' + position, () => {
            const element = make(); element.setAttribute('stdDeviation', '8 9'); const marker = {};
            const value = {valueOf() { throw marker; }};
            try { invoke(method, element, position ? [3, value] : [value, 4]); }
            catch (error) { return error === marker && element.getAttribute('stdDeviation') === '8 9'; }
            return false;
          });
        }
        check(prefix + '/reentrant-conversion-then-single-final-write', () => {
          const element = make(); const order = [];
          const first = {valueOf() { element.setAttribute('stdDeviation', '10 11'); order.push('first'); return 2.5; }};
          const second = {valueOf() { order.push(element.stdDeviationX.baseVal + ':' + element.stdDeviationY.baseVal); return 3.25; }};
          invoke(method, element, [first, second]);
          return order.join(',') === 'first,10:11' && element.stdDeviationX.baseVal === 2.5 && element.stdDeviationY.baseVal === 3.25;
        });
        check(prefix + '/ignore-extra-argument', () => {
          const element = make(); let conversions = 0;
          invoke(method, element, [2.5, 3.25, {valueOf() { conversions++; throw new Error('extra'); }}]);
          return conversions === 0 && element.stdDeviationX.baseVal === 2.5 && element.stdDeviationY.baseVal === 3.25;
        });
        const real = make();
        const revoked = Proxy.revocable(real, {}); revoked.revoke();
        let traps = 0;
        const proxy = new Proxy(real, {get() { traps++; throw new Error('get'); }, getPrototypeOf() { traps++; throw new Error('prototype'); }});
        const wrong = definitions.find(d => d[0] !== tag)[0];
        const badReceivers = [
          ['plain', {}], ['prototype', Object.create(proto)], ['inherit-real', Object.create(real)],
          ['author-proxy', proxy], ['revoked-proxy', revoked.proxy], ['wrong-filter', doc.createElementNS(ns, wrong)],
          ['html-namespace', doc.createElementNS('http://www.w3.org/1999/xhtml', tag)],
        ];
        for (const [label, receiver] of badReceivers) {
          check(prefix + '/receiver/' + label, () => {
            let conversions = 0; const value = {valueOf() { conversions++; return 3; }};
            const result = typeError(method, receiver, [value, value], realm);
            return result && conversions === 0 && traps === 0;
          });
        }
        check(prefix + '/borrowed-foreign-method', () => {
          const other = realm === globalThis ? child : globalThis;
          const foreign = other[name].prototype.setStdDeviation;
          invoke(foreign, real, [2.5, 3.25]);
          return real.stdDeviationX.baseVal === 2.5 && real.stdDeviationY.baseVal === 3.25 &&
            typeError(foreign, {}, [1, 2], other) && typeError(foreign, real, [1, Infinity], other);
        });
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
