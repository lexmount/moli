(() => {
  const rows = [];
  function check(name, test) {
    try {
      test();
      rows.push({name, passed: true});
    } catch (error) {
      rows.push({name, passed: false, error: String(error)});
    }
  }
  function assert(condition, message) {
    if (!condition) throw new Error(message);
  }
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const other = frame.contentWindow;
  const detached = document.implementation.createHTMLDocument('');
  const element = document.createElement('div');
  const foreign = other.document.createElement('div');
  const nativeProxy = detached.createElement('select');
  globalThis.__observerNativeElement = nativeProxy;
  let propertyReads = 0;
  const poison = () => { propertyReads++; throw new Error('public node property read'); };
  for (const target of [element, foreign, nativeProxy]) {
    Object.defineProperty(target, 'nodeType', {get: poison, configurable: true});
  }
  const text = document.createTextNode('text');
  Object.defineProperty(text, 'nodeType', {value: 1});
  const revoked = Proxy.revocable(element, {});
  revoked.revoke();
  const forged = {get nodeType() { return poison(); }};
  const authorProxy = new Proxy(element, {
    get: poison, getPrototypeOf: poison, has: poison,
  });
  const valid = [
    ['element with poisoned nodeType', element],
    ['cross-realm element with poisoned nodeType', foreign],
    ['windowless div', detached.createElement('div')],
    ['windowless select with poisoned nodeType', nativeProxy],
    ['SVG element', document.createElementNS('http://www.w3.org/2000/svg', 'svg')],
  ];
  const invalid = [
    ['undefined', undefined], ['null', null], ['number', 1], ['string', 'div'],
    ['plain object', {nodeType: 1}], ['poisoned plain object', forged],
    ['inherited instance', Object.create(element)],
    ['forged prototype', Object.create(Element.prototype)],
    ['interface prototype', Element.prototype], ['spoofed Text', text],
    ['Document', document], ['DocumentFragment', document.createDocumentFragment()],
    ['author Proxy', authorProxy], ['revoked Proxy', revoked.proxy],
    ['author Proxy around native select', new Proxy(nativeProxy, {get: poison})],
  ];
  for (const [realmName, realm] of [['main', window], ['child', other]]) {
    for (const api of ['ResizeObserver', 'IntersectionObserver']) {
      for (const operation of ['observe', 'unobserve']) {
        for (const [name, target] of valid) {
          check(`${realmName} ${api}.${operation}: ${name}`, () => {
            const observer = new realm[api](() => {});
            let optionsReads = 0;
            const before = propertyReads;
            try {
              observer[operation](target, {get box() { optionsReads++; return 'content-box'; }});
              assert(propertyReads === before, 'must not read nodeType or run Proxy traps');
              assert(optionsReads === (api === 'ResizeObserver' && operation === 'observe' ? 1 : 0),
                     'convert options exactly once after Element');
            } finally { observer.disconnect(); }
          });
        }
        for (const [name, target] of invalid) {
          check(`${realmName} ${api}.${operation}: rejects ${name}`, () => {
            const observer = new realm[api](() => {});
            const before = propertyReads;
            let optionsReads = 0;
            let caught;
            try {
              observer[operation](target, {get box() { optionsReads++; return 'content-box'; }});
            } catch (error) { caught = error; }
            finally { observer.disconnect(); }
            assert(caught instanceof realm.TypeError, 'must throw callee-realm TypeError');
            assert(propertyReads === before, 'must not inspect author properties or Proxy traps');
            assert(optionsReads === 0, 'reject Element before converting options');
          });
        }
      }
    }
    for (const [name, root] of [...valid, ['document', document], ['child document', other.document],
                                ['windowless document', detached], ['null', null]]) {
      check(`${realmName} IntersectionObserver.root: ${name}`, () => {
        const observer = new realm.IntersectionObserver(() => {}, {root});
        try { assert(observer.root === root, 'root identity must be preserved'); }
        finally { observer.disconnect(); }
      });
    }
    for (const [name, root] of invalid.filter(([name]) => !['Document', 'null', 'undefined'].includes(name))) {
      check(`${realmName} IntersectionObserver.root: rejects ${name}`, () => {
        const before = propertyReads;
        let laterReads = 0;
        let caught;
        try {
          new realm.IntersectionObserver(() => {}, {
            root, get rootMargin() { laterReads++; return '0px'; },
          });
        } catch (error) { caught = error; }
        assert(caught instanceof realm.TypeError, 'invalid root must throw callee-realm TypeError');
        assert(propertyReads === before && laterReads === 0, 'root conversion precedes later dictionary members');
      });
    }
    check(`${realmName} ResizeObserver: options exception identity`, () => {
      const observer = new realm.ResizeObserver(() => {});
      const sentinel = {};
      let caught;
      try { observer.observe(element, {get box() { throw sentinel; }}); }
      catch (error) { caught = error; }
      finally { observer.disconnect(); }
      assert(caught === sentinel, 'valid Element must reach options and propagate its exception');
    });
  }
  for (const target of [element, foreign, nativeProxy]) delete target.nodeType;
  frame.remove();
  globalThis.__observerElementResults = rows;
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
