(async () => {
  const checks = [];
  const check = (name, actual, expected) => checks.push({name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected)});
  const observe = (name, action, expected) => { let actual; try { actual = action(); } catch (error) { actual = {exception: error.name}; } check(name, actual, expected); };
  const quote = id => (id.includes('"') ? "'" : '"') + id + (id.includes('"') ? "'" : '"');
  const markup = ([name, pub, sys]) => '<!DOCTYPE ' + name + (pub ? ' PUBLIC ' + quote(pub) : sys ? ' SYSTEM' : '') + (sys ? ' ' + quote(sys) : '') + '>';
  const fields = node => [node.name, node.nodeName, node.publicId, node.systemId];
  const cases = [
    ['root', '', ''], ['', '', ''], ['\uD800', '\uD801', '\uDC00'], ['\uD801', 'p', 's'],
    ['\uDC00', 'p\0', 's\0'], ['root', 'p', ''], ['root', '', 's'],
    ['root', 'p"b', ''], ['root', '', 's"b'], ['root', "p'b", ''], ['root', '', "s'b"],
    ['root', 'p"\'b', 's"\'b'], ['\uFFFD', '\uFFFD', '\uFFFD'], ['😀', '😀', '😀'],
    ['root', '&<>\t\n\r', '&<>\t\n\r'], ['root', 'p\0\uD800', 's\uDC00\0'],
  ];
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  let popup;
  const includePopup = globalThis.__documentTypeIncludePopup !== false;
  try {
    const realms = [['main', window], ['iframe', frame.contentWindow]];
    if (includePopup) {
      popup = window.open('', '_blank');
      if (!popup) throw new Error('popup unavailable');
      await new Promise(resolve => setTimeout(resolve, 0));
      realms.push(['popup', popup]);
    }
    for (const [ownerName, owner] of realms) {
      const documents = [
        ['live', owner.document], ['html', owner.document.implementation.createHTMLDocument('')],
        ['xml', owner.document.implementation.createDocument(null, 'root')],
        ['parsed-html', new owner.DOMParser().parseFromString('<!doctype html><body>', 'text/html')],
        ['parsed-xml', new owner.DOMParser().parseFromString('<root/>', 'application/xml')],
        ['new-document', new owner.Document()],
      ];
      for (const [docName, doc] of documents) for (const [calleeName, callee] of realms) {
        const prefix = `${ownerName}/${docName}/${calleeName}`;
        const create = callee.DOMImplementation.prototype.createDocumentType;
        const serialize = node => callee.XMLSerializer.prototype.serializeToString.call(new callee.XMLSerializer(), node);
        for (const [index, input] of cases.entries()) {
          const label = prefix + '/' + index;
          const expected = [input[0], input[0], input[1], input[2]];
          const node = create.call(doc.implementation, ...input);
          check(label + '/fields', fields(node), expected);
          check(label + '/owner', [node.ownerDocument === doc, node instanceof owner.DocumentType], [true, true]);
          check(label + '/serialize', serialize(node), markup(input));
          for (const deep of [false, true]) {
            observe(label + '/clone-' + deep, () => {
              const clone = callee.Node.prototype.cloneNode.call(node, deep);
              return [fields(clone), node.isEqualNode(clone), clone !== node, clone.ownerDocument === doc];
            }, [expected, true, true, true]);
          }
          const destination = owner.document.implementation.createDocument(null, 'root');
          const imported = destination.importNode(node, true);
          check(label + '/import', [fields(imported), imported.ownerDocument === destination, imported.isEqualNode(node)], [expected, true, true]);
          destination.insertBefore(imported, destination.documentElement);
          check(label + '/connected', [fields(destination.doctype), serialize(destination)], [expected, markup(input) + '<root/>']);
          const adopted = owner.document.implementation.createDocument(null, 'root');
          const source = destination.doctype;
          check(label + '/adopt', [adopted.adoptNode(source) === source, source.ownerDocument === adopted, fields(source)], [true, true, expected]);
          adopted.insertBefore(source, adopted.documentElement);
          check(label + '/adopt-connected', [fields(adopted.doctype), serialize(adopted)], [expected, markup(input) + '<root/>']);
          for (const key of ['name', 'nodeName', 'publicId', 'systemId']) Object.defineProperty(node, key, {get() {throw new Error('author getter: ' + key);}, configurable: true});
          observe(label + '/native-fields', () => {
            const cleanClone = callee.Node.prototype.cloneNode.call(node, false);
            return [serialize(node), fields(cleanClone), cleanClone.isEqualNode(node)];
          }, [markup(input), expected, true]);
        }
        for (const field of ['name', 'publicId', 'systemId']) {
          const left = ['root', 'p', 's']; const right = left.slice();
          const position = field === 'name' ? 0 : field === 'publicId' ? 1 : 2;
          left[position] = '\uD800'; right[position] = '\uD801';
          const a = create.call(doc.implementation, ...left), b = create.call(doc.implementation, ...right);
          right[position] = '\uFFFD'; const replacement = create.call(doc.implementation, ...right);
          check(prefix + '/identity/' + field, [a.isEqualNode(b), b.isEqualNode(a), a.isEqualNode(replacement), a.isEqualNode(callee.Node.prototype.cloneNode.call(a))], [false, false, false, true]);
        }
        const log = [];
        const strings = ['r\uD800', 'p\uD801', 's\uDC00'];
        const converted = create.call(doc.implementation, ...strings.map((value, index) => ({toString() {log.push(index); return value;}})));
        check(prefix + '/conversion', [log, fields(converted)], [[0, 1, 2], [strings[0], strings[0], strings[1], strings[2]]]);
      }
    }
    globalThis.__uiEventResults = {complete: true, includePopup, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
    return true;
  } finally { frame.remove(); if (popup) popup.close(); }
})()
