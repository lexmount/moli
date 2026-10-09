(() => {
  const checks = [], ns = 'http://www.w3.org/2000/svg';
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const near = (actual, expected) => Math.abs(actual - expected) < 1e-5;
  const realms = [window, document.querySelector('iframe').contentWindow];
  for (const [ownerIndex, owner] of realms.entries()) {
    const initial = owner.document.createElement('span');
    initial.style.fontSize = 'initial';
    owner.document.body.appendChild(initial);
    const initialFont = parseFloat(owner.getComputedStyle(initial).fontSize);
    initial.remove();
    const docs = [owner.document,
      owner.document.implementation.createHTMLDocument(''),
      owner.document.implementation.createDocument(ns, 'svg'),
      new owner.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')];
    for (const [docIndex, doc] of docs.entries()) {
      const factory = doc.createElementNS(ns, 'svg');
      factory.style.fontSize = '48px';
      (doc.body || doc.documentElement).appendChild(factory);
      for (const [calleeIndex, callee] of realms.entries()) {
        const prefix = `detached-length/${ownerIndex}/${docIndex}/${calleeIndex}`;
        const proto = callee.SVGLength.prototype;
        const value = Object.getOwnPropertyDescriptor(proto, 'value');
        const specified = Object.getOwnPropertyDescriptor(proto, 'valueInSpecifiedUnits');
        const string = Object.getOwnPropertyDescriptor(proto, 'valueAsString');
        const convert = proto.convertToSpecifiedUnits, assign = proto.newValueSpecifiedUnits;
        check(prefix + '/initial-font', () => initialFont > 0 && Number.isFinite(initialFont));
        for (const [suffix, unit, basis] of [['em', proto.SVG_LENGTHTYPE_EMS, initialFont],
                                            ['ex', proto.SVG_LENGTHTYPE_EXS, initialFont / 2]]) {
          const length = factory.createSVGLength();
          check(prefix + '/' + suffix + '/parse', () => {
            string.set.call(length, '2' + suffix);
            return length.unitType === unit && specified.get.call(length) === 2;
          });
          check(prefix + '/' + suffix + '/value', () => near(value.get.call(length), 2 * basis));
          check(prefix + '/' + suffix + '/specified-setter', () => {
            specified.set.call(length, 4.25);
            return near(value.get.call(length), 4.25 * basis) && length.valueAsString === '4.25' + suffix;
          });
          check(prefix + '/' + suffix + '/absolute-setter', () => {
            value.set.call(length, 3 * basis);
            return length.unitType === unit && near(specified.get.call(length), 3) && near(length.value, 3 * basis);
          });
          check(prefix + '/' + suffix + '/assign', () => {
            assign.call(length, unit, -1.5);
            return near(length.value, -1.5 * basis) && length.valueInSpecifiedUnits === -1.5;
          });
          check(prefix + '/' + suffix + '/convert-px', () => {
            convert.call(length, proto.SVG_LENGTHTYPE_PX);
            return length.unitType === proto.SVG_LENGTHTYPE_PX && near(length.valueInSpecifiedUnits, -1.5 * basis);
          });
          check(prefix + '/' + suffix + '/convert-back', () => {
            convert.call(length, unit);
            return length.unitType === unit && near(length.valueInSpecifiedUnits, -1.5) && near(length.value, -1.5 * basis);
          });
          check(prefix + '/' + suffix + '/factory-font-mutation', () => {
            factory.style.fontSize = '96px';
            const result = near(length.value, -1.5 * basis);
            factory.style.fontSize = '48px';
            return result;
          });
          check(prefix + '/' + suffix + '/factory-removal', () => {
            factory.remove();
            const result = near(length.value, -1.5 * basis);
            (doc.body || doc.documentElement).appendChild(factory);
            return result;
          });
          const fromPx = factory.createSVGLength();
          fromPx.valueAsString = '2px';
          check(prefix + '/' + suffix + '/from-px', () => {
            convert.call(fromPx, unit);
            return fromPx.unitType === unit && near(fromPx.valueInSpecifiedUnits, 2 / basis) && near(fromPx.value, 2);
          });
          check(prefix + '/' + suffix + '/round-trip', () => {
            convert.call(fromPx, proto.SVG_LENGTHTYPE_NUMBER);
            return fromPx.unitType === proto.SVG_LENGTHTYPE_NUMBER && near(fromPx.valueInSpecifiedUnits, 2) && near(fromPx.value, 2);
          });
        }
        const percent = factory.createSVGLength();
        check(prefix + '/percent-basis', () => {
          percent.valueAsString = '25%';
          convert.call(percent, proto.SVG_LENGTHTYPE_EMS);
          return near(percent.value, 25) && near(percent.valueInSpecifiedUnits, 25 / initialFont);
        });
        const real = factory.createSVGLength();
        let proxyTraps = 0;
        const revoked = callee.Proxy.revocable(real, {}); revoked.revoke();
        const invalid = [{}, Object.create(proto), Object.create(real),
          new callee.Proxy(real, {get() { proxyTraps++; throw 'unexpected proxy get'; }}), revoked.proxy];
        for (const [index, receiver] of invalid.entries()) {
          check(prefix + '/brand/' + index, () => {
            let conversions = 0, error;
            try { convert.call(receiver, {valueOf() { conversions++; return proto.SVG_LENGTHTYPE_EMS; }}); }
            catch (caught) { error = caught; }
            return error instanceof callee.TypeError && conversions === 0 && proxyTraps === 0;
          });
        }
        if (docIndex === 0) {
          const text = doc.createElementNS(ns, 'text');
          text.style.fontSize = '40px'; text.setAttribute('x', '2em'); factory.appendChild(text);
          const list = text.x.baseVal, item = list.getItem(0);
          check(prefix + '/list/attached-font', () => near(value.get.call(item), 80));
          check(prefix + '/list/remove-rebases', () => list.removeItem(0) === item && near(value.get.call(item), 2 * initialFont));
          check(prefix + '/list/detached-mutation', () => {
            specified.set.call(item, 6);
            return near(value.get.call(item), 6 * initialFont) && text.getAttribute('x') === '';
          });
          check(prefix + '/list/reattach-rebases', () => {
            text.style.fontSize = '20px';
            return list.appendItem(item) === item && near(value.get.call(item), 120) && text.getAttribute('x') === '6em';
          });
          check(prefix + '/list/clear-rebases', () => {
            list.clear(); return near(value.get.call(item), 6 * initialFont) && text.getAttribute('x') === '';
          });
          text.remove();
        }
      }
      factory.remove();
    }
  }
  globalThis.__svgDetachedLengthResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  globalThis.__uiEventResults = globalThis.__svgDetachedLengthResults;
  return true;
})()
