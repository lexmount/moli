(() => {
  const rows = [];
  let traps = 0, authorTraps = 0;
  const check = (name, body) => {
    traps = 0;
    try {
      if (!body()) throw Error('false');
      rows.push({name, passed: true});
    } catch (error) {
      rows.push({name, passed: false, error: String(error)});
    } finally {
      authorTraps += traps;
    }
  };
  const sameNodes = (nodes, expected) => nodes.length === expected.length &&
    expected.every((node, index) => nodes[index] === node);
  const other = document.querySelector('#child').contentWindow;
  for (const [ci, callee] of [globalThis, other].entries()) {
    for (const [oi, owner] of [globalThis, other].entries()) {
      for (const flavor of ['live', 'html', 'xml', 'parsed', 'new']) {
        const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
          owner.document.implementation.createHTMLDocument('mutations') : flavor === 'xml' ?
          owner.document.implementation.createDocument(null, 'root') : flavor === 'new' ?
          new owner.Document() : new owner.DOMParser().parseFromString('<root/>', 'application/xml');
        const prefix = `${ci}/${oi}/${flavor}`;
        const kinds = ['Document', 'Element', 'DocumentFragment', 'ShadowRoot',
          'Text', 'Comment', 'ProcessingInstruction', 'Attr', 'DocumentType'];
        if (!['live', 'html'].includes(flavor)) kinds.push('CDATASection');
        const makeParent = kind => {
          switch (kind) {
            case 'Document': return doc;
            case 'Element': return doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
            case 'DocumentFragment': return doc.createDocumentFragment();
            case 'ShadowRoot': return doc.createElementNS('http://www.w3.org/1999/xhtml', 'div').attachShadow({mode: 'open'});
            case 'Text': return doc.createTextNode('text');
            case 'Comment': return doc.createComment('comment');
            case 'ProcessingInstruction': return doc.createProcessingInstruction('target', 'data');
            case 'Attr': return doc.createAttribute('attribute');
            case 'DocumentType': return doc.implementation.createDocumentType('root', '', '');
            case 'CDATASection': return doc.createCDATASection('cdata');
          }
        };
        const handler = {
          get() { traps++; throw Error('receiver get'); },
          getPrototypeOf() { traps++; throw Error('receiver prototype'); }
        };
        const typeError = (realm, body) => {
          try { body(); return false; }
          catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
        };
        const domError = (realm, name, body) => {
          try { body(); return false; }
          catch (error) {
            return error.name === name && Object.getPrototypeOf(error) === realm.DOMException.prototype;
          }
        };
        for (const method of ['appendChild', 'insertBefore', 'replaceChild', 'removeChild']) {
          const fn = callee.Node.prototype[method];
          check(`${prefix}/${method}/shape`, () => {
            const descriptor = Object.getOwnPropertyDescriptor(callee.Node.prototype, method);
            return descriptor.enumerable && descriptor.configurable && descriptor.writable &&
              fn.length === (['insertBefore', 'replaceChild'].includes(method) ? 2 : 1);
          });
          const real = makeParent('Element');
          const revoked = Proxy.revocable(real, handler);
          revoked.revoke();
          for (const [ri, receiver] of [null, undefined, {}, Object.create(callee.Node.prototype),
            Object.create(real), new Proxy(real, handler), revoked.proxy, new owner.Event('test')].entries()) {
            check(`${prefix}/${method}/receiver/${ri}`, () => {
              const child = doc.createComment('child');
              const args = ['insertBefore', 'replaceChild'].includes(method) ? [child, child] : [child];
              return typeError(callee, () => fn.apply(receiver, args)) && traps === 0;
            });
          }
          for (const kind of kinds) {
            const sample = makeParent(kind);
            check(`${prefix}/${method}/${kind}/inherited`, () =>
              sample[method] === owner.Node.prototype[method]);
            const styles = [['borrowed', fn, callee]];
            if (ci === 0) styles.push(['local', sample[method], owner]);
            for (const [style, operation, realm] of styles) {
              const name = `${prefix}/${method}/${kind}/${style}`;
              for (const input of ['missing', 'undefined', 'null', 'object', 'forged', 'proxy', 'revoked', 'symbol', 'poison']) {
                check(`${name}/argument/${input}`, () => {
                  const parent = makeParent(kind), child = doc.createComment('child');
                  const before = [...parent.childNodes];
                  const revokedChild = Proxy.revocable(child, handler);
                  revokedChild.revoke();
                  const poison = new Proxy({}, {get() {traps++; throw Error('argument property');}});
                  const value = {undefined, null: null, object: {}, forged: Object.create(child),
                    proxy: new Proxy(child, handler), revoked: revokedChild.proxy,
                    symbol: Symbol(), poison}[input];
                  const args = input === 'missing' ? [] :
                    ['insertBefore', 'replaceChild'].includes(method) ? [value, child] : [value];
                  const rejected = typeError(realm, () => operation.apply(parent, args));
                  return rejected && traps === 0 && sameNodes(parent.childNodes, before) && child.parentNode === null;
                });
              }
              if (['insertBefore', 'replaceChild'].includes(method)) {
                check(`${name}/missing-second`, () =>
                  typeError(realm, () => operation.call(makeParent(kind), doc.createComment('child'))));
                for (const first of ['comment', 'attribute', 'fragment']) {
                  check(`${name}/second-before-hierarchy/${first}`, () => {
                    const parent = makeParent(kind), before = [...parent.childNodes];
                    const child = first === 'comment' ? doc.createComment('child') : first === 'attribute' ?
                      doc.createAttribute('arg') : doc.createDocumentFragment();
                    let converted = 0;
                    const second = new Proxy({}, {get() {converted++; throw Error('second argument');}});
                    const rejected = typeError(realm, () => operation.call(parent, child, second));
                    const untouched = sameNodes(parent.childNodes, before) && child.parentNode === null;
                    if (child.parentNode) child.remove();
                    return rejected && untouched && converted === 0;
                  });
                }
                check(`${name}/foreign-second-conversion-is-atomic`, () => {
                  const sourceOwner = owner === globalThis ? other : globalThis;
                  const source = new sourceOwner.Document();
                  const holder = source.createElement('source'), child = source.createElement('child');
                  holder.appendChild(child);
                  const parent = makeParent(kind), before = [...parent.childNodes];
                  const invalid = new Proxy({}, {get() {traps++; throw Error('foreign argument');}});
                  const rejected = typeError(realm, () => operation.call(parent, child, invalid));
                  const untouched = child.ownerDocument === source && child.parentNode === holder &&
                    holder.firstChild === child && sameNodes(parent.childNodes, before);
                  if (child.parentNode !== holder && child.parentNode) child.remove();
                  return rejected && untouched && traps === 0;
                });
              }
              if (!['Document', 'Element', 'DocumentFragment', 'ShadowRoot'].includes(kind)) {
                check(`${name}/genuine-parent-validation`, () => {
                  const parent = makeParent(kind), child = doc.createComment('child');
                  const args = method === 'insertBefore' ? [child, null] : method === 'replaceChild' ?
                    [child, doc.createComment('old')] : [child];
                  return domError(realm, method === 'removeChild' ? 'NotFoundError' : 'HierarchyRequestError',
                    () => operation.apply(parent, args)) && child.parentNode === null;
                });
              }
              if (method === 'insertBefore') {
                for (const reference of [null, undefined]) check(`${name}/nullable/${String(reference)}`, () => {
                  const parent = makeParent(kind), child = doc.createComment('child');
                  const container = ['Document', 'Element', 'DocumentFragment', 'ShadowRoot'].includes(kind);
                  if (!container) return domError(realm, 'HierarchyRequestError', () => operation.call(parent, child, reference));
                  try { return operation.call(parent, child, reference) === child && parent.lastChild === child; }
                  finally { if (child.parentNode) child.remove(); }
                });
              }
            }
          }
        }
        for (const kind of ['Element', 'DocumentFragment', 'ShadowRoot']) {
          check(`${prefix}/${kind}/native-mutation-records-and-iterator`, () => {
            const parent = makeParent(kind), methods = callee.Node.prototype;
            const a = doc.createElementNS('http://www.w3.org/1999/xhtml', 'a');
            const b = doc.createTextNode('\ud800text\udfff'), c = doc.createComment('comment');
            const observer = new owner.MutationObserver(() => {});
            observer.observe(parent, {childList: true});
            const iterator = kind === 'Element' ? doc.evaluate('child::*', parent, null, 4, null) : null;
            try {
              if (methods.appendChild.call(parent, a) !== a || methods.insertBefore.call(parent, b, a) !== b ||
                  methods.replaceChild.call(parent, c, a) !== a || methods.removeChild.call(parent, b) !== b) return false;
              const records = observer.takeRecords();
              const expected = [[[a], []], [[b], []], [[c], [a]], [[], [b]]];
              return sameNodes(parent.childNodes, [c]) && a.parentNode === null && b.parentNode === null &&
                b.data === '\ud800text\udfff' && [a, b, c].every(node => node.ownerDocument === doc) &&
                records.length === 4 && records.every((record, index) => record.target === parent &&
                  sameNodes(record.addedNodes, expected[index][0]) && sameNodes(record.removedNodes, expected[index][1])) &&
                (iterator === null || iterator.invalidIteratorState === true);
            } finally { observer.disconnect(); }
          });
        }
      }
    }
  }
  rows.push({name: 'no-author-traps', passed: authorTraps === 0});
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
