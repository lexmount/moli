(() => {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const popup = open();
  try {
    const factories = {
      html: () => document.implementation.createHTMLDocument(''),
      xml: () => document.implementation.createDocument(null, 'root'),
      parsedHtml: () => new DOMParser().parseFromString('<body>', 'text/html'),
      parsedXml: () => new DOMParser().parseFromString('<root/>', 'text/xml'),
    };
    for (const [kind, create] of Object.entries(factories)) {
      for (const tag of ['body', 'frameset']) {
        const foreign = create();
        const previous = popup.onerror;
        const node = popup.document.createElement(tag);
        const prefix = `${kind}:${tag}:`;
        node.setAttribute('onerror', 'return true;');
        check(prefix + 'adopted-identity', foreign.adoptNode(node) === node);
        check(prefix + 'native-owner', node.ownerDocument === foreign);
        check(prefix + 'disconnected', node.parentNode === null && !node.isConnected);
        check(prefix + 'windowless-document', foreign.defaultView === null);
        check(prefix + 'original-interface', node instanceof popup.HTMLElement);
        node.setAttribute('onerror', 'return false;');
        check(prefix + 'windowless-handler', node.onerror === null);
        const retained = popup.onerror;
        check(prefix + 'retained-window-source', typeof retained === 'function' && retained() === true);
        check(prefix + 'retained-window-identity', popup.onerror === retained);
        check(prefix + 'return-adopted-identity', popup.document.adoptNode(node) === node);
        check(prefix + 'return-owner', node.ownerDocument === popup.document);
        check(prefix + 'attribute-preserved', node.getAttribute('onerror') === 'return false;');
        node.setAttribute('onerror', 'return 42;');
        check(prefix + 'returned-node-mutation', typeof popup.onerror === 'function' && popup.onerror() === 42);
        popup.onerror = previous;
      }
    }
  } catch (error) {
    checks.push({name: 'fixture-completed-without-exception', passed: false, details: String(error.stack || error)});
  } finally {
    popup?.close();
  }
  globalThis.__uiEventResults = {complete: true, checks, total: checks.length, passed: checks.filter(row => row.passed).length};
  return checks.every(row => row.passed);
})()
