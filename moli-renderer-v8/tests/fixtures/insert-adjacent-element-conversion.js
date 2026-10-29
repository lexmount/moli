(() => {
  const assert = (value, message) => {if (!value) throw Error(message);};
  const other = document.getElementById('child').contentWindow;
  const realms = [window, other];
  let cases = 0;
  for (const callee of realms) for (const owner of realms) {
    const callback = callee.Element.prototype.insertAdjacentElement;
    const typeError = body => {
      let caught;
      try {body();} catch (error) {caught = error;}
      assert(caught && Object.getPrototypeOf(caught) === callee.TypeError.prototype,
        'expected callee-realm TypeError');
      cases++;
    };
    for (const doc of [owner.document, owner.document.implementation.createHTMLDocument(''),
      owner.document.implementation.createDocument(null, 'root')]) {
      const make = () => doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
      const root = make();
      (doc.body || doc.documentElement).appendChild(root);
      const node = make();
      let traps = 0, conversions = 0;
      const poison = {toString() {conversions++; throw 42;}};
      const handler = {get() {traps++; throw 43;}, getPrototypeOf() {traps++; throw 43;}};
      const revoked = Proxy.revocable(root, {}); revoked.revoke();
      for (const receiver of [null, undefined, {}, doc, doc.createTextNode(''),
        Object.create(callee.Element.prototype), Object.create(root),
        new Proxy(root, handler), revoked.proxy]) {
        typeError(() => callback.call(receiver, poison, poison));
      }
      assert(traps === 0 && conversions === 0, 'receiver validation precedes author code');
      const revokedNode = Proxy.revocable(node, {}); revokedNode.revoke();
      const invalid = [undefined, null, false, 0, '', Symbol(), {}, doc,
        doc.createTextNode(''), Object.create(node), new Proxy(node, handler), revokedNode.proxy];
      for (const position of ['beforebegin', 'afterbegin', 'beforeend', 'afterend', 'wrong']) {
        for (const attached of [false, true]) {
          const target = make();
          if (attached) root.appendChild(target);
          for (const value of invalid) typeError(() => callback.call(target, position, value));
          assert(target.childNodes.length === 0, 'failed conversion cannot mutate the target');
          target.remove();
        }
      }
      assert(traps === 0, 'interface conversion must not invoke Proxy traps');
      typeError(() => callback.call(root));
      typeError(() => callback.call(root, 'beforeend'));
      const marker = {};
      let caught;
      try {callback.call(root, {toString() {throw marker;}}, null);} catch (error) {caught = error;}
      assert(caught === marker, 'position conversion happens before the Element argument');
      for (const position of ['beforebegin', 'afterbegin', 'beforeend', 'afterend']) {
        const parent = make(), target = make(), child = make();
        root.appendChild(parent); parent.appendChild(target);
        assert(callback.call(target, position, child) === child, 'return original Element argument');
        assert(child.parentNode === (position === 'afterbegin' || position === 'beforeend' ? target : parent),
          'insert at the requested position');
        parent.remove(); cases++;
      }
      assert(callback.call(node, 'beforebegin', make()) === null, 'valid detached insertion returns null');
      root.remove();
    }
  }
  return cases;
})()
