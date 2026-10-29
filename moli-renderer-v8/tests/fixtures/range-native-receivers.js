(() => {
  const rows = [];
  const check = (name, body) => {
    try { rows.push({name, passed: body() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const realms = [['top', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const abstractNames = ['startContainer', 'startOffset', 'endContainer', 'endOffset', 'collapsed'];
  const methods = ['setStart', 'setEnd', 'setStartBefore', 'setStartAfter', 'setEndBefore', 'setEndAfter',
    'collapse', 'selectNode', 'selectNodeContents', 'compareBoundaryPoints', 'deleteContents',
    'extractContents', 'cloneContents', 'insertNode', 'surroundContents', 'cloneRange', 'detach',
    'isPointInRange', 'comparePoint', 'intersectsNode', 'toString', 'createContextualFragment',
    'getClientRects', 'getBoundingClientRect'];
  const get = (realm, name) => {
    for (let p = realm.Range.prototype; p; p = Object.getPrototypeOf(p)) {
      const d = Object.getOwnPropertyDescriptor(p, name);
      if (d) return d.get;
    }
    throw Error('missing Range getter ' + name);
  };
  const fails = (realm, body) => {
    try { body(); return false; }
    catch (error) { return Object.getPrototypeOf(error) === realm.TypeError.prototype; }
  };
  function sample(owner, flavor) {
    const doc = flavor === 'html' ? owner.document.implementation.createHTMLDocument('range') :
      flavor === 'xml' ? owner.document.implementation.createDocument(null, 'r') :
      flavor === 'parsed' ? new owner.DOMParser().parseFromString('<r/>', 'application/xml') : owner.document;
    const root = doc.createElement('section'), text = root.appendChild(doc.createTextNode('abcdef'));
    (doc.body || doc.documentElement).appendChild(root);
    let range;
    if (flavor === 'constructor') range = new owner.Range();
    else if (flavor === 'subclass') { class Derived extends owner.Range {} range = new Derived(); }
    else range = doc.createRange();
    owner.Range.prototype.setStart.call(range, text, 1);
    owner.Range.prototype.setEnd.call(range, text, 3);
    if (flavor === 'clone') range = owner.Range.prototype.cloneRange.call(range);
    return {doc, root, text, range};
  }
  for (const [calleeName, callee] of realms) {
    const prototype = callee.Range.prototype;
    for (const name of abstractNames) {
      check(`${calleeName} AbstractRange ${name} owns its readonly accessor`, () => {
        const d = Object.getOwnPropertyDescriptor(callee.AbstractRange.prototype, name);
        return typeof d.get === 'function' && d.set === undefined && d.enumerable && d.configurable &&
          d.get.length === 0 && d.get.name === 'get ' + name && Object.getPrototypeOf(d.get) === callee.Function.prototype;
      });
    }
    check(`${calleeName} commonAncestorContainer belongs to Range only`, () => {
      const d = Object.getOwnPropertyDescriptor(prototype, 'commonAncestorContainer');
      return !!d && typeof d.get === 'function' && d.set === undefined && d.enumerable && d.configurable &&
        !Object.hasOwn(callee.AbstractRange.prototype, 'commonAncestorContainer') &&
        !('commonAncestorContainer' in callee.StaticRange.prototype);
    });
    for (const [ownerName, owner] of realms) {
      for (const flavor of ['constructor', 'document', 'html', 'xml', 'parsed', 'clone', 'subclass']) {
        const label = `${calleeName} borrows ${ownerName} ${flavor}`;
        const withSample = body => { const value = sample(owner, flavor); try { return body(value); } finally { value.root.remove(); } };
        check(`${label} native boundary getters`, () => withSample(({range, text}) =>
          get(callee, 'startContainer').call(range) === text && get(callee, 'endContainer').call(range) === text &&
          get(callee, 'startOffset').call(range) === 1 && get(callee, 'endOffset').call(range) === 3 &&
          get(callee, 'collapsed').call(range) === false && get(callee, 'commonAncestorContainer').call(range) === text));
        check(`${label} identity survives prototype removal and public shadows`, () => withSample(({range, text}) => {
          Object.defineProperty(range, 'startOffset', {get() {throw Error('public shadow');}});
          Object.setPrototypeOf(range, null);
          prototype.setStart.call(range, text, 2);
          return get(callee, 'startOffset').call(range) === 2 && prototype.toString.call(range) === 'c';
        }));
        for (const [name, body] of [
          ['setStart', ({range, text}) => { prototype.setStart.call(range, text, 2); return get(callee, 'startOffset').call(range) === 2; }],
          ['setEnd', ({range, text}) => { prototype.setEnd.call(range, text, 2); return get(callee, 'endOffset').call(range) === 2; }],
          ['setStartBefore', ({range, root, text}) => { prototype.setStartBefore.call(range, text); return get(callee, 'startContainer').call(range) === root && get(callee, 'startOffset').call(range) === 0; }],
          ['setStartAfter', ({range, root, text}) => { prototype.setStartAfter.call(range, text); return get(callee, 'startContainer').call(range) === root && get(callee, 'startOffset').call(range) === 1; }],
          ['setEndBefore', ({range, root, text}) => { prototype.setEndBefore.call(range, text); return get(callee, 'endContainer').call(range) === root && get(callee, 'endOffset').call(range) === 0; }],
          ['setEndAfter', ({range, root, text}) => { prototype.setEndAfter.call(range, text); return get(callee, 'endContainer').call(range) === root && get(callee, 'endOffset').call(range) === 1; }],
          ['selectNode', ({range, root, text}) => { prototype.selectNode.call(range, text); return get(callee, 'startContainer').call(range) === root && get(callee, 'startOffset').call(range) === 0 && get(callee, 'endOffset').call(range) === 1; }],
          ['selectNodeContents', ({range, root}) => { prototype.selectNodeContents.call(range, root); return get(callee, 'startContainer').call(range) === root && get(callee, 'startOffset').call(range) === 0 && get(callee, 'endOffset').call(range) === 1; }],
          ['collapse start', ({range}) => { prototype.collapse.call(range, true); return get(callee, 'collapsed').call(range) && get(callee, 'startOffset').call(range) === 1; }],
          ['collapse default end', ({range}) => { prototype.collapse.call(range); return get(callee, 'collapsed').call(range) && get(callee, 'startOffset').call(range) === 3; }],
          ['cloneRange', ({range, text}) => { const clone = prototype.cloneRange.call(range); return clone !== range && get(callee, 'startContainer').call(clone) === text && get(callee, 'startOffset').call(clone) === 1 && prototype.toString.call(clone) === 'bc'; }],
          ['cloneContents', ({range, doc}) => { const fragment = prototype.cloneContents.call(range); return fragment.ownerDocument === doc && fragment.textContent === 'bc'; }],
          ['createContextualFragment', ({range, doc}) => { const fragment = prototype.createContextualFragment.call(range, '<b>hi</b>'); return fragment.ownerDocument === doc && fragment.firstChild.localName === 'b' && fragment.textContent === 'hi'; }],
          ['compareBoundaryPoints', ({range}) => prototype.compareBoundaryPoints.call(range, 0, range) === 0],
          ['comparePoint', ({range, text}) => prototype.comparePoint.call(range, text, 2) === 0],
          ['isPointInRange', ({range, text}) => prototype.isPointInRange.call(range, text, 2) === true],
          ['intersectsNode', ({range, text}) => prototype.intersectsNode.call(range, text) === true],
          ['toString', ({range}) => prototype.toString.call(range) === 'bc'],
          ['detach', ({range, text}) => prototype.detach.call(range) === undefined && get(callee, 'startContainer').call(range) === text && get(callee, 'startOffset').call(range) === 1],
          ['deleteContents', ({range, root}) => prototype.deleteContents.call(range) === undefined && root.textContent === 'adef' && get(callee, 'collapsed').call(range)],
          ['extractContents', ({range, root, doc}) => { const fragment = prototype.extractContents.call(range); return fragment.ownerDocument === doc && fragment.textContent === 'bc' && root.textContent === 'adef' && get(callee, 'collapsed').call(range); }],
          ['insertNode', ({range, root, doc}) => prototype.insertNode.call(range, doc.createTextNode('X')) === undefined && root.textContent === 'aXbcdef'],
          ['surroundContents', ({range, root, doc}) => { const wrapper = doc.createElement('strong'); return prototype.surroundContents.call(range, wrapper) === undefined && wrapper.textContent === 'bc' && root.textContent === 'abcdef'; }],
        ]) check(`${label} ${name} accepts a genuine Range`, () => withSample(body));
      }
      const value = sample(owner, 'document'), {range, text} = value;
      const staticRange = new owner.StaticRange({startContainer: text, startOffset: 1, endContainer: text, endOffset: 3});
      check(`${calleeName} borrows ${ownerName} StaticRange boundary getters`, () =>
        get(callee, 'startContainer').call(staticRange) === text && get(callee, 'endContainer').call(staticRange) === text &&
        get(callee, 'startOffset').call(staticRange) === 1 && get(callee, 'endOffset').call(staticRange) === 3 && !get(callee, 'collapsed').call(staticRange));
      check(`${calleeName} borrows ${ownerName} StaticRange after prototype removal`, () => {
        Object.setPrototypeOf(staticRange, null);
        return get(callee, 'startContainer').call(staticRange) === text && get(callee, 'startOffset').call(staticRange) === 1;
      });
      let traps = 0, conversions = 0;
      const handler = {get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 1;}, has() {traps++; throw 1;}};
      const revoked = Proxy.revocable(range, handler); revoked.revoke();
      const invalid = [['null', null], ['undefined', undefined], ['plain', {}], ['number', 1],
        ['AbstractRange prototype', owner.AbstractRange.prototype], ['Range prototype', owner.Range.prototype],
        ['StaticRange prototype', owner.StaticRange.prototype], ['forged Range', Object.create(owner.Range.prototype)],
        ['forged StaticRange', Object.create(owner.StaticRange.prototype)], ['inherited instance', Object.create(range)],
        ['author Proxy', new Proxy(range, handler)], ['revoked Proxy', revoked.proxy], ['Node', text]];
      const prefix = `${calleeName} rejects ${ownerName}`;
      for (const [label, receiver] of invalid) for (const name of [...abstractNames, 'commonAncestorContainer']) {
        check(`${prefix} ${label} ${name} before traps`, () => {
          const before = traps;
          return fails(callee, () => get(callee, name).call(receiver)) && traps === before;
        });
      }
      for (const [label, receiver] of invalid.concat([['StaticRange instance', staticRange]])) for (const name of methods) {
        check(`${prefix} ${label} ${name} before argument conversion`, () => {
          const before = [traps, conversions];
          const poison = {valueOf() {conversions++; throw 1;}, toString() {conversions++; throw 1;}};
          let args;
          if (['setStart', 'setEnd', 'comparePoint', 'isPointInRange'].includes(name)) args = [text, poison];
          else if (name === 'compareBoundaryPoints') args = [poison, range];
          else if (name === 'createContextualFragment') args = [poison];
          else if (['collapse'].includes(name)) args = [poison];
          else if (['setStartBefore', 'setStartAfter', 'setEndBefore', 'setEndAfter', 'selectNode', 'selectNodeContents', 'insertNode', 'surroundContents', 'intersectsNode'].includes(name)) args = [text];
          else args = [];
          return fails(callee, () => prototype[name].apply(receiver, args)) && traps === before[0] && conversions === before[1];
        });
      }
      check(`${prefix} StaticRange commonAncestorContainer`, () => fails(callee, () => get(callee, 'commonAncestorContainer').call(staticRange)));
      value.root.remove();
    }
  }
  if (new Set(rows.map(row => row.name)).size !== rows.length) throw Error('Duplicate Range probe names');
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
