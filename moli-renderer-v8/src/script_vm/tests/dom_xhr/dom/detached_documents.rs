use super::*;

#[test]
fn detached_document_tag_collection_exposes_named_item() {
    let mut vm = new_storage_test_vm("https://detached-document-named-item.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><head><meta name="greyEnv" content="prod"><meta id="by-id" content="id"></head></html>',
    'text/html'
  );
  const metas = doc.getElementsByTagName('meta');
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return 'throw:' + error.name;
    }
  };
  return [
    Object.prototype.toString.call(metas),
    typeof metas.namedItem,
    Object.hasOwn(metas, 'length'),
    Object.hasOwn(metas, 'item'),
    Object.hasOwn(metas, 'namedItem'),
    Object.hasOwn(metas, Symbol.iterator),
    Object.getOwnPropertyDescriptor(HTMLCollection.prototype, 'length').get.call(metas),
    metas.namedItem({ toString() { return 'greyEnv'; } }) && metas.namedItem('greyEnv').getAttribute('content'),
    metas.namedItem('by-id') && metas.namedItem('by-id').getAttribute('content'),
    metas.namedItem('missing') === null,
    probe(() => metas.namedItem(undefined)),
    probe(() => metas.namedItem()),
    probe(() => metas.namedItem(Symbol('name')))
  ].join('|');
})()
"#,
        )
        .expect("detached document tag collection namedItem should be available");

    assert_eq!(
        result,
        "[object HTMLCollection]|function|false|false|false|false|2|prod|id|true|null|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn detached_document_insert_updates_collection_and_sibling_surface() {
    let mut vm = new_storage_test_vm("https://detached-dom-surface-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><p id="a" name="alpha"></p><span id="b"></span></body></html>',
    'text/html'
  );
  const inserted = doc.createElement('section');
  inserted.id = 'mid';
  inserted.setAttribute('name', 'middle');
  doc.body.insertBefore(inserted, doc.getElementById('b'));
  const children = doc.body.children;
  return [
    Object.prototype.toString.call(children),
    children.length,
    children.item(0) === doc.getElementById('a'),
    children.item(1) === inserted,
    children.item(99) === null,
    children.namedItem('middle') === inserted,
    children.namedItem('mid') === inserted,
    inserted.previousSibling.id,
    inserted.nextSibling.id,
    doc.body.firstChild.id,
    doc.body.lastChild.id,
    doc.body.childNodes.length
  ].join('|');
})()
"#,
        )
        .expect("detached DOM insertion should refresh collection and sibling surface");

    assert_eq!(
        result,
        "[object HTMLCollection]|3|true|true|true|true|true|a|b|a|b|3"
    );
}

#[test]
fn detached_document_node_mutation_methods_refresh_document_surface() {
    let mut vm = new_storage_test_vm("https://detached-document-node-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<!doctype html><html><head><title>x</title></head><body><p id="a"></p></body></html>',
    'text/html'
  );
  const before = [
    typeof doc.removeChild,
    typeof doc.appendChild,
    typeof doc.insertBefore,
    typeof doc.replaceChild,
    doc.doctype && doc.doctype.nodeType,
    doc.documentElement && doc.documentElement.nodeName,
    doc.body && doc.body.nodeName
  ].join(',');

  const originalRoot = doc.documentElement;
  const doctype = doc.doctype;
  const removed = doc.removeChild(originalRoot);
  const afterRemove = [
    removed === originalRoot,
    doc.documentElement === null,
    doc.body === null,
    doc.firstChild === doctype,
    doc.lastChild === doctype,
    originalRoot.parentNode === null,
    originalRoot.isConnected === false
  ].join(',');

  const replacement = originalRoot.cloneNode(true);
  doc.appendChild(replacement);
  const afterAppend = [
    doc.documentElement === replacement,
    doc.body === replacement.querySelector('body'),
    doc.head === replacement.querySelector('head'),
    doc.firstChild === doctype,
    doc.lastChild === replacement,
    replacement.parentNode === doc,
    replacement.isConnected === true,
    doc.childNodes.length
  ].join(',');

  const html2 = doc.createElement('html');
  doc.replaceChild(html2, replacement);
  const afterReplace = [
    doc.documentElement === html2,
    doc.lastChild === html2,
    replacement.parentNode === null,
    html2.parentNode === doc
  ].join(',');

  return `${before}|${afterRemove}|${afterAppend}|${afterReplace}`;
})()
"#,
        )
        .expect("detached document node mutation methods should refresh document surface");

    assert_eq!(
        result,
        "function,function,function,function,10,HTML,BODY|true,true,true,true,true,true,true|true,true,true,true,true,true,true,2|true,true,true,true"
    );
}

#[test]
fn detached_document_can_append_dom_parser_snapshot_clone() {
    let mut vm = new_storage_test_vm("https://detached-document-dom-parser-clone.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const referenceDoc = document.implementation.createHTMLDocument("");
  referenceDoc.removeChild(referenceDoc.documentElement);
  const snapshot = new DOMParser().parseFromString(
    '<!doctype html><html><head><title>x</title></head><body><p id="a"></p></body></html>',
    'text/html'
  );
  const snapshotClone = snapshot.documentElement.cloneNode(true);
  referenceDoc.appendChild(snapshotClone);
  const childDoc = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body></body></html>',
    'text/html'
  );
  childDoc.removeChild(childDoc.documentElement);
  const referenceClone = referenceDoc.documentElement.cloneNode(true);
  const appendedToChild = childDoc.appendChild(referenceClone);
  return [
    referenceDoc.documentElement && referenceDoc.documentElement.nodeName,
    referenceDoc.body && referenceDoc.body.nodeName,
    referenceDoc.getElementById('a') && referenceDoc.getElementById('a').nodeName,
    referenceDoc.documentElement === snapshotClone,
    snapshotClone.parentNode === null,
    snapshot.documentElement.parentNode === snapshot,
    referenceClone.nodeType,
    referenceClone.nodeName,
    referenceClone.localName,
    referenceClone.namespaceURI,
    referenceClone.childNodes && referenceClone.childNodes.length,
    typeof referenceClone.getAttributeNames,
    appendedToChild && appendedToChild.nodeType,
    appendedToChild && appendedToChild.__lmDomParserId === undefined,
    childDoc.documentElement && childDoc.documentElement.nodeName,
    childDoc.body && childDoc.body.nodeName,
    childDoc.getElementById('a') && childDoc.getElementById('a').nodeName,
    childDoc.documentElement === appendedToChild,
    referenceClone.parentNode === childDoc,
    referenceDoc.documentElement.parentNode === referenceDoc
  ].join('|');
})()
"#,
        )
        .expect("detached document should import DOMParser snapshot clones");

    assert_eq!(
        result,
        "HTML|BODY|P|true|false|true|1|HTML|html|http://www.w3.org/1999/xhtml|2|function|1|true|HTML|BODY|P|true|true|true"
    );
}

#[test]
fn dom_parser_import_uses_native_children_after_child_nodes_tamper() {
    let mut vm = new_storage_test_vm("https://dom-parser-import-native-children.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = new DOMParser().parseFromString(
    '<!doctype html><html><body><section id="source"><span id="real">ok</span></section></body></html>',
    'text/html'
  );
  const section = source.getElementById('source');
  const real = source.getElementById('real');
  const fake = source.createElement('fake-node');
  fake.id = 'fake';
  const projected = section.childNodes;
  projected[0] = fake;
  projected.length = 1;
  source.__lmDomParserId = 999999;
  source.__lmDomParserNode = 999999;
  section.__lmDomParserId = 999999;
  section.__lmDomParserNode = 999999;
  real.__lmDomParserId = 999999;
  real.__lmDomParserNode = 999999;
  const realAfterTamper = source.getElementById('real');

  const target = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body></body></html>',
    'text/html'
  );
  const imported = target.body.appendChild(section);
  return [
    imported.nodeName,
    imported.ownerDocument === target,
    imported.childNodes.length,
    imported.firstChild && imported.firstChild.nodeName,
    imported.firstChild && imported.firstChild.id,
    imported.textContent,
    target.getElementById('real') === imported.firstChild,
    realAfterTamper === real,
    realAfterTamper.parentNode === section,
    target.getElementById('fake') === null,
    fake.parentNode === null
  ].join('|');
})()
"#,
        )
        .expect("DOMParser import should use native source children after childNodes tamper");

    assert_eq!(
        result,
        "SECTION|true|1|SPAN|real|ok|true|true|true|true|true"
    );
}

#[test]
fn dom_parser_native_query_selector_all_declares_node_list_shell() {
    let mut vm = new_storage_test_vm("https://dom-parser-native-node-list-shell.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = new DOMParser().parseFromString(
    '<!doctype html><html><body><section id="root"><span id="a"></span><span id="b"></span></section></body></html>',
    'text/html'
  );
  const root = source.getElementById('root');
  const list = root.querySelectorAll('span');
  const methodShape = (prototype, key) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
    return [
      !!descriptor,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.configurable,
      descriptor && descriptor.writable,
      descriptor && typeof descriptor.value,
      descriptor && descriptor.value.name,
      descriptor && descriptor.value.length
    ].join(':');
  };
  const beforeDataName = Object.getOwnPropertyNames(list).includes('data');
  list.data = { items: [] };
  return [
    Object.prototype.toString.call(list),
    list.constructor && list.constructor.name,
    Object.getPrototypeOf(list) === NodeList.prototype,
    list.length,
    list[0].id,
    list[1].id,
    list[2] === undefined,
    list.item(0).id,
    list.item(1).id,
    list.item(2) === null,
    Array.from(list).map(node => node.id).join(','),
    Object.hasOwn(list, 'length'),
    Object.hasOwn(list, 'item'),
    Object.hasOwn(list, Symbol.iterator),
    methodShape(NodeList.prototype, 'item'),
    methodShape(NodeList.prototype, Symbol.iterator).split(':').slice(0, 5).join(':'),
    NodeList.prototype[Symbol.iterator] === Array.prototype.values,
    beforeDataName,
    Object.prototype.hasOwnProperty.call(list, 'data'),
    list.item(0).id,
    Array.from(list).map(node => node.id).join(',')
  ].join('|');
})()
"#,
        )
        .expect("DOMParser native NodeList shell should be declared");

    assert_eq!(
        result,
        "[object NodeList]|NodeList|true|2|a|b|true|a|b|true|a,b|false|false|false|true:false:true:true:function:item:1|true:false:true:true:function|true|false|true|a|a,b"
    );
}

#[test]
fn dom_parser_import_uses_native_attributes_after_attribute_method_tamper() {
    let mut vm = new_storage_test_vm("https://dom-parser-import-native-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = new DOMParser().parseFromString(
    '<!doctype html><html><body><section id="source" data-real="native"></section></body></html>',
    'text/html'
  );
  const section = source.getElementById('source');
  section.setAttributeNS('urn:attr', 'a:flag', 'value');
  section.getAttributeNames = () => ['data-fake'];
  section.getAttribute = () => 'tampered';

  const target = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body></body></html>',
    'text/html'
  );
  const imported = target.body.appendChild(section);
  return [
    imported.getAttribute('id'),
    imported.getAttribute('data-real'),
    imported.getAttribute('data-fake'),
    imported.getAttributeNS('urn:attr', 'flag'),
    imported.hasAttributeNS('urn:attr', 'flag'),
    imported.getAttributeNames().join(',')
  ].join('|');
})()
"#,
        )
        .expect("DOMParser import should use native source attributes after method tamper");

    assert_eq!(result, "source|native||value|true|id,data-real,a:flag");
}

#[test]
fn parent_node_prepend_can_reuse_the_existing_first_child() {
    let mut vm = new_storage_test_vm("https://parent-node-prepend-existing-first.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  const first = document.createElement('span');
  const second = document.createElement('em');
  first.id = 'first';
  second.id = 'second';
  host.append(first, second);
  (document.body || document.documentElement || document).appendChild(host);

  const firstResult = host.prepend(first);
  const afterFirst = Array.from(host.children, child => child.id).join(',');
  const secondResult = host.prepend(second);
  const afterSecond = Array.from(host.children, child => child.id).join(',');
  host.prepend(second);

  return [
    firstResult === undefined,
    afterFirst,
    secondResult === undefined,
    afterSecond,
    Array.from(host.children, child => child.id).join(',')
  ].join('|');
})()
"#,
        )
        .expect("ParentNode.prepend should move an existing reference child");

    assert_eq!(result, "true|first,second|true|second,first|second,first");
}

#[test]
fn detached_document_parent_node_append_and_prepend_match_child_fixture_needs() {
    let mut vm = new_storage_test_vm("https://detached-document-parent-node-append.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body><p id="host"></p></body></html>',
    'text/html'
  );
  const host = doc.getElementById('host');
  const child = doc.createElement('span');
  child.id = 'local';
  child.setAttribute('data-kind', 'fixture');
  const textHost = doc.createElement('p');
  textHost.textContent = 'seed';
  host.append(child, 'tail');
  host.prepend('head');

  const foreignDoc = document.implementation.createHTMLDocument('');
  const foreign = foreignDoc.createElement('em');
  foreign.id = 'foreign';
  foreign.append('copy');
  host.append(foreign);

  return [
    typeof host.append,
    typeof host.prepend,
    host.childNodes.length,
    host.firstChild.nodeValue,
    host.childNodes[1] === child,
    host.childNodes[2].nodeValue,
    host.lastChild.nodeName,
    host.lastChild.id,
    host.lastChild.textContent,
    foreign.parentNode === null,
    doc.getElementById('foreign') === host.lastChild,
    textHost.firstChild && textHost.firstChild.nodeValue,
    textHost.childNodes.length,
    child.attributes.length,
    child.attributes[0] && child.attributes[0].name,
    child.attributes[0] && child.attributes[0].value,
    child.attributes['data-kind'] && child.attributes['data-kind'].value,
    Object.prototype.toString.call(child.attributes)
  ].join('|');
})()
"#,
        )
        .expect("detached document append/prepend should support strings and foreign clones");

    assert_eq!(
        result,
        "function|function|4|head|true|tail|EM|foreign|copy|false|true|seed|1|2|id|local|fixture|[object NamedNodeMap]"
    );
}

#[test]
fn child_content_document_element_attributes_are_indexed() {
    let mut vm = new_storage_test_vm("https://child-content-document-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(iframe);
  const doc = iframe.contentDocument;
  const node = doc.createElement('p');
  node.id = 'child';
  node.setAttribute('data-kind', 'fixture');
  const referenceDoc = document.implementation.createHTMLDocument('');
  referenceDoc.removeChild(referenceDoc.documentElement);
  doc.body.setAttribute('onload', 'run()');
  const sourceClone = doc.documentElement.cloneNode(true);
  const sourceCloneBody = sourceClone.querySelector('body');
  const sourceCloneBodyAttributes = [
    sourceCloneBody.attributes.length,
    sourceCloneBody.attributes[0] && sourceCloneBody.attributes[0].name,
    sourceCloneBody.attributes[0] && sourceCloneBody.attributes[0].value,
    sourceCloneBody.attributes.item(0) && sourceCloneBody.attributes.item(0).name,
    sourceCloneBody.attributes.getNamedItem('onload') && sourceCloneBody.attributes.getNamedItem('onload').value
  ].join(',');
  referenceDoc.appendChild(sourceClone);
  doc.removeChild(doc.documentElement);
  doc.appendChild(referenceDoc.documentElement.cloneNode(true));
  const restored = doc.createElement('p');
  restored.id = 'restored';
  restored.setAttribute('data-kind', 'clone-path');
  return [
    node.attributes.length,
    node.attributes[0] && node.attributes[0].name,
    node.attributes[0] && node.attributes[0].value,
    node.attributes['data-kind'] && node.attributes['data-kind'].value,
    Object.prototype.toString.call(node.attributes),
    restored.attributes.length,
    restored.attributes[0] && restored.attributes[0].name,
    restored.attributes[0] && restored.attributes[0].value,
    restored.attributes['data-kind'] && restored.attributes['data-kind'].value,
    doc.body.attributes.length,
    doc.body.attributes[0] && doc.body.attributes[0].name,
    doc.body.attributes[0] && doc.body.attributes[0].value,
    sourceCloneBodyAttributes
  ].join('|');
})()
"#,
        )
        .expect("child contentDocument element attributes should be indexed");

    assert_eq!(
        result,
        "2|id|child|fixture|[object NamedNodeMap]|2|id|restored|clone-path|1|onload|run()|1,onload,run(),onload,run()"
    );
}

#[test]
fn detached_html_document_created_elements_persist_direct_attributes() {
    let mut vm = new_storage_test_vm("https://detached-create-html-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('');
  const p = doc.createElement('p');
  p.setAttribute('data-kind', 'local');
  p.id = 'host';
  doc.body.setAttribute('onload', 'run()');
  const a = doc.createElement('a');
  a.href = 'http://example.org/?ä';
  const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 10 10');
  return [
    p.getAttribute('data-kind'),
    p.hasAttribute('data-kind'),
    p.attributes.length,
    p.attributes[0] && p.attributes[0].name,
    p.attributes[0] && p.attributes[0].value,
    p.attributes.getNamedItem('data-kind') && p.attributes.getNamedItem('data-kind').value,
    p.id,
    p.getAttribute('id'),
    doc.body.getAttribute('onload'),
    doc.body.attributes[0] && doc.body.attributes[0].name,
    a.getAttribute('href'),
    a.href,
    svg.getAttribute('viewBox'),
    svg.getAttribute('viewbox')
  ].join('|');
})()
"#,
        )
        .expect("detached createHTMLDocument elements should persist direct attributes");

    assert_eq!(
        result,
        "local|true|2|data-kind|local|local|host|host|run()|onload|http://example.org/?ä|http://example.org/?%C3%A4|0 0 10 10|"
    );
}

#[test]
fn constructed_document_xhtml_anchor_reflects_url_href() {
    let mut vm = new_storage_test_vm("https://constructed-document-anchor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new Document();
  const a = doc.createElementNS('http://www.w3.org/1999/xhtml', 'a');
  a.href = 'http://example.org/?ä';
  return [
    a.constructor === HTMLAnchorElement,
    a instanceof HTMLAnchorElement,
    a.getAttribute('href'),
    a.href
  ].join('|');
})()
"#,
        )
        .expect("constructed Document XHTML anchors should reflect URL href");

    assert_eq!(
        result,
        "true|true|http://example.org/?ä|http://example.org/?%C3%A4"
    );
}

#[test]
fn detached_html_document_namespaced_attributes_import_with_metadata() {
    let mut vm = new_storage_test_vm("https://detached-create-html-ns-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('Title');
  doc.body.setAttributeNS('http://example.com/', 'p:name', 'value');
  doc.body.removeAttribute('p:name');
  const removedByQualifiedName = [
    doc.body.getAttribute('p:name') === null,
    doc.body.getAttributeNS('http://example.com/', 'name') === null,
    doc.body.getAttributeNodeNS('http://example.com/', 'name') === null
  ].join(',');
  doc.body.setAttributeNS('http://example.com/', 'p:name', 'value');
  const originalAttr = doc.body.getAttributeNodeNS('http://example.com/', 'name');
  const imported = document.importNode(originalAttr, true);
  const beforeRemove = [
    removedByQualifiedName,
    doc.body.getAttribute('p:name'),
    doc.body.getAttributeNS('http://example.com/', 'name'),
    doc.body.hasAttributeNS('http://example.com/', 'name'),
    originalAttr && originalAttr.name,
    originalAttr && originalAttr.prefix,
    originalAttr && originalAttr.namespaceURI,
    originalAttr && originalAttr.localName,
    imported && imported.prefix,
    imported && imported.namespaceURI,
    imported && imported.localName
  ].join('|');
  doc.body.removeAttributeNS('http://example.com/', 'name');
  return [
    beforeRemove,
    doc.body.getAttribute('p:name') === null,
    doc.body.getAttributeNS('http://example.com/', 'name') === null,
    doc.body.getAttributeNodeNS('http://example.com/', 'name') === null
  ].join('|');
})()
"#,
        )
        .expect("detached createHTMLDocument NS attributes should import");

    assert_eq!(
        result,
        "true,true,true|value|value|true|p:name|p|http://example.com/|name|p|http://example.com/|name|true|true|true"
    );
}

#[test]
fn live_attr_object_cache_uses_private_slot_and_ignores_public_spoofing() {
    let mut vm = new_storage_test_vm("https://attr-cache-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const element = document.createElement('div');
  body.appendChild(element);
  element.setAttribute('data-real', 'one');
  element.setAttributeNS('urn:attr-cache', 'p:flag', 'ns-one');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliAttrObjectCache'))
    .sort()
    .join(',');
  const named = element.getAttributeNode('data-real');
  const namespaced = element.getAttributeNodeNS('urn:attr-cache', 'flag');
  const afterCacheNames = internalNames(element);
  const fakeCache = Object.create(null);
  fakeCache['data-real'] = {
    name: 'data-real',
    value: 'fake',
    ownerElement: null,
    namespaceURI: null,
    localName: 'data-real'
  };
  Element.prototype.__moliAttrObjectCache = fakeCache;
  element.__moliAttrObjectCache = fakeCache;
  const spoofedOwnNames = internalNames(element);
  const namedAfterSpoof = element.getAttributeNode('data-real');
  const namespacedAfterSpoof = element.getAttributeNodeNS('urn:attr-cache', 'flag');
  element.removeAttribute('data-real');
  element.removeAttributeNS('urn:attr-cache', 'flag');
  return JSON.stringify({
    afterCacheNames,
    spoofedOwnNames,
    sameNamed: namedAfterSpoof === named,
    sameNamespaced: namespacedAfterSpoof === namespaced,
    namedValue: namedAfterSpoof && namedAfterSpoof.value,
    namespacedValue: namespacedAfterSpoof && namespacedAfterSpoof.value,
    namedDetached: named.ownerElement === null && named.value === 'one',
    namespacedDetached: namespaced.ownerElement === null && namespaced.value === 'ns-one',
    namedRemoved: element.getAttributeNode('data-real') === null,
    namespacedRemoved: element.getAttributeNodeNS('urn:attr-cache', 'flag') === null
  });
})()
"#,
        )
        .expect("live Attr cache should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"afterCacheNames":"","spoofedOwnNames":"__moliAttrObjectCache","sameNamed":true,"sameNamespaced":true,"namedValue":"one","namespacedValue":"ns-one","namedDetached":true,"namespacedDetached":true,"namedRemoved":true,"namespacedRemoved":true}"#
    );
}

#[test]
fn detached_document_adopts_live_namespaced_attributes() {
    let mut vm = new_storage_test_vm("https://detached-adopt-live-ns-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createDocument('urn:doc', 'root', null);
  const source = document.createElementNS('urn:source', 's:item');
  source.setAttribute('data-real', 'native');
  source.setAttributeNS('urn:attr', 'a:flag', 'value');
  const realGetAttribute = source.getAttribute;
  const realGetAttributeNames = source.getAttributeNames;
  source.getAttributeNames = () => ['data-fake'];
  source.getAttribute = () => 'tampered';
  const adopted = doc.documentElement.appendChild(source);
  const attr = adopted.getAttributeNodeNS('urn:attr', 'flag');
  return [
    realGetAttribute.call(adopted, 'data-real'),
    realGetAttribute.call(adopted, 'data-fake'),
    realGetAttribute.call(adopted, 'a:flag'),
    adopted.getAttributeNS('urn:attr', 'flag'),
    adopted.hasAttributeNS('urn:attr', 'flag'),
    realGetAttributeNames.call(adopted).join(','),
    attr && attr.name,
    attr && attr.prefix,
    attr && attr.namespaceURI,
    attr && attr.localName,
    attr && attr.value
  ].join('|');
})()
"#,
        )
        .expect("detached documents should preserve adopted live namespaced attributes");

    assert_eq!(
        result,
        "native||value|value|true|data-real,a:flag|a:flag|a|urn:attr|flag|value"
    );
}

#[test]
fn detached_element_clone_preserves_namespaced_attributes() {
    let mut vm = new_storage_test_vm("https://detached-clone-ns-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createDocument('urn:doc', 'root', null);
  const source = doc.createElementNS('urn:source', 's:item');
  source.setAttributeNS('urn:attr', 'a:flag', 'value');
  const clone = source.cloneNode(false);
  const attr = clone.getAttributeNodeNS('urn:attr', 'flag');
  return [
    clone.getAttributeNames().join(','),
    clone.getAttribute('a:flag'),
    clone.getAttributeNS('urn:attr', 'flag'),
    clone.hasAttributeNS('urn:attr', 'flag'),
    attr && attr.name,
    attr && attr.prefix,
    attr && attr.namespaceURI,
    attr && attr.localName,
    attr && attr.value
  ].join('|');
})()
"#,
        )
        .expect("detached cloneNode should preserve namespaced attributes");

    assert_eq!(
        result,
        "a:flag|value|value|true|a:flag|a|urn:attr|flag|value"
    );
}

#[test]
fn detached_html_template_content_uses_separate_owner_document() {
    let mut vm = new_storage_test_vm("https://detached-template-content-owner.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptorShape = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(",");
  };
  const doc = document.implementation.createHTMLDocument('');
  const template = doc.createElement('template');
  doc.body.appendChild(template);
  template.content.appendChild(doc.createElement('span'));
  const templateContent = template.content;
  const templateContentDeleteResult = delete template.content;
  template.content = doc.createElement('div');
  DOMParser = function() {
    throw new Error('page-tampered DOMParser should not run');
  };
  doc.body.innerHTML = '<template><div>some text</div></template>';
  const parsedTemplate = doc.querySelector('template');
  const parsedContent = parsedTemplate && parsedTemplate.content;
  const parsedContentDeleteResult = parsedTemplate && delete parsedTemplate.content;
  if (parsedTemplate) {
    parsedTemplate.content = doc.createElement('span');
  }
  return [
    template.content.ownerDocument !== doc,
    template.content.ownerDocument.defaultView === null,
    template.content.firstChild.ownerDocument === template.content.ownerDocument,
    template.content.firstChild.localName,
    template.ownerDocument === doc,
    doc.body.childNodes.length,
    doc.body.innerHTML,
    parsedTemplate !== null,
    parsedTemplate && parsedTemplate.content.ownerDocument !== doc,
    parsedTemplate && parsedTemplate.content.ownerDocument.defaultView === null,
    descriptorShape(HTMLTemplateElement.prototype, 'content'),
    Object.prototype.hasOwnProperty.call(template, 'content'),
    template.content === templateContent,
    templateContentDeleteResult,
    Object.keys(template).includes('content'),
    parsedTemplate && descriptorShape(HTMLTemplateElement.prototype, 'content'),
    parsedTemplate && Object.prototype.hasOwnProperty.call(parsedTemplate, 'content'),
    parsedTemplate && parsedTemplate.content === parsedContent,
    parsedContentDeleteResult,
    parsedTemplate && Object.keys(parsedTemplate).includes('content')
  ].join('|');
})()
"#,
        )
        .expect("detached template content owner should evaluate");

    assert_eq!(
        result,
        "true|true|true|span|true|1|<template><div>some text</div></template>|true|true|true|function,true,true,true|false|true|true|false|function,true,true,true|false|true|true|false"
    );
}

#[test]
fn detached_html_image_decode_uses_prototype_method() {
    let mut vm = new_storage_test_vm("https://detached-image-decode-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('');
  const image = doc.createElement('img');
  const descriptor = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, 'decode');
  return [
    typeof image.decode,
    descriptor.value === image.decode,
    descriptor.value.name,
    descriptor.value.length,
    descriptor.enumerable,
    descriptor.writable,
    descriptor.configurable,
    Object.keys(HTMLImageElement.prototype).includes('decode'),
    Object.keys(image).includes('decode'),
    Object.prototype.hasOwnProperty.call(image, 'decode'),
    typeof image.decode().then
  ].join('|');
})()
"#,
        )
        .expect("detached image decode surface should evaluate");

    assert_eq!(
        result,
        "function|true|decode|0|true|true|true|true|false|false|function"
    );
}

#[test]
fn frameset_inner_html_ignores_parser_inserted_template() {
    let mut vm = new_storage_test_vm("https://frameset-template-fragment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frameset = document.createElement('frameset');
  frameset.innerHTML = '<template>some text</template>';
  const parsedTemplate = frameset.querySelector('template');
  const template = document.createElement('template');
  frameset.appendChild(template);
  const detached = document.implementation.createHTMLDocument('');
  const detachedFrameset = detached.createElement('frameset');
  detachedFrameset.innerHTML = '<template>some text</template>';
  const detachedParsedTemplate = detachedFrameset.querySelector('template');
  detachedFrameset.appendChild(detached.createElement('template'));
  return [
    parsedTemplate === null,
    frameset.querySelectorAll('template').length,
    detachedParsedTemplate === null,
    detachedFrameset.querySelectorAll('template').length
  ].join('|');
})()
"#,
        )
        .expect("frameset innerHTML template handling should evaluate");

    assert_eq!(result, "true|1|true|1");
}

#[test]
fn html_reflection_regression_slice_matches_wpt_expectations() {
    let mut vm = new_storage_test_vm("https://reflection-regression-slice.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.createElement('form');
  form.acceptCharset = 'utf-8';
  form.setAttribute('autocomplete', 'OFF');
  const formAutocomplete = form.autocomplete;

  const hr = document.createElement('hr');
  hr.color = 'red';
  hr.noShade = true;
  hr.size = '4';

  const script = document.createElement('script');
  script.crossOrigin = undefined;
  const scriptMissing = script.getAttribute('crossorigin') === null && script.crossOrigin === null;
  script.setAttribute('crossorigin', 'invalid');
  const scriptInvalid = script.crossOrigin;
  script.setAttribute('src', '');

  const img = document.createElement('img');
  img.crossOrigin = undefined;
  const imgMissing = img.getAttribute('crossorigin') === null && img.crossOrigin === null;
  img.setAttribute('crossorigin', '');
  const imgEmpty = img.crossOrigin;
  img.setAttribute('src', '');
  img.isMap = true;
  img.width = 2147483648;

  const mod = document.createElement('ins');
  mod.setAttribute('cite', ' foo ');

  const a = document.createElement('a');
  a.href = '';

  return [
    form.getAttribute('accept-charset'),
    form.acceptCharset,
    formAutocomplete,
    hr.getAttribute('color'),
    hr.color,
    hr.noShade,
    hr.size,
    scriptMissing,
    scriptInvalid,
    script.src === location.href,
    imgMissing,
    imgEmpty,
    img.isMap,
    img.getAttribute('width'),
    mod.cite,
    a.protocol,
    a.host,
    a.pathname,
    img.src === location.href
  ].join('|');
})()
"#,
        )
        .expect("HTML reflection regression slice should evaluate");

    assert_eq!(
        result,
        "utf-8|utf-8|off|red|red|true|4|true|anonymous|true|true|anonymous|true|0|https://reflection-regression-slice.test/path/foo|https:|reflection-regression-slice.test|/path/page.html|true"
    );
}

#[test]
fn range_insert_node_rejects_ancestor_without_splitting_text() {
    let mut vm = new_storage_test_vm("https://range-insert-node-hierarchy.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const p = document.createElement('p');
  p.textContent = 'abc';
  (document.body || document.documentElement || document).appendChild(p);
  const text = p.firstChild;
  const range = document.createRange();
  range.setStart(text, 0);
  range.setEnd(text, 0);
  Node.prototype.contains = () => false;
  let thrown = null;
  try {
    range.insertNode(p);
  } catch (e) {
    thrown = e;
  }
  return [
    thrown && thrown.name,
    thrown && thrown.code,
    p.firstChild === text,
    text.data,
    p.childNodes.length,
    range.startContainer === text,
    range.startOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode should reject ancestor insertion before text split");

    assert_eq!(result, "HierarchyRequestError|3|true|abc|1|true|0");
}
