(async () => {
  const checks = [], resources = [];
  const check = (name, run) => {
    let passed = false, error;
    try { passed = run() === true; } catch (caught) { error = String(caught); }
    checks.push({ name, passed, ...(error === undefined ? {} : { error }) });
  };
  const capture = run => { try { run(); } catch (error) { return error; } };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  resources.push(frame);
  const popup = window.open('about:blank');
  if (popup) resources.push({ remove() { popup.close(); } });
  try {
    for (const [realmName, w] of [['main', window], ['iframe', frame.contentWindow], ['popup', popup]]) {
      if (!w) { check(`${realmName}/realm available`, () => false); continue; }
      const providers = [
        ['live', () => w.document],
        ['html', () => w.document.implementation.createHTMLDocument('')],
        ['xml', () => w.document.implementation.createDocument(null, 'root')],
        ['parser-html', () => new w.DOMParser().parseFromString('<p>fixture</p>', 'text/html')],
        ['parser-xml', () => new w.DOMParser().parseFromString('<root/>', 'application/xml')],
      ];
      for (const [provider, create] of providers) {
        const doc = create(), prefix = `${realmName}/${provider}`;
        const methods = { importNode: doc.importNode, adoptNode: doc.adoptNode };
        for (const name of ['importNode', 'adoptNode']) {
          check(`${prefix}/${name} reflection`, () => methods[name].name === name && methods[name].length === 1);
        }
        const real = doc.createElement('section');
        const revoked = w.Proxy.revocable(real, {}); revoked.revoke();
        const invalid = [
          ['undefined', undefined], ['null', null], ['number', 1], ['boolean', true],
          ['string', 'node'], ['symbol', w.Symbol('node')], ['bigint', 1n],
          ['object', {}], ['array', []], ['function', () => {}],
          ['Node prototype', Object.create(w.Node.prototype)],
          ['inherits real', Object.create(real)], ['author Proxy', new w.Proxy(real, {})],
          ['revoked Proxy', revoked.proxy], ['spoof element', { nodeType: 1, nodeName: 'DIV', childNodes: [] }],
          ['spoof document', { nodeType: 9, nodeName: '#document' }],
        ];
        for (const [caseName, node] of invalid) {
          for (const name of ['importNode', 'adoptNode']) {
            check(`${prefix}/${name} rejects ${caseName} before options`, () => {
              let reads = 0;
              const options = { get customElementRegistry() { reads++; return undefined; }, get selfOnly() { reads++; return false; } };
              const error = capture(() => methods[name].call(doc, node, options));
              return error instanceof w.TypeError && reads === 0;
            });
          }
        }
        for (const name of ['importNode', 'adoptNode']) {
          check(`${prefix}/${name} required Node`, () => capture(() => methods[name].call(doc)) instanceof w.TypeError);
          check(`${prefix}/${name} rejects receiver before options`, () => {
            let reads = 0;
            const options = { get selfOnly() { reads++; throw new Error('wrong order'); } };
            const error = capture(() => methods[name].call({}, real, options));
            return error instanceof w.TypeError && reads === 0;
          });
        }
        for (const name of ['importNode', 'adoptNode']) {
          check(`${prefix}/${name} rejects trapping author Proxy`, () => {
            let traps = 0, reads = 0;
            const node = new w.Proxy(real, { get() { traps++; throw new Error('node trap'); }, getPrototypeOf() { traps++; throw new Error('prototype trap'); } });
            const options = { get selfOnly() { reads++; return false; } };
            const error = capture(() => methods[name].call(doc, node, options));
            return error instanceof w.TypeError && traps === 0 && reads === 0;
          });
        }
        const parent = doc.createElement('div'); parent.appendChild(doc.createTextNode('\ud800 text'));
        const optionCases = [
          ['missing', [], false], ['undefined', [undefined], false], ['false', [false], false],
          ['true', [true], true], ['null dictionary', [null], true],
          ['zero', [0], false], ['NaN', [NaN], false], ['one', [1], true],
          ['empty string', [''], false], ['string', ['false'], true],
          ['zero bigint', [0n], false], ['bigint', [2n], true], ['symbol', [w.Symbol('deep')], true],
          ['empty dictionary', [{}], true], ['dictionary selfOnly true', [{ selfOnly: true }], false],
          ['dictionary selfOnly false', [{ selfOnly: false }], true], ['array dictionary', [[]], true],
          ['function dictionary', [() => {}], true], ['boxed false dictionary', [new w.Boolean(false)], true],
          ['truthy selfOnly', [{ selfOnly: { valueOf() { throw new Error('ToBoolean has no conversion hooks'); } } }], false],
          ['undefined registry', [{ customElementRegistry: undefined }], true],
        ];
        for (const [caseName, supplied, deep] of optionCases) {
          check(`${prefix}/importNode ${caseName}`, () => {
            const clone = methods.importNode.call(doc, parent, ...supplied);
            return clone !== parent && clone.ownerDocument === doc && clone.parentNode === null &&
              clone.childNodes.length === (deep ? 1 : 0) && (!deep || clone.firstChild.data === '\ud800 text') &&
              parent.childNodes.length === 1;
          });
        }
        check(`${prefix}/dictionary lexical member order`, () => {
          const reads = [];
          const options = new w.Proxy({}, { get(target, key) { reads.push(key); return undefined; } });
          const clone = methods.importNode.call(doc, parent, options);
          return clone.childNodes.length === 1 && reads.join(',') === 'customElementRegistry,selfOnly';
        });
        for (const member of ['customElementRegistry', 'selfOnly']) {
          check(`${prefix}/${member} getter exception identity and stop`, () => {
            const marker = {}, reads = [];
            const options = new w.Proxy({}, { get(target, key) { reads.push(key); if (key === member) throw marker; return undefined; } });
            const error = capture(() => methods.importNode.call(doc, parent, options));
            return error === marker && reads.join(',') === (member === 'customElementRegistry' ? member : 'customElementRegistry,selfOnly') && parent.childNodes.length === 1;
          });
        }
        const registryRevoked = w.Proxy.revocable(w.customElements, {}); registryRevoked.revoke();
        const badRegistries = [
          ['null', null], ['boolean', true], ['number', 1], ['string', 'registry'],
          ['symbol', w.Symbol('registry')], ['object', {}],
          ['forged', Object.create(w.CustomElementRegistry.prototype)],
          ['inherits real', Object.create(w.customElements)],
          ['author Proxy', new w.Proxy(w.customElements, {})], ['revoked Proxy', registryRevoked.proxy],
        ];
        for (const [caseName, registry] of badRegistries) {
          check(`${prefix}/registry rejects ${caseName} before selfOnly`, () => {
            let reads = 0;
            const error = capture(() => methods.importNode.call(doc, parent, { customElementRegistry: registry, get selfOnly() { reads++; return true; } }));
            return error instanceof w.TypeError && reads === 0;
          });
        }
        for (const [name, type, code] of [['importNode', 'NotSupportedError', 9], ['adoptNode', 'NotSupportedError', 9]]) {
          check(`${prefix}/${name} rejects Document`, () => {
            const error = capture(() => methods[name].call(doc, doc));
            return error instanceof w.DOMException && error.name === type && error.code === code;
          });
        }
        const shadowHost = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div'), root = shadowHost.attachShadow({ mode: 'closed' });
        for (const [name, type, code] of [['importNode', 'NotSupportedError', 9], ['adoptNode', 'HierarchyRequestError', 3]]) {
          check(`${prefix}/${name} rejects ShadowRoot`, () => {
            const error = capture(() => methods[name].call(doc, root));
            return error instanceof w.DOMException && error.name === type && error.code === code;
          });
        }
        for (const node of [doc, root]) {
          check(`${prefix}/conversion precedes prohibited ${node === doc ? 'Document' : 'ShadowRoot'} algorithm`, () => {
            const marker = {};
            return capture(() => methods.importNode.call(doc, node, { get customElementRegistry() { throw marker; } })) === marker;
          });
        }
        for (const [kind, make] of [
          ['element', () => doc.createElement('p')], ['text', () => doc.createTextNode('\udfff')],
          ['comment', () => doc.createComment('\ud800')], ['fragment', () => doc.createDocumentFragment()],
          ['pi', () => doc.createProcessingInstruction('fixture', 'text')],
        ]) {
          check(`${prefix}/importNode genuine ${kind}`, () => {
            const source = make(), clone = methods.importNode.call(doc, source);
            return clone !== source && clone.nodeType === source.nodeType && clone.ownerDocument === doc && clone.parentNode === null;
          });
          check(`${prefix}/adoptNode genuine ${kind}`, () => {
            const source = make(); parent.appendChild(source);
            return methods.adoptNode.call(doc, source) === source && source.ownerDocument === doc && source.parentNode === null;
          });
        }
      }
    }
    globalThis.__documentImportAdoptResults = { complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks };
    globalThis.__uiEventResults = globalThis.__documentImportAdoptResults;
    return true;
  } finally { for (const resource of resources.reverse()) resource.remove(); }
})()
