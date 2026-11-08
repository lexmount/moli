(() => {
  const realms = [globalThis, document.getElementById('child').contentWindow];
  const checks = [];
  const group = globalThis.__mutationObserverGroup || 'all';
  const facts = globalThis.__uiEventResults = {complete: false, group, checks};
  const assert = (ok, message) => {if (!ok) throw Error(message);};
  const equal = (a, b, message) => assert(JSON.stringify(a) === JSON.stringify(b),
    message + ': ' + JSON.stringify(a) + ' != ' + JSON.stringify(b));
  function test(name, body) {
    globalThis.__mutationObserverCurrentCase = name;
    try {body(); checks.push({name, passed: true});}
    catch (error) {checks.push({name, passed: false, detail: String(error)});}
  }
  for (const [ownerIndex, owner] of realms.entries()) {
    for (const [calleeIndex, callee] of realms.entries()) {
      const prefix = ownerIndex + ':' + calleeIndex + ':';
      const method = callee.MutationObserver.prototype.observe;
      function withObserver(body, target) {
        const observer = new owner.MutationObserver(() => {});
        target = target || owner.document.createElement('div');
        const observe = options => Reflect.apply(method, observer, [target, options]);
        try {body(observer, target, observe);} finally {observer.disconnect();}
      }
      const names = observer => observer.takeRecords().map(record => record.attributeName);
      const typeError = body => {
        let error;
        try {body();} catch (caught) {error = caught;}
        assert(error instanceof callee.TypeError, 'callee TypeError');
      };
      if (group !== 'filters') {
        test(prefix + 'observe.length', () => assert(method.length === 1, 'one required argument'));
        for (const [label, make] of [
          ['Set', () => new Set(['keep'])],
          ['generator', () => (function*() {yield 'keep';})()],
          ['array custom iterator', () => Object.assign(['drop'], {[Symbol.iterator]: function*() {yield 'keep';}})],
        ]) {
          test(prefix + 'iterable ' + label, () => withObserver((observer, target, observe) => {
            observe({attributes: true, attributeFilter: make()});
            target.setAttribute('drop', 'x'); target.setAttribute('keep', 'y');
            equal(names(observer), ['keep'], 'converted filter');
          }));
        }
        test(prefix + 'empty iterable remains an empty filter', () => withObserver((observer, target, observe) => {
          observe({attributeFilter: new Set()}); target.setAttribute('drop', 'x');
          equal(names(observer), [], 'nothing matches an empty filter');
        }));
        for (const [label, value] of [['null', null], ['string', 'keep'], ['number', 1], ['boolean', false],
          ['plain object', {}], ['array-like', {0: 'keep', length: 1}], ['non-callable iterator', {[Symbol.iterator]: 1}]]) {
          test(prefix + 'reject non-sequence ' + label, () => withObserver((observer, target, observe) => {
            typeError(() => observe({attributes: true, attributeFilter: value}));
            target.setAttribute('keep', 'x'); equal(names(observer), [], 'failure does not register');
          }));
        }
        for (const oldValue of [false, null]) {
          test(prefix + 'attributeOldValue present ' + oldValue, () => withObserver((observer, target, observe) => {
            observe({attributeOldValue: oldValue}); target.setAttribute('keep', 'x');
            const records = observer.takeRecords();
            assert(records.length === 1 && records[0].oldValue === null, 'presence enables attributes');
          }));
          test(prefix + 'characterDataOldValue present ' + oldValue, () => {
            const target = owner.document.createTextNode('before');
            withObserver((observer, node, observe) => {
              observe({characterDataOldValue: oldValue}); node.data = 'after';
              const records = observer.takeRecords();
              assert(records.length === 1 && records[0].type === 'characterData' && records[0].oldValue === null,
                'presence enables characterData');
            }, target);
          });
        }
        test(prefix + 'undefined flags are absent', () => withObserver((observer, target, observe) => {
          target.setAttribute('keep', 'before');
          observe({attributes: undefined, attributeOldValue: true});
          target.setAttribute('keep', 'after');
          const records = observer.takeRecords();
          assert(records.length === 1 && records[0].oldValue === 'before', 'undefined allows implicit attributes');
        }));
        const keys = ['attributeFilter', 'attributeOldValue', 'attributes', 'characterData',
          'characterDataOldValue', 'childList', 'subtree'];
        test(prefix + 'dictionary Get order without HasProperty', () => withObserver((observer, target, observe) => {
          const log = [], options = new Proxy({attributeFilter: ['keep'], attributes: true}, {
            get(object, key, receiver) {log.push(key); return Reflect.get(object, key, receiver);},
            has() {throw Error('unexpected HasProperty');},
          });
          observe(options); equal(log, keys, 'lexical dictionary order');
        }));
        test(prefix + 'inherited members are read once in lexical order', () => withObserver((observer, target, observe) => {
          const log = [], prototype = {};
          for (const key of keys) Object.defineProperty(prototype, key, {get() {
            log.push(key); return key === 'attributes' ? true : undefined;
          }});
          observe(Object.create(prototype)); equal(log, keys, 'inherited getter order');
        }));
        for (const failKey of keys) {
          test(prefix + 'getter abrupt completion at ' + failKey, () => withObserver((observer, target, observe) => {
            const sentinel = {}, log = [], options = {};
            for (const key of keys) Object.defineProperty(options, key, {get() {
              log.push(key); if (key === failKey) throw sentinel;
              return key === 'attributeFilter' ? ['keep'] : key === 'attributes' ? true : undefined;
            }});
            let error; try {observe(options);} catch (caught) {error = caught;}
            assert(error === sentinel, 'original thrown value');
            equal(log, keys.slice(0, keys.indexOf(failKey) + 1), 'stop at failed member');
            target.setAttribute('keep', 'after'); equal(names(observer), [], 'no partial registration');
          }));
        }
        for (const failure of ['iterator', 'item']) {
          test(prefix + 'failed reconfiguration preserves registration: ' + failure, () => withObserver((observer, target, observe) => {
            target.setAttribute('keep', 'before');
            observe({attributes: true, attributeOldValue: true, attributeFilter: ['keep']});
            const sentinel = {}, filter = failure === 'iterator' ? {
              *[Symbol.iterator]() {yield 'changed'; throw sentinel;}
            } : [{toString() {throw sentinel;}}];
            let error, laterGets = 0;
            try {observe({attributeFilter: filter, get attributes() {laterGets++; return true;}});}
            catch (caught) {error = caught;}
            assert(error === sentinel && laterGets === 0, 'conversion fails before later members');
            target.setAttribute('changed', 'x'); target.setAttribute('keep', 'after');
            const records = observer.takeRecords();
            assert(records.length === 1 && records[0].attributeName === 'keep' && records[0].oldValue === 'before',
              'old registration is intact');
          }));
        }
        test(prefix + 'iteration and DOMString conversion happen at member position', () => withObserver((observer, target, observe) => {
          const log = [], iterable = {
            get [Symbol.iterator]() {log.push('iterator'); return new Proxy(function*() {
              log.push('iterate'); yield {toString() {log.push('string'); return 'keep';}};
            }, {});}
          };
          observe({get attributeFilter() {log.push('filter'); return iterable;},
            get attributeOldValue() {log.push('oldValue'); return false;},
            get attributes() {log.push('attributes'); return true;}});
          equal(log, ['filter', 'iterator', 'iterate', 'string', 'oldValue', 'attributes'], 'conversion position');
          target.setAttribute('drop', 'x'); target.setAttribute('keep', 'after');
          equal(names(observer), ['keep'], 'resulting filter');
        }));
        test(prefix + 'invalid target precedes options conversion', () => withObserver((observer, target) => {
          const revoked = Proxy.revocable(target, {}); revoked.revoke();
          for (const invalid of [{}, Object.create(target), new Proxy(target, {}), revoked.proxy, null]) {
            let gets = 0;
            typeError(() => Reflect.apply(method, observer, [invalid, {get childList() {gets++; return true;}}]));
            assert(gets === 0, 'invalid target stops dictionary getters');
          }
        }));
        test(prefix + 'invalid receiver precedes argument conversion', () => withObserver((observer, target) => {
          for (const invalid of [{}, Object.create(observer), new Proxy(observer, {})]) {
            let gets = 0;
            typeError(() => Reflect.apply(method, invalid, [target, {get childList() {gets++; return true;}}]));
            assert(gets === 0, 'receiver validation comes first');
          }
        }));
        test(prefix + 'explicit false stays false', () => withObserver((observer, target, observe) => {
          observe({childList: true, attributes: false, attributeOldValue: false});
          target.setAttribute('keep', 'x'); target.appendChild(owner.document.createTextNode('text'));
          const records = observer.takeRecords();
          assert(records.length === 1 && records[0].type === 'childList', 'no implicit attributes');
          typeError(() => observe({childList: true, attributes: false, attributeFilter: []}));
          typeError(() => observe({childList: true, characterData: false, characterDataOldValue: true}));
        }));
      }
      if (group !== 'conversion') {
        for (const documentKind of ['live', 'html', 'xml']) {
          const doc = documentKind === 'live' ? owner.document : documentKind === 'html' ?
            owner.document.implementation.createHTMLDocument('') :
            owner.document.implementation.createDocument('urn:fixture', 'root');
          test(prefix + documentKind + ':filter excludes non-null namespaces', () => withObserver((observer, target, observe) => {
            observe({attributes: true, attributeFilter: ['keep']});
            target.setAttributeNS('urn:fixture', 'p:keep', 'namespaced');
            target.setAttribute('keep', 'plain');
            const records = observer.takeRecords();
            assert(records.length === 1 && records[0].attributeName === 'keep' && records[0].attributeNamespace === null,
              'only unnamespaced local name matches');
          }, doc.createElement('item')));
          test(prefix + documentKind + ':no filter retains namespaced mutations', () => withObserver((observer, target, observe) => {
            observe({attributes: true}); target.setAttributeNS('urn:fixture', 'p:keep', 'value');
            const records = observer.takeRecords();
            assert(records.length === 1 && records[0].attributeNamespace === 'urn:fixture', 'unfiltered namespace');
          }, doc.createElement('item')));
          test(prefix + documentKind + ':filter preserves lone-surrogate identity', () => withObserver((observer, target, observe) => {
            observe({attributes: true, attributeFilter: [String.fromCharCode(0xd800)]});
            target.setAttribute('\ufffd', 'value'); equal(names(observer), [], 'no replacement-character alias');
          }, doc.createElement('item')));
          test(prefix + documentKind + ':filter preserves well-formed Unicode', () => withObserver((observer, target, observe) => {
            const name = String.fromCodePoint(0x10000);
            observe({attributeFilter: [name]}); target.setAttribute(name, 'value');
            equal(names(observer), [name], 'non-BMP local name');
          }, doc.createElement('item')));
        }
        test(prefix + 'XML attribute filters match case exactly', () => {
          const doc = owner.document.implementation.createDocument('urn:fixture', 'root');
          withObserver((observer, target, observe) => {
            observe({attributeFilter: ['lower']});
            target.setAttribute('LOWER', 'upper'); target.setAttribute('lower', 'lower');
            equal(names(observer), ['lower'], 'case-sensitive local names');
          }, doc.documentElement);
        });
      }
    }
  }
  facts.total = checks.length;
  facts.passed = checks.filter(check => check.passed).length;
  facts.complete = true;
  return facts.passed === facts.total;
})()
