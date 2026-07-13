use super::*;

#[test]
fn detached_document_state_and_collections_use_document_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-document-prototype-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (name, hasSetter = false) => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const names = [
    "currentScript",
    "hidden",
    "visibilityState",
    "prerendering",
    "domain",
    "scrollingElement",
    "forms",
    "images",
    "scripts",
    "links",
    "anchors",
    "embeds",
    "plugins",
    "applets"
  ];
  for (const name of names) {
    accessor(name, name === "domain");
  }

  const html = document.implementation.createHTMLDocument("");
  html.body.innerHTML = [
    "<form></form>",
    "<img>",
    "<script></script>",
    "<a href='/x'></a>",
    "<a name='anchor'></a>",
    "<embed>"
  ].join("");
  const parsed = new DOMParser().parseFromString(html.documentElement.outerHTML, "text/html");
  const xml = document.implementation.createDocument("urn:test", "root", null);
  const htmlOwnBefore = names.map((name) => own(html, name)).join(",");
  const parsedOwnBefore = names.map((name) => own(parsed, name)).join(",");
  const htmlKeys = Object.keys(html).filter((name) => names.includes(name)).join(",");

  assert(html.currentScript === null, "html currentScript");
  assert(html.hidden === true, "html hidden");
  assert(html.visibilityState === "hidden", "html visibility");
  assert(html.prerendering === false, "html prerendering");
  assert(html.scrollingElement === html.documentElement, "html scrollingElement");
  assert(html.forms.length === 1, "html forms");
  assert(html.images.length === 1, "html images");
  assert(html.scripts.length === 1, "html scripts");
  assert(html.links.length === 1, "html links");
  assert(html.anchors.length === 1, "html anchors");
  assert(html.embeds.length === 1, "html embeds");
  assert(html.plugins.length === 1, "html plugins");
  assert(html.applets.length === 0, "html applets");
  assert(parsed.images.length === 1, "parsed images");
  assert(parsed.hidden === true, "parsed hidden");
  assert(parsed.visibilityState === "hidden", "parsed visibility");
  assert(xml.images instanceof HTMLCollection && xml.images.length === 0, "xml images");
  assert(xml.hidden === true, "xml hidden");
  assert(xml.visibilityState === "hidden", "xml visibility");

  for (const name of names) {
    html[name];
    parsed[name];
    assert(!own(html, name), `${name} should not become own on html`);
    assert(!own(parsed, name), `${name} should not become own on parsed`);
  }

  return [
    htmlOwnBefore,
    parsedOwnBefore,
    htmlKeys,
    html.domain,
    parsed.domain
  ].join("|");
})()
"#,
        )
        .expect("detached Document state and collection prototype accessors should evaluate");

    assert_eq!(
        result,
        "false,false,false,false,false,false,false,false,false,false,false,false,false,false|false,false,false,false,false,false,false,false,false,false,false,false,false,false|||detached-document-prototype-state.test"
    );
}

#[test]
fn document_collections_track_html_elements_in_xml_and_detached_html_documents() {
    let mut vm = new_storage_test_vm("https://document-collections.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = "http://www.w3.org/1999/xhtml";
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const equal = (list, values, message) => assert(
    list.length === values.length && values.every((value, i) => list[i] === value), message
  );
  const factories = [
    () => new Document(),
    () => document.implementation.createDocument(null, "root"),
    () => document.implementation.createDocument("http://www.w3.org/2000/svg", "svg"),
    () => new DOMParser().parseFromString("<root/>", "application/xml"),
    () => new DOMParser().parseFromString(`<root xmlns="${html}"/>`, "application/xhtml+xml"),
    () => document.implementation.createHTMLDocument(""),
    () => new DOMParser().parseFromString("<!doctype html><body></body>", "text/html")
  ];
  const kinds = [
    ["images", "img"], ["forms", "form"], ["scripts", "script"],
    ["links", "a", "href"], ["anchors", "a", "name"],
    ["embeds", "embed"], ["plugins", "embed"], ["applets", "applet"]
  ];
  const errors = [];
  for (const [index, factory] of factories.entries()) {
    for (const [name, tag, attr] of kinds) {
      try {
        const doc = factory(), list = doc[name];
        const root = doc.body || doc.documentElement || doc.appendChild(doc.createElement("root"));
        assert(list instanceof HTMLCollection, "HTMLCollection brand");
        assert(list === doc[name], "SameObject before mutation");
        const make = (ns, qualifiedName) => {
          const node = doc.createElementNS(ns, qualifiedName);
          if (attr) node.setAttributeNS(null, attr, "");
          root.appendChild(node);
          return node;
        };
        make(null, tag);
        make("urn:foreign", tag);
        make(html, tag.toUpperCase());
        const first = make(html, tag), second = make(html, "h:" + tag);
        if (name === "applets") {
          make(html, "object");
          make(html, "__moli-never-match__");
          equal(list, [], "applets never matches elements");
          continue;
        }
        equal(list, [first, second], "exact namespace and local name");
        first.setAttribute("id", "first");
        second.setAttribute("name", "second");
        assert(list.namedItem("first") === first && list.second === second, "named access");
        root.insertBefore(second, first);
        equal(list, [second, first], "reordering");
        assert(list.item(1) === first, "item identity");
        const other = document.implementation.createDocument(null, "other");
        const otherList = other[name];
        other.documentElement.appendChild(other.adoptNode(second));
        equal(list, [first], "adoption removes member");
        equal(otherList, [second], "adoption adds member");
        first.setAttribute("id", "renamed");
        assert(list.namedItem("first") === null && list.renamed === first, "renamed id");
        if (attr) {
          first.removeAttributeNS(null, attr);
          first.setAttributeNS("urn:attribute", attr, "");
          first.setAttributeNS(null, attr.toUpperCase(), "");
          equal(list, [], "namespaced and uppercase attributes excluded");
          first.setAttributeNS(null, attr, "");
          equal(list, [first], "empty unnamespaced attribute included");
        }
        if (name === "links") {
          const area = make(html, "area");
          equal(list, [first, area], "area with href included");
          area.removeAttributeNS(null, "href");
          equal(list, [first], "area href removal");
        }
        first.remove();
        equal(list, [], "node removal");
        assert(list.renamed === undefined, "removed named property");
        assert(list === doc[name], "SameObject after mutation");
        assert(doc.plugins === doc.embeds, "plugins aliases embeds");
      } catch (error) {
        errors.push(`${index}/${name}: ${error.message}`);
      }
    }
  }
  return JSON.stringify(errors);
})()
"#,
        )
        .expect("Document collection mutation and filtering probes should evaluate");

    assert_eq!(result, "[]");
}

#[test]
fn detached_legacy_boolean_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-legacy-boolean-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const compactOwners = [
    [HTMLDirectoryElement.prototype, doc.createElement("dir"), "dir"],
    [HTMLDListElement.prototype, doc.createElement("dl"), "dl"],
    [HTMLMenuElement.prototype, doc.createElement("menu"), "menu"],
    [HTMLOListElement.prototype, doc.createElement("ol"), "ol"],
    [HTMLUListElement.prototype, doc.createElement("ul"), "ul"]
  ];
  for (const [prototype] of compactOwners) {
    accessor(prototype, "compact");
  }
  accessor(HTMLHRElement.prototype, "noShade");
  assert(!own(HTMLElement.prototype, "compact"), "compact should not be on HTMLElement.prototype");
  assert(!own(HTMLElement.prototype, "noShade"), "noShade should not be on HTMLElement.prototype");
  const div = doc.createElement("div");
  assert(!("compact" in div), "plain HTMLElement compact absent");
  assert(!("noShade" in div), "plain HTMLElement noShade absent");

  for (const [, element, label] of compactOwners) {
    assert(!own(element, "compact"), `${label}.compact should not be own before set`);
    element.compact = true;
    assert(element.compact === true, `${label}.compact true`);
    assert(element.hasAttribute("compact"), `${label}.compact attr`);
    assert(!own(element, "compact"), `${label}.compact should not be own after true`);
    element.compact = false;
    assert(element.compact === false, `${label}.compact false`);
    assert(!element.hasAttribute("compact"), `${label}.compact attr removed`);
    element.compact = true;
    assert(delete element.compact, `${label}.compact delete`);
    assert(!own(element, "compact"), `${label}.compact should stay inherited`);
    assert(element.compact === true, `${label}.compact after delete`);
  }

  const hr = doc.createElement("hr");
  assert(!own(hr, "noShade"), "hr.noShade should not be own before set");
  hr.noShade = true;
  assert(hr.noShade === true, "hr.noShade true");
  assert(hr.hasAttribute("noshade"), "hr.noShade attr");
  assert(!own(hr, "noShade"), "hr.noShade should not be own after true");
  hr.noShade = false;
  assert(hr.noShade === false, "hr.noShade false");
  assert(!hr.hasAttribute("noshade"), "hr.noShade attr removed");
  hr.noShade = true;
  assert(delete hr.noShade, "hr.noShade delete");
  assert(!own(hr, "noShade"), "hr.noShade should stay inherited");
  assert(hr.noShade === true, "hr.noShade after delete");
  return "ok";
})()
"#,
        )
        .expect("detached legacy boolean owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn document_parse_html_unsafe_uses_about_blank_document_metadata() {
    let mut vm = new_storage_test_vm("https://parse-html-unsafe-url.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const parsed = Document.parseHTMLUnsafe("<html><head></head><body></body></html>");
  return [parsed.URL, parsed.documentURI, parsed.baseURI].join("|");
})()
"#,
        )
        .expect("Document.parseHTMLUnsafe URL metadata probe should evaluate");

    assert_eq!(result, "about:blank|about:blank|about:blank");
}

#[test]
fn document_metadata_uses_document_and_node_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://document-metadata-prototype.test/root/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const names = [
    "URL",
    "documentURI",
    "readyState",
    "contentType",
    "characterSet",
    "charset",
    "inputEncoding",
    "compatMode",
    "referrer"
  ];
  const descriptorShape = (object, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(object, name);
    return [
      !!descriptor,
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const htmlDoc = document.implementation.createHTMLDocument("");
  const xhtmlDoc = document.implementation.createDocument("http://www.w3.org/1999/xhtml", "html", null);
  const svgDoc = document.implementation.createDocument("http://www.w3.org/2000/svg", "svg", null);
  const xmlDoc = document.implementation.createDocument("urn:test", "root", null);
  const parsed = new DOMParser().parseFromString("<html><head><base href='https://base.example/path/'></head><body></body></html>", "text/html");
  const docs = [document, htmlDoc, xhtmlDoc, svgDoc, xmlDoc, parsed];
  const descriptorSummary = names.map((name) => descriptorShape(Document.prototype, name)).join("|");
  const ownSummary = docs.map((doc) => names.map((name) => own(doc, name)).join(",")).join("|");
  const deleteResult = delete htmlDoc.URL;
  htmlDoc.URL = "https://shadow.example/";
  htmlDoc.readyState = "shadow";
  const values = [
    document.URL,
    document.documentURI,
    htmlDoc.URL,
    htmlDoc.documentURI,
    htmlDoc.readyState,
    htmlDoc.contentType,
    htmlDoc.characterSet,
    htmlDoc.charset,
    htmlDoc.inputEncoding,
    htmlDoc.compatMode,
    htmlDoc.referrer,
    xhtmlDoc.contentType,
    svgDoc.contentType,
    xmlDoc.contentType,
    parsed.baseURI,
    Object.getOwnPropertyDescriptor(Document.prototype, "URL").get.call(htmlDoc) === htmlDoc.URL
  ].join(",");
  return [
    descriptorSummary,
    descriptorShape(Node.prototype, "baseURI"),
    ownSummary,
    docs.map((doc) => own(doc, "baseURI")).join(","),
    deleteResult,
    own(htmlDoc, "URL"),
    own(htmlDoc, "readyState"),
    values
  ].join("||");
})()
"#,
        )
        .expect("document metadata prototype accessors should evaluate");

    assert_eq!(
        result,
        "true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true||true:function:true:true:true||false,false,false,false,false,false,false,false,false|false,false,false,false,false,false,false,false,false|false,false,false,false,false,false,false,false,false|false,false,false,false,false,false,false,false,false|false,false,false,false,false,false,false,false,false|false,false,false,false,false,false,false,false,false||false,false,false,false,false,false||true||false||false||https://document-metadata-prototype.test/root/page.html,https://document-metadata-prototype.test/root/page.html,about:blank,about:blank,complete,text/html,UTF-8,UTF-8,UTF-8,CSS1Compat,,application/xhtml+xml,image/svg+xml,application/xml,https://base.example/path/,true"
    );
}

#[test]
fn document_last_modified_uses_source_time_and_readonly_document_accessor() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    let mut vm = new_storage_test_vm("https://document-last-modified.test/");
    vm.document_runtime
        .set_document_source_last_modified(Some(5_025_000.0));
    environment.set_timezone(Some("Asia/Shanghai")).unwrap();

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, "lastModified");
  return JSON.stringify({
    value: document.lastModified,
    own: Object.prototype.hasOwnProperty.call(document, "lastModified"),
    descriptor: [
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ]
  });
})()
"#,
        )
        .expect("document lastModified should evaluate");

    assert_eq!(
        result,
        r#"{"value":"01/01/1970 09:23:45","own":false,"descriptor":["function",true,true,true]}"#
    );
}

#[test]
fn document_structure_uses_document_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://document-structure-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor.get;
  };

  const documentElementGetter = accessor("documentElement");
  const doctypeGetter = accessor("doctype");
  const xmlDoctype = document.implementation.createDocumentType("qorflesnorf", "pub", "sys");
  const xmlDoc = document.implementation.createDocument("urn:test", "root", xmlDoctype);
  const htmlDoc = document.implementation.createHTMLDocument("");
  const parsedDoc = new DOMParser().parseFromString(
    "<!doctype html><html><body><p>parsed</p></body></html>",
    "text/html"
  );
  const emptyDoc = document.implementation.createDocument(null, null, null);
  const docs = [document, htmlDoc, xmlDoc, parsedDoc, emptyDoc];

  for (const doc of docs) {
    for (const name of ["documentElement", "doctype"]) {
      assert(!own(doc, name), `${name} should not be own before use`);
      assert(!Object.keys(doc).includes(name), `${name} should not be enumerable own`);
    }
  }

  assert(htmlDoc.documentElement.localName === "html", "HTML documentElement");
  assert(htmlDoc.doctype.name === "html", "HTML doctype");
  assert(xmlDoc.documentElement.localName === "root", "XML documentElement localName");
  assert(xmlDoc.documentElement.namespaceURI === "urn:test", "XML documentElement namespace");
  assert(xmlDoc.doctype === xmlDoctype, "XML doctype identity");
  assert(xmlDoc.doctype.name === "qorflesnorf", "XML doctype name");
  assert(xmlDoc.doctype.publicId === "pub", "XML doctype publicId");
  assert(xmlDoc.doctype.systemId === "sys", "XML doctype systemId");
  assert(parsedDoc.documentElement.localName === "html", "parsed documentElement");
  assert(parsedDoc.doctype.name === "html", "parsed doctype");
  assert(emptyDoc.documentElement === null, "empty documentElement");
  assert(emptyDoc.doctype === null, "empty doctype");
  assert(documentElementGetter.call(xmlDoc) === xmlDoc.documentElement, "documentElement getter identity");
  assert(doctypeGetter.call(xmlDoc) === xmlDoctype, "doctype getter identity");

  assert(delete htmlDoc.documentElement, "delete documentElement");
  assert(delete htmlDoc.doctype, "delete doctype");
  htmlDoc.documentElement = document.createElement("span");
  htmlDoc.doctype = xmlDoctype;
  assert(!own(htmlDoc, "documentElement"), "documentElement should not become own");
  assert(!own(htmlDoc, "doctype"), "doctype should not become own");
  assert(htmlDoc.documentElement.localName === "html", "documentElement after assignment");
  assert(htmlDoc.doctype.name === "html", "doctype after assignment");
  return "ok";
})()
"#,
        )
        .expect("document structure prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_document_type_metadata_uses_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-doctype-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(DocumentType.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor.get;
  };

  const getters = {
    name: accessor("name"),
    publicId: accessor("publicId"),
    systemId: accessor("systemId")
  };
  const xmlDoctype = document.implementation.createDocumentType("qorflesnorf", "pub", "sys");
  const xmlDoc = document.implementation.createDocument("urn:test", "root", xmlDoctype);
  const htmlDoc = document.implementation.createHTMLDocument("");
  const parsedDoc = new DOMParser().parseFromString(
    "<!doctype html><html><body></body></html>",
    "text/html"
  );
  const doctypes = [xmlDoctype, xmlDoc.doctype, htmlDoc.doctype, parsedDoc.doctype];

  for (const doctype of doctypes) {
    for (const name of ["name", "publicId", "systemId"]) {
      assert(!own(doctype, name), `${name} should not be own before use`);
      assert(!Object.keys(doctype).includes(name), `${name} should not be enumerable own`);
    }
  }

  assert(xmlDoc.doctype === xmlDoctype, "XML doctype identity");
  assert(xmlDoctype.name === "qorflesnorf", "XML doctype name");
  assert(xmlDoctype.publicId === "pub", "XML doctype publicId");
  assert(xmlDoctype.systemId === "sys", "XML doctype systemId");
  assert(htmlDoc.doctype.name === "html", "HTML doctype name");
  assert(htmlDoc.doctype.publicId === "", "HTML doctype publicId");
  assert(htmlDoc.doctype.systemId === "", "HTML doctype systemId");
  assert(parsedDoc.doctype.name === "html", "parsed doctype name");

  assert(getters.name.call(xmlDoctype) === "qorflesnorf", "name getter");
  assert(getters.publicId.call(xmlDoctype) === "pub", "publicId getter");
  assert(getters.systemId.call(xmlDoctype) === "sys", "systemId getter");

  assert(delete xmlDoctype.name, "delete name");
  assert(delete xmlDoctype.publicId, "delete publicId");
  assert(delete xmlDoctype.systemId, "delete systemId");
  xmlDoctype.name = "shadow";
  xmlDoctype.publicId = "shadow";
  xmlDoctype.systemId = "shadow";
  assert(xmlDoctype.name === "qorflesnorf", "name after assignment");
  assert(xmlDoctype.publicId === "pub", "publicId after assignment");
  assert(xmlDoctype.systemId === "sys", "systemId after assignment");
  for (const name of ["name", "publicId", "systemId"]) {
    assert(!own(xmlDoctype, name), `${name} should not become own`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached DocumentType metadata prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn document_html_structure_uses_document_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://document-html-structure-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const titleDescriptor = accessor("title", true);
  const headDescriptor = accessor("head", false);
  const bodyDescriptor = accessor("body", true);
  const htmlDoc = document.implementation.createHTMLDocument("Initial");
  const parsedDoc = new DOMParser().parseFromString(
    "<!doctype html><html><head><title>Parsed</title></head><body><p>body</p></body></html>",
    "text/html"
  );
  const xmlDoc = document.implementation.createDocument("urn:test", "root", null);
  const docs = [document, htmlDoc, parsedDoc, xmlDoc];

  for (const doc of docs) {
    for (const name of ["title", "head", "body"]) {
      assert(!own(doc, name), `${name} should not be own before use`);
      assert(!Object.keys(doc).includes(name), `${name} should not be enumerable own`);
    }
  }

  assert(htmlDoc.title === "Initial", "HTML title");
  assert(htmlDoc.head.localName === "head", "HTML head");
  assert(htmlDoc.body.localName === "body", "HTML body");
  assert(parsedDoc.title === "Parsed", "parsed title");
  assert(parsedDoc.head.localName === "head", "parsed head");
  assert(parsedDoc.body.firstElementChild.localName === "p", "parsed body");
  assert(xmlDoc.title === "", "XML title");
  assert(xmlDoc.head === null, "XML head");
  assert(xmlDoc.body === null, "XML body");
  assert(titleDescriptor.get.call(parsedDoc) === "Parsed", "title getter call");
  assert(headDescriptor.get.call(parsedDoc) === parsedDoc.head, "head getter identity");
  assert(bodyDescriptor.get.call(parsedDoc) === parsedDoc.body, "body getter identity");

  htmlDoc.title = "Changed";
  assert(htmlDoc.title === "Changed", "title setter");
  assert(htmlDoc.querySelector("title").textContent === "Changed", "title text");
  const replacementBody = htmlDoc.createElement("body");
  replacementBody.append(htmlDoc.createElement("main"));
  htmlDoc.body = replacementBody;
  assert(htmlDoc.body === replacementBody, "body setter identity");
  assert(htmlDoc.body.firstElementChild.localName === "main", "body setter content");

  assert(delete htmlDoc.title, "delete title");
  assert(delete htmlDoc.head, "delete head");
  assert(delete htmlDoc.body, "delete body");
  htmlDoc.title = "AfterDelete";
  htmlDoc.head = document.createElement("head");
  htmlDoc.body = htmlDoc.createElement("body");
  assert(!own(htmlDoc, "title"), "title should not become own");
  assert(!own(htmlDoc, "head"), "head should not become own");
  assert(!own(htmlDoc, "body"), "body should not become own");
  assert(htmlDoc.title === "AfterDelete", "title after delete");
  assert(htmlDoc.head.localName === "head", "head after assignment");
  assert(htmlDoc.body.localName === "body", "body after delete");
  return "ok";
})()
"#,
        )
        .expect("document HTML structure prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn document_body_getter_and_setter_follow_body_or_frameset_semantics() {
    let mut vm = new_storage_test_vm("https://document-body-or-frameset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const emptyDocument = () => {
    const doc = document.implementation.createHTMLDocument("");
    doc.removeChild(doc.documentElement);
    return doc;
  };
  const errorName = callback => {
    try {
      callback();
      return null;
    } catch (error) {
      return error.name;
    }
  };

  const ordered = emptyDocument();
  const orderedRoot = ordered.appendChild(ordered.createElement("html"));
  const firstFrameset = orderedRoot.appendChild(ordered.createElement("frameset"));
  orderedRoot.appendChild(ordered.createElement("body"));
  assert(ordered.body === firstFrameset, "first frameset wins over later body");

  const nested = emptyDocument();
  const nestedRoot = nested.appendChild(nested.createElement("html"));
  nestedRoot.appendChild(nested.createElement("x"))
    .appendChild(nested.createElement("frameset"));
  const directFrameset = nestedRoot.appendChild(nested.createElement("frameset"));
  assert(nested.body === directFrameset, "nested frameset is ignored");

  const typed = document.implementation.createHTMLDocument("");
  assert(errorName(() => { typed.body = "text"; }) === "TypeError", "string type error");
  assert(
    errorName(() => { typed.body = typed.createTextNode("text"); }) === "TypeError",
    "Text type error"
  );
  assert(
    errorName(() => { typed.body = typed.createElementNS("urn:test", "body"); }) === "TypeError",
    "foreign-namespace element type error"
  );
  assert(
    errorName(() => { typed.body = typed.createElement("div"); }) === "HierarchyRequestError",
    "HTMLElement algorithm error"
  );

  const replacementFrameset = typed.createElement("frameset");
  typed.body = replacementFrameset;
  assert(typed.body === replacementFrameset, "frameset setter identity");
  const replacementBody = typed.createElement("body");
  typed.body = replacementBody;
  assert(replacementFrameset.parentNode === null, "old frameset detached");
  assert(typed.body === replacementBody, "body replaces frameset");

  const firstMatch = emptyDocument();
  const firstMatchRoot = firstMatch.appendChild(firstMatch.createElement("html"));
  const oldBody = firstMatchRoot.appendChild(firstMatch.createElement("body"));
  const trailingFrameset = firstMatchRoot.appendChild(firstMatch.createElement("frameset"));
  const newFrameset = firstMatch.createElement("frameset");
  firstMatch.body = newFrameset;
  assert(oldBody.parentNode === null, "first body detached");
  assert(newFrameset.nextSibling === trailingFrameset, "first match replacement position");
  assert(firstMatch.body === newFrameset, "new frameset is getter result");

  const nonHtmlRoot = emptyDocument();
  const testRoot = nonHtmlRoot.appendChild(nonHtmlRoot.createElement("test"));
  const insertedBody = nonHtmlRoot.createElement("body");
  nonHtmlRoot.body = insertedBody;
  assert(nonHtmlRoot.documentElement === testRoot, "non-html documentElement identity");
  assert(testRoot.firstChild === insertedBody, "setter appends to non-html root");
  assert(nonHtmlRoot.body === null, "getter rejects non-html root");
  return "ok";
})()
"#,
        )
        .expect("Document body-or-frameset probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_processing_instruction_target_uses_prototype_accessor() {
    let mut vm = new_storage_test_vm("https://detached-pi-target-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createDocument("urn:test", "root", null);
  const pi = doc.createProcessingInstruction("xml-stylesheet", "href='x.css'");
  const descriptor = Object.getOwnPropertyDescriptor(ProcessingInstruction.prototype, "target");
  const before = [
    !!descriptor,
    typeof descriptor.get,
    descriptor.set === undefined,
    descriptor.enumerable,
    descriptor.configurable,
    Object.prototype.hasOwnProperty.call(pi, "target"),
    Object.keys(pi).includes("target"),
    pi.target
  ].join(":");
  const deleteResult = delete pi.target;
  pi.target = "page-shadow";
  return [
    before,
    deleteResult,
    pi.target,
    descriptor.get.call(pi),
    Object.prototype.hasOwnProperty.call(pi, "target")
  ].join("|");
})()
"#,
        )
        .expect("detached ProcessingInstruction target prototype accessor should evaluate");

    assert_eq!(
        result,
        "true:function:true:true:true:false:false:xml-stylesheet|true|xml-stylesheet|xml-stylesheet|false"
    );
}

#[test]
fn detached_target_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-target-prototypes.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const anchor = doc.createElement("a");
  const area = doc.createElement("area");
  const base = doc.createElement("base");
  const link = doc.createElement("link");
  const form = doc.createElement("form");
  const div = doc.createElement("div");
  doc.head.append(base, link);
  doc.body.append(anchor, area, form, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const targetDescriptors = [
    [accessor(HTMLAnchorElement.prototype, "target"), anchor, "anchor"],
    [accessor(HTMLAreaElement.prototype, "target"), area, "area"],
    [accessor(HTMLBaseElement.prototype, "target"), base, "base"],
    [accessor(HTMLLinkElement.prototype, "target"), link, "link"],
    [accessor(HTMLFormElement.prototype, "target"), form, "form"]
  ];
  assert(!own(HTMLElement.prototype, "target"), "target should not be on HTMLElement.prototype");
  assert(!("target" in div), "target should not be on div");

  for (const [descriptor, element, label] of targetDescriptors) {
    assert(!own(element, "target"), `${label}.target should not be own before set`);
    descriptor.set.call(element, `${label}-target`);
    assert(element.target === `${label}-target`, `${label}.target getter`);
    assert(descriptor.get.call(element) === `${label}-target`, `${label}.target direct getter`);
    assert(element.getAttribute("target") === `${label}-target`, `${label}.target attr`);
    assert(!own(element, "target"), `${label}.target should not be own after set`);
    for (const receiver of [{}, doc.createTextNode("x"), div]) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${label}.target getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, "bad")), `${label}.target setter receiver`);
    }
    for (const [, otherElement, otherLabel] of targetDescriptors) {
      if (otherElement === element) continue;
      assert(throwsTypeError(() => descriptor.get.call(otherElement)), `${label}.target getter rejects ${otherLabel}`);
      assert(throwsTypeError(() => descriptor.set.call(otherElement, "bad")), `${label}.target setter rejects ${otherLabel}`);
    }
    assert(delete element.target, `${label}.target delete`);
    assert(!own(element, "target"), `${label}.target should stay inherited`);
    assert(element.target === `${label}-target`, `${label}.target after delete`);
  }
  return "ok";
})()
"##,
        )
        .expect("detached target owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_rel_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-rel-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const anchor = doc.createElement("a");
  const area = doc.createElement("area");
  const form = doc.createElement("form");
  const link = doc.createElement("link");
  const div = doc.createElement("div");
  doc.head.append(link);
  doc.body.append(anchor, area, form, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const cases = [
    [HTMLAnchorElement.prototype, anchor, "anchor"],
    [HTMLAreaElement.prototype, area, "area"],
    [HTMLFormElement.prototype, form, "form"],
    [HTMLLinkElement.prototype, link, "link"]
  ];
  for (const [prototype] of cases) {
    accessor(prototype, "rel");
    accessor(prototype, "relList");
  }
  assert(!own(HTMLElement.prototype, "rel"), "rel should not be on HTMLElement.prototype");
  assert(!own(HTMLElement.prototype, "relList"), "relList should not be on HTMLElement.prototype");
  assert(!("rel" in div), "rel should not be on div");
  assert(!("relList" in div), "relList should not be on div");

  for (const [, element, label] of cases) {
    assert(!own(element, "rel"), `${label}.rel should not be own before set`);
    assert(!own(element, "relList"), `${label}.relList should not be own before set`);
    const list = element.relList;
    assert(Object.prototype.toString.call(list) === "[object DOMTokenList]", `${label}.relList tag`);
    assert(list === element.relList, `${label}.relList should be stable`);
    element.rel = `${label}-one ${label}-two ${label}-one`;
    assert(element.rel === `${label}-one ${label}-two ${label}-one`, `${label}.rel getter`);
    assert(element.getAttribute("rel") === `${label}-one ${label}-two ${label}-one`, `${label}.rel attr`);
    assert(list.length === 2, `${label}.relList length`);
    assert(list.contains(`${label}-one`), `${label}.relList contains`);
    element.relList = `${label}-three`;
    assert(element.rel === `${label}-three`, `${label}.relList setter`);
    assert(list.length === 1 && list.contains(`${label}-three`), `${label}.relList after setter`);
    assert(!own(element, "rel"), `${label}.rel should not be own after set`);
    assert(!own(element, "relList"), `${label}.relList should not be own after set`);
    assert(delete element.rel, `${label}.rel delete`);
    assert(delete element.relList, `${label}.relList delete`);
    assert(!own(element, "rel"), `${label}.rel should stay inherited`);
    assert(!own(element, "relList"), `${label}.relList should stay inherited`);
    assert(element.rel === `${label}-three`, `${label}.rel after delete`);
    assert(element.relList === list, `${label}.relList stable after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached rel owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_form_methods_use_standard_descriptors() {
    let mut vm = new_storage_test_vm("https://detached-form-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const form = doc.createElement("form");
  const input = doc.createElement("input");
  input.setAttribute("value", "default");
  input.value = "changed";
  form.appendChild(input);
  const summarize = (name) => {
    const method = form[name];
    const descriptor = Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, name);
    return [
      typeof method,
      method.name,
      method.length,
      descriptor.value === method,
      descriptor.writable,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(form, name)
    ].join(":");
  };
  form.reset();
  form.submit();
  return [
    summarize("requestSubmit"),
    summarize("submit"),
    summarize("reset"),
    summarize("checkValidity"),
    summarize("reportValidity"),
    input.value
  ].join("|");
})()
"#,
        )
        .expect("detached form method descriptors should evaluate");

    assert_eq!(
        result,
        "function:requestSubmit:1:true:true:true:true:false|function:submit:0:true:true:true:true:false|function:reset:0:true:true:true:true:false|function:checkValidity:0:true:true:true:true:false|function:reportValidity:0:true:true:true:true:false|default"
    );
}

#[test]
fn document_template_methods_keep_declared_reflection_shape() {
    let mut vm = new_storage_test_vm("https://document-template-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methods = [
    ["getElementById", 1, false],
    ["createElement", 1],
    ["createAttribute", 1],
    ["createAttributeNS", 2],
    ["createElementNS", 2],
    ["createTextNode", 1],
    ["createComment", 1],
    ["createDocumentFragment", 0],
    ["createProcessingInstruction", 2],
    ["createCDATASection", 1],
    ["importNode", 2],
    ["adoptNode", 1],
    ["write", 0],
    ["writeln", 0],
    ["open", 0],
    ["close", 0],
    ["execCommand", 1],
    ["elementFromPoint", 2],
    ["elementsFromPoint", 2],
    ["caretPositionFromPoint", 2],
    ["createNodeIterator", 1],
    ["createTreeWalker", 1],
    ["createNSResolver", 1],
    ["createExpression", 1],
    ["evaluate", 2],
    ["hasStorageAccess", 0],
    ["requestStorageAccess", 0]
  ];
  for (const [name, length, enumerable = true] of methods) {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    if (
      typeof descriptor?.value !== "function" ||
      descriptor.value.name !== name ||
      descriptor.value.length !== length ||
      descriptor.writable !== true ||
      descriptor.enumerable !== enumerable ||
      descriptor.configurable !== true ||
      Object.prototype.hasOwnProperty.call(document, name)
    ) {
      throw new Error(`${name}:${JSON.stringify(descriptor)}`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("Document template method descriptors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_form_accessors_use_html_form_element_prototype() {
    let mut vm = new_storage_test_vm("https://detached-form-accessors.test/base/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const form = doc.createElement("form");
  const input = doc.createElement("input");
  input.setAttribute("name", "q");
  form.appendChild(input);
  doc.body.appendChild(form);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessorNames = [
    "action",
    "acceptCharset",
    "autocomplete",
    "enctype",
    "encoding",
    "elements",
    "length",
    "method",
    "name",
    "noValidate",
    "target"
  ];
  const readonly = new Set(["elements", "length"]);
  for (const name of accessorNames) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === !readonly.has(name), `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    assert(!own(form, name), `${name} should not be own`);
  }

  form.name = "search";
  form.target = "frame";
  form.method = "POST";
  form.noValidate = true;
  form.acceptCharset = "utf-8";
  form.action = "/submit";
  const deleteResult = delete form.name && delete form.elements && delete form.length;

  assert(deleteResult, "delete should report success");
  assert(!own(form, "name"), "name should stay inherited");
  assert(!own(form, "elements"), "elements should stay inherited");
  assert(!own(form, "length"), "length should stay inherited");
  assert(form.name === "search", "name reflection");
  assert(form.target === "frame", "target reflection");
  assert(form.method === "post", "method normalization");
  assert(form.noValidate === true, "noValidate reflection");
  assert(form.acceptCharset === "utf-8", "acceptCharset reflection");
  assert(form.getAttribute("action") === "/submit", "action setter");
  assert(form.length === 1, "length");
  assert(form.elements[0] === input, "indexed element");
  assert(form.elements.namedItem("q") === input, "named element");
  return "ok";
})()
"#,
        )
        .expect("detached form prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_form_associated_name_uses_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-form-associated-name.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const cases = [
    ["button", HTMLButtonElement.prototype],
    ["fieldset", HTMLFieldSetElement.prototype],
    ["input", HTMLInputElement.prototype],
    ["object", HTMLObjectElement.prototype],
    ["output", HTMLOutputElement.prototype],
    ["select", HTMLSelectElement.prototype],
    ["textarea", HTMLTextAreaElement.prototype]
  ];
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  for (const [tag, prototype] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "name");
    assert(!!descriptor, `${tag} name descriptor missing`);
    assert(typeof descriptor.get === "function", `${tag} name getter`);
    assert(typeof descriptor.set === "function", `${tag} name setter`);
    assert(descriptor.enumerable === true, `${tag} name enumerable`);
    assert(descriptor.configurable === true, `${tag} name configurable`);

    const element = doc.createElement(tag);
    assert(!own(element, "name"), `${tag} name should not be own initially`);
    element.name = `${tag}-name`;
    assert(element.getAttribute("name") === `${tag}-name`, `${tag} name setter`);
    assert(element.name === `${tag}-name`, `${tag} name getter`);
    assert(!own(element, "name"), `${tag} name should stay inherited after set`);
    assert(delete element.name, `${tag} delete name`);
    assert(!own(element, "name"), `${tag} name should stay inherited after delete`);
    assert(element.name === `${tag}-name`, `${tag} name after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached form-associated name prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_form_associated_form_uses_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-form-associated-form.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const form = doc.createElement("form");
  const idForm = doc.createElement("form");
  idForm.id = "owner";
  doc.body.append(form, idForm);
  const cases = [
    ["button", HTMLButtonElement.prototype],
    ["fieldset", HTMLFieldSetElement.prototype],
    ["input", HTMLInputElement.prototype],
    ["object", HTMLObjectElement.prototype],
    ["output", HTMLOutputElement.prototype],
    ["select", HTMLSelectElement.prototype],
    ["textarea", HTMLTextAreaElement.prototype]
  ];
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  for (const [tag, prototype] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "form");
    assert(!!descriptor, `${tag} form descriptor missing`);
    assert(typeof descriptor.get === "function", `${tag} form getter`);
    assert(descriptor.set === undefined, `${tag} form setter`);
    assert(descriptor.enumerable === true, `${tag} form enumerable`);
    assert(descriptor.configurable === true, `${tag} form configurable`);

    const nested = doc.createElement(tag);
    form.appendChild(nested);
    assert(!own(nested, "form"), `${tag} nested form should not be own`);
    assert(nested.form === form, `${tag} nested form owner`);
    assert(delete nested.form, `${tag} delete nested form`);
    nested.form = null;
    assert(!own(nested, "form"), `${tag} nested form should stay inherited`);
    assert(nested.form === form, `${tag} nested form after assignment`);

    const associated = doc.createElement(tag);
    associated.setAttribute("form", "owner");
    doc.body.appendChild(associated);
    assert(!own(associated, "form"), `${tag} associated form should not be own`);
    assert(associated.form === idForm, `${tag} associated form owner`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached form-associated form prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_form_control_values_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-form-control-value-prototypes.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const absent = (prototype, name) => {
    assert(!Object.getOwnPropertyDescriptor(prototype, name), `${prototype.constructor.name}.${name} should be absent`);
  };

  const button = doc.createElement("button");
  const fieldset = doc.createElement("fieldset");
  const input = doc.createElement("input");
  const object = doc.createElement("object");
  const output = doc.createElement("output");
  const select = doc.createElement("select");
  const option = doc.createElement("option");
  const textarea = doc.createElement("textarea");
  select.append(option);
  doc.body.append(button, fieldset, input, object, output, select, textarea);

  accessor(HTMLButtonElement.prototype, "value", true);
  absent(HTMLButtonElement.prototype, "defaultValue");
  accessor(HTMLInputElement.prototype, "value", true);
  accessor(HTMLInputElement.prototype, "defaultValue", true);
  accessor(HTMLTextAreaElement.prototype, "value", true);
  accessor(HTMLTextAreaElement.prototype, "defaultValue", true);
  accessor(HTMLOutputElement.prototype, "value", true);
  accessor(HTMLOutputElement.prototype, "defaultValue", true);
  accessor(HTMLSelectElement.prototype, "value", true);
  absent(HTMLSelectElement.prototype, "defaultValue");
  accessor(HTMLOptionElement.prototype, "value", true);
  accessor(HTMLOptionElement.prototype, "text", true);
  accessor(HTMLOptionElement.prototype, "defaultSelected", true);
  accessor(HTMLOptionElement.prototype, "disabled", true);
  accessor(HTMLOptionElement.prototype, "form", false);
  accessor(HTMLOptionElement.prototype, "index", false);
  absent(HTMLOptionElement.prototype, "name");
  accessor(HTMLOptionElement.prototype, "selected", true);
  accessor(HTMLSelectElement.prototype, "disabled", true);
  accessor(HTMLSelectElement.prototype, "multiple", true);
  accessor(HTMLSelectElement.prototype, "required", true);
  accessor(HTMLSelectElement.prototype, "size", true);
  absent(HTMLFieldSetElement.prototype, "value");
  absent(HTMLFieldSetElement.prototype, "defaultValue");
  absent(HTMLObjectElement.prototype, "value");
  absent(HTMLObjectElement.prototype, "defaultValue");

  for (const [element, names] of [
    [button, ["value"]],
    [input, ["value", "defaultValue"]],
    [textarea, ["value", "defaultValue"]],
    [output, ["value", "defaultValue"]],
    [select, ["value", "disabled", "multiple", "required", "size"]],
    [option, ["value", "text", "defaultSelected", "disabled", "form", "index", "selected"]]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own`);
    }
  }
  for (const element of [fieldset, object]) {
    assert(!("value" in element), `${element.localName}.value should not exist`);
    assert(!("defaultValue" in element), `${element.localName}.defaultValue should not exist`);
    assert(!own(element, "value"), `${element.localName}.value should not be own`);
    assert(!own(element, "defaultValue"), `${element.localName}.defaultValue should not be own`);
  }

  button.value = "go";
  input.value = "typed";
  input.defaultValue = "seed";
  textarea.value = "body";
  textarea.defaultValue = "default body";
  output.value = "shown";
  output.defaultValue = "fallback";
  option.value = "choice";
  option.text = "Choice";
  option.defaultSelected = true;
  option.disabled = true;
  option.selected = true;
  select.disabled = true;
  select.multiple = true;
  select.required = true;
  select.size = 4;
  select.value = "choice";

  assert(button.value === "go", "button value");
  assert(input.value === "typed", "input value");
  assert(input.defaultValue === "seed", "input defaultValue");
  assert(textarea.value === "body", "textarea value");
  assert(textarea.defaultValue === "default body", "textarea defaultValue");
  assert(output.value === "shown", "output value");
  assert(output.defaultValue === "fallback", "output defaultValue");
  assert(option.value === "choice", "option value");
  assert(option.text === "Choice", "option text");
  assert(option.defaultSelected === true && option.hasAttribute("selected"), "option defaultSelected");
  assert(option.disabled === true && option.hasAttribute("disabled"), "option disabled");
  assert(option.form === null, "option form");
  assert(option.index === 0, "option index");
  assert(option.selected === true, "option selected");
  assert(select.value === "choice", "select value");
  assert(select.disabled === true && select.hasAttribute("disabled"), "select disabled");
  assert(select.multiple === true && select.hasAttribute("multiple"), "select multiple");
  assert(select.required === true && select.hasAttribute("required"), "select required");
  assert(select.size === 4, "select size");

  for (const [element, names] of [
    [button, ["value"]],
    [input, ["value", "defaultValue"]],
    [textarea, ["value", "defaultValue"]],
    [output, ["value", "defaultValue"]],
    [select, ["value", "disabled", "multiple", "required", "size"]],
    [option, ["value", "text", "defaultSelected", "disabled", "form", "index", "selected"]]
  ]) {
    for (const name of names) {
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }
  assert(button.value === "go", "button value after delete");
  assert(input.value === "typed", "input value after delete");
  assert(input.defaultValue === "seed", "input defaultValue after delete");
  assert(textarea.value === "body", "textarea value after delete");
  assert(textarea.defaultValue === "default body", "textarea defaultValue after delete");
  assert(output.value === "shown", "output value after delete");
  assert(output.defaultValue === "fallback", "output defaultValue after delete");
  assert(option.value === "choice", "option value after delete");
  assert(option.text === "Choice", "option text after delete");
  assert(option.defaultSelected === true, "option defaultSelected after delete");
  assert(option.disabled === true, "option disabled after delete");
  assert(option.form === null, "option form after delete");
  assert(option.index === 0, "option index after delete");
  assert(option.selected === true, "option selected after delete");
  assert(select.value === "choice", "select value after delete");
  assert(select.disabled === true, "select disabled after delete");
  assert(select.multiple === true, "select multiple after delete");
  assert(select.required === true, "select required after delete");
  assert(select.size === 4, "select size after delete");

  return "ok";
})()
"##,
        )
        .expect("detached form control prototype values should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_select_element_members_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-select-receiver-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const makeOption = (id, value) => {
    const option = doc.createElement("option");
    option.id = id;
    option.value = value;
    option.text = value;
    return option;
  };

  const select = doc.createElement("select");
  const first = makeOption("first", "a");
  const second = makeOption("second", "b");
  select.append(first, second);
  doc.body.append(select);

  const input = doc.createElement("input");
  const option = doc.createElement("option");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input, option];

  const cases = [
    ["disabled", true, value => value === true],
    ["multiple", true, value => value === true],
    ["required", true, value => value === true],
    ["size", 3, value => value === 3],
    ["length", 2, value => value === 2],
    ["selectedIndex", 0, value => value === 0],
    ["value", "a", value => value === "a"]
  ];
  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(select, value);
    assert(check(descriptor.get.call(select)), `${name} valid receiver`);
    assert(!own(select, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }

  for (const [name, check] of [
    ["options", value => value.length === 2 && value[0] === first],
    ["selectedOptions", value => value.length === 1 && value[0] === first]
  ]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} readonly`);
    assert(check(descriptor.get.call(select)), `${name} valid receiver`);
    assert(!own(select, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
  }

  const third = makeOption("third", "c");
  const methods = [
    ["item", [0], value => value?.id === "first" && value?.value === "a"],
    ["namedItem", ["first"], value => value?.id === "first" && value?.value === "a"],
    ["add", [third], value => value === undefined && select.length === 3],
    ["remove", [2], value => value === undefined && select.length === 2]
  ];
  for (const [name, args, check] of methods) {
    const method = HTMLSelectElement.prototype[name];
    assert(typeof method === "function", `${name} method`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} method receiver`);
    }
    assert(check(method.call(select, ...args)), `${name} valid receiver`);
    assert(!own(select, name), `${name} should stay inherited`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached select receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_option_element_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-option-receiver-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const form = doc.createElement("form");
  const select = doc.createElement("select");
  const option = doc.createElement("option");
  select.append(option);
  form.append(select);
  doc.body.append(form);

  const input = doc.createElement("input");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input, select];
  const cases = [
    ["value", "choice", value => value === "choice"],
    ["text", "Choice", value => value === "Choice"],
    ["defaultSelected", true, value => value === true],
    ["disabled", true, value => value === true],
    ["label", "Label", value => value === "Label"],
    ["selected", true, value => value === true]
  ];
  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLOptionElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(option, value);
    assert(check(descriptor.get.call(option)), `${name} valid receiver`);
    assert(!own(option, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }

  for (const [name, check] of [
    ["form", value => value === form],
    ["index", value => value === 0]
  ]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLOptionElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} readonly`);
    assert(check(descriptor.get.call(option)), `${name} valid receiver`);
    assert(!own(option, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached option receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_input_element_accessors_use_owner_prototype() {
    let mut vm =
        new_storage_test_vm("https://detached-input-accessor-prototypes.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `HTMLInputElement.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const names = [
    ["accept", true],
    ["alt", true],
    ["defaultChecked", true],
    ["defaultValue", true],
    ["disabled", true],
    ["dirName", true],
    ["files", true],
    ["formAction", true],
    ["formEnctype", true],
    ["formMethod", true],
    ["formNoValidate", true],
    ["formTarget", true],
    ["height", true],
    ["list", false],
    ["maxLength", true],
    ["max", true],
    ["minLength", true],
    ["min", true],
    ["multiple", true],
    ["pattern", true],
    ["placeholder", true],
    ["readOnly", true],
    ["required", true],
    ["size", true],
    ["src", true],
    ["step", true],
    ["type", true],
    ["valueAsDate", true],
    ["valueAsNumber", true],
    ["value", true],
    ["width", true],
    ["checked", true],
    ["indeterminate", true]
  ];
  for (const [name, hasSetter] of names) {
    accessor(HTMLInputElement.prototype, name, hasSetter);
  }

  const input = doc.createElement("input");
  const datalist = doc.createElement("datalist");
  datalist.id = "choices";
  doc.body.append(input, datalist);
  input.setAttribute("list", "choices");
  for (const [name] of names) {
    assert(!own(input, name), `${name} should not be own before set`);
  }

  input.accept = "image/png";
  input.alt = "preview";
  input.defaultChecked = true;
  input.defaultValue = "seed";
  input.disabled = true;
  input.dirName = "field.dir";
  input.formAction = "/submit";
  input.formEnctype = "multipart/form-data";
  input.formMethod = "post";
  input.formNoValidate = true;
  input.formTarget = "frame";
  input.height = 12;
  input.maxLength = 10;
  input.max = "9";
  input.minLength = 2;
  input.min = "1";
  input.multiple = true;
  input.pattern = "[a-z]+";
  input.placeholder = "hint";
  input.readOnly = true;
  input.required = true;
  input.size = 7;
  input.src = "/button.png";
  input.step = "2";
  input.type = "number";
  input.value = "4";
  input.valueAsNumber = 6.5;
  input.width = 20;
  input.checked = true;
  input.indeterminate = true;

  assert(input.accept === "image/png", "accept");
  assert(input.alt === "preview", "alt");
  assert(input.defaultChecked === true && input.hasAttribute("checked"), "defaultChecked");
  assert(input.defaultValue === "seed", "defaultValue");
  assert(input.disabled === true && input.hasAttribute("disabled"), "disabled");
  assert(input.dirName === "field.dir", "dirName");
  assert(input.files === null, "files on non-file input");
  assert(input.getAttribute("formaction") === "/submit", "formAction attribute");
  assert(typeof input.formAction === "string" && input.formAction.length > 0, "formAction getter");
  assert(input.formEnctype === "multipart/form-data", "formEnctype");
  assert(input.formMethod === "post", "formMethod");
  assert(input.formNoValidate === true && input.hasAttribute("formnovalidate"), "formNoValidate");
  assert(input.formTarget === "frame", "formTarget");
  assert(input.height === 12, "height");
  assert(input.list === datalist, "list");
  assert(input.maxLength === 10, "maxLength");
  assert(input.max === "9", "max");
  assert(input.minLength === 2, "minLength");
  assert(input.min === "1", "min");
  assert(input.multiple === true && input.hasAttribute("multiple"), "multiple");
  assert(input.pattern === "[a-z]+", "pattern");
  assert(input.placeholder === "hint", "placeholder");
  assert(input.readOnly === true && input.hasAttribute("readonly"), "readOnly");
  assert(input.required === true && input.hasAttribute("required"), "required");
  assert(input.size === 7, "size");
  assert(input.getAttribute("src") === "/button.png", "src attribute");
  assert(typeof input.src === "string" && input.src.length > 0, "src getter");
  assert(input.step === "2", "step");
  assert(input.type === "number", "type number");
  assert(input.value === "6.5", "valueAsNumber writes value");
  assert(input.valueAsNumber === 6.5, "valueAsNumber");
  assert(input.width === 20, "width");
  assert(input.checked === true, "checked");
  assert(input.indeterminate === true, "indeterminate");

  input.type = "date";
  input.valueAsDate = new Date(Date.UTC(2020, 0, 2));
  assert(input.type === "date", "type date");
  assert(input.value === "2020-01-02", "valueAsDate writes value");
  assert(input.valueAsDate instanceof Date, "valueAsDate getter");

  const fileInput = doc.createElement("input");
  fileInput.type = "file";
  doc.body.append(fileInput);
  assert(!own(fileInput, "files"), "file input files should not be own");
  assert(fileInput.files !== null, "file input files getter");

  for (const [name] of names) {
    assert(!own(input, name), `${name} should not be own after set`);
    assert(delete input[name], `delete ${name}`);
    assert(!own(input, name), `${name} should stay inherited`);
  }
  assert(input.accept === "image/png", "accept after delete");
  assert(input.defaultValue === "seed", "defaultValue after delete");
  assert(input.disabled === true, "disabled after delete");
  assert(input.list === datalist, "list after delete");
  assert(input.value === "2020-01-02", "value after delete");
  assert(input.width === 20, "width after delete");
  assert(input.checked === true, "checked after delete");
  assert(input.indeterminate === true, "indeterminate after delete");
  return "ok";
})()
"##,
        )
        .expect("detached input owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn document_forwarded_reflections_use_html_targets() {
    let mut vm = new_storage_test_vm("https://document-forwarded-reflections.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const names = ["dir", "fgColor", "linkColor", "vlinkColor", "alinkColor", "bgColor"];
  const colorAttributes = {
    fgColor: "text",
    linkColor: "link",
    vlinkColor: "vlink",
    alinkColor: "alink",
    bgColor: "bgcolor"
  };
  const descriptors = new Map();
  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    descriptors.set(name, descriptor);
  }

  if (!document.documentElement) {
    const html = document.createElement("html");
    html.append(document.createElement("head"), document.createElement("body"));
    document.append(html);
  }
  const detachedDocument = document.implementation.createHTMLDocument("");
  for (const [doc, label] of [[document, "live"], [detachedDocument, "detached"]]) {
    for (const name of names) {
      assert(!own(doc, name), `${label}.${name} should not be own before set`);
      assert(doc[name] === "", `${label}.${name} missing-value default`);
    }

    doc.documentElement.setAttribute("dir", "RTL");
    assert(doc.dir === "rtl", `${label}.dir canonical getter`);
    doc.dir = { toString: () => "AUTO" };
    assert(doc.documentElement.getAttribute("dir") === "AUTO", `${label}.dir target attribute`);
    assert(doc.dir === "auto", `${label}.dir setter canonical getter`);
    doc.dir = null;
    assert(doc.documentElement.getAttribute("dir") === "null", `${label}.dir null attribute`);
    assert(doc.dir === "", `${label}.dir null canonical getter`);

    for (const [name, attribute] of Object.entries(colorAttributes)) {
      doc[name] = { toString: () => `${label}-${name}` };
      assert(doc[name] === `${label}-${name}`, `${label}.${name} getter`);
      assert(doc.body.getAttribute(attribute) === `${label}-${name}`, `${label}.${name} target attribute`);
      doc[name] = null;
      assert(doc[name] === "", `${label}.${name} null getter`);
      assert(doc.body.getAttribute(attribute) === "", `${label}.${name} null attribute`);
    }

    for (const name of names) {
      assert(!own(doc, name), `${label}.${name} should stay inherited after set`);
      assert(delete doc[name], `${label}.${name} delete`);
      assert(!own(doc, name), `${label}.${name} should stay inherited after delete`);
    }
  }

  const xmlDocument = document.implementation.createDocument("urn:test", "root", null);
  let converted = false;
  xmlDocument.dir = { toString() { converted = true; return "rtl"; } };
  assert(converted, "XML document setter should still convert the value");
  assert(xmlDocument.dir === "", "XML document dir getter");
  assert(!xmlDocument.documentElement.hasAttribute("dir"), "XML document should not forward dir");
  xmlDocument.fgColor = "red";
  assert(xmlDocument.fgColor === "", "XML document color getter");

  const framesetDocument = document.implementation.createHTMLDocument("");
  const frameset = framesetDocument.createElement("frameset");
  frameset.setAttribute("text", "seed");
  framesetDocument.body = frameset;
  assert(framesetDocument.fgColor === "seed", "frameset color getter target");
  framesetDocument.fgColor = "changed";
  assert(frameset.getAttribute("text") === "seed", "frameset color setter should be a no-op");

  for (const [name, descriptor] of descriptors) {
    for (const receiver of [document.documentElement, {}]) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, "wrong")), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("document forwarded reflections should evaluate");

    assert_eq!(result, "ok");
}
