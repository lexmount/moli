(() => {
  const rows = [];
  function check(name, body) {
    try { rows.push({name, passed: body() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  }
  function typeError(realm, body) {
    try { body(); return false; }
    catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
  }
  const realms = [['top', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const getters = ['host', 'mode', 'delegatesFocus', 'slotAssignment', 'clonable',
    'serializable', 'activeElement', 'innerHTML', 'styleSheets', 'adoptedStyleSheets', 'onslotchange'];
  function sample(owner, flavor, mode = 'open') {
    const doc = flavor === 'html' ? owner.document.implementation.createHTMLDocument('shadow') :
      flavor === 'xml' ? owner.document.implementation.createDocument(null, 'r') : owner.document;
    const host = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
    const root = host.attachShadow({mode, delegatesFocus: true, clonable: true, serializable: true});
    const text = doc.createTextNode('text');
    host.appendChild(text);
    (doc.body || doc.documentElement).appendChild(host);
    return {doc, host, root, text};
  }
  function withSample(owner, flavor, mode, body) {
    const value = sample(owner, flavor, mode);
    try { return body(value); }
    finally { value.host.remove(); }
  }
  function invalidValues(owner, value, seen) {
    const revoked = owner.Proxy.revocable(value.root, {}); revoked.revoke();
    return [
      ['undefined', undefined], ['null', null], ['number', 1], ['string', 'root'],
      ['symbol', Symbol('root')], ['object', {}], ['function', function() {}],
      ['prototype', Object.create(owner.ShadowRoot.prototype)],
      ['inherited', Object.create(value.root)], ['fragment', value.doc.createDocumentFragment()],
      ['document', value.doc], ['element', value.host], ['text', value.text],
      ['author proxy', new owner.Proxy(value.root, {
        get() {seen.push('get'); throw Error('author get');},
        getPrototypeOf() {seen.push('getPrototypeOf'); throw Error('author prototype');}
      })], ['revoked', revoked.proxy]
    ];
  }
  for (const [calleeName, callee] of realms) {
    for (const [proto, names] of [
      [callee.ShadowRoot.prototype, ['getHTML', 'setHTMLUnsafe', 'elementFromPoint', 'elementsFromPoint', 'getSelection']],
      [callee.HTMLSlotElement.prototype, ['assignedNodes', 'assignedElements', 'assign']]
    ]) for (const name of names) check(`${calleeName} ${name} method descriptor`, () => {
      const d = Object.getOwnPropertyDescriptor(proto, name);
      return d?.enumerable === true && d.configurable === true && d.writable === true && typeof d.value === 'function';
    });
    for (const [ownerName, owner] of realms) for (const flavor of ['live', 'html', 'xml']) {
      const label = `${calleeName} binding ${ownerName} ${flavor}`;
      withSample(owner, flavor, 'open', value => {
        const seen = [];
        for (const [badName, receiver] of invalidValues(owner, value, seen)) {
          for (const name of getters) check(`${label} ${name} rejects ${badName} without traps`, () => {
            seen.length = 0;
            const get = Object.getOwnPropertyDescriptor(callee.ShadowRoot.prototype, name).get;
            return typeError(callee, () => Reflect.apply(get, receiver, [])) && seen.length === 0;
          });
          for (const name of ['innerHTML', 'adoptedStyleSheets', 'onslotchange'])
            check(`${label} ${name} setter rejects ${badName} before conversion`, () => {
              seen.length = 0; let conversions = 0;
              const arg = {toString() {conversions++; return '';},
                get [Symbol.iterator]() {conversions++; throw Error('sequence conversion');}};
              const set = Object.getOwnPropertyDescriptor(callee.ShadowRoot.prototype, name).set;
              return typeError(callee, () => Reflect.apply(set, receiver, [arg])) && conversions === 0 && seen.length === 0;
            });
          for (const name of ['getHTML', 'setHTMLUnsafe', 'elementFromPoint', 'elementsFromPoint', 'getSelection'])
            check(`${label} ${name} rejects ${badName} before conversion`, () => {
              seen.length = 0; let conversions = 0;
              const arg = {valueOf() {conversions++; return 0;}, toString() {conversions++; return '';},
                get serializableShadowRoots() {conversions++; return false;}};
              return typeError(callee, () => Reflect.apply(callee.ShadowRoot.prototype[name], receiver, [arg, arg])) &&
                conversions === 0 && seen.length === 0;
            });
        }
      });
      for (const mode of ['open', 'closed']) for (const name of getters)
        check(`${label} ${name} accepts genuine ${mode} root with poisoned public state`, () =>
          withSample(owner, flavor, mode, ({host, root}) => {
            const get = Object.getOwnPropertyDescriptor(callee.ShadowRoot.prototype, name).get;
            let reads = 0;
            for (const prop of ['nodeType', 'ownerDocument', 'parentNode', 'host', 'mode'])
              Object.defineProperty(root, prop, {configurable: true, get() {reads++; throw Error(prop);}});
            const value = Reflect.apply(get, root, []);
            return reads === 0 && (name === 'host' ? value === host : name === 'mode' ? value === mode :
              ['delegatesFocus', 'clonable', 'serializable'].includes(name) ? value === true :
              name === 'slotAssignment' ? value === 'named' : name === 'innerHTML' ? value === '' :
              ['styleSheets', 'adoptedStyleSheets'].includes(name) ? value.length === 0 : value === null);
          }));
      check(`${label} innerHTML setter preserves conversion exception identity`, () =>
        withSample(owner, flavor, 'open', ({root}) => {
          const sentinel = {}; let conversions = 0;
          try { Object.getOwnPropertyDescriptor(callee.ShadowRoot.prototype, 'innerHTML').set.call(root,
            {toString() {conversions++; throw sentinel;}}); return false; }
          catch (error) {return error === sentinel && conversions === 1;}
        }));
      withSample(owner, flavor, 'open', value => {
        const get = Object.getOwnPropertyDescriptor(callee.Text.prototype, 'assignedSlot').get;
        const seen = [];
        for (const [badName, receiver] of invalidValues(owner, value, seen).filter(([name]) => name !== 'text'))
          check(`${label} Text.assignedSlot rejects ${badName}`, () => {
            seen.length = 0;
            return typeError(callee, () => Reflect.apply(get, receiver, [])) && seen.length === 0;
          });
        check(`${label} Text.assignedSlot accepts genuine Text`, () => Reflect.apply(get, value.text, []) === null);
      });
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
