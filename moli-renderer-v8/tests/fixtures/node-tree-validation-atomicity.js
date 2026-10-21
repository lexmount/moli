(() => {
  const rows = [];
  const other = document.querySelector('#child').contentWindow;
  for (const [ci, callee] of [globalThis, other].entries()) {
    for (const [oi, owner] of [globalThis, other].entries()) {
      for (const flavor of ['live', 'html', 'xml', 'parsed', 'new']) {
        const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
          owner.document.implementation.createHTMLDocument('mutations') : flavor === 'xml' ?
          owner.document.implementation.createDocument(null, 'root') : flavor === 'new' ?
          new owner.Document() : new owner.DOMParser().parseFromString('<root/>', 'application/xml');
        const prefix = `${ci}/${oi}/${flavor}`;
        const check = (name, expectedError, makeParent, makeChild, reference) => {
          const sourceOwner = owner === globalThis ? other : globalThis;
          const source = sourceOwner.document.implementation.createDocument(null, 'source');
          const holder = source.documentElement;
          const child = makeChild(source);
          holder.appendChild(child);
          const parent = makeParent(), before = [...parent.childNodes];
          try {
            let error;
            try {callee.Node.prototype[name].call(parent, child, reference);} catch (caught) {error = caught;}
            const passed = error && error.name === expectedError &&
              Object.getPrototypeOf(error) === callee.DOMException.prototype &&
              child.ownerDocument === source && child.parentNode === holder && holder.firstChild === child &&
              before.length === parent.childNodes.length && before.every((node, i) => parent.childNodes[i] === node);
            rows.push({name: `${prefix}/${name}/${expectedError}/${parent.nodeType}/${child.nodeType}`, passed: !!passed,
              error: passed ? undefined : String(error)});
          } catch (error) {rows.push({name: `${prefix}/${name}/probe-error`, passed: false, error: String(error)});}
        };
        for (const method of ['appendChild', 'insertBefore', 'replaceChild']) {
          const parents = [() => doc.createTextNode('text'), () => doc.createComment('comment'),
            () => doc.createAttribute('attr'), () => doc.implementation.createDocumentType('r', '', '')];
          for (const makeParent of parents) check(method, 'HierarchyRequestError', makeParent,
            source => source.createComment('child'), method === 'replaceChild' ? doc.createComment('old') : null);
          if (method !== 'appendChild') check(method, 'NotFoundError', () => doc.createElement('parent'),
            source => source.createComment('child'), doc.createComment('not-a-child'));
          if (method !== 'replaceChild') check(method, 'HierarchyRequestError', () => doc,
            source => source.createTextNode('text'), null);
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
