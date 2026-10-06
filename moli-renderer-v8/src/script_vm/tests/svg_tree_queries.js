(() => {
  const checks = [];
  const check = (name, body) => {
    try { checks.push({name, passed: body() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const ns = 'http://www.w3.org/2000/svg';
  const child = document.querySelector('iframe').contentWindow;
  const documents = [];
  for (const [owner, realm] of [['top', window], ['child', child]]) {
    for (const [kind, doc] of [
      ['live', realm.document],
      ['html', realm.document.implementation.createHTMLDocument('')],
      ['xml', realm.document.implementation.createDocument(ns, 'svg')],
      ['parser', new realm.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')],
    ]) documents.push({owner, kind, realm, doc});
  }
  for (const [callee, realm] of [['top', window], ['child', child]]) {
    const getter = Object.getOwnPropertyDescriptor(realm.SVGElement.prototype, 'viewportElement')?.get;
    const method = realm.SVGSVGElement.prototype.getElementById;
    check(callee + ' viewport descriptor', () => {
      const d = Object.getOwnPropertyDescriptor(realm.SVGElement.prototype, 'viewportElement');
      return typeof getter === 'function' && getter.name === 'get viewportElement' && getter.length === 0 &&
        d.enumerable && d.configurable && d.set === undefined;
    });
    check(callee + ' ID method descriptor', () => {
      const d = Object.getOwnPropertyDescriptor(realm.SVGSVGElement.prototype, 'getElementById');
      return typeof method === 'function' && method.name === 'getElementById' && method.length === 1 &&
        d.enumerable && d.configurable && d.writable;
    });
    for (const {owner, kind, realm: ownerRealm, doc} of documents) {
      const prefix = callee + '/' + owner + '/' + kind;
      const svg = tag => doc.createElementNS(ns, tag);
      const root = svg('svg'), group = svg('g'), leaf = svg('rect');
      root.id = 'same'; group.appendChild(leaf); root.appendChild(group);
      (doc.body || doc.documentElement).appendChild(root);
      const query = id => Reflect.apply(method, root, [id]);
      const viewport = node => Reflect.apply(getter, node, []);
      check(prefix + ' root viewport follows actual ancestry', () =>
        viewport(root) === (root.parentNode.namespaceURI === ns ? root.parentNode : null));
      check(prefix + ' nested viewport and identity', () => viewport(leaf) === root &&
        Object.getPrototypeOf(viewport(leaf)) === ownerRealm.SVGSVGElement.prototype);
      for (const tag of ['svg', 'symbol', 'image', 'g', 'marker', 'pattern', 'foreignObject', 'filter']) {
        check(prefix + ' viewport ancestor ' + tag, () => {
          const parent = svg(tag), item = svg('rect');
          parent.appendChild(item); root.appendChild(parent);
          const expected = ['svg', 'symbol', 'image'].includes(tag) ? parent : root;
          const passed = viewport(item) === expected && viewport(parent) === root;
          parent.remove(); return passed;
        });
      }
      check(prefix + ' detached ancestry', () => {
        root.remove(); const passed = viewport(leaf) === root && viewport(root) === null;
        (doc.body || doc.documentElement).appendChild(root); return passed;
      });
      check(prefix + ' reparent updates viewport', () => {
        const nested = svg('svg'); root.appendChild(nested); nested.appendChild(leaf);
        const passed = viewport(leaf) === nested; group.appendChild(leaf); nested.remove();
        return passed && viewport(leaf) === root;
      });
      check(prefix + ' excludes root', () => query('same') === null);
      check(prefix + ' first descendant in tree order', () => {
        const a = svg('rect'), b = svg('circle'); a.id = b.id = 'same';
        group.appendChild(a); root.appendChild(b);
        const first = query('same') === a; root.insertBefore(b, group);
        const reordered = query('same') === b; b.remove(); a.remove();
        return first && reordered && query('same') === null;
      });
      check(prefix + ' subtree isolation', () => {
        const outside = svg('rect'); outside.id = 'outside'; root.parentNode.appendChild(outside);
        const passed = query('outside') === null; outside.remove(); return passed;
      });
      for (const id of ['case', 'Case', 'a:b', '#x', ' a ', '\u0000', '\ufffd']) {
        check(prefix + ' exact ID ' + JSON.stringify(id), () => {
          leaf.setAttribute('id', id); const passed = query(id) === leaf;
          leaf.removeAttribute('id'); return passed;
        });
      }
      check(prefix + ' incoming surrogate ID does not match replacement', () => {
        leaf.id = '\ufffd';
        const passed = query('\ud800') === null && query('\ufffd') === leaf;
        leaf.removeAttribute('id'); return passed;
      });
      check(prefix + ' empty ID matches no element', () => {
        leaf.id = ''; return query('') === null;
      });
      check(prefix + ' namespaced id is not an ID', () => {
        leaf.removeAttribute('id'); leaf.setAttributeNS('urn:test', 'test:id', 'namespaced');
        const passed = query('namespaced') === null; leaf.removeAttributeNS('urn:test', 'id'); return passed;
      });
      check(prefix + ' ID case sensitivity', () => {
        leaf.id = 'Case'; const passed = query('Case') === leaf && query('case') === null;
        leaf.removeAttribute('id'); return passed;
      });
      check(prefix + ' cloned subtree uses clone identities', () => {
        leaf.id = 'clone'; const clone = root.cloneNode(true);
        const expected = clone.firstElementChild.firstElementChild;
        const passed = Reflect.apply(method, clone, ['clone']) === expected && expected !== leaf;
        leaf.removeAttribute('id'); return passed;
      });
      check(prefix + ' native lookup ignores own DOM methods', () => {
        leaf.id = 'native'; let calls = 0;
        root.querySelector = () => {calls++; throw 42;};
        leaf.getAttribute = () => {calls++; throw 42;};
        const passed = query('native') === leaf && calls === 0;
        delete root.querySelector; delete leaf.getAttribute; leaf.removeAttribute('id'); return passed;
      });
      check(prefix + ' DOMString conversion and mutation ordering', () => {
        let conversions = 0;
        const value = {toString() {conversions++; leaf.id = 'converted'; return 'converted';}};
        const passed = query(value) === leaf && conversions === 1;
        leaf.removeAttribute('id'); return passed;
      });
      check(prefix + ' conversion exception identity', () => {
        const sentinel = {}; let error;
        try {query({toString() {throw sentinel;}});} catch (caught) {error = caught;}
        return typeof method === 'function' && error === sentinel;
      });
      check(prefix + ' missing argument throws callee TypeError', () => {
        let error; try {Reflect.apply(method, root, []);} catch (caught) {error = caught;}
        return typeof method === 'function' && Object.getPrototypeOf(error) === realm.TypeError.prototype;
      });
      check(prefix + ' Symbol conversion throws callee TypeError', () => {
        let error; try {query(Symbol());} catch (caught) {error = caught;}
        return typeof method === 'function' && Object.getPrototypeOf(error) === realm.TypeError.prototype;
      });
      check(prefix + ' foreign HTML descendants and shadow exclusion', () => {
        const foreign = svg('foreignObject'), host = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
        host.id = 'html-descendant'; foreign.appendChild(host); root.appendChild(foreign);
        const shadow = host.attachShadow({mode: 'open'}), item = svg('rect'); item.id = 'shadow';
        shadow.appendChild(item);
        const passed = query('html-descendant') === host && query('shadow') === null && viewport(item) === root;
        foreign.remove(); return passed;
      });
      check(prefix + ' nested outermost SVG in foreignObject', () => {
        const foreign = svg('foreignObject'), nested = svg('svg'); foreign.appendChild(nested); root.appendChild(foreign);
        const passed = viewport(nested) === null; foreign.remove(); return passed;
      });
      for (const [member, fn, real, interfaceName] of [
        ['viewport', getter, leaf, 'SVGElement'], ['ID', method, root, 'SVGSVGElement'],
      ]) {
        let traps = 0;
        const author = new Proxy(real, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}});
        const revoked = Proxy.revocable(real, {}); revoked.revoke();
        const wrong = doc.createElementNS('http://www.w3.org/1999/xhtml', 'svg');
        const receivers = [undefined, null, true, 1, '', 0n, Symbol(), {}, Object.create(null),
          realm[interfaceName].prototype, Object.create(realm[interfaceName].prototype),
          Object.create(real), author, revoked.proxy, doc, wrong];
        if (member === 'ID') receivers.push(leaf);
        receivers.forEach((receiver, index) => {
          check(prefix + '/' + member + ' reject receiver ' + index + ' before conversion', () => {
            let conversions = 0, error;
            const value = {toString() {conversions++; return 'same';}};
            try {Reflect.apply(fn, receiver, member === 'ID' ? [value] : []);} catch (caught) {error = caught;}
            return typeof fn === 'function' && error && Object.getPrototypeOf(error) === realm.TypeError.prototype &&
              conversions === 0 && traps === 0;
          });
        });
      }
      root.remove();
    }
  }
  globalThis.__uiEventResults = {complete: true, passed: checks.filter(row => row.passed).length,
    total: checks.length, checks};
  return true;
})()
