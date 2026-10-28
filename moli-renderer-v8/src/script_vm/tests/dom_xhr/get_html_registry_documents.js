(() => {
  const container = document.createElement('section');
  const template = document.createElement('template');
  const inertDocument = template.content.ownerDocument;
  const makeHost = owner => {
    const host = owner.createElement('div');
    host.attachShadow({mode: 'open', serializable: true}).innerHTML = '<i>inside</i>';
    return host;
  };
  container.append(makeHost(document), template, makeHost(document));
  template.content.append(makeHost(inertDocument));

  const shadowMarkup = registryAttribute =>
    '<template shadowrootmode="open" shadowrootserializable=""' +
    (registryAttribute ? ' shadowrootcustomelementregistry=""' : '') +
    '><i>inside</i></template>';
  const expected = registryAttribute =>
    '<div>' + shadowMarkup(false) + '</div><template><div>' +
    shadowMarkup(registryAttribute) + '</div></template><div>' +
    shadowMarkup(false) + '</div>';
  const rows = [{
    id: 'template contents switch owner documents within one serialization',
    passed: inertDocument !== document &&
      inertDocument.customElementRegistry === null &&
      container.getHTML({serializableShadowRoots: true}) === expected(false)
  }];

  const registry = new CustomElementRegistry();
  const reads = [];
  const changed = container.getHTML({
    get serializableShadowRoots() {
      reads.push('serializableShadowRoots');
      registry.initialize(inertDocument);
      return true;
    },
    get shadowRoots() {
      reads.push('shadowRoots');
      return [];
    }
  });
  rows.push({
    id: 'document registry changes during option conversion apply to this and later calls',
    passed: reads.join(',') === 'serializableShadowRoots,shadowRoots' &&
      inertDocument.customElementRegistry === registry &&
      changed === expected(true) &&
      container.getHTML({serializableShadowRoots: true}) === changed
  });
  return JSON.stringify({rows});
})()
