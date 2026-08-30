use super::*;

#[test]
fn detached_html_elements_use_specific_prototypes_for_common_tags() {
    let mut vm = new_storage_test_vm("https://detached-html-element-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const expected = new Map([
    ['a', 'HTMLAnchorElement'],
    ['img', 'HTMLImageElement'],
    ['form', 'HTMLFormElement'],
    ['input', 'HTMLInputElement'],
    ['button', 'HTMLButtonElement'],
    ['script', 'HTMLScriptElement'],
    ['iframe', 'HTMLIFrameElement'],
    ['canvas', 'HTMLCanvasElement'],
    ['textarea', 'HTMLTextAreaElement'],
    ['select', 'HTMLSelectElement'],
    ['option', 'HTMLOptionElement'],
    ['section', 'HTMLElement']
  ]);
  const doc = new DOMParser().parseFromString(
    '<html><body><img id="parsed"><form id="form"><input id="input"></form></body></html>',
    'text/html'
  );
  const created = [];
  for (const [tag, ctorName] of expected) {
    const element = doc.createElement(tag);
    const ctor = globalThis[ctorName];
    created.push([
      tag,
      Object.prototype.toString.call(element),
      ctor && element instanceof ctor,
      element instanceof HTMLElement,
      element.constructor && element.constructor.name
    ].join(','));
  }
  const parsedImg = doc.getElementById('parsed');
  const parsedInput = doc.getElementById('input');
  const probeDiv = doc.createElement('div');
  probeDiv.innerHTML = '<span class="x"></span>';
  const probeProto = Object.getPrototypeOf(probeDiv);
  const probeProtoParent = Object.getPrototypeOf(probeProto);
  const hasOwn = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const methodShape = [
    hasOwn(probeDiv, 'appendChild'),
    hasOwn(probeDiv, 'querySelector'),
    hasOwn(probeDiv, 'getAttribute'),
    hasOwn(probeDiv, 'matches'),
    probeProto === HTMLDivElement.prototype,
    probeProtoParent === HTMLElement.prototype,
    typeof probeDiv.appendChild,
    probeDiv.querySelector('.x').tagName
  ].join(',');
  const xmlDoc = new DOMParser().parseFromString('<input/>', 'application/xml');
  return [
    created.join(';'),
    Object.prototype.toString.call(parsedImg),
    parsedImg instanceof HTMLImageElement,
    Object.prototype.toString.call(parsedInput),
    parsedInput instanceof HTMLInputElement,
    Object.prototype.toString.call(xmlDoc.documentElement),
    xmlDoc.documentElement instanceof Element,
    xmlDoc.documentElement instanceof HTMLInputElement,
    methodShape
  ].join('|');
})()
"#,
        )
        .expect("detached HTML elements should use common specialized prototypes");

    assert_eq!(
        result,
        "a,[object HTMLAnchorElement],true,true,HTMLAnchorElement;img,[object HTMLImageElement],true,true,HTMLImageElement;form,[object HTMLFormElement],true,true,HTMLFormElement;input,[object HTMLInputElement],true,true,HTMLInputElement;button,[object HTMLButtonElement],true,true,HTMLButtonElement;script,[object HTMLScriptElement],true,true,HTMLScriptElement;iframe,[object HTMLIFrameElement],true,true,HTMLIFrameElement;canvas,[object HTMLCanvasElement],true,true,HTMLCanvasElement;textarea,[object HTMLTextAreaElement],true,true,HTMLTextAreaElement;select,[object HTMLSelectElement],true,true,HTMLSelectElement;option,[object HTMLOptionElement],true,true,HTMLOptionElement;section,[object HTMLElement],true,true,HTMLElement|[object HTMLImageElement]|true|[object HTMLInputElement]|true|[object Element]|true|false|false,false,false,false,true,true,function,SPAN"
    );
}

#[test]
fn live_html_create_element_matches_replay_brands_for_standard_tags() {
    let mut vm = new_storage_test_vm("https://live-html-element-replay-brands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const expected = new Map([
    ['html', 'HTMLHtmlElement'],
    ['head', 'HTMLHeadElement'],
    ['body', 'HTMLBodyElement'],
    ['input', 'HTMLInputElement'],
    ['select', 'HTMLSelectElement'],
    ['option', 'HTMLOptionElement'],
    ['fieldset', 'HTMLFieldSetElement'],
    ['meta', 'HTMLMetaElement'],
    ['title', 'HTMLTitleElement'],
    ['span', 'HTMLSpanElement'],
    ['p', 'HTMLParagraphElement'],
    ['area', 'HTMLAreaElement'],
    ['base', 'HTMLBaseElement'],
    ['br', 'HTMLBRElement'],
    ['data', 'HTMLDataElement'],
    ['datalist', 'HTMLDataListElement'],
    ['map', 'HTMLMapElement'],
    ['object', 'HTMLObjectElement'],
    ['output', 'HTMLOutputElement'],
    ['progress', 'HTMLProgressElement'],
    ['table', 'HTMLTableElement'],
    ['caption', 'HTMLTableCaptionElement'],
    ['col', 'HTMLTableColElement'],
    ['tbody', 'HTMLTableSectionElement'],
    ['tr', 'HTMLTableRowElement'],
    ['slot', 'HTMLSlotElement'],
    ['source', 'HTMLSourceElement'],
    ['del', 'HTMLModElement'],
    ['pre', 'HTMLPreElement'],
    ['frame', 'HTMLFrameElement'],
    ['frameset', 'HTMLFrameSetElement'],
    ['font', 'HTMLFontElement'],
    ['marquee', 'HTMLMarqueeElement'],
    ['meter', 'HTMLMeterElement'],
    ['ul', 'HTMLUListElement'],
    ['section', 'HTMLElement']
  ]);
  const out = [];
  for (const [tag, ctorName] of expected) {
    const element = document.createElement(tag);
    const ctor = globalThis[ctorName];
    out.push([
      tag,
      element.constructor && element.constructor.name,
      Object.prototype.toString.call(element),
      !!ctor && element instanceof ctor,
      element instanceof HTMLElement
    ].join(':'));
  }
  return out.join('|');
})()
"#,
        )
        .expect("live createElement should expose Chromium-like brands");

    assert_eq!(
        result,
        "html:HTMLHtmlElement:[object HTMLHtmlElement]:true:true|head:HTMLHeadElement:[object HTMLHeadElement]:true:true|body:HTMLBodyElement:[object HTMLBodyElement]:true:true|input:HTMLInputElement:[object HTMLInputElement]:true:true|select:HTMLSelectElement:[object HTMLSelectElement]:true:true|option:HTMLOptionElement:[object HTMLOptionElement]:true:true|fieldset:HTMLFieldSetElement:[object HTMLFieldSetElement]:true:true|meta:HTMLMetaElement:[object HTMLMetaElement]:true:true|title:HTMLTitleElement:[object HTMLTitleElement]:true:true|span:HTMLSpanElement:[object HTMLSpanElement]:true:true|p:HTMLParagraphElement:[object HTMLParagraphElement]:true:true|area:HTMLAreaElement:[object HTMLAreaElement]:true:true|base:HTMLBaseElement:[object HTMLBaseElement]:true:true|br:HTMLBRElement:[object HTMLBRElement]:true:true|data:HTMLDataElement:[object HTMLDataElement]:true:true|datalist:HTMLDataListElement:[object HTMLDataListElement]:true:true|map:HTMLMapElement:[object HTMLMapElement]:true:true|object:HTMLObjectElement:[object HTMLObjectElement]:true:true|output:HTMLOutputElement:[object HTMLOutputElement]:true:true|progress:HTMLProgressElement:[object HTMLProgressElement]:true:true|table:HTMLTableElement:[object HTMLTableElement]:true:true|caption:HTMLTableCaptionElement:[object HTMLTableCaptionElement]:true:true|col:HTMLTableColElement:[object HTMLTableColElement]:true:true|tbody:HTMLTableSectionElement:[object HTMLTableSectionElement]:true:true|tr:HTMLTableRowElement:[object HTMLTableRowElement]:true:true|slot:HTMLSlotElement:[object HTMLSlotElement]:true:true|source:HTMLSourceElement:[object HTMLSourceElement]:true:true|del:HTMLModElement:[object HTMLModElement]:true:true|pre:HTMLPreElement:[object HTMLPreElement]:true:true|frame:HTMLFrameElement:[object HTMLFrameElement]:true:true|frameset:HTMLFrameSetElement:[object HTMLFrameSetElement]:true:true|font:HTMLFontElement:[object HTMLFontElement]:true:true|marquee:HTMLMarqueeElement:[object HTMLMarqueeElement]:true:true|meter:HTMLMeterElement:[object HTMLMeterElement]:true:true|ul:HTMLUListElement:[object HTMLUListElement]:true:true|section:HTMLElement:[object HTMLElement]:true:true"
    );
}

#[test]
fn parsed_html_template_content_is_exposed_through_live_wrapper() {
    let mut vm = new_parsed_test_vm(
        "https://live-template-content.test/",
        "<!doctype html><html><body><template id=t>Hello<span id=inner>world</span></template></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const template = document.getElementById('t');
  const content = template.content;
  const childNodes = Array.prototype.map.call(content.childNodes, (node) => {
    return [node.nodeType, node.nodeName, node.nodeValue || node.localName].join(':');
  });
  const descriptor = Object.getOwnPropertyDescriptor(HTMLTemplateElement.prototype, 'content');
  return [
    template.constructor && template.constructor.name,
    Object.prototype.toString.call(template),
    template instanceof HTMLTemplateElement,
    content instanceof DocumentFragment,
    Object.prototype.toString.call(content),
    content.childNodes.length,
    childNodes.join(','),
    content.querySelector('#inner').textContent,
    typeof descriptor.get,
    descriptor.enumerable,
    descriptor.configurable
  ].join('|');
})()
"#,
        )
        .expect("parsed template content should be visible through live wrapper");

    assert_eq!(
        result,
        "HTMLTemplateElement|[object HTMLTemplateElement]|true|true|[object DocumentFragment]|2|3:#text:Hello,1:SPAN:span|world|function|true|true"
    );
}

#[test]
fn parsed_xml_xhtml_template_preserves_interface_and_content() {
    let mut vm = new_storage_test_vm("https://xml-template-content.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xml = new DOMParser().parseFromString(
    "<template xmlns='http://www.w3.org/1999/xhtml'><test/></template>",
    "text/xml"
  );
  const template = xml.documentElement;
  return [
    template.constructor && template.constructor.name,
    Object.prototype.toString.call(template),
    template instanceof HTMLTemplateElement,
    template.childElementCount,
    template.content instanceof DocumentFragment,
    template.content.firstChild.localName,
  ].join('|');
})()
            "#,
        )
        .expect("XML XHTML template content should be exposed");

    assert_eq!(
        result,
        "HTMLTemplateElement|[object HTMLTemplateElement]|true|0|true|test"
    );
}

#[test]
fn xml_serializer_synthesizes_required_namespace_declarations() {
    let mut vm = new_storage_test_vm("https://xml-serializer-namespaces.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xml = document.implementation.createDocument('urn:catalog', 'c:catalog', null);
  const root = xml.documentElement;
  root.setAttributeNS('http://www.w3.org/2000/xmlns/', 'xmlns:m', 'urn:meta');
  const item = xml.createElementNS('urn:catalog', 'c:item');
  item.setAttributeNS('urn:meta', 'm:code', 'code-7');
  item.append(xml.createTextNode('alpha & beta'));
  root.append(item);

  const serialized = new XMLSerializer().serializeToString(xml);
  const reparsed = new DOMParser().parseFromString(serialized, 'application/xml');
  return [
    serialized,
    reparsed.documentElement.namespaceURI,
    reparsed.documentElement.getAttributeNS(
      'http://www.w3.org/2000/xmlns/',
      'c'
    ),
    reparsed.getElementsByTagNameNS('urn:catalog', 'item').length,
    reparsed.getElementsByTagNameNS('urn:catalog', 'item')[0]
      .getAttributeNS('urn:meta', 'code')
  ].join('|');
})()
"#,
        )
        .expect("XMLSerializer namespace projection should evaluate");

    assert_eq!(
        result,
        concat!(
            "<c:catalog xmlns:c=\"urn:catalog\" xmlns:m=\"urn:meta\">",
            "<c:item m:code=\"code-7\">alpha &amp; beta</c:item>",
            "</c:catalog>|urn:catalog|urn:catalog|1|code-7"
        )
    );
}

#[test]
fn xml_serializer_matches_chromium_for_empty_elements_attrs_and_parser_errors() {
    let mut vm = new_storage_test_vm("https://xml-serializer-node-kinds.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const serializer = new XMLSerializer();
  const htmlVoid = document.createElement('br');
  const htmlVoidWithChild = document.createElement('br');
  htmlVoidWithChild.append('child');
  const xml = document.implementation.createDocument(null, 'root');
  const owner = document.createElement('div');
  owner.setAttribute('data-value', 'a<&">\t\n\r');
  const attribute = owner.getAttributeNode('data-value');
  const emptyXml = new DOMParser().parseFromString('', 'text/xml');
  const emptyError = emptyXml.getElementsByTagName('parsererror')[0];
  const emptySerialized = serializer.serializeToString(emptyXml);
  const partialXml = new DOMParser().parseFromString(
    '<catalog><item></catalog>',
    'application/xml'
  );
  const partialError = partialXml.getElementsByTagName('parsererror')[0];
  const partialSerialized = serializer.serializeToString(partialXml);

  return [
    serializer.serializeToString(htmlVoid) ===
      '<br xmlns="http://www.w3.org/1999/xhtml" />',
    serializer.serializeToString(htmlVoidWithChild) ===
      '<br xmlns="http://www.w3.org/1999/xhtml">child</br>',
    serializer.serializeToString(xml.documentElement) === '<root/>',
    serializer.serializeToString(attribute) ===
      'a&lt;&amp;&quot;&gt;&#9;&#10;&#13;',
    emptyXml.documentElement.localName === 'html',
    emptyXml.documentElement.namespaceURI === 'http://www.w3.org/1999/xhtml',
    emptyXml.documentElement.getAttribute('xmlns') === null,
    emptyError.getAttributeNames().join(',') === 'style',
    emptySerialized.startsWith(
      '<html xmlns="http://www.w3.org/1999/xhtml"><body><parsererror style='
    ),
    !emptySerialized.includes('<parsererror xmlns='),
    partialError.getAttribute('xmlns') === null,
    partialSerialized.startsWith(
      '<catalog><parsererror xmlns="http://www.w3.org/1999/xhtml" style='
    )
  ].join('|');
})()
"#,
        )
        .expect("XMLSerializer node-kind projection should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn dom_parser_xml_errors_preserve_the_partial_document_root() {
    let mut vm = new_storage_test_vm("https://dom-parser-partial-xml-error.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parsed = new DOMParser().parseFromString(
    '<catalog><item></catalog>',
    'application/xml'
  );
  const errors = parsed.getElementsByTagName('parsererror');
  return [
    parsed.documentElement.localName,
    errors.length,
    errors[0].parentNode === parsed.documentElement,
    errors[0].namespaceURI,
    errors[0].nextElementSibling.localName,
    errors[0].querySelectorAll('h3').length
  ].join('|');
})()
"#,
        )
        .expect("DOMParser partial XML error tree should evaluate");

    assert_eq!(result, "catalog|1|true|http://www.w3.org/1999/xhtml|item|2");
}

#[test]
fn dom_parser_rejects_public_doctype_without_system_literal() {
    let mut vm = new_storage_test_vm("https://dom-parser-doctype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const prefix = '<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN"';
  const suffix = '><html><div id="test"/></html>';
  const invalid = parser.parseFromString(prefix + suffix, 'application/xhtml+xml');
  const emptySystemId = parser.parseFromString(prefix + ' ""' + suffix, 'application/xhtml+xml');
  const quotedSystemId = parser.parseFromString(prefix + ' "x"' + suffix, 'application/xhtml+xml');

  return [
    invalid.getElementById('test') === null,
    invalid.getElementsByTagName('parsererror').length === 1,
    emptySystemId.getElementById('test') !== null,
    quotedSystemId.getElementById('test') !== null
  ].join('|');
})()
"#,
        )
        .expect("DOMParser doctype system ID validation should evaluate");

    assert_eq!(result, "true|true|true|true");
}

#[test]
fn xhtml_element_interface_survives_move_through_xml_document() {
    let mut vm = new_storage_test_vm("https://xhtml-xml-document-move.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const xml = document.implementation.createDocument(
    "http://www.w3.org/1999/xhtml",
    "html"
  );
  const style = document.createElement("style");
  style.setAttribute("nonce", "allowme");
  const initialInterface = Object.getPrototypeOf(style) === HTMLStyleElement.prototype;

  xml.documentElement.appendChild(style);
  const xmlInterface = Object.getPrototypeOf(style) === HTMLStyleElement.prototype;
  const xmlNonce = style.nonce;

  body.appendChild(style);
  return [
    initialInterface,
    xmlInterface,
    xmlNonce,
    Object.getPrototypeOf(style) === HTMLStyleElement.prototype,
    style.nonce,
    style.getAttribute("nonce")
  ].join("|");
})()
            "#,
        )
        .expect("XHTML element interface should survive XML document adoption");

    assert_eq!(result, "true|true|allowme|true|allowme|");
}

#[test]
fn child_content_document_template_uses_child_realm_template_surface() {
    let mut vm = new_storage_test_vm("https://child-template-content.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  doc.open();
  doc.write('<!doctype html><html><body><template id=t>Hello<span id=inner>world</span></template></body></html>');
  doc.close();
  const template = doc.getElementById('t');
  const content = template.content;
  return [
    template.constructor === frame.contentWindow.HTMLTemplateElement,
    template instanceof frame.contentWindow.HTMLTemplateElement,
    Object.prototype.toString.call(template),
    content instanceof frame.contentWindow.DocumentFragment,
    Object.prototype.toString.call(content),
    content.childNodes.length,
    content.querySelector('#inner').textContent,
    typeof Object.getOwnPropertyDescriptor(frame.contentWindow.HTMLTemplateElement.prototype, 'content').get
  ].join('|');
})()
"#,
        )
        .expect("child template content should be visible through child realm wrapper");

    assert_eq!(
        result,
        "true|true|[object HTMLTemplateElement]|true|[object DocumentFragment]|2|world|function"
    );
}

#[test]
fn live_html_create_element_uses_html_unknown_element_for_observed_unknown_tags() {
    let mut vm = new_storage_test_vm("https://live-html-unknown-element-brands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const tags = ['applet', 'bgsound', 'blink', 'content', 'decorator', 'element', 'image', 'isindex', 'menuitem', 'shadow', 'spacer'];
  return tags.map((tag) => {
    const element = document.createElement(tag);
    return [
      tag,
      element.constructor && element.constructor.name,
      Object.prototype.toString.call(element),
      element instanceof HTMLUnknownElement,
      element instanceof HTMLElement
    ].join(':');
  }).join('|');
})()
"#,
        )
        .expect("unknown replay tags should brand as HTMLUnknownElement");

    assert_eq!(
        result,
        "applet:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|bgsound:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|blink:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|content:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|decorator:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|element:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|image:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|isindex:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|menuitem:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|shadow:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|spacer:HTMLUnknownElement:[object HTMLUnknownElement]:true:true"
    );
}
