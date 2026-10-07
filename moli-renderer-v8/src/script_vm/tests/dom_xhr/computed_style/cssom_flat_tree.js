(() => {
  const checks = [];
  const record = (name, verify) => {
    try {
      checks.push({name, passed: verify() === true});
    } catch (error) {
      checks.push({name, passed: false, error: String(error)});
    }
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  const owners = [window, frame.contentWindow, popup];
  try {
    for (const [ownerIndex, owner] of owners.entries()) {
      const doc = owner.document;
      const callers = ownerIndex === 1 ? [window, owner] : [owner];
      const styles = (target, pseudo) => callers.map(caller => caller.getComputedStyle(target, pseudo));
      const observe = (name, declarations, connected) => {
        for (const [index, declaration] of declarations.entries()) {
          const label = `owner ${ownerIndex}/${name}/caller ${index}`;
          record(label + '/length', () => connected ? declaration.length > 0 : declaration.length === 0);
          record(label + '/color', () => declaration.color === (connected ? 'rgb(1, 2, 3)' : ''));
          record(label + '/font', () => declaration.fontSize === (connected ? '19px' : ''));
          record(label + '/custom', () => declaration.getPropertyValue('--flat-token') === (connected ? 'local' : ''));
          record(label + '/item', () => connected ? declaration.item(0) !== '' : declaration.item(0) === '');
          record(label + '/cssText', () => declaration.cssText === '');
        }
      };
      const create = () => {
        const target = doc.createElement('div');
        target.style.cssText = 'color: rgb(1, 2, 3); font-size: 19px; --flat-token: local';
        return target;
      };
      for (const [kind, target] of [['HTML', create()], ['SVG', doc.createElementNS('http://www.w3.org/2000/svg', 'svg')]]) {
        target.style.cssText = 'color: rgb(1, 2, 3); font-size: 19px; --flat-token: local';
        const detached = styles(target);
        observe(kind + '/detached', detached, false);
        doc.body.appendChild(target);
        const connected = styles(target);
        observe(kind + '/inserted held', detached, true);
        observe(kind + '/inserted fresh', connected, true);
        target.remove();
        observe(kind + '/removed held', connected, false);
        observe(kind + '/removed fresh', styles(target), false);
        doc.body.appendChild(target);
        observe(kind + '/reinserted detached held', detached, true);
        observe(kind + '/reinserted connected held', connected, true);
        target.remove();
      }
      const host = doc.createElement('section');
      doc.body.appendChild(host);
      const root = host.attachShadow({mode: 'closed'});
      const light = create(), descendant = create();
      light.appendChild(descendant);
      host.appendChild(light);
      const lightHeld = styles(light), descendantHeld = styles(descendant);
      observe('unslotted light', lightHeld, false);
      observe('unslotted descendant', descendantHeld, false);
      const slot = doc.createElement('slot');
      root.appendChild(slot);
      observe('assigned light held', lightHeld, true);
      observe('assigned descendant held', descendantHeld, true);
      const fallback = create(), fallbackDescendant = create();
      fallback.appendChild(fallbackDescendant);
      slot.appendChild(fallback);
      const fallbackHeld = styles(fallback), fallbackDescendantHeld = styles(fallbackDescendant);
      observe('inactive fallback', fallbackHeld, false);
      observe('inactive fallback descendant', fallbackDescendantHeld, false);
      light.remove();
      observe('active fallback held', fallbackHeld, true);
      observe('active fallback descendant held', fallbackDescendantHeld, true);
      host.appendChild(light);
      observe('inactive fallback again', fallbackHeld, false);
      observe('inactive fallback descendant again', fallbackDescendantHeld, false);
      slot.remove();
      observe('unassigned light held', lightHeld, false);
      observe('unassigned descendant held', descendantHeld, false);
      const nested = doc.createElement('article');
      const nestedRoot = nested.attachShadow({mode: 'open'});
      nested.appendChild(slot);
      root.appendChild(nested);
      observe('slot outside flat tree', lightHeld, false);
      observe('slot descendant outside flat tree', descendantHeld, false);
      nestedRoot.appendChild(doc.createElement('slot'));
      observe('slot enters flat tree', lightHeld, true);
      observe('slot descendant enters flat tree', descendantHeld, true);
      host.remove();
      observe('removed shadow host light', lightHeld, false);
      observe('removed shadow host descendant', descendantHeld, false);
      doc.body.appendChild(host);
      observe('reinserted shadow host light', lightHeld, true);
      observe('reinserted shadow host descendant', descendantHeld, true);
      host.remove();
      const fragment = doc.createDocumentFragment(), fragmentTarget = create();
      fragment.appendChild(fragmentTarget);
      const fragmentHeld = styles(fragmentTarget);
      observe('fragment', fragmentHeld, false);
      doc.body.appendChild(fragment);
      observe('fragment insertion held', fragmentHeld, true);
      fragmentTarget.remove();
    }
  } finally {
    popup.close();
    frame.remove();
  }
  const passed = checks.filter(row => row.passed).length;
  globalThis.__cssomFlatTreeResults = {complete: true, total: checks.length, passed, checks};
  globalThis.__uiEventResults = globalThis.__cssomFlatTreeResults;
  return true;
})()
