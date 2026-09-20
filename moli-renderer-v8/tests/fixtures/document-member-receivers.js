async function documentMemberReceiverProbe() {
  const failures = [];
  const child = document.getElementById('document-member-child').contentWindow;
  let traps = 0;
  let conversions = 0;
  const poison = new Proxy({}, {
    get() { conversions++; throw new Error('argument conversion'); }
  });
  const revoked = Proxy.revocable(document, {});
  revoked.revoke();
  const invalid = [
    undefined, null, false, 1, 'document', Symbol('document'), 1n, {}, window,
    Document.prototype, HTMLDocument.prototype, XMLDocument.prototype,
    Object.create(Document.prototype), Object.create(document),
    Object.create(child.document), document.createElement('div'),
    document.createTextNode('text'), document.createDocumentFragment(),
    new Proxy(document, {
      get() { traps++; throw new Error('receiver get trap'); },
      getPrototypeOf() { traps++; throw new Error('receiver prototype trap'); }
    }), revoked.proxy, new Proxy(child.document, {})
  ];
  const promiseMethods = new Set(['hasStorageAccess', 'requestStorageAccess']);
  const node = document.createElement('div');
  const nodeArguments = {
    importNode: [node, false], adoptNode: [node], moveBefore: [node, null],
    createNodeIterator: [document], createTreeWalker: [document],
    createNSResolver: [document], evaluate: ['.', document, null, 0, null]
  };
  for (const [realmName, realm] of [['parent', window], ['child', child]]) {
    // Discover every installed member so newly added Document members cannot
    // silently escape the same receiver contract. Inherited Node methods have
    // their own interface contract and are tested separately.
    for (const C of [realm.Document, realm.HTMLDocument, realm.XMLDocument]) {
      for (const [name, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(C.prototype))) {
        if (name === 'constructor') continue;
        for (const kind of ['get', 'set', 'value']) {
          const fn = descriptor[kind];
          if (typeof fn !== 'function') continue;
          const label = `${realmName}/${C.name}.${name}/${kind}`;
          const lenient = name === 'onmouseenter' || name === 'onmouseleave';
          const argumentLists = kind === 'get' ? [[]] : kind === 'set' ? [[poison]] :
            [[], Array(5).fill(poison), ...(nodeArguments[name] ? [nodeArguments[name]] : [])];
          const errors = [];
          for (const args of argumentLists) {
            for (const [index, receiver] of invalid.entries()) {
              let result;
              let caught;
              try { result = Reflect.apply(fn, receiver, args); }
              catch (error) { caught = error; }
              if (promiseMethods.has(name)) {
                if (caught || !(result instanceof realm.Promise)) {
                  errors.push([index, caught ? `threw ${caught.name}` : 'not a callee Promise']);
                  continue;
                }
                try { await result; } catch (error) { caught = error; }
              }
              if (lenient) {
                if (caught || result !== undefined) errors.push([index, caught ? caught.name : 'returned a value']);
              } else if (!caught || Object.getPrototypeOf(caught) !== realm.TypeError.prototype) {
                errors.push([index, caught ? caught.name : 'accepted']);
              }
            }
          }
          if (errors.length) failures.push([label, errors]);
        }
      }
    }
  }
  if (traps) failures.push(['receiver traps', traps]);
  if (conversions) failures.push(['argument conversions', conversions]);
  return failures;
}

async function documentGenuineReceiverProbe() {
  const failures = [];
  const check = (label, condition) => { if (!condition) failures.push(label); };
  const child = document.getElementById('document-member-child').contentWindow;
  const collections = ['forms', 'images', 'scripts', 'links', 'anchors', 'embeds', 'plugins', 'applets'];
  const html = '<form id="form" name="named"></form><img id="image">' +
    '<script id="script" type="text/plain"></script><a id="link" name="anchor" href="#"></a>' +
    '<embed id="embed"><div id="target" class="target" name="named"></div>';
  for (const [realmName, realm] of [['parent', window], ['child', child]]) {
    const proto = realm.Document.prototype;
    const get = (name, doc) => Object.getOwnPropertyDescriptor(proto, name).get.call(doc);
    for (const owner of [window, child]) {
      const doc = owner.document.implementation.createHTMLDocument('collections');
      doc.body.innerHTML = html;
      const expectedIds = ['form', 'image', 'script', 'link', 'link', 'embed', 'embed', ''];
      const form = doc.getElementById('form');
      const target = doc.getElementById('target');
      const root = doc.documentElement;
      Object.setPrototypeOf(doc, null);
      for (const [index, name] of collections.entries()) {
        const collection = get(name, doc);
        check(`${realmName}/${name}/contents`, Array.from(collection, node => node.id).join() === expectedIds[index]);
        check(`${realmName}/${name}/length`, collection.length === (name === 'applets' ? 0 : 1));
      }
      form.remove();
      check(`${realmName}/forms/removed`, get('forms', doc).length === 0);
      for (const [name, args, expectedNode] of [
        ['getElementById', ['target'], target], ['querySelector', ['.target'], target],
        ['getElementsByTagName', ['div'], target],
        ['getElementsByTagNameNS', ['http://www.w3.org/1999/xhtml', 'div'], target],
        ['getElementsByClassName', ['target'], target], ['getElementsByName', ['named'], target],
        ['querySelectorAll', ['.target'], target]
      ]) {
        const result = Reflect.apply(proto[name], doc, args);
        check(`${realmName}/${name}/native`, (result[0] || result) === expectedNode);
      }
      check(`${realmName}/children/native`, get('children', doc)[0] === root);
      check(`${realmName}/firstElementChild/native`, get('firstElementChild', doc) === root);
      check(`${realmName}/lastElementChild/native`, get('lastElementChild', doc) === root);
      check(`${realmName}/childElementCount/native`, get('childElementCount', doc) === 1);
      check(`${realmName}/createRange/native`, proto.createRange.call(doc).startContainer === doc);
      check(`${realmName}/createEvent/native`, proto.createEvent.call(doc, 'Event').type === '');
      check(`${realmName}/createElement/native`, proto.createElement.call(doc, 'section').ownerDocument === doc);
      check(`${realmName}/createTextNode/native`, proto.createTextNode.call(doc, 'text').ownerDocument === doc);
      check(`${realmName}/createDocumentFragment/native`, proto.createDocumentFragment.call(doc).ownerDocument === doc);
      for (const name of ['clear', 'captureEvents', 'releaseEvents']) {
        check(`${realmName}/${name}/native`, proto[name].call(doc) === undefined);
      }
      const design = Object.getOwnPropertyDescriptor(proto, 'designMode');
      design.set.call(doc, 'on');
      check(`${realmName}/designMode/native`, design.get.call(doc) === 'on');
      const fullscreen = Object.getOwnPropertyDescriptor(proto, 'fullscreenEnabled');
      const before = fullscreen.get.call(doc);
      const poison = new Proxy({}, {get() { throw new Error('lenient setter conversion'); }});
      check(`${realmName}/fullscreenEnabled/lenient-set`, fullscreen.set.call(doc, poison) === undefined);
      check(`${realmName}/fullscreenEnabled/unchanged`, fullscreen.get.call(doc) === before);
    }
    // HTML and XML documents, subclass instances and a changed prototype all
    // retain native identity even when ordinary instanceof checks cannot work.
    const changed = new child.Document();
    Object.setPrototypeOf(changed, null);
    for (const doc of [document, child.document, new Document(), new child.Document(),
      new (class extends Document {})(), changed,
      new DOMParser().parseFromString('<root/>', 'application/xml')]) {
      for (const name of collections) {
        try { get(name, doc); } catch (error) { failures.push(`${realmName}/${name}/genuine: ${error.name}`); }
      }
      check(`${realmName}/factory/genuine`, proto.createComment.call(doc, 'comment').ownerDocument === doc);
    }
    for (const doc of [document, child.document]) {
      const forms = get('forms', doc);
      const length = forms.length;
      const form = doc.body.appendChild(doc.createElement('form'));
      check(`${realmName}/forms/live-insert`, forms.length === length + 1);
      form.remove();
      check(`${realmName}/forms/live-remove`, forms.length === length);
      for (const name of ['hasStorageAccess', 'requestStorageAccess']) {
        const promise = proto[name].call(doc);
        check(`${realmName}/${name}/Promise-realm`, promise instanceof realm.Promise);
        try { await promise; } catch (error) { failures.push(`${realmName}/${name}/genuine: ${error.name}`); }
      }
    }
  }
  return failures;
}
