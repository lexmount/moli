(() => {
  let state;
  globalThis.crossOwnerRegistryGc = {
    async setup(detached) {
      const popup = open('about:blank');
      const doc = popup.document;
      const registry = new CustomElementRegistry();
      const host = doc.createElement('div');
      host.attachShadow({mode: 'open', customElementRegistry: registry});
      doc.body.append(host);
      state = {popup, retained: detached ? host : doc, reference: new WeakRef(registry), detached};
      if (detached) {
        host.remove();
        popup.close();
        await new Promise(resolve => setTimeout(resolve, 500));
      }
      return 'ready';
    },
    retained() {
      const registry = state.reference.deref();
      const host = state.detached ? state.retained : state.retained.body.firstElementChild;
      return JSON.stringify({alive: registry !== undefined,
        identity: registry !== undefined && host.shadowRoot.customElementRegistry === registry});
    },
    release() {
      state.popup.close();
      state.popup = null;
      state.retained = null;
      return new Promise(resolve => setTimeout(resolve, 500));
    },
    collected() { return state.reference.deref() === undefined; },
  };
  return 'ready';
})()
