(() => {
  const checks = [];
  const check = (name, passed, details = '') => checks.push({name, passed: !!passed, details});
  const popup = open();
  const owners = [
    ['root', globalThis],
    ['child', document.querySelector('iframe').contentWindow],
    ['popup', popup],
  ];
  const value = String.fromCharCode(0xd800, 0x7c, 0xdc00, 0x7c, 0, 0x7c, 0xd83d, 0xde00);
  try {
    for (const [label, owner] of owners) {
      const doc = owner.document;
      for (const namespace of ['html', 'svg']) {
        const node = namespace === 'html' ? doc.createElement('button')
          : doc.createElementNS('http://www.w3.org/2000/svg', 'rect');
        const source = `return "${value}";`;
        node.setAttribute('onclick', source);
        Object.defineProperty(node, 'getAttribute', {value() { throw Error('author getAttribute'); }});
        const handler = node.onclick;
        const prefix = `${label}:${namespace}:`;
        check(prefix + 'compiled', typeof handler === 'function');
        check(prefix + 'value-units', typeof handler === 'function' && handler.call(node) === value);
        check(prefix + 'source-units', typeof handler === 'function' && owner.Function.prototype.toString.call(handler).includes(source));
        check(prefix + 'callee-realm', typeof handler === 'function' && handler instanceof owner.Function);
        check(prefix + 'cached-identity', node.onclick === handler);
        node.setAttribute('onclick', 'return "replacement";');
        const replacement = node.onclick;
        check(prefix + 'replacement', typeof replacement === 'function' && replacement !== handler && replacement.call(node) === 'replacement');
        node.removeAttribute('onclick');
        check(prefix + 'removal', node.onclick === null);
      }
      for (const mode of ['body-adopt', 'body-adopt-mutate', 'body-windowless-mutate', 'frameset-adopt-mutate']) {
        const previous = owner.onresize;
        const destinationFrame = doc.createElement('iframe');
        doc.body.appendChild(destinationFrame);
        const destinationWindow = destinationFrame.contentWindow;
        const destination = mode.includes('windowless')
          ? document.implementation.createHTMLDocument('destination') : destinationWindow.document;
        const sourceElement = doc.createElement(mode.startsWith('frameset') ? 'frameset' : 'body');
        const marker = `${label}:${mode}`;
        sourceElement.setAttribute('onresize', `globalThis.__rawHandlerMarker = ${JSON.stringify(marker)}; return "${value}";`);
        destination.adoptNode(sourceElement);
        destination.body.appendChild(sourceElement);
        if (mode.includes('mutate')) sourceElement.setAttribute('onresize', 'globalThis.__rawHandlerMarker = "foreign"; return "foreign";');
        const handler = owner.onresize;
        const prefix = `${label}:${mode}:`;
        check(prefix + 'original-window-compiled', typeof handler === 'function');
        check(prefix + 'original-source', typeof handler === 'function' && handler.call(owner) === value);
        check(prefix + 'original-global', owner.__rawHandlerMarker === marker, String(owner.__rawHandlerMarker));
        check(prefix + 'original-realm', typeof handler === 'function' && handler instanceof owner.Function);
        check(prefix + 'cached-identity', owner.onresize === handler);
        if (mode.includes('mutate') && !mode.includes('windowless')) {
          const foreign = destinationWindow.onresize;
          check(prefix + 'destination-source', typeof foreign === 'function' && foreign.call(destinationWindow) === 'foreign');
          check(prefix + 'destination-global', destinationWindow.__rawHandlerMarker === 'foreign');
        }
        sourceElement.removeAttribute('onresize');
        check(prefix + 'foreign-removal-preserves-original', owner.onresize === handler);
        const idl = () => marker;
        owner.onresize = idl;
        sourceElement.setAttribute('onresize', 'return "later";');
        check(prefix + 'foreign-change-preserves-original-IDL', owner.onresize === idl);
        owner.onresize = previous;
        destinationFrame.remove();
      }
      const previousError = owner.onerror;
      const body = doc.createElement('body');
      const foreign = document.implementation.createHTMLDocument('foreign error');
      body.setAttribute('onerror', `globalThis.__rawErrorArguments = [event, source, lineno, colno, error]; return true;`);
      foreign.adoptNode(body);
      body.setAttribute('onerror', 'return false;');
      const errorHandler = owner.onerror;
      check(`${label}:onerror:compiled`, typeof errorHandler === 'function');
      const error = new owner.Error('identity');
      check(`${label}:onerror:return`, typeof errorHandler === 'function' && errorHandler.call(owner, 'message', 'source', 3, 4, error) === true);
      const args = owner.__rawErrorArguments;
      check(`${label}:onerror:arguments`, Array.isArray(args) && args.length === 5 && args[0] === 'message' && args[1] === 'source' && args[2] === 3 && args[3] === 4 && args[4] === error);
      owner.onerror = previousError;
    }
  } catch (error) {
    check('fixture-completed-without-exception', false, String(error.stack || error));
  } finally {
    popup?.close();
  }
  globalThis.__uiEventResults = {complete: true, checks, total: checks.length, passed: checks.filter(row => row.passed).length};
  return checks.every(row => row.passed);
})()
