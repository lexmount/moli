(() => {
  const rows = [];
  const check = (name, body) => {
    try { rows.push({name, passed: body() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const typeError = (body, realm) => {
    try { body(); return false; }
    catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
  };
  const realms = [['top', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const entries = [['item', 1], ['getNamedItem', 1], ['getNamedItemNS', 2],
    ['setNamedItem', 1], ['setNamedItemNS', 1], ['removeNamedItem', 1], ['removeNamedItemNS', 2]];
  const setup = (owner, flavor = 'live') => {
    const doc = flavor === 'live' ? owner.document :
      flavor === 'html' ? owner.document.implementation.createHTMLDocument('attributes') :
      flavor === 'xml' ? owner.document.implementation.createDocument(null, 'root') :
      new owner.DOMParser().parseFromString('<root/>', 'application/xml');
    const root = flavor === 'xml' || flavor === 'parsed' ? doc.documentElement : doc.createElement('div');
    root.setAttribute('data-x', 'one');
    root.setAttributeNS('urn:map', 'p:y', 'two');
    return {doc, root, map: root.attributes};
  };
  for (const [calleeName, callee] of realms) {
    const prototype = callee.NamedNodeMap.prototype;
    const length = Object.getOwnPropertyDescriptor(prototype, 'length').get;
    const methods = Object.fromEntries(entries.map(([name]) => [name, prototype[name]]));
    check(`${calleeName} length prototype descriptor`, () => {
      const d = Object.getOwnPropertyDescriptor(prototype, 'length');
      return d.enumerable && d.configurable && d.get.name === 'get length' && d.get.length === 0 &&
        d.set === undefined && Object.getPrototypeOf(d.get) === callee.Function.prototype;
    });
    for (const [name, arity] of entries) {
      check(`${calleeName} ${name} prototype descriptor`, () => {
        const d = Object.getOwnPropertyDescriptor(prototype, name);
        return d.enumerable && d.configurable && d.writable && d.value.name === name &&
          d.value.length === arity && Object.getPrototypeOf(d.value) === callee.Function.prototype;
      });
    }
    for (const [ownerName, owner] of realms) {
      for (const flavor of ['live', 'html', 'xml', 'parsed']) {
        const label = `${calleeName} borrows ${ownerName} ${flavor}`;
        const {doc, root, map} = setup(owner, flavor);
        check(`${label} factory realm and SameObject`, () =>
          Object.getPrototypeOf(map) === owner.NamedNodeMap.prototype &&
          map instanceof owner.NamedNodeMap && root.attributes === map);
        check(`${label} borrowed getters and lookups retain Attr identity`, () =>
          length.call(map) === 2 && methods.item.call(map, 0) === root.getAttributeNode('data-x') &&
          methods.getNamedItem.call(map, 'data-x') === map[0] &&
          methods.getNamedItemNS.call(map, 'urn:map', 'y') === root.getAttributeNodeNS('urn:map', 'y'));
        check(`${label} live mutation retains cached Attr identity`, () => {
          const attr = methods.getNamedItem.call(map, 'data-x');
          root.setAttribute('data-x', 'updated'); root.setAttribute('data-late', 'later');
          return root.attributes === map && length.call(map) === 3 &&
            methods.getNamedItem.call(map, 'data-x') === attr && attr.value === 'updated' &&
            methods.getNamedItem.call(map, 'data-late').value === 'later';
        });
        check(`${label} set and remove preserve owner and return value`, () => {
          const sample = setup(owner, flavor), attr = sample.doc.createAttribute('data-new'); attr.value = 'new';
          return methods.setNamedItem.call(sample.map, attr) === null && attr.ownerElement === sample.root &&
            length.call(sample.map) === 3 && methods.getNamedItem.call(sample.map, 'data-new') === attr &&
            methods.removeNamedItem.call(sample.map, 'data-new') === attr && attr.ownerElement === null &&
            length.call(sample.map) === 2;
        });
        check(`${label} namespace mutation preserves owner and return value`, () => {
          const sample = setup(owner, flavor), attr = sample.doc.createAttributeNS('urn:new', 'n:later'); attr.value = 'new';
          return methods.setNamedItemNS.call(sample.map, attr) === null && attr.ownerElement === sample.root &&
            methods.getNamedItemNS.call(sample.map, 'urn:new', 'later') === attr &&
            methods.removeNamedItemNS.call(sample.map, 'urn:new', 'later') === attr &&
            attr.ownerElement === null && length.call(sample.map) === 2;
        });
        check(`${label} replacement returns the previous Attr`, () => {
          const sample = setup(owner, flavor), old = sample.root.getAttributeNode('data-x');
          const next = sample.doc.createAttribute('data-x'); next.value = 'replacement';
          return methods.setNamedItem.call(sample.map, next) === old && old.ownerElement === null &&
            next.ownerElement === sample.root && methods.getNamedItem.call(sample.map, 'data-x') === next &&
            length.call(sample.map) === 2;
        });
        check(`${label} prototype removal preserves native backing`, () => {
          const sample = setup(owner, flavor); Object.setPrototypeOf(sample.map, null);
          const attr = sample.doc.createAttribute('data-new');
          return length.call(sample.map) === 2 && methods.getNamedItem.call(sample.map, 'data-x') === sample.map[0] &&
            methods.setNamedItem.call(sample.map, attr) === null &&
            methods.removeNamedItem.call(sample.map, 'data-new') === attr;
        });
        check(`${label} author Proxy still forwards own indexed and named data`, () => {
          const sample = setup(owner, flavor), proxy = new Proxy(sample.map, {});
          return proxy[0] === sample.map[0] && proxy['data-x'] === sample.map['data-x'];
        });
        check(`${label} intrinsic iterator preserves Attr values`, () => {
          const sample = setup(owner, flavor), iterate = prototype[Symbol.iterator];
          const values = Array.from(iterate.call(sample.map));
          return iterate === callee.Array.prototype.values && values.length === 2 &&
            values[0] === methods.item.call(sample.map, 0) && values[1] === methods.item.call(sample.map, 1);
        });
      }
      const sample = setup(owner);
      let traps = 0, conversions = 0;
      const handler = {get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 1;}, has() {traps++; throw 1;}};
      const proxy = new Proxy(sample.map, handler);
      const revoked = Proxy.revocable(sample.map, handler); revoked.revoke();
      const invalid = [['null', null], ['undefined', undefined], ['plain', {}], ['number', 1],
        ['prototype', owner.NamedNodeMap.prototype], ['forged', Object.create(owner.NamedNodeMap.prototype)],
        ['inherited instance', Object.create(sample.map)], ['author Proxy', proxy], ['revoked Proxy', revoked.proxy],
        ['Node', sample.root], ['Attr', sample.root.getAttributeNode('data-x')],
        ['NodeIterator', owner.document.createNodeIterator(sample.root)], ['DOMTokenList', sample.root.classList]];
      const converted = {toString() {conversions++; throw 1;}, valueOf() {conversions++; throw 1;}};
      for (const [receiverName, receiver] of invalid) {
        check(`${calleeName} length rejects ${ownerName} ${receiverName} in callee realm`, () =>
          typeError(() => length.call(receiver), callee));
        for (const [name] of entries) {
          const attr = sample.doc.createAttribute('data-never');
          const args = name === 'item' ? [0] : name.endsWith('NS') ?
            name.startsWith('set') ? [attr] : ['urn:map', 'y'] : name.startsWith('set') ? [attr] : ['data-x'];
          check(`${calleeName} ${name} rejects ${ownerName} ${receiverName} with valid arguments`, () =>
            typeError(() => methods[name].call(receiver, ...args), callee));
          const poisoned = name.startsWith('set') ? [new Proxy(attr, handler)] :
            name.endsWith('NS') ? [converted, converted] : [converted];
          check(`${calleeName} ${name} rejects ${ownerName} ${receiverName} before conversion`, () =>
            typeError(() => methods[name].call(receiver, ...poisoned), callee) && conversions === 0 && traps === 0);
        }
      }
      check(`${calleeName} ${ownerName} receiver validation executes no author traps or conversions`, () =>
        traps === 0 && conversions === 0 && length.call(sample.map) === 2 && sample.root.getAttribute('data-x') === 'one');
      for (const [name, arity] of entries) {
        for (let supplied = 0; supplied < arity; supplied++) {
          check(`${calleeName} ${name} enforces ${ownerName} required arguments after ${supplied} supplied`, () =>
            typeError(() => methods[name].call(sample.map, ...Array(supplied).fill(null)), callee));
        }
      }
      const realAttr = sample.doc.createAttribute('data-invalid');
      const revokedAttr = Proxy.revocable(realAttr, handler); revokedAttr.revoke();
      const badAttributes = [['undefined', undefined], ['null', null], ['plain', {}], ['Node', sample.root],
        ['prototype', owner.Attr.prototype], ['forged', Object.create(owner.Attr.prototype)],
        ['inherited instance', Object.create(realAttr)], ['author Proxy', new Proxy(realAttr, handler)],
        ['revoked Proxy', revokedAttr.proxy]];
      for (const name of ['setNamedItem', 'setNamedItemNS']) {
        for (const [argumentName, argument] of badAttributes) {
          check(`${calleeName} ${name} rejects ${ownerName} ${argumentName} Attr argument before mutation`, () =>
            typeError(() => methods[name].call(sample.map, argument), callee) && traps === 0 &&
              length.call(sample.map) === 2 && sample.root.getAttribute('data-x') === 'one');
        }
      }
      check(`${calleeName} ${ownerName} valid argument conversion retains order`, () => {
        const trace = [];
        const namespace = {toString() {trace.push('namespace'); return 'urn:map';}};
        const localName = {toString() {trace.push('name'); return 'y';}};
        return methods.getNamedItemNS.call(sample.map, namespace, localName) === sample.root.getAttributeNodeNS('urn:map', 'y') &&
          trace.join(',') === 'namespace,name';
      });
    }
    check(`${calleeName} factory uses intrinsic identity after constructor replacement`, () => {
      const saved = Object.getOwnPropertyDescriptor(callee, 'NamedNodeMap');
      try {
        Object.defineProperty(callee, 'NamedNodeMap', {...saved, value: function FakeMap() { throw 1; }});
        const sample = setup(callee);
        return Object.getPrototypeOf(sample.map) === prototype && length.call(sample.map) === 2 &&
          methods.getNamedItem.call(sample.map, 'data-x') === sample.map[0];
      } finally { Object.defineProperty(callee, 'NamedNodeMap', saved); }
    });
  }
  if (new Set(rows.map(row => row.name)).size !== rows.length) throw new Error('Duplicate NamedNodeMap probe names');
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
