(() => {
  const rows = [], frame = document.querySelector('#child'), other = frame.contentWindow;
  const popup = open('about:blank', '_blank');
  if (!popup) throw Error('controlled owner probe requires an opened popup');
  const owners = [['top', globalThis], ['frame', other], ['popup', popup]];
  const methods = ['appendChild', 'insertBefore', 'replaceChild', 'append', 'prepend', 'replaceChildren', 'before', 'after', 'replaceWith'];
  const check = (name, body) => {
    try {if (!body()) throw Error('false');rows.push({name, passed: true});}
    catch (error) {rows.push({name, passed: false, error: String(error)});}
  };
  for (const [, owner] of owners) owner.__ownerMutationTrace = Object.create(null);
  const traceIs = (owner, label, expected) => owners.every(([, realm]) => realm.__ownerMutationTrace[label] === (realm === owner ? expected : undefined));
  const makeScript = (owner, label) => {
    const script = owner.document.createElement('script');script.id = label;
    script.textContent = `globalThis.__ownerMutationTrace[${JSON.stringify(label)}] = (globalThis.__ownerMutationTrace[${JSON.stringify(label)}] || 0) + 1; globalThis.__ownerMutationCurrent = [document.currentScript.id, globalThis === window, document === window.document];`;
    return script;
  };
  const invoke = (owner, receiver, method, style, script) => {
    const callee = owner === globalThis ? other : globalThis;
    const prototype = ['appendChild', 'insertBefore', 'replaceChild'].includes(method) ? callee.Node.prototype : receiver.nodeType === 11 ? callee.DocumentFragment.prototype : callee.Element.prototype;
    const operation = style === 'local' ? receiver[method] : prototype[method];
    if (['insertBefore', 'replaceChild'].includes(method)) operation.call(receiver, script, receiver.firstChild);
    else operation.call(receiver, script);
  };
  for (const [kind, owner] of owners) for (const flavor of ['native', 'adopted', 'view', 'parsed']) {
    for (const method of methods) for (const style of ['local', 'borrowed']) {
      const label = `${kind}/${flavor}/${method}/${style}`;
      check(label, () => {
        const doc = owner.document;
        const inert = doc.implementation.createHTMLDocument('inert');
        const container = flavor === 'adopted' ? inert.createElement('section') : flavor === 'parsed' ?
          new owner.DOMParser().parseFromString('<section/>', 'text/html').body.firstChild : doc.createElement('section');
        if (flavor === 'view') {
          inert.body.appendChild(container);
          const child = doc.createComment('view');container.appendChild(child);
          if (child.parentNode !== container) return false;
        }
        doc.body.appendChild(container);
        if (method === 'replaceChild') container.appendChild(doc.createComment('old'));
        const receiver = ['before', 'after', 'replaceWith'].includes(method) ? container.appendChild(doc.createElement('i')) : container;
        const script = makeScript(owner, label);
        try {
          invoke(owner, receiver, method, style, script);
          if (!traceIs(owner, label, 1) || owner.__ownerMutationCurrent.join('|') !== `${label}|true|true` || owner.document.currentScript !== null) return false;
          script.remove();container.appendChild(script);
          return traceIs(owner, label, 1);
        } finally {container.remove();}
      });
    }
  }
  for (const [kind, owner] of owners) for (const flavor of ['document', 'fragment', 'template']) {
    for (const method of methods) for (const style of ['local', 'borrowed']) {
      const label = `${kind}/${flavor}/${method}/${style}/activation`;
      check(label, () => {
        const doc = owner.document, inert = doc.implementation.createHTMLDocument('inert');
        const container = flavor === 'document' ? inert.createElement('section') : flavor === 'template' ? doc.createElement('template').content : doc.createDocumentFragment();
        if (flavor === 'document') inert.body.appendChild(container);
        if (method === 'replaceChild') container.appendChild(doc.createComment('old'));
        const receiver = ['before', 'after', 'replaceWith'].includes(method) ? container.appendChild(doc.createElement('i')) : container;
        const script = makeScript(owner, label);
        invoke(owner, receiver, method, style, script);
        if (!traceIs(owner, label, undefined)) return false;
        const clone = script.cloneNode(true);clone.id = label + '/clone';
        clone.textContent = `globalThis.__ownerMutationTrace[${JSON.stringify(label+'/clone')}] = 1;`;
        try {
          doc.body.appendChild(script);doc.body.appendChild(clone);
          return traceIs(owner, label, flavor === 'document' ? undefined : 1) &&
            traceIs(owner, label+'/clone', flavor === 'document' ? undefined : 1);
        } finally {script.remove();clone.remove();}
      });
    }
  }
  popup.close();
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
