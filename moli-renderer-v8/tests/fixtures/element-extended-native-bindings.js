(() => {
  const rows = [];
  const check = (name, body) => {
    traps = 0;
    try { if (!body()) throw Error('false'); rows.push({name, passed:true}); }
    catch(error) { rows.push({name, passed:false, error:String(error)}); }
  };
  const other = document.querySelector('#child').contentWindow;
  const realms = [globalThis, other];
  const element = doc => doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
  let traps = 0;
  const handler = {get() {traps++; throw Error('author get');}, getPrototypeOf() {traps++; throw Error('author prototype');}};
  for (const [ci, callee] of realms.entries()) for (const [oi, owner] of realms.entries())
  for (const flavor of ['live', 'html', 'xml']) {
    const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
      owner.document.implementation.createHTMLDocument('attributes') : owner.document.implementation.createDocument(null, 'root');
    const prefix = `${ci}/${oi}/${flavor}`;
    const root = element(doc);
    (doc.body || doc.documentElement).appendChild(root);
    const proto = callee.Element.prototype;
    const methods = ['getAttributeNode', 'getAttributeNodeNS', 'setAttributeNode', 'setAttributeNodeNS',
      'removeAttributeNode', 'insertAdjacentElement', 'insertAdjacentText', 'insertAdjacentHTML'];
    const typeError = body => {try {body(); return false;} catch(error) {return Object.getPrototypeOf(error) === callee.TypeError.prototype;}};
    const revoked = Proxy.revocable(root, handler); revoked.revoke();
    const badReceivers = [null, undefined, {}, Object.create(proto), Object.create(root),
      new Proxy(root, handler), revoked.proxy, doc, doc.createTextNode('text')];
    for (const name of methods) for (const [ri, receiver] of badReceivers.entries()) {
      check(`${prefix}/${name}/receiver/${ri}`, () => {
        let conversions = 0;
        const poison = {toString() {conversions++; throw Error('conversion');}};
        const args = name.includes('AttributeNode') && !name.startsWith('get') ? [poison] : [poison, poison];
        return typeError(() => proto[name].apply(receiver, args)) && conversions === 0 && traps === 0;
      });
    }
    for (const name of methods) check(`${prefix}/${name}/missing`, () => typeError(() => proto[name].call(root)));
    const attr = doc.createAttribute('data-x');
    const revokedAttr = Proxy.revocable(attr, handler); revokedAttr.revoke();
    const badAttrs = [undefined, null, 0, false, '', Symbol(), {}, root, doc.createTextNode(''),
      Object.create(callee.Attr.prototype), Object.create(attr), new Proxy(attr, handler), revokedAttr.proxy];
    for (const name of ['setAttributeNode', 'setAttributeNodeNS', 'removeAttributeNode'])
    for (const [ai, value] of badAttrs.entries()) check(`${prefix}/${name}/argument/${ai}`, () =>
      typeError(() => proto[name].call(root, value)) && root.attributes.length === 0 && traps === 0);
    for (const position of ['beforebegin','afterbegin','beforeend','afterend','wrong']) {
      for (const attached of [false, true]) for (const [vi, value] of [null, doc.createTextNode(''), new Proxy(root, handler)].entries()) {
        check(`${prefix}/adjacent/type/${position}/${attached}/${vi}`, () => {
          const target = element(doc); if (attached) root.appendChild(target);
          try {return typeError(() => proto.insertAdjacentElement.call(target, position, value)) && traps === 0;}
          finally {target.remove();}
        });
      }
    }
    check(`${prefix}/adjacent/position-conversion-first`, () => {
      const sentinel = {};
      try {proto.insertAdjacentElement.call(root, {toString() {throw sentinel;}}, null); return false;}
      catch(error) {return error === sentinel;}
    });
    for (const position of ['beforebegin','afterbegin','beforeend','afterend']) {
      check(`${prefix}/adjacent/insert/${position}`, () => {
        const parent = element(doc), target = element(doc), node = element(doc);
        parent.appendChild(target); root.appendChild(parent);
        const value = proto.insertAdjacentElement.call(target, position, node);
        const expectedParent = position === 'afterbegin' || position === 'beforeend' ? target : parent;
        const ok = value === node && node.parentNode === expectedParent;
        parent.remove(); return ok;
      });
      for (const [vi, value] of ['\ud800', '\udfff', '\ud800a\udfff', '𝌆', '', null, 0].entries())
      check(`${prefix}/adjacent/text/${position}/${vi}`, () => {
        const parent = element(doc), target = element(doc); parent.appendChild(target); root.appendChild(parent);
        const result = proto.insertAdjacentText.call(target, position, value);
        const node = position === 'beforebegin' ? target.previousSibling : position === 'afterend' ? target.nextSibling : target.firstChild;
        const ok = result === undefined && node.nodeType === 3 && node.data === String(value) && node.ownerDocument === doc;
        parent.remove(); return ok;
      });
    }
    check(`${prefix}/attributes/replace-and-remove`, () => {
      const a = doc.createAttributeNS('urn:x', 'p:entry'); a.value = '\ud800x\udfff';
      const b = doc.createAttributeNS('urn:x', 'q:entry'); b.value = 'replacement';
      if (proto.setAttributeNodeNS.call(root, a) !== null || a.ownerElement !== root ||
        proto.getAttributeNodeNS.call(root, 'urn:x', 'entry') !== a) return false;
      if (proto.setAttributeNodeNS.call(root, b) !== a || a.ownerElement !== null || a.value !== '\ud800x\udfff') return false;
      return proto.removeAttributeNode.call(root, b) === b && b.ownerElement === null && root.attributes.length === 0;
    });
    check(`${prefix}/attributes/in-use`, () => {
      const a = doc.createAttribute('owned'), second = element(doc); root.appendChild(second);
      proto.setAttributeNode.call(root, a);
      let error; try {proto.setAttributeNode.call(second, a);} catch(value) {error = value;}
      const ok = error?.name === 'InUseAttributeError' && a.ownerElement === root;
      proto.removeAttributeNode.call(root, a); second.remove(); return ok;
    });
    check(`${prefix}/attributes/not-found`, () => {
      let error; try {proto.removeAttributeNode.call(root, doc.createAttribute('absent'));} catch(value) {error = value;}
      return error?.name === 'NotFoundError';
    });
    check(`${prefix}/attributes/ignore-public-metadata`, () => {
      const a = doc.createAttribute('private'); a.value = '\ud800';
      const names = ['name','localName','namespaceURI','prefix','value','ownerElement','nodeType'];
      for (const name of names) Object.defineProperty(a, name, {configurable:true, get() {throw Error('public Attr ' + name);}});
      if (proto.setAttributeNode.call(root, a) !== null) return false;
      const ok = root.getAttribute('private') === '\ud800' && proto.removeAttributeNode.call(root, a) === a;
      for (const name of names) delete a[name]; return ok && a.ownerElement === null && a.value === '\ud800';
    });
    root.remove();
  }
  globalThis.__uiEventResults = {complete:true, total:rows.length, passed:rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
