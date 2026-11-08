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
  const attributes = {
    NodeIterator: ['root', 'whatToShow', 'filter', 'referenceNode', 'pointerBeforeReferenceNode'],
    TreeWalker: ['root', 'whatToShow', 'filter', 'currentNode']
  };
  const operations = {
    NodeIterator: ['nextNode', 'previousNode', 'detach'],
    TreeWalker: ['parentNode', 'firstChild', 'lastChild', 'previousSibling', 'nextSibling', 'previousNode', 'nextNode']
  };
  const setup = (owner, kind, flavor = 'live', filter = null) => {
    const doc = flavor === 'live' ? owner.document :
      flavor === 'html' ? owner.document.implementation.createHTMLDocument('traversal') :
      flavor === 'xml' ? owner.document.implementation.createDocument(null, 'root') :
      new owner.DOMParser().parseFromString('<root/>', 'application/xml');
    const root = flavor === 'xml' || flavor === 'parsed' ? doc.documentElement : doc.createElement('root');
    const a = root.appendChild(doc.createElement('a'));
    const leaf = a.appendChild(doc.createElement('leaf'));
    const b = root.appendChild(doc.createElement('b'));
    const traversal = doc['create' + kind](root, 1, filter);
    return {doc, root, a, leaf, b, traversal};
  };
  for (const [calleeName, callee] of realms) {
    for (const kind of ['NodeIterator', 'TreeWalker']) {
      const C = callee[kind], prototype = C.prototype;
      const prefix = `${calleeName} ${kind}`;
      const getters = Object.fromEntries(attributes[kind].map(name =>
        [name, Object.getOwnPropertyDescriptor(prototype, name).get]));
      const methods = Object.fromEntries(operations[kind].map(name => [name, prototype[name]]));
      const setter = kind === 'TreeWalker' ? Object.getOwnPropertyDescriptor(prototype, 'currentNode').set : null;
      for (const name of attributes[kind]) {
        check(`${prefix} ${name} prototype descriptor`, () => {
          const d = Object.getOwnPropertyDescriptor(prototype, name);
          return d.enumerable && d.configurable && d.get.name === 'get ' + name && d.get.length === 0 &&
            Object.getPrototypeOf(d.get) === callee.Function.prototype &&
            (name === 'currentNode' ? d.set.name === 'set currentNode' && d.set.length === 1 : d.set === undefined);
        });
      }
      for (const name of operations[kind]) {
        check(`${prefix} ${name} prototype descriptor`, () => {
          const d = Object.getOwnPropertyDescriptor(prototype, name);
          return d.enumerable && d.configurable && d.writable && d.value.name === name &&
            d.value.length === 0 && Object.getPrototypeOf(d.value) === callee.Function.prototype;
        });
      }
      for (const [ownerName, owner] of realms) {
        for (const flavor of ['live', 'html', 'xml', 'parsed']) {
          const label = `${prefix} borrows ${ownerName} ${flavor}`;
          const filter = {acceptNode() { return 1; }};
          const {root, a, leaf, b, traversal} = setup(owner, kind, flavor, filter);
          check(`${label} factory realm and interface identity`, () =>
            Object.getPrototypeOf(traversal) === owner[kind].prototype && traversal instanceof owner[kind]);
          check(`${label} root mask and filter preserve identity`, () =>
            getters.root.call(traversal) === root && getters.whatToShow.call(traversal) === 1 &&
            getters.filter.call(traversal) === filter &&
            Object.getPrototypeOf(root) === (flavor === 'live' || flavor === 'html' ? owner.HTMLUnknownElement.prototype : owner.Element.prototype));
          check(`${label} traversal state and detach remain functional`, () => {
            if (kind === 'NodeIterator') {
              if (getters.referenceNode.call(traversal) !== root || !getters.pointerBeforeReferenceNode.call(traversal)) return false;
              if (methods.nextNode.call(traversal) !== root || getters.pointerBeforeReferenceNode.call(traversal)) return false;
              if (methods.nextNode.call(traversal) !== a || methods.previousNode.call(traversal) !== a) return false;
              return methods.detach.call(traversal) === undefined && methods.nextNode.call(traversal) === a;
            }
            if (getters.currentNode.call(traversal) !== root || methods.firstChild.call(traversal) !== a) return false;
            if (methods.firstChild.call(traversal) !== leaf || methods.parentNode.call(traversal) !== a) return false;
            if (methods.nextSibling.call(traversal) !== b || methods.previousSibling.call(traversal) !== a) return false;
            if (setter.call(traversal, root) !== undefined || methods.lastChild.call(traversal) !== b) return false;
            if (methods.previousNode.call(traversal) !== leaf || methods.nextNode.call(traversal) !== b) return false;
            return getters.currentNode.call(traversal) === b;
          });
          check(`${label} frozen receiver keeps private native state`, () => {
            const sample = setup(owner, kind, flavor);
            Object.freeze(sample.traversal);
            return methods.nextNode.call(sample.traversal) === (kind === 'NodeIterator' ? sample.root : sample.a) &&
              getters.root.call(sample.traversal) === sample.root && Reflect.ownKeys(sample.traversal).length === 0;
          });
          check(`${label} prototype removal preserves native identity`, () => {
            const sample = setup(owner, kind, flavor);
            Object.setPrototypeOf(sample.traversal, null);
            return getters.root.call(sample.traversal) === sample.root &&
              methods.nextNode.call(sample.traversal) === (kind === 'NodeIterator' ? sample.root : sample.a);
          });
        }
        const valid = setup(owner, kind);
        const opposite = kind === 'NodeIterator' ? 'TreeWalker' : 'NodeIterator';
        const other = setup(owner, opposite);
        let traps = 0;
        const handler = {get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 1;}, has() {traps++; throw 1;}};
        const author = new Proxy(valid.traversal, handler);
        const revoked = Proxy.revocable(valid.traversal, handler); revoked.revoke();
        const invalid = [['null', null], ['undefined', undefined], ['plain', {}], ['number', 1],
          ['prototype', owner[kind].prototype], ['forged', Object.create(owner[kind].prototype)],
          ['inherited instance', Object.create(valid.traversal)], ['author Proxy', author],
          ['revoked Proxy', revoked.proxy], ['opposite traversal', other.traversal], ['Node', valid.root]];
        for (const [receiverName, receiver] of invalid) {
          for (const name of attributes[kind]) {
            check(`${prefix} ${name} rejects ${ownerName} ${receiverName} in callee realm`, () =>
              typeError(() => getters[name].call(receiver), callee));
          }
          for (const name of operations[kind]) {
            check(`${prefix} ${name} rejects ${ownerName} ${receiverName} in callee realm`, () =>
              typeError(() => methods[name].call(receiver), callee));
          }
          if (setter) {
            check(`${prefix} currentNode setter rejects ${ownerName} ${receiverName} before argument access`, () =>
              typeError(() => setter.call(receiver, new Proxy(valid.root, handler)), callee));
          }
        }
        check(`${prefix} ${ownerName} invalid receivers execute no author traps`, () => traps === 0);
        check(`${prefix} ${ownerName} opposite traversal state remains unchanged`, () =>
          other.traversal.root === other.root && (opposite === 'TreeWalker' ?
            other.traversal.currentNode === other.root : other.traversal.referenceNode === other.root && other.traversal.pointerBeforeReferenceNode));
        if (setter) {
          check(`${prefix} currentNode setter rejects ${ownerName} author Node Proxy without traps`, () =>
            typeError(() => setter.call(valid.traversal, new Proxy(valid.a, handler)), callee) &&
              getters.currentNode.call(valid.traversal) === valid.root && traps === 0);
        }
      }
      check(`${prefix} factory ignores replaced author constructor`, () => {
        const saved = Object.getOwnPropertyDescriptor(callee, kind);
        try {
          Object.defineProperty(callee, kind, {...saved, value: function FakeTraversal() { throw 1; }});
          const sample = setup(callee, kind);
          return Object.getPrototypeOf(sample.traversal) === prototype &&
            getters.root.call(sample.traversal) === sample.root &&
            methods.nextNode.call(sample.traversal) === (kind === 'NodeIterator' ? sample.root : sample.a);
        } finally { Object.defineProperty(callee, kind, saved); }
      });
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
