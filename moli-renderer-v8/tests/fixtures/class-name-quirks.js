function classNameQuirksProbe(mainQuirks) {
  const checks = [];
  const check = (label, actual, expected) => checks.push({label, actual, expected, pass: actual === expected});
  const list = (label, value, expected) => {
    check(label, Array.from(value, node => node.id).join(','), expected);
    check(label + '/item', value.item(0)?.id ?? null, expected.split(',')[0] || null);
  };
  const parser = new DOMParser();
  const html = '<html><head></head><body></body></html>';
  const standard = parser.parseFromString('<!doctype html>' + html, 'text/html');
  const quirks = mainQuirks ? document : null;
  const limited = parser.parseFromString('<!DOCTYPE HTML PUBLIC "-//W3C//DTD XHTML 1.0 Transitional//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd">' + html, 'text/html');
  function runDocument(doc, label, insensitive) {
    let root, outside, mutations;
    try {
      check(label + '/mode', doc.compatMode, insensitive ? 'BackCompat' : 'CSS1Compat');
      const parent = doc.body || doc.documentElement;
      root = parent.appendChild(doc.createElementNS('http://www.w3.org/1999/xhtml', 'section'));
      root.id = 'root'; root.setAttribute('class', 'a b');
      outside = parent.appendChild(doc.createElementNS('http://www.w3.org/1999/xhtml', 'span'));
      outside.id = 'outside'; outside.setAttribute('class', 'a b');
      const rows = [
        ['lower', 'a b'], ['upper', 'A B'], ['mixed', 'A b'], ['both', 'a A b B'],
        ['svg', 'a b', 'http://www.w3.org/2000/svg'], ['math', 'a b', 'http://www.w3.org/1998/Math/MathML'],
        ['k', 'k'], ['K', 'K'], ['kelvin', '\u212a'], ['ring', '\u00e5'], ['RING', '\u00c5'],
        ['nbsp', 'a\u00a0b'], ['vt', 'a\u000bb'], ['empty', ''],
      ];
      for (const [id, value, namespace = 'http://www.w3.org/1999/xhtml'] of rows) {
        const node = root.appendChild(doc.createElementNS(namespace, namespace.endsWith('/svg') ? 'g' : namespace.endsWith('/MathML') ? 'mi' : 'span'));
        node.id = id; node.setAttribute('class', value);
      }
      const all = 'lower,upper,mixed,both,svg,math';
      const queries = [
        ['A', insensitive ? all : 'upper,mixed,both'],
        ['a', insensitive ? all : 'lower,both,svg,math'],
        ['B', insensitive ? all : 'upper,both'],
        ['a A', insensitive ? all : 'both'],
        ['A A', insensitive ? all : 'upper,mixed,both'],
        [' \tA\nB\r\f', insensitive ? all : 'upper,both'],
        ['K', insensitive ? 'k,K' : 'K'], ['\u212a', 'kelvin'], ['\u00e5', 'ring'],
        ['a\u00a0b', 'nbsp'], ['a\u000bb', 'vt'], ['', ''], [' \t\n\r\f', ''],
      ];
      for (const [query, expected] of queries) {
        list(label + '/query/' + JSON.stringify(query), root.getElementsByClassName(query), expected);
      }
      const expectedDocument = insensitive ? 'root,' + all + ',outside' : 'upper,mixed,both';
      list(label + '/document', doc.getElementsByClassName('A'), expectedDocument);
      check(label + '/classList-stays-sensitive', root.classList.contains('A'), false);

      mutations = parent.appendChild(doc.createElementNS('http://www.w3.org/1999/xhtml', 'section'));
      const first = mutations.appendChild(doc.createElementNS('http://www.w3.org/1999/xhtml', 'span'));
      const second = mutations.appendChild(doc.createElementNS('http://www.w3.org/1999/xhtml', 'span'));
      first.id = 'first'; second.id = 'second'; first.setAttribute('class', 'thing'); second.setAttribute('class', 'THING');
      const held = mutations.getElementsByClassName('THING');
      list(label + '/live-initial', held, insensitive ? 'first,second' : 'second');
      first.setAttribute('class', 'THING');
      list(label + '/live-attribute', held, 'first,second');
      mutations.insertBefore(second, first);
      list(label + '/live-reorder', held, 'second,first');
      first.remove();
      list(label + '/live-remove', held, 'second');
      first.setAttribute('class', 'thing'); mutations.appendChild(first);
      list(label + '/live-insert', held, insensitive ? 'second,first' : 'second');
      second.removeAttribute('class'); mutations.remove();
      list(label + '/live-disconnected', held, insensitive ? 'first' : '');
    } catch (error) { check(label + '/error', String(error.stack || error), 'success'); }
    finally { root?.remove(); outside?.remove(); mutations?.remove(); }
  }
  for (const [doc, label, insensitive] of [
    [document, 'live', mainQuirks],
    [standard, 'parsed-standard', false], [limited, 'parsed-limited', false],
    [document.implementation.createHTMLDocument(''), 'created', false],
    [parser.parseFromString('<root/>', 'application/xml'), 'xml', false],
  ]) runDocument(doc, label, insensitive);

  if (quirks) try {
    const root = quirks.createElement('section');
    const child = root.appendChild(quirks.createElement('span'));
    child.id = 'adopted'; child.className = 'a';
    const held = root.getElementsByClassName('A');
    list('adoption/quirks', held, 'adopted');
    standard.adoptNode(root);
    check('adoption/owner', child.ownerDocument === standard, true);
    list('adoption/standard', held, '');
    child.className = 'A';
    list('adoption/changed', held, 'adopted');
    child.className = 'a'; quirks.adoptNode(root);
    list('adoption/back-to-quirks', held, 'adopted');
  } catch (error) { check('adoption/error', String(error.stack || error), 'success'); }

  let frame;
  try {
    frame = document.body.appendChild(document.createElement('iframe'));
    const other = frame.contentWindow;
    const foreign = other.document.implementation.createHTMLDocument('');
    const root = foreign.body.appendChild(foreign.createElement('section'));
    const child = root.appendChild(foreign.createElement('span'));
    child.id = 'foreign'; child.className = 'A';
    for (const [name, method, receiver] of [
      ['local-document', Document.prototype.getElementsByClassName, foreign],
      ['local-element', Element.prototype.getElementsByClassName, root],
      ['foreign-document', other.Document.prototype.getElementsByClassName, foreign],
      ['foreign-element', other.Element.prototype.getElementsByClassName, root],
    ]) list('realm/' + name, method.call(receiver, 'A'), 'foreign');
    const local = standard.body.appendChild(standard.createElement('section'));
    local.appendChild(standard.createElement('span')).className = 'a';
    list('realm/foreign-method-standard-receiver', other.Element.prototype.getElementsByClassName.call(local, 'A'), '');
    local.remove();
  } catch (error) { check('realm/error', String(error.stack || error), 'success'); }
  finally { frame?.remove(); }
  return {state: checks.every(item => item.pass) ? 'pass' : 'fail', checks};
}
