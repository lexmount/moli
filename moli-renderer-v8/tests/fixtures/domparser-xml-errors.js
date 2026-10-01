function domParserXmlErrorsProbe() {
  const checks = [];
  const check = (label, actual, expected) => checks.push({label, actual, expected, pass: actual === expected});
  const namespace = 'http://www.mozilla.org/newlayout/xml/parsererror.xml';
  const types = ['text/xml', 'application/xml', 'application/xhtml+xml', 'image/svg+xml'];
  const invalid = [
    ['empty', ''],
    ['whitespace', ' \t\r\n'],
    ['prologue-only', '<!--before--><?only data?>'],
    ['unclosed', '<root>'],
    ['mismatched', '<root><child></root>'],
    ['multiple-roots', '<root/><second/>'],
    ['partial-doctype', '<!DOCTYPE root><!--before--><?before data?><root><kept id="original"/><script xmlns="http://www.w3.org/1999/xhtml">globalThis.domParserErrorScriptRan = true;</script><child></root>'],
    ['namespace', '<root undeclared:attr="value"/>'],
    ['encoding-declaration', '<?xml version="1.0" encoding="ISO-8859-1"?><root><child>'],
  ];
  function metadata(doc, mime, label, realm) {
    check(label + '/Document', doc instanceof realm.Document, true);
    check(label + '/contentType', doc.contentType, mime);
    for (const property of ['characterSet', 'charset', 'inputEncoding']) check(label + '/' + property, doc[property], 'UTF-8');
  }
  function errorDocument(realm, parser, source, mime, label) {
    const doc = parser.parseFromString(source, mime);
    metadata(doc, mime, label, realm);
    const root = doc.documentElement;
    check(label + '/namespace', root.namespaceURI, namespace);
    check(label + '/localName', root.localName, 'parsererror');
    check(label + '/tagName', root.tagName, 'parsererror');
    check(label + '/prefix', root.prefix, null);
    check(label + '/only-child', doc.childNodes.length, 1);
    check(label + '/first-child', doc.firstChild === root, true);
    check(label + '/parent', root.parentNode === doc, true);
    check(label + '/owner', root.ownerDocument === doc, true);
    check(label + '/doctype-removed', doc.doctype, null);
    check(label + '/partial-tree-removed', doc.getElementById('original'), null);
    check(label + '/error-count', doc.getElementsByTagName('parsererror').length, 1);
    check(label + '/error-by-namespace', doc.getElementsByTagNameNS(namespace, 'parsererror')[0] === root, true);
    check(label + '/error-description', root.textContent.length > 0, true);
    check(label + '/body', doc.body, null);
    check(label + '/head', doc.head, null);
    check(label + '/script-inert', realm.domParserErrorScriptRan, undefined);
    const serialized = new realm.XMLSerializer().serializeToString(doc);
    check(label + '/serialized-root', serialized.startsWith('<parsererror'), true);
    const roundtrip = parser.parseFromString(serialized, mime);
    check(label + '/roundtrip-namespace', roundtrip.documentElement.namespaceURI, namespace);
    check(label + '/roundtrip-text', roundtrip.documentElement.textContent, root.textContent);
    check(label + '/new-document', roundtrip !== doc, true);
  }
  function run(realm, cases, label) {
    const parser = new realm.DOMParser();
    for (const mime of types) {
      for (const [name, source] of cases) {
        const prefix = label + '/' + mime + '/' + name;
        try { errorDocument(realm, parser, source, mime, prefix); }
        catch (error) { check(prefix + '/exception', String(error.stack || error), 'success'); }
      }
      for (const authorNamespace of [namespace, 'http://www.w3.org/1999/xhtml']) {
        const prefix = label + '/' + mime + '/author/' + authorNamespace;
        try {
          const doc = parser.parseFromString('<!DOCTYPE parsererror><parsererror xmlns="' + authorNamespace + '" data-author="yes"><part xmlns="urn:kept">kept &lt; &amp; &#x1F4A1;</part></parsererror>', mime);
          metadata(doc, mime, prefix, realm);
          check(prefix + '/namespace', doc.documentElement.namespaceURI, authorNamespace);
          check(prefix + '/marker', doc.documentElement.getAttribute('data-author'), 'yes');
          check(prefix + '/doctype', doc.doctype?.name, 'parsererror');
          check(prefix + '/child-namespace', doc.documentElement.firstElementChild.namespaceURI, 'urn:kept');
          check(prefix + '/text', doc.documentElement.textContent, 'kept < & 💡');
        } catch (error) { check(prefix + '/exception', String(error.stack || error), 'success'); }
      }
    }
  }
  run(globalThis, invalid, 'main');
  let frame;
  try {
    frame = document.body.appendChild(document.createElement('iframe'));
    run(frame.contentWindow, [['mismatched', '<root><child></root>']], 'child');
  } catch (error) { check('child/exception', String(error.stack || error), 'success'); }
  finally { frame?.remove(); }
  return {state: checks.every(item => item.pass) ? 'pass' : 'fail', checks};
}
