globalThis.__installHoverPathProbe = function (poison, pointer, modifiers) {
  const windows = [window], names = new Map(), rows = [], checks = [];
  document.documentElement.style.cssText = 'margin:0;padding:0';
  document.body.style.cssText = 'margin:0;padding:0';
  const owner = window, holder = document.body;
  function element(doc, parent, name, left, top, width, height) {
    const node = doc.createElement('div');
    node.id = name;
    node.style.cssText = `position:fixed;left:${left}px;top:${top}px;width:${width}px;height:${height}px;background:#acf`;
    parent.appendChild(node);
    names.set(node, name);
    return node;
  }
  const outer = element(owner.document, holder, 'outer', 40, 40, 220, 200);
  const inner = element(owner.document, outer, 'inner', 100, 80, 80, 80);
  const peer = element(owner.document, holder, 'peer', 320, 40, 100, 200);
  element(document, document.body, 'outside', 520, 40, 80, 200);
  const intrinsics = new Map(windows.map(win => [win, {Mouse: win.MouseEvent, Pointer: win.PointerEvent}]));
  let phase = 'reset', constructorReads = 0, parentReads = 0;
  const types = ['pointerover', 'mouseover', 'pointerenter', 'mouseenter', 'pointerout', 'mouseout', 'pointerleave', 'mouseleave'];
  for (const node of [outer, inner, peer]) {
    for (const type of types) node.addEventListener(type, e => {
      e.stopPropagation();
      if (phase === 'reset') return;
      const isPointer = type.startsWith('pointer'), Kind = intrinsics.get(owner)[isPointer ? 'Pointer' : 'Mouse'];
      rows.push({phase, type, target: names.get(e.target), listener: names.get(node), related: names.get(e.relatedTarget) || null,
        exactPrototype: Object.getPrototypeOf(e) === Kind.prototype, typed: e instanceof Kind,
        view: e.view === owner, targetIdentity: e.target === node, currentIdentity: e.currentTarget === node,
        pathIdentity: e.composedPath()[0] === node, trusted: e.isTrusted,
        bubbles: e.bubbles, cancelable: e.cancelable, composed: e.composed,
        button: e.button, buttons: e.buttons,
        modifiers: [e.altKey, e.ctrlKey, e.metaKey, e.shiftKey]});
    });
  }
  for (const win of windows) {
    for (const name of ['PointerEvent', 'MouseEvent']) {
      if (poison === 'deleted') delete win[name];
      if (poison === 'replaced') win[name] = function () { constructorReads++; throw new Error('author constructor'); };
      if (poison === 'getter') Object.defineProperty(win, name, {configurable: true, get() { constructorReads++; throw new Error('author constructor'); }});
    }
    if (poison === 'getter') Object.defineProperty(win.Node.prototype, 'parentNode', {
      configurable: true, get() { parentReads++; throw new Error('author parentNode'); }
    });
  }
  function check(name, actual, expected) {
    checks.push({name, actual, expected, pass: JSON.stringify(actual) === JSON.stringify(expected)});
  }
  return {prepare(value) { phase = value; }, finish() {
    const expected = [
      ['deep', 'inner', ['pointerover']],
      ['deep', 'outer', ['pointerenter']],
      ['deep', 'inner', ['pointerenter', 'mouseover']],
      ['deep', 'outer', ['mouseenter']],
      ['deep', 'inner', ['mouseenter']],
      ['parent', 'inner', ['pointerout', 'pointerleave']],
      ['parent', 'outer', ['pointerover']],
      ['parent', 'inner', ['mouseout', 'mouseleave']],
      ['parent', 'outer', ['mouseover']],
      ['child', 'outer', ['pointerout']],
      ['child', 'inner', ['pointerover', 'pointerenter']],
      ['child', 'outer', ['mouseout']],
      ['child', 'inner', ['mouseover', 'mouseenter']],
      ['peer', 'inner', ['pointerout', 'pointerleave']],
      ['peer', 'outer', ['pointerleave']],
      ['peer', 'peer', ['pointerover', 'pointerenter']],
      ['peer', 'inner', ['mouseout', 'mouseleave']],
      ['peer', 'outer', ['mouseleave']],
      ['peer', 'peer', ['mouseover', 'mouseenter']],
      ['exit', 'peer', ['pointerout', 'pointerleave', 'mouseout', 'mouseleave']]
    ];
    check('boundary order', rows.map(r => [r.phase, r.listener, r.type]),
      expected.flatMap(([at, target, events]) => events.map(type => [at, target, type])));
    for (const [i, r] of rows.entries()) {
      for (const key of ['exactPrototype', 'typed', 'targetIdentity', 'currentIdentity', 'pathIdentity', 'trusted']) check(i + '/' + key, r[key], true);
      const boundary = r.type.endsWith('enter') || r.type.endsWith('leave');
      for (const key of ['bubbles', 'cancelable']) check(i + '/' + key, r[key], !boundary);
      check(i + '/button', r.button, r.type.startsWith('pointer') ? -1 : 0);
      check(i + '/buttons', r.buttons, 0);
      check(i + '/modifiers', r.modifiers, [Boolean(modifiers & 1), Boolean(modifiers & 2), Boolean(modifiers & 4), Boolean(modifiers & 8)]);
      const related = {deep: 'outside', parent: r.listener === 'inner' ? 'outer' : 'inner',
        child: r.listener === 'outer' ? 'inner' : 'outer', peer: r.listener === 'peer' ? 'inner' : 'peer', exit: 'outside'};
      check(i + '/related', r.related, related[r.phase]);
    }
    check('author constructor not read', constructorReads, 0);
    check('author parentNode not read', parentReads, 0);
    return {poison, pointer, modifiers, rows, checks, constructorReads, parentReads,
      passed: checks.filter(c => c.pass).length, total: checks.length, complete: checks.every(c => c.pass)};
  }};
};
