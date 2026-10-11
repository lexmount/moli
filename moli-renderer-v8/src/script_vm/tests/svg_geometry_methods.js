(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const child = document.querySelector('iframe').contentWindow;
  const ns = 'http://www.w3.org/2000/svg';
  const methods = [
    ['SVGGraphicsElement', 'getBBox', 0],
    ['SVGGraphicsElement', 'getCTM', 0],
    ['SVGGraphicsElement', 'getScreenCTM', 0],
    ['SVGGeometryElement', 'isPointInFill', 0],
    ['SVGGeometryElement', 'isPointInStroke', 0],
    ['SVGGeometryElement', 'getTotalLength', 0],
    ['SVGGeometryElement', 'getPointAtLength', 1],
  ];
  const documents = [];
  for (const [ownerName, realm] of [['top', window], ['child', child]]) {
    for (const [kind, doc] of [
      ['live', realm.document],
      ['html', realm.document.implementation.createHTMLDocument('')],
      ['xml', realm.document.implementation.createDocument(ns, 'svg')],
      ['parser', new realm.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')],
    ]) {
      const root = doc.createElementNS(ns, 'svg');
      const rect = doc.createElementNS(ns, 'rect');
      rect.setAttribute('x', '2'); rect.setAttribute('y', '3');
      rect.setAttribute('width', '10'); rect.setAttribute('height', '10');
      rect.setAttribute('pathLength', '1');
      const group = doc.createElementNS(ns, 'g');
      root.append(rect, group);
      (doc.body || doc.documentElement).appendChild(root);
      documents.push({ownerName, kind, realm, doc, root, rect, group});
    }
  }
  for (const [calleeName, realm] of [['top', window], ['child', child]]) {
    for (const [name, member, length] of methods) {
      const method = realm[name].prototype[member];
      check(calleeName + '.' + name + '.' + member + ' descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(realm[name].prototype, member);
        return typeof method === 'function' && method.name === member && method.length === length &&
          d.enumerable && d.writable && d.configurable;
      });
      for (const {ownerName, kind, doc, rect, group} of documents) {
        const prefix = calleeName + '/' + ownerName + '/' + kind + '/' + member;
        const invoke = (receiver, args = member === 'getPointAtLength' ? [5] : []) =>
          Reflect.apply(method, receiver, args);
        check(prefix + ' genuine', () => {
          const value = invoke(rect);
          if (member === 'getBBox') return ['x','y','width','height'].every(key => Number.isFinite(value[key]));
          if (member === 'getCTM' || member === 'getScreenCTM') return value === null || typeof value.a === 'number';
          if (member === 'getTotalLength') return value === 40;
          if (member === 'getPointAtLength') return value.x === 7 && value.y === 3;
          return typeof value === 'boolean';
        });
        if (name === 'SVGGraphicsElement') {
          check(prefix + ' graphics group', () => { invoke(group); return true; });
        }
        let traps = 0;
        const author = new Proxy(rect, {
          get() { traps++; throw 42; },
          getPrototypeOf() { traps++; throw 42; },
        });
        const revoked = Proxy.revocable(rect, {}); revoked.revoke();
        const wrong = doc.createElementNS(ns, name === 'SVGGeometryElement' ? 'g' : 'feBlend');
        const invalid = [
          ['undefined', undefined], ['null', null], ['boolean', true], ['number', 0],
          ['string', ''], ['bigint', 0n], ['symbol', Symbol()], ['object', {}],
          ['null-prototype', Object.create(null)], ['prototype', realm[name].prototype],
          ['forged', Object.create(realm[name].prototype)], ['inherited', Object.create(rect)],
          ['proxy', author], ['revoked', revoked.proxy], ['document', doc],
          ['html', doc.createElement('div')], ['wrong-svg', wrong],
        ];
        for (const [kind, receiver] of invalid) {
          check(prefix + ' reject ' + kind + ' before conversion', () => {
            let conversions = 0;
            const argument = member === 'getPointAtLength'
              ? {valueOf() { conversions++; return 5; }}
              : member === 'getBBox'
                ? {get fill() { conversions++; return true; }}
                : {get x() { conversions++; return 5; }};
            let error;
            try { invoke(receiver, [argument]); } catch (caught) { error = caught; }
            return error !== undefined && Object.getPrototypeOf(error) === realm.TypeError.prototype &&
              conversions === 0 && traps === 0;
          });
        }
        if (member === 'isPointInFill' || member === 'isPointInStroke' || member === 'getPointAtLength') {
          check(prefix + ' preserve conversion exception', () => {
            const sentinel = {}; let conversions = 0;
            const arg = member === 'getPointAtLength'
              ? {valueOf() { conversions++; throw sentinel; }}
              : {get x() { conversions++; throw sentinel; }};
            let error; try { invoke(rect, [arg]); } catch (caught) { error = caught; }
            return error === sentinel && conversions === 1;
          });
        }
      }
    }
    for (const {ownerName, kind, rect} of documents) {
      const method = realm.SVGGeometryElement.prototype.isPointInFill;
      for (const fill of ['none', 'transparent', 'red']) {
        rect.setAttribute('fill', fill);
        check(calleeName + '/' + ownerName + '/' + kind + ' geometric fill ' + fill,
          () => Reflect.apply(method, rect, [{x: 5, y: 5}]) === true);
      }
      rect.removeAttribute('fill');
    }
  }
  globalThis.__uiEventResults = {
    complete: true, checks, total: checks.length,
    passed: checks.filter(row => row.passed).length,
  };
  return true;
})()
