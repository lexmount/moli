(() => {
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const popup = open();
  try {
    const meta = popup.document.createElement('meta');
    meta.httpEquiv = 'Content-Security-Policy';
    meta.content = "script-src-attr 'none'";
    popup.document.head.appendChild(meta);
    const factories = {
      html: realm => realm.document.implementation.createHTMLDocument(''),
      xml: realm => realm.document.implementation.createDocument(null, 'root'),
      parsed: realm => new realm.DOMParser().parseFromString('<body>', 'text/html'),
      template: realm => realm.document.createElement('template').content.ownerDocument,
    };
    for (const [direction, source, target, allowed] of [
      ['allowed-to-blocked', window, popup, true],
      ['blocked-to-allowed', popup, window, false],
    ]) {
      for (const [kind, create] of Object.entries(factories)) {
        for (const operation of ['adoptNode', 'appendChild']) {
          const prefix = `${direction}:${kind}:${operation}:`;
          const doc = create(source);
          const parent = doc.createElementNS('http://www.w3.org/1999/xhtml', 'div');
          const prior = doc.createElementNS('http://www.w3.org/1999/xhtml', 'button');
          const fresh = doc.createElementNS('http://www.w3.org/1999/xhtml', 'button');
          parent.append(prior, fresh);
          prior.callOrder = [];
          prior.addEventListener('click', () => prior.callOrder.push('before'));
          const idl = () => prior.callOrder.push('prior');
          prior.onclick = idl;
          prior.addEventListener('click', () => prior.callOrder.push('after'));
          prior.setAttribute('onclick', "this.callOrder.push('content')");
          const code = "this.setAttribute('data-called', 'yes')";
          fresh.setAttribute('onclick', code);
          check(prefix + 'windowless', doc.defaultView === null);
          check(prefix + 'admission-preserves-idl', (prior.onclick === idl) === !allowed);
          const adopted = operation === 'adoptNode'
            ? target.document.adoptNode(parent)
            : target.document.body.appendChild(parent);
          check(prefix + 'identity', adopted === parent);
          if (operation === 'adoptNode') target.document.body.appendChild(parent);
          check(prefix + 'descendant-owners', [parent, prior, fresh].every(node => node.ownerDocument === target.document));
          check(prefix + 'descendant-wrappers', parent.firstChild === prior && parent.lastChild === fresh);
          check(prefix + 'original-prototype', parent instanceof source.HTMLElement);
          const admitted = prior.onclick;
          const freshHandler = fresh.onclick;
          check(prefix + 'retained-idl', (admitted === idl) === !allowed);
          prior.dispatchEvent(new target.Event('click'));
          fresh.dispatchEvent(new target.Event('click'));
          check(prefix + 'listener-order', prior.callOrder.join(',') === (allowed ? 'before,content,after' : 'before,prior,after'));
          check(prefix + 'fresh-source-execution', (fresh.getAttribute('data-called') === 'yes') === allowed);
          check(prefix + 'fresh-handler', allowed ? typeof freshHandler === 'function' : freshHandler === null);
          check(prefix + 'attribute-retained', fresh.getAttribute('onclick') === code);
          check(prefix + 'return-identity', source.document.adoptNode(parent) === parent);
          check(prefix + 'return-state', prior.ownerDocument === source.document && prior.onclick === admitted && fresh.onclick === freshHandler);
        }
      }
    }
  } catch (error) {
    checks.push({name:'fixture-completed-without-exception',passed:false,details:String(error.stack || error)});
  } finally {
    popup?.close();
  }
  globalThis.__uiEventResults = {complete:true,checks,total:checks.length,passed:checks.filter(row => row.passed).length};
  return checks.every(row => row.passed);
})()
