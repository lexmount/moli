(() => {
  const checks = [];
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const caught = fn => { try { fn(); return null; } catch (error) { return error; } };
  const ns = 'http://www.w3.org/2000/svg';
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  const resources = [frame];
  try {
    for (const [realm, owner] of [window, frame.contentWindow, popup].entries()) {
      const root = doc => doc.createElementNS(ns, 'svg');
      const live = root(owner.document); owner.document.body.appendChild(live); resources.push(live);
      const detached = root(owner.document);
      const windowless = root(owner.document.implementation.createHTMLDocument(''));
      const xml = owner.document.implementation.createDocument(ns, 'svg').documentElement;
      const parsed = new owner.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml').documentElement;
      const scale = Object.getOwnPropertyDescriptor(owner.SVGSVGElement.prototype, 'currentScale');
      const translate = Object.getOwnPropertyDescriptor(owner.SVGSVGElement.prototype, 'currentTranslate');
      for (const [kind, svg] of [['live', live], ['detached', detached], ['windowless', windowless], ['XML', xml], ['parsed SVG', parsed]]) {
        const prefix = `${realm}/${kind}`;
        check(prefix + '/initial native state', () => svg.currentScale === 1 && svg.currentTranslate.x === 0 && svg.currentTranslate.y === 0);
        check(prefix + '/SameObject and owner-realm DOMPoint', () => svg.currentTranslate === svg.currentTranslate &&
          svg.currentTranslate instanceof owner.DOMPoint && svg.currentTranslate instanceof owner.DOMPointReadOnly &&
          svg.currentTranslate.z === 0 && svg.currentTranslate.w === 1);
        check(prefix + '/IDL descriptors', () => scale?.enumerable === true && scale.configurable &&
          typeof scale.get === 'function' && typeof scale.set === 'function' && translate?.enumerable === true &&
          translate.configurable && typeof translate.get === 'function' && translate.set === undefined &&
          !Object.hasOwn(svg, 'currentScale') && !Object.hasOwn(svg, 'currentTranslate'));
        for (const [name, value, expected] of [['fraction', 1.1, Math.fround(1.1)], ['zero', 0, 0], ['negative', -2, -2],
          ['negative zero', -0, -0], ['null', null, 0], ['boolean', true, 1], ['string', '2.5', 2.5]]) {
          check(prefix + '/restricted float/' + name, () => {
            scale.set.call(svg, value);
            return Object.is(svg.currentScale, expected);
          });
        }
        for (const [name, value] of [['NaN', NaN], ['Infinity', Infinity], ['negative Infinity', -Infinity],
          ['undefined', undefined], ['overflow', 1e100], ['Symbol', Symbol()], ['BigInt', 1n]]) {
          check(prefix + '/scale rejection/' + name, () => {
            scale.set.call(svg, 3);
            const error = caught(() => scale.set.call(svg, value));
            return error instanceof owner.TypeError && svg.currentScale === 3;
          });
        }
        for (const mode of ['valueOf', 'toPrimitive', 'getter']) {
          check(prefix + '/scale conversion exception/' + mode, () => {
            const sentinel = {}, calls = [];
            const value = mode === 'valueOf' ? {valueOf() { calls.push('valueOf'); throw sentinel; }} :
              mode === 'toPrimitive' ? {[Symbol.toPrimitive](hint) { calls.push(hint); throw sentinel; }} :
              {get valueOf() { calls.push('getter'); throw sentinel; }};
            const error = caught(() => scale.set.call(svg, value));
            return error === sentinel && calls.length === 1 && svg.currentScale === 3;
          });
        }
        const point = svg.currentTranslate;
        for (const field of ['x', 'y', 'z', 'w']) {
          const setter = Object.getOwnPropertyDescriptor(owner.DOMPoint.prototype, field)?.set;
          for (const [name, value] of [['double', 1.1], ['NaN', NaN], ['Infinity', Infinity]]) {
            check(prefix + '/DOMPoint/' + field + '/' + name, () => {
              setter.call(point, value);
              return Object.is(point[field], value) && svg.currentTranslate === point && Object.is(svg.currentTranslate[field], value);
            });
          }
          check(prefix + '/DOMPoint/' + field + '/conversion exception', () => {
            const sentinel = {}, before = point[field];
            const error = caught(() => setter.call(point, {valueOf() { throw sentinel; }}));
            return error === sentinel && Object.is(point[field], before);
          });
          check(prefix + '/DOMPoint/' + field + '/reentrant other coordinate', () => {
            const other = field === 'x' ? 'y' : 'x'; let calls = 0;
            setter.call(point, {valueOf() { calls++; point[other] = 42; return 7; }});
            return calls === 1 && point[field] === 7 && point[other] === 42;
          });
        }
        check(prefix + '/coordinate snapshots are independent', () => {
          point.x = 5; point.y = 6; point.z = 7; point.w = 8;
          const json = owner.DOMPointReadOnly.prototype.toJSON.call(point);
          const clone = owner.structuredClone(point);
          point.x = 9;
          return json.x === 5 && clone.x === 5 && point.x === 9 && json.y === 6 && clone.w === 8;
        });
        check(prefix + '/attributes do not change user transform', () => {
          scale.set.call(svg, 4); point.x = 10; point.y = 20;
          svg.setAttribute('transform', 'translate(100 200) scale(9)'); svg.setAttribute('viewBox', '1 2 3 4');
          return svg.currentScale === 4 && point.x === 10 && point.y === 20 && svg.currentTranslate === point;
        });
        check(prefix + '/clone and import reset user state', () => {
          const clone = svg.cloneNode(true), imported = owner.document.importNode(svg, true);
          return [clone, imported].every(copy => copy.currentScale === 1 && copy.currentTranslate.x === 0 &&
            copy.currentTranslate.y === 0 && copy.currentTranslate !== point && copy.getAttribute('transform') === svg.getAttribute('transform'));
        });
        check(prefix + '/readonly currentTranslate attribute', () => {
          let conversions = 0;
          const value = {valueOf() { conversions++; return 10; }};
          const error = caught(() => { 'use strict'; svg.currentTranslate = value; });
          return error?.name === 'TypeError' && conversions === 0 && svg.currentTranslate === point;
        });
        let traps = 0;
        const handler = {get() { traps++; throw Error('get trap'); }, getPrototypeOf() { traps++; throw Error('prototype trap'); }};
        const revoked = owner.Proxy.revocable(svg, handler); revoked.revoke();
        const invalid = [{}, Object.create(svg), Object.create(owner.SVGSVGElement.prototype), new owner.Proxy(svg, handler),
          revoked.proxy, owner.document, owner.document.createElementNS(ns, 'rect')];
        for (const [index, receiver] of invalid.entries()) {
          check(prefix + '/receiver ' + index + '/scale getter', () => caught(() => scale.get.call(receiver)) instanceof owner.TypeError);
          check(prefix + '/receiver ' + index + '/scale setter before conversion', () => {
            let conversions = 0;
            const error = caught(() => scale.set.call(receiver, {valueOf() { conversions++; return 2; }}));
            return error instanceof owner.TypeError && conversions === 0;
          });
          check(prefix + '/receiver ' + index + '/translation getter', () => caught(() => translate.get.call(receiver)) instanceof owner.TypeError);
        }
        check(prefix + '/native brand checks do not invoke Proxy traps', () => traps === 0);
      }

      const outer = root(owner.document), inner = root(owner.document);
      owner.document.body.appendChild(outer); resources.push(outer);
      outer.appendChild(inner);
      check(realm + '/nested scale ignores converted value', () => {
        let calls = 0; scale.set.call(inner, {valueOf() { calls++; return 3; }});
        return calls === 1 && inner.currentScale === 1 && outer.currentScale === 1;
      });
      check(realm + '/nested scale still rejects nonfinite conversion', () => caught(() => scale.set.call(inner, NaN)) instanceof owner.TypeError);
      for (const field of ['x', 'y', 'z', 'w']) {
        check(realm + '/nested DOMPoint/' + field + '/readonly after conversion', () => {
          let calls = 0; const p = inner.currentTranslate, before = p[field];
          const error = caught(() => { p[field] = {valueOf() { calls++; return 8; }}; });
          return calls === 1 && error instanceof owner.DOMException && error.name === 'NoModificationAllowedError' && p[field] === before;
        });
      }
      check(realm + '/nesting resets translation without an intervening getter', () => {
        const moving = root(owner.document); owner.document.body.appendChild(moving); resources.push(moving);
        const p = moving.currentTranslate; p.x = 11; p.y = 12; p.z = 13; p.w = 14;
        outer.appendChild(moving); owner.document.body.appendChild(moving);
        return moving.currentTranslate === p && p.x === 0 && p.y === 0 && p.z === 13 && p.w === 14;
      });
      check(realm + '/scale checks nesting after conversion', () => {
        const moving = root(owner.document); owner.document.body.appendChild(moving); scale.set.call(moving, 2);
        scale.set.call(moving, {valueOf() { outer.appendChild(moving); return 7; }});
        const nested = moving.currentScale === 1;
        owner.document.body.appendChild(moving); resources.push(moving);
        return nested && moving.currentScale === 2;
      });
      check(realm + '/point checks readonly after reentrant nesting', () => {
        const moving = root(owner.document); owner.document.body.appendChild(moving); resources.push(moving);
        const p = moving.currentTranslate; p.x = 10;
        const error = caught(() => { p.x = {valueOf() { outer.appendChild(moving); return 99; }}; });
        return error instanceof owner.DOMException && error.name === 'NoModificationAllowedError' && p.x === 0;
      });
      check(realm + '/point becomes writable during conversion', () => {
        const p = inner.currentTranslate;
        p.x = {valueOf() { owner.document.body.appendChild(inner); return 19; }}; resources.push(inner);
        return p.x === 19 && inner.currentTranslate === p;
      });
      check(realm + '/foreignObject starts an independent SVG fragment', () => {
        const foreign = owner.document.createElementNS(ns, 'foreignObject'), svg = root(owner.document);
        outer.appendChild(foreign); foreign.appendChild(svg);
        svg.currentScale = 5; svg.currentTranslate.x = 8;
        return svg.currentScale === 5 && svg.currentTranslate.x === 8 && outer.currentScale === 1;
      });
      check(realm + '/document root replacement transfers magnification and panning', () => {
        const doc = owner.document.implementation.createDocument(ns, 'svg'), before = doc.documentElement;
        const point = before.currentTranslate; before.currentScale = 6; point.x = 21; point.y = 22; point.z = 23;
        const replacement = root(doc); doc.replaceChild(replacement, before);
        return replacement.currentScale === 6 && replacement.currentTranslate.x === 21 && replacement.currentTranslate.y === 22 &&
          replacement.currentTranslate.z === 0 && point.x === 0 && point.y === 0 && point.z === 23;
      });
      check(realm + '/cloned document starts a new user transform', () => {
        const doc = owner.document.implementation.createDocument(ns, 'svg');
        doc.documentElement.currentScale = 4; doc.documentElement.currentTranslate.x = 12;
        const clone = doc.cloneNode(true);
        return clone.documentElement.currentScale === 1 && clone.documentElement.currentTranslate.x === 0;
      });
      check(realm + '/cross-realm first access retains owner realm and SameObject', () => {
        const svg = root(owner.document);
        const first = Object.getOwnPropertyDescriptor(SVGSVGElement.prototype, 'currentTranslate').get.call(svg);
        return first === svg.currentTranslate && Object.getPrototypeOf(first) === owner.DOMPoint.prototype;
      });
      check(realm + '/adoption preserves native values and point identity', () => {
        const svg = root(owner.document), point = svg.currentTranslate;
        svg.currentScale = 8; point.x = 31;
        document.adoptNode(svg); document.body.appendChild(svg); resources.push(svg);
        return svg.currentScale === 8 && point.x === 31 && svg.currentTranslate === point && svg.ownerDocument === document;
      });
    }
  } finally {
    for (const resource of resources) resource.remove();
    popup?.close();
  }
  globalThis.__svgUserTransformResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
