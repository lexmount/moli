(() => {
  const checks = [], ns = 'http://www.w3.org/2000/svg';
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const caught = fn => { try { fn(); return null; } catch (error) { return error; } };
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  try {
    for (const [realm, owner] of [window, frame.contentWindow, popup].entries()) {
      const documentWithRoot = () => owner.document.implementation.createDocument(ns, 'svg');
      for (const operation of ['replaceChild', 'replaceChildren', 'replaceWith', 'fragment']) {
        check(realm + '/SVG document default magnification/' + operation, () => {
          const doc = documentWithRoot(), old = doc.documentElement;
          const replacement = doc.createElementNS(ns, 'svg');
          const point = replacement.currentTranslate;
          replacement.currentScale = 7; point.x = 9; point.z = 10;
          if (operation === 'replaceChild') doc.replaceChild(replacement, old);
          if (operation === 'replaceChildren') doc.replaceChildren(replacement);
          if (operation === 'replaceWith') old.replaceWith(replacement);
          if (operation === 'fragment') {
            const fragment = doc.createDocumentFragment(); fragment.appendChild(replacement);
            doc.replaceChild(fragment, old);
          }
          return doc.documentElement === replacement && replacement.currentScale === 1 &&
            replacement.currentTranslate === point && point.x === 0 && point.y === 0 && point.z === 10;
        });
      }
      check(realm + '/translation preserves negative zero in all coordinates', () => {
        for (const field of ['x', 'y', 'z', 'w']) {
          const svg = owner.document.createElementNS(ns, 'svg'), point = svg.currentTranslate;
          point[field] = -0;
          if (!Object.is(point[field], -0) || !Object.is(svg.currentTranslate[field], -0)) return false;
          const clone = owner.structuredClone(point);
          if (!Object.is(clone[field], -0)) return false;
        }
        return true;
      });
      check(realm + '/readonly translation retains author conversion exception', () => {
        const svg = owner.document.createElementNS(ns, 'svg'), inner = owner.document.createElementNS(ns, 'svg');
        svg.appendChild(inner); const point = inner.currentTranslate, sentinel = {};
        return caught(() => { point.x = {valueOf() { throw sentinel; }}; }) === sentinel && point.x === 0;
      });
      check(realm + '/failed root replacement preserves state', () => {
        const doc = documentWithRoot(), svg = doc.documentElement, point = svg.currentTranslate;
        svg.currentScale = 5; point.x = 6;
        const error = caught(() => doc.replaceChild(doc.createElementNS(ns, 'svg'), doc.createElementNS(ns, 'svg')));
        return error?.name === 'NotFoundError' && doc.documentElement === svg && svg.currentScale === 5 && point.x === 6;
      });
      check(realm + '/outermost decision does not read author DOM properties', () => {
        const svg = owner.document.createElementNS(ns, 'svg'); let calls = 0;
        for (const field of ['parentNode', 'ownerSVGElement', 'isConnected', 'namespaceURI', 'localName']) {
          Object.defineProperty(svg, field, {get() { calls++; throw Error(field); }});
        }
        svg.currentScale = 4; svg.currentTranslate.x = 7;
        return svg.currentScale === 4 && svg.currentTranslate.x === 7 && calls === 0;
      });
      check(realm + '/retained point updates before rereading replacement root', () => {
        const doc = documentWithRoot(), old = doc.documentElement;
        old.currentScale = 2; old.currentTranslate.x = 5;
        const replacement = doc.createElementNS(ns, 'svg'), retained = replacement.currentTranslate;
        retained.x = 30; doc.replaceChild(replacement, old);
        return retained.x === 5 && replacement.currentScale === 2;
      });
      check(realm + '/point matrixTransform observes nesting reset', () => {
        const svg = owner.document.createElementNS(ns, 'svg'), outer = owner.document.createElementNS(ns, 'svg');
        const point = svg.currentTranslate; point.x = 7; point.y = 8;
        outer.appendChild(svg);
        const transformed = owner.DOMPointReadOnly.prototype.matrixTransform.call(point);
        return transformed.x === 0 && transformed.y === 0 && transformed !== point;
      });
      check(realm + '/unrestricted panning survives root replacement', () => {
        const doc = documentWithRoot(), old = doc.documentElement;
        old.currentTranslate.x = NaN; old.currentTranslate.y = Infinity;
        const replacement = doc.createElementNS(ns, 'svg'); doc.replaceChild(replacement, old);
        return Number.isNaN(replacement.currentTranslate.x) && replacement.currentTranslate.y === Infinity && old.currentTranslate.x === 0;
      });
      check(realm + '/root replacement updates only x and y of retained point', () => {
        const doc = documentWithRoot(), old = doc.documentElement;
        old.currentScale = 2; old.currentTranslate.x = 5; old.currentTranslate.y = 6;
        const replacement = doc.createElementNS(ns, 'svg'), point = replacement.currentTranslate;
        point.z = 7; point.w = 8; doc.replaceChild(replacement, old);
        return point.x === 5 && point.y === 6 && point.z === 7 && point.w === 8 && replacement.currentScale === 2;
      });
    }
  } finally { frame.remove(); popup?.close(); }
  globalThis.__svgUserTransformDocumentResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
