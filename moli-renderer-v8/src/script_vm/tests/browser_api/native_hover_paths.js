globalThis.__installHoverPathProbe = function (context, poison, pointer, modifiers) {
  const root = window, windows = [root], names = new Map(), rows = [], checks = [];
  document.documentElement.style.cssText = 'margin:0;padding:0';
  document.body.style.cssText = 'margin:0;padding:0';
  function frame(doc) {
    const iframe = doc.createElement('iframe');
    iframe.style.cssText = 'position:fixed;left:0;top:0;width:460px;height:300px;border:0';
    doc.body.appendChild(iframe);
    const win = iframe.contentWindow;
    windows.push(win);
    win.document.documentElement.style.cssText = 'margin:0;padding:0';
    win.document.body.style.cssText = 'margin:0;padding:0';
    return win;
  }
  let owner = root;
  if (context !== 'root') owner = frame(document);
  if (context === 'nested') owner = frame(owner.document);
  let holder = owner.document.body;
  if (context.startsWith('shadow-') || context === 'slotted') {
    const host = owner.document.createElement('div');
    host.style.cssText = 'position:fixed;left:0;top:0;width:460px;height:300px';
    holder.appendChild(host);
    const shadow = host.attachShadow({mode: context === 'shadow-closed' ? 'closed' : 'open'});
    if (context === 'slotted') shadow.appendChild(owner.document.createElement('slot'));
    else holder = shadow;
  }
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
        onlyOwner: windows.every(win => win === owner || !(e instanceof intrinsics.get(win)[isPointer ? 'Pointer' : 'Mouse'])),
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
      for (const key of ['exactPrototype', 'typed', 'onlyOwner', 'view', 'targetIdentity', 'currentIdentity', 'pathIdentity', 'trusted']) check(i + '/' + key, r[key], true);
      const boundary = r.type.endsWith('enter') || r.type.endsWith('leave');
      for (const key of ['bubbles', 'cancelable', 'composed']) check(i + '/' + key, r[key], !boundary);
      check(i + '/button', r.button, r.type.startsWith('pointer') ? -1 : 0);
      check(i + '/buttons', r.buttons, 0);
      check(i + '/modifiers', r.modifiers, [Boolean(modifiers & 1), Boolean(modifiers & 2), Boolean(modifiers & 4), Boolean(modifiers & 8)]);
      const related = {deep: context === 'root' ? 'outside' : null, parent: r.listener === 'inner' ? 'outer' : 'inner',
        child: r.listener === 'outer' ? 'inner' : 'outer', peer: r.listener === 'peer' ? 'inner' : 'peer', exit: context === 'root' ? 'outside' : null};
      check(i + '/related', r.related, related[r.phase]);
    }
    check('author constructor not read', constructorReads, 0);
    check('author parentNode not read', parentReads, 0);
    return {context, poison, pointer, modifiers, rows, checks, constructorReads, parentReads,
      passed: checks.filter(c => c.pass).length, total: checks.length, complete: checks.every(c => c.pass)};
  }};
};
