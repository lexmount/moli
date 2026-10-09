(() => {
  const checks = [];
  const check = (name, fn) => {
    try { checks.push({name, passed: fn() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const caught = fn => { try { fn(); return null; } catch (error) { return error; } };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  const owners = [window, frame.contentWindow, popup];
  const resources = [];
  const initial = 'color: red; background-color: blue; --marker: seed;';
  const operations = [
    ['color', (s, v) => { s.color = v; }, 'green', 'color'],
    ['camel', (s, v) => { s.backgroundColor = v; }, 'green', 'background-color'],
    ['dashed', (s, v) => { s['background-color'] = v; }, 'green', 'background-color'],
    ['cssText', (s, v) => { s.cssText = v; }, 'color: green;', 'color'],
    ['setProperty', (s, v) => s.setProperty('color', v), 'green', 'color'],
    ['custom', (s, v) => s.setProperty('--marker', v), 'green', '--marker'],
  ];
  function descriptor(owner, name) {
    const style = owner.document.createElement('div').style;
    for (let p = Object.getPrototypeOf(style); p; p = Object.getPrototypeOf(p)) {
      const d = Object.getOwnPropertyDescriptor(p, name);
      if (d) return d;
    }
    throw new Error('Missing CSSOM descriptor ' + name);
  }
  try {
    for (const [realm, owner] of owners.entries()) {
      const doc = owner.document;
      const live = doc.createElement('div'); doc.body.appendChild(live); resources.push(live);
      const detached = doc.createElement('div');
      const windowless = doc.implementation.createHTMLDocument('').createElement('div');
      const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
      const sheetElement = doc.createElement('style');
      sheetElement.textContent = '.write-conversion-target { color: red; }';
      doc.head.appendChild(sheetElement); resources.push(sheetElement);
      const Sheet = sheetElement.sheet.constructor;
      const constructed = new Sheet(); constructed.insertRule('.constructed { color: red; }');
      const providers = [
        ['inline', live.style, false], ['detached', detached.style, false],
        ['windowless', windowless.style, false], ['SVG', svg.style, false],
        ['rule', sheetElement.sheet.cssRules[0].style, false],
        ['constructed rule', constructed.cssRules[0].style, false],
        ['computed', owner.getComputedStyle(live), true],
      ];
      for (const [kind, style, readonly] of providers) {
        const reset = () => { live.style.cssText = initial; if (!readonly) style.cssText = initial; };
        for (const [operation, write, good, property] of operations) {
          const prefix = `${realm}/${kind}/${operation}`;
          for (const failure of ['toString', 'toPrimitive', 'getter', 'fallback', 'symbol', 'nonprimitive']) {
            check(prefix + '/' + failure + '/exception and unchanged declaration', () => {
              reset(); const before = style.cssText; const marker = {prefix, failure}; const calls = [];
              let value;
              if (failure === 'toString') value = {toString() { calls.push('toString'); throw marker; }};
              if (failure === 'toPrimitive') value = {[Symbol.toPrimitive](hint) { calls.push(hint); throw marker; }};
              if (failure === 'getter') value = {get toString() { calls.push('get'); throw marker; }};
              if (failure === 'fallback') value = {toString() { calls.push('toString'); return {}; }, valueOf() { calls.push('valueOf'); throw marker; }};
              if (failure === 'symbol') value = Symbol('value');
              if (failure === 'nonprimitive') value = {[Symbol.toPrimitive](hint) { calls.push(hint); return {}; }};
              const error = caught(() => write(style, value));
              const expected = failure === 'fallback' ? ['toString', 'valueOf'] : failure === 'getter' ? ['get'] : failure === 'toString' ? ['toString'] : failure === 'symbol' ? [] : ['string'];
              return (failure === 'symbol' || failure === 'nonprimitive' ? error?.name === 'TypeError' : error === marker) &&
                JSON.stringify(calls) === JSON.stringify(expected) && style.cssText === before;
            });
          }
          check(prefix + '/successful conversion precedes native write or readonly error', () => {
            reset(); let calls = 0;
            const error = caught(() => write(style, {[Symbol.toPrimitive](hint) { calls++; if (hint !== 'string') throw new Error(hint); return good; }}));
            return calls === 1 && (readonly ? error?.name === 'NoModificationAllowedError' : error === null && style.getPropertyValue(property) === 'green');
          });
          for (const [name, value] of [['null', null], ['undefined', undefined]]) {
            check(prefix + '/' + name + '/IDL string conversion', () => {
              reset(); const before = style.getPropertyValue(property);
              const error = caught(() => write(style, value));
              if (readonly) return error?.name === 'NoModificationAllowedError';
              const expected = operation === 'cssText' || value === null ? '' : operation === 'custom' ? 'undefined' : before;
              return error === null && style.getPropertyValue(property) === expected;
            });
          }
          if (!readonly) {
            for (const throws of [false, true]) {
              check(prefix + '/reentrant mutation/' + (throws ? 'throw' : 'success'), () => {
                reset(); const marker = {};
                const error = caught(() => write(style, {toString() {
                  style.setProperty(property, 'green');
                  if (throws) throw marker;
                  return operation === 'cssText' ? 'color: blue;' : 'blue';
                }}));
                return error === (throws ? marker : null) && style.getPropertyValue(property) === (throws ? 'green' : 'blue');
              });
            }
          }
        }
        for (const throwingArgument of ['none', 'property', 'value', 'priority']) {
          check(`${realm}/${kind}/setProperty/conversion order/${throwingArgument}`, () => {
            reset(); const before = style.cssText; const calls = []; const marker = {};
            const argument = (name, text) => ({toString() { calls.push(name); if (name === throwingArgument) throw marker; return text; }});
            const error = caught(() => style.setProperty(argument('property', 'color'), argument('value', 'green'), argument('priority', 'important')));
            const expected = throwingArgument === 'property' ? ['property'] : throwingArgument === 'value' ? ['property', 'value'] : ['property', 'value', 'priority'];
            return JSON.stringify(calls) === JSON.stringify(expected) && (throwingArgument === 'none' ?
              readonly ? error?.name === 'NoModificationAllowedError' : error === null && style.color === 'green' && style.getPropertyPriority('color') === 'important' :
              error === marker && style.cssText === before);
          });
        }
        for (const property of ['color', '--marker', 'unknown-css-property']) {
          for (const priorityThrows of [false, true]) {
            check(`${realm}/${kind}/undefined value/${property}/${priorityThrows}`, () => {
              reset(); const before = style.cssText; const calls = []; const marker = {};
              const error = caught(() => style.setProperty({toString() { calls.push('property'); return property; }}, undefined,
                {toString() { calls.push('priority'); if (priorityThrows) throw marker; return ''; }}));
              if (JSON.stringify(calls) !== '["property","priority"]') return false;
              if (priorityThrows) return error === marker && style.cssText === before;
              if (readonly) return error?.name === 'NoModificationAllowedError';
              return error === null && (property === '--marker' ? style.getPropertyValue(property) === 'undefined' : style.cssText === before);
            });
          }
        }
        check(`${realm}/${kind}/expando bypasses CSS conversion`, () => {
          const value = {toString() { throw new Error('Expando coerced'); }};
          style.__writeConversionExpando = value;
          return style.__writeConversionExpando === value;
        });
        for (const [calleeIndex, callee] of owners.entries()) {
          for (const member of ['cssText', 'setProperty']) {
            const binding = descriptor(callee, member);
            const fn = member === 'setProperty' ? binding.value : binding.set;
            const errorPrototype = fn.constructor('return TypeError.prototype')();
            for (const badKind of ['object', 'inherited', 'author Proxy', 'revoked Proxy']) {
              check(`${realm}/${kind}/${calleeIndex}/${member}/${badKind}/receiver before conversion`, () => {
                const revocable = Proxy.revocable(style, {}); revocable.revoke();
                const receiver = badKind === 'object' ? {} : badKind === 'inherited' ? Object.create(style) : badKind === 'author Proxy' ? new Proxy(style, {}) : revocable.proxy;
                let conversions = 0; const value = {toString() { conversions++; return 'green'; }};
                const error = caught(() => Reflect.apply(fn, receiver, member === 'setProperty' ? [value, value, value] : [value]));
                return Object.getPrototypeOf(error) === errorPrototype && conversions === 0;
              });
            }
          }
        }
      }
    }
  } finally {
    for (const resource of resources) resource.remove();
    popup.close(); frame.remove();
  }
  const facts = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  globalThis.__cssomWriteConversionResults = facts;
  globalThis.__uiEventResults = facts;
  return facts.passed === facts.total;
})()
