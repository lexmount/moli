(() => {
  const checks = [];
  const check = (name, passed, details = '') => checks.push({name, passed: !!passed, details});
  const popup = open();
  const owners = [
    ['root', globalThis],
    ['child', document.querySelector('iframe').contentWindow],
    ['popup', popup],
  ];
  const prepare = (node, name, source, resource) => {
    eval(`\n\nnode.setAttribute(name, source);\n//# sourceURL=${resource}`);
  };
  try {
    for (const [label, owner] of owners) {
      const errors = [];
      const onerror = event => {
        errors.push({filename: event.filename, line: event.lineno, column: event.colno,
          nativeError: event.error instanceof owner.SyntaxError, trusted: event.isTrusted});
        event.preventDefault();
      };
      owner.addEventListener('error', onerror);
      const previousPrepare = owner.Error.prepareStackTrace;
      let hookCalls = 0;
      owner.Error.prepareStackTrace = () => { hookCalls++; throw new Error('author stack hook'); };
      try {
        Object.defineProperty(owner.document, 'URL', {configurable: true, get() { throw new Error('author Document.URL'); }});
        for (const mode of ['ordinary', 'same-value', 'attr-value', 'clone', 'import', 'foreign-import', 'fragment']) {
          const sourceDocument = mode === 'foreign-import'
            ? (owner === globalThis ? popup.document : document) : owner.document;
          const node = sourceDocument.createElement('button');
          const original = `https://handler-location.test/${label}/${mode}/original.js?query#fragment`;
          const replacement = `https://handler-location.test/${label}/${mode}/replacement.js?query#fragment`;
          prepare(node, 'onclick', 'let = ;', original);
          let target = node;
          if (mode === 'same-value') {
            prepare(node, 'onclick', 'let = ;', replacement);
          } else if (mode === 'attr-value') {
            const attribute = node.getAttributeNode('onclick');
            eval(`\n\nattribute.value = 'let = ;';\n//# sourceURL=${replacement}`);
          } else if (mode === 'clone') {
            eval(`\n\ntarget = node.cloneNode(true);\n//# sourceURL=${replacement}`);
          } else if (mode === 'import' || mode === 'foreign-import') {
            eval(`\n\ntarget = owner.document.importNode(node, true);\n//# sourceURL=${replacement}`);
          } else if (mode === 'fragment') {
            eval(`\n\nnode.innerHTML = '<span onclick="let = ;"></span>';\n//# sourceURL=${replacement}`);
            target = node.firstChild;
          }
          Object.defineProperty(target, 'getAttribute', {value() { throw new Error('author getAttribute'); }});
          const before = errors.length;
          const handler = target.onclick;
          const error = errors[before];
          const prefix = `${label}:${mode}:`;
          check(prefix + 'null-after-parse-error', handler === null);
          check(prefix + 'one-native-error', errors.length === before + 1, JSON.stringify(errors));
          check(prefix + 'original-resource', error?.filename === (mode === 'ordinary' ? original : replacement), JSON.stringify(error));
          check(prefix + 'original-line', error?.line === 3, JSON.stringify(error));
          check(prefix + 'source-column', error?.column > 6, JSON.stringify(error));
          check(prefix + 'target-realm', error?.nativeError === true);
          check(prefix + 'trusted-report', error?.trusted === true);
          check(prefix + 'cached-null', target.onclick === null && errors.length === before + 1);
        }
        for (const kind of ['element', 'window']) {
          const sourceFrame = owner.document.createElement('iframe');
          owner.document.body.appendChild(sourceFrame);
          const sourceWindow = sourceFrame.contentWindow;
          const node = owner.document.createElement(kind === 'window' ? 'body' : 'button');
          const name = kind === 'window' ? 'onresize' : 'onclick';
          const original = `https://handler-location.test/${label}/adopt-${kind}.js?query#fragment`;
          const previous = owner.onresize;
          prepare(node, name, 'let = ;', original);
          sourceWindow.document.adoptNode(node);
          const adoptedErrors = [];
          const adoptedError = event => {
            adoptedErrors.push({filename: event.filename, line: event.lineno,
              nativeError: event.error instanceof sourceWindow.SyntaxError});
            event.preventDefault();
          };
          sourceWindow.addEventListener('error', adoptedError);
          const before = errors.length;
          const handler = kind === 'window' ? owner.onresize : node.onclick;
          const error = kind === 'window' ? errors[before] : adoptedErrors[0];
          const prefix = `${label}:adopt-${kind}:`;
          check(prefix + 'null-after-error', handler === null);
          check(prefix + 'original-resource', error?.filename === original, JSON.stringify(error));
          check(prefix + 'original-line', error?.line === 3, JSON.stringify(error));
          check(prefix + 'target-error-realm', error?.nativeError === true);
          check(prefix + 'correct-error-window', kind === 'window'
            ? errors.length === before + 1 && adoptedErrors.length === 0
            : errors.length === before && adoptedErrors.length === 1);
          owner.onresize = previous;
          sourceWindow.removeEventListener('error', adoptedError);
          sourceFrame.remove();
        }
        check(`${label}:no-author-stack-hooks`, hookCalls === 0, String(hookCalls));
      } finally {
        delete owner.document.URL;
        owner.Error.prepareStackTrace = previousPrepare;
        owner.removeEventListener('error', onerror);
      }
    }
  } catch (error) {
    check('fixture-completed-without-exception', false, String(error));
  } finally {
    popup?.close();
  }
  globalThis.__uiEventResults = {complete: true, checks, total: checks.length,
    passed: checks.filter(row => row.passed).length};
  return checks.every(row => row.passed);
})()
