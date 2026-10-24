(() => {
  const rows = [];
  function check(name, body) {
    try { rows.push({name, passed: body() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  }
  function throws(realm, kind, body) {
    try { body(); return false; }
    catch (error) {
      return kind === 'TypeError' ? Object.getPrototypeOf(error) === realm.TypeError.prototype :
        Object.getPrototypeOf(error) === realm.DOMException.prototype && error.name === kind;
    }
  }
  const realms = [['top', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const methods = ['setStart', 'setEnd', 'setStartBefore', 'setStartAfter', 'setEndBefore',
    'setEndAfter', 'selectNode', 'selectNodeContents', 'comparePoint', 'isPointInRange',
    'intersectsNode', 'insertNode', 'surroundContents'];
  const offsetMethods = ['setStart', 'setEnd', 'comparePoint', 'isPointInRange'];
  function sample(owner, flavor) {
    const doc = flavor === 'html' ? owner.document.implementation.createHTMLDocument('arguments') :
      flavor === 'xml' ? owner.document.implementation.createDocument(null, 'r') : owner.document;
    const root = doc.createElement('section'), text = root.appendChild(doc.createTextNode('abcdef'));
    (doc.body || doc.documentElement).appendChild(root);
    const range = doc.createRange();
    range.setStart(text, 0); range.setEnd(text, 3);
    return {doc, root, text, range};
  }
  function withSample(owner, flavor, body) {
    const value = sample(owner, flavor);
    try { return body(value); }
    finally { value.root.remove(); }
  }
  function poison(node, names, seen) {
    for (const name of names) Object.defineProperty(node, name, {
      configurable: true, get() { seen.push(name); throw Error('author Node property: ' + name); }
    });
  }
  function invalidValues(owner, text, seen) {
    const revoked = owner.Proxy.revocable(text, {}); revoked.revoke();
    return [
      ['undefined', undefined], ['null', null], ['number', 1], ['string', 'text'], ['symbol', Symbol('node')],
      ['function', function () {}], ['object', {}], ['shape', {nodeType: 3, length: 6}],
      ['prototype', Object.create(owner.Node.prototype)], ['inherited', Object.create(text)],
      ['getter', {get nodeType() {seen.push('nodeType'); throw Error('fake nodeType');}}],
      ['proxy', new owner.Proxy(text, {
        get() {seen.push('get'); throw Error('author proxy get');},
        getPrototypeOf() {seen.push('getPrototypeOf'); throw Error('author proxy prototype');}
      })], ['revoked', revoked.proxy]
    ];
  }
  for (const [calleeName, callee] of realms) {
    const prototype = callee.Range.prototype;
    for (const name of methods) check(`${calleeName} ${name} required arity`, () => {
      const r = new callee.Range();
      return throws(callee, 'TypeError', () => prototype[name].call(r));
    });
    check(`${calleeName} compareBoundaryPoints arity precedes how conversion`, () => {
      let conversions = 0;
      return throws(callee, 'TypeError', () => prototype.compareBoundaryPoints.call(new callee.Range(), {
        valueOf() {conversions++; throw Error('must not convert missing-argument call');}
      })) && conversions === 0;
    });
    for (const [ownerName, owner] of realms) for (const flavor of ['live', 'html', 'xml']) {
      const label = `${calleeName} accepts ${ownerName} ${flavor}`;
      for (const name of methods) {
        const seen = [];
        const values = withSample(owner, flavor, ({text}) => invalidValues(owner, text, seen));
        for (const [invalidName, value] of values) check(`${label} ${name} rejects ${invalidName} before offset conversion`, () =>
          withSample(owner, flavor, ({range, text}) => {
            seen.length = 0; let conversions = 0;
            const offset = {valueOf() {conversions++; return 1;}};
            const rejected = throws(callee, 'TypeError', () => prototype[name].call(range, value, offset));
            return rejected && conversions === 0 && seen.length === 0 &&
              range.startContainer === text && range.startOffset === 0 && range.endOffset === 3;
          }));
      }
      for (const name of offsetMethods) {
        check(`${label} ${name} converts offset exactly once after Node`, () => withSample(owner, flavor, ({range, text}) => {
          const seen = [];
          poison(text, ['nodeType', 'length', 'childNodes', 'parentNode', 'previousSibling'], seen);
          let conversions = 0;
          const result = prototype[name].call(range, text, {valueOf() {conversions++; return 1;}});
          return conversions === 1 && seen.length === 0 && (name === 'setStart' ? range.startOffset === 1 :
            name === 'setEnd' ? range.endOffset === 1 : name === 'comparePoint' ? result === 0 : result === true);
        }));
        check(`${label} ${name} preserves offset exception identity`, () => withSample(owner, flavor, ({range, text}) => {
          const sentinel = {}, seen = [];
          poison(text, ['nodeType', 'length'], seen);
          try { prototype[name].call(range, text, {valueOf() {throw sentinel;}}); return false; }
          catch (error) { return error === sentinel && seen.length === 0 && range.startOffset === 0 && range.endOffset === 3; }
        }));
      }
      for (const name of ['setStartBefore', 'setStartAfter', 'setEndBefore', 'setEndAfter', 'selectNode', 'selectNodeContents', 'intersectsNode']) {
        check(`${label} ${name} ignores public Node state`, () => withSample(owner, flavor, ({range, text, root}) => {
          const seen = [];
          poison(text, ['nodeType', 'length', 'childNodes', 'parentNode', 'previousSibling'], seen);
          poison(root, ['childNodes'], seen);
          const result = prototype[name].call(range, text);
          if (seen.length) return false;
          if (name === 'intersectsNode') return result === true;
          if (name === 'selectNodeContents') return range.startContainer === text && range.startOffset === 0 && range.endOffset === 6;
          if (name === 'selectNode') return range.startContainer === root && range.startOffset === 0 && range.endOffset === 1;
          const side = name.includes('Start') ? 'start' : 'end';
          return range[side + 'Container'] === root && range[side + 'Offset'] === (name.endsWith('After') ? 1 : 0);
        }));
      }
      for (const name of ['insertNode', 'surroundContents']) check(`${label} ${name} accepts a Node with poisoned public type`, () =>
        withSample(owner, flavor, ({range, doc, root}) => {
          const node = doc.createElement('b'), seen = [];
          poison(node, ['nodeType', 'parentNode', 'previousSibling', 'childNodes'], seen);
          prototype[name].call(range, node);
          return seen.length === 0 && root.textContent === 'abcdef' &&
            (name === 'insertNode' ? root.firstChild.nextSibling === node : node.textContent === 'abc');
        }));
      check(`${label} compareBoundaryPoints accepts genuine cross-realm Range`, () => withSample(owner, flavor, ({range}) => {
        const clone = range.cloneRange();
        Object.setPrototypeOf(clone, null);
        return prototype.compareBoundaryPoints.call(range, 0, clone) === 0;
      }));
      for (const invalidName of ['static', 'object', 'prototype', 'inherited', 'proxy', 'revoked']) {
        check(`${label} compareBoundaryPoints rejects ${invalidName} with Range brand`, () => withSample(owner, flavor, ({range, text}) => {
          let traps = 0, conversions = 0;
          const revoked = owner.Proxy.revocable(range, {}); revoked.revoke();
          const invalid = invalidName === 'static' ? new owner.StaticRange({startContainer: text, startOffset: 0, endContainer: text, endOffset: 3}) :
            invalidName === 'prototype' ? Object.create(owner.Range.prototype) : invalidName === 'inherited' ? Object.create(range) :
            invalidName === 'proxy' ? new owner.Proxy(range, {get() {traps++; throw Error('range proxy');}}) :
            invalidName === 'revoked' ? revoked.proxy : {};
          const rejected = throws(callee, 'TypeError', () => prototype.compareBoundaryPoints.call(range, {
            valueOf() {conversions++; return 0;}
          }, invalid));
          return rejected && conversions === 1 && traps === 0;
        }));
      }
      check(`${label} StaticRangeInit lexical member conversion order`, () => withSample(owner, flavor, ({text}) => {
        const seen = [], init = {};
        for (const name of ['startOffset', 'startContainer', 'endOffset', 'endContainer']) Object.defineProperty(init, name, {get() {
          seen.push(name);
          return name.endsWith('Container') ? text : {valueOf() {seen.push(name + '.valueOf'); return name === 'startOffset' ? 1 : 3;}};
        }});
        poison(text, ['nodeType'], seen);
        const r = new callee.StaticRange(init);
        return JSON.stringify(seen) === JSON.stringify(['endContainer', 'endOffset', 'endOffset.valueOf', 'startContainer', 'startOffset', 'startOffset.valueOf']) &&
          r.startContainer === text && r.endContainer === text && r.startOffset === 1 && r.endOffset === 3;
      }));
      for (const endpoint of ['endContainer', 'startContainer']) {
        const seen = [], values = withSample(owner, flavor, ({text}) => invalidValues(owner, text, seen));
        for (const [invalidName, invalid] of values) check(`${label} StaticRangeInit ${endpoint} rejects ${invalidName} before later members`, () =>
          withSample(owner, flavor, ({text}) => {
            seen.length = 0; const order = [], init = {};
            for (const name of ['endContainer', 'endOffset', 'startContainer', 'startOffset']) Object.defineProperty(init, name, {get() {
              order.push(name); return name === endpoint ? invalid : name.endsWith('Container') ? text :
                {valueOf() {order.push(name + '.valueOf'); return 0;}};
            }});
            const rejected = throws(callee, 'TypeError', () => new callee.StaticRange(init));
            const expected = endpoint === 'endContainer' ? ['endContainer'] : ['endContainer', 'endOffset', 'endOffset.valueOf', 'startContainer'];
            return rejected && seen.length === 0 && JSON.stringify(order) === JSON.stringify(expected);
          }));
      }
      for (const type of ['Attr', 'DocumentType']) for (const endpoint of ['startContainer', 'endContainer']) {
        check(`${label} StaticRange rejects native ${type} after all conversions at ${endpoint}`, () => withSample(owner, flavor, ({doc, text}) => {
          const invalid = type === 'Attr' ? doc.createAttribute('x') : doc.implementation.createDocumentType('r', '', '');
          const seen = [];
          Object.defineProperty(invalid, 'nodeType', {get() {throw Error('must use native node type');}});
          const init = {};
          for (const name of ['endContainer', 'endOffset', 'startContainer', 'startOffset']) Object.defineProperty(init, name, {get() {
            seen.push(name); return name === endpoint ? invalid : name.endsWith('Container') ? text :
              {valueOf() {seen.push(name + '.valueOf'); return 0;}};
          }});
          return throws(callee, 'InvalidNodeTypeError', () => new callee.StaticRange(init)) && JSON.stringify(seen) ===
            JSON.stringify(['endContainer', 'endOffset', 'endOffset.valueOf', 'startContainer', 'startOffset', 'startOffset.valueOf']);
        }));
      }
      check(`${label} StaticRange keeps unconstrained converted offsets and roots`, () => withSample(owner, flavor, ({doc, text}) => {
        const other = doc.createTextNode('x');
        const r = new callee.StaticRange({startContainer: text, startOffset: -1, endContainer: other, endOffset: 4294967298});
        return r.startContainer === text && r.startOffset === 4294967295 && r.endContainer === other && r.endOffset === 2;
      }));
      for (const member of ['endContainer', 'endOffset', 'startContainer', 'startOffset']) {
        check(`${label} StaticRangeInit preserves ${member} getter exception identity`, () => withSample(owner, flavor, ({text}) => {
          const sentinel = {}, init = {startContainer: text, endContainer: text, startOffset: 0, endOffset: 0};
          Object.defineProperty(init, member, {get() {throw sentinel;}});
          try { new callee.StaticRange(init); return false; }
          catch (error) { return error === sentinel; }
        }));
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.length > 0 && rows.every(row => row.passed);
})()
