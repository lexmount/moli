(() => {
  const child = document.getElementById('child').contentWindow;
  const documents = [
    ['main', document], ['iframe', child.document],
    ['windowless', document.implementation.createHTMLDocument('')],
    ['xml', document.implementation.createDocument(null, 'root')]
  ];
  const failures = [];
  let checks = 0;
  const check = (name, condition, details) => {
    checks++;
    if (!condition) failures.push({name, details});
  };
  for (const [context, doc] of documents) {
    for (const [entry, normalize] of [
      ['own', node => node.normalize()],
      ['top', node => Node.prototype.normalize.call(node)],
      ['iframe', node => child.Node.prototype.normalize.call(node)]
    ]) {
      for (const values of [
        ['ab', 'cd', 'ef'], ['', 'a', '', 'b', ''], ['', '', ''],
        ['😀', '\uD800', '\uDC00', 'z'], ['only'], []
      ]) {
        const parent = doc.createElement('container');
        const nodes = values.map(value => parent.appendChild(doc.createTextNode(value)));
        const first = values.findIndex(value => value.length > 0);
        const prefixLength = index => values.slice(first, index).join('').length;
        const ranges = [];
        const addRange = (node, offset, expectedNode, expectedOffset) => {
          const range = doc.createRange();
          range.setStart(node, offset);
          range.collapse(true);
          ranges.push({range, expectedNode, expectedOffset});
        };
        for (let index = 0; index < nodes.length; index++) {
          for (let offset = 0; offset <= values[index].length; offset++) {
            addRange(nodes[index], offset,
              first < 0 || index < first ? parent : nodes[first],
              first < 0 || index < first ? 0 : prefixLength(index) + offset);
          }
        }
        for (let index = 0; index <= nodes.length; index++) {
          const atParent = first < 0 || index <= first || index === nodes.length;
          const expectedOffset = first < 0 || index <= first ? 0
            : index === nodes.length ? 1 : prefixLength(index);
          addRange(parent, index, atParent ? parent : nodes[first], expectedOffset);
        }
        normalize(parent);
        const label = `${context}/${entry}/${JSON.stringify(values)}`;
        check(`${label}/contents`, parent.textContent === values.join('') &&
          parent.childNodes.length === (first < 0 ? 0 : 1));
        const validateRanges = phase => {
          for (const [index, {range, expectedNode, expectedOffset}] of ranges.entries()) {
            check(`${label}/${phase}/${index}`,
              range.startContainer === expectedNode && range.endContainer === expectedNode &&
              range.startOffset === expectedOffset && range.endOffset === expectedOffset,
              [range.startOffset, range.endOffset, expectedOffset]);
          }
        };
        validateRanges('normalized');
        for (let index = 0; index < nodes.length; index++) {
          if (index !== first) nodes[index].data = 'removed';
        }
        validateRanges('after-removed-node-edit');
      }
      for (const empty of [false, true]) {
        for (const before of [false, true]) {
          const parent = doc.createElement('container');
          const first = parent.appendChild(doc.createTextNode(empty ? '' : 'ab'));
          const second = parent.appendChild(doc.createTextNode(empty ? '' : 'cd'));
          const tail = parent.appendChild(doc.createElement('tail'));
          const iterator = doc.createNodeIterator(parent);
          while (iterator.nextNode() !== second) {}
          if (before) { iterator.nextNode(); iterator.previousNode(); iterator.previousNode(); }
          normalize(parent);
          const reference = before ? tail : empty ? parent : first;
          check(`${context}/${entry}/iterator/${empty}/${before}`,
            iterator.referenceNode === reference && iterator.pointerBeforeReferenceNode === before);
          check(`${context}/${entry}/iterator-next/${empty}/${before}`, iterator.nextNode() === tail);
        }
      }
      const container = doc.createElement('container');
      const left = container.appendChild(doc.createTextNode('a'));
      const comment = container.appendChild(doc.createComment(''));
      const nested = container.appendChild(doc.createElement('nested'));
      nested.append(doc.createTextNode('x'), doc.createTextNode('y'));
      const right = container.appendChild(doc.createTextNode('b'));
      normalize(container);
      check(`${context}/${entry}/separate-runs`, container.childNodes.length === 4 &&
        left.data === 'a' && right.data === 'b' && comment.parentNode === container &&
        nested.childNodes.length === 1 && nested.textContent === 'xy');
    }
    if (context === 'xml') {
      const parent = doc.createElement('container');
      parent.append(doc.createTextNode('a'), doc.createCDATASection(''), doc.createTextNode('b'));
      parent.normalize();
      check('xml/cdata-separates-runs', parent.childNodes.length === 3);
    }
  }
  for (const realm of [window, child]) {
    const doc = realm.document;
    for (const shadow of [false, true]) {
      const host = doc.body.appendChild(doc.createElement('div'));
      const parent = shadow ? host.attachShadow({mode: 'open'}) : host;
      const first = parent.appendChild(doc.createTextNode('ab'));
      const second = parent.appendChild(doc.createTextNode('cd'));
      const selection = realm.getSelection();
      selection.setBaseAndExtent(second, 2, second, 1);
      if (shadow) {
        host.normalize();
        check(`${realm === window}/shadow-not-traversed`, parent.childNodes.length === 2);
      }
      parent.normalize();
      check(`${realm === window}/selection/${shadow}`,
        selection.anchorNode === first && selection.anchorOffset === 4 &&
        selection.focusNode === first && selection.focusOffset === 3);
      const [composed] = selection.getComposedRanges({shadowRoots: shadow ? [parent] : []});
      check(`${realm === window}/composed/${shadow}`, composed.startContainer === first &&
        composed.startOffset === 3 && composed.endContainer === first && composed.endOffset === 4);
      second.data = 'removed';
      check(`${realm === window}/selection-stable/${shadow}`, selection.anchorNode === first &&
        selection.anchorOffset === 4 && selection.focusNode === first && selection.focusOffset === 3);
      selection.removeAllRanges();
      host.remove();
    }
  }
  globalThis.__nodeNormalizationFailures = failures;
  globalThis.__nodeNormalizationCheckCount = checks;
})()
