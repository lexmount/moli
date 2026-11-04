(() => {
  const rows = [];
  const factories = {
    document: () => document.implementation.createHTMLDocument('handlers'),
    shadow: () => {
      const host = document.createElement('div');
      document.body.appendChild(host);
      return host.attachShadow({mode: 'open'});
    }
  };
  const scenarios = {
    replacement(target, property, type, log, fire) {
      target.addEventListener(type, () => { log.push('before'); target[property] = () => log.push('replacement'); });
      target[property] = () => log.push('old');
      target.addEventListener(type, () => log.push('after'));
      fire();
      return 'before,replacement,after';
    },
    removeAndReadd(target, property, type, log, fire) {
      let changed = false;
      target.addEventListener(type, () => {
        log.push('before');
        if (!changed) { changed = true; target[property] = null; target[property] = () => log.push('new'); }
      });
      target[property] = () => log.push('old');
      target.addEventListener(type, () => log.push('after'));
      fire(); log.push('|'); fire();
      return 'before,after,|,before,after,new';
    },
    removal(target, property, type, log, fire) {
      target.addEventListener(type, () => { log.push('before'); target[property] = null; });
      target[property] = () => log.push('removed');
      target.addEventListener(type, () => log.push('after'));
      fire();
      return 'before,after';
    },
    immediateStop(target, property, type, log, fire) {
      target.addEventListener(type, () => log.push('before'));
      target[property] = event => { log.push('handler'); event.stopImmediatePropagation(); };
      target.addEventListener(type, () => log.push('after'));
      fire();
      return 'before,handler';
    },
    captureAddsHandler(target, property, type, log, fire) {
      target.addEventListener(type, () => log.push('before'));
      target.addEventListener(type, () => { log.push('capture'); target[property] = () => log.push('added'); }, {capture: true, once: true});
      target.addEventListener(type, () => log.push('after'));
      fire();
      return 'capture,before,after,added';
    },
    captureStopPropagation(target, property, type, log, fire) {
      target.addEventListener(type, event => { log.push('capture'); event.stopPropagation(); }, true);
      target[property] = () => log.push('handler');
      target.addEventListener(type, () => log.push('after'));
      fire();
      return 'capture';
    },
    handlerAddsListener(target, property, type, log, fire) {
      const late = () => log.push('late');
      target.addEventListener(type, () => log.push('before'));
      target[property] = () => { log.push('handler'); target.addEventListener(type, late, {once: true}); };
      target.addEventListener(type, () => log.push('after'));
      fire(); log.push('|'); fire();
      return 'before,handler,after,|,before,handler,after,late';
    }
  };
  for (const [kind, make] of Object.entries(factories)) {
    for (const [name, run] of Object.entries(scenarios)) {
      const target = make();
      const property = kind === 'document' ? 'onclick' : 'onslotchange';
      const type = property.slice(2);
      const log = [];
      const expected = run(target, property, type, log, () => target.dispatchEvent(new Event(type, {cancelable: true})));
      rows.push({name: kind + '/' + name, actual: log.join(), expected});
    }
  }
  return rows;
})()
