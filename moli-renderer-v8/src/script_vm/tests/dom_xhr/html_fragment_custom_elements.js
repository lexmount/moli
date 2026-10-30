(() => {
  const rows = [], events = [];
  const frame = document.body.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  class FragmentChild extends HTMLElement {
    constructor() { super(); events.push('constructed'); }
    connectedCallback() { events.push('connected'); }
  }
  customElements.define('x-fragment-child', FragmentChild);
  const markup = '<x-fragment-child></x-fragment-child>';
  const replace = (target, method) => {
    if (method === 'innerHTML') target.innerHTML = markup;
    else target.setHTMLUnsafe(markup);
  };
  for (const origin of ['imperative', 'declarative']) {
    for (const method of ['innerHTML', 'setHTMLUnsafe']) {
      const container = document.body.appendChild(document.createElement('div'));
      let host, root;
      if (origin === 'imperative') {
        host = container.appendChild(document.createElement('section'));
        root = host.attachShadow({mode: 'open'});
      } else {
        container.setHTMLUnsafe('<section><template shadowrootmode="open"></template></section>');
        host = container.firstElementChild;
        root = host.shadowRoot;
      }
      events.length = 0;
      replace(root, method);
      rows.push({kind: 'shadow', origin, method, events: events.slice(),
        upgraded: root.firstElementChild instanceof FragmentChild});
      container.remove();
    }
  }
  for (const origin of ['created', 'cloned', 'imported', 'adopted']) {
    for (const method of ['innerHTML', 'setHTMLUnsafe']) {
      let template;
      if (origin === 'created') template = document.createElement('template');
      if (origin === 'cloned') template = document.createElement('template').cloneNode(true);
      if (origin === 'imported') template = document.importNode(other.document.createElement('template'), true);
      if (origin === 'adopted') template = document.adoptNode(other.document.createElement('template'));
      document.body.appendChild(template);
      events.length = 0;
      replace(template, method);
      const before = events.slice();
      const child = template.content.firstElementChild;
      const inert = !(child instanceof FragmentChild);
      document.body.appendChild(child);
      rows.push({kind: 'template', origin, method, before, inert,
        events: events.slice(), upgraded: child instanceof FragmentChild});
      child.remove(); template.remove();
    }
  }
  const realmInfo = {distinctObject: other.Object !== Object, distinctTypeError: other.TypeError !== TypeError};
  frame.remove();
  return {rows, realmInfo};
})()
