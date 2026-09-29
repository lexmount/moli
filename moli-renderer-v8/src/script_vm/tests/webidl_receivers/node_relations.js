(() => {
  const childWindow = document.getElementById('child').contentWindow;
  const documents = [document, childWindow.document, document.implementation.createHTMLDocument(''), new DOMParser().parseFromString('<p></p>', 'text/html'), document.implementation.createDocument('urn:root', 'root')];
  const rows = [];
  const check = (name, expected, run) => {
    try { const actual = run(); rows.push({name, expected, actual, pass: actual === expected}); }
    catch (e) { rows.push({name, expected, error: e.name, pass: false}); }
  };
  function tree(d) {
    const root = d.createElementNS('urn:test', 'p:item');
    root.setAttributeNS('urn:attribute', 'a:key', 'value');
    root.appendChild(d.createTextNode('text'));
    root.appendChild(d.createComment('comment'));
    return root;
  }
  for (let i = 0; i < documents.length; i++) {
    for (let j = 0; j < documents.length; j++) {
      const left = tree(documents[i]), right = tree(documents[j]);
      let reads = 0;
      for (const node of [left, right]) {
        for (const key of ['nodeType', 'nodeName', 'parentNode', 'childNodes', 'ownerDocument']) {
          Object.defineProperty(node, key, {get() { reads++; throw new Error('public property read'); }, configurable: true});
        }
      }
      for (const [realm, methods] of [['own', left], ['top', Node.prototype], ['iframe', childWindow.Node.prototype]]) {
        const prefix = i+'/'+j+'/'+realm;
        check(prefix+'/equal', true, () => methods.isEqualNode.call(left, right));
        check(prefix+'/same', false, () => methods.isSameNode.call(left, right));
        check(prefix+'/contains', false, () => methods.contains.call(left, right));
        check(prefix+'/disconnected', true, () => (methods.compareDocumentPosition.call(left, right) & 33) === 33);
        check(prefix+'/child-contains', true, () => methods.contains.call(left, left.firstChild));
        check(prefix+'/child-position', 20, () => methods.compareDocumentPosition.call(left, left.firstChild));
        check(prefix+'/no-public-property-reads', 0, () => reads);
      }
      right.firstChild.data = 'changed';
      check(i+'/'+j+'/changed-child', false, () => left.isEqualNode(right));
    }
    const parent = documents[i].createElement('div');
    const child = parent.appendChild(documents[i].createElement('span'));
    const observer = new MutationObserver(() => {});
    observer.observe(parent, {childList:true});
    check(i+'/remove-return', child, () => parent.removeChild(child));
    check(i+'/remove-parent', null, () => child.parentNode);
    const records = observer.takeRecords();
    check(i+'/remove-record-count', 1, () => records.length);
    check(i+'/remove-record-node', child, () => records[0].removedNodes[0]);
    observer.disconnect();
  }
  for (const [realm, w] of [['top', window], ['iframe', childWindow]]) {
    for (const method of ['contains', 'isSameNode', 'isEqualNode', 'compareDocumentPosition', 'removeChild']) {
      const node = document.createElement('div');
      const revoked = Proxy.revocable(node, {}); revoked.revoke();
      for (const [kind, receiver] of [['object', {}], ['prototype', Object.create(Node.prototype)], ['inherited', Object.create(node)], ['proxy', new Proxy(node,{})], ['revoked',revoked.proxy], ['null',null]]) {
        check(realm+'/'+method+'/receiver-'+kind, true, () => {
          try { Reflect.apply(w.Node.prototype[method], receiver, [node]); return false; }
          catch(e) { return e instanceof w.TypeError; }
        });
      }
    }
  }
  // Store boolean results for identity checks so the evidence does not retain DOM objects.
  globalThis.__nodeArgumentResults = {total:rows.length, passed:rows.filter(r=>r.pass).length, failures:rows.filter(r=>!r.pass).map(({name,error})=>({name,error})), rows:rows.map(({name,pass,error})=>({name,pass,error}))};
  return rows.every(r=>r.pass);
})()
