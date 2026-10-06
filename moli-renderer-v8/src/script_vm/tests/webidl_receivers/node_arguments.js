(() => {
  const rows = [];
  const other = document.getElementById('child').contentWindow;
  const contexts = [
    ['main', document], ['iframe', other.document],
    ['windowless', document.implementation.createHTMLDocument('')],
    ['parsed-html', new DOMParser().parseFromString('<!doctype html><p>x</p>', 'text/html')],
    ['xml', document.implementation.createDocument('urn:test', 'root')]
  ];
  function record(name, expected, run, ErrorClass = TypeError, reads = () => 0) {
    let value, error;
    try { value = run(); } catch (e) { error = e; }
    const actual = error ? {error: error.name, correctRealm: error instanceof ErrorClass} : {value};
    const pass = expected === 'TypeError' ? !!error && error instanceof ErrorClass && reads() === 0
      : !error && value === expected && reads() === 0;
    rows.push({name, expected, actual, reads: reads(), pass});
  }
  for (const [context, d] of contexts) {
    for (const [kind, receiver] of [['element', d.createElement('div')], ['select', d.createElement('select')], ['text', d.createTextNode('x')], ['document', d]]) {
      for (const method of ['contains', 'isSameNode', 'isEqualNode', 'compareDocumentPosition', 'removeChild']) {
        for (const [surface, fn, ErrorClass] of [
          ['own', receiver[method], context === 'iframe' ? other.TypeError : TypeError],
          ['top-prototype', Node.prototype[method], TypeError],
          ['iframe-prototype', other.Node.prototype[method], other.TypeError]
        ]) {
          const prefix = context + '/' + kind + '/' + method + '/' + surface;
          record(prefix+'/missing', 'TypeError', () => Reflect.apply(fn, receiver, []), ErrorClass);
          for (const [label, value] of [['null', null], ['undefined', undefined]]) {
            record(prefix+'/'+label, method === 'compareDocumentPosition' || method === 'removeChild' ? 'TypeError' : false,
              () => Reflect.apply(fn, receiver, [value]), ErrorClass);
          }
          let reads = 0;
          const handler = {get() { reads++; throw new Error('author get'); }, getPrototypeOf() { reads++; throw new Error('author prototype'); }};
          const revoked = Proxy.revocable(receiver, {}); revoked.revoke();
          const lookalike = {get nodeType() { reads++; return 1; }, get parentNode() { reads++; return receiver; }};
          const invalids = [['object', {}], ['number', 1], ['string', 'x'], ['symbol', Symbol('x')], ['bigint', 1n],
            ['prototype', Object.create(Node.prototype)], ['inherited-instance', Object.create(receiver)],
            ['proxy', new Proxy(receiver, handler)], ['revoked', revoked.proxy], ['lookalike', lookalike]];
          for (const [label, value] of invalids) {
            reads = 0;
            record(prefix+'/'+label, 'TypeError', () => Reflect.apply(fn, receiver, [value]), ErrorClass, () => reads);
          }
          if (method !== 'removeChild') {
            record(prefix+'/self', method === 'compareDocumentPosition' ? 0 : true, () => Reflect.apply(fn, receiver, [receiver]), ErrorClass);
            record(prefix+'/foreign-node', method === 'compareDocumentPosition' ? true : false,
              () => { const r = Reflect.apply(fn, receiver, [other.document.createComment('foreign')]); return method === 'compareDocumentPosition' ? (r & 33) === 33 : r; }, ErrorClass);
          }
        }
      }
    }
    const attr = d.createAttribute('x');
    for (const [surface, fn, ErrorClass] of [['own', attr.isSameNode, context === 'iframe' ? other.TypeError : TypeError], ['top-prototype', Node.prototype.isSameNode, TypeError], ['iframe-prototype', other.Node.prototype.isSameNode, other.TypeError]]) {
      record(context+'/attr/isSameNode/'+surface+'/self', true, () => Reflect.apply(fn, attr, [attr]), ErrorClass);
      record(context+'/attr/isSameNode/'+surface+'/null', false, () => Reflect.apply(fn, attr, [null]), ErrorClass);
      record(context+'/attr/isSameNode/'+surface+'/invalid', 'TypeError', () => Reflect.apply(fn, attr, [{}]), ErrorClass);
      record(context+'/attr/isSameNode/'+surface+'/missing', 'TypeError', () => Reflect.apply(fn, attr, []), ErrorClass);
    }
  }
  globalThis.__nodeArgumentResults = {total: rows.length, passed: rows.filter(r => r.pass).length, failures: rows.filter(r => !r.pass), rows};
  return rows.every(r => r.pass);
})()
