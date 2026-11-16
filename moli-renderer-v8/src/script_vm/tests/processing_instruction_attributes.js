(() => {
  const check = (condition, description) => {
    if (!condition) throw Error(description);
  };
  const same = (actual, expected, description) =>
    check(JSON.stringify(actual) === JSON.stringify(expected), description + ': ' + JSON.stringify(actual));
  const xml = new DOMParser().parseFromString('<root/>', 'application/xml');
  const html = document.implementation.createHTMLDocument('detached');
  const frame = document.createElement('iframe');
  document.body.append(frame);
  for (const owner of [document, xml, html, frame.contentDocument]) {
    const pi = owner.createProcessingInstruction('probe', 'A="one" b="&lt;&#x1F95A;"');
    check(pi.nodeType === 7 && pi.target === 'probe', 'native PI identity');
    same(pi.getAttributeNames(), ['A', 'b'], 'initial order');
    check(pi.hasAttributes() && pi.hasAttribute('A') && !pi.hasAttribute('a'), 'case-sensitive names');
    check(pi.getAttribute('b') === '<🥚' && pi.getAttribute('a') === null, 'XML references');
    const observer = new MutationObserver(() => {});
    observer.observe(pi, {characterData: true, characterDataOldValue: true, attributes: true});
    const original = pi.data;
    check(pi.setAttribute('A', 'updated') === undefined, 'set return value');
    const serialized = pi.data;
    pi.setAttribute('A', 'updated');
    pi.removeAttribute('missing');
    check(pi.toggleAttribute('A', true) === true, 'forced existing toggle');
    check(pi.toggleAttribute('missing', false) === false, 'forced absent toggle');
    const records = observer.takeRecords();
    check(records.length === 2 && records.every(record => record.type === 'characterData'
      && record.target === pi && record.attributeName === null), 'only real CharacterData records');
    same(records.map(record => record.oldValue), [original, serialized], 'old values including equal edits');
    const names = pi.getAttributeNames(); names.push('author');
    same(pi.getAttributeNames(), ['A', 'b'], 'attribute names are a snapshot');
    check(pi.toggleAttribute('z', undefined) && !pi.toggleAttribute('z', null), 'optional Boolean conversion');
    check(observer.takeRecords().length === 2, 'toggle mutation records');
    const range = owner.createRange();
    range.setStart(pi, 1); range.setEnd(pi, pi.length);
    pi.setAttribute('b', '<&>"');
    check(pi.data === 'A="updated" b="&lt;&amp;&gt;&quot;"', 'serialized escaping');
    check(range.startOffset === 0 && range.endOffset === 0, 'live range replacement');
    pi.data = 'unquoted=value';
    check(!pi.hasAttributes(), 'unquoted data is not XML pseudo-attributes');
    pi.data = 'a="1"'; pi.replaceData(3, 1, '2');
    check(pi.getAttribute('a') === '2', 'replaceData reparses');
    pi.insertData(0, 'A="x" ');
    same(pi.getAttributeNames(), ['A', 'a'], 'insertData reparses');
    pi.deleteData(0, 6);
    check(!pi.hasAttribute('A') && pi.getAttribute('a') === '2', 'deleteData reparses');
    pi.nodeValue = 'a="3"';
    check(pi.getAttribute('a') === '3', 'nodeValue reparses');
    pi.textContent = 'b="4"';
    check(pi.getAttribute('a') === null && pi.getAttribute('b') === '4', 'textContent reparses');
    pi.setAttribute('odd"key', 'retained');
    check(pi.getAttribute('odd"key') === 'retained', 'attribute serialization suppresses reparsing');
    pi.data = pi.data;
    check(!pi.hasAttributes(), 'an equal data assignment reparses the whole map');
    pi.setAttribute('raw', '\ud800a\udc00');
    check(pi.getAttribute('raw') === '\ud800a\udc00', 'UTF-16 attribute values');
    pi.data = pi.data;
    check(pi.getAttribute('raw') === '\ud800a\udc00', 'UTF-16 data round trip');
    pi.setAttribute('\ud800', 'name');
    check(pi.getAttribute('\ud800') === 'name' && pi.getAttribute('\ufffd') === null, 'UTF-16 attribute names');
    pi.data = 'a="1" a="2"';
    check(!pi.hasAttributes(), 'duplicate data invalidates the complete map');
    observer.disconnect();
  }
  const pi = document.createProcessingInstruction('conversion', '');
  let calls = [];
  pi.setAttribute({toString() { calls.push('name'); pi.data = 'B="changed"'; return 'A'; }},
    {toString() { calls.push('value'); return 'after'; }});
  same(calls, ['name', 'value'], 'WebIDL conversion order');
  same(pi.getAttributeNames(), ['B', 'A'], 'read current attributes after reentrancy');
  const sentinel = {};
  try {
    pi.setAttribute('x', {toString() { throw sentinel; }});
    throw Error('value conversion must throw');
  } catch (error) { check(error === sentinel, 'author exception identity'); }
  check(!pi.hasAttribute('x'), 'conversion failure makes no attribute change');
  for (const name of ['', 'a=b', 'a/b', 'a>b', 'a b', '\0', '\t', '\r', '\n', '\f']) {
    for (const operation of [() => pi.setAttribute(name, 'v'), () => pi.toggleAttribute(name, false)]) {
      let error; try { operation(); } catch (caught) { error = caught; }
      check(error instanceof DOMException && error.name === 'InvalidCharacterError', 'invalid name');
    }
  }
  let conversions = 0, traps = 0;
  const argument = {toString() { conversions++; return 'x'; }};
  const proxy = new Proxy(pi, {get() { traps++; throw sentinel; }, getPrototypeOf() { traps++; throw sentinel; }});
  const revoked = Proxy.revocable(pi, {}); revoked.revoke();
  const methods = [
    ['hasAttributes', [], 0], ['getAttributeNames', [], 0], ['getAttribute', [argument], 1],
    ['hasAttribute', [argument], 1], ['setAttribute', [argument, argument], 2],
    ['removeAttribute', [argument], 1], ['toggleAttribute', [argument], 1]
  ];
  for (const [name, args, length] of methods) {
    const method = ProcessingInstruction.prototype[name];
    check(method.name === name && method.length === length, 'method metadata: ' + name);
    for (const receiver of [null, undefined, {}, Object.create(pi), document.createComment(''),
      document.createTextNode(''), document.createElement('div'), proxy, revoked.proxy]) {
      let error; try { Reflect.apply(method, receiver, args); } catch (caught) { error = caught; }
      check(error instanceof TypeError, 'native receiver before conversion: ' + name);
    }
  }
  check(conversions === 0 && traps === 0, 'receiver validation does not execute author code');
  frame.remove();
  return true;
})()
