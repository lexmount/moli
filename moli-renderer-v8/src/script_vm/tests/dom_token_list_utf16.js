(() => {
  const checks = [];
  const check = (name, run) => {
    try { checks.push({name, passed: run() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const same = (actual, expected) => actual.length === expected.length &&
    actual.every((value, index) => value === expected[index]);
  const SVG = 'http://www.w3.org/2000/svg';
  const child = document.querySelector('iframe').contentWindow;
  const surfaces = [
    ['HTML class', null, 'div', 'classList', 'class'],
    ['SVG class', SVG, 'g', 'classList', 'class'],
    ['part', null, 'div', 'part', 'part'],
    ['HTML rel', null, 'a', 'relList', 'rel'],
    ['link rel', null, 'link', 'relList', 'rel'],
    ['SVG rel', SVG, 'a', 'relList', 'rel'],
    ['sandbox', null, 'iframe', 'sandbox', 'sandbox'],
    ['htmlFor', null, 'output', 'htmlFor', 'for'],
    ['sizes', null, 'link', 'sizes', 'sizes'],
  ];
  const samples = ['\ud800', '\udfff', '\ud800x\udfff', '\ufffd', '\ud83d\ude00',
    'a\u0000b', 'a\u000bb', 'a\u00a0b', 'a\u0085b', 'a\u2003b', 'MiXeD'];
  for (const [realmName, realm] of [['top', globalThis], ['child', child]]) {
    const proto = realm.DOMTokenList.prototype;
    for (const name of ['item', 'contains', 'add', 'remove', 'toggle', 'replace', 'supports', 'toString']) {
      check(`${realmName} own method ${name}`, () => {
        const d = Object.getOwnPropertyDescriptor(proto, name);
        return d.enumerable && d.configurable && d.writable && typeof d.value === 'function';
      });
    }
    const documents = [['live', realm.document],
      ['windowless HTML', realm.document.implementation.createHTMLDocument('')],
      ['XML', new realm.DOMParser().parseFromString(`<svg xmlns="${SVG}"/>`, 'image/svg+xml')]];
    for (const [docName, doc] of documents) for (const [label, ns, tag, property, attr] of surfaces) {
      const el = ns ? doc.createElementNS(ns, tag) : doc.createElementNS('http://www.w3.org/1999/xhtml', tag);
      const list = el[property];
      const prefix = `${realmName} ${docName} ${label}`;
      const reset = value => value === null ? el.removeAttribute(attr) : el.setAttribute(attr, value);
      const state = (value, tokens) => el.getAttribute(attr) === value && list.value === (value ?? '') &&
        proto.toString.call(list) === (value ?? '') && list.length === tokens.length && same([...list], tokens);
      check(`${prefix} native owner realm and SameObject`, () =>
        Object.getPrototypeOf(list) === proto && el[property] === list);
      for (const token of samples) {
        const name = `${prefix} ${JSON.stringify(token)}`;
        check(`${name} parse ordered set`, () => {
          reset(`\t${token} \r\n${token}\f tail tail `);
          return state(`\t${token} \r\n${token}\f tail tail `, [token, 'tail']);
        });
        check(`${name} item index and descriptor`, () => {
          reset(`${token} tail`);
          const d = Object.getOwnPropertyDescriptor(list, '0');
          return list.item(0) === token && list[0] === token && d.value === token &&
            !d.writable && d.enumerable && d.configurable && list.item(2) === null && list[2] === undefined;
        });
        check(`${name} contains distinguishes UTF16 identity`, () => {
          reset(token);
          return list.contains(token) && !list.contains(`${token}x`) &&
            (token === '\ud800' || token === '\udfff' ? !list.contains('\ufffd') : true);
        });
        check(`${name} add deduplicates without rewriting units`, () => {
          reset('head');
          return proto.add.call(list, token, token) === undefined && state(`head ${token}`, ['head', token]);
        });
        check(`${name} remove compares exact units`, () => {
          reset(`head ${token} tail`);
          return proto.remove.call(list, token) === undefined && state('head tail', ['head', 'tail']);
        });
        check(`${name} toggle round trip`, () => {
          reset('head');
          return list.toggle(token) === true && state(`head ${token}`, ['head', token]) &&
            list.toggle(token) === false && state('head', ['head']);
        });
        check(`${name} toggle no-op keeps original attribute`, () => {
          reset(` ${token}  ${token} `);
          return list.toggle(token, true) === true && state(` ${token}  ${token} `, [token]);
        });
        check(`${name} replace preserves set order`, () => {
          reset(`first ${token} last`);
          return list.replace(token, 'replacement') && state('first replacement last', ['first', 'replacement', 'last']);
        });
        check(`${name} replace merges existing token`, () => {
          reset(`first ${token} last`);
          return list.replace('first', token) && state(`${token} last`, [token, 'last']);
        });
        check(`${name} replace same token normalizes once`, () => {
          reset(` ${token}  ${token} `);
          return list.replace(token, token) && state(token, [token]);
        });
        check(`${name} iterable values and entries`, () => {
          reset(`${token} tail`);
          const entries = [...list.entries()], seen = [], thisArg = {};
          list.forEach(function(value, index, object) { seen.push([value, index, object === list, this === thisArg]); }, thisArg);
          return same([...list.values()], [token, 'tail']) && same([...list.keys()], [0, 1]) &&
            entries.length === 2 && same(entries[0], [0, token]) && same(entries[1], [1, 'tail']) &&
            same(seen[0], [token, 0, true, true]) && same(seen[1], ['tail', 1, true, true]);
        });
        check(`${name} value and stringifier preserve literal units`, () => {
          const value = ` ${token}\t${token} `;
          list.value = value;
          return state(value, [token]) && String(list) === value;
        });
      }
      check(`${prefix} replacement character stays distinct`, () => {
        reset('\ud800 \udfff \ufffd');
        list.remove('\ufffd');
        return state('\ud800 \udfff', ['\ud800', '\udfff']) && !list.contains('\ufffd');
      });
      check(`${prefix} conversion exceptions are atomic`, () => {
        reset('original');
        const marker = new realm.Error('conversion'); let error, conversions = '';
        try { list.add({toString(){conversions += 'a'; return '\ud800';}}, {toString(){conversions += 'b'; throw marker;}}); }
        catch (caught) { error = caught; }
        return error === marker && conversions === 'ab' && state('original', ['original']);
      });
      check(`${prefix} all conversions precede token validation`, () => {
        reset('original'); let error, conversions = '';
        try { list.add({toString(){conversions += 'a'; return ''; }}, {toString(){conversions += 'b'; return '\ud800';}}); }
        catch (caught) { error = caught; }
        return error instanceof realm.DOMException && error.name === 'SyntaxError' && conversions === 'ab' && state('original', ['original']);
      });
      check(`${prefix} reentrant conversion sees latest attribute`, () => {
        reset('before');
        list.add({toString(){reset('\udfff after'); return '\ud800';}});
        return state('\udfff after \ud800', ['\udfff', 'after', '\ud800']);
      });
      check(`${prefix} mutation after adoption retains identity`, () => {
        const other = realm.document.implementation.createHTMLDocument('');
        reset('before');
        list.add({toString(){other.adoptNode(el); reset('\udfff moved'); return '\ud800';}});
        return el.ownerDocument === other && el[property] === list && state('\udfff moved \ud800', ['\udfff', 'moved', '\ud800']);
      });
      check(`${prefix} live iterator observes changed tokens`, () => {
        reset('\ud800 first'); const iterator = list.values();
        if (iterator.next().value !== '\ud800') return false;
        reset('\udfff second third');
        return iterator.next().value === 'second' && iterator.next().value === 'third' && iterator.next().done;
      });
      for (const whitespace of [' ', '\t', '\n', '\r', '\f']) for (const method of ['add', 'remove', 'toggle', 'replace']) {
        check(`${prefix} ${method} rejects ${JSON.stringify(whitespace)} atomically`, () => {
          reset('\ud800 original'); let error;
          try { method === 'replace' ? list.replace('\ud800', `a${whitespace}b`) : list[method](`a${whitespace}b`); }
          catch (caught) { error = caught; }
          return error instanceof realm.DOMException && error.name === 'InvalidCharacterError' && state('\ud800 original', ['\ud800', 'original']);
        });
      }
      check(`${prefix} supports preserves conversion semantics`, () => {
        reset('\ud800'); let converted = 0, error, result;
        try { result = list.supports({toString(){converted++; return '\ud800';}}); } catch (caught) { error = caught; }
        return converted === 1 && (['HTML rel', 'link rel', 'SVG rel', 'sandbox'].includes(label) ? result === false : error instanceof realm.TypeError);
      });
      check(`${prefix} no-op on missing attribute`, () => {
        reset(null); list.add(); list.remove();
        return !el.hasAttribute(attr) && list.toggle('\ud800', false) === false && !el.hasAttribute(attr) && list.length === 0;
      });
    }
    const real = realm.document.createElement('div').classList;
    const revoked = realm.Proxy.revocable(real, {}); revoked.revoke();
    let traps = 0;
    const proxy = new realm.Proxy(real, {get(){traps++; throw Error('trap');}, getPrototypeOf(){traps++; throw Error('trap');}});
    const value = Object.getOwnPropertyDescriptor(proto, 'value');
    const length = Object.getOwnPropertyDescriptor(proto, 'length');
    for (const [kind, receiver] of [['null',null], ['undefined',undefined], ['plain',{}], ['forged',Object.create(proto)],
      ['inherited',Object.create(real)], ['node',realm.document.createElement('div')], ['proxy',proxy], ['revoked',revoked.proxy]]) {
      for (const method of ['item','contains','add','remove','toggle','replace','supports','toString']) {
        check(`${realmName} ${method} rejects ${kind} before conversion`, () => {
          let error, conversions = 0; const input = {toString(){conversions++; return 'bad';}, valueOf(){conversions++; return 0;}};
          try { proto[method].call(receiver, input, input); } catch (caught) { error = caught; }
          return error instanceof realm.TypeError && conversions === 0 && traps === 0;
        });
      }
      for (const [name, getter] of [['length',length.get], ['value',value.get]]) check(`${realmName} ${name} rejects ${kind}`, () => {
        let error; try { getter.call(receiver); } catch (caught) { error = caught; }
        return error instanceof realm.TypeError && traps === 0;
      });
      check(`${realmName} value setter rejects ${kind} before conversion`, () => {
        let error, conversions = 0;
        try { value.set.call(receiver, {toString(){conversions++; return 'bad';}}); } catch (caught) { error = caught; }
        return error instanceof realm.TypeError && conversions === 0 && traps === 0;
      });
    }
    check(`${realmName} array intrinsic identity retained`, () => ['entries','keys','values','forEach'].every(name => proto[name] === realm.Array.prototype[name]) && proto[realm.Symbol.iterator] === realm.Array.prototype.values);
  }
  for (const [calleeName, callee, owner] of [['top',globalThis,child], ['child',child,globalThis]]) {
    const el = owner.document.createElement('div'), list = el.classList, proto = callee.DOMTokenList.prototype;
    check(`${calleeName} borrowed methods preserve receiver units`, () => {
      proto.add.call(list, '\ud800', '\udfff');
      return el.getAttribute('class') === '\ud800 \udfff' && proto.item.call(list, 0) === '\ud800' &&
        proto.toString.call(list) === '\ud800 \udfff' && proto.contains.call(list, '\udfff');
    });
    check(`${calleeName} invalid token DOMException belongs to callee`, () => {
      let error; try { proto.add.call(list, 'has space'); } catch (caught) { error = caught; }
      return error instanceof callee.DOMException && !(error instanceof owner.DOMException) && error.name === 'InvalidCharacterError';
    });
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})();
