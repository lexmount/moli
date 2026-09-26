(() => {
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const cases = [];
  const row = '<button id="prev">previous</button><button id="focus">focus</button><button id="next">next</button>';
  const add = (name, change, html = row, expected = ['next', 'prev']) =>
    cases.push({ name, change, html, expected });
  add('remove focused control', c => c.focus.remove());
  add('remove focused control and previous sibling', c => { c.focus.remove(); c.root.querySelector('#gone').remove(); },
      '<button id="prev">previous</button><button id="gone">gone</button><button id="focus">focus</button><button id="next">next</button>');
  add('remove focused control and next sibling', c => { c.focus.remove(); c.root.querySelector('#gone').remove(); },
      '<button id="prev">previous</button><button id="focus">focus</button><button id="gone">gone</button><button id="next">next</button>');
  add('remove ancestor', c => c.focus.parentNode.remove(),
      '<button id="prev">previous</button><section><button id="focus">focus</button></section><button id="next">next</button>');
  add('remove control then ancestor', c => { const parent = c.focus.parentNode; c.focus.remove(); parent.remove(); },
      '<button id="prev">previous</button><section><button id="focus">focus</button></section><button id="next">next</button>');
  add('insert after the gap', c => { const next = c.root.querySelector('#next'); c.focus.remove(); next.before(Object.assign(document.createElement('button'), { id: 'inserted' })); }, row, ['inserted', 'prev']);
  add('insert before the anchor', c => { const prev = c.root.querySelector('#prev'); c.focus.remove(); prev.before(Object.assign(document.createElement('button'), { id: 'inserted' })); });
  add('grow previous sibling subtree', c => { c.focus.remove(); c.root.querySelector('section').append(Object.assign(document.createElement('button'), { id: 'prev' })); },
      '<section><span>prefix</span></section><button id="focus">focus</button><button id="next">next</button>');
  add('backward search includes preceding descendants', c => c.focus.remove(),
      '<section><button id="prev">previous</button></section><button id="focus">focus</button><button id="next">next</button>');
  add('move previous sibling', c => { c.focus.remove(); c.root.querySelector('#away').append(c.root.querySelector('#gone')); },
      '<button id="prev">previous</button><button id="gone">gone</button><button id="focus">focus</button><button id="next">next</button><section id="away"></section>');
  add('move previous sibling atomically', c => { c.focus.remove(); c.root.querySelector('#away').moveBefore(c.root.querySelector('#gone'), null); },
      '<button id="prev">previous</button><button id="gone">gone</button><button id="focus">focus</button><button id="next">next</button><section id="away"></section>');
  add('move next sibling atomically', c => { c.focus.remove(); c.root.querySelector('#away').moveBefore(c.root.querySelector('#gone'), null); },
      '<button id="prev">previous</button><button id="focus">focus</button><button id="gone">gone</button><button id="next">next</button><section id="away"></section>');
  add('normal move leaves original gap', c => c.root.querySelector('#away').append(c.focus),
      row + '<section id="away"></section>');
  for (const blur of ['before', 'after', 'neither']) {
    add(`atomic move navigation, blur ${blur}`, c => {
      if (blur === 'before') c.focus.blur();
      c.root.querySelector('#away').moveBefore(c.focus, c.root.querySelector('#otherNext'));
      if (blur === 'after') c.focus.blur();
    }, row + '<section id="away"><button id="otherPrev">other previous</button><button id="otherNext">other next</button></section>', blur === 'neither' ? ['otherNext', 'otherPrev'] : ['next', 'prev']);
  }
  add('adoption leaves original gap', c => document.implementation.createHTMLDocument('').adoptNode(c.focus));
  add('replaceChild self leaves original gap', c => c.focus.parentNode.replaceChild(c.focus, c.focus), row, ['focus', 'prev']);
  add('replaceWith self leaves original gap', c => c.focus.replaceWith(c.focus), row, ['focus', 'prev']);
  add('split previous text keeps the position after both halves', c => {
    const text = c.root.querySelector('#prev').nextSibling;
    c.focus.remove();
    const second = text.splitText(1);
    second.before(Object.assign(document.createElement('button'), {id:'inserted'}));
  }, '<button id="prev">previous</button>ab<button id="focus">focus</button><button id="next">next</button>', ['next', 'inserted']);
  add('blur keeps element origin', c => c.focus.blur());
  add('disabled element remains a DOM origin', c => { c.focus.disabled = true; });
  add('disable and reenable keeps element origin', c => { c.focus.disabled = true; c.focus.disabled = false; });
  add('negative tabindex uses DOM order', c => { c.focus.tabIndex = -1; });
  add('removed positive tabindex uses DOM order', c => c.focus.remove(),
      '<button id="priority" tabindex="1">priority</button><button id="prev" tabindex="4">previous</button><button id="focus" tabindex="2">focus</button><button id="next" tabindex="3">next</button>');
  add('disabled positive tabindex uses DOM order', c => { c.focus.disabled = true; },
      '<button id="priority" tabindex="1">priority</button><button id="prev" tabindex="4">previous</button><button id="focus" tabindex="2">focus</button><button id="next" tabindex="3">next</button>');
  add('reinsert removed control does not revive old identity', c => { c.focus.remove(); c.root.prepend(c.focus); });
  add('subsequent focus overrides the old gap', c => { c.focus.remove(); c.root.querySelector('#override').focus(); },
      row + '<button id="override">override</button><button id="last">last</button>', ['last', 'next']);
  for (const mode of ['open', 'closed']) {
    cases.push({
      name: `${mode} shadow host replaced with itself`,
      html: '<button id="prev">previous</button><div id="host"></div><button id="next">next</button>',
      prepare(c) {
        c.host = c.root.querySelector('#host');
        const shadow = c.host.attachShadow({ mode });
        shadow.innerHTML = '<button id="focus">focus</button>';
        c.focus = shadow.firstChild;
        c.shadows.push(shadow);
      },
      change: c => c.host.parentNode.replaceChild(c.host, c.host),
      expected: ['focus', 'prev'],
    });
    for (const removeHost of [false, true]) {
      cases.push({
        name: `${mode} shadow ${removeHost ? 'host' : 'control'} removal`,
        html: '<button id="prev">previous</button><div id="host"></div><button id="next">next</button>',
        prepare(c) {
          const host = c.root.querySelector('#host');
          const shadow = host.attachShadow({ mode });
          shadow.innerHTML = '<button id="focus">focus</button>';
          c.focus = shadow.firstChild;
          c.host = host;
          c.shadows.push(shadow);
        },
        change: c => removeHost ? c.host.remove() : c.focus.remove(),
        expected: ['next', 'prev'],
      });
      cases.push({
        name: `${mode} shadow internal removal ${removeHost ? 'ancestor' : 'control'}`,
        html: '<div id="host"></div>',
        prepare(c) {
          const shadow = c.root.firstChild.attachShadow({ mode });
          shadow.innerHTML = '<button id="prev">previous</button><section><button id="focus">focus</button></section><button id="next">next</button>';
          c.focus = shadow.querySelector('#focus');
          c.shadows.push(shadow);
        },
        change: c => removeHost ? c.focus.parentNode.remove() : c.focus.remove(),
        expected: ['next', 'prev'],
      });
    }
  }
  let current;
  const selection = () => {
    const s = getSelection();
    return [s.anchorNode === document.querySelector('#selection').firstChild,
            s.anchorOffset, s.focusNode === s.anchorNode, s.focusOffset, s.direction];
  };
  globalThis.__focusNavigation = {
    names: cases.map(c => c.name),
    setup(index, reverse, preserveSelection = true) {
      document.body.innerHTML = '<p id="selection">preserve</p><button id="before">before</button><main id="root"></main><button id="after">after</button>';
      const c = { root: document.querySelector('#root'), shadows: [] };
      const item = cases[index];
      c.root.innerHTML = item.html;
      c.focus = c.root.querySelector('#focus');
      item.prepare?.(c);
      if (preserveSelection) {
        const text = document.querySelector('#selection').firstChild;
        getSelection().setBaseAndExtent(text, 1, text, 4);
      }
      c.focus.focus();
      item.change(c);
      current = { ...c, expected: item.expected[reverse ? 1 : 0],
                  beforeSelection: preserveSelection ? selection() : null };
      return { name: item.name, expected: current.expected };
    },
    snapshot() {
      let active = document.activeElement;
      for (;;) {
        const shadow = current.shadows.find(root => root.host === active);
        if (!shadow?.activeElement) break;
        active = shadow.activeElement;
      }
      return { actual: active.id, expected: current.expected,
               selectionPreserved: current.beforeSelection === null ||
                 JSON.stringify(selection()) === JSON.stringify(current.beforeSelection) };
    },
  };
  return cases.length;
})()
