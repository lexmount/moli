(async () => {
  const checks = [];
  const check = (name, actual, expected) => {
    checks.push({name, passed: JSON.stringify(actual) === JSON.stringify(expected), actual, expected});
  };
  const capture = (action, sentinel) => {
    try { action(); return null; }
    catch (error) { return error === sentinel ? 'sentinel' : error.name; }
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = globalThis.__attributeOrderIncludePopup === false ? null : open('');
  const realms = [['main', window], ['iframe', frame.contentWindow]];
  if (popup) realms.push(['popup', popup]);
  try {
    for (const [realmName, w] of realms) {
      const docs = [
        ['live', w.document],
        ['html', w.document.implementation.createHTMLDocument('')],
        ['xml', w.document.implementation.createDocument(null, 'root')],
        ['parsed', new w.DOMParser().parseFromString('<root/>', 'application/xml')],
      ];
      for (const [docName, doc] of docs) {
        for (const [method, namespace, name, expectedError] of [
          ['setAttribute', null, 'x y', 'InvalidCharacterError'],
          ['setAttributeNS', 'urn:test', 'x y', 'InvalidCharacterError'],
          ['setAttributeNS', null, 'p:name', 'NamespaceError'],
          ['setAttributeNS', 'urn:test', 'xml:name', 'NamespaceError'],
        ]) {
          for (const shape of ['plain', 'throw', 'symbol', 'primitive-throw']) {
            const element = doc.createElement('div');
            const records = new w.MutationObserver(() => {});
            records.observe(element, {attributes: true});
            const log = [];
            const sentinel = {};
            const value = shape === 'symbol' ? Symbol('value') :
              shape === 'primitive-throw' ? {[Symbol.toPrimitive](hint) {log.push(hint); throw sentinel;}} :
              {toString() {log.push('value'); if (shape === 'throw') throw sentinel; return 'ok';}};
            const args = method === 'setAttribute' ? [name, value] : [namespace, name, value];
            const error = capture(() => element[method](...args), sentinel);
            const expectedLog = shape === 'symbol' ? [] : [shape === 'primitive-throw' ? 'string' : 'value'];
            const expected = shape === 'symbol' ? 'TypeError' : shape.includes('throw') ? 'sentinel' : expectedError;
            const label = `${realmName}/${docName}/${method}/${name}/${shape}`;
            check(label + '/order', [log, error], [expectedLog, expected]);
            check(label + '/mutation', [element.attributes.length, records.takeRecords().length], [0, 0]);
            records.disconnect();
          }
        }

        for (const method of ['setAttribute', 'setAttributeNS']) {
          const element = doc.createElement('div');
          const ordinalCount = method === 'setAttribute' ? 2 : 3;
          for (let stop = -1; stop < ordinalCount; stop++) {
            const log = [], sentinel = {};
            const texts = method === 'setAttribute' ? ['data-order', 'value'] : ['urn:test', 'p:order', 'value'];
            const args = texts.map((text, index) => ({toString() {log.push(index); if (index === stop) throw sentinel; return text;}}));
            const error = capture(() => element[method](...args), sentinel);
            const label = `${realmName}/${docName}/${method}/argument-${stop}`;
            check(label, [log, error], [Array.from({length: stop < 0 ? ordinalCount : stop + 1}, (_, index) => index), stop < 0 ? null : 'sentinel']);
          }
          for (let count = 0; count < ordinalCount; count++) {
            const log = [];
            const args = Array.from({length: count}, () => ({toString() {log.push('unexpected'); return 'x y';}}));
            check(`${realmName}/${docName}/${method}/arity-${count}`, [capture(() => element[method](...args)), log], ['TypeError', []]);
          }
          for (const text of ['', '\ud800', 'a\udc00z', '\ud83d\ude00', 'nul\u0000value']) {
            let conversions = 0;
            const value = {toString() {conversions++; return text;}};
            if (method === 'setAttribute') element[method]('data-units', value);
            else element[method]('urn:test', 'p:units', value);
            const actual = method === 'setAttribute' ? element.getAttribute('data-units') : element.getAttributeNS('urn:test', 'units');
            check(`${realmName}/${docName}/${method}/UTF16-${JSON.stringify(text)}`, [actual, conversions], [text, 1]);
          }
          const invalidReceivers = [{}, Object.create(element), new w.Proxy(element, {})];
          const revoked = w.Proxy.revocable(element, {}); revoked.revoke(); invalidReceivers.push(revoked.proxy);
          for (let index = 0; index < invalidReceivers.length; index++) {
            const log = [];
            const arg = {toString() {log.push('unexpected'); return 'value';}};
            const args = method === 'setAttribute' ? [arg, arg] : [arg, arg, arg];
            const error = capture(() => w.Element.prototype[method].call(invalidReceivers[index], ...args));
            check(`${realmName}/${docName}/${method}/receiver-${index}`, [error, log], ['TypeError', []]);
          }
        }
      }

      for (const method of ['setAttribute', 'setAttributeNS']) {
        for (const intoHtml of [false, true]) {
          const html = w.document.implementation.createHTMLDocument('');
          const xml = w.document.implementation.createDocument(null, 'root');
          const source = intoHtml ? xml : html, target = intoHtml ? html : xml;
          const element = source.createElementNS('http://www.w3.org/1999/xhtml', 'div');
          let calls = 0;
          const value = {toString() {calls++; target.adoptNode(element); return '\ud800value';}};
          if (method === 'setAttribute') element[method]('DATA-CASE', value);
          else element[method](null, 'DATA-CASE', value);
          const name = method === 'setAttribute' && intoHtml ? 'data-case' : 'DATA-CASE';
          check(`${realmName}/${method}/adopt-${intoHtml}`, [element.ownerDocument === target, element.getAttributeNames(), element.getAttributeNodeNS(null, name)?.value, calls], [true, [name], '\ud800value', 1]);
        }
      }

      const policy = w.trustedTypes.createPolicy('attribute-order', {
        createHTML: value => value,
        createScript: value => value,
        createScriptURL: value => value,
      });
      for (const method of ['setAttribute', 'setAttributeNS']) {
        for (const kind of ['HTML', 'Script', 'ScriptURL']) {
          const trusted = policy['create' + kind]('trusted-\ud800');
          Object.defineProperty(trusted, 'toString', {value() {throw new Error('author toString must not run');}});
          const element = w.document.createElement('div');
          const args = method === 'setAttribute' ? ['data-trusted', trusted] : ['urn:test', 'p:trusted', trusted];
          check(`${realmName}/${method}/native-Trusted${kind}`, capture(() => element[method](...args)), null);
          const actual = method === 'setAttribute' ? element.getAttribute('data-trusted') : element.getAttributeNS('urn:test', 'trusted');
          check(`${realmName}/${method}/native-Trusted${kind}/value`, actual, kind === 'ScriptURL' ? 'trusted-\ufffd' : 'trusted-\ud800');
        }
      }
    }

    const strict = document.createElement('iframe');
    const loaded = new Promise(resolve => strict.addEventListener('load', resolve, {once: true}));
    strict.srcdoc = `<!doctype html><meta http-equiv="Content-Security-Policy" content="require-trusted-types-for 'script'; trusted-types *"><body>`;
    document.body.appendChild(strict);
    await loaded;
    try {
      const w = strict.contentWindow;
      const policyLog = [];
      w.trustedTypes.createPolicy('default', {createScript(value) {policyLog.push(value); return value;}});
      for (const ns of [false, true]) {
        const element = w.document.createElement('div');
        let calls = 0;
        const value = {toString() {calls++; return 'return false;';}};
        policyLog.length = 0;
        const invalid = capture(() => ns ? element.setAttributeNS(null, 'p:onclick', value) : element.setAttribute('on click', value));
        check(`default-policy/${ns}/invalid`, [invalid, calls, policyLog], [ns ? 'NamespaceError' : 'InvalidCharacterError', 1, []]);
        policyLog.length = 0; calls = 0;
        const valid = capture(() => ns ? element.setAttributeNS(null, 'onclick', value) : element.setAttribute('onclick', value));
        check(`default-policy/${ns}/valid`, [valid, calls, policyLog, element.getAttribute('onclick')], [null, 1, ['return false;'], 'return false;']);
      }
    } finally { strict.remove(); }
  } catch (error) {
    checks.push({name: 'uncaught', passed: false, actual: String(error), expected: 'no exception'});
  } finally { frame.remove(); if (popup) popup.close(); }
  globalThis.__uiEventResults = {complete: true, includePopup: Boolean(popup), total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
