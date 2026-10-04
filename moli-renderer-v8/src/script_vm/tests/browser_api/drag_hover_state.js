globalThis.__installDragHoverProbe = function (context, pointer, capture, mode, poison, modifiers) {
  const windows = [window], rows = [], names = new Map(), checks = [];
  document.body.style.cssText = 'margin:0';
  function frame(win) {
    const element = win.document.createElement('iframe');
    element.style.cssText = 'position:fixed;left:0;top:0;width:760px;height:300px;border:0';
    win.document.body.appendChild(element);
    const child = element.contentWindow;
    child.document.body.style.cssText = 'margin:0';
    windows.push(child);
    return child;
  }
  let owner = window;
  if (context === 'child' || context === 'nested') owner = frame(owner);
  if (context === 'nested') owner = frame(owner);
  const doc = owner.document;
  names.set(doc.documentElement, 'html');
  names.set(doc.body, 'body');
  function element(parent, name, left, width) {
    const node = doc.createElement('div');
    node.id = name;
    node.style.cssText = `position:fixed;left:${left}px;top:40px;width:${width}px;height:140px;background:#acf`;
    parent.appendChild(node);
    names.set(node, name);
    return node;
  }
  let holder = doc.body, extra = [];
  if (context.startsWith('shadow-') || context === 'slotted') {
    const host = element(holder, 'host', 0, 760);
    const shadow = host.attachShadow({mode: context === 'shadow-closed' ? 'closed' : 'open'});
    extra = ['host'];
    if (context === 'slotted') {
      const slot = doc.createElement('slot');
      names.set(slot, 'slot');
      shadow.appendChild(slot);
      extra.unshift('slot');
      holder = host;
    } else holder = shadow;
  }
  const outer = element(holder, 'outer', 20, 180);
  const source = element(outer, 'source', 40, 140);
  const peer = element(holder, 'peer', 500, 140);
  source.draggable = true;
  let phase = 'setup', pointerId = null, reads = 0, parentReads = 0, ended = 0;
  const intrinsics = windows.map(win => ({win, Pointer: win.PointerEvent, Mouse: win.MouseEvent}));
  const kinds = intrinsics.find(entry => entry.win === owner);
  source.addEventListener('pointerdown', event => {
    pointerId = event.pointerId;
    if (capture) source.setPointerCapture(pointerId);
  });
  source.addEventListener('dragstart', event => {
    event.dataTransfer.setData('text/plain', 'hover');
    event.dataTransfer.effectAllowed = 'copy';
    if (mode === 'cancel') event.preventDefault();
  });
  for (const type of ['dragenter', 'dragover', 'drop']) peer.addEventListener(type, event => {
    event.dataTransfer.dropEffect = 'copy';
    event.preventDefault();
  });
  source.addEventListener('dragend', () => ended++);
  const types = ['pointercancel', 'pointerout', 'pointerleave', 'pointerover', 'pointerenter',
    'mouseout', 'mouseleave', 'mouseover', 'mouseenter'];
  for (const [node, name] of names) for (const type of types) node.addEventListener(type, event => {
    event.stopPropagation();
    const isPointer = type.startsWith('pointer'), Kind = isPointer ? kinds.Pointer : kinds.Mouse;
    rows.push({phase, type, target: name, related: names.get(event.relatedTarget) || null,
      exactPrototype: Object.getPrototypeOf(event) === Kind.prototype, typed: event instanceof Kind,
      onlyOwner: intrinsics.every(entry => entry.win === owner || !(event instanceof (isPointer ? entry.Pointer : entry.Mouse))),
      view: event.view === owner, targetIdentity: event.target === node, currentIdentity: event.currentTarget === node,
      pathIdentity: event.composedPath()[0] === node, trusted: event.isTrusted,
      bubbles: event.bubbles, cancelable: event.cancelable, composed: event.composed,
      button: event.button, buttons: event.buttons, pointerType: event.pointerType,
      x: event.clientX, y: event.clientY, pressure: event.pressure,
      modifiers: [event.altKey, event.ctrlKey, event.metaKey, event.shiftKey]});
  });
  if (poison === 'getter') for (const win of windows) {
    for (const name of ['PointerEvent', 'MouseEvent']) Object.defineProperty(win, name, {
      configurable: true, get() { reads++; throw new Error('author constructor'); }
    });
    Object.defineProperty(win.Node.prototype, 'parentNode', {
      configurable: true, get() { parentReads++; throw new Error('author parent'); }
    });
  }
  function check(name, actual, expected) {
    checks.push({name, actual, expected, pass: JSON.stringify(actual) === JSON.stringify(expected)});
  }
  return {prepare(value) { phase = value; }, finish() {
    const active = mode !== 'cancel';
    const start = rows.filter(row => row.phase === 'start');
    check('suppression leaves every pointer ancestor', start.map(row => [row.type, row.target]), active
      ? [['pointercancel', 'source'], ['pointerout', 'source'],
        ...['source', 'outer', ...extra, 'body', 'html'].map(name => ['pointerleave', name])]
      : []);
    // A prevented drag can retain capture until release. Check its complete
    // transition sequence, including the first move after capture is released.
    const transitionPhases = active ? ['after'] : ['drag', 'up', 'after'];
    const mouse = rows.filter(row => transitionPhases.includes(row.phase) && !row.type.startsWith('pointer'));
    check('legacy mouse retains its previous target', mouse.map(row => [row.type, row.target, row.related]), [
      ['mouseout', 'source', 'peer'], ['mouseleave', 'source', 'peer'], ['mouseleave', 'outer', 'peer'],
      ['mouseover', 'peer', 'source'], ['mouseenter', 'peer', 'source']]);
    const after = rows.filter(row => transitionPhases.includes(row.phase) && row.type.startsWith('pointer'));
    check('pointer transitions reflect whether drag was suppressed', after.map(row => [row.type, row.target, row.related]), active
      ? [['pointerover', 'peer', null], ...['html', 'body', ...extra.slice().reverse(), 'peer'].map(name => ['pointerenter', name, null])]
      : [['pointerout', 'source', 'peer'], ['pointerleave', 'source', 'peer'], ['pointerleave', 'outer', 'peer'],
        ['pointerover', 'peer', 'source'], ['pointerenter', 'peer', 'source']]);
    check('no boundary events during native drag', rows.filter(row => active && row.phase === 'drag').length, 0);
    check('dragend count', ended, active ? 1 : 0);
    check('capture released', source.hasPointerCapture(pointerId), false);
    for (const [i, row] of rows.entries()) {
      for (const key of ['exactPrototype', 'typed', 'onlyOwner', 'view', 'targetIdentity', 'currentIdentity', 'pathIdentity', 'trusted']) check(i + '/' + key, row[key], true);
      const enterLeave = row.type.endsWith('enter') || row.type.endsWith('leave');
      check(i + '/bubbles', row.bubbles, !enterLeave);
      check(i + '/cancelable', row.cancelable, !enterLeave && row.type !== 'pointercancel');
      check(i + '/composed', row.composed, !enterLeave);
    }
    check('native factories ignore author constructors', reads, 0);
    check('native ancestry ignores author parentNode', parentReads, 0);
    return {context, pointer, capture, mode, poison, modifiers, rows, checks, reads, parentReads,
      passed: checks.filter(row => row.pass).length, total: checks.length, complete: checks.every(row => row.pass)};
  }};
};
