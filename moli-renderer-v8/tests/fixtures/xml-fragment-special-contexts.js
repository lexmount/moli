(() => {
  const H = 'http://www.w3.org/1999/xhtml', N = 'http://www.w3.org/2000/xmlns/';
  const rows = [], realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const check = (name, body) => {
    try {rows.push({name, passed: body() === true});}
    catch (error) {rows.push({name, passed: false, error: String(error)});}
  };
  const makeDocument = (owner, flavor) => flavor === 'impl' ? owner.document.implementation.createDocument('urn:root', 'Root') :
    new owner.DOMParser().parseFromString(flavor === 'xml' ? '<Root/>' : '<html xmlns="'+H+'"/>', flavor === 'xml' ? 'application/xml' : 'application/xhtml+xml');
  for (const [ownerName, owner] of realms) for (const flavor of ['impl', 'xml', 'xhtml']) for (const [calleeName, callee] of realms) {
    const doc = makeDocument(owner, flavor), label = `${calleeName}/${ownerName}/${flavor}`;
    const host = doc.createElementNS(H, 'div'); host.setAttributeNS(N, 'xmlns:p', 'urn:local'); doc.documentElement.appendChild(host);
    const shadow = host.attachShadow({mode: 'open'});
    const setter = Object.getOwnPropertyDescriptor(callee.ShadowRoot.prototype, 'innerHTML').set;
    check(label+'/ShadowRoot uses native host namespaces', () => {
      setter.call(shadow, '<MixedCase p:Token="v"/><p:Item/>');
      return shadow.childNodes.length === 2 && shadow.firstChild.localName === 'MixedCase' && shadow.firstChild.namespaceURI === H &&
        shadow.firstChild.getAttributeNS('urn:local', 'Token') === 'v' && shadow.lastChild.namespaceURI === 'urn:local' && shadow.firstChild.ownerDocument === doc;
    });
    for (const [name, markup] of [['unclosed', '<Item>'], ['surrogate', '\ud800']]) check(label+'/ShadowRoot rejects '+name+' atomically', () => {
      const before = [...shadow.childNodes], observer = new owner.MutationObserver(() => {});
      observer.observe(shadow, {childList: true, subtree: true}); let error;
      try {setter.call(shadow, markup);} catch (caught) {error = caught;}
      const records = observer.takeRecords(); observer.disconnect();
      return error?.name === 'SyntaxError' && Object.getPrototypeOf(error) === callee.DOMException.prototype && !records.length &&
        shadow.childNodes.length === before.length && before.every((node, index) => shadow.childNodes[index] === node);
    });
    check(label+'/ShadowRoot legacy null clears', () => {setter.call(shadow, null); return shadow.childNodes.length === 0;});
    check(label+'/template innerHTML uses template contents', () => {
      const template = doc.createElementNS(H, 'template'); doc.documentElement.appendChild(template);
      Object.getOwnPropertyDescriptor(callee.Element.prototype, 'innerHTML').set.call(template, '<MixedCase/>tail');
      return template.childNodes.length === 0 && template.content.childNodes.length === 2 && template.content.firstChild.localName === 'MixedCase' && template.content.firstChild.namespaceURI === H;
    });
    check(label+'/template Range returns children', () => {
      const template = doc.createElementNS(H, 'template'); doc.documentElement.appendChild(template);
      const range = doc.createRange(); range.selectNodeContents(template);
      const fragment = callee.Range.prototype.createContextualFragment.call(range, '<MixedCase/>tail');
      return fragment.childNodes.length === 2 && fragment.firstChild.localName === 'MixedCase' && fragment.firstChild.namespaceURI === H && fragment.ownerDocument === doc;
    });
    for (const kind of ['Document', 'DocumentFragment']) check(label+'/'+kind+' Range uses fallback body', () => {
      const range = doc.createRange(); range.selectNodeContents(kind === 'Document' ? doc : doc.createDocumentFragment());
      const fragment = callee.Range.prototype.createContextualFragment.call(range, '<Item/>');
      return fragment.firstChild.localName === 'Item' && fragment.firstChild.namespaceURI === H && fragment.ownerDocument === doc;
    });
    check(label+'/outerHTML fragment parent uses fallback body', () => {
      const fragment = doc.createDocumentFragment(), marker = doc.createElementNS('urn:other', 'Marker'); fragment.appendChild(marker);
      Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').set.call(marker, '<Item/>');
      return marker.parentNode === null && fragment.childNodes.length === 1 && fragment.firstChild.localName === 'Item' && fragment.firstChild.namespaceURI === H && fragment.firstChild.ownerDocument === doc;
    });
  }
  for (const [ownerName, owner] of realms) {
    const policy = owner.trustedTypes.createPolicy('xml-fragment-special-contexts-'+ownerName, {createHTML: value => value});
    for (const kind of ['inner', 'outer', 'adjacent', 'range']) for (const markup of ['<MixedCase/>', '\ud800']) check(ownerName+'/'+kind+'/native TrustedHTML '+(markup.length === 1 ? 'surrogate' : 'value'), () => {
      const doc = makeDocument(owner, 'impl'), parent = doc.createElementNS('urn:parent', 'Parent'), target = doc.createElementNS('urn:target', 'Target');
      doc.documentElement.appendChild(parent); parent.appendChild(target);
      const value = policy.createHTML(markup); Object.defineProperty(value, 'toString', {value() {throw Error('author toString');}});
      let result, error;
      try {
        if (kind === 'inner') target.innerHTML = value;
        else if (kind === 'outer') target.outerHTML = value;
        else if (kind === 'adjacent') target.insertAdjacentHTML('beforeend', value);
        else {const range = doc.createRange(); range.selectNodeContents(target); result = range.createContextualFragment(value);}
      } catch (caught) {error = caught;}
      if (markup.length === 1) return error?.name === 'SyntaxError' && target.parentNode === parent && target.childNodes.length === 0;
      const node = kind === 'outer' ? parent.firstChild : kind === 'range' ? result?.firstChild : target.firstChild;
      return !error && node?.localName === 'MixedCase' && node.namespaceURI === (kind === 'outer' ? 'urn:parent' : 'urn:target');
    });
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
