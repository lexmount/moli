(() => {
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  function setup(doc, mode) {
    const rootElement = doc.documentElement || doc.appendChild(doc.createElement('html'));
    const body = doc.body || rootElement.appendChild(doc.createElement('body'));
    const box = body.appendChild(doc.createElement('section'));
    box.innerHTML = '<span>before</span><div></div><span>after</span><div></div><p>unrelated</p>';
    const [before, host, after, otherHost, unrelated] = box.children;
    const root = host.attachShadow({mode});
    root.innerHTML = '<span>inside</span><div></div>';
    const nested = root.lastChild.attachShadow({mode});
    nested.textContent = 'nested';
    const other = otherHost.attachShadow({mode});
    other.textContent = 'second';
    return {box, host, before: before.firstChild, inside: root.firstChild.firstChild,
      after: after.firstChild, nested: nested.firstChild, second: other.firstChild,
      unrelated: unrelated.firstChild, roots: {shadowRoots: [root, nested, other]}};
  }
  function boundary(range, a, ai, b, bi) {
    return range.startContainer === a && range.startOffset === ai &&
      range.endContainer === b && range.endOffset === bi;
  }
  const selection = getSelection();
  const pairs = [
    ['inside', 2, 'after', 3, false], ['after', 3, 'inside', 2, true],
    ['before', 2, 'inside', 3, false], ['inside', 3, 'before', 2, true],
    ['inside', 2, 'second', 3, false], ['second', 3, 'inside', 2, true],
    ['inside', 2, 'nested', 3, false], ['nested', 3, 'inside', 2, true],
    ['inside', 2, 'box', 1, true], ['inside', 2, 'box', 2, false],
    ['inside', 2, 'inside', 5, false], ['inside', 5, 'inside', 2, true],
  ];
  for (const mode of ['open', 'closed']) {
    for (const [aName, ai, fName, fi, backward] of pairs) {
      const nodes = setup(document, mode);
      const a = nodes[aName], f = nodes[fName];
      const [start, si, end, ei] = backward ? [f, fi, a, ai] : [a, ai, f, fi];
      const crossRoot = a.getRootNode() !== f.getRootNode();
      const label = mode + ':' + aName + ':' + fName + ':' + backward;
      selection.setBaseAndExtent(a, ai, f, fi);
      check(selection.direction === (backward ? 'backward' : 'forward'), label + ': direction');
      check(selection.isCollapsed === crossRoot, label + ': observable collapsed projection');
      check(boundary(selection.getComposedRanges(nodes.roots)[0], start, si, end, ei), label + ': composed span');
      const oldRange = selection.getRangeAt(0);
      check(crossRoot ? boundary(oldRange, end, ei, end, ei) : boundary(oldRange, start, si, end, ei), label + ': range projection');
      selection.collapse(a, ai);
      const collapsedRange = selection.getRangeAt(0);
      selection.extend(f, fi);
      const extendedRange = selection.getRangeAt(0);
      check(extendedRange !== collapsedRange && extendedRange !== oldRange, label + ': fresh extended range');
      check(boundary(collapsedRange, a, ai, a, ai), label + ': old range unchanged');
      check(crossRoot ? boundary(extendedRange, f, fi, f, fi) : boundary(extendedRange, start, si, end, ei), label + ': extend boundary');
      check(selection.direction === (crossRoot ? 'none' : backward ? 'backward' : 'forward'), label + ': extend direction');
      check(crossRoot ? boundary(selection.getComposedRanges(nodes.roots)[0], f, fi, f, fi) :
        boundary(selection.getComposedRanges(nodes.roots)[0], start, si, end, ei), label + ': extend composed boundary');
      nodes.box.remove();
    }
    for (const backward of [false, true]) {
      for (const action of ['unrelated-data', 'insert-start', 'delete-start', 'reset-start', 'split-start',
        'insert-end', 'split-end', 'remove-unrelated', 'remove-host', 'parent-split']) {
        const nodes = setup(document, mode);
        let start = nodes.inside, si = 2, end = nodes.after, ei = 3;
        if (action === 'parent-split') { start = nodes.inside.parentNode; si = 1; }
        if (backward) selection.setBaseAndExtent(end, ei, start, si);
        else selection.setBaseAndExtent(start, si, end, ei);
        const range = selection.getRangeAt(0);
        const unrelatedRange = document.createRange();
        unrelatedRange.setStart(nodes.unrelated, 2);
        unrelatedRange.setEnd(nodes.unrelated, 5);
        if (action === 'unrelated-data') nodes.unrelated.insertData(0, 'X');
        if (action === 'insert-start') { nodes.inside.insertData(0, '\u{1F600}'); si += 2; }
        if (action === 'delete-start') { nodes.inside.deleteData(0, 1); si--; }
        if (action === 'reset-start') { nodes.inside.data = 'reset'; si = 0; }
        if (action === 'split-start') { start = nodes.inside.splitText(1); si = 1; }
        if (action === 'insert-end') { nodes.after.insertData(0, '\u{1F600}'); ei += 2; }
        if (action === 'split-end') { end = nodes.after.splitText(1); ei = 2; }
        if (action === 'remove-unrelated') nodes.unrelated.parentNode.remove();
        if (action === 'remove-host') { nodes.host.remove(); start = nodes.box; si = 1; }
        if (action === 'parent-split') nodes.inside.splitText(1);
        const label = mode + ':' + backward + ':' + action;
        check(selection.direction === (backward ? 'backward' : 'forward'), label + ': retain direction');
        check(selection.getRangeAt(0) === range && boundary(range, end, ei, end, ei), label + ': associated range');
        check(boundary(selection.getComposedRanges(nodes.roots)[0], start, si, end, ei), label + ': live composed endpoints');
        check(selection.isCollapsed && selection.anchorNode === end && selection.anchorOffset === ei &&
          selection.focusNode === end && selection.focusOffset === ei, label + ': live observable endpoints');
        nodes.box.remove();
      }
    }
  }
  // Composed parent offsets stay fixed for insertion but adjust for removal;
  // the exposed Range follows the live child boundaries in both cases.
  for (const crossRoot of [false, true]) {
    for (const offset of [1, 2, 3]) {
      for (const action of ['split', 'insert', 'remove']) {
        const nodes = setup(document, 'closed');
        const parent = nodes.inside.parentNode;
        parent.insertBefore(document.createElement('b'), nodes.inside);
        parent.appendChild(document.createElement('b'));
        const end = crossRoot ? nodes.after : parent;
        selection.setBaseAndExtent(parent, offset, end, 3);
        const range = selection.getRangeAt(0);
        if (action === 'split') nodes.inside.splitText(2);
        if (action === 'insert') parent.insertBefore(document.createElement('i'), parent.firstChild);
        if (action === 'remove') parent.firstChild.remove();
        const rawStart = action === 'remove' ? offset - 1 : offset;
        const rawEnd = !crossRoot && action === 'remove' ? 2 : 3;
        const liveStart = action === 'remove' ? offset - 1 : action === 'insert' || offset > 1 ? offset + 1 : offset;
        const label = 'parent offsets:' + crossRoot + ':' + offset + ':' + action;
        check(boundary(selection.getComposedRanges(nodes.roots)[0], parent, rawStart, end, rawEnd), label + ': raw offsets');
        check(crossRoot ? boundary(range, end, 3, end, 3) : boundary(range, parent, liveStart, parent, action === 'remove' ? 2 : 4),
          label + ': live range offsets updated');
        nodes.box.remove();
      }
    }
  }
  const parent = setup(document, 'closed');
  selection.setBaseAndExtent(parent.after, 3, parent.inside, 2);
  const retained = selection.getRangeAt(0);
  const frame = parent.box.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  const child = setup(other.document, 'closed');
  const childSelection = other.getSelection();
  childSelection.setBaseAndExtent(child.inside, 2, child.after, 3);
  const childRange = childSelection.getRangeAt(0);
  CharacterData.prototype.insertData.call(child.inside, 0, '\u{1F600}');
  check(childSelection.direction === 'forward' && childSelection.getRangeAt(0) === childRange &&
    boundary(childSelection.getComposedRanges(child.roots)[0], child.inside, 4, child.after, 3), 'cross-realm composed mutation');
  check(selection.direction === 'backward' && selection.getRangeAt(0) === retained &&
    boundary(selection.getComposedRanges(parent.roots)[0], parent.inside, 2, parent.after, 3), 'unrelated parent selection retained');
  child.box.remove();
  for (const action of ['text', 'host', 'parent-child']) {
    const nodes = setup(other.document, 'closed');
    const container = nodes.inside.parentNode;
    container.insertBefore(other.document.createElement('b'), nodes.inside);
    childSelection.setBaseAndExtent(action === 'parent-child' ? container : nodes.inside, 2, nodes.after, 3);
    const range = childSelection.getRangeAt(0);
    if (action === 'text') Node.prototype.removeChild.call(container, nodes.inside);
    if (action === 'host') Element.prototype.remove.call(nodes.host);
    if (action === 'parent-child') Node.prototype.removeChild.call(container, container.firstChild);
    check(childSelection.direction === 'forward' && childSelection.getRangeAt(0) === range &&
      boundary(childSelection.getComposedRanges(nodes.roots)[0], action === 'host' ? nodes.box : container, 1, nodes.after, 3),
      action + ': borrowed removal updates owner composed selection');
    check(selection.direction === 'backward' && selection.getRangeAt(0) === retained &&
      boundary(selection.getComposedRanges(parent.roots)[0], parent.inside, 2, parent.after, 3),
      action + ': borrowed removal preserves caller selection');
    nodes.box.remove();
  }
  parent.box.remove();
  return failures.join('\n');
})()
