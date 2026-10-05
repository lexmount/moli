(async () => {
  const checks = [];
  const run = (name, callback) => {
    try {checks.push({name, passed: !!callback(), detail: null});}
    catch (error) {checks.push({name, passed: false, detail: String(error)});}
  };
  const caught = callback => {try {callback();} catch (error) {return error;}};
  const frame = document.querySelector('iframe');
  if (frame.contentDocument.readyState !== 'complete') await new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const other = frame.contentWindow, ns = 'http://www.w3.org/2000/svg';
  const cases = [
    ['feFuncR', 'SVGComponentTransferFunctionElement', 'tableValues'],
    ['feFuncG', 'SVGComponentTransferFunctionElement', 'tableValues'],
    ['feFuncB', 'SVGComponentTransferFunctionElement', 'tableValues'],
    ['feFuncA', 'SVGComponentTransferFunctionElement', 'tableValues'],
    ['feColorMatrix', 'SVGFEColorMatrixElement', 'values'],
    ['feConvolveMatrix', 'SVGFEConvolveMatrixElement', 'kernelMatrix'],
    ['text', 'SVGTextPositioningElement', 'rotate'],
  ];
  const values = list => Array.from({length: list.numberOfItems}, (_, i) => list.getItem(i).value);
  const same = (list, expected) => JSON.stringify(values(list)) === JSON.stringify(expected);
  for (const w of [window, other]) {
    const realm = w === window ? 'main' : 'child';
    const documents = [w.document, w.document.implementation.createHTMLDocument(''),
      new w.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')];
    for (const [tag, owner, attribute] of cases) {
      const getter = Object.getOwnPropertyDescriptor(w[owner].prototype, attribute)?.get;
      run(realm + '/' + tag + '/descriptor', () => {
        const d = Object.getOwnPropertyDescriptor(w[owner].prototype, attribute);
        return d.enumerable && d.configurable && d.set === undefined && d.get.name === 'get ' + attribute && d.get.length === 0;
      });
      for (const [docIndex, doc] of documents.entries()) {
        const label = realm + '/' + tag + '/' + docIndex;
        const create = raw => {
          const e = doc.createElementNS(ns, tag);
          if (raw !== undefined) e.setAttribute(attribute, raw);
          return e;
        };
        const number = value => {const n = w.document.createElementNS(ns, 'svg').createSVGNumber();n.value = value;return n;};
        run(label + '/empty-native-cache', () => {
          const e = create(), a = getter.call(e);
          return a === getter.call(e) && Object.getPrototypeOf(a) === w.SVGAnimatedNumberList.prototype &&
            Object.getPrototypeOf(a.baseVal) === w.SVGNumberList.prototype && a.baseVal === a.baseVal && a.animVal === a.animVal &&
            a.baseVal !== a.animVal && a.baseVal.length === 0 && a.animVal.length === 0 && !e.hasAttribute(attribute);
        });
        const e = create(), revoked = Proxy.revocable(e, {});revoked.revoke();
        let traps = 0;
        for (const [index, bad] of [{}, w[owner].prototype, Object.create(e), new Proxy(e, {get() {traps++;throw 42;}}),
          revoked.proxy, doc.createElementNS(ns, 'feFlood'), doc.createElement(tag), doc.createElementNS('urn:wrong', tag)].entries())
          run(label + '/receiver/' + index, () => typeof getter === 'function' && caught(() => getter.call(bad)) instanceof w.TypeError && traps === 0);
        run(label + '/parse-and-retained-live-lists', () => {
          const e = create('.5, -2 3e1'), a = e[attribute], base = a.baseVal, anim = a.animVal;
          if (!same(base, [.5,-2,30]) || !same(anim, [.5,-2,30]) || base[1] !== base.getItem(1)) return false;
          const first = base.getItem(0), animatedFirst = anim.getItem(0);
          e.setAttribute(attribute, '7 8 9 10');
          return first.value === 7 && animatedFirst.value === 7 && same(base, [7,8,9,10]) && same(anim, [7,8,9,10]) &&
            base.getItem(0) === first && anim.getItem(0) === animatedFirst && Object.getPrototypeOf(anim.getItem(3)) === w.SVGNumber.prototype;
        });
        run(label + '/removed-items-detach', () => {
          const e = create('1 2'), a = e[attribute], old = a.baseVal.getItem(1), oldAnim = a.animVal.getItem(1);
          e.removeAttribute(attribute);
          if (a.baseVal.length !== 0 || a.animVal.length !== 0) return false;
          old.value = 3;oldAnim.value = 4;
          return old.value === 3 && oldAnim.value === 4 && !e.hasAttribute(attribute);
        });
        run(label + '/invalid-list-empty', () => {
          const e = create('1 , , 2');return same(e[attribute].baseVal, []) && same(e[attribute].animVal, []);
        });
        run(label + '/editable-base-and-readonly-anim-item', () => {
          const e = create('1 2'), a = e[attribute], first = a.baseVal.getItem(0), animated = a.animVal.getItem(0);
          first.value = 3;
          const error = caught(() => {animated.value = 7;});
          return same(a.baseVal, [3,2]) && animated.value === 3 && error?.name === 'NoModificationAllowedError' &&
            same(a.animVal, [3,2]) && values(a.baseVal)[0] === 3;
        });
        for (const method of ['initialize','insertItemBefore','replaceItem','appendItem']) run(label + '/operation/' + method, () => {
          const e = create('1 2'), base = e[attribute].baseVal, n = number(7);
          const result = method === 'insertItemBefore' || method === 'replaceItem' ? base[method](n, 1) : base[method](n);
          const expected = {initialize:[7],insertItemBefore:[1,7,2],replaceItem:[1,7],appendItem:[1,2,7]}[method];
          return result === n && same(base, expected) && same(e[attribute].animVal, expected);
        });
        run(label + '/indexed-replace-remove-clear', () => {
          const e = create('1 2 3'), base = e[attribute].baseVal, old = base.getItem(1), n = number(7);
          base[1] = n;
          if (base.getItem(1) !== n || !same(base, [1,7,3])) return false;
          old.value = 5;
          if (!same(base, [1,7,3]) || base.removeItem(1) !== n) return false;
          n.value = 8;
          if (!same(base, [1,3])) return false;
          base.clear();return same(base, []) && same(e[attribute].animVal, []) && e.hasAttribute(attribute);
        });
        run(label + '/attached-item-copied-between-lists', () => {
          const a = create('1 2'), b = create('3'), original = a[attribute].baseVal.getItem(0), copied = b[attribute].baseVal.appendItem(original);
          if (copied === original || copied.value !== original.value) return false;
          copied.value = 7;
          return same(a[attribute].baseVal, [1,2]) && same(b[attribute].baseVal, [3,7]);
        });
        run(label + '/initialize-detaches-before-copy', () => {
          const e = create('1 2'), base = e[attribute].baseVal, n = base.getItem(1);
          return base.initialize(n) === n && same(base, [2]);
        });
        run(label + '/readonly-item-copy-becomes-writable', () => {
          const a = create('1'), b = create('2'), original = a[attribute].animVal.getItem(0), copied = b[attribute].baseVal.appendItem(original);
          copied.value = 7;return original !== copied && original.value === 1 && same(b[attribute].baseVal, [2,7]);
        });
        run(label + '/list-conversion-before-body-read', () => {
          const e = create('1 2'), base = e[attribute].baseVal;
          const item = base.getItem({valueOf() {e.setAttribute(attribute, '9');return 0;}});
          return item.value === 9 && base.length === 1;
        });
        run(label + '/readonly-conversion-before-body', () => {
          const e = create('1'), anim = e[attribute].animVal, sentinel = {}, n = number(2);
          return caught(() => anim.insertItemBefore(n, {valueOf() {throw sentinel;}})) === sentinel && same(anim, [1]);
        });
        for (const method of ['clear','initialize','insertItemBefore','replaceItem','removeItem','appendItem'])
          run(label + '/readonly-operation/' + method, () => {
            const e = create('1'), anim = e[attribute].animVal, n = number(2);
            const args = {clear:[],initialize:[n],insertItemBefore:[n,0],replaceItem:[n,0],removeItem:[0],appendItem:[n]}[method];
            return caught(() => anim[method](...args))?.name === 'NoModificationAllowedError' && same(anim, [1]);
          });
        run(label + '/brand-before-conversion', () => {
          const e = create('1'), base = e[attribute].baseVal;
          let conversions = 0, traps = 0;
          const receiver = new Proxy(base, {get() {traps++;throw 42;}});
          const error = caught(() => w.SVGNumberList.prototype.getItem.call(receiver, {valueOf() {conversions++;return 0;}}));
          return error instanceof w.TypeError && conversions === 0 && traps === 0;
        });
        run(label + '/interface-argument-before-index', () => {
          const e = create('1'), base = e[attribute].baseVal;let conversions = 0;
          const error = caught(() => base.insertItemBefore({}, {valueOf() {conversions++;return 0;}}));
          return error instanceof w.TypeError && conversions === 0 && same(base,[1]);
        });
        run(label + '/replace-reads-after-index-conversion', () => {
          const e = create('1 2'), base = e[attribute].baseVal, n = number(7);
          base.replaceItem(n,{valueOf() {e.setAttribute(attribute,'9 8');return 0;}});
          return same(base,[7,8]);
        });
        run(label + '/invalid-item-atomic', () => {
          const e = create('1 2'), base = e[attribute].baseVal, n = number(3);
          for (const value of [{},Object.create(n),new Proxy(n,{})]) if (!(caught(() => base.initialize(value)) instanceof w.TypeError) || !same(base,[1,2])) return false;
          return true;
        });
        run(label + '/float-rounding', () => {
          const n = number(1/3), e = create('0.3333333333333333');
          return n.value === Math.fround(1/3) && e[attribute].baseVal.getItem(0).value === Math.fround(1/3);
        });
        run(label + '/float-errors-atomic', () => {
          const n = number(1);
          for (const value of [NaN,Infinity,-Infinity,1e300]) if (!(caught(() => {n.value=value;}) instanceof w.TypeError) || n.value!==1) return false;
          return true;
        });
        run(label + '/attribute-float-range', () => {
          const e = create('1'), a = e[attribute];
          for (const raw of ['1e300','1e40','3.5e38']) {
            e.setAttribute(attribute, raw);
            if (!same(a.baseVal,[]) || !same(a.animVal,[])) return false;
          }
          e.setAttribute(attribute,'3.4e38');
          if (!same(a.baseVal,[Math.fround(3.4e38)]) || !same(a.animVal,[Math.fround(3.4e38)])) return false;
          e.setAttribute(attribute,'1e-100');
          return same(a.baseVal,[0]) && same(a.animVal,[0]);
        });
        run(label + '/readonly-number-conversion-order', () => {
          const e = create('1'), n = e[attribute].animVal.getItem(0), sentinel = {};
          return caught(() => {n.value={valueOf() {throw sentinel;}};}) === sentinel && n.value === 1;
        });
        run(label + '/private-cache-and-state', () => {
          const e = create('1'), a = getter.call(e), base = a.baseVal, method = w.SVGNumberList.prototype.getItem;
          e.__moliSvgFilterNumberListValues = 'forged';Object.setPrototypeOf(e,null);Object.freeze(e);
          Object.setPrototypeOf(base,null);
          return getter.call(e) === a && method.call(base,0).value === 1;
        });
      }
      run(realm + '/' + tag + '/borrowed-accessor-owner-realm', () => {
        const ownerWindow = w === window ? other : window, e = ownerWindow.document.createElementNS(ns,tag);
        e.setAttribute(attribute,'1');const a = getter.call(e);
        return Object.getPrototypeOf(a) === ownerWindow.SVGAnimatedNumberList.prototype &&
          Object.getPrototypeOf(a.baseVal.getItem(0)) === ownerWindow.SVGNumber.prototype && a === e[attribute];
      });
      run(realm + '/' + tag + '/copied-item-recipient-realm', () => {
        const sourceWindow = w === window ? other : window;
        const source = sourceWindow.document.createElementNS(ns,tag), recipient = w.document.createElementNS(ns,tag);
        source.setAttribute(attribute,'7');
        const original = source[attribute].baseVal.getItem(0);
        const copied = w.SVGNumberList.prototype.appendItem.call(recipient[attribute].baseVal, original);
        return copied !== original && copied.value === 7 && Object.getPrototypeOf(copied) === w.SVGNumber.prototype;
      });
    }
  }
  run('retained-list-after-iframe-removal', () => {
    const f = document.createElement('iframe');document.body.appendChild(f);
    const e = f.contentDocument.createElementNS(ns,'feColorMatrix'), a = e.values, svg = f.contentDocument.createElementNS(ns,'svg'), n = svg.createSVGNumber();
    f.remove();n.value=7;a.baseVal.appendItem(n);return e.getAttribute('values') === '7' && same(a.animVal,[7]);
  });
  globalThis.__uiEventResults = {complete: true, checks, total: checks.length, passed: checks.filter(row => row.passed).length};
  return true;
})()
