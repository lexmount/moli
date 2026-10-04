globalThis.__installCompatMouseTargetProbe = function (mode, trigger, captured, inChild) {
  const root = window;
  document.documentElement.style.cssText = 'margin:0;padding:0';
  document.body.style.cssText = 'margin:0;padding:0';
  let win = root, doc = document;
  if (inChild) {
    const frame = document.createElement('iframe');
    frame.style.cssText = 'position:fixed;left:0;top:0;width:600px;height:420px;border:0';
    document.body.appendChild(frame);
    win = frame.contentWindow; doc = frame.contentDocument;
  }
  doc.documentElement.style.cssText = 'margin:0;padding:0';
  doc.body.style.cssText = 'margin:0;padding:0;user-select:none';
  const nodes = [doc, win], names = new Map([[doc, 'document'], [win, 'window']]);
  function element(name, parent) {
    const node = doc.createElement('div'); node.id = name;
    names.set(node, name); nodes.push(node); parent.appendChild(node); return node;
  }
  const parent = element('parent', doc.body);
  parent.style.cssText = 'position:fixed;left:30px;top:30px;width:450px;height:180px';
  const branch = element('branch', parent);
  branch.style.cssText = 'position:fixed;left:40px;top:40px;width:180px;height:150px';
  const other = element('other', doc.body);
  other.style.cssText = 'position:fixed;left:300px;top:240px;width:180px;height:150px';
  let holder = branch, host = null, slot = null, slotParent = null;
  if (mode.startsWith('shadow-') || mode === 'nested-shadow-host' || mode.startsWith('slotted-')) {
    host = element('host', branch);
    host.style.cssText = 'position:fixed;left:40px;top:40px;width:180px;height:150px';
    const shadow = host.attachShadow({mode:mode.endsWith('closed') ? 'closed' : 'open'});
    nodes.push(shadow); names.set(shadow, 'shadow');
    if (mode === 'nested-shadow-host') {
      const innerHost = element('innerHost', shadow);
      innerHost.style.cssText = host.style.cssText;
      const innerShadow = innerHost.attachShadow({mode:'closed'});
      nodes.push(innerShadow); names.set(innerShadow, 'innerShadow');
      holder = innerShadow;
      host = innerHost;
    } else if (mode.startsWith('slotted-')) {
      slotParent = element('slotParent', shadow);
      slot = doc.createElement('slot'); slot.id = 'slot'; slotParent.appendChild(slot);
      nodes.push(slot); names.set(slot, 'slot');
      holder = host;
    } else {
      holder = shadow;
    }
  }
  const source = element('source', holder);
  source.style.cssText = 'position:fixed;left:50px;top:50px;width:100px;height:100px;background:#acf';
  const expected = mode === 'remove-target' || mode === 'remove-and-throw' || mode === 'poison-native-relations' || mode === 'adopt-detached-document' ? branch :
    mode === 'remove-parent' ? parent :
    mode.startsWith('shadow-host-') ? branch :
    mode.startsWith('shadow-child-') ? host :
    mode === 'nested-shadow-host' ? holder.host.parentNode.host :
    mode === 'slotted-target' ? slot :
    mode === 'remove-document-element' ? doc : source;
  const beforeDocument = doc;
  const observations = [], events = [], seen = new WeakMap(), checks = [];
  let phase = 'hover', getterReads = 0, mutated = false;
  const mouseType = trigger === 'down' ? 'mousedown' : trigger === 'up' ? 'mouseup' : 'mousemove';
  const pointerType = trigger === 'down' ? 'pointerdown' : trigger === 'up' ? 'pointerup' : 'pointermove';
  function observe(e) {
    if (phase !== trigger || ![mouseType, pointerType].includes(e.type)) return;
    let row = seen.get(e);
    if (!row) {
      row = {type:e.type, button:e.button, buttons:e.buttons, clientX:e.clientX, clientY:e.clientY,
        trusted:e.isTrusted, bubbles:e.bubbles, cancelable:e.cancelable, composed:e.composed,
        realm:e instanceof win.MouseEvent, ownerRealm:win === root || !(e instanceof root.MouseEvent),
        view:e.view === win, matchedExpected:false, matchedSource:false};
      seen.set(e, row); events.push(row);
    }
    const path = e.composedPath();
    observations.push({type:e.type, listener:names.get(e.currentTarget), target:names.get(e.target),
      path:path.map(node => names.get(node) || node.nodeName || 'unknown')});
    if (e.currentTarget === expected && e.target === expected && path[0] === expected) row.matchedExpected = true;
    if (e.currentTarget === source && e.target === source && path[0] === source) row.matchedSource = true;
  }
  for (const node of nodes) {
    node.addEventListener(mouseType, observe, true);
    node.addEventListener(pointerType, observe, true);
  }
  source.addEventListener('pointerdown', e => { if (captured) source.setPointerCapture(e.pointerId); });
  source.addEventListener(pointerType, () => {
    if (phase !== trigger || mutated) return;
    mutated = true;
    if (['remove-target', 'remove-and-throw', 'poison-native-relations', 'slotted-target'].includes(mode) || mode.startsWith('shadow-child-')) source.remove();
    if (mode === 'remove-parent') branch.remove();
    if (mode.startsWith('shadow-host-') || mode === 'nested-shadow-host') host.remove();
    if (mode === 'slotted-slot') slot.remove();
    if (mode === 'slotted-slot-parent') slotParent.remove();
    if (mode === 'reparent-same-document') other.appendChild(source);
    if (mode === 'remove-reinsert') { source.remove(); holder.appendChild(source); }
    if (mode === 'adopt-detached-document') beforeDocument.implementation.createHTMLDocument('').body.appendChild(source);
    if (mode === 'remove-document-element') beforeDocument.documentElement.remove();
    if (mode === 'remove-and-throw') throw new win.Error('intentional removed-target listener exception');
  });
  win.addEventListener('error', e => e.preventDefault());
  win.addEventListener('contextmenu', e => e.preventDefault());
  if (mode === 'poison-native-relations') {
    for (const node of [source, branch, parent]) {
      for (const key of ['parentNode', 'ownerDocument', 'isConnected', 'assignedSlot']) {
        Object.defineProperty(node, key, {get() {getterReads++; throw new win.Error(key);}});
      }
    }
  }
  function check(name, actual, wanted) {
    checks.push({name, actual, expected:wanted, pass:JSON.stringify(actual) === JSON.stringify(wanted)});
  }
  return {down:[80,80], up:[85,80], prepare(value) {phase = value;}, finish(pointer) {
    const mouse = events.filter(e => e.type === mouseType), pointerEvents = events.filter(e => e.type === pointerType);
    check('pointer event at the original target', pointerEvents.length, 1);
    if (pointerEvents.length) check('original pointer target preserved', pointerEvents[0].matchedSource, true);
    check('listener mutation ran', mutated, true);
    check('one compatibility mouse event', mouse.length, 1);
    if (mouse.length) {
      const event = mouse[0];
      check('compatibility target from original path', event.matchedExpected, true);
      check('mouse button', event.button, 0);
      check('mouse buttons', event.buttons, trigger === 'up' ? 0 : 1);
      check('mouse coordinates', [event.clientX,event.clientY], trigger === 'down' ? [80,80] : [85,80]);
      check('trusted compatibility event', event.trusted, true);
      check('compatibility bubbles', event.bubbles, true);
      check('compatibility cancelable', event.cancelable, true);
      check('compatibility composed', event.composed, true);
      check('owner MouseEvent realm', event.realm, true);
      check('child constructor identity', event.ownerRealm, true);
      check('view identifies original Window', event.view, true);
      check('pointer precedes compatibility event', events.indexOf(pointerEvents[0]) < events.indexOf(event), true);
    }
    check('native relation lookup does not call author getters', getterReads, 0);
    return {mode,trigger,captured,inChild,pointer,expectedTarget:names.get(expected),events,observations,checks,
      getterReads,mutated,passed:checks.filter(c=>c.pass).length,total:checks.length,complete:checks.every(c=>c.pass)};
  }};
};
