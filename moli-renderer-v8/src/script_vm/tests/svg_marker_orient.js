(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const ns = 'http://www.w3.org/2000/svg';
  const realms = [window, document.querySelector('iframe').contentWindow];
  const values = ['', 'auto', 'auto-start-reverse', '90deg', '200grad', '12rad',
    'invalid angle', ' 90deg ', null, undefined, true, 23, '\0\uD800x\uDC00'];
  const raises = (realm, callback) => {
    try { callback(); }
    catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
    return false;
  };
  for (const [calleeIndex, callee] of realms.entries()) {
    const descriptor = Object.getOwnPropertyDescriptor(callee.SVGMarkerElement.prototype, 'orient');
    const get = receiver => {
      if (typeof descriptor?.get !== 'function') throw Error('Missing orient getter');
      return Reflect.apply(descriptor.get, receiver, []);
    };
    const set = (receiver, value) => {
      if (typeof descriptor?.set !== 'function') throw Error('Missing orient setter');
      return Reflect.apply(descriptor.set, receiver, [value]);
    };
    check(`${calleeIndex}/orient/descriptor`, () => descriptor.enumerable && descriptor.configurable &&
      descriptor.get.name === 'get orient' && descriptor.get.length === 0 &&
      descriptor.set.name === 'set orient' && descriptor.set.length === 1);
    check(`${calleeIndex}/orient/function-realm`, () =>
      Object.getPrototypeOf(descriptor.get) === callee.Function.prototype &&
      Object.getPrototypeOf(descriptor.set) === callee.Function.prototype);
    for (const [ownerIndex, owner] of realms.entries()) {
      const documents = [owner.document, owner.document.implementation.createHTMLDocument(''),
        new owner.DOMParser().parseFromString('<svg xmlns="' + ns + '"/>', 'image/svg+xml')];
      for (const [documentIndex, doc] of documents.entries()) {
        const prefix = `${calleeIndex}/${ownerIndex}/${documentIndex}/orient/`;
        const make = () => doc.createElementNS(ns, 'marker');
        check(prefix + 'missing-and-removal', () => {
          const marker = make();
          if (get(marker) !== '' || marker.hasAttribute('orient')) return false;
          set(marker, 'auto'); marker.removeAttribute('orient');
          return get(marker) === '' && !marker.hasAttribute('orient');
        });
        for (const [index, value] of values.entries()) {
          check(prefix + 'DOMString-' + index, () => {
            const marker = make();
            return set(marker, value) === undefined && get(marker) === String(value) &&
              marker.getAttribute('orient') === String(value) && !Object.hasOwn(marker, 'orient');
          });
        }
        check(prefix + 'cached-angle-and-enumeration', () => {
          const marker = make(), angle = marker.orientAngle, type = marker.orientType;
          set(marker, '90deg');
          return marker.orientAngle === angle && marker.orientType === type &&
            angle.baseVal.value === 90 && angle.animVal.value === 90 &&
            type.baseVal === 2 && type.animVal === 2;
        });
        check(prefix + 'native-setAttribute', () => {
          const marker = make(); marker.setAttribute('orient', '200grad');
          return get(marker) === '200grad' && marker.orientAngle.baseVal.value === 180;
        });
        check(prefix + 'auto-method-and-enumeration', () => {
          const marker = make(); marker.setOrientToAuto();
          if (get(marker) !== 'auto' || marker.orientType.baseVal !== 1) return false;
          set(marker, 'auto-start-reverse');
          return get(marker) === 'auto-start-reverse' && marker.orientType.baseVal === 3;
        });
        check(prefix + 'angle-method-and-writeback', () => {
          const marker = make(), angle = owner.document.createElementNS(ns, 'svg').createSVGAngle();
          angle.newValueSpecifiedUnits(owner.SVGAngle.SVG_ANGLETYPE_GRAD, 200);
          marker.setOrientToAngle(angle);
          if (get(marker) !== '200grad') return false;
          marker.orientAngle.baseVal.value = 45;
          return get(marker) === marker.getAttribute('orient') && marker.orientAngle.baseVal.value === 45;
        });
        check(prefix + 'coercion-before-mutation', () => {
          const marker = make(), calls = []; marker.setAttribute('orient', 'auto');
          set(marker, {[Symbol.toPrimitive](hint) {
            calls.push([hint, get(marker)]); return '45deg';
          }});
          return JSON.stringify(calls) === '[["string","auto"]]' && get(marker) === '45deg';
        });
        check(prefix + 'original-conversion-exception', () => {
          const marker = make(), sentinel = {}; marker.setAttribute('orient', 'auto');
          let thrown;
          try { set(marker, {toString() { throw sentinel; }}); } catch (error) { thrown = error; }
          return thrown === sentinel && get(marker) === 'auto';
        });
        check(prefix + 'Symbol-error-realm-and-no-mutation', () => {
          const marker = make(); marker.setAttribute('orient', 'auto');
          return raises(callee, () => set(marker, Symbol())) && get(marker) === 'auto';
        });
        check(prefix + 'bypass-author-methods-and-property', () => {
          const marker = make(); let traps = 0;
          for (const name of ['getAttribute', 'setAttribute', 'orient']) {
            Object.defineProperty(marker, name, {get() { traps++; throw 42; }});
          }
          set(marker, '30deg');
          return get(marker) === '30deg' && marker.orientAngle.baseVal.value === 30 && traps === 0;
        });
        check(prefix + 'clone-and-import', () => {
          const marker = make(); set(marker, 'auto-start-reverse');
          return get(marker.cloneNode(false)) === 'auto-start-reverse' &&
            get(owner.document.importNode(marker, false)) === 'auto-start-reverse';
        });
        if (documentIndex !== 0) continue;
        const real = make(), revoked = Proxy.revocable(real, {}); revoked.revoke();
        let traps = 0;
        const author = new Proxy(real, {get() { traps++; throw 42; }, getPrototypeOf() { traps++; throw 42; }});
        const invalid = [{}, Object.create(callee.SVGMarkerElement.prototype), Object.create(real),
          doc.createElementNS(ns, 'rect'), doc.createElement('div'), doc,
          author, revoked.proxy, null, undefined, 1, 'marker'];
        for (const [index, receiver] of invalid.entries()) {
          check(prefix + 'invalid-get-' + index, () => raises(callee, () => get(receiver)) && traps === 0);
          check(prefix + 'invalid-set-before-conversion-' + index, () => {
            let conversions = 0;
            return raises(callee, () => set(receiver, {toString() { conversions++; throw 42; }})) &&
              conversions === 0 && traps === 0;
          });
        }
      }
    }
  }
  globalThis.__svgMarkerOrientResults = {complete: true, total: checks.length,
    passed: checks.filter(row => row.passed).length, checks};
  globalThis.__uiEventResults = __svgMarkerOrientResults;
  return true;
})()
