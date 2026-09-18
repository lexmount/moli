(() => {
  const failures = [];
  let checks = 0;
  const methodNames = ['open', 'write', 'writeln', 'close'];
  const mainBody = document.body;
  const mainRoot = document.documentElement;
  const makeFrame = () => mainBody.appendChild(document.createElement('iframe'));
  const methodsOf = object => Object.fromEntries(methodNames.map(name => [name, object[name]]));
  const equal = (label, actual, expected) => {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  };
  const donorFrame = makeFrame();
  const donorWindow = donorFrame.contentWindow;
  const donorDocument = donorFrame.contentDocument;
  const donorRoot = donorDocument.documentElement;
  const parentMethods = methodsOf(Document.prototype);
  const childPrototypeMethods = methodsOf(donorWindow.Document.prototype);
  const capturedChildMethods = methodsOf(donorDocument);
  const ChildTypeError = donorWindow.TypeError;

  for (const method of methodNames) {
    equal(method + ' is inherited', Object.hasOwn(donorDocument, method), false);
    equal(method + ' uses its realm prototype', donorDocument[method] === childPrototypeMethods[method], true);
  }

  for (const [realm, methods, ExpectedTypeError] of [
    ['parent', parentMethods, TypeError],
    ['child', capturedChildMethods, ChildTypeError],
  ]) {
    for (const method of methodNames) {
      let traps = 0;
      const revoked = Proxy.revocable(donorDocument, {});
      revoked.revoke();
      const receivers = [
        ['plain', {}],
        ['forged prototype', Object.create(donorWindow.Document.prototype)],
        ['inherited real document', Object.create(donorDocument)],
        ['author proxy', new Proxy(donorDocument, {get() { traps++; return undefined; }})],
        ['revoked proxy', revoked.proxy],
      ];
      for (const [kind, receiver] of receivers) {
        let conversions = 0;
        const argument = {toString() { conversions++; return ''; }};
        let error;
        try { methods[method].call(receiver, argument, argument, argument); }
        catch (caught) { error = caught; }
        const label = `${realm} ${method} ${kind}`;
        equal(label + ' rejects in callee realm', error instanceof ExpectedTypeError, true);
        equal(label + ' checks receiver before conversion', conversions, 0);
      }
      equal(`${realm} ${method} does not invoke proxy traps`, traps, 0);
    }
  }

  for (const [methodSource, methods] of [
    ['parent prototype', parentMethods],
    ['child prototype', childPrototypeMethods],
    ['captured child methods', capturedChildMethods],
  ]) {
    for (const removed of [false, true]) {
      const label = methodSource + (removed ? ' removed document' : ' live document');
      const frame = makeFrame();
      const doc = frame.contentDocument;
      const oldBody = doc.body;
      const detached = doc.createElement('div');
      const shadowHost = oldBody.appendChild(doc.createElement('div'));
      const shadowChild = shadowHost.attachShadow({mode: 'open'}).appendChild(doc.createElement('span'));
      const template = oldBody.appendChild(doc.createElement('template'));
      const templateChild = template.content.appendChild(doc.createElement('span'));
      let oldListenerCalls = 0;
      let detachedListenerCalls = 0;
      let templateListenerCalls = 0;
      if (removed) frame.remove();
      doc.addEventListener('stream-probe', () => oldListenerCalls++);
      oldBody.addEventListener('stream-probe', () => oldListenerCalls++);
      shadowChild.addEventListener('stream-probe', () => oldListenerCalls++);
      detached.addEventListener('stream-probe', () => detachedListenerCalls++);
      templateChild.addEventListener('stream-probe', () => templateListenerCalls++);
      try {
        equal(label + ' open returns receiver', methods.open.call(doc) === doc, true);
        equal(label + ' open clears the document', doc.childNodes.length, 0);
        equal(label + ' open starts loading', doc.readyState, 'loading');
        doc.dispatchEvent(new Event('stream-probe'));
        oldBody.dispatchEvent(new Event('stream-probe'));
        shadowChild.dispatchEvent(new Event('stream-probe'));
        detached.dispatchEvent(new Event('stream-probe'));
        templateChild.dispatchEvent(new Event('stream-probe'));
        equal(label + ' clears current document tree listeners', oldListenerCalls, 0);
        if (!removed) equal(label + ' keeps detached node listeners', detachedListenerCalls, 1);
        if (!removed) equal(label + ' keeps template content listeners', templateListenerCalls, 1);
        methods.write.call(doc, '<!doctype html><head><title>written</title></head><body><section id="stream"><b id="keep">kept</b>');
        const keep = doc.getElementById('keep');
        methods.write.call(doc, '<i id="ta');
        methods.writeln.call(doc, 'il">tail</i></section>');
        equal(label + ' preserves split tokens', doc.getElementById('tail')?.textContent, 'tail');
        equal(label + ' preserves node identity', doc.getElementById('keep') === keep && keep !== null, true);
        equal(label + ' uses document parser', doc.title, 'written');
        methods.close.call(doc);
        methods.write.call(doc, '<p id="replacement">replacement</p>');
        equal(label + ' implicit open replaces old tree', doc.getElementById('keep'), null);
        equal(label + ' implicit write targets receiver', doc.body?.textContent, 'replacement');
        methods.close.call(doc);
        equal(label + ' preserves main document', document.body === mainBody, true);
        equal(label + ' preserves method donor document', donorDocument.documentElement === donorRoot, true);
      } catch (error) {
        equal(label + ' unexpected exception', error.name + ': ' + error.message, null);
      } finally {
        frame.remove();
        // Restore only the probe page after reproducing an implementation that
        // mistakenly replaces it through a borrowed child Document method.
        if (document.documentElement !== mainRoot) document.replaceChildren(mainRoot);
      }
    }
  }

  donorFrame.remove();
  const targetFrame = makeFrame();
  const targetDocument = targetFrame.contentDocument;
  try {
    capturedChildMethods.open.call(targetDocument);
    capturedChildMethods.write.call(targetDocument, '<p id="retired-donor">retired method</p>');
    capturedChildMethods.close.call(targetDocument);
    equal('method from removed frame uses actual receiver', targetDocument.getElementById('retired-donor')?.textContent, 'retired method');
    equal('method from removed frame leaves donor alone', donorDocument.documentElement === donorRoot, true);
    equal('method from removed frame leaves main alone', document.body === mainBody, true);
  } catch (error) {
    equal('retired method unexpected exception', error.name + ': ' + error.message, null);
  } finally {
    targetFrame.remove();
    if (document.documentElement !== mainRoot) document.replaceChildren(mainRoot);
  }
  return {checks, failures};
})()
