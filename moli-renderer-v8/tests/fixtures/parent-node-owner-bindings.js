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
  const kinds = ['Element', 'DocumentFragment', 'Document'];
  const attributes = ['children', 'firstElementChild', 'lastElementChild', 'childElementCount'];
  const operations = [['prepend', 0], ['append', 0], ['replaceChildren', 0], ['moveBefore', 2],
    ['querySelector', 1], ['querySelectorAll', 1]];
  let sequence = 0;
  const makeDocument = (owner, flavor) => flavor === 'live' ? owner.document :
    flavor === 'html' ? owner.document.implementation.createHTMLDocument('parent') :
    flavor === 'xml' ? owner.document.implementation.createDocument(null, 'root') :
    new owner.DOMParser().parseFromString('<root/>', 'application/xml');
  const setup = (owner, kind, flavor = 'html') => {
    const doc = makeDocument(owner, flavor);
    const root = kind === 'Document' ? doc : kind === 'Element' ? doc.createElement('div') : doc.createDocumentFragment();
    const box = kind === 'Document' ? (doc.body || doc.documentElement).appendChild(doc.createElement('fixture')) : root;
    const token = 'parent-binding-' + ++sequence;
    const a = box.appendChild(doc.createElement('a'));
    const nested = a.appendChild(doc.createElement('nested'));
    box.appendChild(doc.createTextNode('separator'));
    const b = box.appendChild(doc.createElement('b'));
    for (const [node, name] of [[a, 'first'], [nested, 'nested'], [b, 'last']]) {
      node.setAttribute('id', token + '-' + name); node.setAttribute('data-binding', token);
    }
    return {doc, root, box, a, nested, b, token, selector: '[data-binding="' + token + '"]'};
  };
  for (const [calleeName, callee] of realms) {
    for (const kind of kinds) {
      const prototype = callee[kind].prototype;
      const prefix = `${calleeName} ${kind}`;
      const getters = Object.fromEntries(attributes.map(name => [name, Object.getOwnPropertyDescriptor(prototype, name).get]));
      const methods = Object.fromEntries(operations.map(([name]) => [name, prototype[name]]));
      if (kind === 'DocumentFragment') methods.getElementById = prototype.getElementById;
      for (const name of attributes) {
        check(`${prefix} ${name} descriptor`, () => {
          const d = Object.getOwnPropertyDescriptor(prototype, name);
          return d.enumerable && d.configurable && d.set === undefined && d.get.name === 'get ' + name &&
            d.get.length === 0 && Object.getPrototypeOf(d.get) === callee.Function.prototype;
        });
      }
      for (const [name, arity] of operations.concat(kind === 'DocumentFragment' ? [['getElementById', 1]] : [])) {
        check(`${prefix} ${name} descriptor`, () => {
          const d = Object.getOwnPropertyDescriptor(prototype, name);
          return d.enumerable && d.configurable && d.writable && d.value.name === name &&
            d.value.length === arity && Object.getPrototypeOf(d.value) === callee.Function.prototype;
        });
      }
      for (const [ownerName, owner] of realms) {
        const flavors = kind === 'Document' ? ['html', 'xml', 'parsed'] : ['live', 'html', 'xml', 'parsed'];
        for (const flavor of flavors) {
          const sample = setup(owner, kind, flavor), {root, doc, a, nested, b, selector, token} = sample;
          const label = `${prefix} borrows ${ownerName} ${flavor}`;
          check(`${label} canonical method identity`, () => Object.entries(methods).every(([name, method]) =>
            root[name] === owner[kind].prototype[name] && !Object.hasOwn(root, name)));
          check(`${label} element getters and live collection`, () => {
            const children = getters.children.call(root);
            return children === getters.children.call(root) &&
              children.length === (kind === 'Document' ? 1 : 2) &&
              getters.firstElementChild.call(root) === (kind === 'Document' ? doc.documentElement : a) &&
              getters.lastElementChild.call(root) === (kind === 'Document' ? doc.documentElement : b) &&
              getters.childElementCount.call(root) === children.length;
          });
          check(`${label} query identity tree order and result realm`, () => {
            const list = methods.querySelectorAll.call(root, selector);
            return methods.querySelector.call(root, selector) === a && list.length === 3 &&
              list[0] === a && list[1] === nested && list[2] === b &&
              Object.getPrototypeOf(list) === owner.NodeList.prototype;
          });
          check(`${label} querySelectorAll remains static after mutation`, () => {
            const list = methods.querySelectorAll.call(root, selector);
            const later = doc.createElement('later'); later.setAttribute('data-binding', token);
            sample.box.appendChild(later);
            const ok = list.length === 3 && methods.querySelectorAll.call(root, selector).length === 4;
            later.remove(); return ok;
          });
          check(`${label} required selectors in callee realm`, () =>
            typeError(() => methods.querySelector.call(root), callee) &&
            typeError(() => methods.querySelectorAll.call(root), callee));
          check(`${label} selector conversion occurs once and preserves thrown identity`, () => {
            let conversions = 0;
            const selected = methods.querySelector.call(root, {toString() {conversions++; return selector;}});
            const marker = {};
            let propagated = false;
            try { methods.querySelectorAll.call(root, {toString() {throw marker;}}); }
            catch (error) { propagated = error === marker; }
            return selected === a && conversions === 1 && propagated;
          });
          check(`${label} invalid selector throws a callee-realm DOMException`, () => {
            for (const method of [methods.querySelector, methods.querySelectorAll]) {
              try { method.call(root, ':not('); return false; }
              catch (error) {
                if (error.name !== 'SyntaxError' || Object.getPrototypeOf(error) !== callee.DOMException.prototype) return false;
              }
            }
            return true;
          });
          check(`${label} native identity survives prototype removal`, () => {
            const value = setup(owner, kind, flavor);
            Object.setPrototypeOf(value.root, null);
            return methods.querySelector.call(value.root, value.selector) === value.a &&
              methods.querySelectorAll.call(value.root, value.selector).length === 3 &&
              getters.childElementCount.call(value.root) === (kind === 'Document' ? 1 : 2);
          });
          if (kind !== 'Document') {
            check(`${label} prepend append and replaceChildren retain mutation semantics`, () => {
              const value = setup(owner, kind, flavor), live = getters.children.call(value.root);
              const first = value.doc.createElement('first'), last = value.doc.createElement('last');
              if (methods.prepend.call(value.root, first) !== undefined || methods.append.call(value.root, last) !== undefined) return false;
              if (getters.firstElementChild.call(value.root) !== first || getters.lastElementChild.call(value.root) !== last || live.length !== 4) return false;
              const next = value.doc.createElement('next');
              return methods.replaceChildren.call(value.root, next, 'tail') === undefined && live.length === 1 &&
                getters.firstElementChild.call(value.root) === next && value.root.childNodes.length === 2 &&
                value.root.lastChild.data === 'tail';
            });
          }
          if (kind === 'DocumentFragment') {
            check(`${label} getElementById retains identity and required argument`, () =>
              methods.getElementById.call(root, token + '-nested') === nested &&
              methods.getElementById.call(root, '') === null &&
              typeError(() => methods.getElementById.call(root), callee));
          }
        }
        const valid = setup(owner, kind), otherKinds = kinds.filter(name => name !== kind);
        let traps = 0, conversions = 0;
        const handler = {get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 1;}, has() {traps++; throw 1;}};
        const revoked = Proxy.revocable(valid.root, handler); revoked.revoke();
        const invalid = [['null', null], ['undefined', undefined], ['plain', {}], ['number', 1],
          ['prototype', owner[kind].prototype], ['forged', Object.create(owner[kind].prototype)],
          ['inherited instance', Object.create(valid.root)], ['author Proxy', new Proxy(valid.root, handler)],
          ['revoked Proxy', revoked.proxy], ['Text', owner.document.createTextNode('text')],
          ['Attr', owner.document.createAttribute('data-x')],
          ...otherKinds.map(name => [name, setup(owner, name).root])];
        for (const [receiverName, receiver] of invalid) {
          for (const [name, getter] of Object.entries(getters)) {
            check(`${prefix} ${name} rejects ${ownerName} ${receiverName}`, () => {
              const before = traps;
              return typeError(() => getter.call(receiver), callee) && traps === before;
            });
          }
          for (const [name, method] of Object.entries(methods)) {
            check(`${prefix} ${name} rejects ${ownerName} ${receiverName} before conversion`, () => {
              const before = [traps, conversions];
              const poison = {toString() {conversions++; throw 1;}};
              const args = name === 'moveBefore' ? [owner.document.createElement('unused'), null] : [poison];
              return typeError(() => method.call(receiver, ...args), callee) && traps === before[0] && conversions === before[1];
            });
          }
        }
      }
    }
    for (const [ownerName, owner] of realms) {
      check(`${calleeName} borrows ${ownerName} active Document`, () =>
        callee.Document.prototype.querySelector.call(owner.document, 'html') === owner.document.documentElement &&
        Object.getOwnPropertyDescriptor(callee.Document.prototype, 'children').get.call(owner.document)[0] === owner.document.documentElement);
      for (const flavor of ['live', 'html', 'xml', 'parsed']) {
        const doc = makeDocument(owner, flavor);
        const host = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
        const shadow = host.attachShadow({mode: 'open'}), a = shadow.appendChild(doc.createElement('a'));
        a.setAttribute('id', 'inside'); a.setAttribute('data-inside', 'yes');
        for (const name of ['getElementById', 'querySelector', 'querySelectorAll']) {
          check(`${calleeName} ${ownerName} ${flavor} ShadowRoot inherits ${name}`, () =>
            !Object.hasOwn(owner.ShadowRoot.prototype, name) && shadow[name] === owner.DocumentFragment.prototype[name]);
        }
        check(`${calleeName} ${ownerName} ${flavor} ShadowRoot accepts DocumentFragment query operations`, () => {
          const prototype = callee.DocumentFragment.prototype;
          return prototype.getElementById.call(shadow, 'inside') === a &&
            prototype.querySelector.call(shadow, '[data-inside]') === a &&
            prototype.querySelectorAll.call(shadow, '[data-inside]')[0] === a;
        });
      }
    }
  }
  if (new Set(rows.map(row => row.name)).size !== rows.length) throw new Error('Duplicate ParentNode probe names');
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
