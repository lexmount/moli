(async () => {
  const rows = [];
  for (const registryRealm of ['main', 'popup']) {
    for (const superRealm of ['main', 'popup']) {
      const popup = open('about:blank');
      try {
        popup.addEventListener('error', event => event.preventDefault());
        const owner = registryRealm === 'main' ? window : popup;
        const target = registryRealm === 'main' ? popup : window;
        const doc = target.document;
        const registry = new owner.CustomElementRegistry();
        const base = superRealm === 'main' ? HTMLElement : popup.HTMLElement;
        const name = `cross-super-${registryRealm}-${superRealm}`;
        let calls = 0;
        const Constructor = class extends base {
          constructor() { super(); ++calls; }
        };
        const host = doc.createElement('div');
        const shadow = host.attachShadow({mode: 'open', customElementRegistry: registry});
        const existing = doc.createElement(name, {customElementRegistry: registry});
        shadow.append(existing);
        doc.body.append(host);
        registry.define(name, Constructor);
        let created;
        let exception = null;
        try { created = doc.createElement(name, {customElementRegistry: registry}); }
        catch (error) { exception = error.name; }
        rows.push({case: `${registryRealm}-registry-${superRealm}-super`,
          upgraded: existing instanceof Constructor,
          created: created instanceof Constructor,
          existingOwner: existing.ownerDocument === doc,
          createdOwner: created?.ownerDocument === doc,
          constructorCalls: calls, association: existing.customElementRegistry === registry, exception});
        host.remove();
      } finally { popup.close(); }
    }
  }
  return JSON.stringify({rows});
})()
