(() => {
  const checks = [];
  const record = (name, verify) => {
    try { checks.push({name, passed: verify() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = window.open('about:blank', '_blank');
  const owners = [window, frame.contentWindow, popup];
  const realmObservations = [];
  try {
    for (const [ownerIndex, owner] of owners.entries()) {
      const doc = owner.document;
      const sheet = doc.createElement('style');
      sheet.textContent = '.cssom-identity-target::before { content: "x"; color: rgb(4, 5, 6); }';
      doc.head.appendChild(sheet);
      const html = doc.createElement('div'), svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
      const detached = doc.createElement('div');
      for (const target of [html, svg, detached]) {
        target.setAttribute('class', 'cssom-identity-target');
        target.style.color = 'rgb(1, 2, 3)';
      }
      doc.body.append(html, svg);
      for (const [kind, target] of [['HTML', html], ['SVG', svg], ['detached', detached]]) {
        for (const [pseudoName, pseudo, expected] of [
          ['originating', undefined, 'rgb(1, 2, 3)'], ['nullable', null, 'rgb(1, 2, 3)'],
          ['empty string', '', 'rgb(1, 2, 3)'], ['before', '::before', 'rgb(4, 5, 6)'],
          ['single colon before', ':before', 'rgb(4, 5, 6)'], ['invalid', '::unknown-pseudo', ''],
          ['part', '::part(token)', ''],
        ]) {
          const label = `owner ${ownerIndex}/${kind}/${pseudoName}`;
          const first = owner.getComputedStyle(target, pseudo);
          const second = owner.getComputedStyle(target, pseudo);
          const originalPrototype = Object.getPrototypeOf(first);
          record(label + '/new object', () => first !== second);
          record(label + '/platform brand', () => second instanceof owner.CSSStyleDeclaration);
          record(label + '/value', () => second.color === (kind === 'detached' ? '' : expected));
          try {
            record(label + '/own expando', () => {
              first.__identityMarker = 'first';
              return Object.hasOwn(first, '__identityMarker');
            });
            record(label + '/own descriptor', () => {
              Object.defineProperty(first, '__identityDescriptor', {value: 17, configurable: true});
              return Object.getOwnPropertyDescriptor(first, '__identityDescriptor').value === 17;
            });
            record(label + '/expando isolated', () => !Object.hasOwn(second, '__identityMarker'));
            record(label + '/descriptor isolated', () => Object.getOwnPropertyDescriptor(second, '__identityDescriptor') === undefined);
            Object.setPrototypeOf(first, Object.create(originalPrototype));
            record(label + '/prototype isolated', () => Object.getPrototypeOf(second) === originalPrototype);
            record(label + '/third object', () => {
              const third = owner.getComputedStyle(target, pseudo);
              return third !== first && third !== second && Object.getPrototypeOf(third) === originalPrototype;
            });
          } finally {
            Object.setPrototypeOf(first, originalPrototype);
            delete first.__identityMarker;
            delete first.__identityDescriptor;
          }
        }
        const one = owner.getComputedStyle(target), two = owner.getComputedStyle(target);
        const inline = target.style;
        target.style.color = 'rgb(7, 8, 9)';
        record(`owner ${ownerIndex}/${kind}/held first stays live`, () => one.color === (kind === 'detached' ? '' : 'rgb(7, 8, 9)'));
        record(`owner ${ownerIndex}/${kind}/held second stays live`, () => two.color === (kind === 'detached' ? '' : 'rgb(7, 8, 9)'));
        record(`owner ${ownerIndex}/${kind}/inline SameObject`, () => target.style === inline);
        record(`owner ${ownerIndex}/${kind}/inline distinct`, () => inline !== one && inline !== two);
        if (typeof target.computedStyleMap === 'function') {
          const map = target.computedStyleMap();
          record(`owner ${ownerIndex}/${kind}/computed map SameObject`, () => target.computedStyleMap() === map);
        }
        target.style.color = 'rgb(1, 2, 3)';
      }
      html.remove(); svg.remove(); sheet.remove();
    }
    const root = document.createElement('div');
    root.style.color = 'rgb(1, 2, 3)';
    document.body.appendChild(root);
    const declarations = Array.from({length: 128}, () => window.getComputedStyle(root));
    record('128 calls yield 128 objects', () => new Set(declarations).size === 128);
    record('128 expando writes accepted', () => {
      for (const [index, declaration] of declarations.entries()) declaration.__identityToken = index;
      return true;
    });
    record('128 independent expandos', () => declarations.every((declaration, index) => declaration.__identityToken === index));
    root.style.color = 'rgb(10, 11, 12)';
    record('128 declarations share live native state', () => declarations.every(declaration => declaration.color === 'rgb(10, 11, 12)'));
    root.remove();
    record('128 declarations observe disconnection', () => declarations.every(declaration => declaration.length === 0));
    document.body.appendChild(root);
    record('128 declarations observe reconnection', () => declarations.every(declaration => declaration.color === 'rgb(10, 11, 12)'));
    root.remove();
    for (const [targetIndex, targetOwner] of owners.entries()) {
      const target = targetOwner.document.createElement('div');
      target.style.color = 'rgb(1, 2, 3)';
      targetOwner.document.body.appendChild(target);
      for (const [functionIndex, functionOwner] of owners.entries()) {
        for (const [receiverIndex, receiver] of owners.entries()) {
          try {
            const result = Reflect.apply(functionOwner.getComputedStyle, receiver, [target]);
            realmObservations.push({targetIndex, functionIndex, receiverIndex, null: result === null,
              color: result?.color, prototypeRealms: result === null ? [] : owners.map(owner =>
                Object.getPrototypeOf(result) === (owner.CSSStyleProperties ?? owner.CSSStyleDeclaration).prototype)});
          } catch (error) {
            realmObservations.push({targetIndex, functionIndex, receiverIndex, error: String(error)});
          }
        }
      }
      target.remove();
    }
    globalThis.__computedStyleIdentityResults = {complete: true, total: checks.length,
      passed: checks.filter(row => row.passed).length, checks, realmObservations};
    globalThis.__uiEventResults = globalThis.__computedStyleIdentityResults;
    return true;
  } finally {
    frame.remove();
    if (popup && !popup.closed) popup.close();
  }
})();
