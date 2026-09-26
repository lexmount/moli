(() => {
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const cases = [];
  const button = '<button id="focus">focus</button>';
  const add = (name, change, html = button, extra = {}) =>
    cases.push({name, change, html, expected: 'BODY', ...extra});

  add('disabled property', c => { c.focus.disabled = true; });
  add('disabled attribute', c => c.focus.setAttribute('disabled', ''));
  add('fieldset disabled property', c => { c.focus.parentNode.disabled = true; },
      '<fieldset>' + button + '</fieldset>');
  add('fieldset disabled attribute', c => c.focus.parentNode.setAttribute('disabled', ''),
      '<fieldset>' + button + '</fieldset>');
  add('first legend stays enabled', c => { c.root.querySelector('fieldset').disabled = true; },
      '<fieldset><legend>' + button + '</legend></fieldset>', {expected: 'focus'});
  add('replacing the first legend disables the control', c => {
    const fieldset = c.root.querySelector('fieldset');
    fieldset.prepend(document.createElement('legend'));
  }, '<fieldset disabled><legend>' + button + '</legend></fieldset>');
  add('input becomes hidden', c => { c.focus.type = 'hidden'; }, '<input id="focus">');
  add('tabindex removed', c => c.focus.removeAttribute('tabindex'), '<div id="focus" tabindex="0">focus</div>');
  add('negative tabindex stays focusable', c => { c.focus.tabIndex = -1; }, button, {expected: 'focus'});
  add('readonly stays focusable', c => { c.focus.readOnly = true; }, '<input id="focus">', {expected: 'focus'});
  add('editing host becomes uneditable', c => { c.focus.contentEditable = 'false'; },
      '<div id="focus" contenteditable>focus</div>');
  add('anchor loses href', c => c.focus.removeAttribute('href'), '<a id="focus" href="#">focus</a>');
  add('hidden attribute', c => { c.focus.hidden = true; });
  add('display none', c => { c.focus.style.display = 'none'; });
  add('visibility hidden', c => { c.focus.style.visibility = 'hidden'; });
  add('hidden ancestor', c => { c.focus.parentNode.hidden = true; }, '<section>' + button + '</section>');
  add('inert ancestor', c => { c.focus.parentNode.inert = true; }, '<section>' + button + '</section>');
  add('inert control', c => { c.focus.inert = true; });
  add('class hides control', c => { c.focus.className = 'hidden-control'; },
      '<style>.hidden-control { display: none }</style>' + button);
  add('unrelated disabled sibling', c => { c.other.disabled = true; }, button, {expected: 'focus'});
  add('reenabled before rendering', c => { c.focus.disabled = true; c.focus.disabled = false; }, button, {expected: 'focus'});
  add('unhidden before rendering', c => { c.focus.hidden = true; c.focus.hidden = false; }, button, {expected: 'focus'});
  add('reenabled in animation callback', c => { c.focus.disabled = true; }, button,
      {expected: 'focus', duringFrame: c => { c.focus.disabled = false; }});
  add('focus moves before rendering', c => { c.focus.disabled = true; c.other.focus(); }, button,
      {before: 'other', expected: 'other'});
  add('focus moves in animation callback', c => { c.focus.disabled = true; }, button,
      {frame: 'other', expected: 'other', duringFrame: c => c.other.focus()});
  add('blur handler moves focus', c => {
    c.focus.addEventListener('blur', () => c.other.focus(), {once: true});
    c.focus.disabled = true;
  }, button, {expected: 'other', reentrant: true});
  for (const mode of ['open', 'closed']) {
    for (const changeHost of [false, true]) {
      add(`${mode} shadow ${changeHost ? 'inert host' : 'disabled control'}`, c => {
        if (changeHost) c.host.inert = true;
        else c.focus.disabled = true;
      }, '<div id="host"></div>', {prepare(c) {
        c.host = c.root.querySelector('#host');
        const shadow = c.host.attachShadow({mode});
        shadow.innerHTML = button;
        c.shadows.push(shadow);
        c.focus = shadow.firstChild;
      }});
    }
  }
  add('child document disabled from its parent realm', c => { c.focus.disabled = true; },
      '<iframe id="frame"></iframe>', {prepare(c) {
        c.window = c.root.firstChild.contentWindow;
        c.window.document.body.innerHTML = button;
        c.focus = c.window.document.getElementById('focus');
      }});
  let current;
  const active = c => {
    let element = c.window.document.activeElement;
    for (;;) {
      const shadow = c.shadows.find(root => root.host === element);
      if (!shadow?.activeElement) break;
      element = shadow.activeElement;
    }
    return element?.id || element?.tagName || null;
  };
  globalThis.__focusFixup = {
    names: cases.map(c => c.name),
    setup(index) {
      document.body.innerHTML = '<main id="root"></main><button id="other">other</button>';
      const item = cases[index];
      const c = {window, root: document.getElementById('root'), other: document.getElementById('other'), shadows: [], events: [], timerGets: 0};
      c.root.innerHTML = item.html;
      c.focus = c.root.querySelector('#focus');
      item.prepare?.(c);
      c.focus.focus();
      if (active(c) !== 'focus') throw new Error('initial focus failed: ' + item.name);
      for (const type of ['blur', 'focusout']) c.focus.addEventListener(type, event => {
        c.events.push({type, trusted: event.isTrusted, nullRelatedTarget: event.relatedTarget === null});
      });
      const raf = c.window.requestAnimationFrame.bind(c.window);
      const descriptors = new Map();
      for (const name of ['setTimeout', 'requestAnimationFrame']) {
        descriptors.set(name, Object.getOwnPropertyDescriptor(c.window, name));
        Object.defineProperty(c.window, name, {configurable: true, get() {
          c.timerGets++;
          throw new Error('focus fixup accessed author ' + name);
        }});
      }
      c.restore = () => {
        for (const [name, descriptor] of descriptors) Object.defineProperty(c.window, name, descriptor);
      };
      current = c;
      try {
        item.change(c);
        c.sync = active(c);
        c.expectedBefore = item.before || 'focus';
        c.expectedFrame = item.frame || c.expectedBefore;
        c.expected = item.expected;
        c.reentrant = !!item.reentrant;
        c.done = new Promise(resolve => raf(() => {
          item.duringFrame?.(c);
          c.frame = active(c);
          raf(() => { c.final = active(c); c.restore(); resolve(); });
        }));
        queueMicrotask(() => { c.microtask = active(c); });
      } catch (error) { c.restore(); throw error; }
      return item.name;
    },
    done() { return current.done; },
    snapshot() {
      const c = current;
      return {sync: c.sync, microtask: c.microtask, frame: c.frame, final: c.final,
              expectedBefore: c.expectedBefore, expectedFrame: c.expectedFrame,
              expected: c.expected, events: c.events, timerGets: c.timerGets,
              reentrant: c.reentrant};
    },
  };
  return cases.length;
})()
