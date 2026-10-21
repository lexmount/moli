(async () => {
  const selected = globalThis.__mutationRecordGroup || 'all';
  const realms = [globalThis, document.getElementById('child').contentWindow];
  const constructors = realms.map(realm => realm.MutationRecord);
  const prototypes = constructors.map(constructor => constructor.prototype);
  const keys = ['type', 'target', 'addedNodes', 'removedNodes', 'previousSibling',
    'nextSibling', 'attributeName', 'attributeNamespace', 'oldValue'];
  const checks = [];
  const facts = globalThis.__uiEventResults = {complete: false, selected, checks};
  const assert = (value, message) => {if (!value) throw Error(message);};
  const wants = group => selected === 'all' || selected === group ||
    (selected === 'snapshots' && (group === 'prototype' || group === 'records'));
  function test(group, name, body) {
    if (!wants(group)) return;
    globalThis.__mutationRecordCurrentCase = name;
    try {body(); checks.push({group, name, passed: true});}
    catch (error) {checks.push({group, name, passed: false, detail: String(error)});}
  }
  function getter(calleeIndex, key) {
    const descriptor = Object.getOwnPropertyDescriptor(prototypes[calleeIndex], key);
    assert(descriptor && typeof descriptor.get === 'function', key + ' prototype getter');
    return descriptor.get;
  }
  function typeError(callee, body) {
    let caught;
    try {body();} catch (error) {caught = error;}
    assert(caught instanceof callee.TypeError, 'callee TypeError');
  }
  function values(record, expected, calleeIndex, borrowed) {
    for (const key of keys) {
      const value = borrowed ? getter(calleeIndex, key).call(record) : record[key];
      if (key === 'addedNodes' || key === 'removedNodes') {
        assert(Object.prototype.toString.call(value) === '[object NodeList]', key + ' NodeList');
        assert(value.length === expected[key].length, key + ' snapshot length');
        assert(Array.from(value).every((node, index) => node === expected[key][index]), key + ' identities');
        if (borrowed) assert(value === getter(calleeIndex, key).call(record), key + ' SameObject');
      } else {
        assert(value === expected[key], key + ' initialized value');
      }
    }
  }
  function produce(owner, doc) {
    const host = doc.createElement('item');
    const left = doc.createTextNode('left'), right = doc.createTextNode('right');
    const text = doc.createTextNode('old text\udfff'), middle = doc.createElement('middle');
    host.append(left, right, text);
    host.setAttributeNS('urn:record', 'p:keep', 'old attr\ud800');
    const observer = new owner.MutationObserver(() => {});
    observer.observe(host, {attributes: true, attributeOldValue: true,
      characterData: true, characterDataOldValue: true, childList: true, subtree: true});
    host.setAttributeNS('urn:record', 'p:keep', 'after');
    text.data = 'after';
    host.insertBefore(middle, right);
    const records = observer.takeRecords();
    observer.disconnect();
    assert(records.length === 3, 'three mutation kinds');
    const empty = {addedNodes: [], removedNodes: [], previousSibling: null, nextSibling: null,
      attributeName: null, attributeNamespace: null, oldValue: null};
    const expected = [
      {...empty, type: 'attributes', target: host, attributeName: 'keep',
        attributeNamespace: 'urn:record', oldValue: 'old attr\ud800'},
      {...empty, type: 'characterData', target: text, oldValue: 'old text\udfff'},
      {...empty, type: 'childList', target: host, addedNodes: [middle], previousSibling: left, nextSibling: right},
    ];
    return {records, expected, host, text, middle};
  }
  for (const [calleeIndex, callee] of realms.entries()) {
    for (const key of keys) {
      test('prototype', calleeIndex + ':descriptor:' + key, () => {
        const descriptor = Object.getOwnPropertyDescriptor(prototypes[calleeIndex], key);
        const get = getter(calleeIndex, key);
        assert(descriptor.enumerable && descriptor.configurable && descriptor.set === undefined,
          'enumerable configurable getter without setter');
        assert(!('value' in descriptor) && !('writable' in descriptor), 'accessor descriptor');
        assert(get.length === 0 && get.name === 'get ' + key, 'getter function metadata');
        assert(Object.getPrototypeOf(get) === callee.Function.prototype, 'getter realm');
        assert(!Object.prototype.hasOwnProperty.call(get, 'prototype'), 'getter is not a constructor');
      });
    }
    test('prototype', calleeIndex + ':attribute order', () => {
      const actual = Object.getOwnPropertyNames(prototypes[calleeIndex]).filter(key => keys.includes(key));
      assert(JSON.stringify(actual) === JSON.stringify(keys), 'IDL attribute declaration order');
    });
    test('prototype', calleeIndex + ':illegal constructor', () => {
      typeError(callee, () => callee.Reflect.construct(constructors[calleeIndex], []));
    });
  }
  if (wants('records') || wants('brands')) {
    for (const [ownerIndex, owner] of realms.entries()) {
      for (const documentKind of ['live', 'html', 'xml']) {
        const doc = documentKind === 'live' ? owner.document : documentKind === 'html' ?
          owner.document.implementation.createHTMLDocument('') :
          owner.document.implementation.createDocument('urn:fixture', 'root');
        for (const [calleeIndex, callee] of realms.entries()) {
          const prefix = ownerIndex + ':' + documentKind + ':' + calleeIndex + ':';
          const {records, expected, host, text, middle} = produce(owner, doc);
          for (let kind = 0; kind < records.length; kind++) {
            const record = records[kind], label = expected[kind].type;
            test('records', prefix + label + ':values', () => values(record, expected[kind], calleeIndex, false));
            test('records', prefix + label + ':borrowed getters', () => values(record, expected[kind], calleeIndex, true));
            test('records', prefix + label + ':private state', () => {
              assert(Reflect.ownKeys(record).length === 0, 'no own payload properties');
              assert(Object.getPrototypeOf(record) === prototypes[ownerIndex], 'producer prototype');
              assert(Object.prototype.toString.call(record) === '[object MutationRecord]', 'native tag');
            });
          }
          for (const key of keys) {
            test('records', prefix + 'readonly:' + key, () => {
              const record = records[2], original = Object.getOwnPropertyDescriptor(record, key);
              const before = record[key];
              let conversions = 0;
              const replacement = {toString() {conversions++; return 'forged';}};
              try {
                assert(Reflect.set(record, key, replacement) === false, 'ordinary assignment fails');
                typeError(globalThis, () => { 'use strict'; record[key] = replacement; });
                assert(record[key] === before && conversions === 0, 'initialized state remains unchanged');
                assert(!Object.prototype.hasOwnProperty.call(record, key), 'assignment creates no own property');
              } finally {
                if (original) Object.defineProperty(record, key, original);
                else delete record[key];
              }
            });
            test('brands', prefix + 'receiver:' + key, () => {
              const record = records[0], get = getter(calleeIndex, key);
              let traps = 0;
              const author = new Proxy(record, {
                get() {traps++; throw Error('author get');},
                getPrototypeOf() {traps++; throw Error('author prototype');},
              });
              const revoked = Proxy.revocable(record, {}); revoked.revoke();
              const forged = Object.defineProperties(Object.create(prototypes[ownerIndex]),
                Object.fromEntries(Object.entries(expected[0]).map(([name, value]) =>
                  [name, {value, writable: true, configurable: true, enumerable: true}])));
              for (const invalid of [undefined, null, 0, 'text', {}, prototypes[ownerIndex],
                Object.create(record), forged, author, revoked.proxy, host]) {
                typeError(callee, () => get.call(invalid));
              }
              assert(traps === 0, 'native branding invokes no author traps');
            });
            test('brands', prefix + 'shadow:' + key, () => {
              const record = records[0], get = getter(calleeIndex, key);
              const value = get.call(record), original = Object.getOwnPropertyDescriptor(record, key);
              const shadow = {};
              try {
                Object.defineProperty(record, key, {value: shadow, configurable: true, enumerable: true});
                assert(record[key] === shadow && get.call(record) === value, 'own shadow cannot replace native state');
              } finally {
                if (original) Object.defineProperty(record, key, original);
                else delete record[key];
              }
              assert(record[key] === value, 'deleting shadow reveals native getter');
            });
          }
          test('brands', prefix + 'mutable prototype preserves native identity', () => {
            const record = records[0], prototype = Object.getPrototypeOf(record);
            try {
              Object.setPrototypeOf(record, null);
              values(record, expected[0], calleeIndex, true);
            } finally {Object.setPrototypeOf(record, prototype);}
          });
          test('records', prefix + 'JSON has no payload properties', () => {
            assert(records.every(record => JSON.stringify(record) === '{}'), 'native state is not JSON data');
          });
          test('records', prefix + 'native records cannot be structured cloned', () => {
            for (const record of records) {
              let error; try {owner.structuredClone(record);} catch (caught) {error = caught;}
              assert(error && error.name === 'DataCloneError', 'MutationRecord remains a platform object');
            }
          });
          test('records', prefix + 'expando names do not alias private state', () => {
            const record = records[0];
            try {
              record.extra = 42; record.__moliMutationRecordTarget = 'public';
              assert(record.extra === 42, 'extensible object');
              values(record, expected[0], calleeIndex, false);
            } finally {delete record.extra; delete record.__moliMutationRecordTarget;}
          });
          test('records', prefix + 'snapshots survive later DOM edits', () => {
            host.removeChild(middle); host.replaceChildren();
            host.setAttributeNS('urn:record', 'p:keep', 'later'); text.data = 'later';
            records.forEach((record, kind) => values(record, expected[kind], calleeIndex, false));
          });
          for (let kind = 0; kind < records.length; kind++) {
            test('records', prefix + expected[kind].type + ':frozen record', () => {
              const fresh = produce(owner, doc);
              const record = fresh.records[kind];
              Object.freeze(record);
              assert(Object.isFrozen(record), 'frozen instance');
              values(record, fresh.expected[kind], calleeIndex, true);
            });
          }
        }
      }
      test('records', ownerIndex + ':factory ignores replaced global constructor', () => {
        const descriptor = Object.getOwnPropertyDescriptor(owner, 'MutationRecord');
        let reads = 0;
        try {
          Object.defineProperty(owner, 'MutationRecord', {configurable: true, get() {
            reads++; throw Error('author MutationRecord constructor');
          }});
          const fresh = produce(owner, owner.document);
          assert(fresh.records.every(record => Object.getPrototypeOf(record) === prototypes[ownerIndex]),
            'intrinsic producer prototype');
          assert(reads === 0, 'factory uses registered native constructor');
        } finally {Object.defineProperty(owner, 'MutationRecord', descriptor);}
      });
    }
  }
  if (wants('delivery')) {
    const retained = [];
    for (const [ownerIndex, owner] of realms.entries()) {
      for (const [calleeIndex] of realms.entries()) {
        const prefix = ownerIndex + ':' + calleeIndex + ':delivery:';
        const host = owner.document.createElement('item');
        host.setAttribute('keep', 'before');
        await new Promise(resolve => {
          let observer;
          const receive = (receiver, records, self) => {
            const expected = {type: 'attributes', target: host, addedNodes: [], removedNodes: [],
              previousSibling: null, nextSibling: null, attributeName: 'keep', attributeNamespace: null, oldValue: 'before'};
            test('delivery', prefix + 'callback attributes', () => {
              assert(receiver === observer && self === observer && records.length === 1, 'callback arguments');
              values(records[0], expected, calleeIndex, true);
            });
            test('delivery', prefix + 'callback relevant realm prototype', () => {
              assert(Object.getPrototypeOf(records[0]) === prototypes[calleeIndex], 'callback record realm');
            });
            retained.push({prefix, record: records[0], expected, calleeIndex});
            observer.disconnect(); resolve();
          };
          const callback = realms[calleeIndex].Function('receive',
            'return function(records, observer) { receive(this, records, observer); };')(receive);
          observer = new owner.MutationObserver(callback);
          observer.observe(host, {attributes: true, attributeOldValue: true});
          host.setAttribute('keep', 'after');
        });
      }
    }
    document.getElementById('child').remove();
    for (const {prefix, record, expected, calleeIndex} of retained) {
      test('delivery', prefix + 'retained initialized values', () => values(record, expected, calleeIndex, true));
    }
  }
  facts.total = checks.length;
  facts.passed = checks.filter(check => check.passed).length;
  facts.byGroup = Object.fromEntries(['prototype', 'records', 'brands', 'delivery'].map(group =>
    [group, checks.filter(check => check.group === group).length]));
  facts.complete = true;
  return facts.passed === facts.total;
})()
