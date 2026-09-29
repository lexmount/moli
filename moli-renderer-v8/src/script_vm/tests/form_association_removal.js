(() => {
  if (!document.documentElement) document.append(document.createElement('html'));
  if (!document.body) document.documentElement.append(document.createElement('body'));
  const iframe = document.createElement('iframe');
  document.body.append(iframe);
  const failures = [];
  const check = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      failures.push(`${label}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
    }
  };
  for (const doc of [document, iframe.contentDocument]) {
    const win = doc.defaultView;
    class MovingFace extends win.HTMLElement {
      static formAssociated = true;
      constructor() { super(); this.internals = this.attachInternals(); this.events = []; }
      connectedCallback() { this.events.push('connected'); }
      disconnectedCallback() { this.events.push('disconnected'); }
      connectedMoveCallback() { this.events.push('moved'); }
      formAssociatedCallback(form) { this.events.push(`form:${form ? form.dataset.label : 'null'}`); }
      formDisabledCallback(value) { this.events.push(`disabled:${value}`); }
    }
    win.customElements.define('form-removal-face', MovingFace);
    const form = (id) => {
      const node = doc.createElement('form');
      node.id = id;
      node.dataset.label = id;
      return node;
    };
    for (const connected of [false, true]) {
      for (const explicit of [false, true]) {
        for (const operation of ['appendChild', 'insertBefore', 'append', 'prepend', 'before',
          'after', 'replaceChild', 'replaceWith', 'replaceChildren', 'selfAppend', 'selfReplace',
          'wrapper', 'remove', 'fragment']) {
          const label = `${doc === document ? 'main' : 'iframe'}/${connected}/${explicit}/${operation}`;
          const root = doc.createElement('div');
          const a = form('a'), b = form('b');
          const wrapper = doc.createElement('div');
          const sibling = doc.createElement('span');
          const face = doc.createElement('form-removal-face');
          if (explicit) face.setAttribute('form', 'a');
          wrapper.append(face); a.append(wrapper); b.append(sibling); root.append(a, b);
          if (connected) doc.body.append(root);
          face.events.length = 0;
          switch (operation) {
            case 'appendChild': b.appendChild(face); break;
            case 'insertBefore': b.insertBefore(face, sibling); break;
            case 'append': b.append(face); break;
            case 'prepend': b.prepend(face); break;
            case 'before': sibling.before(face); break;
            case 'after': sibling.after(face); break;
            case 'replaceChild': b.replaceChild(face, sibling); break;
            case 'replaceWith': sibling.replaceWith(face); break;
            case 'replaceChildren': b.replaceChildren(face); break;
            case 'selfAppend': wrapper.append(face); break;
            case 'selfReplace': wrapper.replaceChild(face, face); break;
            case 'wrapper': b.append(wrapper); break;
            case 'remove': face.remove(); break;
            case 'fragment': {
              const fragment = doc.createDocumentFragment();
              fragment.append(face);
              b.append(fragment);
              break;
            }
          }
          const owner = operation === 'remove' ? null :
            operation.startsWith('self') || (connected && explicit) ? a : b;
          const expected = connected ? ['disconnected', 'form:null'] : ['form:null'];
          if (operation !== 'remove') {
            if (connected) expected.push('connected');
            expected.push(`form:${owner.dataset.label}`);
          }
          check(face.events, expected, label);
          check(face.internals.form === owner, true, `${label}/owner`);
          root.remove();
        }
      }
    }
    for (const connected of [false, true]) {
      const owner = form('disabled-owner'), fieldset = doc.createElement('fieldset');
      fieldset.disabled = true;
      const face = doc.createElement('form-removal-face');
      fieldset.append(face); owner.append(fieldset);
      if (connected) doc.body.append(owner);
      face.events.length = 0;
      face.remove();
      check(face.events, [...(connected ? ['disconnected'] : []), 'form:null', 'disabled:false'],
        `remove disabled FACE/${connected}`);
      check(face.internals.form, null, 'removed disabled FACE owner');
      owner.remove();
    }
    // Removal of the old form must be observed before its same-ID replacement.
    for (const attached of [false, true]) {
      const root = doc.createElement('div'), oldForm = form('target'), newForm = form('target');
      oldForm.dataset.label = 'old'; newForm.dataset.label = 'new';
      const face = doc.createElement('form-removal-face'); face.setAttribute('form', 'target');
      root.append(oldForm, face);
      if (attached) root.append(newForm);
      doc.body.append(root); face.events.length = 0;
      root.replaceChild(newForm, oldForm);
      check(face.events, ['form:null', 'form:new'], `replace external form/${attached}`);
      check(face.internals.form === newForm, true, 'external replacement owner');
      root.remove();
    }
    // All replace-all entry points perform the same removal, including detached
    // forms, and retain one combined childList record for the target.
    for (const connected of [false, true]) {
      for (const method of ['textContent', 'textContentWithText', 'innerHTML', 'replaceChildren']) {
        const owner = form('bulk-owner'), face = doc.createElement('form-removal-face');
        owner.append(face);
        if (connected) doc.body.append(owner);
        const observer = new win.MutationObserver(() => {});
        observer.observe(owner, {childList: true}); face.events.length = 0;
        if (method === 'textContentWithText') owner.textContent = 'replacement';
        else if (method === 'replaceChildren') owner.replaceChildren();
        else owner[method] = '';
        const records = observer.takeRecords(); observer.disconnect();
        const label = `bulk removal/${connected}/${method}`;
        check(face.events, [...(connected ? ['disconnected'] : []), 'form:null'], label);
        check(face.internals.form, null, `${label}/owner`);
        check(records.length, 1, `${label}/records`);
        if (records.length) {
          check(Array.from(records[0].removedNodes).includes(face), true, `${label}/removed`);
          check(records[0].addedNodes.length, method === 'textContentWithText' ? 1 : 0, `${label}/added`);
        }
        owner.remove();
      }
    }
    // An implicit owner traveling in the same subtree stays associated.
    {
      const root = doc.createElement('div'), owner = form('carried');
      const face = doc.createElement('form-removal-face');
      owner.append(face); root.append(owner); doc.body.append(root); face.events.length = 0;
      root.append(owner);
      check(face.events, ['disconnected', 'connected'], 'move owner with descendant');
      face.events.length = 0;
      owner.remove();
      check(face.events, ['disconnected'], 'remove owner with descendant');
      check(face.internals.form === owner, true, 'detached subtree keeps implicit owner');
      root.remove();
    }
    // Explicit owners outside the moved subtree must see both ID lookup states.
    for (const duplicate of [false, true]) {
      const root = doc.createElement('div'), a = form('target'), b = form('other');
      if (duplicate) b.id = 'target';
      const face = doc.createElement('form-removal-face'); face.setAttribute('form', 'target');
      root.append(a, b, face); doc.body.append(root); face.events.length = 0;
      root.append(a);
      check(face.events, duplicate ? ['form:other'] : ['form:null', 'form:target'], 'move external owner');
      check(face.internals.form === (duplicate ? b : a), true, 'external owner after move');
      root.remove();
    }
    // A non-form element can mask the referenced ID during each phase, too.
    {
      const root = doc.createElement('div'), owner = form('target');
      const mask = doc.createElement('div'); mask.id = 'target';
      const face = doc.createElement('form-removal-face'); face.setAttribute('form', 'target');
      root.append(mask, owner, face); doc.body.append(root); face.events.length = 0;
      root.insertBefore(mask, mask);
      check(face.events, ['form:target', 'form:null'], 'remove and reinsert ID mask');
      check(face.internals.form, null, 'ID mask restored');
      root.remove();
    }
    // Pre-insertion validation must finish before any removal or reactions.
    {
      const owner = form('invalid'), face = doc.createElement('form-removal-face');
      owner.append(face); doc.body.append(owner); face.events.length = 0;
      let error = null;
      try { face.append(owner); } catch (e) { error = e.name; }
      check(error, 'HierarchyRequestError', 'cyclic insertion');
      check(face.events, [], 'invalid insertion has no reactions');
      check(face.internals.form === owner, true, 'invalid insertion retains owner');
      owner.remove();
    }
    // CE reactions can reenter the DOM after the original removal/insert has
    // completed. Callback arguments retain history; internals.form is live.
    {
      const root = doc.createElement('div'), a = form('a'), b = form('b'), c = form('c');
      const events = [];
      class ReentrantFace extends win.HTMLElement {
        static formAssociated = true;
        constructor() { super(); this.internals = this.attachInternals(); }
        formAssociatedCallback(owner) {
          events.push([owner && owner.id, this.internals.form && this.internals.form.id]);
          if (!owner && this.parentNode === b) c.append(this);
        }
      }
      win.customElements.define('reentrant-form-removal-face', ReentrantFace);
      const face = doc.createElement('reentrant-form-removal-face');
      root.append(a, b, c); a.append(face); doc.body.append(root); events.length = 0;
      b.append(face);
      check(events, [[null, 'b'], ['b', 'c'], [null, 'c'], ['c', 'c']], 'reentrant owner history');
      check(face.internals.form === c, true, 'reentrant final owner');
      root.remove();
    }
    // Focus reset belongs to removal, before the old event path disappears.
    {
      const a = doc.createElement('section'), b = doc.createElement('section');
      const button = doc.createElement('button'); a.append(button); doc.body.append(a, b);
      const events = [];
      const listener = event => { if (event.target === button) events.push(event.type); };
      doc.addEventListener('focusin', listener); doc.addEventListener('focusout', listener);
      button.focus(); b.moveBefore(button, null);
      check(events, ['focusin'], 'atomic move preserves focus');
      check(doc.activeElement === button, true, 'atomic active element');
      a.append(button);
      check(events, ['focusin'], 'ordinary move resets focus without events');
      button.focus(); button.remove();
      check(events, ['focusin', 'focusin'], 'direct removal resets focus without events');
      check(doc.activeElement === doc.body, true, 'removal resets active element');
      doc.removeEventListener('focusin', listener); doc.removeEventListener('focusout', listener);
      a.remove(); b.remove();
    }
    // A replacement's observer record must retain its timing relative to an
    // inserted classic script, as well as its eventual contents.
    for (const method of ['replaceChild', 'replaceChildren', 'textContent']) {
      const parent = doc.createElement(method === 'textContent' ? 'script' : 'div');
      const old = doc.createElement('span');
      if (method !== 'textContent') parent.append(old);
      doc.body.append(parent);
      const observer = new win.MutationObserver(() => {});
      observer.observe(parent, {childList: true});
      const describe = records => Array.from(records, record => ({
        added: Array.from(record.addedNodes, node => node.nodeName),
        removed: Array.from(record.removedNodes, node => node.nodeName)
      }));
      let during = null;
      win.__formRemovalObserveInsertion = () => { during = describe(observer.takeRecords()); };
      const script = doc.createElement('script');
      script.textContent = '__formRemovalObserveInsertion()';
      if (method === 'replaceChild') parent.replaceChild(script, old);
      else if (method === 'replaceChildren') parent.replaceChildren(script);
      else parent.textContent = '__formRemovalObserveInsertion()';
      const expected = [{added: [method === 'textContent' ? '#text' : 'SCRIPT'],
        removed: method === 'textContent' ? [] : ['SPAN']}];
      check(during, method === 'replaceChild' ? expected : [], `${method}/observer during script`);
      check(describe(observer.takeRecords()), method === 'replaceChild' ? [] : expected,
        `${method}/observer after script`);
      observer.disconnect(); parent.remove(); delete win.__formRemovalObserveInsertion;
    }
  }
  iframe.remove();
  if (failures.length) throw new Error(failures.join('\n'));
  return true;
})()
