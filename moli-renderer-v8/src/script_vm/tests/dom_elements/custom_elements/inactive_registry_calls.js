(async () => {
  const rows = [];
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  for (const kind of ['global', 'scoped']) {
    const frame = document.createElement('iframe');
    body.appendChild(frame);
    const owner = frame.contentWindow;
    const registry = kind === 'global' ? owner.customElements : new owner.CustomElementRegistry();
    const Constructor = class extends owner.HTMLElement {};
    frame.remove();
    const name = `detached-${kind}-late`;
    const methods = CustomElementRegistry.prototype;
    let resolved = false;
    const pending = methods.whenDefined.call(registry, name);
    pending.then(() => { resolved = true; });
    methods.define.call(registry, name, Constructor);
    await Promise.resolve();
    rows.push({case: kind,
      promise: Object.prototype.toString.call(pending) === '[object Promise]',
      samePending: methods.whenDefined.call(registry, name) === pending,
      noLateDefinition: methods.get.call(registry, name) === undefined,
      pending: !resolved,
    });
  }
  return JSON.stringify({rows});
})()
