globalThis.__installClickTargetProbe = function (mode, button, inChild) {
  const root = window;
  let win = root, doc = document;
  document.documentElement.style.cssText = 'margin:0;padding:0';
  document.body.style.cssText = 'margin:0;padding:0';
  if (inChild) {
    const frame = document.createElement('iframe');
    frame.style.cssText = 'position:fixed;left:0;top:0;width:600px;height:420px;border:0';
    document.body.appendChild(frame);
    win = frame.contentWindow;
    doc = frame.contentDocument;
  }
  doc.documentElement.style.cssText = 'margin:0;padding:0';
  doc.body.style.cssText = 'margin:0;padding:0;user-select:none';
  doc.body.id = 'body';
  const parent = doc.createElement(mode === 'common-label' ? 'label' : 'div');
  parent.id = 'parent';
  parent.style.cssText = 'position:fixed;left:30px;top:30px;width:500px;height:160px;background:#ddd;user-select:none';
  const source = doc.createElement('div'), destination = doc.createElement('div');
  source.id = 'source'; destination.id = 'destination';
  source.style.cssText = 'position:fixed;left:40px;top:40px;width:100px;height:100px;background:#faa';
  destination.style.cssText = 'position:fixed;left:260px;top:40px;width:100px;height:100px;background:#aaf';
  parent.append(source, destination);
  doc.body.appendChild(parent);
  const other = doc.createElement('div');
  other.id = 'other';
  other.style.cssText = 'position:fixed;left:30px;top:240px;width:500px;height:120px';
  doc.body.appendChild(other);
  const control = doc.createElement('input');
  control.type = 'checkbox'; control.id = 'control';
  control.style.cssText = 'position:fixed;left:540px;top:300px';
  doc.body.appendChild(control);
  if (mode === 'common-label') parent.htmlFor = 'control';
  if (mode === 'root-common') doc.body.appendChild(destination);
  if (mode === 'nested-common') {
    for (const [element, id] of [[source, 'branch1'], [destination, 'branch2']]) {
      const branch = doc.createElement('div'); branch.id = id;
      parent.appendChild(branch); branch.appendChild(element);
    }
  }
  const rows = [], clicks = [], checks = [];
  let getterReads = 0, activePointerId = null;
  const types = ['pointerdown', 'pointerup', 'pointermove', 'mousedown', 'mouseup',
                 'gotpointercapture', 'lostpointercapture', 'click', 'auxclick', 'contextmenu'];
  for (const type of types) win.addEventListener(type, e => {
    if (type === 'pointerdown') activePointerId = e.pointerId;
    const row = {type:e.type, target:e.target.id || e.target.nodeName,
      button:e.button, buttons:e.buttons, trusted:e.isTrusted, bubbles:e.bubbles,
      cancelable:e.cancelable, composed:e.composed, pointerId:e.pointerId,
      pointerType:e.pointerType, detail:e.detail,
      realm:e instanceof win.PointerEvent, ownerRealm:win === root || !(e instanceof root.PointerEvent),
      path:e.composedPath().map(v => v.id || v.nodeName || (v === win ? 'window' : 'unknown'))};
    rows.push(row);
    if (type === 'click' || type === 'auxclick') clicks.push(row);
  }, true);
  win.addEventListener('contextmenu', e => e.preventDefault());
  source.addEventListener('pointerdown', e => {
    const capture = mode === 'capture-source' ? source :
      ['capture-sibling', 'release-on-up', 'switch-on-up'].includes(mode) ? destination :
      ['capture-parent', 'release-before-up'].includes(mode) ? parent : null;
    if (capture) capture.setPointerCapture(e.pointerId);
    if (mode === 'cancel-pointerdown') e.preventDefault();
  });
  source.addEventListener('mousedown', e => {
    if (mode === 'cancel-mousedown') e.preventDefault();
  });
  destination.addEventListener('pointerup', e => {
    if (mode === 'release-on-up') destination.releasePointerCapture(e.pointerId);
    if (mode === 'switch-on-up') parent.setPointerCapture(e.pointerId);
  });
  win.addEventListener('mouseup', () => {
    if (mode === 'mouseup-reparent-up') other.appendChild(destination);
    if (mode === 'mouseup-reparent-down') other.appendChild(source);
    if (mode === 'mouseup-reparent-both') other.append(source, destination);
    if (mode === 'same-up-reparent') other.appendChild(source);
  }, {once:true});
  if (mode === 'poison-parent-getters') {
    for (const element of [source, destination, parent]) {
      for (const name of ['parentNode', 'parentElement', 'ownerDocument']) {
        Object.defineProperty(element, name, {get() {getterReads++; throw new Error(name);}});
      }
    }
  }
  const down = mode === 'down-parent-up-child' ? [470,80] : [80,80];
  const up = ['same', 'capture-sibling', 'release-on-up', 'switch-on-up', 'same-up-reparent'].includes(mode) ? [80,80] :
    mode === 'down-child-up-parent' ? [470,80] : [300,80];
  const expectedTarget = ['same', 'capture-source', 'same-up-reparent'].includes(mode) ? 'source' :
    ['capture-sibling', 'release-on-up', 'switch-on-up'].includes(mode) ? 'destination' :
    ['root-common', 'mouseup-reparent-up', 'mouseup-reparent-down'].includes(mode) ? 'body' :
    mode === 'mouseup-reparent-both' ? 'other' : 'parent';
  function check(name, actual, expected) {
    checks.push({name, actual, expected, pass:JSON.stringify(actual) === JSON.stringify(expected)});
  }
  return {down, up, prepareUp() {
    if (mode === 'release-before-up') parent.releasePointerCapture(activePointerId);
  }, finish(pointerType) {
    const activation = clicks.filter(e => e.target !== 'control');
    check('one click-like event', activation.length, 1);
    if (activation.length) {
      const click = activation[0];
      check('activation target', click.target, expectedTarget);
      check('activation type', click.type, button === 0 ? 'click' : 'auxclick');
      check('activation button', click.button, button);
      check('activation buttons', click.buttons, 0);
      check('activation pointer type', click.pointerType, pointerType);
      check('activation trusted', click.trusted, true);
      check('activation bubbles', click.bubbles, true);
      check('activation cancelable', click.cancelable, true);
      check('activation composed', click.composed, true);
      check('activation PointerEvent realm', click.realm, true);
      check('activation owner realm', click.ownerRealm, true);
      check('activation detail', click.detail, 1);
      const upIndex = rows.findIndex(e => e.type === 'pointerup');
      check('pointerup precedes activation', upIndex >= 0 && rows.indexOf(click) > upIndex, true);
    }
    check('no author getter reads', getterReads, 0);
    if (mode === 'common-label') {
      check('label default activation', control.checked, button === 0);
      check('one forwarded control click', clicks.filter(e=>e.target === 'control').length, button === 0 ? 1 : 0);
    }
    return {mode,button,inChild,rows,checks,getterReads,expectedTarget,controlChecked:control.checked,
      passed:checks.filter(c=>c.pass).length,total:checks.length,complete:checks.every(c=>c.pass)};
  }};
};
