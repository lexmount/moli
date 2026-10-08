(expectedSecureContext => {
  const name = 'onpointerrawupdate';
  const failures = [];
  const check = (condition, message) => {
    if (!condition) failures.push(message);
  };

  for (const w of [window, document.getElementById('child').contentWindow]) {
    check(w.isSecureContext === expectedSecureContext, 'native secure context');
    const secureDescriptor = Object.getOwnPropertyDescriptor(w, 'isSecureContext');
    let secureGetterCalls = 0;
    Object.defineProperty(w, 'isSecureContext', {
      configurable: true,
      get() {
        secureGetterCalls++;
        throw new Error('author-visible isSecureContext must not control exposure');
      }
    });

    try {
      const targets = [
        [w, w],
        [w.document, w.Document.prototype],
        [w.document.createElement('div'), w.HTMLElement.prototype],
        [w.document.createElementNS('http://www.w3.org/2000/svg', 'svg'), w.SVGElement.prototype],
        [w.document.createElementNS('http://www.w3.org/1998/Math/MathML', 'math'), w.MathMLElement.prototype],
        [w.document.implementation.createHTMLDocument(''), w.Document.prototype]
      ];

      for (const [target, owner] of targets) {
        const label = Object.prototype.toString.call(target);
        const descriptor = Object.getOwnPropertyDescriptor(owner, name);
        check((name in target) === expectedSecureContext, label + ': exposure');

        if (!expectedSecureContext) {
          check(descriptor === undefined, label + ': insecure prototype exposure');
          check(target[name] === undefined, label + ': insecure value');
          let calls = 0;
          target[name] = () => { calls++; };
          const expando = Object.getOwnPropertyDescriptor(target, name);
          check(expando && typeof expando.value === 'function' && !expando.get,
                label + ': ordinary author property');
          let listenerCalls = 0;
          const listener = () => { listenerCalls++; };
          target.addEventListener('pointerrawupdate', listener);
          target.dispatchEvent(new w.Event('pointerrawupdate'));
          check(calls === 0, label + ': author property must not register a handler');
          check(listenerCalls === 1, label + ': explicit event listener');
          let propertyReads = 0;
          Object.defineProperty(target, name, {
            configurable: true,
            get() { propertyReads++; return () => { calls++; }; }
          });
          target.dispatchEvent(new w.Event('pointerrawupdate'));
          check(propertyReads === 0 && calls === 0, label + ': author getter must stay inert');
          check(listenerCalls === 2, label + ': explicit event listener with author getter');
          target.removeEventListener('pointerrawupdate', listener);
          delete target[name];
          continue;
        }

        check(descriptor && descriptor.enumerable && descriptor.configurable &&
              typeof descriptor.get === 'function' && typeof descriptor.set === 'function',
              label + ': secure accessor');
        check(target[name] === null, label + ': initial value');
        let calls = 0;
        const handler = function(event) {
          check(this === target && event.type === 'pointerrawupdate', label + ': callback receiver');
          calls++;
          return false;
        };
        target[name] = handler;
        check(target[name] === handler, label + ': callback identity');
        check(!target.dispatchEvent(new w.Event('pointerrawupdate', {cancelable: true})) && calls === 1,
              label + ': registered handler');
        target[name] = null;
        check(target[name] === null, label + ': cleared value');
        check(target.dispatchEvent(new w.Event('pointerrawupdate', {cancelable: true})) && calls === 1,
              label + ': cleared handler');
      }

      check(secureGetterCalls === 0, 'exposure must use trusted realm state');
    } finally {
      Object.defineProperty(w, 'isSecureContext', secureDescriptor);
    }
  }

  if (failures.length) throw new Error(failures.join('\n'));
  return true;
})
