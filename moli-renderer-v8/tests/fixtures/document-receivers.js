async function documentReceiverProbe() {
  const checks = [];
  const check = (label, actual, wanted) => checks.push({
    label, actual, wanted, pass: JSON.stringify(actual) === JSON.stringify(wanted)
  });
  let frame = document.getElementById('document-receiver-child');
  if (!frame) {
    frame = document.createElement('iframe');
    frame.id = 'document-receiver-child';
    (document.body || document.documentElement).appendChild(frame);
  }
  try {
    const child = frame.contentWindow;
    const names = [
      'URL', 'documentURI', 'readyState', 'contentType', 'characterSet', 'charset',
      'inputEncoding', 'compatMode', 'lastModified', 'referrer', 'defaultView',
      'activeElement', 'implementation', 'fonts', 'currentScript', 'hidden',
      'visibilityState', 'prerendering', 'wasDiscarded', 'domain', 'scrollingElement'
    ];
    let traps = 0;
    const revoked = Proxy.revocable(document, {});
    revoked.revoke();
    const element = document.createElement('div');
    const invalid = [
      undefined, null, 1, 'document', false, Symbol('document'), 1n, {},
      Document.prototype, HTMLDocument.prototype, Object.create(document),
      Object.create(child.Document.prototype), element,
      document.createTextNode('text'), document.createDocumentFragment(),
      new Proxy(document, {
        get() { traps++; throw new Error('get trap'); },
        getPrototypeOf() { traps++; throw new Error('prototype trap'); }
      }),
      revoked.proxy, new Proxy(child.document, {})
    ];
    const changed = document.implementation.createHTMLDocument('changed prototype');
    const genuine = [
      document, child.document, new Document(), new child.Document(),
      document.implementation.createHTMLDocument('windowless'),
      child.document.implementation.createDocument(null, 'root'),
      new DOMParser().parseFromString('<root/>', 'application/xml'),
      new (class extends Document {})(), changed
    ];
    Object.setPrototypeOf(changed, null);
    for (const [realmName, realm] of [['parent', window], ['child', child]]) {
      const typeError = callback => {
        try { callback(); return 'returned'; }
        catch (error) {
          return Object.getPrototypeOf(error) === realm.TypeError.prototype
            ? 'callee TypeError' : error.name;
        }
      };
      for (const name of names) {
        const descriptor = Object.getOwnPropertyDescriptor(realm.Document.prototype, name);
        check(`${realmName}/${name}/invalid`, invalid.map(value =>
          typeError(() => descriptor.get.call(value))), invalid.map(() => 'callee TypeError'));
        check(`${realmName}/${name}/genuine`, genuine.map(value => {
          try { return descriptor.get.call(value) !== undefined; }
          catch (error) { return error.name; }
        }), genuine.map(() => true));
      }
      const domain = Object.getOwnPropertyDescriptor(realm.Document.prototype, 'domain');
      let conversions = 0;
      const sentinel = {};
      const value = {toString() { conversions++; throw sentinel; }};
      check(`${realmName}/domain/invalid-set`, invalid.map(receiver =>
        typeError(() => domain.set.call(receiver, value))), invalid.map(() => 'callee TypeError'));
      check(`${realmName}/domain/no-conversion`, conversions, 0);
      for (const [index, receiver] of genuine.entries()) {
        let caught;
        try { domain.set.call(receiver, value); } catch (error) { caught = error; }
        check(`${realmName}/domain/genuine-conversion-${index}`, caught === sentinel, true);
      }
      check(`${realmName}/domain/conversion-count`, conversions, genuine.length);
      const ready = Object.getOwnPropertyDescriptor(realm.Document.prototype, 'onreadystatechange');
      for (const [name, descriptor] of [
        ['onreadystatechange', ready],
        ['onmouseenter', Object.getOwnPropertyDescriptor(realm.HTMLElement.prototype, 'onmouseenter')],
        ['onmouseleave', Object.getOwnPropertyDescriptor(realm.HTMLElement.prototype, 'onmouseleave')]
      ]) {
        const incompatible = name === 'onreadystatechange'
          ? invalid : invalid.filter(receiver => receiver !== element);
        check(`${realmName}/${name}/lenient`, incompatible.map(receiver => {
          try {
            return descriptor.get.call(receiver) === undefined &&
              descriptor.set.call(receiver, value) === undefined;
          } catch (error) { return error.name; }
        }), incompatible.map(() => true));
      }
    }
    check('author-proxy-traps', traps, 0);
    const parentView = Object.getOwnPropertyDescriptor(Document.prototype, 'defaultView').get;
    const childView = Object.getOwnPropertyDescriptor(child.Document.prototype, 'defaultView').get;
    check('borrowed-defaultView', [parentView.call(child.document) === child,
      childView.call(document) === window, parentView.call(changed) === null], [true, true, true]);
    const hidden = Object.getOwnPropertyDescriptor(child.Document.prototype, 'hidden').get;
    const visibility = Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState').get;
    check('windowless-visibility', [hidden.call(changed), visibility.call(changed)], [true, 'hidden']);
  } finally { frame.remove(); }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
