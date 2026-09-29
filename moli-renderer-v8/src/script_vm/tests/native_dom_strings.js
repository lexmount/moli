(() => {
  const child = document.getElementById('child').contentWindow;
  const documents = [
    ['main', document],
    ['iframe', child.document],
    ['windowless', document.implementation.createHTMLDocument('')],
    ['xml', document.implementation.createDocument(null, 'root')]
  ];
  const failures = [];
  const units = value => Array.from({length: value.length}, (_, i) => value.charCodeAt(i));
  const check = (name, expected, operation) => {
    let actual;
    try { actual = operation(); }
    catch (error) { actual = {error: error.name}; }
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      failures.push({name, expected, actual});
    }
  };
  const values = ['\uD800', '\uDC00', 'a\uD801b', '😀', 'x\0\uFFFFy'];
  for (const [context, doc] of documents) {
    const factories = [
      ['text', value => doc.createTextNode(value)],
      ['comment', value => doc.createComment(value)],
      ['pi', value => doc.createProcessingInstruction('data', value)]
    ];
    if (context === 'xml') factories.push(['cdata', value => doc.createCDATASection(value)]);
    for (const [kind, create] of factories) {
      for (const value of values) {
        const prefix = `${context}/${kind}/${units(value)}`;
        const node = create(value);
        for (const property of ['data', 'nodeValue', 'textContent']) {
          check(`${prefix}/${property}`, units(value), () => units(node[property]));
        }
        check(`${prefix}/length`, value.length, () => node.length);
        check(`${prefix}/clone`, units(value), () => units(node.cloneNode().data));
        for (const [target, destination] of documents) {
          if (kind === 'cdata' && target !== 'xml') continue;
          check(`${prefix}/import/${target}`, units(value),
            () => units(destination.importNode(node, true).data));
        }
        const parent = doc.createElement('container');
        parent.appendChild(node);
        check(`${prefix}/deep-clone`, units(value),
          () => units(parent.cloneNode(true).firstChild.data));
        const mutations = [
          ['data', n => { n.data = '\uD802'; }, '\uD802'],
          ['nodeValue', n => { n.nodeValue = '\uD802'; }, '\uD802'],
          ['textContent', n => { n.textContent = '\uD802'; }, '\uD802'],
          ['append', n => n.appendData('\uDC03'), value + '\uDC03'],
          ['insert', n => n.insertData(0, '\uDC03'), '\uDC03' + value],
          ['replace', n => n.replaceData(0, 1, '\uD802'), '\uD802' + value.slice(1)],
          ['same', n => { n.data = value; }, value]
        ];
        for (const [action, apply, next] of mutations) {
          const edited = create(value);
          const observer = new MutationObserver(() => {});
          observer.observe(edited, {characterData: true, characterDataOldValue: true});
          check(`${prefix}/${action}/result`, units(next), () => {
            apply(edited);
            return units(edited.data);
          });
          check(`${prefix}/${action}/old`, [units(value)],
            () => observer.takeRecords().map(record => units(record.oldValue)));
          observer.disconnect();
        }
        if (kind === 'text' || kind === 'cdata') {
          for (let offset = 0; offset <= value.length; offset++) {
            check(`${prefix}/split/${offset}`,
              [units(value.slice(0, offset)), units(value.slice(offset))], () => {
                const left = create(value);
                const right = left.splitText(offset);
                return [units(left.data), units(right.data)];
              });
          }
        }
        for (const prototype of [Node.prototype, child.Node.prototype]) {
          check(`${prefix}/equal`, true, () => prototype.isEqualNode.call(node, create(value)));
          check(`${prefix}/unequal`, false,
            () => prototype.isEqualNode.call(node, create(value + '\uD801')));
        }
      }
      for (const [left, right] of [['\uD800', '\uD801'], ['\uDC00', '\uDC01'], ['\uD800', '\uFFFD']]) {
        check(`${context}/${kind}/distinct-${units(left)}-${units(right)}`, false,
          () => create(left).isEqualNode(create(right)));
      }
    }
    for (const [left, right] of [['\uD800', '\uDC00'], ['\uD800', 'b'], ['a', '\uDC00']]) {
      const parent = doc.createElement('container');
      parent.append(doc.createTextNode(left), doc.createTextNode(right));
      check(`${context}/descendant-text/${units(left)}/${units(right)}`, units(left + right),
        () => units(parent.textContent));
      check(`${context}/normalize/${units(left)}/${units(right)}`, [units(left + right), 1], () => {
        parent.normalize();
        return [units(parent.firstChild.data), parent.childNodes.length];
      });
    }
    for (const namespace of [null, 'urn:attribute']) {
      const left = doc.createElement('container');
      const right = doc.createElement('container');
      left.setAttributeNS(namespace, 'value', '\uD800');
      right.setAttributeNS(namespace, 'value', '\uD801');
      check(`${context}/attribute/${namespace}/different`, false, () => left.isEqualNode(right));
      right.setAttributeNS(namespace, 'value', '\uD800');
      check(`${context}/attribute/${namespace}/same`, true, () => left.isEqualNode(right));
    }
  }
  for (const realm of [window, child]) {
    for (const Constructor of [realm.Text, realm.Comment]) {
      for (const value of values) {
        check(`${Constructor.name}/construct/${units(value)}`, units(value),
          () => units(new Constructor(value).data));
      }
      check(`${Constructor.name}/default`, '', () => new Constructor().data);
      check(`${Constructor.name}/throwing-conversion`, {error: 'RangeError'},
        () => new Constructor({toString() { throw new RangeError('conversion'); }}));
      check(`${Constructor.name}/symbol`, {error: 'TypeError'}, () => new Constructor(Symbol()));
    }
  }
  globalThis.__nativeDomStringFailures = failures;
})()
