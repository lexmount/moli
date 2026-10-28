(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  body.appendChild(frame);
  try {
    const owners = [
      ['main', document],
      ['child', frame.contentDocument],
      ['windowless', document.implementation.createHTMLDocument('')],
    ];
    const rows = [];
    for (const [owner, doc] of owners) {
      const registry = new CustomElementRegistry();
      const contextHost = doc.createElement('div');
      const context = contextHost.attachShadow({mode: 'open', customElementRegistry: registry});
      for (const mode of ['open', 'closed']) {
        for (const explicitNull of [false, true]) {
          const wrapper = doc.createElement('div');
          context.appendChild(wrapper);
          wrapper.setHTMLUnsafe('<article><template shadowrootmode="' + mode + '"' +
            (explicitNull ? ' shadowrootcustomelementregistry' : '') +
            '><b>inside</b></template><span customelementregistry></span></article>');
          const host = wrapper.firstElementChild;
          const root = mode === 'open' ? host.shadowRoot : host.attachShadow({mode});
          const expectedRegistry = explicitNull ? null : doc.customElementRegistry;
          const before = root.customElementRegistry === expectedRegistry;
          const nullElement = host.querySelector('span').customElementRegistry === null;
          const contextRegistry = context.customElementRegistry === registry;
          wrapper.setHTMLUnsafe('<p>replacement</p>');
          rows.push({owner, mode, explicitNull, before, nullElement, contextRegistry,
            retained: root.customElementRegistry === expectedRegistry});
          wrapper.remove();
        }
      }
    }
    return JSON.stringify({rows});
  } finally {
    frame.remove();
  }
})()
