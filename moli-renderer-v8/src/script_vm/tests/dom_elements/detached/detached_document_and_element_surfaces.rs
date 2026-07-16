use super::*;

#[test]
fn detached_document_shallow_clones_are_empty_and_accept_a_new_root() {
    let mut vm = new_storage_test_vm("https://document-shallow-clone.test/");
    let fixture = include_str!("../../../../../tests/fixtures/document-shallow-clone.js");
    let result = vm
        .eval(&format!("JSON.stringify({fixture})"))
        .expect("Document shallow clone probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["checks"], 273);
}

#[test]
fn detached_domparser_parses_noscript_with_scripting_disabled() {
    let mut vm = new_storage_test_vm("https://detached-domparser-noscript.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    "<body><noscript><span id='fallback'></span></noscript></body>",
    "text/html"
  );
  const reparsed = doc.createElement("div");
  reparsed.innerHTML =
    "<noscript><span id='fragment-fallback'></span></noscript>";
  doc.body.appendChild(reparsed);

  const created = document.implementation.createHTMLDocument("");
  created.body.innerHTML =
    "<noscript><span id='created-fallback'></span></noscript>";

  const raw = doc.createElement("noscript");
  raw.textContent = "<em>fallback&</em>";

  return [
    doc.getElementById("fallback") !== null,
    doc.querySelector("noscript")?.children.length,
    doc.getElementById("fragment-fallback") !== null,
    created.getElementById("created-fallback") !== null,
    raw.innerHTML,
    raw.getHTML(),
    raw.outerHTML
  ].join("|");
})()
"#,
        )
        .expect("detached DOMParser noscript probe should evaluate");

    assert_eq!(
        result,
        concat!(
            "true|1|true|true|",
            "&lt;em&gt;fallback&amp;&lt;/em&gt;|",
            "&lt;em&gt;fallback&amp;&lt;/em&gt;|",
            "<noscript>&lt;em&gt;fallback&amp;&lt;/em&gt;</noscript>"
        )
    );
}

#[test]
fn template_contents_serialize_noscript_with_the_inert_node_document() {
    let mut vm = new_storage_test_vm("https://template-noscript-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const template = document.createElement("template");
  const noscript = document.createElement("noscript");
  noscript.textContent = "<em>fallback&</em>";
  template.content.append(noscript);

  return [
    template.innerHTML,
    template.outerHTML,
    template.getHTML(),
    noscript.innerHTML,
    noscript.outerHTML,
    noscript.getHTML(),
    noscript.ownerDocument === template.content.ownerDocument,
    noscript.ownerDocument === document
  ].join("|");
})()
"#,
        )
        .expect("template noscript serialization probe should evaluate");

    assert_eq!(
        result,
        concat!(
            "<noscript>&lt;em&gt;fallback&amp;&lt;/em&gt;</noscript>|",
            "<template><noscript>&lt;em&gt;fallback&amp;&lt;/em&gt;</noscript></template>|",
            "<noscript>&lt;em&gt;fallback&amp;&lt;/em&gt;</noscript>|",
            "&lt;em&gt;fallback&amp;&lt;/em&gt;|",
            "<noscript>&lt;em&gt;fallback&amp;&lt;/em&gt;</noscript>|",
            "&lt;em&gt;fallback&amp;&lt;/em&gt;|true|false"
        )
    );
}

#[test]
fn detached_document_write_preserves_existing_noscript_text() {
    let mut vm = new_storage_test_vm("https://detached-write-noscript-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  doc.open();
  const noscript = doc.createElement("noscript");
  noscript.textContent = "<em>fallback&</em>";
  doc.body.append(noscript);
  doc.write("<span>tail</span>");
  return [
    doc.body.innerHTML,
    noscript.innerHTML,
    noscript.firstElementChild === null,
    doc.body.lastElementChild?.localName
  ].join("|");
})()
"#,
        )
        .expect("detached document.write noscript probe should evaluate");

    assert_eq!(
        result,
        concat!(
            "<noscript>&lt;em&gt;fallback&amp;&lt;/em&gt;</noscript><span>tail</span>|",
            "&lt;em&gt;fallback&amp;&lt;/em&gt;|true|span"
        )
    );
}

#[test]
fn obsolete_document_and_window_event_methods_are_branded_noops() {
    let mut vm = new_storage_test_vm("https://obsolete-noop-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.implementation.createHTMLDocument("");
  const xml = document.implementation.createDocument("urn:test", "root");
  const marker = document.createElement("p");
  const markerParent = document.body || document.documentElement || document;
  markerParent.append(marker);
  const error = callback => {
    try {
      callback();
      return "none";
    } catch (exception) {
      return exception.name;
    }
  };
  const shape = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.writable,
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(":");
  };

  const values = [];
  for (const name of ["clear", "captureEvents", "releaseEvents"]) {
    values.push(shape(Document.prototype, name));
    values.push(String(Document.prototype[name].call(document)));
    values.push(String(Document.prototype[name].call(html, { ignored: true })));
    values.push(String(Document.prototype[name].call(xml)));
    values.push(error(() => Document.prototype[name].call({})));
  }
  for (const name of ["captureEvents", "releaseEvents"]) {
    values.push(shape(window, name));
    values.push(String(window[name].call(window, { ignored: true })));
    values.push(error(() => window[name].call({})));
    values.push(error(() => window[name].call(document)));
    const forged = document.createElement("div");
    Object.setPrototypeOf(forged, Window.prototype);
    values.push(error(() => window[name].call(forged)));
  }
  values.push(String(marker.isConnected), String(markerParent.contains(marker)));
  return values.join("|");
})()
"#,
        )
        .expect("obsolete Document and Window no-op methods should evaluate");

    assert_eq!(
        result,
        "function:clear:0:true:true:true|undefined|undefined|undefined|TypeError|function:captureEvents:0:true:true:true|undefined|undefined|undefined|TypeError|function:releaseEvents:0:true:true:true|undefined|undefined|undefined|TypeError|function:captureEvents:0:true:true:true|undefined|TypeError|TypeError|TypeError|function:releaseEvents:0:true:true:true|undefined|TypeError|TypeError|TypeError|true|true"
    );
}

#[test]
fn detached_domparser_query_and_element_collections_use_native_handles() {
    let mut vm = new_storage_test_vm("https://detached-domparser-query.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = new DOMParser().parseFromString(
    "<html><body><section id='host'><p class='a'></p><p class='a b'></p><span class='b'></span></section></body></html>",
    "text/html"
  );
  const host = doc.getElementById("host");
  const byQuery = doc.querySelector("section");
  const paragraphs = host && host.getElementsByTagName("P");
  const allDescendants = host && host.getElementsByTagName("*");
  const classMatch = host && host.getElementsByClassName("a b");
  return JSON.stringify({
    host: !!host,
    queryIsHost: byQuery === host,
    paragraphsType: Object.prototype.toString.call(paragraphs),
    paragraphsLength: paragraphs && paragraphs.length,
    allLength: allDescendants && allDescendants.length,
    classType: Object.prototype.toString.call(classMatch),
    classLength: classMatch && classMatch.length,
    classIsSecond: !!classMatch && classMatch[0] === host.childNodes[1]
  });
})()
"##,
        )
        .expect("detached DOMParser query and collection probe should evaluate");

    assert_eq!(
        result,
        r#"{"host":true,"queryIsHost":true,"paragraphsType":"[object HTMLCollection]","paragraphsLength":2,"allLength":3,"classType":"[object HTMLCollection]","classLength":1,"classIsSecond":true}"#
    );
}

#[test]
fn adopted_xml_cdata_uses_html_fragment_serialization() {
    let mut vm = new_storage_test_vm("https://adopted-xml-cdata.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parsed = new DOMParser().parseFromString(
    '<svg xmlns="http://www.w3.org/2000/svg"><![CDATA[<img>]]></svg>',
    'application/xml'
  );
  const element = parsed.documentElement;
  const before = element.outerHTML;
  const adopted = document.adoptNode(element);
  return JSON.stringify({
    before,
    after: adopted.outerHTML,
    declarationValue: adopted.getAttributeNS(
      'http://www.w3.org/2000/xmlns/',
      'xmlns'
    ),
    cdataNodeType: adopted.firstChild && adopted.firstChild.nodeType
  });
})()
"#,
        )
        .expect("adopted XML CDATA serialization probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":"<svg xmlns=\"http://www.w3.org/2000/svg\"><![CDATA[<img>]]></svg>","after":"<svg xmlns=\"http://www.w3.org/2000/svg\">&lt;img&gt;</svg>","declarationValue":"http://www.w3.org/2000/svg","cdataNodeType":4}"#
    );
}

#[test]
fn detached_query_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-query-brand-check.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = new DOMParser().parseFromString(
    "<html><body><section id='host'><p id='target' class='a b'></p></section></body></html>",
    "text/html"
  );
  const host = doc.getElementById("host");
  const target = doc.getElementById("target");
  const docGet = Document.prototype.getElementById.call(doc, "target");
  const closest = Element.prototype.closest.call(target, "section");
  const query = Element.prototype.querySelector.call(host, ".a.b");
  const all = Element.prototype.querySelectorAll.call(host, ".a");
  const byTag = Element.prototype.getElementsByTagName.call(host, "p");
  const byClass = Element.prototype.getElementsByClassName.call(host, "a b");
  return JSON.stringify({
    docGetSame: docGet === target,
    closestSame: closest === host,
    querySame: query === target,
    allType: Object.prototype.toString.call(all),
    allSame: all.length === 1 && all[0] === target,
    tagType: Object.prototype.toString.call(byTag),
    tagSame: byTag.length === 1 && byTag[0] === target,
    classType: Object.prototype.toString.call(byClass),
    classSame: byClass.length === 1 && byClass[0] === target,
    docGetOwn: Object.prototype.hasOwnProperty.call(doc, "getElementById"),
    closestOwn: Object.prototype.hasOwnProperty.call(target, "closest")
  });
})()
"##,
        )
        .expect("detached query prototype brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"docGetSame":true,"closestSame":true,"querySame":true,"allType":"[object NodeList]","allSame":true,"tagType":"[object HTMLCollection]","tagSame":true,"classType":"[object HTMLCollection]","classSame":true,"docGetOwn":false,"closestOwn":false}"#
    );
}

#[test]
fn detached_insert_adjacent_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-insert-adjacent-brand-check.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("div");
  const before = doc.createElement("b");
  const child = doc.createElement("span");
  const after = doc.createElement("i");
  doc.body.appendChild(host);
  function probe(callback) {
    try {
      const value = callback();
      return value === null ? "null" : value === undefined ? "undefined" : String(value);
    } catch (error) {
      return error && error.name;
    }
  }
  const ownerName = (object, name) => {
    let current = object;
    while (current) {
      if (Object.prototype.hasOwnProperty.call(current, name)) {
        if (current === Element.prototype) return "Element";
        if (current === HTMLElement.prototype) return "HTMLElement";
        return current.constructor && current.constructor.name;
      }
      current = Object.getPrototypeOf(current);
    }
    return "missing";
  };
  const methodShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    return [
      !!descriptor,
      typeof descriptor?.value,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  const adjacentNames = [
    "insertAdjacentElement",
    "insertAdjacentText",
    "insertAdjacentHTML"
  ];
  const returned = Element.prototype.insertAdjacentElement.call(host, "beforeend", child);
  Element.prototype.insertAdjacentText.call(host, "afterbegin", "text");
  Element.prototype.insertAdjacentHTML.call(host, "beforeend", "<em id='html'>html</em>");
  Element.prototype.insertAdjacentElement.call(host, "beforebegin", before);
  Element.prototype.insertAdjacentElement.call(host, "afterend", after);
  const html = doc.getElementById("html");
  return JSON.stringify({
    returnedSame: returned === child,
    order: Array.from(doc.body.childNodes).map(node => node.localName).join(","),
    hostText: host.firstChild.data,
    hostChildren: Array.from(host.childNodes).map(node => node.nodeType === 3 ? "#text" : node.localName).join(","),
    htmlSame: html === host.lastElementChild,
    elementOwn: Object.prototype.hasOwnProperty.call(host, "insertAdjacentElement"),
    textOwn: Object.prototype.hasOwnProperty.call(host, "insertAdjacentText"),
    htmlOwn: Object.prototype.hasOwnProperty.call(host, "insertAdjacentHTML"),
    owners: adjacentNames.map(name => ownerName(host, name)).join(","),
    shapes: adjacentNames.map(methodShape).join("|"),
    looseBefore: probe(() => Element.prototype.insertAdjacentElement.call(doc.createElement("p"), "beforebegin", doc.createElement("u"))),
    documentBefore: probe(() => Element.prototype.insertAdjacentElement.call(doc.documentElement, "beforebegin", doc.createElement("u"))),
    invalidPosition: probe(() => Element.prototype.insertAdjacentText.call(host, "sideways", "x")),
    invalidNode: probe(() => Element.prototype.insertAdjacentElement.call(host, "beforeend", doc.doctype))
  });
})()
"##,
        )
        .expect("detached insertAdjacent prototype brand checks should evaluate");

    assert_eq!(
        result,
        r##"{"returnedSame":true,"order":"b,div,i","hostText":"text","hostChildren":"#text,span,em","htmlSame":true,"elementOwn":false,"textOwn":false,"htmlOwn":false,"owners":"Element,Element,Element","shapes":"true:function:2:true:true:true|true:function:2:true:true:true|true:function:2:true:true:true","looseBefore":"null","documentBefore":"HierarchyRequestError","invalidPosition":"SyntaxError","invalidNode":"TypeError"}"##
    );
}

#[test]
fn detached_attribute_node_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-attr-node-brand-check.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const el = doc.createElement("div");
  const attr = doc.createAttribute("data-x");
  const nsAttr = doc.createAttributeNS("urn:test", "lm:flag");
  const ownerName = (object, name) => {
    let current = object;
    while (current) {
      if (Object.prototype.hasOwnProperty.call(current, name)) {
        if (current === Element.prototype) return "Element";
        if (current === HTMLElement.prototype) return "HTMLElement";
        return current.constructor && current.constructor.name;
      }
      current = Object.getPrototypeOf(current);
    }
    return "missing";
  };
  const methodShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    return [
      !!descriptor,
      typeof descriptor?.value,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  attr.value = "one";
  nsAttr.value = "two";
  const first = Element.prototype.setAttributeNode.call(el, attr);
  const second = Element.prototype.setAttributeNodeNS.call(el, nsAttr);
  const byName = Element.prototype.getAttributeNode.call(el, "data-x");
  const byNs = Element.prototype.getAttributeNodeNS.call(el, "urn:test", "flag");
  const removed = Element.prototype.removeAttributeNode.call(el, attr);
  return JSON.stringify({
    firstNull: first === null,
    secondNull: second === null,
    byNameSame: byName === attr,
    byNsSame: byNs === nsAttr,
    removedSame: removed === attr,
    missingAfterRemove: Element.prototype.getAttributeNode.call(el, "data-x") === null,
    nsStillPresent: Element.prototype.getAttributeNodeNS.call(el, "urn:test", "flag") === nsAttr,
    getOwn: Object.prototype.hasOwnProperty.call(el, "getAttributeNode"),
    setOwn: Object.prototype.hasOwnProperty.call(el, "setAttributeNode"),
    removeOwn: Object.prototype.hasOwnProperty.call(el, "removeAttributeNode"),
    owners: [
      "getAttributeNode",
      "getAttributeNodeNS",
      "setAttributeNode",
      "setAttributeNodeNS",
      "removeAttributeNode"
    ].map(name => ownerName(el, name)).join(","),
    shapes: [
      "getAttributeNode",
      "getAttributeNodeNS",
      "setAttributeNode",
      "setAttributeNodeNS",
      "removeAttributeNode"
    ].map(methodShape).join("|")
  });
})()
"#,
        )
        .expect("detached attribute node prototype brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"firstNull":true,"secondNull":true,"byNameSame":true,"byNsSame":true,"removedSame":true,"missingAfterRemove":true,"nsStillPresent":true,"getOwn":false,"setOwn":false,"removeOwn":false,"owners":"Element,Element,Element,Element,Element","shapes":"true:function:1:true:true:true|true:function:2:true:true:true|true:function:1:true:true:true|true:function:1:true:true:true|true:function:1:true:true:true"}"#
    );
}

#[test]
fn detached_interaction_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-interaction-brand-check.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("section");
  const button = doc.createElement("button");
  host.append(button);
  doc.body.append(host);

  const root = Element.prototype.attachShadow.call(host, { mode: "open" });
  const rect = Element.prototype.getBoundingClientRect.call(host);
  const rects = Element.prototype.getClientRects.call(host);
  const ownerName = (object, name) => {
    let current = object;
    while (current) {
      if (Object.prototype.hasOwnProperty.call(current, name)) {
        if (current === Element.prototype) return "Element";
        if (current === HTMLElement.prototype) return "HTMLElement";
        return current.constructor && current.constructor.name;
      }
      current = Object.getPrototypeOf(current);
    }
    return "missing";
  };
  const methodShape = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      !!descriptor,
      typeof descriptor?.value,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  const elementMethodNames = [
    "attachShadow",
    "getBoundingClientRect",
    "getClientRects",
    "querySelector",
    "querySelectorAll",
    "getElementsByTagName",
    "getElementsByTagNameNS",
    "getElementsByClassName"
  ];

  const events = [];
  button.addEventListener("focus", () => events.push("focus"));
  button.addEventListener("blur", () => events.push("blur"));
  button.addEventListener("click", () => events.push("click"));
  HTMLElement.prototype.focus.call(button);
  const activeAfterFocus = doc.activeElement === button;
  HTMLElement.prototype.click.call(button);
  HTMLElement.prototype.blur.call(button);
  const activeAfterBlur = doc.activeElement === null;

  return JSON.stringify({
    rootType: Object.prototype.toString.call(root),
    rootSame: root === host.shadowRoot,
    rectType: Object.prototype.toString.call(rect),
    rectWidthType: typeof rect.width,
    rectsType: Object.prototype.toString.call(rects),
    activeAfterFocus,
    activeAfterBlur,
    events: events.join(","),
    attachOwn: Object.prototype.hasOwnProperty.call(host, "attachShadow"),
    boundingOwn: Object.prototype.hasOwnProperty.call(host, "getBoundingClientRect"),
    rectsOwn: Object.prototype.hasOwnProperty.call(host, "getClientRects"),
    focusOwn: Object.prototype.hasOwnProperty.call(button, "focus"),
    blurOwn: Object.prototype.hasOwnProperty.call(button, "blur"),
    clickOwn: Object.prototype.hasOwnProperty.call(button, "click"),
    elementOwners: elementMethodNames.map(name => ownerName(host, name)).join(","),
    elementShapes: elementMethodNames.map(name => methodShape(Element.prototype, name)).join("|"),
    actionOwners: ["focus", "blur", "click"].map(name => ownerName(button, name)).join(","),
    actionShapes: ["focus", "blur", "click"].map(name => methodShape(HTMLElement.prototype, name)).join("|")
  });
})()
"#,
        )
        .expect("detached interaction prototype brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"rootType":"[object ShadowRoot]","rootSame":true,"rectType":"[object DOMRect]","rectWidthType":"number","rectsType":"[object DOMRectList]","activeAfterFocus":true,"activeAfterBlur":true,"events":"focus,click,blur","attachOwn":false,"boundingOwn":false,"rectsOwn":false,"focusOwn":false,"blurOwn":false,"clickOwn":false,"elementOwners":"Element,Element,Element,Element,Element,Element,Element,Element","elementShapes":"true:function:1:true:true:true|true:function:0:true:true:true|true:function:0:true:true:true|true:function:1:true:true:true|true:function:1:true:true:true|true:function:1:true:true:true|true:function:2:true:true:true|true:function:1:true:true:true","actionOwners":"HTMLElement,HTMLElement,HTMLElement","actionShapes":"true:function:0:true:true:true|true:function:0:true:true:true|true:function:0:true:true:true"}"#
    );
}

#[test]
fn detached_scroll_offsets_use_element_prototype_accessors_and_methods() {
    let mut vm = new_storage_test_vm("https://detached-scroll-offsets.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };
  const method = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor.value;
  };

  const scrollTop = accessor("scrollTop");
  const scrollLeft = accessor("scrollLeft");
  const scroll = method("scroll");
  const scrollTo = method("scrollTo");
  const scrollBy = method("scrollBy");

  const htmlDoc = document.implementation.createHTMLDocument("");
  const parserDoc = new DOMParser().parseFromString("<html><body><section></section></body></html>", "text/html");
  const htmlDiv = htmlDoc.createElement("div");
  const parsedSection = parserDoc.querySelector("section");
  htmlDoc.body.append(htmlDiv);

  for (const element of [htmlDiv, parsedSection]) {
    assert(!own(element, "scrollTop"), "scrollTop should not be own initially");
    assert(!own(element, "scrollLeft"), "scrollLeft should not be own initially");
    assert(!own(element, "scroll"), "scroll should not be own initially");
    assert(!own(element, "scrollTo"), "scrollTo should not be own initially");
    assert(!own(element, "scrollBy"), "scrollBy should not be own initially");

    scrollTop.set.call(element, 12);
    scrollLeft.set.call(element, 7);
    assert(scrollTop.get.call(element) === 0, "detached scrollTop remains zero");
    assert(scrollLeft.get.call(element) === 0, "detached scrollLeft remains zero");

    scrollTo.call(element, { left: 10 });
    assert(element.scrollLeft === 0, "detached scrollTo is a no-op");
    assert(element.scrollTop === 0, "detached scrollTo preserves zero top");

    scrollBy.call(element, { left: 5, top: 7 });
    assert(element.scrollLeft === 0, "detached scrollBy is a no-op");
    assert(element.scrollTop === 0, "detached scrollBy preserves zero top");

    scroll.call(element, -3, 4);
    assert(element.scrollLeft === 0, "scroll clamps negative left");
    assert(element.scrollTop === 0, "detached scroll positional top remains zero");

    element.scrollLeft = 23;
    element.scrollTop = 31;
    assert(element.scrollLeft === 0, "detached direct scrollLeft is a no-op");
    assert(element.scrollTop === 0, "detached direct scrollTop is a no-op");

    assert(delete element.scrollLeft, "delete inherited scrollLeft");
    assert(delete element.scrollTop, "delete inherited scrollTop");
    assert(!own(element, "scrollLeft"), "scrollLeft should stay inherited");
    assert(!own(element, "scrollTop"), "scrollTop should stay inherited");
    assert(element.scrollLeft === 0, "detached scrollLeft after delete");
    assert(element.scrollTop === 0, "detached scrollTop after delete");
  }
  return "ok";
})()
"#,
        )
        .expect("detached scroll offset prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_specialized_method_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-specialized-method-brand-check.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const form = doc.createElement("form");
  const input = doc.createElement("input");
  const submitter = doc.createElement("button");
  const textarea = doc.createElement("textarea");
  const audio = doc.createElement("audio");
  const mediaEvents = [];
  const formEvents = [];

  input.name = "q";
  input.type = "number";
  input.value = "2";
  submitter.type = "submit";
  textarea.value = "abcdef";
  form.append(input, submitter, textarea);
  doc.body.append(form, audio);
  form.addEventListener("submit", event => {
    event.preventDefault();
    formEvents.push(event.submitter === submitter);
  });
  audio.addEventListener("play", () => mediaEvents.push("play"));
  audio.addEventListener("pause", () => mediaEvents.push("pause"));
  audio.addEventListener("emptied", () => mediaEvents.push("emptied"));

  HTMLInputElement.prototype.stepUp.call(input);
  const afterStepUp = input.value;
  HTMLInputElement.prototype.stepDown.call(input, 2);
  const afterStepDown = input.value;
  HTMLInputElement.prototype.showPicker.call(input);

  HTMLInputElement.prototype.setCustomValidity.call(input, "bad");
  const invalidCustom = HTMLInputElement.prototype.checkValidity.call(input) === false;
  const invalidForm = HTMLFormElement.prototype.checkValidity.call(form) === false;
  HTMLInputElement.prototype.setCustomValidity.call(input, "");
  const validControl = HTMLInputElement.prototype.reportValidity.call(input) === true;
  const validForm = HTMLFormElement.prototype.reportValidity.call(form) === true;
  HTMLFormElement.prototype.requestSubmit.call(form, submitter);

  HTMLTextAreaElement.prototype.setSelectionRange.call(textarea, 1, 4);
  HTMLTextAreaElement.prototype.setRangeText.call(textarea, "XY", 2, 5, "select");
  HTMLTextAreaElement.prototype.select.call(textarea);

  HTMLMediaElement.prototype.play.call(audio);
  const pausedAfterPlay = audio.paused;
  HTMLMediaElement.prototype.pause.call(audio);
  const pausedAfterPause = audio.paused;
  HTMLMediaElement.prototype.load.call(audio);

  return JSON.stringify({
    afterStepUp,
    afterStepDown,
    invalidCustom,
    invalidForm,
    validControl,
    validForm,
    submitEvents: formEvents.join(","),
    textareaValue: textarea.value,
    textareaSelection: [textarea.selectionStart, textarea.selectionEnd].join(","),
    pausedAfterPlay,
    pausedAfterPause,
    mediaEvents: mediaEvents.join(","),
    inputOwn: ["stepUp", "stepDown", "showPicker", "checkValidity", "reportValidity", "setCustomValidity"]
      .some(name => Object.prototype.hasOwnProperty.call(input, name)),
    textareaOwn: ["setSelectionRange", "setRangeText", "select"]
      .some(name => Object.prototype.hasOwnProperty.call(textarea, name)),
    formOwn: ["requestSubmit", "checkValidity", "reportValidity"]
      .some(name => Object.prototype.hasOwnProperty.call(form, name)),
    mediaOwn: ["play", "pause", "load"]
      .some(name => Object.prototype.hasOwnProperty.call(audio, name))
  });
})()
"#,
        )
        .expect("detached specialized method prototype brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"afterStepUp":"3","afterStepDown":"1","invalidCustom":true,"invalidForm":true,"validControl":true,"validForm":true,"submitEvents":"true","textareaValue":"abXYf","textareaSelection":"0,5","pausedAfterPlay":false,"pausedAfterPause":true,"mediaEvents":"play,pause,emptied","inputOwn":false,"textareaOwn":false,"formOwn":false,"mediaOwn":false}"#
    );
}

#[test]
fn detached_element_attribute_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://detached-attribute-webidl.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><div></div></body></html>', 'text/html');
  const el = doc.querySelector('div');
  function probe(callback) {
    try {
      return callback();
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  const ownerName = (object, name) => {
    let current = object;
    while (current) {
      if (Object.prototype.hasOwnProperty.call(current, name)) {
        if (current === Element.prototype) return "Element";
        if (current === HTMLElement.prototype) return "HTMLElement";
        return current.constructor && current.constructor.name;
      }
      current = Object.getPrototypeOf(current);
    }
    return "missing";
  };
  const methodShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    return [
      !!descriptor,
      typeof descriptor?.value,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  const names = [
    "getAttribute",
    "getAttributeNS",
    "getAttributeNames",
    "hasAttribute",
    "hasAttributeNS",
    "setAttribute",
    "setAttributeNS",
    "removeAttribute",
    "removeAttributeNS"
  ];
  el.setAttribute(null, undefined);
  const attrNode = el.getAttributeNode({ toString() { return "null"; } });
  const beforeRemove = [
    el.getAttribute("null"),
    attrNode && attrNode.value,
    el.hasAttribute("null"),
    probe(() => el.getAttribute()),
    probe(() => el.getAttributeNode()),
    probe(() => el.setAttribute("x", Symbol())),
    probe(() => el.hasAttribute(Symbol())),
    probe(() => el.getAttributeNode(Symbol()))
  ].join("|");
  el.removeAttribute(null);
  return [
    beforeRemove,
    el.hasAttribute("null"),
    names.map(name => ownerName(el, name)).join(","),
    names.map(methodShape).join("|")
  ].join("|");
})()
"##,
        )
        .expect("detached Element attribute WebIDL args should evaluate");

    assert_eq!(
        result,
        "undefined|undefined|true|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|false|Element,Element,Element,Element,Element,Element,Element,Element,Element|true:function:1:true:true:true|true:function:2:true:true:true|true:function:0:true:true:true|true:function:1:true:true:true|true:function:2:true:true:true|true:function:2:true:true:true|true:function:3:true:true:true|true:function:1:true:true:true|true:function:2:true:true:true"
    );
}

#[test]
fn detached_element_get_attribute_preserves_empty_string_values() {
    let mut vm = new_storage_test_vm("https://detached-empty-attribute.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const createdDocument = document.implementation.createHTMLDocument("");
  const created = createdDocument.createElement("input");
  created.setAttribute("required", "");
  created.setAttribute("data-empty", "");

  const parsedDocument = new DOMParser().parseFromString(
    "<html><body><input required data-empty=''></body></html>",
    "text/html"
  );
  const parsed = parsedDocument.querySelector("input");

  const summarize = element => [
    element.hasAttribute("required"),
    element.getAttribute("required") === "",
    element.hasAttribute("data-empty"),
    element.getAttribute("data-empty") === "",
    element.getAttribute("missing") === null
  ].join(":");

  return [
    summarize(created),
    summarize(parsed)
  ].join("|");
})()
"##,
        )
        .expect("detached empty attribute probe should evaluate");

    assert_eq!(result, "true:true:true:true:true|true:true:true:true:true");
}

#[test]
fn detached_element_attribute_name_validation_matches_chromium() {
    let mut vm = new_storage_test_vm("https://detached-attribute-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><div></div></body></html>', 'text/html');
  const allowed = [
    "@slotchange$lit$",
    ".ariahidden$lit$",
    "?inert$lit$",
    "1name",
    "invalid^Name",
    "\\",
    "'",
    "\"",
    "~",
    "<",
    "\u0001"
  ];
  const invalid = ["", "name\u0000", "has space", "name>", "name/name", "name="];
  function probe(callback) {
    try {
      const value = callback();
      return value === undefined ? "undefined" : String(value);
    } catch (error) {
      return "throw:" + error.name;
    }
  }
  const setAllowed = allowed.every(name => {
    const el = doc.createElement("div");
    return probe(() => el.setAttribute(name, "v")) === "undefined" &&
      el.hasAttribute(name) &&
      el.getAttribute(name) === "v";
  });
  const createAllowed = allowed.every(name =>
    probe(() => doc.createAttribute(name).name.length === name.length) === "true"
  );
  const nsAllowed = [
    "@slotchange$lit$",
    "1name",
    "a:0",
    "0:a",
    "a:b:c"
  ].every(name => {
    const el = doc.createElement("div");
    return probe(() => el.setAttributeNS("urn:test", name, "v")) === "undefined";
  });
  const invalidSet = invalid.map(name =>
    probe(() => doc.createElement("div").setAttribute(name, "v"))
  ).join(",");
  const invalidCreate = invalid.map(name =>
    probe(() => doc.createAttribute(name))
  ).join(",");
  const invalidRemove = invalid.map(name => {
    const el = doc.createElement("div");
    el.setAttribute("data-ok", "1");
    return probe(() => el.removeAttribute(name)) + ":" + el.getAttribute("data-ok");
  }).join(",");
  return [
    setAllowed,
    createAllowed,
    nsAllowed,
    invalidSet,
    invalidCreate,
    invalidRemove
  ].join("|");
})()
"#,
        )
        .expect("detached attribute name validation should evaluate");

    assert_eq!(
        result,
        "true|true|true|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|undefined:1,undefined:1,undefined:1,undefined:1,undefined:1,undefined:1"
    );
}

#[test]
fn detached_element_outer_text_setter_throws() {
    let mut vm = new_storage_test_vm("https://detached-outer-text.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const node = document.createElement("span");
  try {
    node.outerText = "";
    return "no-throw";
  } catch (error) {
    return [
      error.name,
      error.code,
      error instanceof DOMException
    ].join("|");
  }
})()
"##,
        )
        .expect("detached outerText setter should evaluate");

    assert_eq!(result, "NoModificationAllowedError|7|true");
}

#[test]
fn detached_inner_outer_text_use_html_element_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-inner-outer-text-prototype.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const inner = accessor("innerText");
  const outer = accessor("outerText");
  const doc = new DOMParser().parseFromString(
    "<!doctype html><html><body><section id='read'>Alpha <span>Beta</span></section><p id='replace'><span>Old</span></p></body></html>",
    "text/html"
  );
  const read = doc.querySelector("#read");
  const replace = doc.querySelector("#replace span");

  assert(!own(read, "innerText"), "innerText should not be own before use");
  assert(!own(replace, "outerText"), "outerText should not be own before use");
  assert(inner.get.call(read) === "Alpha Beta", "innerText getter");
  assert(outer.get.call(replace) === "Old", "outerText getter");
  inner.set.call(read, "Line one\nLine two");
  assert(read.innerHTML === "Line one<br>Line two", "innerText setter fragment");
  assert(read.textContent === "Line oneLine two", "innerText setter text content");
  assert(read.querySelector("br").ownerDocument === doc, "innerText setter owner document");
  outer.set.call(replace, "Done");
  assert(doc.querySelector("#replace").textContent === "Done", "outerText setter");
  assert(!own(read, "innerText"), "innerText should not be own after set");
  assert(!own(doc.querySelector("#replace").firstChild, "outerText"), "outerText should not be own after set");
  return "ok";
})()
"##,
        )
        .expect("detached innerText/outerText prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_serialization_uses_element_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-html-serialization-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><h1>Title</h1><p><strong>Body</strong></p></body></html>',
    'text/html'
  );
  const body = doc.body;
  const h1 = doc.querySelector('h1');
  const inner = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML');
  const outer = Object.getOwnPropertyDescriptor(Element.prototype, 'outerHTML');
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  const before = [
    !!inner,
    typeof inner.get,
    typeof inner.set,
    !!outer,
    typeof outer.get,
    typeof outer.set,
    own(body, 'innerHTML'),
    own(h1, 'outerHTML'),
    inner.get.call(body),
    inner.get.call(h1)
  ].join('|');

  inner.set.call(body, '<template><span>inside</span></template><section id="next">Next</section>');
  const template = body.firstElementChild;
  const section = body.lastElementChild;
  outer.set.call(section, '<article id="done"><em>Done</em></article>');
  const article = body.lastElementChild;

  return [
    before,
    body.innerHTML,
    template.innerHTML,
    template.content.firstElementChild.localName,
    article.localName,
    article.innerHTML,
    own(body, 'innerHTML'),
    own(article, 'outerHTML')
  ].join('||');
})()
"#,
        )
        .expect("detached HTML serialization prototype accessors should evaluate");

    assert_eq!(
        result,
        "true|function|function|true|function|function|false|false|<h1>Title</h1><p><strong>Body</strong></p>|Title||<template><span>inside</span></template><article id=\"done\"><em>Done</em></article>||<span>inside</span>||span||article||<em>Done</em>||false||false"
    );
}

#[test]
fn script_inner_html_uses_element_prototype_accessor() {
    let mut vm = new_storage_test_vm("https://script-inner-html-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const inner = Object.getOwnPropertyDescriptor(Element.prototype, "innerHTML");
  assert(!!inner, "Element.innerHTML descriptor");
  assert(typeof inner.get === "function", "Element.innerHTML getter");
  assert(typeof inner.set === "function", "Element.innerHTML setter");
  assert(!own(HTMLScriptElement.prototype, "innerHTML"), "script prototype should inherit innerHTML");

  const parsed = new DOMParser().parseFromString(
    "<html><body><script>old</script></body></html>",
    "text/html"
  );
  const scripts = [
    [document.createElement("script"), "live"],
    [parsed.querySelector("script"), "detached"]
  ];
  for (const [script, name] of scripts) {
    assert(!own(script, "innerHTML"), `${name} script innerHTML should not be own before set`);
    script.innerHTML = "<b>&amp;</b>";
    assert(!own(script, "innerHTML"), `${name} script innerHTML should not be own after set`);
    assert(script.innerHTML === "<b>&amp;</b>", `${name} script innerHTML value`);
    assert(script.text === "<b>&amp;</b>", `${name} script text value`);
    assert(script.textContent === "<b>&amp;</b>", `${name} script textContent value`);
    assert(script.childNodes.length === 1, `${name} script child count`);
    assert(script.firstChild.nodeType === Node.TEXT_NODE, `${name} script text child`);
    assert(script.firstChild.data === "<b>&amp;</b>", `${name} script text data`);
    assert(delete script.innerHTML, `${name} script innerHTML delete`);
    assert(!own(script, "innerHTML"), `${name} script innerHTML should not be own after delete`);
    assert(script.innerHTML === "<b>&amp;</b>", `${name} script innerHTML after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("script innerHTML prototype accessor should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn script_standard_accessors_use_html_script_element_prototype() {
    let mut vm = new_storage_test_vm("https://script-standard-prototype.test/base/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const names = [
    "src",
    "charset",
    "type",
    "async",
    "text",
    "defer",
    "noModule",
    "integrity",
    "event",
    "htmlFor"
  ];
  for (const name of names) {
    accessor(HTMLScriptElement.prototype, name);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }
  for (const prototype of [HTMLElement.prototype, SVGElement.prototype, MathMLElement.prototype]) {
    accessor(prototype, "nonce");
  }
  assert(!own(HTMLScriptElement.prototype, "nonce"), "nonce should be inherited from HTMLElement");

  const parsed = new DOMParser().parseFromString(
    "<html><body><script>old</script></body></html>",
    "text/html"
  );
  const scripts = [
    [document.createElement("script"), "live"],
    [parsed.querySelector("script"), "detached"]
  ];
  for (const [script, label] of scripts) {
    for (const name of names) {
      assert(!own(script, name), `${label}.${name} should not be own before set`);
    }
    assert(!own(script, "nonce"), `${label}.nonce should not be own before set`);
    script.src = "assets/app.js";
    script.nonce = "nonce-value";
    script.charset = "utf-8";
    script.type = "module";
    script.async = false;
    script.text = "console.log('<ok>')";
    script.defer = true;
    script.noModule = true;
    script.integrity = "sha256-test";
    script.event = "load";
    script.htmlFor = "window";
    for (const name of names) {
      assert(!own(script, name), `${label}.${name} should not be own after set`);
    }
    assert(!own(script, "nonce"), `${label}.nonce should not be own after set`);
    assert(script.src === "https://script-standard-prototype.test/base/assets/app.js", `${label}.src`);
    assert(script.getAttribute("src") === "assets/app.js", `${label}.src attribute`);
    assert(script.nonce === "nonce-value", `${label}.nonce`);
    assert(script.charset === "utf-8", `${label}.charset`);
    assert(script.type === "module", `${label}.type`);
    assert(script.async === false, `${label}.async`);
    assert(script.text === "console.log('<ok>')", `${label}.text`);
    assert(script.textContent === "console.log('<ok>')", `${label}.textContent`);
    assert(script.defer === true && script.hasAttribute("defer"), `${label}.defer`);
    assert(script.noModule === true && script.hasAttribute("nomodule"), `${label}.noModule`);
    assert(script.integrity === "sha256-test", `${label}.integrity`);
    assert(script.event === "load", `${label}.event`);
    assert(script.htmlFor === "window" && script.getAttribute("for") === "window", `${label}.htmlFor`);
    for (const name of names) {
      assert(delete script[name], `${label}.${name} delete`);
      assert(!own(script, name), `${label}.${name} should not be own after delete`);
    }
    assert(script.type === "module", `${label}.type after delete`);
    assert(script.text === "console.log('<ok>')", `${label}.text after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("script standard prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn nonce_mixin_preserves_hidden_values_for_html_svg_and_mathml_elements() {
    let mut vm = new_storage_test_vm("https://nonce-mixin.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const cases = [
    ["http://www.w3.org/1999/xhtml", "div", HTMLElement.prototype],
    ["http://www.w3.org/2000/svg", "g", SVGElement.prototype],
    ["http://www.w3.org/1998/Math/MathML", "mrow", MathMLElement.prototype]
  ];
  for (const [namespace, name, prototype] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "nonce");
    if (!descriptor || typeof descriptor.get !== "function" || typeof descriptor.set !== "function") {
      throw new Error(`${name}: nonce descriptor`);
    }

    const reflected = document.createElementNS(namespace, name);
    if (reflected.nonce !== "" || reflected.getAttribute("nonce") !== null) {
      throw new Error(`${name}: initial nonce`);
    }
    reflected.setAttribute("nonce", "content-secret");
    if (reflected.nonce !== "content-secret" || reflected.getAttribute("nonce") !== "content-secret") {
      throw new Error(`${name}: reflected nonce`);
    }
    if (reflected.cloneNode().nonce !== "content-secret") {
      throw new Error(`${name}: pre-insertion clone nonce`);
    }
    body.appendChild(reflected);
    if (reflected.nonce !== "content-secret" || reflected.getAttribute("nonce") !== "") {
      throw new Error(`${name}: hidden nonce`);
    }
    if (reflected.cloneNode().nonce !== "content-secret") {
      throw new Error(`${name}: hidden clone nonce`);
    }
    reflected.remove();

    const internal = document.createElementNS(namespace, name);
    internal.nonce = "idl-secret";
    if (internal.nonce !== "idl-secret" || internal.getAttribute("nonce") !== null) {
      throw new Error(`${name}: IDL nonce`);
    }
    body.appendChild(internal);
    if (internal.nonce !== "idl-secret" || internal.getAttribute("nonce") !== null) {
      throw new Error(`${name}: inserted IDL nonce`);
    }
    internal.remove();
  }
  return "ok";
})()
"#,
        )
        .expect("nonce mixin probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn referrer_policy_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://referrer-policy-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
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
    ["a", HTMLAnchorElement.prototype],
    ["area", HTMLAreaElement.prototype],
    ["img", HTMLImageElement.prototype],
    ["iframe", HTMLIFrameElement.prototype],
    ["link", HTMLLinkElement.prototype],
    ["script", HTMLScriptElement.prototype]
  ];
  for (const [, prototype] of cases) {
    accessor(prototype, "referrerPolicy");
  }
  assert(!own(HTMLElement.prototype, "referrerPolicy"), "HTMLElement should not own referrerPolicy");
  assert(!("referrerPolicy" in document.createElement("div")), "div should not expose referrerPolicy");

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const [tag] of cases) {
    const live = document.createElement(tag);
    const detached = detachedDoc.createElement(tag);
    for (const [element, label] of [[live, "live"], [detached, "detached"]]) {
      assert(!own(element, "referrerPolicy"), `${label}.${tag} referrerPolicy should not be own before set`);
      assert(element.referrerPolicy === "", `${label}.${tag} default referrerPolicy`);
      element.referrerPolicy = "origin";
      assert(!own(element, "referrerPolicy"), `${label}.${tag} referrerPolicy should not be own after set`);
      assert(element.referrerPolicy === "origin", `${label}.${tag} origin referrerPolicy`);
      assert(element.getAttribute("referrerpolicy") === "origin", `${label}.${tag} attr after origin`);
      element.referrerPolicy = "not-a-policy";
      assert(element.referrerPolicy === "", `${label}.${tag} invalid referrerPolicy canonicalizes`);
      assert(element.getAttribute("referrerpolicy") === "not-a-policy", `${label}.${tag} invalid attr is reflected`);
      assert(delete element.referrerPolicy, `${label}.${tag} delete referrerPolicy`);
      assert(!own(element, "referrerPolicy"), `${label}.${tag} referrerPolicy should stay inherited`);
      assert(element.referrerPolicy === "", `${label}.${tag} referrerPolicy after delete`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("referrerPolicy prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn cross_origin_and_loading_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://cross-origin-loading-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
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

  accessor(HTMLImageElement.prototype, "crossOrigin");
  accessor(HTMLLinkElement.prototype, "crossOrigin");
  accessor(HTMLMediaElement.prototype, "crossOrigin");
  accessor(HTMLScriptElement.prototype, "crossOrigin");
  accessor(HTMLImageElement.prototype, "loading");
  accessor(HTMLIFrameElement.prototype, "loading");
  accessor(HTMLMediaElement.prototype, "loading");
  assert(!own(HTMLElement.prototype, "crossOrigin"), "HTMLElement should not own crossOrigin");
  assert(!own(HTMLElement.prototype, "loading"), "HTMLElement should not own loading");
  assert(!own(HTMLAudioElement.prototype, "crossOrigin"), "audio prototype should inherit crossOrigin");
  assert(!own(HTMLVideoElement.prototype, "loading"), "video prototype should inherit loading");
  assert(!("crossOrigin" in document.createElement("div")), "div should not expose crossOrigin");
  assert(!("loading" in document.createElement("div")), "div should not expose loading");

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const tag of ["img", "link", "script", "audio", "video"]) {
    for (const [element, label] of [
      [document.createElement(tag), "live"],
      [detachedDoc.createElement(tag), "detached"]
    ]) {
      assert(!own(element, "crossOrigin"), `${label}.${tag} crossOrigin should not be own before set`);
      assert(element.crossOrigin === null, `${label}.${tag} default crossOrigin`);
      element.crossOrigin = "use-credentials";
      assert(!own(element, "crossOrigin"), `${label}.${tag} crossOrigin should not be own after set`);
      assert(element.crossOrigin === "use-credentials", `${label}.${tag} use-credentials crossOrigin`);
      assert(element.getAttribute("crossorigin") === "use-credentials", `${label}.${tag} crossOrigin attr`);
      element.crossOrigin = undefined;
      assert(element.getAttribute("crossorigin") === null, `${label}.${tag} undefined crossOrigin attr`);
      assert(element.crossOrigin === null, `${label}.${tag} crossOrigin after undefined`);
      element.setAttribute("crossorigin", "invalid");
      assert(element.crossOrigin === "anonymous", `${label}.${tag} invalid crossOrigin canonicalizes`);
      assert(delete element.crossOrigin, `${label}.${tag} delete crossOrigin`);
      assert(!own(element, "crossOrigin"), `${label}.${tag} crossOrigin should stay inherited`);
    }
  }

  for (const tag of ["img", "iframe", "audio", "video"]) {
    for (const [element, label] of [
      [document.createElement(tag), "live"],
      [detachedDoc.createElement(tag), "detached"]
    ]) {
      assert(!own(element, "loading"), `${label}.${tag} loading should not be own before set`);
      assert(element.loading === "eager", `${label}.${tag} default loading`);
      element.loading = "lazy";
      assert(!own(element, "loading"), `${label}.${tag} loading should not be own after set`);
      assert(element.loading === "lazy", `${label}.${tag} lazy loading`);
      assert(element.getAttribute("loading") === "lazy", `${label}.${tag} loading attr`);
      element.loading = "eager";
      assert(element.loading === "eager", `${label}.${tag} eager loading`);
      assert(delete element.loading, `${label}.${tag} delete loading`);
      assert(!own(element, "loading"), `${label}.${tag} loading should stay inherited`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("crossOrigin/loading prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn hyperlink_metadata_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://hyperlink-metadata-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
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

  accessor(HTMLAnchorElement.prototype, "download");
  accessor(HTMLAnchorElement.prototype, "ping");
  accessor(HTMLAnchorElement.prototype, "hreflang");
  accessor(HTMLAreaElement.prototype, "download");
  accessor(HTMLAreaElement.prototype, "ping");
  accessor(HTMLAreaElement.prototype, "hreflang");
  accessor(HTMLAreaElement.prototype, "type");
  accessor(HTMLLinkElement.prototype, "hreflang");
  for (const name of ["download", "ping", "hreflang"]) {
    assert(!own(HTMLElement.prototype, name), `HTMLElement should not own ${name}`);
  }
  const div = document.createElement("div");
  assert(!("download" in div), "div should not expose download");
  assert(!("ping" in div), "div should not expose ping");
  assert(!("hreflang" in div), "div should not expose hreflang");
  assert(!("download" in document.createElement("link")), "link should not expose download");
  assert(!("ping" in document.createElement("link")), "link should not expose ping");
  for (const property of ["hreflang", "type"]) {
    const area = document.createElement("area");
    area.setAttribute(property, "retained attribute");
    assert(!(property in area), `area should not expose ${property}`);
    assert(area.getAttribute(property) === "retained attribute", `${property} remains an attribute`);
  }

  const parsed = new DOMParser().parseFromString(
    "<html><head><link></head><body><a></a><area></area></body></html>",
    "text/html"
  );
  const cases = [
    [document.createElement("a"), parsed.querySelector("a"), ["download", "ping", "hreflang"], "anchor"],
    [document.createElement("area"), parsed.querySelector("area"), ["download", "ping", "hreflang", "type"], "area"],
    [document.createElement("link"), parsed.querySelector("link"), ["hreflang"], "link"]
  ];
  for (const [live, detached, names, label] of cases) {
    for (const element of [live, detached]) {
      for (const name of names) {
        assert(!own(element, name), `${label}.${name} should not be own before set`);
      }
      if (names.includes("download")) {
        element.download = `${label}.txt`;
        assert(!own(element, "download"), `${label}.download should not be own after set`);
        assert(element.download === `${label}.txt`, `${label}.download value`);
        assert(element.getAttribute("download") === `${label}.txt`, `${label}.download attr`);
        assert(delete element.download, `${label}.download delete`);
        assert(!own(element, "download"), `${label}.download should stay inherited`);
        assert(element.download === `${label}.txt`, `${label}.download after delete`);
      }
      if (names.includes("ping")) {
        element.ping = `${label}-ping`;
        assert(!own(element, "ping"), `${label}.ping should not be own after set`);
        assert(element.ping === `${label}-ping`, `${label}.ping value`);
        assert(element.getAttribute("ping") === `${label}-ping`, `${label}.ping attr`);
        element.ping = "bad-\uD800";
        assert(element.getAttribute("ping").charCodeAt(4) === 0xFFFD, `${label}.ping USVString conversion`);
        assert(delete element.ping, `${label}.ping delete`);
        assert(!own(element, "ping"), `${label}.ping should stay inherited`);
      }
      if (names.includes("hreflang")) {
        element.hreflang = `${label}-lang`;
        assert(!own(element, "hreflang"), `${label}.hreflang should not be own after set`);
        assert(element.hreflang === `${label}-lang`, `${label}.hreflang value`);
        assert(element.getAttribute("hreflang") === `${label}-lang`, `${label}.hreflang attr`);
        assert(delete element.hreflang, `${label}.hreflang delete`);
        assert(!own(element, "hreflang"), `${label}.hreflang should stay inherited`);
        assert(element.hreflang === `${label}-lang`, `${label}.hreflang after delete`);
      }
      if (names.includes("type")) {
        element.type = `${label}/type`;
        assert(!own(element, "type"), `${label}.type should not be own after set`);
        assert(element.type === `${label}/type`, `${label}.type value`);
        assert(element.getAttribute("type") === `${label}/type`, `${label}.type attr`);
        assert(delete element.type, `${label}.type delete`);
        assert(!own(element, "type"), `${label}.type should stay inherited`);
        assert(element.type === `${label}/type`, `${label}.type after delete`);
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("hyperlink metadata prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn hyperlink_legacy_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://hyperlink-legacy-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
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

  for (const name of ["coords", "charset", "shape"]) {
    accessor(HTMLAnchorElement.prototype, name);
  }
  for (const name of ["coords", "shape", "noHref"]) {
    accessor(HTMLAreaElement.prototype, name);
  }
  accessor(HTMLLinkElement.prototype, "charset");
  accessor(HTMLScriptElement.prototype, "charset");
  for (const name of ["coords", "charset", "shape", "noHref"]) {
    assert(!own(HTMLElement.prototype, name), `HTMLElement should not own ${name}`);
  }

  const div = document.createElement("div");
  assert(!("coords" in div), "div should not expose coords");
  assert(!("charset" in div), "div should not expose charset");
  assert(!("shape" in div), "div should not expose shape");
  assert(!("noHref" in div), "div should not expose noHref");
  assert(!("charset" in document.createElement("area")), "area should not expose charset");
  assert(!("coords" in document.createElement("link")), "link should not expose coords");
  assert(!("shape" in document.createElement("link")), "link should not expose shape");
  assert(!("noHref" in document.createElement("a")), "anchor should not expose noHref");
  assert(!("coords" in document.createElement("script")), "script should not expose coords");

  const parsed = new DOMParser().parseFromString(
    "<html><head><link><script></script></head><body><a></a><area></area></body></html>",
    "text/html"
  );
  const cases = [
    [document.createElement("a"), parsed.querySelector("a"), ["coords", "charset", "shape"], "anchor"],
    [document.createElement("area"), parsed.querySelector("area"), ["coords", "shape", "noHref"], "area"],
    [document.createElement("link"), parsed.querySelector("link"), ["charset"], "link"],
    [document.createElement("script"), parsed.querySelector("script"), ["charset"], "script"]
  ];

  for (const [live, detached, names, label] of cases) {
    for (const element of [live, detached]) {
      for (const name of names) {
        assert(!own(element, name), `${label}.${name} should not be own before set`);
      }
      if (names.includes("coords")) {
        element.coords = `${label}-coords`;
        assert(!own(element, "coords"), `${label}.coords should not be own after set`);
        assert(element.coords === `${label}-coords`, `${label}.coords value`);
        assert(element.getAttribute("coords") === `${label}-coords`, `${label}.coords attr`);
        assert(delete element.coords, `${label}.coords delete`);
        assert(!own(element, "coords"), `${label}.coords should stay inherited`);
      }
      if (names.includes("charset")) {
        element.charset = `${label}-charset`;
        assert(!own(element, "charset"), `${label}.charset should not be own after set`);
        assert(element.charset === `${label}-charset`, `${label}.charset value`);
        assert(element.getAttribute("charset") === `${label}-charset`, `${label}.charset attr`);
        assert(delete element.charset, `${label}.charset delete`);
        assert(!own(element, "charset"), `${label}.charset should stay inherited`);
      }
      if (names.includes("shape")) {
        element.shape = `${label}-shape`;
        assert(!own(element, "shape"), `${label}.shape should not be own after set`);
        assert(element.shape === `${label}-shape`, `${label}.shape value`);
        assert(element.getAttribute("shape") === `${label}-shape`, `${label}.shape attr`);
        assert(delete element.shape, `${label}.shape delete`);
        assert(!own(element, "shape"), `${label}.shape should stay inherited`);
      }
      if (names.includes("noHref")) {
        assert(element.noHref === false, `${label}.noHref default`);
        element.noHref = true;
        assert(!own(element, "noHref"), `${label}.noHref should not be own after set`);
        assert(element.noHref === true, `${label}.noHref value`);
        assert(element.hasAttribute("nohref"), `${label}.noHref attr`);
        element.noHref = false;
        assert(element.noHref === false, `${label}.noHref false value`);
        assert(!element.hasAttribute("nohref"), `${label}.noHref removed attr`);
        assert(delete element.noHref, `${label}.noHref delete`);
        assert(!own(element, "noHref"), `${label}.noHref should stay inherited`);
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("hyperlink legacy prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn text_legacy_dom_string_reflectors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://text-legacy-dom-string-reflectors.test/");

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
  const cases = [
    [HTMLAnchorElement.prototype, "a", "rev"],
    [HTMLBRElement.prototype, "br", "clear"]
  ];
  const detachedDocument = document.implementation.createHTMLDocument("");

  for (const [prototype, tag, name] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);

    for (const [doc, label] of [[document, "live"], [detachedDocument, "detached"]]) {
      const element = doc.createElement(tag);
      assert(!own(element, name), `${label}.${name} should not be own before set`);
      assert(element[name] === "", `${label}.${name} missing-value default`);
      element[name] = { toString: () => `${name}-value` };
      assert(element[name] === `${name}-value`, `${label}.${name} getter`);
      assert(element.getAttribute(name) === `${name}-value`, `${label}.${name} attribute`);
      assert(!own(element, name), `${label}.${name} should stay inherited after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited after delete`);
      assert(element[name] === `${name}-value`, `${label}.${name} after delete`);
    }

    const wrongTag = tag === "a" ? "br" : "a";
    const wrongElement = document.createElement(wrongTag);
    assert(throwsTypeError(() => descriptor.get.call(wrongElement)), `${name} wrong-element getter`);
    assert(throwsTypeError(() => descriptor.set.call(wrongElement, "wrong")), `${name} wrong-element setter`);
    assert(throwsTypeError(() => descriptor.get.call({})), `${name} object getter`);
    assert(throwsTypeError(() => descriptor.set.call({}, "wrong")), `${name} object setter`);
  }
  return "ok";
})()
"#,
        )
        .expect("text legacy DOMString reflectors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn domparser_html_preserves_quirks_mode_and_parses_with_scripting_disabled() {
    let mut vm = new_storage_test_vm("https://domparser-html-mode.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const quirks = parser.parseFromString(
    "<html><head></head><body></body></html>",
    "text/html"
  );
  const standards = parser.parseFromString(
    "<!doctype html><html><head></head><body></body></html>",
    "text/html"
  );
  const noscript = parser.parseFromString(
    "<body><noscript><p id='first'></p><p id='second'></p></noscript></body>",
    "text/html"
  );
  return JSON.stringify({
    quirks: quirks.compatMode,
    standards: standards.compatMode,
    noscriptChildren: Array.from(noscript.querySelector("noscript").children)
      .map(element => element.id)
  });
})()
"#,
        )
        .expect("DOMParser HTML parse mode probe should evaluate");

    assert_eq!(
        result,
        r#"{"quirks":"BackCompat","standards":"CSS1Compat","noscriptChildren":["first","second"]}"#
    );
}

#[test]
fn domparser_xml_preserves_requested_content_type_for_success_and_error_documents() {
    let mut vm = new_storage_test_vm("https://domparser-xml-content-type.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  return JSON.stringify([
    "text/xml",
    "application/xml",
    "application/xhtml+xml",
    "image/svg+xml"
  ].map(contentType => {
    const valid = parser.parseFromString("<root/>", contentType);
    const invalid = parser.parseFromString(
      '<span x:test="testing">1</span>',
      contentType
    );
    return [
      valid.contentType,
      invalid.contentType,
      invalid.documentElement.localName
    ];
  }));
})()
"#,
        )
        .expect("DOMParser XML content type probe should evaluate");

    assert_eq!(
        result,
        r#"[["text/xml","text/xml","html"],["application/xml","application/xml","html"],["application/xhtml+xml","application/xhtml+xml","html"],["image/svg+xml","image/svg+xml","html"]]"#
    );
}
