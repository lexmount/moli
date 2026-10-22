(() => {
  const rows = [];
  const check = (name, category, body) => {
    try { rows.push({name, category, passed: body() === true}); }
    catch (error) { rows.push({name, category, passed: false, error: String(error)}); }
  };
  const H = 'http://www.w3.org/1999/xhtml', N = 'http://www.w3.org/2000/xmlns/';
  const realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const sinks = ['range', 'inner', 'outer', 'beforebegin', 'afterbegin', 'beforeend', 'afterend'];
  const makeDocument = (owner, flavor) => flavor === 'impl' ?
    owner.document.implementation.createDocument('urn:root', 'r:Root') :
    new owner.DOMParser().parseFromString(flavor === 'xml' ? '<Root xmlns="urn:root"/>' :
      '<html xmlns="http://www.w3.org/1999/xhtml"/>', flavor === 'xml' ? 'application/xml' : 'application/xhtml+xml');
  const withContext = (doc, settings, body) => {
    const section = doc.createElementNS('urn:local', 'p:Section');
    section.setAttributeNS(N, 'xmlns:p', 'urn:local');
    const context = doc.createElementNS(settings.reset ? null : settings.prefixed ? 'urn:context' : 'urn:outer',
      settings.prefixed ? 'p:Context' : settings.name || 'Context');
    if (settings.reset) context.setAttributeNS(N, 'xmlns', '');
    if (settings.prefixUri) context.setAttributeNS(N, 'xmlns:q', settings.prefixUri);
    section.appendChild(context); doc.documentElement.appendChild(section);
    try { return body(context); }
    finally { doc.documentElement.removeChild(section); }
  };
  const prepare = (owner, callee, doc, context, sink) => {
    const marker = doc.createElementNS('urn:marker', 'Marker'); marker.textContent = 'keep';
    context.appendChild(marker);
    const range = doc.createRange(); range.selectNodeContents(context);
    const inner = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'innerHTML').set;
    const outer = Object.getOwnPropertyDescriptor(callee.Element.prototype, 'outerHTML').set;
    const insert = callee.Element.prototype.insertAdjacentHTML;
    return {
      marker, range,
      invoke(value) {
        if (sink === 'range') {
          const fragment = callee.Range.prototype.createContextualFragment.call(range, value);
          return {nodes: [...fragment.childNodes], fragment};
        }
        if (sink === 'inner') inner.call(context, value);
        else if (sink === 'outer') outer.call(marker, value);
        else insert.call(sink === 'beforebegin' || sink === 'afterend' ? marker : context, sink, value);
        return {nodes: [...context.childNodes].filter(node => node !== marker), fragment: null};
      },
      rejects(value, exceptionName = 'SyntaxError') {
        const before = [...context.childNodes];
        const observer = new owner.MutationObserver(() => {});
        observer.observe(context, {subtree: true, childList: true, attributes: true, characterData: true});
        let error;
        try { this.invoke(value); } catch (caught) { error = caught; }
        const records = observer.takeRecords(); observer.disconnect();
        return error?.name === exceptionName && Object.getPrototypeOf(error) === callee.DOMException.prototype &&
          records.length === 0 && context.childNodes.length === before.length &&
          before.every((node, index) => context.childNodes[index] === node) && marker.textContent === 'keep';
      }
    };
  };
  for (const [ownerName, owner] of realms) for (const flavor of ['impl', 'xml', 'xhtml']) {
    const doc = makeDocument(owner, flavor);
    for (const [calleeName, callee] of realms) for (const sink of sinks) {
      const label = `${calleeName}/${ownerName}/${flavor}/${sink}`;
      const sample = (name, category, body, settings = {}) => check(label+'/'+name, category, () =>
        withContext(doc, settings, context => body(prepare(owner, callee, doc, context, sink), context)));
      sample('case namespace and node kinds', 'valid', ({invoke}) => {
        const {nodes, fragment} = invoke('<MixedCase p:Token="v"/><p:Item/>tail<![CDATA[a<b]]><!--end--><?target data?>');
        return nodes.length === 6 && nodes.map(node => node.nodeType).join() === '1,1,3,4,8,7' &&
          nodes[0].localName === 'MixedCase' && nodes[0].namespaceURI === 'urn:outer' &&
          nodes[0].getAttributeNS('urn:local', 'Token') === 'v' && nodes[1].localName === 'Item' &&
          nodes[1].prefix === 'p' && nodes[1].namespaceURI === 'urn:local' &&
          nodes[2].data === 'tail' && nodes[3].data === 'a<b' && nodes[4].data === 'end' &&
          nodes[5].target === 'target' && nodes[5].data === 'data' &&
          nodes.every(node => node.ownerDocument === doc) &&
          (!fragment || Object.getPrototypeOf(fragment) === owner.DocumentFragment.prototype);
      });
      sample('context prefix wins', 'valid', ({invoke}) => {
        const {nodes} = invoke('<p:Item/>');
        return nodes[0].namespaceURI === 'urn:context' && nodes[0].prefix === 'p';
      }, {prefixed: true});
      sample('default namespace reset', 'valid', ({invoke}) => {
        const {nodes} = invoke('<Item/>'); return nodes[0].namespaceURI === null && nodes[0].localName === 'Item';
      }, {reset: true});
      sample('namespace declaration escaping', 'valid', ({invoke}) => {
        const {nodes} = invoke('<q:Item/>'); return nodes[0].namespaceURI === 'urn:a&b"c<d>e';
      }, {prefixUri: 'urn:a&b"c<d>e'});
      sample('predefined and numeric entities', 'valid', ({invoke}) => {
        const {nodes} = invoke('&lt;&amp;&quot;&apos;&gt;&#x1F98A;');
        return nodes.length === 1 && nodes[0].data === '<&\"\'>🦊';
      });
      sample('empty markup', 'valid', ({invoke}) => invoke('').nodes.length === 0);
      sample('conversion changes namespace', 'conversion', ({invoke}, context) => {
        let conversions = 0;
        const {nodes} = invoke({toString() {conversions++; context.setAttributeNS(N, 'xmlns:q', 'urn:changed'); return '<q:Item/>';}});
        return conversions === 1 && nodes[0].namespaceURI === 'urn:changed';
      });
      sample('conversion exception identity', 'conversion', ({invoke}) => {
        const sentinel = {}; let conversions = 0;
        try {invoke({toString() {conversions++; throw sentinel;}}); return false;}
        catch (error) {return error === sentinel && conversions === 1;}
      });
      sample('native context ignores public getters', 'native', ({invoke}, context) => {
        const names = ['nodeType', 'localName', 'namespaceURI', 'prefix', 'parentNode', 'ownerDocument', 'attributes', 'lookupNamespaceURI'];
        let reads = 0;
        for (const name of names) Object.defineProperty(context, name, {configurable: true, get() {reads++; throw Error('public '+name);}});
        try {const {nodes} = invoke('<p:Item/>'); return reads === 0 && nodes[0].namespaceURI === 'urn:local';}
        finally {for (const name of names) delete context[name];}
      });
      for (const [name, markup] of [
        ['unclosed', '<Item>'], ['mismatched', '<A></B>'], ['unknown entity', '&unknown;'],
        ['duplicate attribute', '<Item x="1" x="2"/>'],
        ['duplicate expanded attribute', '<Item xmlns:q="urn:local" p:x="1" q:x="2"/>'],
        ['unquoted attribute', '<Item x=value/>'], ['xml declaration', '<?xml version="1.0"?>'],
        ['doctype', '<!DOCTYPE Item><Item/>'], ['nul', '\u0000'], ['vertical tab', '\u000b'],
        ['high surrogate', '\ud800'], ['low surrogate', '\udc00'], ['noncharacter', '\ufffe'],
        ['invalid character reference', '&#xD800;'], ['root escape', '</Context><!--escape--><Context>']
      ]) sample('reject '+name, 'malformed', sample => sample.rejects(markup));
      sample('reject swallowed context end tag', 'malformed', sample => sample.rejects('x</foo--><!--y'), {name: 'foo--'});
      for (const [name, markup] of [
        ['unbound prefix', '<missing:Item/>'], ['empty prefix binding', '<p:Item xmlns:p=""/>'],
        ['xml prefix binding', '<Item xmlns:xml="urn:wrong"/>'],
        ['xmlns prefix binding', '<Item xmlns:xmlns="urn:wrong"/>'],
        ['unused empty prefix', '<Item xmlns:q=""/>'],
        ['reserved XML URI prefix', '<Item xmlns:q="http://www.w3.org/XML/1998/namespace"/>'],
        ['reserved XML URI default', '<Item xmlns="http://www.w3.org/XML/1998/namespace"/>'],
        ['reserved XMLNS URI prefix', '<Item xmlns:q="http://www.w3.org/2000/xmlns/"/>'],
        ['reserved XMLNS URI default', '<Item xmlns="http://www.w3.org/2000/xmlns/"/>']
      ]) sample('reject '+name, 'strict-namespace', sample => sample.rejects(markup));
      sample('XML scripts remain inert after adoption', 'script', ({invoke}) => {
        owner.__xmlFragmentScriptRuns = 0;
        const {nodes} = invoke('<script xmlns="'+H+'">globalThis.__xmlFragmentScriptRuns++;</script>');
        const before = owner.__xmlFragmentScriptRuns;
        const holder = owner.document.createElement('div'); owner.document.body.appendChild(holder);
        holder.append(...nodes);
        const after = owner.__xmlFragmentScriptRuns; holder.remove();
        return before === 0 && after === 0 && nodes[0].namespaceURI === H &&
          Object.getPrototypeOf(nodes[0]) === owner.HTMLScriptElement.prototype;
      });
    }
  }
  for (const [ownerName, owner] of realms) {
    const doc = new owner.DOMParser().parseFromString('<!DOCTYPE Root [<!ENTITY inherited "secret">]><Root/>', 'application/xml');
    for (const [calleeName, callee] of realms) for (const sink of sinks) {
      check(`${calleeName}/${ownerName}/${sink}/no inherited doctype entities`, 'malformed', () =>
        withContext(doc, {}, context => prepare(owner, callee, doc, context, sink).rejects('&inherited;')));
    }
    const control = makeDocument(owner, 'impl').documentElement;
    check(ownerName+'/setHTMLUnsafe uses HTML parser in XML documents', 'control', () => {
      control.setHTMLUnsafe('<MixedCase>');
      return control.firstChild.localName === 'mixedcase';
    });
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
