(async () => {
  const checks = [];
  const check = (name, actual, expected) => checks.push({name, actual, expected, passed: JSON.stringify(actual) === JSON.stringify(expected)});
  const capture = (action, sentinel, callee) => {
    try { action(); return null; } catch (error) {
      if (error === sentinel) return 'sentinel';
      if (error.name === 'TypeError' && Object.getPrototypeOf(error) !== callee.TypeError.prototype) return 'wrong-realm-TypeError';
      return error.name;
    }
  };
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  let popup;
  try {
    const realms = [['main', window], ['iframe', frame.contentWindow]];
    const includePopup = globalThis.__domImplementationIncludePopup !== false;
    if (includePopup) {
      popup = window.open('', '_blank');
      if (!popup) throw new Error('popup unavailable');
      await new Promise(resolve => setTimeout(resolve, 0));
      realms.push(['popup', popup]);
    }
    for (const [ownerName, owner] of realms) {
      const documents = [
        ['live', owner.document],
        ['html', owner.document.implementation.createHTMLDocument('')],
        ['xml', owner.document.implementation.createDocument(null, 'root')],
        ['parsed-html', new owner.DOMParser().parseFromString('<!doctype html><body>', 'text/html')],
        ['parsed-xml', new owner.DOMParser().parseFromString('<root/>', 'application/xml')],
        ['new-document', new owner.Document()],
      ];
      for (const [docName, doc] of documents) {
        const implementation = doc.implementation;
        for (const [calleeName, callee] of realms) {
          const prefix = `${ownerName}/${docName}/${calleeName}`;
          const methods = callee.DOMImplementation.prototype;
          check(prefix + '/genuine-hasFeature', methods.hasFeature.call(implementation), true);
          const html = methods.createHTMLDocument.call(implementation, 'native title');
          check(prefix + '/genuine-html', [html.title, html instanceof owner.HTMLDocument, html.body.ownerDocument === html], ['native title', true, true]);
          const doctype = methods.createDocumentType.call(implementation, 'root', 'public', 'system');
          check(prefix + '/genuine-doctype', [doctype.name, doctype.publicId, doctype.systemId, doctype.ownerDocument === doc, doctype instanceof owner.DocumentType], ['root', 'public', 'system', true, true]);
          const xml = methods.createDocument.call(implementation, 'urn:test', 'root', doctype);
          check(prefix + '/genuine-xml', [xml instanceof owner.XMLDocument, xml.doctype === doctype, doctype.ownerDocument === xml, xml.documentElement.namespaceURI], [true, true, true, 'urn:test']);

          for (const [receiverName, makeReceiver] of [
            ['plain', () => ({})],
            ['prototype', () => Object.create(owner.DOMImplementation.prototype)],
            ['inherited', () => Object.create(implementation)],
            ['proxy', () => new Proxy(implementation, {})],
            ['revoked', () => { const revocable = Proxy.revocable(implementation, {}); revocable.revoke(); return revocable.proxy; }],
            ['document', () => doc],
            ['element', () => doc.createElement('select')],
            ['null', () => null],
            ['undefined', () => undefined],
            ['number', () => 1],
            ['symbol', () => Symbol()],
          ]) {
            for (const method of ['hasFeature', 'createHTMLDocument', 'createDocumentType', 'createDocument']) {
              const log = [];
              const argument = label => ({toString() {log.push(label); return 'root';}});
              const args = method === 'hasFeature' ? [argument('feature'), argument('version')]
                : method === 'createHTMLDocument' ? [argument('title')]
                : method === 'createDocumentType' ? [argument('name'), argument('public'), argument('system')]
                : [argument('namespace'), argument('name'), {}];
              check(`${prefix}/receiver/${receiverName}/${method}`, [capture(() => methods[method].apply(makeReceiver(), args), undefined, callee), log], ['TypeError', []]);
              check(`${prefix}/receiver-arity/${receiverName}/${method}`, capture(() => methods[method].call(makeReceiver()), undefined, callee), 'TypeError');
            }
          }
          const traps = [];
          const receiver = new Proxy(implementation, {
            get() {traps.push('get'); throw new Error('receiver get trap');},
            getPrototypeOf() {traps.push('getPrototypeOf'); throw new Error('receiver prototype trap');},
          });
          for (const method of ['hasFeature', 'createHTMLDocument', 'createDocumentType', 'createDocument']) {
            check(`${prefix}/receiver-traps/${method}`, [capture(() => methods[method].call(receiver), undefined, callee), traps.slice()], ['TypeError', []]);
          }

          for (const [valueName, makeValue, accepted] of [
            ['omitted', () => undefined, true],
            ['null', () => null, true],
            ['genuine', () => implementation.createDocumentType('root', '', ''), true],
            ['plain', () => ({}), false],
            ['prototype', () => Object.create(owner.DocumentType.prototype), false],
            ['inherited', () => Object.create(implementation.createDocumentType('root', '', '')), false],
            ['proxy', () => new Proxy(implementation.createDocumentType('root', '', ''), {}), false],
            ['revoked', () => {const revocable = Proxy.revocable(implementation.createDocumentType('root', '', ''), {}); revocable.revoke(); return revocable.proxy;}, false],
            ['number', () => 1, false],
            ['string', () => 'root', false],
            ['symbol', () => Symbol(), false],
            ['document', () => doc, false],
          ]) {
            for (const nameValid of [false, true]) for (const stop of [-1, 0, 1]) {
              const log = [], sentinel = {};
              const argument = (text, label, index) => ({toString() {log.push(label); if (stop === index) throw sentinel; return text;}});
              const namespace = argument('urn:test', 'namespace', 0);
              const name = argument(nameValid ? 'root' : 'bad name', 'name', 1);
              const value = makeValue();
              const expectedError = stop >= 0 ? 'sentinel' : !accepted ? 'TypeError' : nameValid ? null : 'InvalidCharacterError';
              const expectedLog = stop === 0 ? ['namespace'] : ['namespace', 'name'];
              check(`${prefix}/doctype/${valueName}/${nameValid}/${stop}`, [capture(() => methods.createDocument.call(implementation, namespace, name, value), sentinel, callee), log], [expectedError, expectedLog]);
            }
          }
          for (const value of [null, undefined]) {
            const empty = methods.createDocument.call(implementation, null, value, null);
            check(`${prefix}/legacy-null-name/${String(value)}`, empty.documentElement?.localName ?? null, value === null ? null : 'undefined');
          }
          const nameLog = [], sentinel = {};
          check(prefix + '/throwing-name-before-invalid-doctype', capture(() => methods.createDocument.call(implementation, null, {toString() {nameLog.push('name'); throw sentinel;}}, 1), sentinel, callee), 'sentinel');
          check(prefix + '/throwing-name-log', nameLog, ['name']);
          for (const value of [{}, 1, Symbol()]) {
            check(`${prefix}/symbol-name/${typeof value}`, capture(() => methods.createDocument.call(implementation, null, Symbol(), value), undefined, callee), 'TypeError');
            check(`${prefix}/namespace-before-doctype/${typeof value}`, capture(() => methods.createDocument.call(implementation, null, 'p:root', value), undefined, callee), 'TypeError');
          }
          const argumentTraps = [];
          const invalidDoctype = new Proxy(doctype, {
            get() {argumentTraps.push('get'); throw new Error('doctype get trap');},
            getPrototypeOf() {argumentTraps.push('getPrototypeOf'); throw new Error('doctype prototype trap');},
          });
          check(prefix + '/doctype-traps', [capture(() => methods.createDocument.call(implementation, null, 'bad name', invalidDoctype), undefined, callee), argumentTraps], ['TypeError', []]);
          for (const [method, args, expectedLog] of [
            ['createDocument', [{toString() {throw new Error('arity conversion');}}], []],
            ['createDocumentType', ['root', {toString() {throw new Error('arity conversion');}}], []],
          ]) {
            const log = [];
            check(`${prefix}/genuine-arity/${method}`, [capture(() => methods[method].apply(implementation, args), undefined, callee), log], ['TypeError', expectedLog]);
          }
        }
      }
    }
    globalThis.__uiEventResults = {complete: true, includePopup, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
    return checks.every(row => row.passed);
  } finally {
    if (popup) popup.close();
    frame.remove();
  }
})()
