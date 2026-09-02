use super::*;

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
    rectsArray: Array.isArray(rects),
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
        r#"{"rootType":"[object ShadowRoot]","rootSame":true,"rectType":"[object DOMRect]","rectWidthType":"number","rectsArray":true,"activeAfterFocus":true,"activeAfterBlur":true,"events":"focus,click,blur","attachOwn":false,"boundingOwn":false,"rectsOwn":false,"focusOwn":false,"blurOwn":false,"clickOwn":false,"elementOwners":"Element,Element,Element,Element,Element,Element,Element,Element","elementShapes":"true:function:1:true:true:true|true:function:0:true:true:true|true:function:0:true:true:true|true:function:1:true:true:true|true:function:1:true:true:true|true:function:1:true:true:true|true:function:2:true:true:true|true:function:1:true:true:true","actionOwners":"HTMLElement,HTMLElement,HTMLElement","actionShapes":"true:function:0:true:true:true|true:function:0:true:true:true|true:function:0:true:true:true"}"#
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
  assert(read.textContent === "Line one\nLine two", "innerText setter");
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

  const parsed = new DOMParser().parseFromString(
    "<html><head><link></head><body><a></a><area></area></body></html>",
    "text/html"
  );
  const cases = [
    [document.createElement("a"), parsed.querySelector("a"), ["download", "ping", "hreflang"], "anchor"],
    [document.createElement("area"), parsed.querySelector("area"), ["download", "ping", "hreflang"], "area"],
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
fn table_cell_legacy_accessors_use_owner_prototype() {
    let mut vm = new_storage_test_vm("https://table-cell-legacy-prototype.test/");

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
  const names = ["headers", "abbr", "axis", "scope", "noWrap"];
  for (const name of names) {
    accessor(HTMLTableCellElement.prototype, name);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }

  for (const [tag, missing] of [
    ["div", names],
    ["table", ["headers", "abbr", "axis", "scope", "noWrap"]],
    ["tr", ["headers", "abbr", "axis", "scope", "noWrap"]],
    ["a", ["headers", "abbr", "axis", "scope", "noWrap"]]
  ]) {
    const element = document.createElement(tag);
    for (const name of missing) {
      assert(!(name in element), `${tag} should not expose ${name}`);
    }
  }

  const parsed = new DOMParser().parseFromString(
    "<html><body><table><tr><td></td><th></th></tr></table></body></html>",
    "text/html"
  );
  const cells = [
    [document.createElement("td"), "live td"],
    [document.createElement("th"), "live th"],
    [parsed.querySelector("td"), "detached td"],
    [parsed.querySelector("th"), "detached th"]
  ];

  for (const [cell, label] of cells) {
    for (const name of names) {
      assert(!own(cell, name), `${label}.${name} should not be own before set`);
    }
    cell.headers = `${label}-headers`;
    cell.abbr = `${label}-abbr`;
    cell.axis = `${label}-axis`;
    cell.scope = "ROWGROUP";
    cell.noWrap = true;
    for (const name of names) {
      assert(!own(cell, name), `${label}.${name} should not be own after set`);
    }
    assert(cell.headers === `${label}-headers`, `${label}.headers value`);
    assert(cell.getAttribute("headers") === `${label}-headers`, `${label}.headers attr`);
    assert(cell.abbr === `${label}-abbr`, `${label}.abbr value`);
    assert(cell.getAttribute("abbr") === `${label}-abbr`, `${label}.abbr attr`);
    assert(cell.axis === `${label}-axis`, `${label}.axis value`);
    assert(cell.getAttribute("axis") === `${label}-axis`, `${label}.axis attr`);
    assert(cell.scope === "rowgroup", `${label}.scope canonical`);
    assert(cell.getAttribute("scope") === "ROWGROUP", `${label}.scope attr`);
    cell.scope = "invalid";
    assert(cell.scope === "", `${label}.scope invalid canonical`);
    assert(cell.getAttribute("scope") === "invalid", `${label}.scope invalid attr`);
    assert(cell.noWrap === true && cell.hasAttribute("nowrap"), `${label}.noWrap true`);
    cell.noWrap = false;
    assert(cell.noWrap === false && !cell.hasAttribute("nowrap"), `${label}.noWrap false`);
    for (const name of names) {
      assert(delete cell[name], `${label}.${name} delete`);
      assert(!own(cell, name), `${label}.${name} should stay inherited`);
    }
    assert(cell.headers === `${label}-headers`, `${label}.headers after delete`);
    assert(cell.scope === "", `${label}.scope after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("table cell legacy prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_table_structural_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-table-structural-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter shape`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  for (const [prototype, name, hasSetter] of [
    [HTMLTableSectionElement.prototype, "rows", false],
    [HTMLTableRowElement.prototype, "rowIndex", false],
    [HTMLTableRowElement.prototype, "sectionRowIndex", false],
    [HTMLTableRowElement.prototype, "cells", false],
    [HTMLTableCellElement.prototype, "colSpan", true],
    [HTMLTableCellElement.prototype, "rowSpan", true],
    [HTMLTableCellElement.prototype, "cellIndex", false]
  ]) {
    accessor(prototype, name, hasSetter);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const [doc, label] of [[document, "live"], [detachedDoc, "detached"]]) {
    const table = doc.createElement("table");
    const tbody = doc.createElement("tbody");
    const firstRow = doc.createElement("tr");
    const secondRow = doc.createElement("tr");
    const firstCell = doc.createElement("td");
    const secondCell = doc.createElement("th");
    table.append(tbody);
    tbody.append(firstRow, secondRow);
    firstRow.append(firstCell);
    secondRow.append(secondCell);

    for (const [element, names, elementLabel] of [
      [tbody, ["rows"], "tbody"],
      [firstRow, ["rowIndex", "sectionRowIndex", "cells"], "firstRow"],
      [secondRow, ["rowIndex", "sectionRowIndex", "cells"], "secondRow"],
      [firstCell, ["colSpan", "rowSpan", "cellIndex"], "firstCell"],
      [secondCell, ["colSpan", "rowSpan", "cellIndex"], "secondCell"]
    ]) {
      for (const name of names) {
        assert(!own(element, name), `${label}.${elementLabel}.${name} should not be own before access`);
      }
    }

    assert(tbody.rows.length === 2, `${label}.rows length`);
    assert(firstRow.rowIndex === 0, `${label}.first rowIndex`);
    assert(firstRow.sectionRowIndex === 0, `${label}.first sectionRowIndex`);
    assert(secondRow.rowIndex === 1, `${label}.second rowIndex`);
    assert(secondRow.sectionRowIndex === 1, `${label}.second sectionRowIndex`);
    assert(firstRow.cells.length === 1, `${label}.first cells`);
    assert(secondRow.cells.length === 1, `${label}.second cells`);
    assert(firstCell.cellIndex === 0, `${label}.first cellIndex`);
    assert(secondCell.cellIndex === 0, `${label}.second cellIndex`);

    firstCell.colSpan = 7;
    firstCell.rowSpan = 0;
    secondCell.colSpan = 2000;
    secondCell.rowSpan = -5;
    assert(firstCell.colSpan === 7 && firstCell.getAttribute("colspan") === "7", `${label}.colSpan`);
    assert(firstCell.rowSpan === 0 && firstCell.getAttribute("rowspan") === "0", `${label}.rowSpan zero`);
    assert(secondCell.colSpan === 1000 && secondCell.getAttribute("colspan") === "1000", `${label}.colSpan clamp`);
    assert(secondCell.rowSpan === 1 && secondCell.getAttribute("rowspan") === "1", `${label}.rowSpan clamp`);

    for (const [element, names, elementLabel] of [
      [tbody, ["rows"], "tbody"],
      [firstRow, ["rowIndex", "sectionRowIndex", "cells"], "firstRow"],
      [secondRow, ["rowIndex", "sectionRowIndex", "cells"], "secondRow"],
      [firstCell, ["colSpan", "rowSpan", "cellIndex"], "firstCell"],
      [secondCell, ["colSpan", "rowSpan", "cellIndex"], "secondCell"]
    ]) {
      for (const name of names) {
        assert(!own(element, name), `${label}.${elementLabel}.${name} should not be own after access`);
        assert(delete element[name], `${label}.${elementLabel}.${name} delete`);
        assert(!own(element, name), `${label}.${elementLabel}.${name} should stay inherited`);
      }
    }
    assert(tbody.rows.length === 2, `${label}.rows after delete`);
    assert(firstRow.cells.length === 1, `${label}.cells after delete`);
    assert(firstCell.colSpan === 7, `${label}.colSpan after delete`);
    assert(firstCell.rowSpan === 0, `${label}.rowSpan after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached table structural owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_table_structural_members_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-table-receiver-brand.test/base/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const doc = document.implementation.createHTMLDocument("");
  const table = doc.createElement("table");
  const caption = doc.createElement("caption");
  const thead = doc.createElement("thead");
  const tfoot = doc.createElement("tfoot");
  const tbody = doc.createElement("tbody");
  const row = doc.createElement("tr");
  const td = doc.createElement("td");
  const th = doc.createElement("th");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  table.append(caption, thead, tbody, tfoot);
  tbody.append(row);
  row.append(td, th);

  const tableBad = [{}, text, div, tbody, row, td, th];
  const sectionBad = [{}, text, div, table, row, td, th];
  const rowBad = [{}, text, div, table, tbody, td, th];
  const cellBad = [{}, text, div, table, tbody, row];

  for (const name of ["caption", "tHead", "tFoot", "rows", "tBodies"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTableElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get.call(table) !== "undefined", `${name} valid getter`);
    if (typeof descriptor.set === "function") {
      descriptor.set.call(table, null);
    }
    for (const receiver of tableBad) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      if (typeof descriptor.set === "function") {
        assert(throwsTypeError(() => descriptor.set.call(receiver, null)), `${name} setter receiver`);
      }
    }
  }

  const tableMethods = {
    createCaption: [],
    deleteCaption: [],
    createTHead: [],
    deleteTHead: [],
    createTFoot: [],
    deleteTFoot: [],
    createTBody: [],
    insertRow: [-1],
    deleteRow: [-1]
  };
  const methodTable = doc.createElement("table");
  for (const [name, args] of Object.entries(tableMethods)) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableElement.prototype, name).value;
    assert(typeof method === "function", `${name} method`);
    method.call(methodTable, ...args);
    for (const receiver of tableBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} method receiver`);
    }
  }

  const sectionRows = Object.getOwnPropertyDescriptor(HTMLTableSectionElement.prototype, "rows");
  assert(sectionRows.get.call(tbody).length === 1, "section rows valid getter");
  for (const receiver of sectionBad) {
    assert(throwsTypeError(() => sectionRows.get.call(receiver)), "section rows receiver");
  }
  for (const [name, args] of [["insertRow", [-1]], ["deleteRow", [-1]]]) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableSectionElement.prototype, name).value;
    const section = doc.createElement("tbody");
    method.call(section, ...args);
    for (const receiver of sectionBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} receiver`);
    }
  }

  for (const name of ["rowIndex", "sectionRowIndex", "cells"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTableRowElement.prototype, name);
    assert(typeof descriptor.get.call(row) !== "undefined", `${name} valid getter`);
    for (const receiver of rowBad) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} receiver`);
    }
  }
  for (const [name, args] of [["insertCell", [-1]], ["deleteCell", [-1]]]) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableRowElement.prototype, name).value;
    const methodRow = doc.createElement("tr");
    method.call(methodRow, ...args);
    for (const receiver of rowBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} receiver`);
    }
  }

  for (const cell of [td, th]) {
    for (const name of ["colSpan", "rowSpan", "cellIndex"]) {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLTableCellElement.prototype, name);
      assert(typeof descriptor.get.call(cell) !== "undefined", `${name} valid getter`);
      if (typeof descriptor.set === "function") {
        descriptor.set.call(cell, 2);
      }
      for (const receiver of cellBad) {
        assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
        if (typeof descriptor.set === "function") {
          assert(throwsTypeError(() => descriptor.set.call(receiver, 2)), `${name} setter receiver`);
        }
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached HTML table structural receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn table_legacy_alignment_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://table-legacy-alignment-prototype.test/");

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

  const tableAlignmentNames = ["ch", "chOff", "vAlign"];
  for (const prototype of [
    HTMLTableSectionElement.prototype,
    HTMLTableRowElement.prototype,
    HTMLTableColElement.prototype,
    HTMLTableCellElement.prototype
  ]) {
    for (const name of tableAlignmentNames) accessor(prototype, name);
  }
  accessor(HTMLTableColElement.prototype, "span");

  for (const name of [...tableAlignmentNames, "span"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }
  for (const tag of ["div", "table", "a"]) {
    const element = document.createElement(tag);
    for (const name of tableAlignmentNames) {
      assert(!(name in element), `${tag} should not expose ${name}`);
    }
    assert(!("span" in element), `${tag} should not expose span`);
  }
  for (const tag of ["thead", "tr", "td"]) {
    assert(!("span" in document.createElement(tag)), `${tag} should not expose span`);
  }

  const parsed = new DOMParser().parseFromString(
    "<html><body><table><colgroup><col></colgroup><thead></thead><tbody><tr><td></td></tr></tbody></table></body></html>",
    "text/html"
  );
  const alignmentCases = [
    [document.createElement("thead"), "live section"],
    [document.createElement("tr"), "live row"],
    [document.createElement("colgroup"), "live colgroup"],
    [document.createElement("col"), "live col"],
    [document.createElement("td"), "live cell"],
    [parsed.querySelector("thead"), "detached section"],
    [parsed.querySelector("tr"), "detached row"],
    [parsed.querySelector("colgroup"), "detached colgroup"],
    [parsed.querySelector("col"), "detached col"],
    [parsed.querySelector("td"), "detached cell"]
  ];

  for (const [element, label] of alignmentCases) {
    for (const name of tableAlignmentNames) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
    element.ch = `${label}-char`;
    element.chOff = `${label}-charoff`;
    element.vAlign = `${label}-valign`;
    for (const name of tableAlignmentNames) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
    }
    assert(element.ch === `${label}-char`, `${label}.ch value`);
    assert(element.getAttribute("char") === `${label}-char`, `${label}.char attr`);
    assert(element.chOff === `${label}-charoff`, `${label}.chOff value`);
    assert(element.getAttribute("charoff") === `${label}-charoff`, `${label}.charoff attr`);
    assert(element.vAlign === `${label}-valign`, `${label}.vAlign value`);
    assert(element.getAttribute("valign") === `${label}-valign`, `${label}.valign attr`);
    for (const name of tableAlignmentNames) {
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
    assert(element.ch === `${label}-char`, `${label}.ch after delete`);
    assert(element.chOff === `${label}-charoff`, `${label}.chOff after delete`);
    assert(element.vAlign === `${label}-valign`, `${label}.vAlign after delete`);
  }

  const colCases = [
    [document.createElement("colgroup"), "live colgroup"],
    [document.createElement("col"), "live col"],
    [parsed.querySelector("colgroup"), "detached colgroup"],
    [parsed.querySelector("col"), "detached col"]
  ];
  for (const [element, label] of colCases) {
    assert(!own(element, "span"), `${label}.span should not be own before set`);
    assert(element.span === 1, `${label}.span default`);
    element.span = 12;
    assert(!own(element, "span"), `${label}.span should not be own after set`);
    assert(element.span === 12, `${label}.span numeric value`);
    assert(element.getAttribute("span") === "12", `${label}.span attr`);
    element.span = 0;
    assert(element.getAttribute("span") === "0", `${label}.span zero attr`);
    assert(element.span === 1, `${label}.span zero canonical`);
    element.span = 1002;
    assert(element.getAttribute("span") === "1002", `${label}.span large attr`);
    assert(element.span === 1000, `${label}.span large canonical`);
    element.setAttribute("span", "invalid");
    assert(element.span === 1, `${label}.span invalid canonical`);
    assert(delete element.span, `${label}.span delete`);
    assert(!own(element, "span"), `${label}.span should stay inherited`);
  }
  return "ok";
})()
"#,
        )
        .expect("table legacy alignment prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn legacy_align_accessor_uses_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://legacy-align-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, label) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "align");
    assert(!!descriptor, `${label}.align descriptor missing`);
    assert(typeof descriptor.get === "function", `${label}.align getter`);
    assert(typeof descriptor.set === "function", `${label}.align setter`);
    assert(descriptor.enumerable === true, `${label}.align enumerable`);
    assert(descriptor.configurable === true, `${label}.align configurable`);
  };

  const ownerPrototypes = [
    [HTMLDivElement.prototype, "HTMLDivElement"],
    [HTMLHeadingElement.prototype, "HTMLHeadingElement"],
    [HTMLParagraphElement.prototype, "HTMLParagraphElement"],
    [HTMLHRElement.prototype, "HTMLHRElement"],
    [HTMLImageElement.prototype, "HTMLImageElement"],
    [HTMLObjectElement.prototype, "HTMLObjectElement"],
    [HTMLIFrameElement.prototype, "HTMLIFrameElement"],
    [HTMLEmbedElement.prototype, "HTMLEmbedElement"],
    [HTMLLegendElement.prototype, "HTMLLegendElement"],
    [HTMLTableCaptionElement.prototype, "HTMLTableCaptionElement"],
    [HTMLTableElement.prototype, "HTMLTableElement"],
    [HTMLTableSectionElement.prototype, "HTMLTableSectionElement"],
    [HTMLTableRowElement.prototype, "HTMLTableRowElement"],
    [HTMLTableColElement.prototype, "HTMLTableColElement"],
    [HTMLTableCellElement.prototype, "HTMLTableCellElement"],
    [HTMLInputElement.prototype, "HTMLInputElement"]
  ];
  for (const [prototype, label] of ownerPrototypes) accessor(prototype, label);

  assert(!own(HTMLElement.prototype, "align"), "align should not be on HTMLElement.prototype");
  for (const tag of ["body", "section", "a", "span", "button"]) {
    const element = document.createElement(tag);
    assert(!("align" in element), `${tag} should not expose align`);
  }

  const parsed = new DOMParser().parseFromString(
    `<!doctype html><html><body>
      <div></div><h1></h1><p></p><hr><img><object></object><iframe></iframe><embed>
      <fieldset><legend></legend></fieldset><input>
      <table><caption></caption><colgroup><col></colgroup><thead></thead><tbody><tr><td></td></tr></tbody></table>
    </body></html>`,
    "text/html"
  );
  const cases = [
    [document.createElement("div"), parsed.querySelector("div"), "div"],
    [document.createElement("h1"), parsed.querySelector("h1"), "h1"],
    [document.createElement("p"), parsed.querySelector("p"), "p"],
    [document.createElement("hr"), parsed.querySelector("hr"), "hr"],
    [document.createElement("img"), parsed.querySelector("img"), "img"],
    [document.createElement("object"), parsed.querySelector("object"), "object"],
    [document.createElement("iframe"), parsed.querySelector("iframe"), "iframe"],
    [document.createElement("embed"), parsed.querySelector("embed"), "embed"],
    [document.createElement("legend"), parsed.querySelector("legend"), "legend"],
    [document.createElement("caption"), parsed.querySelector("caption"), "caption"],
    [document.createElement("table"), parsed.querySelector("table"), "table"],
    [document.createElement("thead"), parsed.querySelector("thead"), "thead"],
    [document.createElement("tr"), parsed.querySelector("tr"), "tr"],
    [document.createElement("colgroup"), parsed.querySelector("colgroup"), "colgroup"],
    [document.createElement("col"), parsed.querySelector("col"), "col"],
    [document.createElement("td"), parsed.querySelector("td"), "td"],
    [document.createElement("input"), parsed.querySelector("input"), "input"]
  ];

  for (const [live, detached, tag] of cases) {
    for (const [element, flavor] of [[live, "live"], [detached, "detached"]]) {
      assert(!own(element, "align"), `${flavor} ${tag}.align should not be own before set`);
      element.align = `${flavor}-${tag}-align`;
      assert(!own(element, "align"), `${flavor} ${tag}.align should not be own after set`);
      assert(element.getAttribute("align") === `${flavor}-${tag}-align`, `${flavor} ${tag}.align attr`);
      assert(element.align === `${flavor}-${tag}-align`, `${flavor} ${tag}.align value`);
      assert(delete element.align, `${flavor} ${tag}.align delete`);
      assert(!own(element, "align"), `${flavor} ${tag}.align should stay inherited`);
      assert(element.align === `${flavor}-${tag}-align`, `${flavor} ${tag}.align after delete`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("legacy align prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_element_names_use_element_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-element-names-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const htmlDoc = document.implementation.createHTMLDocument("");
  const htmlElement = htmlDoc.createElement("div");
  const svgElement = htmlDoc.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  const xmlDoc = document.implementation.createDocument("urn:doc", "root", null);
  const xmlElement = xmlDoc.createElementNS("urn:item", "p:item");
  const parsedDoc = new DOMParser().parseFromString("<html><body><section></section></body></html>", "text/html");
  const parsedElement = parsedDoc.querySelector("section");
  const adoptedElement = xmlDoc.adoptNode(document.createElementNS("urn:live", "q:live"));

  const names = ["tagName", "localName", "namespaceURI", "prefix"];
  const descriptorShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const ownShape = (element) =>
    names.map((name) => `${name}:${Object.prototype.hasOwnProperty.call(element, name)}`).join(",");
  const values = (element) =>
    [element.tagName, element.localName, element.namespaceURI, element.prefix].join(",");

  htmlElement.tagName = "shadow";
  htmlElement.localName = "shadow";
  htmlElement.namespaceURI = "urn:shadow";
  htmlElement.prefix = "shadow";
  const deleteResult = [
    delete htmlElement.tagName,
    delete htmlElement.localName,
    delete htmlElement.namespaceURI,
    delete htmlElement.prefix
  ].join(",");

  return [
    names.map(descriptorShape).join("|"),
    [htmlElement, svgElement, xmlElement, parsedElement, adoptedElement].map(ownShape).join("|"),
    [htmlElement, svgElement, xmlElement, parsedElement, adoptedElement].map(values).join("|"),
    deleteResult,
    ownShape(htmlElement)
  ].join("||");
})()
"#,
        )
        .expect("detached element name prototype accessors should evaluate");

    assert_eq!(
        result,
        "true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true||tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false||DIV,div,http://www.w3.org/1999/xhtml,|svg:g,g,http://www.w3.org/2000/svg,svg|p:item,item,urn:item,p|SECTION,section,http://www.w3.org/1999/xhtml,|q:live,live,urn:live,q||true,true,true,true||tagName:false,localName:false,namespaceURI:false,prefix:false"
    );
}

#[test]
fn detached_element_declared_prototype_members_are_not_public_own_properties() {
    let mut vm = new_storage_test_vm("https://detached-element-own-property-audit.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const declaredPrototypeNames = (object) => {
    const names = new Set();
    for (
      let prototype = Object.getPrototypeOf(object);
      prototype && prototype !== Object.prototype;
      prototype = Object.getPrototypeOf(prototype)
    ) {
      for (const name of Object.getOwnPropertyNames(prototype)) {
        if (name !== "constructor") names.add(name);
      }
    }
    return names;
  };
  const publicOwnNames = (object) =>
    Object.getOwnPropertyNames(object).filter((name) =>
      !name.startsWith("__moli") &&
      !name.startsWith("__lm") &&
      !/^\d+$/.test(name)
    );
  const offendersFor = (object) => {
    const declared = declaredPrototypeNames(object);
    return publicOwnNames(object).filter((name) => declared.has(name)).sort();
  };

  const html = document.implementation.createHTMLDocument("");
  const liveTags = [
    "a", "area", "audio", "base", "br", "button", "canvas", "data",
    "datalist", "details", "dialog", "div", "embed", "fieldset", "font",
    "form", "h1", "hr", "iframe", "img", "input", "label", "legend", "li",
    "link", "map", "marquee", "meta", "meter", "object", "ol", "optgroup",
    "option", "output", "p", "param", "pre", "progress", "q", "script",
    "select", "source", "span", "style", "table", "caption", "colgroup",
    "col", "tbody", "tr", "td", "textarea", "time", "title", "track", "ul",
    "video"
  ];
  const objects = [
    ["detached:html", html.documentElement],
    ["detached:head", html.head],
    ["detached:body", html.body]
  ];
  for (const tag of liveTags) {
    objects.push([`detached:${tag}`, html.createElement(tag)]);
    objects.push([`live:${tag}`, document.createElement(tag)]);
  }

  const svg = html.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  const xml = document.implementation.createDocument("urn:test", "root", null);
  objects.push(["detached:svg:g", svg]);
  objects.push(["detached:xml:p:item", xml.createElementNS("urn:item", "p:item")]);
  objects.push(["detached:xml:plain", xml.createElement("MixedCase")]);

  const offenders = [];
  for (const [label, object] of objects) {
    const names = offendersFor(object);
    if (names.length) offenders.push(`${label}:${names.join(",")}`);
  }

  assert(offenders.length === 0, offenders.join("|"));
  return `ok:${objects.length}`;
})()
"#,
        )
        .expect("detached element own-property audit should evaluate");

    assert_eq!(result, "ok:120");
}

#[test]
fn legacy_dimension_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://legacy-dimensions-prototype.test/");

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
    assert(!!descriptor, `${prototype.constructor.name}.${name} missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const ownerChecks = [
    [HTMLHRElement.prototype, "size"],
    [HTMLHRElement.prototype, "width"],
    [HTMLFontElement.prototype, "size"],
    [HTMLMarqueeElement.prototype, "height"],
    [HTMLMarqueeElement.prototype, "width"],
    [HTMLTableElement.prototype, "width"],
    [HTMLTableColElement.prototype, "width"],
    [HTMLTableCellElement.prototype, "height"],
    [HTMLTableCellElement.prototype, "width"],
    [HTMLIFrameElement.prototype, "height"],
    [HTMLIFrameElement.prototype, "width"],
    [HTMLEmbedElement.prototype, "height"],
    [HTMLEmbedElement.prototype, "width"],
    [HTMLObjectElement.prototype, "height"],
    [HTMLObjectElement.prototype, "width"],
    [HTMLPreElement.prototype, "width"],
    [HTMLImageElement.prototype, "sizes"],
    [HTMLSourceElement.prototype, "sizes"],
    [HTMLSourceElement.prototype, "height"],
    [HTMLSourceElement.prototype, "width"]
  ];
  for (const [prototype, name] of ownerChecks) accessor(prototype, name);
  for (const name of ["size", "height", "width", "sizes"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!own(document.createElement("div"), name), `${name} should not be own on div`);
  }

  const parsed = new DOMParser().parseFromString(
    `<!doctype html><html><body>
      <hr><font></font><marquee></marquee><table><colgroup><col></colgroup><tbody><tr><td></td></tr></tbody></table>
      <iframe></iframe><embed><object></object><pre></pre><img><source>
    </body></html>`,
    "text/html"
  );
  const pairs = [
    [document.createElement("hr"), parsed.querySelector("hr"), [["size", "11", "11"], ["width", "33", "33"]]],
    [document.createElement("font"), parsed.querySelector("font"), [["size", "5", "5"]]],
    [document.createElement("marquee"), parsed.querySelector("marquee"), [["height", "44", "44"], ["width", "55", "55"]]],
    [document.createElement("table"), parsed.querySelector("table"), [["width", "66", "66"]]],
    [document.createElement("col"), parsed.querySelector("col"), [["width", "77", "77"]]],
    [document.createElement("td"), parsed.querySelector("td"), [["height", "88", "88"], ["width", "99", "99"]]],
    [document.createElement("iframe"), parsed.querySelector("iframe"), [["height", "101", "101"], ["width", "102", "102"]]],
    [document.createElement("embed"), parsed.querySelector("embed"), [["height", "103", "103"], ["width", "104", "104"]]],
    [document.createElement("object"), parsed.querySelector("object"), [["height", "105", "105"], ["width", "106", "106"]]],
    [document.createElement("pre"), parsed.querySelector("pre"), [["width", 107, 107]]],
    [document.createElement("img"), parsed.querySelector("img"), [["sizes", "10px", "10px"]]],
    [document.createElement("source"), parsed.querySelector("source"), [["sizes", "20px", "20px"], ["height", 108, 108], ["width", 109, 109]]]
  ];

  for (const [live, detached, checks] of pairs) {
    for (const element of [live, detached]) {
      for (const [name, value, expected] of checks) {
        assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
        element[name] = value;
        assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
        assert(element[name] === expected, `${element.localName}.${name} value`);
        assert(element.getAttribute(name) === String(expected), `${element.localName}.${name} attr`);
        assert(delete element[name], `${element.localName}.${name} delete`);
        assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
        assert(element[name] === expected, `${element.localName}.${name} after delete`);
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("legacy dimension owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_reflected_element_attributes_use_owner_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-reflected-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const div = doc.createElement("div");
  const section = doc.createElement("section");
  const svg = doc.createElementNS("http://www.w3.org/2000/svg", "svg");
  doc.body.append(div, section, svg);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.configurable === true, `${name} configurable`);
    assert(descriptor.writable === true, `${name} writable`);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  accessor(Element.prototype, "id", true);
  accessor(Element.prototype, "className", true);
  accessor(Element.prototype, "innerHTML", true);
  accessor(Element.prototype, "outerHTML", true);
  accessor(Element.prototype, "classList", true);
  accessor(Element.prototype, "part", true);
  accessor(Element.prototype, "attributes", false);
  accessor(Element.prototype, "shadowRoot", false);
  accessor(Element.prototype, "ariaLabel", true);
  accessor(Element.prototype, "ariaControlsElements", true);
  accessor(HTMLElement.prototype, "contentEditable", true);
  accessor(HTMLElement.prototype, "isContentEditable", false);
  method(Element.prototype, "getHTML");
  method(Element.prototype, "setHTMLUnsafe");
  assert(!own(HTMLElement.prototype, "id"), "id duplicated on HTMLElement");
  assert(!own(HTMLDivElement.prototype, "id"), "id duplicated on HTMLDivElement");
  assert(!own(Element.prototype, "contentEditable"), "contentEditable duplicated on Element");

  div.id = "alpha";
  div.className = "one two";
  section.id = "proxy";
  section.className = "proxy-class";
  section.classList.add("extra");
  section.part = "badge primary";
  section.ariaLabel = "Proxy label";
  section.contentEditable = "plaintext-only";
  section.innerHTML = "<b>x</b>";
  section.setAttribute("data-x", "1");
  const root = section.attachShadow({ mode: "open" });
  root.innerHTML = "<u>s</u>";
  svg.id = "svg-id";

  const names = [
    "id",
    "className",
    "innerHTML",
    "outerHTML",
    "getHTML",
    "setHTMLUnsafe",
    "classList",
    "part",
    "attributes",
    "shadowRoot",
    "ariaLabel",
    "ariaControlsElements",
    "contentEditable",
    "isContentEditable"
  ];
  for (const element of [div, section, svg]) {
    for (const name of names) {
      assert(!own(element, name), `${name} should not be own before delete`);
    }
  }
  assert(div.id === "alpha" && div.className === "one two", "div reflected values");
  assert(section.id === "proxy", "proxy id");
  assert(section.className === "proxy-class extra", "proxy className");
  assert(section.classList.value === "proxy-class extra", "proxy classList");
  assert(section.part.value === "badge primary", "proxy part");
  assert(section.attributes.length === 6, "proxy attributes length");
  assert(section.attributes.getNamedItem("data-x").value === "1", "proxy attributes item");
  assert(section.shadowRoot === root, "proxy shadowRoot");
  assert(section.shadowRoot.innerHTML === "<u>s</u>", "proxy shadowRoot content");
  assert(section.ariaLabel === "Proxy label", "proxy ariaLabel");
  assert(section.getAttribute("aria-label") === "Proxy label", "proxy aria-label attribute");
  const controls = [div];
  section.ariaControlsElements = controls;
  assert(section.ariaControlsElements === controls, "proxy ariaControlsElements");
  assert(section.getAttribute("aria-controls") === "", "proxy aria-controls attribute");
  assert(!own(section, "ariaControlsElements"), "ariaControlsElements should stay inherited after set");
  assert(section.contentEditable === "plaintext-only", "proxy contentEditable");
  assert(section.isContentEditable === true, "proxy isContentEditable");
  assert(section.innerHTML === "<b>x</b>", "proxy innerHTML");
  assert(section.getHTML() === "<b>x</b>", "proxy getHTML");
  assert(svg.id === "svg-id", "svg id");
  assert(!("contentEditable" in svg), "SVG should not expose contentEditable");

  assert(delete section.id, "delete id");
  assert(delete section.className, "delete className");
  assert(delete section.contentEditable, "delete contentEditable");
  assert(delete section.innerHTML, "delete innerHTML");
  assert(delete section.getHTML, "delete getHTML");
  assert(delete section.setHTMLUnsafe, "delete setHTMLUnsafe");
  assert(delete section.classList, "delete classList");
  assert(delete section.part, "delete part");
  assert(delete section.attributes, "delete attributes");
  assert(delete section.shadowRoot, "delete shadowRoot");
  assert(delete section.ariaLabel, "delete ariaLabel");
  assert(delete section.ariaControlsElements, "delete ariaControlsElements");
  for (const name of names) {
    assert(!own(section, name), `${name} should not be own after delete`);
  }
  assert(section.id === "proxy", "proxy id after delete");
  assert(section.className === "proxy-class extra", "proxy className after delete");
  assert(section.classList.value === "proxy-class extra", "proxy classList after delete");
  assert(section.part.value === "badge primary", "proxy part after delete");
  assert(section.attributes.getNamedItem("data-x").value === "1", "proxy attributes after delete");
  assert(section.shadowRoot === root, "proxy shadowRoot after delete");
  assert(section.ariaLabel === "Proxy label", "proxy ariaLabel after delete");
  assert(section.ariaControlsElements === controls, "proxy ariaControlsElements after delete");
  assert(section.contentEditable === "plaintext-only", "proxy contentEditable after delete");
  assert(section.isContentEditable === true, "proxy isContentEditable after delete");
  assert(section.getHTML() === "<b>x</b>", "proxy getHTML after delete");
  section.setHTMLUnsafe("<i>y</i>");
  assert(section.innerHTML === "<i>y</i>", "proxy setHTMLUnsafe");
  return "ok";
})()
"#,
        )
        .expect("detached reflected attribute prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn aria_nullable_dom_string_reflection_removes_content_attributes() {
    let mut vm = new_storage_test_vm("https://aria-nullable-reflection.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const root = document.appendChild(document.createElement("html"));
  const connected = root.appendChild(document.createElement("div"));
  const detached = document.implementation.createHTMLDocument("").createElement("div");
  const roleDescriptor = Object.getOwnPropertyDescriptor(Element.prototype, "role");
  assert(typeof roleDescriptor?.get === "function", "role getter");
  assert(typeof roleDescriptor?.set === "function", "role setter");
  assert(roleDescriptor.enumerable && roleDescriptor.configurable, "role descriptor flags");
  const connectedText = connected.appendChild(document.createTextNode("connected"));
  const detachedText = detached.ownerDocument.createTextNode("detached");
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  for (const receiver of [connectedText, detachedText, {}]) {
    assert(throwsTypeError(() => roleDescriptor.get.call(receiver)), "getter checks Element brand");
    assert(
      throwsTypeError(() => roleDescriptor.set.call(receiver, null)),
      "nullable setter checks Element brand"
    );
    assert(
      throwsTypeError(() => roleDescriptor.set.call(receiver, "button")),
      "string setter checks Element brand"
    );
  }

  for (const element of [connected, detached]) {
    assert(element.role === null, "missing role is null");
    assert(element.ariaAtomic === null, "missing ariaAtomic is null");
    element.setAttribute("role", "button");
    element.setAttribute("aria-atomic", "true");
    assert(element.role === "button", "role reads content attribute");
    assert(element.ariaAtomic === "true", "ariaAtomic reads content attribute");

    element.role = { toString() { return "checkbox"; } };
    element.ariaAtomic = 0;
    assert(element.getAttribute("role") === "checkbox", "role uses DOMString conversion");
    assert(element.getAttribute("aria-atomic") === "0", "ariaAtomic uses DOMString conversion");

    element.role = null;
    element.ariaAtomic = undefined;
    assert(element.role === null && !element.hasAttribute("role"), "null removes role");
    assert(
      element.ariaAtomic === null && !element.hasAttribute("aria-atomic"),
      "undefined removes ariaAtomic"
    );
    let symbolError = "";
    try {
      element.role = Symbol("role");
    } catch (error) {
      symbolError = error.name;
    }
    assert(symbolError === "TypeError", "Symbol conversion throws TypeError");
  }
  return "ok";
})()
"#,
        )
        .expect("nullable ARIA DOMString reflection probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_global_html_attributes_use_html_element_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-global-html-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptorFor = (object, name) => {
    for (
      let prototype = Object.getPrototypeOf(object);
      prototype;
      prototype = Object.getPrototypeOf(prototype)
    ) {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
      if (descriptor) return descriptor;
    }
  };
  const accessor = (object, name) => {
    const descriptor = descriptorFor(object, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const html = document.implementation.createHTMLDocument("");
  const parent = html.createElement("section");
  const detached = html.createElement("button");
  const live = document.createElement("button");
  parent.translate = false;
  parent.append(detached);

  const names = [
    "title",
    "lang",
    "autocapitalize",
    "autocorrect",
    "translate",
    "dir",
    "hidden",
    "accessKey",
    "draggable",
    "spellcheck",
    "enterKeyHint",
    "inputMode",
    "autofocus",
    "tabIndex"
  ];
  const descriptors = Object.fromEntries(names.map((name) => [name, accessor(detached, name)]));
  assert(descriptors.translate.get.call(detached) === false, "detached translate should inherit");

  for (const [label, element] of [["live", live], ["detached", detached]]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }

    descriptors.title.set.call(element, "Title");
    descriptors.lang.set.call(element, "en");
    descriptors.autocapitalize.set.call(element, "WORDS");
    descriptors.autocorrect.set.call(element, false);
    descriptors.dir.set.call(element, "rtl");
    descriptors.accessKey.set.call(element, "k");
    descriptors.enterKeyHint.set.call(element, "send");
    descriptors.inputMode.set.call(element, "email");
    descriptors.hidden.set.call(element, true);
    descriptors.autofocus.set.call(element, true);
    descriptors.translate.set.call(element, false);
    descriptors.draggable.set.call(element, true);
    descriptors.spellcheck.set.call(element, false);
    descriptors.tabIndex.set.call(element, 7);

    assert(descriptors.title.get.call(element) === "Title", `${label}.title`);
    assert(descriptors.lang.get.call(element) === "en", `${label}.lang`);
    assert(descriptors.autocapitalize.get.call(element) === "words", `${label}.autocapitalize`);
    assert(descriptors.autocorrect.get.call(element) === false, `${label}.autocorrect`);
    assert(descriptors.dir.get.call(element) === "rtl", `${label}.dir`);
    assert(descriptors.accessKey.get.call(element) === "k", `${label}.accessKey`);
    assert(descriptors.enterKeyHint.get.call(element) === "send", `${label}.enterKeyHint`);
    assert(descriptors.inputMode.get.call(element) === "email", `${label}.inputMode`);
    assert(descriptors.hidden.get.call(element) === true, `${label}.hidden`);
    assert(descriptors.autofocus.get.call(element) === true, `${label}.autofocus`);
    assert(descriptors.translate.get.call(element) === false, `${label}.translate`);
    assert(descriptors.draggable.get.call(element) === true, `${label}.draggable`);
    assert(descriptors.spellcheck.get.call(element) === false, `${label}.spellcheck`);
    assert(descriptors.tabIndex.get.call(element) === 7, `${label}.tabIndex`);
    assert(element.getAttribute("translate") === "no", `${label}.translate attr`);
    assert(element.getAttribute("draggable") === "true", `${label}.draggable attr`);
    assert(element.getAttribute("spellcheck") === "false", `${label}.spellcheck attr`);
    assert(element.getAttribute("tabindex") === "7", `${label}.tabindex attr`);

    descriptors.hidden.set.call(element, false);
    descriptors.autofocus.set.call(element, false);
    assert(descriptors.hidden.get.call(element) === false, `${label}.hidden false`);
    assert(descriptors.autofocus.get.call(element) === false, `${label}.autofocus false`);
    assert(!element.hasAttribute("hidden"), `${label}.hidden removed`);
    assert(!element.hasAttribute("autofocus"), `${label}.autofocus removed`);

    for (const name of names) {
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
    assert(descriptors.title.get.call(element) === "Title", `${label}.title after delete`);
    assert(descriptors.tabIndex.get.call(element) === 7, `${label}.tabIndex after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached global HTML attribute prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn specialized_url_surfaces_use_owner_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://specialized-url-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  for (const prototype of [
    HTMLAnchorElement.prototype,
    HTMLAreaElement.prototype,
    HTMLBaseElement.prototype,
    HTMLLinkElement.prototype
  ]) {
    accessor(prototype, "href", true);
  }
  accessor(HTMLAnchorElement.prototype, "text", true);
  method(HTMLAnchorElement.prototype, "toString");
  for (const prototype of [
    HTMLImageElement.prototype,
    HTMLIFrameElement.prototype,
    HTMLSourceElement.prototype,
    HTMLEmbedElement.prototype
  ]) {
    accessor(prototype, "src", true);
  }
  accessor(HTMLIFrameElement.prototype, "srcdoc", true);
  accessor(HTMLIFrameElement.prototype, "contentDocument", false);
  accessor(HTMLIFrameElement.prototype, "contentWindow", false);
  assert(!own(HTMLElement.prototype, "href"), "href should not be on HTMLElement.prototype");
  assert(!own(HTMLElement.prototype, "src"), "src should not be on HTMLElement.prototype");

  const parsed = new DOMParser().parseFromString(
    '<html><head><base><link></head><body><a>Detached</a><area><iframe></iframe><img><source><embed></body></html>',
    'text/html'
  );
  const pairs = [
    [document.createElement("a"), parsed.querySelector("a"), "HTMLAnchorElement"],
    [document.createElement("area"), parsed.querySelector("area"), "HTMLAreaElement"],
    [document.createElement("base"), parsed.querySelector("base"), "HTMLBaseElement"],
    [document.createElement("link"), parsed.querySelector("link"), "HTMLLinkElement"]
  ];
  for (const [live, detached, name] of pairs) {
    for (const element of [live, detached]) {
      assert(!own(element, "href"), `${name}.href should not be own before set`);
      element.href = `https://example.test/${name}/path?q=1#hash`;
      assert(!own(element, "href"), `${name}.href should not be own after set`);
      assert(element.getAttribute("href") === `https://example.test/${name}/path?q=1#hash`, `${name}.href attribute`);
      assert(element.href === `https://example.test/${name}/path?q=1#hash`, `${name}.href value`);
      assert(delete element.href, `${name}.href delete`);
      assert(!own(element, "href"), `${name}.href should not be own after delete`);
      assert(element.href === `https://example.test/${name}/path?q=1#hash`, `${name}.href after delete`);
    }
  }

  const liveAnchor = pairs[0][0];
  const detachedAnchor = pairs[0][1];
  for (const anchor of [liveAnchor, detachedAnchor]) {
    anchor.text = "Updated";
    assert(!own(anchor, "text"), "anchor.text should not be own");
    assert(!own(anchor, "toString"), "anchor.toString should not be own");
    assert(anchor.text === "Updated", "anchor.text value");
    assert(anchor.toString() === anchor.href, "anchor.toString");
  }

  const srcPairs = [
    [document.createElement("img"), parsed.querySelector("img"), "HTMLImageElement"],
    [document.createElement("iframe"), parsed.querySelector("iframe"), "HTMLIFrameElement"],
    [document.createElement("source"), parsed.querySelector("source"), "HTMLSourceElement"],
    [document.createElement("embed"), parsed.querySelector("embed"), "HTMLEmbedElement"]
  ];
  for (const [live, detached, name] of srcPairs) {
    for (const element of [live, detached]) {
      assert(!own(element, "src"), `${name}.src should not be own before set`);
      element.src = `https://assets.test/${name}/asset.bin`;
      assert(!own(element, "src"), `${name}.src should not be own after set`);
      assert(element.getAttribute("src") === `https://assets.test/${name}/asset.bin`, `${name}.src attribute`);
      assert(element.src === `https://assets.test/${name}/asset.bin`, `${name}.src value`);
      assert(delete element.src, `${name}.src delete`);
      assert(!own(element, "src"), `${name}.src should not be own after delete`);
      assert(element.src === `https://assets.test/${name}/asset.bin`, `${name}.src after delete`);
    }
  }

  const liveFrame = document.createElement("iframe");
  const detachedFrame = parsed.querySelector("iframe");
  for (const frame of [liveFrame, detachedFrame]) {
    assert(!own(frame, "srcdoc"), "iframe.srcdoc should not be own before set");
    assert(!own(frame, "contentDocument"), "iframe.contentDocument should not be own");
    assert(!own(frame, "contentWindow"), "iframe.contentWindow should not be own");
    frame.srcdoc = "<p>child</p>";
    assert(!own(frame, "srcdoc"), "iframe.srcdoc should not be own after set");
    assert(frame.getAttribute("srcdoc") === "<p>child</p>", "iframe.srcdoc attribute");
    assert(frame.srcdoc === "<p>child</p>", "iframe.srcdoc value");
    assert(delete frame.srcdoc, "iframe.srcdoc delete");
    assert(!own(frame, "srcdoc"), "iframe.srcdoc should not be own after delete");
    assert(frame.srcdoc === "<p>child</p>", "iframe.srcdoc after delete");
  }

  return "ok";
})()
"#,
        )
        .expect("specialized URL prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_shadow_root_surface_uses_shadow_root_prototype() {
    let mut vm = new_storage_test_vm("https://detached-shadow-root-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("section");
  doc.body.append(host);
  const root = host.attachShadow({
    mode: "open",
    delegatesFocus: true,
    slotAssignment: "manual",
    clonable: true,
    serializable: true,
    referenceTarget: "target-id"
  });

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const accessors = [
    ["host", false],
    ["mode", false],
    ["delegatesFocus", false],
    ["slotAssignment", false],
    ["clonable", false],
    ["serializable", false],
    ["referenceTarget", true],
    ["activeElement", false],
    ["innerHTML", true],
    ["styleSheets", false],
    ["adoptedStyleSheets", true]
  ];
  for (const [name, hasSetter] of accessors) {
    accessor(ShadowRoot.prototype, name, hasSetter);
  }
  for (const name of ["getHTML", "setHTMLUnsafe", "getSelection"]) {
    method(ShadowRoot.prototype, name);
  }
  method(Node.prototype, "cloneNode");

  const surface = accessors.map(([name]) => name).concat([
    "getHTML",
    "setHTMLUnsafe",
    "getSelection",
    "cloneNode"
  ]);
  for (const name of surface) {
    assert(!own(root, name), `${name} should not be own before use`);
  }

  root.innerHTML = '<style>.x{color:red}</style><button id="target-id">go</button>';
  const button = root.querySelector("button");
  assert(root.host === host, "host identity");
  assert(root.mode === "open", "mode");
  assert(root.delegatesFocus === true, "delegatesFocus");
  assert(root.slotAssignment === "manual", "slotAssignment");
  assert(root.clonable === true, "clonable");
  assert(root.serializable === true, "serializable");
  assert(root.referenceTarget === "target-id", "referenceTarget init");
  root.referenceTarget = null;
  assert(root.referenceTarget === null, "referenceTarget null");
  root.referenceTarget = true;
  assert(root.referenceTarget === "true", "referenceTarget string");
  assert(button && button.id === "target-id", "innerHTML parsed");
  assert(root.getHTML().includes('id="target-id"'), "getHTML behavior");
  assert(root.styleSheets === root.styleSheets, "styleSheets stable wrapper");
  assert(typeof root.styleSheets.length === "number", "styleSheets length shape");
  assert(Array.isArray(root.adoptedStyleSheets), "adoptedStyleSheets array");
  root.adoptedStyleSheets = [];
  assert(Array.isArray(root.adoptedStyleSheets), "adoptedStyleSheets setter");
  assert(root.getSelection() === null, "getSelection detached document");
  const focusButton = doc.createElement("button");
  focusButton.id = "focus-target";
  root.append(focusButton);
  focusButton.focus();
  assert(root.activeElement === focusButton, "detached shadow activeElement");
  root.setHTMLUnsafe("<em>done</em>");
  assert(root.innerHTML === "<em>done</em>", "setHTMLUnsafe behavior");

  const cloneResult = (() => {
    try {
      root.cloneNode(true);
      return "no-throw";
    } catch (error) {
      return `${error.name}:${error.code}:${error instanceof DOMException}`;
    }
  })();
  assert(cloneResult === "NotSupportedError:9:true", "cloneNode behavior");

  for (const name of surface) {
    assert(delete root[name], `${name} delete`);
    assert(!own(root, name), `${name} should not be own after delete`);
  }
  assert(root.host === host, "host after delete");
  assert(root.referenceTarget === "true", "referenceTarget after delete");
  return "ok";
})()
"#,
        )
        .expect("detached ShadowRoot prototype surface should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_slot_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-slot-brand-check.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor.value;
  };
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const assignedNodes = method(HTMLSlotElement.prototype, "assignedNodes");
  const assignedElements = method(HTMLSlotElement.prototype, "assignedElements");
  const assign = method(HTMLSlotElement.prototype, "assign");
  const elementAssignedSlot = accessor(Element.prototype, "assignedSlot");
  const elementSlot = accessor(Element.prototype, "slot");
  const slotName = accessor(HTMLSlotElement.prototype, "name");
  const textAssignedSlot = accessor(Text.prototype, "assignedSlot");
  assert(typeof elementSlot.set === "function", "Element.prototype.slot setter");
  assert(typeof slotName.set === "function", "HTMLSlotElement.prototype.name setter");

  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("section");
  doc.body.append(host);
  const root = host.attachShadow({ mode: "open", slotAssignment: "manual" });
  const slot = doc.createElement("slot");
  const text = doc.createTextNode("alpha");
  const span = doc.createElement("span");
  elementSlot.set.call(span, "main");
  slotName.set.call(slot, "main");
  host.append(text, span);
  root.append(slot);

  assign.call(slot, text, span);
  const nodes = assignedNodes.call(slot);
  const elements = assignedElements.call(slot);

  assert(nodes.length === 2, "assignedNodes length");
  assert(Array.prototype.includes.call(nodes, text), "assignedNodes text");
  assert(Array.prototype.includes.call(nodes, span), "assignedNodes span");
  assert(elements.length === 1, "assignedElements length");
  assert(elements[0] === span, "assignedElements span");
  assert(elementAssignedSlot.get.call(span) === slot, "element assignedSlot");
  assert(textAssignedSlot.get.call(text) === slot, "text assignedSlot");
  assert(elementSlot.get.call(span) === "main", "Element.prototype.slot getter");
  assert(slotName.get.call(slot) === "main", "HTMLSlotElement.prototype.name getter");
  assert(span.getAttribute("slot") === "main", "slot reflected attribute");
  assert(slot.getAttribute("name") === "main", "name reflected attribute");

  for (const [object, names] of [
    [slot, ["name", "assignedNodes", "assignedElements", "assign"]],
    [span, ["slot", "assignedSlot"]],
    [text, ["assignedSlot"]]
  ]) {
    for (const name of names) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }
  assert(delete slot.name, "delete inherited slot name");
  assert(delete span.slot, "delete inherited element slot");
  assert(!own(slot, "name"), "name should stay inherited after delete");
  assert(!own(span, "slot"), "slot should stay inherited after delete");
  assert(slotName.get.call(slot) === "main", "slot name after delete");
  assert(elementSlot.get.call(span) === "main", "element slot after delete");
  return "ok";
})()
"#,
        )
        .expect("detached slot prototype brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_document_view_uses_document_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-document-view-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const docs = [
    document.implementation.createHTMLDocument(""),
    document.implementation.createDocument("urn:test", "root", null),
    new DOMParser().parseFromString("<html><body></body></html>", "text/html")
  ];
  const descriptorShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const own = (doc, name) => Object.prototype.hasOwnProperty.call(doc, name);
  const before = docs.map((doc) => [
    doc.defaultView === null,
    typeof doc.parentWindow,
    own(doc, "defaultView"),
    "parentWindow" in doc,
    Object.keys(doc).includes("defaultView"),
    Object.keys(doc).includes("parentWindow")
  ].join(",")).join("|");
  const deleteResult = delete docs[0].defaultView;
  docs[0].defaultView = window;
  const parentWindowDeleteResult = delete docs[0].parentWindow;
  return [
    descriptorShape("defaultView"),
    Object.getOwnPropertyDescriptor(Document.prototype, "parentWindow") === undefined,
    before,
    deleteResult,
    parentWindowDeleteResult,
    docs[0].defaultView === null,
    typeof docs[0].parentWindow,
    own(docs[0], "defaultView"),
    own(docs[0], "parentWindow")
  ].join("||");
})()
"#,
        )
        .expect("detached document view prototype accessors should evaluate");

    assert_eq!(
        result,
        "true:function:true:true:true||true||true,undefined,false,false,false,false|true,undefined,false,false,false,false|true,undefined,false,false,false,false||true||true||true||undefined||false||false"
    );
}

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
  assert(html.hidden === false, "html hidden");
  assert(html.visibilityState === "visible", "html visibility");
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
  assert(xml.images === undefined, "xml images");
  assert(xml.hidden === false, "xml hidden");
  assert(xml.visibilityState === "visible", "xml visibility");

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
    let mut vm = new_storage_test_vm("https://document-last-modified.test/");
    vm.document_runtime
        .set_document_source_last_modified(Some(5_025_000.0));
    vm.set_timezone_override_and_sync_surface(Some("Asia/Shanghai"))
        .expect("timezone override should sync into the Date surface");

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
    ["evaluate", 5],
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
fn detached_input_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-input-submit-overrides-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, button];
  const names = ["formAction", "formEnctype", "formMethod", "formTarget", "formNoValidate"];

  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    const value = name === "formNoValidate" ? true : `${name}-value`;
    descriptor.set.call(input, value);
    assert(!Object.prototype.hasOwnProperty.call(input, name), `${name} should stay inherited`);
    assert(typeof descriptor.get.call(input) !== "undefined", `${name} direct getter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached input submitter override receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_input_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-input-reflected-brand.test/");

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

  const input = doc.createElement("input");
  const textarea = doc.createElement("textarea");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, textarea, button];
  const cases = [
    ["accept", "image/png", value => value === "image/png"],
    ["alt", "preview", value => value === "preview"],
    ["disabled", true, value => value === true],
    ["dirName", "field.dir", value => value === "field.dir"],
    ["height", 12, value => value === 12],
    ["maxLength", 10, value => value === 10],
    ["max", "9", value => value === "9"],
    ["minLength", 2, value => value === 2],
    ["min", "1", value => value === "1"],
    ["multiple", true, value => value === true],
    ["pattern", "[a-z]+", value => value === "[a-z]+"],
    ["placeholder", "hint", value => value === "hint"],
    ["readOnly", true, value => value === true],
    ["required", true, value => value === true],
    ["src", "/button.png", value => typeof value === "string" && value.endsWith("/button.png")],
    ["step", "2", value => value === "2"],
    ["width", 20, value => value === 20]
  ];

  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(input, value);
    assert(check(descriptor.get.call(input)), `${name} valid receiver`);
    assert(!own(input, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached input reflected receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_control_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-simple-control-prototypes.test/");

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
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLFieldSetElement.prototype, "disabled", true);
  accessor(HTMLFieldSetElement.prototype, "type", false);
  accessor(HTMLFieldSetElement.prototype, "elements", false);
  accessor(HTMLDataListElement.prototype, "options", false);
  accessor(HTMLLegendElement.prototype, "form", false);
  accessor(HTMLOutputElement.prototype, "type", false);
  for (const name of ["value", "min", "max", "low", "high", "optimum"]) {
    accessor(HTMLMeterElement.prototype, name, true);
  }
  accessor(HTMLProgressElement.prototype, "value", true);
  accessor(HTMLProgressElement.prototype, "max", true);
  accessor(HTMLProgressElement.prototype, "position", false);

  const form = doc.createElement("form");
  const fieldset = doc.createElement("fieldset");
  const legend = doc.createElement("legend");
  const input = doc.createElement("input");
  const datalist = doc.createElement("datalist");
  const option = doc.createElement("option");
  const output = doc.createElement("output");
  const meter = doc.createElement("meter");
  const progress = doc.createElement("progress");
  fieldset.append(legend, input);
  datalist.append(option);
  form.append(fieldset, datalist, output, meter, progress);
  doc.body.append(form);

  const checked = [
    [fieldset, ["disabled", "type", "elements"]],
    [datalist, ["options"]],
    [legend, ["form"]],
    [output, ["type"]],
    [meter, ["value", "min", "max", "low", "high", "optimum"]],
    [progress, ["value", "max", "position"]]
  ];
  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
    }
  }

  fieldset.disabled = true;
  meter.min = 1;
  meter.max = 10;
  meter.low = 2;
  meter.high = 8;
  meter.optimum = 4;
  meter.value = 5;
  progress.max = 10;
  progress.value = 5;

  assert(fieldset.disabled === true && fieldset.hasAttribute("disabled"), "fieldset disabled");
  assert(fieldset.type === "fieldset", "fieldset type");
  assert(fieldset.elements.length === 1 && fieldset.elements[0] === input, "fieldset elements");
  assert(datalist.options.length === 1 && datalist.options[0] === option, "datalist options");
  assert(legend.form === form, "legend form");
  assert(output.type === "output", "output type");
  assert(meter.min === 1 && meter.max === 10 && meter.low === 2, "meter lower values");
  assert(meter.high === 8 && meter.optimum === 4 && meter.value === 5, "meter upper values");
  assert(progress.max === 10 && progress.value === 5 && progress.position === 0.5, "progress values");

  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }

  assert(fieldset.disabled === true, "fieldset disabled after delete");
  assert(fieldset.type === "fieldset", "fieldset type after delete");
  assert(fieldset.elements.length === 1, "fieldset elements after delete");
  assert(datalist.options.length === 1, "datalist options after delete");
  assert(legend.form === form, "legend form after delete");
  assert(output.type === "output", "output type after delete");
  assert(meter.value === 5 && meter.min === 1 && meter.max === 10, "meter after delete");
  assert(progress.value === 5 && progress.max === 10 && progress.position === 0.5, "progress after delete");
  return "ok";
})()
"##,
        )
        .expect("detached simple control owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_control_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-simple-control-receiver-brand.test/");

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
  const fieldset = doc.createElement("fieldset");
  const input = doc.createElement("input");
  const meter = doc.createElement("meter");
  const progress = doc.createElement("progress");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  fieldset.append(input);
  form.append(fieldset, meter, progress);
  doc.body.append(form);

  const badReceivers = [{}, text, div, input, doc.createElement("button")];
  const assertGetterRejects = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
    return descriptor;
  };
  const assertSetterRejects = (descriptor, name, value) => {
    assert(typeof descriptor.set === "function", `${name} setter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  };

  const disabled = assertGetterRejects(HTMLFieldSetElement.prototype, "disabled");
  assertSetterRejects(disabled, "disabled", true);
  disabled.set.call(fieldset, true);
  assert(disabled.get.call(fieldset) === true, "fieldset disabled valid receiver");
  assert(!own(fieldset, "disabled"), "fieldset disabled should stay inherited");

  const fieldsetType = assertGetterRejects(HTMLFieldSetElement.prototype, "type");
  assert(fieldsetType.set === undefined, "fieldset type readonly");
  assert(fieldsetType.get.call(fieldset) === "fieldset", "fieldset type valid receiver");

  const elements = assertGetterRejects(HTMLFieldSetElement.prototype, "elements");
  assert(elements.set === undefined, "fieldset elements readonly");
  assert(elements.get.call(fieldset).length === 1, "fieldset elements valid receiver");

  const meterValues = [
    ["min", 1, 1],
    ["max", 10, 10],
    ["low", 2, 2],
    ["high", 8, 8],
    ["optimum", 4, 4],
    ["value", 5, 5]
  ];
  for (const [name, value, expected] of meterValues) {
    const descriptor = assertGetterRejects(HTMLMeterElement.prototype, name);
    assertSetterRejects(descriptor, name, value);
    descriptor.set.call(meter, value);
    assert(descriptor.get.call(meter) === expected, `meter ${name} valid receiver`);
    assert(!own(meter, name), `meter ${name} should stay inherited`);
  }

  const progressValue = assertGetterRejects(HTMLProgressElement.prototype, "value");
  assertSetterRejects(progressValue, "value", 5);
  const progressMax = assertGetterRejects(HTMLProgressElement.prototype, "max");
  assertSetterRejects(progressMax, "max", 10);
  progressMax.set.call(progress, 10);
  progressValue.set.call(progress, 5);
  assert(progressMax.get.call(progress) === 10, "progress max valid receiver");
  assert(progressValue.get.call(progress) === 5, "progress value valid receiver");
  assert(!own(progress, "max") && !own(progress, "value"), "progress writable attrs inherited");

  const position = assertGetterRejects(HTMLProgressElement.prototype, "position");
  assert(position.set === undefined, "progress position readonly");
  assert(position.get.call(progress) === 0.5, "progress position valid receiver");
  assert(!own(progress, "position"), "progress position should stay inherited");

  return "ok";
})()
"#,
        )
        .expect("detached simple control receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_popover_accessor_uses_html_element_prototype() {
    let mut vm = new_storage_test_vm("https://detached-popover-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "popover");
  assert(!!descriptor, "popover descriptor");
  assert(typeof descriptor.get === "function", "popover getter");
  assert(typeof descriptor.set === "function", "popover setter");
  assert(descriptor.enumerable === true, "popover enumerable");
  assert(descriptor.configurable === true, "popover configurable");

  const html = document.implementation.createHTMLDocument("");
  const cases = [
    ["live", document.createElement("div")],
    ["detached", html.createElement("div")]
  ];
  for (const [label, element] of cases) {
    assert(!own(element, "popover"), `${label}.popover should not be own before set`);
    assert(descriptor.get.call(element) === null, `${label}.popover missing`);
    descriptor.set.call(element, "");
    assert(!own(element, "popover"), `${label}.popover should not be own after empty set`);
    assert(element.getAttribute("popover") === "", `${label}.popover empty attr`);
    assert(descriptor.get.call(element) === "auto", `${label}.popover auto`);
    descriptor.set.call(element, "hint");
    assert(element.getAttribute("popover") === "hint", `${label}.popover hint attr`);
    assert(descriptor.get.call(element) === "hint", `${label}.popover hint`);
    descriptor.set.call(element, "invalid");
    assert(element.getAttribute("popover") === "invalid", `${label}.popover invalid attr`);
    assert(descriptor.get.call(element) === "manual", `${label}.popover canonical manual`);
    descriptor.set.call(element, null);
    assert(!element.hasAttribute("popover"), `${label}.popover removed`);
    assert(descriptor.get.call(element) === null, `${label}.popover removed value`);
    assert(delete element.popover, `${label}.popover delete`);
    assert(!own(element, "popover"), `${label}.popover should stay inherited`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached popover prototype accessor should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_popover_methods_use_html_element_prototype_brand_checks() {
    let mut vm = new_storage_test_vm("https://detached-popover-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptorShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.value,
      descriptor.value && descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const outcome = (callback) => {
    try {
      const value = callback();
      return `OK:${value === undefined ? "undefined" : String(value)}`;
    } catch (error) {
      return `ERR:${error.name}:${error.code || ""}`;
    }
  };

  const html = document.implementation.createHTMLDocument("");
  const plain = html.createElement("div");
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const popover = html.createElement("div");
  popover.setAttribute("popover", "");

  return JSON.stringify({
    shapes: ["showPopover", "hidePopover", "togglePopover"].map(descriptorShape).join("|"),
    own: [
      own(popover, "showPopover"),
      own(popover, "hidePopover"),
      own(popover, "togglePopover")
    ].join(","),
    elementOwn: [
      own(Element.prototype, "showPopover"),
      own(Element.prototype, "hidePopover"),
      own(Element.prototype, "togglePopover")
    ].join(","),
    svgTypes: [
      typeof svg.showPopover,
      typeof svg.hidePopover,
      typeof svg.togglePopover
    ].join(","),
    direct: [
      outcome(() => plain.showPopover()),
      outcome(() => popover.showPopover()),
      outcome(() => popover.hidePopover()),
      outcome(() => popover.togglePopover())
    ].join("|"),
    prototype: [
      outcome(() => HTMLElement.prototype.showPopover.call(plain)),
      outcome(() => HTMLElement.prototype.showPopover.call(popover)),
      outcome(() => HTMLElement.prototype.hidePopover.call(popover)),
      outcome(() => HTMLElement.prototype.togglePopover.call(popover))
    ].join("|")
  });
})()
"#,
        )
        .expect("detached popover method brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"shapes":"true:function:0:true:true|true:function:0:true:true|true:function:0:true:true","own":"false,false,false","elementOwn":"false,false,false","svgTypes":"undefined,undefined,undefined","direct":"ERR:NotSupportedError:9|ERR:InvalidStateError:11|ERR:InvalidStateError:11|ERR:InvalidStateError:11","prototype":"ERR:NotSupportedError:9|ERR:InvalidStateError:11|ERR:InvalidStateError:11|ERR:InvalidStateError:11"}"#
    );
}

#[test]
fn detached_button_and_textarea_accessors_use_owner_prototypes() {
    let mut vm =
        new_storage_test_vm("https://detached-button-textarea-prototypes.test/base/page.html");

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
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const buttonNames = [
    "disabled",
    "formAction",
    "formEnctype",
    "formMethod",
    "formNoValidate",
    "formTarget",
    "type",
    "commandForElement",
    "popoverTargetElement",
    "popoverTargetAction",
    "interestForElement",
    "value"
  ];
  for (const name of buttonNames) accessor(HTMLButtonElement.prototype, name, true);

  const textareaSetters = [
    "disabled",
    "dirName",
    "maxLength",
    "minLength",
    "required",
    "cols",
    "rows",
    "wrap",
    "placeholder",
    "readOnly",
    "defaultValue",
    "value"
  ];
  for (const name of textareaSetters) accessor(HTMLTextAreaElement.prototype, name, true);
  accessor(HTMLTextAreaElement.prototype, "textLength", false);
  accessor(HTMLTextAreaElement.prototype, "type", false);

  const form = doc.createElement("form");
  const button = doc.createElement("button");
  const target = doc.createElement("div");
  const textarea = doc.createElement("textarea");
  assert(!("required" in button), "button required should not be an IDL property");
  assert(!Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, "required"),
         "HTMLButtonElement.required descriptor should be absent");
  button.setAttribute("required", false);
  assert(button.getAttribute("required") === "false", "button required=false attribute text");
  button.setAttribute("required", true);
  assert(button.getAttribute("required") === "true", "button required=true attribute text");
  target.id = "target";
  form.append(button, textarea);
  doc.body.append(target, form);

  const checked = [
    [button, buttonNames],
    [textarea, [...textareaSetters, "textLength", "type"]]
  ];
  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
    }
  }

  button.disabled = true;
  button.formAction = "/submit";
  button.formEnctype = "text/plain";
  button.formMethod = "post";
  button.formNoValidate = true;
  button.formTarget = "_blank";
  button.type = "reset";
  button.commandForElement = target;
  button.popoverTargetElement = target;
  button.popoverTargetAction = "show";
  button.interestForElement = target;
  button.value = "go";

  textarea.disabled = true;
  textarea.dirName = "comment.dir";
  textarea.maxLength = 12;
  textarea.minLength = 2;
  textarea.required = true;
  textarea.cols = 40;
  textarea.rows = 6;
  textarea.wrap = "hard";
  textarea.placeholder = "hint";
  textarea.readOnly = true;
  textarea.defaultValue = "default";
  textarea.value = "hello";

  assert(button.disabled === true && button.hasAttribute("disabled"), "button disabled");
  assert(button.getAttribute("formaction") === "/submit", "button formAction attribute");
  assert(typeof button.formAction === "string" && button.formAction.length > 0, "button formAction");
  assert(button.formEnctype === "text/plain", "button formEnctype");
  assert(button.formMethod === "post", "button formMethod");
  assert(button.formNoValidate === true, "button formNoValidate");
  assert(button.formTarget === "_blank", "button formTarget");
  assert(button.type === "reset", "button type");
  assert(button.commandForElement === target, "button commandForElement");
  assert(button.popoverTargetElement === target, "button popoverTargetElement");
  assert(button.popoverTargetAction === "show", "button popoverTargetAction");
  assert(button.interestForElement === target, "button interestForElement");
  assert(!("required" in button), "button required should remain absent");
  assert(button.getAttribute("required") === "true", "button required attribute stays textual");
  assert(button.value === "go", "button value");

  assert(textarea.disabled === true, "textarea disabled");
  assert(textarea.dirName === "comment.dir", "textarea dirName");
  assert(textarea.maxLength === 12 && textarea.minLength === 2, "textarea length limits");
  assert(textarea.required === true, "textarea required");
  assert(textarea.cols === 40 && textarea.rows === 6, "textarea dimensions");
  assert(textarea.wrap === "hard", "textarea wrap");
  assert(textarea.placeholder === "hint", "textarea placeholder");
  assert(textarea.readOnly === true, "textarea readOnly");
  assert(textarea.defaultValue === "default", "textarea defaultValue");
  assert(textarea.value === "hello" && textarea.textLength === 5, "textarea value");
  assert(textarea.type === "textarea", "textarea type");

  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }

  assert(button.commandForElement === target, "button commandForElement after delete");
  assert(button.popoverTargetElement === target, "button popoverTargetElement after delete");
  assert(button.popoverTargetAction === "show", "button popoverTargetAction after delete");
  assert(button.interestForElement === target, "button interestForElement after delete");
  assert(textarea.value === "hello" && textarea.textLength === 5, "textarea after delete");
  return "ok";
})()
"##,
        )
        .expect("detached button/textarea owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_button_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-button-submit-overrides-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input];
  const names = ["formAction", "formEnctype", "formMethod", "formTarget", "formNoValidate"];

  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    const value = name === "formNoValidate" ? true : `${name}-value`;
    descriptor.set.call(button, value);
    assert(!Object.prototype.hasOwnProperty.call(button, name), `${name} should stay inherited`);
    assert(typeof descriptor.get.call(button) !== "undefined", `${name} direct getter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached button submitter override receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_button_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-button-reflected-brand.test/");

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

  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input];
  const cases = [
    ["disabled", true, value => value === true],
    ["type", "reset", value => value === "reset"],
    ["value", "go", value => value === "go"]
  ];

  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(button, value);
    assert(check(descriptor.get.call(button)), `${name} valid receiver`);
    assert(!own(button, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached button reflected receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_textarea_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-textarea-reflected-brand.test/");

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

  const textarea = doc.createElement("textarea");
  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input, button];
  const cases = [
    ["disabled", true, value => value === true],
    ["required", true, value => value === true],
    ["readOnly", true, value => value === true],
    ["dirName", "posted", value => value === "posted"],
    ["maxLength", 12, value => value === 12],
    ["minLength", 2, value => value === 2],
    ["cols", 12, value => value === 12],
    ["rows", 4, value => value === 4],
    ["wrap", "hard", value => value === "hard"],
    ["placeholder", "enter text", value => value === "enter text"],
    ["defaultValue", "seed", value => value === "seed"],
    ["value", "body", value => value === "body"]
  ];

  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(textarea, value);
    assert(check(descriptor.get.call(textarea)), `${name} valid receiver`);
    assert(!own(textarea, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }

  const readonlyCases = [
    ["textLength", value => value === 4],
    ["type", value => value === "textarea"]
  ];
  for (const [name, check] of readonlyCases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} readonly`);
    assert(check(descriptor.get.call(textarea)), `${name} valid receiver`);
    assert(!own(textarea, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached textarea reflected receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_object_param_and_data_accessors_use_owner_prototypes() {
    let mut vm =
        new_storage_test_vm("https://detached-object-param-data-prototypes.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const object = doc.createElement("object");
  const param = doc.createElement("param");
  const data = doc.createElement("data");
  const div = doc.createElement("div");
  doc.body.append(object, param, data, div);

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
  const absent = (prototype, name) => {
    assert(
      Object.getOwnPropertyDescriptor(prototype, name) === undefined,
      `${prototype.constructor.name}.${name} should be absent`
    );
  };

  for (const name of [
    "data",
    "type",
    "archive",
    "code",
    "codeBase",
    "codeType",
    "declare",
    "standby"
  ]) {
    accessor(HTMLObjectElement.prototype, name);
    absent(HTMLElement.prototype, name);
    assert(!own(object, name), `object.${name} should not be own`);
    assert(!(name in div), `div.${name} should be absent`);
  }
  for (const name of ["value", "type", "valueType"]) {
    accessor(HTMLParamElement.prototype, name);
    absent(HTMLElement.prototype, name);
    assert(!own(param, name), `param.${name} should not be own`);
    assert(!(name in div), `div.${name} should be absent`);
  }
  accessor(HTMLDataElement.prototype, "value");
  absent(HTMLElement.prototype, "value");
  assert(!own(data, "value"), "data.value should not be own");

  object.data = "https://assets.detached/plugin.bin";
  object.type = "application/x-test";
  object.archive = "archive.jar";
  object.code = "Applet";
  object.codeBase = "https://assets.detached/classes/";
  object.codeType = "application/java";
  object.declare = true;
  object.standby = "Loading";
  assert(object.data === "https://assets.detached/plugin.bin", "object data");
  assert(object.type === "application/x-test", "object type");
  assert(object.archive === "archive.jar", "object archive");
  assert(object.code === "Applet", "object code");
  assert(object.codeBase === "https://assets.detached/classes/", "object codeBase");
  assert(object.codeType === "application/java", "object codeType");
  assert(object.declare === true, "object declare");
  assert(object.hasAttribute("declare"), "object declare attr");
  assert(object.standby === "Loading", "object standby");

  param.value = "param-value";
  param.type = "text/plain";
  param.valueType = "data";
  assert(param.value === "param-value", "param value");
  assert(param.type === "text/plain", "param type");
  assert(param.valueType === "data", "param valueType");

  data.value = "data-value";
  assert(data.value === "data-value", "data value");

  for (const [element, names] of [
    [object, ["data", "type", "archive", "code", "codeBase", "codeType", "declare", "standby"]],
    [param, ["value", "type", "valueType"]],
    [data, ["value"]]
  ]) {
    for (const name of names) {
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }
  assert(object.data === "https://assets.detached/plugin.bin", "object data after delete");
  assert(object.declare === true, "object declare after delete");
  assert(param.valueType === "data", "param valueType after delete");
  assert(data.value === "data-value", "data value after delete");
  return "ok";
})()
"##,
        )
        .expect("detached object/param/data owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_media_quote_mod_time_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-html-media-quote-mod-time.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const html = doc.documentElement;
  const audio = doc.createElement("audio");
  const video = doc.createElement("video");
  const q = doc.createElement("q");
  const blockquote = doc.createElement("blockquote");
  const ins = doc.createElement("ins");
  const del = doc.createElement("del");
  const time = doc.createElement("time");
  const div = doc.createElement("div");
  doc.body.append(audio, video, q, blockquote, ins, del, time, div);

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
  const absent = (prototype, name) => {
    assert(
      Object.getOwnPropertyDescriptor(prototype, name) === undefined,
      `${prototype.constructor.name}.${name} should be absent`
    );
  };

  accessor(HTMLHtmlElement.prototype, "version");
  accessor(HTMLMediaElement.prototype, "preload");
  accessor(HTMLQuoteElement.prototype, "cite");
  accessor(HTMLModElement.prototype, "cite");
  accessor(HTMLModElement.prototype, "dateTime");
  accessor(HTMLTimeElement.prototype, "dateTime");
  for (const name of ["version", "preload", "cite", "dateTime"]) {
    absent(HTMLElement.prototype, name);
  }
  assert(!own(HTMLAudioElement.prototype, "preload"), "audio should inherit preload");
  assert(!own(HTMLVideoElement.prototype, "preload"), "video should inherit preload");

  for (const [element, names, label] of [
    [html, ["version"], "html"],
    [audio, ["preload"], "audio"],
    [video, ["preload"], "video"],
    [q, ["cite"], "q"],
    [blockquote, ["cite"], "blockquote"],
    [ins, ["cite", "dateTime"], "ins"],
    [del, ["cite", "dateTime"], "del"],
    [time, ["dateTime"], "time"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
  }
  for (const name of ["version", "preload", "cite", "dateTime"]) {
    assert(!(name in div), `div.${name} should be absent`);
  }

  html.version = "4.01";
  audio.preload = "metadata";
  video.preload = "none";
  q.cite = "https://assets.detached/q.html";
  blockquote.cite = "https://assets.detached/quote.html";
  ins.cite = "https://assets.detached/ins.html";
  ins.dateTime = "2026-06-19";
  del.cite = "https://assets.detached/del.html";
  del.dateTime = "2026-06-20";
  time.dateTime = "2026-06-21";

  assert(html.version === "4.01" && html.getAttribute("version") === "4.01", "html version");
  assert(audio.preload === "metadata" && audio.getAttribute("preload") === "metadata", "audio preload");
  assert(video.preload === "none" && video.getAttribute("preload") === "none", "video preload");
  audio.preload = "invalid";
  assert(audio.preload === "auto" && audio.getAttribute("preload") === "invalid", "audio invalid preload");
  assert(q.cite === "https://assets.detached/q.html", "q cite URL");
  assert(blockquote.cite === "https://assets.detached/quote.html", "blockquote cite URL");
  assert(ins.cite === "https://assets.detached/ins.html", "ins cite URL");
  assert(ins.dateTime === "2026-06-19", "ins dateTime");
  assert(del.cite === "https://assets.detached/del.html", "del cite URL");
  assert(del.dateTime === "2026-06-20", "del dateTime");
  assert(time.dateTime === "2026-06-21", "time dateTime");

  for (const [element, names, label] of [
    [html, ["version"], "html"],
    [audio, ["preload"], "audio"],
    [video, ["preload"], "video"],
    [q, ["cite"], "q"],
    [blockquote, ["cite"], "blockquote"],
    [ins, ["cite", "dateTime"], "ins"],
    [del, ["cite", "dateTime"], "del"],
    [time, ["dateTime"], "time"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
  }
  assert(html.version === "4.01", "html version after delete");
  assert(audio.preload === "auto", "audio preload after delete");
  assert(q.cite === "https://assets.detached/q.html", "q cite after delete");
  assert(ins.dateTime === "2026-06-19", "ins dateTime after delete");
  assert(time.dateTime === "2026-06-21", "time dateTime after delete");
  return "ok";
})()
"#,
        )
        .expect("detached HTML/media/quote/mod/time owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_label_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-label-owner-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const optgroup = doc.createElement("optgroup");
  const option = doc.createElement("option");
  const track = doc.createElement("track");
  const div = doc.createElement("div");
  const select = doc.createElement("select");
  option.textContent = "Fallback";
  doc.body.append(optgroup, option, track, div, select);

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

  accessor(HTMLOptGroupElement.prototype, "label");
  accessor(HTMLOptionElement.prototype, "label");
  accessor(HTMLTrackElement.prototype, "label");
  assert(!own(HTMLElement.prototype, "label"), "label should not be on HTMLElement.prototype");
  assert(!("label" in div), "div should not expose label");
  assert(!("label" in select), "select should not expose label");

  for (const [element, tag] of [[optgroup, "optgroup"], [option, "option"], [track, "track"]]) {
    assert(!own(element, "label"), `${tag}.label should not be own before set`);
  }
  assert(optgroup.label === "", "optgroup default label");
  assert(option.label === "Fallback", "option label fallback");
  assert(track.label === "", "track default label");

  optgroup.label = "Group";
  option.label = "Explicit";
  track.label = "English";
  assert(optgroup.label === "Group" && optgroup.getAttribute("label") === "Group", "optgroup label");
  assert(option.label === "Explicit" && option.getAttribute("label") === "Explicit", "option label");
  assert(track.label === "English" && track.getAttribute("label") === "English", "track label");

  for (const [element, tag] of [[optgroup, "optgroup"], [option, "option"], [track, "track"]]) {
    assert(!own(element, "label"), `${tag}.label should not be own after set`);
    assert(delete element.label, `${tag}.label delete`);
    assert(!own(element, "label"), `${tag}.label should stay inherited`);
  }
  assert(optgroup.label === "Group", "optgroup label after delete");
  assert(option.label === "Explicit", "option label after delete");
  assert(track.label === "English", "track label after delete");
  option.removeAttribute("label");
  assert(option.label === "Fallback", "option label fallback after attribute removal");
  return "ok";
})()
"#,
        )
        .expect("detached label owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_structural_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-simple-structural-owner-prototypes.test/");

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
  const method = (prototype, name, length) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.value.length === length, `${name} length`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const prototypeCases = [
    [HTMLLIElement.prototype, ["value"]],
    [HTMLOListElement.prototype, ["start", "reversed", "type"]],
    [HTMLOptGroupElement.prototype, ["disabled"]],
    [HTMLDetailsElement.prototype, ["open"]],
    [HTMLDialogElement.prototype, ["open", "returnValue"]],
    [HTMLMetaElement.prototype, ["content", "httpEquiv"]],
    [HTMLTitleElement.prototype, ["text"]]
  ];
  for (const [prototype, names] of prototypeCases) {
    for (const name of names) {
      accessor(prototype, name);
      assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    }
  }
  method(HTMLDialogElement.prototype, "show", 0);
  method(HTMLDialogElement.prototype, "showModal", 0);
  method(HTMLDialogElement.prototype, "close", 1);

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const [doc, label] of [[document, "live"], [detachedDoc, "detached"]]) {
    const li = doc.createElement("li");
    const ol = doc.createElement("ol");
    const optgroup = doc.createElement("optgroup");
    const details = doc.createElement("details");
    const dialog = doc.createElement("dialog");
    const meta = doc.createElement("meta");
    const title = doc.createElement("title");

    const cases = [
      [li, "value", 7, "7", "value"],
      [ol, "start", 3, "3", "start"],
      [ol, "reversed", true, "", "reversed"],
      [ol, "type", "A", "A", "type"],
      [optgroup, "disabled", true, "", "disabled"],
      [details, "open", true, "", "open"],
      [dialog, "open", true, "", "open"],
      [meta, "content", "width=device-width", "width=device-width", "content"],
      [meta, "httpEquiv", "refresh", "refresh", "http-equiv"],
      [title, "text", "Page Title", "Page Title", null]
    ];

    for (const [element, name, value, expected, attribute] of cases) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
      element[name] = value;
      assert(element[name] === value || element[name] === expected, `${label}.${name} getter`);
      if (attribute === null) {
        assert(element.textContent === expected, `${label}.${name} text content`);
      } else {
        assert(element.getAttribute(attribute) === expected, `${label}.${name} attribute`);
      }
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
      assert(element[name] === value || element[name] === expected, `${label}.${name} after delete`);
    }

    assert(!own(dialog, "returnValue"), `${label}.returnValue should not be own before set`);
    dialog.returnValue = "done";
    assert(dialog.returnValue === "done", `${label}.returnValue getter`);
    assert(dialog.getAttribute("returnvalue") === null, `${label}.returnValue must not reflect`);
    assert(!own(dialog, "returnValue"), `${label}.returnValue should not be own after set`);
    assert(delete dialog.returnValue, `${label}.returnValue delete`);
    assert(dialog.returnValue === "done", `${label}.returnValue after delete`);

    for (const name of ["show", "showModal", "close"]) {
      assert(!own(dialog, name), `${label}.dialog.${name} should not be own`);
    }
    assert(!Object.prototype.hasOwnProperty.call(dialog, "__moliDialogHandle"), `${label}.dialog private handle should not be own`);
    dialog.show();
    assert(dialog.open === true, `${label}.dialog show behavior`);
    dialog.close("closed");
    assert(dialog.open === false && dialog.returnValue === "closed", `${label}.dialog close behavior`);
    let showModalError;
    try {
      dialog.showModal();
    } catch (error) {
      showModalError = error;
    }
    assert(showModalError instanceof DOMException, `${label}.dialog showModal DOMException`);
    assert(showModalError.name === "InvalidStateError", `${label}.dialog showModal error name`);
    assert(dialog.open === false, `${label}.dialog showModal keeps disconnected dialog closed`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached simple structural owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_specialized_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-simple-specialized-receiver-brand.test/");

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

  const doc = document.implementation.createHTMLDocument("");
  const li = doc.createElement("li");
  const ol = doc.createElement("ol");
  const optgroup = doc.createElement("optgroup");
  const details = doc.createElement("details");
  const meta = doc.createElement("meta");
  const title = doc.createElement("title");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const elements = [li, ol, optgroup, details, meta, title, div];

  const cases = [
    [HTMLLIElement.prototype, "value", li, 7],
    [HTMLOListElement.prototype, "start", ol, 3],
    [HTMLOListElement.prototype, "reversed", ol, true],
    [HTMLOListElement.prototype, "type", ol, "A"],
    [HTMLOptGroupElement.prototype, "disabled", optgroup, true],
    [HTMLDetailsElement.prototype, "open", details, true],
    [HTMLMetaElement.prototype, "content", meta, "width=device-width"],
    [HTMLMetaElement.prototype, "httpEquiv", meta, "refresh"],
    [HTMLTitleElement.prototype, "text", title, "Page Title"]
  ];

  for (const [prototype, name, element, value] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(typeof descriptor.get.call(element) !== "undefined", `${name} valid getter`);
    descriptor.set.call(element, value);
    assert(!own(element, name), `${name} should stay inherited`);

    for (const receiver of [{}, text, ...elements.filter(candidate => candidate !== element)]) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached simple specialized receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_frame_legacy_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-frame-legacy-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const frame = doc.createElement("frame");
  const iframe = doc.createElement("iframe");
  const img = doc.createElement("img");
  const div = doc.createElement("div");
  doc.body.append(frame, iframe, img, div);

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

  const frameNames = ["scrolling", "frameBorder", "longDesc", "marginHeight", "marginWidth"];
  for (const name of frameNames) {
    accessor(HTMLFrameElement.prototype, name);
    accessor(HTMLIFrameElement.prototype, name);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }
  accessor(HTMLImageElement.prototype, "longDesc");
  for (const name of ["scrolling", "frameBorder", "marginHeight", "marginWidth"]) {
    assert(!(name in img), `${name} should not be on img`);
  }

  for (const [element, label] of [[frame, "frame"], [iframe, "iframe"]]) {
    for (const name of frameNames) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
    element.scrolling = `${label}-scroll`;
    element.frameBorder = `${label}-border`;
    element.longDesc = `https://assets.example/${label}-desc`;
    element.marginHeight = null;
    element.marginWidth = `${label}-width`;
    assert(element.scrolling === `${label}-scroll`, `${label} scrolling`);
    assert(element.getAttribute("scrolling") === `${label}-scroll`, `${label} scrolling attr`);
    assert(element.frameBorder === `${label}-border`, `${label} frameBorder`);
    assert(element.getAttribute("frameborder") === `${label}-border`, `${label} frameBorder attr`);
    assert(element.longDesc === `https://assets.example/${label}-desc`, `${label} longDesc`);
    assert(element.getAttribute("longdesc") === `https://assets.example/${label}-desc`, `${label} longDesc attr`);
    assert(element.marginHeight === "", `${label} marginHeight null`);
    assert(element.getAttribute("marginheight") === "", `${label} marginHeight attr`);
    assert(element.marginWidth === `${label}-width`, `${label} marginWidth`);
    assert(element.getAttribute("marginwidth") === `${label}-width`, `${label} marginWidth attr`);
    for (const name of frameNames) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
  }

  assert(!own(img, "longDesc"), "img.longDesc should not be own before set");
  img.longDesc = "https://assets.example/image-desc";
  assert(img.longDesc === "https://assets.example/image-desc", "image longDesc");
  assert(img.getAttribute("longdesc") === "https://assets.example/image-desc", "image longDesc attr");
  assert(!own(img, "longDesc"), "img.longDesc should not be own after set");
  assert(delete img.longDesc, "img.longDesc delete");
  assert(!own(img, "longDesc"), "img.longDesc should stay inherited");
  assert(img.longDesc === "https://assets.example/image-desc", "image longDesc after delete");
  return "ok";
})()
"#,
        )
        .expect("detached frame legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_resource_legacy_accessors_use_owner_prototypes() {
    let mut vm =
        new_storage_test_vm("https://detached-resource-legacy-prototypes.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const area = doc.createElement("area");
  const image = doc.createElement("img");
  const source = doc.createElement("source");
  const object = doc.createElement("object");
  const div = doc.createElement("div");
  doc.body.append(area, image, source, object, div);

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

  accessor(HTMLAreaElement.prototype, "alt");
  accessor(HTMLImageElement.prototype, "alt");
  accessor(HTMLImageElement.prototype, "useMap");
  accessor(HTMLImageElement.prototype, "srcset");
  accessor(HTMLImageElement.prototype, "lowsrc");
  accessor(HTMLImageElement.prototype, "decoding");
  accessor(HTMLSourceElement.prototype, "srcset");
  accessor(HTMLObjectElement.prototype, "useMap");
  for (const name of ["alt", "useMap", "srcset", "lowsrc", "decoding"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }

  for (const [element, names, label] of [
    [area, ["alt"], "area"],
    [image, ["alt", "useMap", "srcset", "lowsrc", "decoding"], "image"],
    [source, ["srcset"], "source"],
    [object, ["useMap"], "object"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
  }

  area.alt = "map alt";
  image.alt = "image alt";
  image.useMap = "#main-map";
  image.srcset = "small.png 1x, large.png 2x";
  image.lowsrc = "https://assets.example/low.png";
  image.decoding = "ASYNC";
  source.srcset = "source-small.png 1x";
  object.useMap = "#object-map";

  assert(area.alt === "map alt" && area.getAttribute("alt") === "map alt", "area alt");
  assert(image.alt === "image alt" && image.getAttribute("alt") === "image alt", "image alt");
  assert(image.useMap === "#main-map" && image.getAttribute("usemap") === "#main-map", "image useMap");
  assert(image.srcset === "small.png 1x, large.png 2x", "image srcset");
  assert(image.lowsrc === "https://assets.example/low.png", "image lowsrc");
  assert(image.decoding === "async" && image.getAttribute("decoding") === "ASYNC", "image decoding canonical");
  image.decoding = "invalid";
  assert(image.decoding === "auto", "image decoding invalid");
  assert(source.srcset === "source-small.png 1x" && source.getAttribute("srcset") === "source-small.png 1x", "source srcset");
  assert(object.useMap === "#object-map" && object.getAttribute("usemap") === "#object-map", "object useMap");

  for (const [element, names, label] of [
    [area, ["alt"], "area"],
    [image, ["alt", "useMap", "srcset", "lowsrc", "decoding"], "image"],
    [source, ["srcset"], "source"],
    [object, ["useMap"], "object"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
  }
  assert(image.useMap === "#main-map", "image useMap after delete");
  assert(image.decoding === "auto", "image decoding after delete");
  assert(source.srcset === "source-small.png 1x", "source srcset after delete");
  assert(object.useMap === "#object-map", "object useMap after delete");
  return "ok";
})()
"##,
        )
        .expect("detached resource legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_legacy_dimension_and_color_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-legacy-dimension-color-prototypes.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const table = doc.createElement("table");
  const row = doc.createElement("tr");
  const cell = doc.createElement("td");
  const image = doc.createElement("img");
  const object = doc.createElement("object");
  const hr = doc.createElement("hr");
  const font = doc.createElement("font");
  const marquee = doc.createElement("marquee");
  const div = doc.createElement("div");
  doc.body.append(table, row, cell, image, object, hr, font, marquee, div);

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

  const ownerChecks = [
    [HTMLBodyElement.prototype, "bgColor"],
    [HTMLTableElement.prototype, "bgColor"],
    [HTMLTableRowElement.prototype, "bgColor"],
    [HTMLTableCellElement.prototype, "bgColor"],
    [HTMLMarqueeElement.prototype, "bgColor"],
    [HTMLTableElement.prototype, "border"],
    [HTMLImageElement.prototype, "border"],
    [HTMLObjectElement.prototype, "border"],
    [HTMLHRElement.prototype, "color"],
    [HTMLFontElement.prototype, "color"],
    [HTMLImageElement.prototype, "hspace"],
    [HTMLImageElement.prototype, "vspace"],
    [HTMLObjectElement.prototype, "hspace"],
    [HTMLObjectElement.prototype, "vspace"],
    [HTMLMarqueeElement.prototype, "hspace"],
    [HTMLMarqueeElement.prototype, "vspace"]
  ];
  for (const [prototype, name] of ownerChecks) accessor(prototype, name);
  for (const name of ["bgColor", "border", "color", "hspace", "vspace"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }

  for (const [element, name] of [
    [table, "bgColor"], [row, "bgColor"], [cell, "bgColor"], [marquee, "bgColor"],
    [table, "border"], [image, "border"], [object, "border"],
    [hr, "color"], [font, "color"],
    [image, "hspace"], [image, "vspace"], [object, "hspace"], [object, "vspace"],
    [marquee, "hspace"], [marquee, "vspace"]
  ]) {
    assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
  }

  table.bgColor = "red";
  row.bgColor = "green";
  cell.bgColor = null;
  marquee.bgColor = "blue";
  table.border = "3";
  image.border = null;
  object.border = null;
  hr.color = "black";
  font.color = null;
  image.hspace = 7;
  image.vspace = 8;
  object.hspace = 9;
  object.vspace = 10;
  marquee.hspace = 11;
  marquee.vspace = 12;

  assert(table.bgColor === "red" && table.getAttribute("bgcolor") === "red", "table bgColor");
  assert(row.bgColor === "green" && row.getAttribute("bgcolor") === "green", "row bgColor");
  assert(cell.bgColor === "" && cell.getAttribute("bgcolor") === "", "cell bgColor null");
  assert(marquee.bgColor === "blue" && marquee.getAttribute("bgcolor") === "blue", "marquee bgColor");
  assert(table.border === "3" && table.getAttribute("border") === "3", "table border");
  assert(image.border === "" && image.getAttribute("border") === "", "image border null");
  assert(object.border === "" && object.getAttribute("border") === "", "object border null");
  assert(hr.color === "black" && hr.getAttribute("color") === "black", "hr color");
  assert(font.color === "" && font.getAttribute("color") === "", "font color null");
  assert(image.hspace === 7 && image.getAttribute("hspace") === "7", "image hspace");
  assert(image.vspace === 8 && image.getAttribute("vspace") === "8", "image vspace");
  assert(object.hspace === 9 && object.getAttribute("hspace") === "9", "object hspace");
  assert(object.vspace === 10 && object.getAttribute("vspace") === "10", "object vspace");
  assert(marquee.hspace === 11 && marquee.getAttribute("hspace") === "11", "marquee hspace");
  assert(marquee.vspace === 12 && marquee.getAttribute("vspace") === "12", "marquee vspace");

  for (const [element, name] of [
    [table, "bgColor"], [row, "bgColor"], [cell, "bgColor"], [marquee, "bgColor"],
    [table, "border"], [image, "border"], [object, "border"],
    [hr, "color"], [font, "color"],
    [image, "hspace"], [image, "vspace"], [object, "hspace"], [object, "vspace"],
    [marquee, "hspace"], [marquee, "vspace"]
  ]) {
    assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
    assert(delete element[name], `${element.localName}.${name} delete`);
    assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
  }
  assert(table.bgColor === "red", "table bgColor after delete");
  assert(cell.bgColor === "", "cell bgColor after delete");
  assert(font.color === "", "font color after delete");
  assert(image.hspace === 7 && marquee.vspace === 12, "unsigned after delete");
  return "ok";
})()
"##,
        )
        .expect("detached legacy dimension and color owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_marquee_numeric_attributes_follow_legacy_reflection() {
    let mut vm = new_storage_test_vm("https://detached-marquee-numeric.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const marquee = doc.createElement("marquee");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throws = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.name;
    }
  };

  for (const name of ["loop", "scrollAmount", "scrollDelay"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLMarqueeElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable && descriptor.configurable, `${name} descriptor flags`);
    assert(!Object.prototype.hasOwnProperty.call(HTMLElement.prototype, name), `${name} owner`);
    assert(throws(() => descriptor.get.call(doc.createElement("div"))) === "TypeError", `${name} getter brand`);
    assert(throws(() => descriptor.set.call(doc.createElement("div"), 2)) === "TypeError", `${name} setter brand`);
  }

  for (const [raw, expected] of [
    [null, -1],
    ["a1", -1],
    ["-2", -1],
    ["0", -1],
    ["2", 2],
    [" 5 trailing", 5],
    ["2147483648", -1],
    ["\u000b7", -1]
  ]) {
    if (raw === null) marquee.removeAttribute("loop");
    else marquee.setAttribute("loop", raw);
    assert(marquee.loop === expected, `loop ${raw}`);
  }

  marquee.loop = 4;
  assert(marquee.loop === 4 && marquee.getAttribute("loop") === "4", "loop positive setter");
  marquee.loop = -1;
  assert(marquee.loop === -1 && marquee.getAttribute("loop") === "-1", "loop -1 setter");
  marquee.setAttribute("loop", "3");
  assert(throws(() => { marquee.loop = 0; }) === "IndexSizeError", "loop zero setter");
  assert(marquee.getAttribute("loop") === "3", "loop zero preserves attribute");
  assert(throws(() => { marquee.loop = -2; }) === "IndexSizeError", "loop negative setter");
  assert(marquee.getAttribute("loop") === "3", "loop negative preserves attribute");
  assert(throws(() => { marquee.loop = Symbol("loop"); }) === "TypeError", "loop symbol setter");
  assert(marquee.getAttribute("loop") === "3", "loop symbol preserves attribute");

  for (const [name, attribute, defaultValue, cases] of [
    ["scrollAmount", "scrollamount", 6, [[null, 6], ["aa", 6], ["-1", 6], ["0", 0], ["10", 10], [" +7tail", 7], ["2147483648", 6]]],
    ["scrollDelay", "scrolldelay", 85, [[null, 85], ["aa", 85], ["-1", 85], ["1", 1], ["100", 100], ["2147483648", 85]]]
  ]) {
    for (const [raw, expected] of cases) {
      if (raw === null) marquee.removeAttribute(attribute);
      else marquee.setAttribute(attribute, raw);
      assert(marquee[name] === expected, `${name} ${raw}`);
    }
    marquee[name] = 12;
    assert(marquee[name] === 12 && marquee.getAttribute(attribute) === "12", `${name} setter`);
    marquee[name] = -1;
    assert(marquee[name] === defaultValue && marquee.getAttribute(attribute) === String(defaultValue), `${name} wrapped setter`);
    marquee.setAttribute(attribute, "14");
    assert(throws(() => { marquee[name] = Symbol(name); }) === "TypeError", `${name} symbol setter`);
    assert(marquee.getAttribute(attribute) === "14", `${name} symbol preserves attribute`);
  }
  return "ok";
})()
"##,
        )
        .expect("detached marquee numeric reflection should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_body_legacy_accessors_use_owner_prototype() {
    let mut vm = new_storage_test_vm("https://detached-body-legacy-prototype.test/");

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

  const names = ["onload", "text", "link", "vLink", "aLink", "background"];
  const bodyOnlyNames = ["text", "link", "vLink", "aLink", "background"];
  for (const name of names) {
    accessor(HTMLBodyElement.prototype, name);
  }
  for (const name of bodyOnlyNames) {
    assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    assert(!(name in document.createElement("div")), `${name} should not be on div`);
  }

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const [body, label] of [[detachedDoc.body, "detached"], [document.createElement("body"), "created"]]) {
    for (const name of names) {
      assert(!own(body, name), `${label}.${name} should not be own before set`);
    }
    const handler = () => `${label}-load`;
    body.onload = handler;
    assert(window.onload === handler, `${label}.onload setter syncs window.onload`);
    assert(body.onload === handler, `${label}.onload getter reads window.onload`);
    body.text = `${label}-text`;
    body.link = `${label}-link`;
    body.vLink = `${label}-vlink`;
    body.aLink = `${label}-alink`;
    body.background = `${label}-background`;
    assert(body.text === `${label}-text` && body.getAttribute("text") === `${label}-text`, `${label}.text`);
    assert(body.link === `${label}-link` && body.getAttribute("link") === `${label}-link`, `${label}.link`);
    assert(body.vLink === `${label}-vlink` && body.getAttribute("vlink") === `${label}-vlink`, `${label}.vLink`);
    assert(body.aLink === `${label}-alink` && body.getAttribute("alink") === `${label}-alink`, `${label}.aLink`);
    assert(body.background === `${label}-background` && body.getAttribute("background") === `${label}-background`, `${label}.background`);
    for (const name of names) {
      assert(!own(body, name), `${label}.${name} should not be own after set`);
      assert(delete body[name], `${label}.${name} delete`);
      assert(!own(body, name), `${label}.${name} should stay inherited`);
    }
    assert(body.onload === handler, `${label}.onload after delete`);
    assert(body.text === `${label}-text`, `${label}.text after delete`);
  }
  window.onload = null;
  return "ok";
})()
"#,
        )
        .expect("detached body legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_global_event_handler_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-global-event-handlers.test/");

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
    return descriptor;
  };

  const click = accessor(HTMLElement.prototype, "onclick");
  accessor(HTMLElement.prototype, "onsubmit");
  assert(HTMLElement.prototype.onclick === null, "HTMLElement.prototype.onclick default");
  assert(Object.getOwnPropertyDescriptor(HTMLBodyElement.prototype, "onload").get !==
    Object.getOwnPropertyDescriptor(HTMLElement.prototype, "onload").get,
    "HTMLBodyElement.onload should keep body/window override");

  const detachedDoc = document.implementation.createHTMLDocument("");
  const detachedDiv = detachedDoc.createElement("div");
  const detachedForm = detachedDoc.createElement("form");
  const createdDiv = document.createElement("div");
  function handler() {}

  for (const [element, name, label] of [
    [detachedDiv, "onclick", "detachedDiv"],
    [detachedForm, "onsubmit", "detachedForm"],
    [createdDiv, "onclick", "createdDiv"],
  ]) {
    assert(!own(element, name), `${label}.${name} should not be own before set`);
    element[name] = handler;
    assert(element[name] === handler, `${label}.${name} handler`);
    assert(!own(element, name), `${label}.${name} should not be own after set`);
    assert(delete element[name], `${label}.${name} delete`);
    assert(!own(element, name), `${label}.${name} should stay inherited`);
    assert(element[name] === handler, `${label}.${name} after delete`);
  }

  assert(click.get.call({}) === null, "forged getter is lenient null");
  return "ok";
})()
"#,
        )
        .expect("detached global event handler owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_media_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-media-prototypes.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const link = doc.createElement("link");
  const source = doc.createElement("source");
  const style = doc.createElement("style");
  const meta = doc.createElement("meta");
  const div = doc.createElement("div");
  doc.head.append(link, style, meta);
  doc.body.append(source, div);

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

  accessor(HTMLLinkElement.prototype, "media");
  accessor(HTMLSourceElement.prototype, "media");
  accessor(HTMLStyleElement.prototype, "media");
  accessor(HTMLMetaElement.prototype, "media");
  assert(!own(HTMLElement.prototype, "media"), "media should not be on HTMLElement.prototype");
  assert(!("media" in div), "media should not be on div");

  for (const [element, label] of [
    [link, "link"],
    [source, "source"],
    [style, "style"],
    [meta, "meta"]
  ]) {
    assert(!own(element, "media"), `${label}.media should not be own before set`);
    element.media = `${label}-media`;
    assert(element.media === `${label}-media`, `${label}.media getter`);
    assert(element.getAttribute("media") === `${label}-media`, `${label}.media attr`);
    assert(!own(element, "media"), `${label}.media should not be own after set`);
    assert(delete element.media, `${label}.media delete`);
    assert(!own(element, "media"), `${label}.media should stay inherited`);
    assert(element.media === `${label}-media`, `${label}.media after delete`);
  }
  return "ok";
})()
"##,
        )
        .expect("detached media owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_media_element_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-html-media-element-prototypes.test/base/");

    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter shape`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const mediaWritable = [
    "src", "volume", "muted", "defaultMuted", "playbackRate", "currentTime",
    "autoplay", "controls", "loop"
  ];
  const mediaReadonly = [
    "paused", "duration", "ended", "seeking", "readyState", "networkState", "textTracks"
  ];
  const videoWritable = ["poster", "width", "height", "playsInline"];
  const videoReadonly = ["videoWidth", "videoHeight"];

  for (const name of mediaWritable) accessor(HTMLMediaElement.prototype, name, true);
  for (const name of mediaReadonly) accessor(HTMLMediaElement.prototype, name, false);
  for (const name of [...mediaWritable, ...mediaReadonly]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    assert(!own(HTMLAudioElement.prototype, name), `${name} should not duplicate on audio`);
    assert(!own(HTMLVideoElement.prototype, name), `${name} should not duplicate on video`);
  }
  for (const name of videoWritable) accessor(HTMLVideoElement.prototype, name, true);
  for (const name of videoReadonly) accessor(HTMLVideoElement.prototype, name, false);
  for (const name of [...videoWritable, ...videoReadonly]) {
    assert(!own(HTMLMediaElement.prototype, name), `${name} should not live on HTMLMediaElement`);
    assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    assert(!own(HTMLAudioElement.prototype, name), `${name} should not live on audio`);
  }

  const detachedDoc = document.implementation.createHTMLDocument("");
  const mediaCases = [
    [document.createElement("audio"), "live-audio"],
    [detachedDoc.createElement("audio"), "detached-audio"],
    [document.createElement("video"), "live-video"],
    [detachedDoc.createElement("video"), "detached-video"]
  ];

  for (const [element, label] of mediaCases) {
    for (const name of [...mediaWritable, ...mediaReadonly]) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
    assert(element.paused === true, `${label}.paused default`);
    assert(element.volume === 1, `${label}.volume default`);
    assert(element.muted === false, `${label}.muted default`);
    assert(Number.isNaN(element.duration), `${label}.duration default`);
    assert(element.ended === false, `${label}.ended default`);
    assert(typeof element.seeking === "boolean", `${label}.seeking default`);
    assert(element.readyState === element.HAVE_NOTHING, `${label}.readyState default`);
    assert(element.networkState === element.NETWORK_EMPTY, `${label}.networkState default`);
    assert(element.textTracks === element.textTracks, `${label}.textTracks cache`);

    element.src = `${label}.mp4`;
    element.volume = 0.25;
    element.muted = true;
    element.defaultMuted = true;
    element.playbackRate = 1.5;
    element.currentTime = 12.25;
    element.autoplay = true;
    element.controls = true;
    element.loop = true;

    assert(element.src.includes(`${label}.mp4`), `${label}.src getter`);
    assert(Math.abs(element.volume - 0.25) < 0.0001, `${label}.volume set`);
    assert(element.muted === true, `${label}.muted set`);
    assert(element.defaultMuted === true && element.hasAttribute("muted"), `${label}.defaultMuted set`);
    assert(Math.abs(element.playbackRate - 1.5) < 0.0001, `${label}.playbackRate set`);
    assert(Math.abs(element.currentTime - 12.25) < 0.0001, `${label}.currentTime set`);
    assert(element.autoplay === true && element.hasAttribute("autoplay"), `${label}.autoplay set`);
    assert(element.controls === true && element.hasAttribute("controls"), `${label}.controls set`);
    assert(element.loop === true && element.hasAttribute("loop"), `${label}.loop set`);

    for (const name of [...mediaWritable, ...mediaReadonly]) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
    assert(element.src.includes(`${label}.mp4`), `${label}.src after delete`);
    assert(element.muted === true, `${label}.muted after delete`);
    assert(element.defaultMuted === true, `${label}.defaultMuted after delete`);
  }

  for (const [video, label] of [
    [document.createElement("video"), "live-video-only"],
    [detachedDoc.createElement("video"), "detached-video-only"]
  ]) {
    for (const name of [...videoWritable, ...videoReadonly]) {
      assert(!own(video, name), `${label}.${name} should not be own before set`);
    }
    video.poster = `${label}.png`;
    video.width = 320;
    video.height = 180;
    video.playsInline = true;
    assert(video.poster.includes(`${label}.png`), `${label}.poster getter`);
    assert(video.width === 320 && video.getAttribute("width") === "320", `${label}.width set`);
    assert(video.height === 180 && video.getAttribute("height") === "180", `${label}.height set`);
    assert(video.playsInline === true && video.hasAttribute("playsinline"), `${label}.playsInline set`);
    assert(video.videoWidth === 0, `${label}.videoWidth default`);
    assert(video.videoHeight === 0, `${label}.videoHeight default`);
    for (const name of [...videoWritable, ...videoReadonly]) {
      assert(!own(video, name), `${label}.${name} should not be own after set`);
      assert(delete video[name], `${label}.${name} delete`);
      assert(!own(video, name), `${label}.${name} should stay inherited`);
    }
    assert(video.width === 320 && video.height === 180, `${label}.dimensions after delete`);
    assert(video.playsInline === true, `${label}.playsInline after delete`);
  }

  return "ok";
})()
"##,
        )
        .expect("detached HTML media element owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_media_element_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-media-receiver-brand.test/base/");

    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const doc = document.implementation.createHTMLDocument("");
  const audio = doc.createElement("audio");
  const video = doc.createElement("video");
  const div = doc.createElement("div");
  const img = doc.createElement("img");
  const source = doc.createElement("source");
  const text = doc.createTextNode("x");
  const mediaBadReceivers = [{}, text, div, img, source];
  const videoBadReceivers = [{}, text, div, img, source, audio];

  const mediaValues = {
    crossOrigin: "anonymous",
    loading: "lazy",
    preload: "metadata",
    src: "clip.mp4",
    volume: 0.25,
    muted: true,
    defaultMuted: true,
    playbackRate: 1.5,
    currentTime: 2,
    autoplay: true,
    controls: true,
    loop: true
  };
  const mediaNames = [
    "crossOrigin", "loading", "preload", "src", "volume", "muted", "defaultMuted",
    "playbackRate", "currentTime", "paused", "duration", "ended", "seeking",
    "readyState", "networkState", "textTracks", "autoplay", "controls", "loop"
  ];
  for (const name of mediaNames) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get.call(audio) !== "undefined", `${name} audio getter`);
    assert(typeof descriptor.get.call(video) !== "undefined", `${name} video getter`);
    if (typeof descriptor.set === "function") {
      descriptor.set.call(audio, mediaValues[name]);
      descriptor.set.call(video, mediaValues[name]);
      assert(!Object.prototype.hasOwnProperty.call(audio, name), `${name} audio inherited`);
      assert(!Object.prototype.hasOwnProperty.call(video, name), `${name} video inherited`);
    }
    for (const receiver of mediaBadReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      if (typeof descriptor.set === "function") {
        assert(throwsTypeError(() => descriptor.set.call(receiver, mediaValues[name])), `${name} setter receiver`);
      }
    }
  }

  const mediaMethods = {
    play: [],
    pause: [],
    load: [],
    canPlayType: ["audio/mpeg"],
    addTextTrack: ["subtitles"]
  };
  for (const [name, args] of Object.entries(mediaMethods)) {
    const method = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, name).value;
    assert(typeof method === "function", `${name} method`);
    method.call(audio, ...args);
    method.call(video, ...args);
    for (const receiver of mediaBadReceivers) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} method receiver`);
    }
  }

  const videoValues = {
    poster: "poster.png",
    width: 320,
    height: 180,
    playsInline: true
  };
  for (const name of ["poster", "width", "height", "playsInline", "videoWidth", "videoHeight"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLVideoElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get.call(video) !== "undefined", `${name} getter`);
    if (typeof descriptor.set === "function") {
      descriptor.set.call(video, videoValues[name]);
      assert(!Object.prototype.hasOwnProperty.call(video, name), `${name} inherited`);
    }
    for (const receiver of videoBadReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      if (typeof descriptor.set === "function") {
        assert(throwsTypeError(() => descriptor.set.call(receiver, videoValues[name])), `${name} setter receiver`);
      }
    }
  }
  return "ok";
})()
"##,
        )
        .expect("detached HTML media element receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_text_reflection_uses_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-text-reflection.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const cases = [
    ["a", HTMLAnchorElement.prototype, "anchor old", "anchor new"],
    ["title", HTMLTitleElement.prototype, "title old", "title new"],
    ["option", HTMLOptionElement.prototype, "option old", "option new"]
  ];
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  for (const [tag, prototype, oldText, newText] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "text");
    assert(!!descriptor, `${tag} text descriptor missing`);
    assert(typeof descriptor.get === "function", `${tag} text getter`);
    assert(typeof descriptor.set === "function", `${tag} text setter`);
    assert(descriptor.enumerable === true, `${tag} text enumerable`);
    assert(descriptor.configurable === true, `${tag} text configurable`);

    const element = doc.createElement(tag);
    element.textContent = oldText;
    assert(!own(element, "text"), `${tag} text should not be own initially`);
    assert(element.text === oldText, `${tag} text getter`);
    element.text = newText;
    assert(element.textContent === newText, `${tag} text setter content`);
    assert(element.text === newText, `${tag} text setter getter`);
    assert(!own(element, "text"), `${tag} text should stay inherited after set`);
    assert(delete element.text, `${tag} delete text`);
    assert(!own(element, "text"), `${tag} text should stay inherited after delete`);
    element.text = oldText;
    assert(element.text === oldText, `${tag} text after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached text reflection prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_specialized_url_resource_properties_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-specialized-url-resource.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const anchor = doc.createElement("a");
  const area = doc.createElement("area");
  const image = doc.createElement("img");
  const iframe = doc.createElement("iframe");
  doc.body.append(anchor, area, image, iframe);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name, length) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.value.length === length, `${name} length`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const anchorNames = [
    "href",
    "protocol",
    "host",
    "hostname",
    "port",
    "pathname",
    "search",
    "hash"
  ];
  method(HTMLAnchorElement.prototype, "toString", 0);
  assert(!own(anchor, "toString"), "anchor toString should not be own");
  for (const name of anchorNames) {
    accessor(HTMLAnchorElement.prototype, name, true);
    assert(!own(anchor, name), `anchor ${name} should not be own`);
    accessor(HTMLAreaElement.prototype, name, true);
    assert(!own(area, name), `area ${name} should not be own`);
  }
  for (const name of ["src", "srcset"]) {
    accessor(HTMLImageElement.prototype, name, true);
    assert(!own(image, name), `image ${name} should not be own`);
  }
  method(HTMLImageElement.prototype, "decode", 0);
  assert(!own(image, "decode"), "image decode should not be own");
  accessor(HTMLIFrameElement.prototype, "src", true);
  accessor(HTMLIFrameElement.prototype, "srcdoc", true);
  accessor(HTMLIFrameElement.prototype, "contentDocument", false);
  accessor(HTMLIFrameElement.prototype, "contentWindow", false);
  for (const name of ["src", "srcdoc", "contentDocument", "contentWindow"]) {
    assert(!own(iframe, name), `iframe ${name} should not be own`);
  }

  anchor.href = "https://old.test/base/path?x=1#old";
  anchor.protocol = "http";
  anchor.host = "example.test:8080";
  anchor.pathname = "next";
  anchor.search = "q=2";
  anchor.hash = "done";
  assert(anchor.href === "http://example.test:8080/next?q=2#done", "anchor href mutation");
  assert(anchor.protocol === "http:", "anchor protocol");
  assert(anchor.host === "example.test:8080", "anchor host");
  assert(anchor.hostname === "example.test", "anchor hostname");
  assert(anchor.port === "8080", "anchor port");
  assert(anchor.pathname === "/next", "anchor pathname");
  assert(anchor.search === "?q=2", "anchor search");
  assert(anchor.hash === "#done", "anchor hash");
  for (const name of anchorNames) {
    assert(delete anchor[name], `delete anchor ${name}`);
    assert(!own(anchor, name), `anchor ${name} should stay inherited`);
  }
  assert(anchor.href === "http://example.test:8080/next?q=2#done", "anchor href after delete");
  assert(anchor.toString() === anchor.href, "anchor toString after delete");
  assert(HTMLAnchorElement.prototype.toString.call(anchor) === anchor.href, "anchor toString descriptor call");
  assert(delete anchor.toString, "delete anchor toString");
  assert(!own(anchor, "toString"), "anchor toString should stay inherited");
  assert(anchor.toString() === anchor.href, "anchor toString after toString delete");
  area.href = "https://area.test/map";
  assert(area.href === "https://area.test/map", "area href reflection");
  assert(delete area.href, "delete area href");
  assert(!own(area, "href"), "area href should stay inherited");
  assert(area.href === "https://area.test/map", "area href after delete");

  image.src = "https://cdn.test/image.png";
  image.srcset = "small.png 1x, large.png 2x";
  assert(image.src === "https://cdn.test/image.png", "image src reflection");
  assert(image.srcset === "small.png 1x, large.png 2x", "image srcset reflection");
  for (const name of ["src", "srcset"]) {
    assert(delete image[name], `delete image ${name}`);
    assert(!own(image, name), `image ${name} should stay inherited`);
  }
  assert(image.src === "https://cdn.test/image.png", "image src after delete");
  assert(image.srcset === "small.png 1x, large.png 2x", "image srcset after delete");
  assert(typeof image.decode().then === "function", "image decode behavior");

  iframe.src = "https://frame.test/initial.html";
  assert(iframe.src === "https://frame.test/initial.html", "iframe src reflection");
  iframe.srcdoc = "<!doctype html><body><p id='marker'>first</p></body>";
  const firstDocument = iframe.contentDocument;
  const firstWindow = iframe.contentWindow;
  assert(firstDocument.body.textContent.trim() === "first", "first srcdoc document");
  assert(firstWindow.document === firstDocument, "first contentWindow document");
  iframe.srcdoc = "<!doctype html><body><p id='marker'>second</p></body>";
  const secondDocument = iframe.contentDocument;
  assert(secondDocument.body.textContent.trim() === "second", "srcdoc clears cached document");
  assert(secondDocument !== firstDocument, "srcdoc replacement materializes new document");
  for (const name of ["src", "srcdoc", "contentDocument", "contentWindow"]) {
    assert(delete iframe[name], `delete iframe ${name}`);
    assert(!own(iframe, name), `iframe ${name} should stay inherited`);
  }
  assert(iframe.contentDocument.body.textContent.trim() === "second", "iframe after delete");

  return "ok";
})()
"##,
        )
        .expect("detached specialized URL/resource prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn hyperlink_stringifiers_use_native_href_and_enforce_owner_brand() {
    let mut vm = new_storage_test_vm("https://hyperlink-stringifier.test/base/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const detachedDocument = document.implementation.createHTMLDocument("");
  const cases = [
    [HTMLAnchorElement.prototype, "a", document.createElement("a"), detachedDocument.createElement("a")],
    [HTMLAreaElement.prototype, "area", document.createElement("area"), detachedDocument.createElement("area")]
  ];

  for (const [prototype, label, live, detached] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "toString");
    assert(!!descriptor, `${label} toString descriptor`);
    assert(typeof descriptor.value === "function", `${label} toString method`);
    assert(descriptor.value.length === 0, `${label} toString length`);

    for (const [element, suffix] of [[live, "live"], [detached, "detached"]]) {
      const expected = `https://example.test/${label}/${suffix}`;
      element.setAttribute("href", expected);
      assert(descriptor.value.call(element) === expected, `${label} ${suffix} value`);
      Object.defineProperty(element, "href", {
        configurable: true,
        get() { throw new Error("stringifier read the JavaScript href property"); }
      });
      assert(descriptor.value.call(element) === expected, `${label} ${suffix} shadowed href`);
    }
  }

  const anchorToString = HTMLAnchorElement.prototype.toString;
  const areaToString = HTMLAreaElement.prototype.toString;
  const invalidReceivers = [null, undefined, {}, window, document.createElement("div")];
  for (const receiver of invalidReceivers) {
    assert(throwsTypeError(() => anchorToString.call(receiver)), "anchor invalid receiver");
    assert(throwsTypeError(() => areaToString.call(receiver)), "area invalid receiver");
  }
  assert(throwsTypeError(() => anchorToString.call(document.createElement("area"))), "anchor rejects area");
  assert(throwsTypeError(() => areaToString.call(document.createElement("a"))), "area rejects anchor");
  return "ok";
})()
"#,
        )
        .expect("hyperlink stringifiers should enforce their owner interface");

    assert_eq!(result, "ok");
}

#[test]
fn detached_canvas_image_state_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-canvas-image-resource.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLCanvasElement.prototype, "width", true);
  accessor(HTMLCanvasElement.prototype, "height", true);
  accessor(HTMLImageElement.prototype, "width", true);
  accessor(HTMLImageElement.prototype, "height", true);
  accessor(HTMLImageElement.prototype, "naturalWidth", false);
  accessor(HTMLImageElement.prototype, "naturalHeight", false);
  accessor(HTMLImageElement.prototype, "isMap", true);
  accessor(HTMLImageElement.prototype, "complete", false);
  accessor(HTMLImageElement.prototype, "currentSrc", false);
  for (const name of ["width", "height", "naturalWidth", "naturalHeight", "isMap", "complete", "currentSrc"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }

  const parsed = new DOMParser().parseFromString(
    "<!doctype html><html><body><canvas></canvas><img></body></html>",
    "text/html"
  );
  const liveCanvas = document.createElement("canvas");
  const detachedCanvas = parsed.querySelector("canvas");
  const liveImage = document.createElement("img");
  const detachedImage = parsed.querySelector("img");
  const canvasWidth = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "width");
  const canvasHeight = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, "height");

  for (const canvas of [liveCanvas, detachedCanvas]) {
    for (const name of ["width", "height"]) {
      assert(!own(canvas, name), `canvas ${name} should not be own before set`);
    }
    canvasWidth.set.call(canvas, 640);
    canvasHeight.set.call(canvas, 480);
    assert(canvasWidth.get.call(canvas) === 640, "canvas width value");
    assert(canvasHeight.get.call(canvas) === 480, "canvas height value");
    assert(canvas.getAttribute("width") === "640", "canvas width attr");
    assert(canvas.getAttribute("height") === "480", "canvas height attr");
    assert(!own(canvas, "width"), "canvas width should stay inherited after set");
    assert(!own(canvas, "height"), "canvas height should stay inherited after set");
    const context = HTMLCanvasElement.prototype.getContext.call(canvas, "2d");
    assert(Object.prototype.toString.call(context) === "[object CanvasRenderingContext2D]", "canvas context");
    assert(HTMLCanvasElement.prototype.toDataURL.call(canvas).startsWith("data:image/png;base64,"), "canvas data URL");
    const offscreen = HTMLCanvasElement.prototype.transferControlToOffscreen.call(canvas);
    assert(offscreen instanceof OffscreenCanvas, "canvas offscreen instance");
    assert(offscreen.width === 640, "canvas offscreen width");
    assert(offscreen.height === 480, "canvas offscreen height");
    assert(delete canvas.width, "canvas width delete");
    assert(delete canvas.height, "canvas height delete");
    assert(canvas.width === 640, "canvas width after delete");
    assert(canvas.height === 480, "canvas height after delete");
  }

  for (const image of [liveImage, detachedImage]) {
    for (const name of ["width", "height", "naturalWidth", "naturalHeight", "isMap", "complete", "currentSrc"]) {
      assert(!own(image, name), `image ${name} should not be own before set`);
    }
    image.width = 33;
    image.height = 44;
    image.isMap = true;
    assert(image.width === 33, "image width value");
    assert(image.height === 44, "image height value");
    assert(image.naturalWidth === 0, "image naturalWidth default");
    assert(image.naturalHeight === 0, "image naturalHeight default");
    assert(image.isMap === true, "image isMap value");
    assert(image.complete === true, "image complete without source");
    assert(image.currentSrc === "", "image currentSrc without source");
    assert(image.getAttribute("width") === "33", "image width attr");
    assert(image.getAttribute("height") === "44", "image height attr");
    assert(image.getAttribute("ismap") === "", "image ismap attr");
    for (const name of ["width", "height", "naturalWidth", "naturalHeight", "isMap", "complete", "currentSrc"]) {
      assert(!own(image, name), `image ${name} should stay inherited after mutation`);
      assert(delete image[name], `image ${name} delete`);
      assert(!own(image, name), `image ${name} should stay inherited after delete`);
    }
    assert(image.width === 33, "image width after delete");
    assert(image.height === 44, "image height after delete");
    assert(image.isMap === true, "image isMap after delete");
  }
  return "ok";
})()
"#,
        )
        .expect("canvas and image state prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_resource_template_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-resource-template.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter = true) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  for (const name of ["type", "media", "blocking", "disabled"]) {
    accessor(HTMLStyleElement.prototype, name);
  }
  accessor(HTMLLinkElement.prototype, "disabled");
  accessor(HTMLIFrameElement.prototype, "sandbox");
  accessor(HTMLIFrameElement.prototype, "allowFullscreen");
  for (const name of ["default", "kind", "src", "srclang", "label"]) {
    accessor(HTMLTrackElement.prototype, name);
  }
  accessor(HTMLTrackElement.prototype, "readyState", false);
  accessor(HTMLTrackElement.prototype, "track", false);

  const div = document.createElement("div");
  for (const name of ["blocking", "sandbox", "allowFullscreen", "default", "srclang", "readyState", "track"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }

  const detachedDocument = document.implementation.createHTMLDocument("");
  const styleElements = [document.createElement("style"), detachedDocument.createElement("style")];
  const linkElements = [document.createElement("link"), detachedDocument.createElement("link")];
  const iframeElements = [document.createElement("iframe"), detachedDocument.createElement("iframe")];
  const trackElements = [document.createElement("track"), detachedDocument.createElement("track")];

  for (const style of styleElements) {
    for (const name of ["type", "media", "blocking", "disabled"]) {
      assert(!own(style, name), `style.${name} should not be own before set`);
    }
    assert(style.type === "", "style type default");
    style.type = "text/less";
    style.media = "print";
    style.blocking = "render";
    style.disabled = true;
    assert(style.type === "text/less" && style.getAttribute("type") === "text/less", "style type");
    assert(style.media === "print" && style.getAttribute("media") === "print", "style media");
    assert(style.blocking === "render" && style.getAttribute("blocking") === "render", "style blocking");
    assert(typeof style.disabled === "boolean", "style disabled boolean");
    for (const name of ["type", "media", "blocking", "disabled"]) {
      assert(!own(style, name), `style.${name} should stay inherited after set`);
      assert(delete style[name], `style.${name} delete`);
      assert(!own(style, name), `style.${name} should stay inherited after delete`);
    }
    assert(style.type === "text/less", "style type after delete");
    assert(style.media === "print", "style media after delete");
    assert(style.blocking === "render", "style blocking after delete");
  }

  for (const link of linkElements) {
    assert(!own(link, "disabled"), "link.disabled should not be own before set");
    link.disabled = true;
    assert(link.disabled === true, "link disabled true");
    assert(link.getAttribute("disabled") === "", "link disabled attr");
    assert(!own(link, "disabled"), "link.disabled should stay inherited after true");
    link.disabled = false;
    assert(link.disabled === false, "link disabled false");
    assert(link.getAttribute("disabled") === null, "link disabled attr removed");
    assert(!own(link, "disabled"), "link.disabled should stay inherited after false");
    assert(delete link.disabled, "link.disabled delete");
    assert(!own(link, "disabled"), "link.disabled should stay inherited after delete");
  }

  for (const iframe of iframeElements) {
    for (const name of ["sandbox", "allowFullscreen"]) {
      assert(!own(iframe, name), `iframe.${name} should not be own before set`);
    }
    iframe.sandbox = "allow-scripts";
    iframe.allowFullscreen = true;
    assert(iframe.sandbox === "allow-scripts" && iframe.getAttribute("sandbox") === "allow-scripts", "iframe sandbox");
    assert(iframe.allowFullscreen === true && iframe.getAttribute("allowfullscreen") === "", "iframe allowFullscreen");
    for (const name of ["sandbox", "allowFullscreen"]) {
      assert(!own(iframe, name), `iframe.${name} should stay inherited after set`);
      assert(delete iframe[name], `iframe.${name} delete`);
      assert(!own(iframe, name), `iframe.${name} should stay inherited after delete`);
    }
    assert(iframe.sandbox === "allow-scripts", "iframe sandbox after delete");
    assert(iframe.allowFullscreen === true, "iframe allowFullscreen after delete");
  }

  for (const track of trackElements) {
    for (const name of ["default", "kind", "src", "srclang", "label", "readyState", "track"]) {
      assert(!own(track, name), `track.${name} should not be own before set`);
    }
    track.default = true;
    track.kind = "CAPTIONS";
    track.src = "captions.vtt";
    track.srclang = "en";
    track.label = "English";
    assert(track.default === true && track.getAttribute("default") === "", "track default");
    assert(track.kind === "captions" && track.getAttribute("kind") === "CAPTIONS", "track kind");
    assert(track.src.includes("captions.vtt") && track.getAttribute("src") === "captions.vtt", "track src");
    assert(track.srclang === "en" && track.getAttribute("srclang") === "en", "track srclang");
    assert(track.label === "English" && track.getAttribute("label") === "English", "track label");
    assert(track.readyState === 0, "track readyState default");
    assert(track.track && track.track.kind === "captions", "track TextTrack kind");
    for (const name of ["default", "kind", "src", "srclang", "label", "readyState", "track"]) {
      assert(!own(track, name), `track.${name} should stay inherited after set`);
      assert(delete track[name], `track.${name} delete`);
      assert(!own(track, name), `track.${name} should stay inherited after delete`);
    }
    assert(track.default === true, "track default after delete");
    assert(track.kind === "captions", "track kind after delete");
    assert(track.readyState === 0, "track readyState after delete");
    assert(track.track && track.track.kind === "captions", "track TextTrack after delete");
  }
  return "ok";
})()
"##,
        )
        .expect("detached resource template owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_node_tree_accessors_use_standard_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-node-tree-accessors.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const first = doc.createElement("section");
  const spacer = doc.createTextNode("gap");
  const second = doc.createElement("article");
  const text = doc.createTextNode("alpha");
  const doctype = doc.implementation.createDocumentType("html", "", "");
  first.appendChild(text);
  doc.body.append(first, spacer, second);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, setter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === setter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const nodeNames = [
    "nodeType",
    "nodeName",
    "parentNode",
    "parentElement",
    "ownerDocument",
    "childNodes",
    "firstChild",
    "lastChild",
    "previousSibling",
    "nextSibling",
    "isConnected"
  ];
  for (const name of nodeNames) {
    accessor(Node.prototype, name, "undefined");
    for (const object of [doc, doctype, doc.body, first, spacer, second, text]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }

  const parentNames = ["children", "firstElementChild", "lastElementChild", "childElementCount"];
  for (const name of parentNames) {
    accessor(Element.prototype, name, "undefined");
    for (const object of [doc, doc.body, first]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }

  const siblingNames = ["previousElementSibling", "nextElementSibling"];
  for (const name of siblingNames) {
    accessor(Element.prototype, name, "undefined");
    accessor(CharacterData.prototype, name, "undefined");
    for (const object of [first, spacer, second, text]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }

  const nodeValueDescriptor = accessor(Node.prototype, "nodeValue", "function");
  const textContentDescriptor = accessor(Node.prototype, "textContent", "function");
  for (const name of ["nodeValue", "textContent"]) {
    for (const object of [doc, doctype, doc.body, first, spacer, second, text]) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }
  assert(nodeValueDescriptor.get.call(text) === "alpha", "nodeValue prototype getter");
  assert(textContentDescriptor.get.call(first) === "alpha", "textContent prototype getter");

  assert(first.nodeType === 1, "element nodeType");
  assert(first.nodeName === "SECTION", "element nodeName");
  assert(text.nodeType === 3, "text nodeType");
  assert(text.nodeName === "#text", "text nodeName");
  assert(doctype.nodeType === 10, "doctype nodeType");
  assert(doctype.nodeName === "html", "doctype nodeName");
  assert(doctype.nodeValue === null, "doctype nodeValue");
  assert(doctype.parentNode === null, "doctype parentNode");
  assert(doctype.ownerDocument === doc, "doctype ownerDocument");
  assert(doc.ownerDocument === null, "document ownerDocument");
  assert(first.ownerDocument === doc, "element ownerDocument");
  assert(first.parentNode === doc.body, "parentNode");
  assert(first.parentElement === doc.body, "parentElement");
  assert(first.childNodes.length === 1, "childNodes length");
  assert(first.firstChild === text, "firstChild");
  assert(first.lastChild === text, "lastChild");
  assert(first.nextSibling === spacer, "nextSibling");
  assert(spacer.previousSibling === first, "previousSibling");
  assert(doc.body.children.length === 2, "children length");
  assert(doc.body.firstElementChild === first, "firstElementChild");
  assert(doc.body.lastElementChild === second, "lastElementChild");
  assert(doc.body.childElementCount === 2, "childElementCount");
  assert(spacer.previousElementSibling === first, "previousElementSibling");
  assert(spacer.nextElementSibling === second, "nextElementSibling");
  nodeValueDescriptor.set.call(text, "beta");
  assert(text.nodeValue === "beta", "nodeValue prototype setter");
  assert(first.textContent === "beta", "textContent after nodeValue setter");
  nodeValueDescriptor.set.call(text, null);
  assert(text.nodeValue === "", "nodeValue null setter");
  textContentDescriptor.set.call(first, "gamma");
  assert(first.textContent === "gamma", "textContent prototype setter");
  assert(first.childNodes.length === 1, "textContent replacement child count");
  assert(first.firstChild.nodeType === 3, "textContent replacement child type");
  assert(!own(first, "textContent"), "textContent should stay inherited after set");
  assert(!own(first.firstChild, "nodeValue"), "nodeValue should stay inherited after replacement");
  nodeValueDescriptor.set.call(first, "ignored");
  assert(first.nodeValue === null, "element nodeValue setter ignored");
  textContentDescriptor.set.call(doctype, "ignored");
  textContentDescriptor.set.call(doc, "ignored");
  assert(doctype.textContent === null, "doctype textContent setter ignored");
  assert(doc.textContent === null, "document textContent setter ignored");
  assert(delete first.nodeType, "delete inherited nodeType");
  assert(first.nodeType === 1, "nodeType after delete");

  return "ok";
})()
"##,
        )
        .expect("detached Node and DOM mixin prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_pointer_capture_methods_use_element_prototype_no_frame_behavior() {
    let mut vm = new_storage_test_vm("https://detached-pointer-capture.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const target = doc.createElement("div");
  doc.body.appendChild(target);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const method = (name, length) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} value`);
    assert(descriptor.value.length === length, `${name} length`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor.value;
  };
  const outcome = (callback) => {
    try {
      const value = callback();
      return `OK:${value === undefined ? "undefined" : String(value)}`;
    } catch (error) {
      return `ERR:${error.name}:${error.code || ""}`;
    }
  };

  const set = method("setPointerCapture", 1);
  const release = method("releasePointerCapture", 1);
  const has = method("hasPointerCapture", 1);
  for (const name of ["setPointerCapture", "releasePointerCapture", "hasPointerCapture"]) {
    assert(!own(target, name), `${name} should not be own`);
    assert(!own(doc.body, name), `${name} should not be own on body`);
  }

  return [
    outcome(() => target.setPointerCapture(1)),
    outcome(() => target.releasePointerCapture(1)),
    outcome(() => target.hasPointerCapture(1)),
    outcome(() => set.call(target, 1)),
    outcome(() => release.call(target, 1)),
    outcome(() => has.call(target, 1))
  ].join("|");
})()
"#,
        )
        .expect("detached pointer capture prototype methods should evaluate");

    assert_eq!(
        result,
        "OK:undefined|OK:undefined|OK:false|OK:undefined|OK:undefined|OK:false"
    );
}

#[test]
fn detached_label_accessors_use_html_label_element_prototype() {
    let mut vm = new_storage_test_vm("https://detached-label-accessors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const form = doc.createElement("form");
  const explicitLabel = doc.createElement("label");
  const explicitInput = doc.createElement("input");
  const implicitLabel = doc.createElement("label");
  const implicitInput = doc.createElement("textarea");
  explicitInput.id = "target";
  explicitLabel.htmlFor = "target";
  implicitLabel.append("implicit", implicitInput);
  form.append(explicitLabel, explicitInput, implicitLabel);
  doc.body.append(form);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLLabelElement.prototype, "htmlFor", true);
  accessor(HTMLLabelElement.prototype, "control", false);
  accessor(HTMLLabelElement.prototype, "form", false);
  for (const label of [explicitLabel, implicitLabel]) {
    assert(!own(label, "htmlFor"), "htmlFor should not be own");
    assert(!own(label, "control"), "control should not be own");
    assert(!own(label, "form"), "form should not be own");
  }

  assert(explicitLabel.htmlFor === "target", "htmlFor reflection");
  assert(explicitLabel.control === explicitInput, "explicit control");
  assert(implicitLabel.control === implicitInput, "implicit control");
  assert(explicitLabel.form === form, "explicit form");
  assert(implicitLabel.form === form, "implicit form");
  assert(delete explicitLabel.htmlFor, "delete htmlFor");
  assert(delete explicitLabel.control, "delete control");
  assert(delete explicitLabel.form, "delete form");
  explicitLabel.htmlFor = "target";
  explicitLabel.control = null;
  explicitLabel.form = null;
  assert(!own(explicitLabel, "htmlFor"), "htmlFor should stay inherited");
  assert(!own(explicitLabel, "control"), "control should stay inherited");
  assert(!own(explicitLabel, "form"), "form should stay inherited");
  assert(explicitLabel.htmlFor === "target", "htmlFor after assignment");
  assert(explicitLabel.control === explicitInput, "control after assignment");
  assert(explicitLabel.form === form, "form after assignment");
  return "ok";
})()
"#,
        )
        .expect("detached label prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_document_title_getter_and_setter_walk_full_tree() {
    let mut vm = new_storage_test_vm("https://detached-title.test/");
    let result = vm
        .eval(
            r#"
(() => {
  // createHTMLDocument flows through the detached_surface accessor path that
  // exposes a real `title` getter/setter (unlike DOMParser snapshots, which
  // expose `title` as a plain own-property and are tracked separately).
  const doc = document.implementation.createHTMLDocument('ORIG');
  const out = [];
  out.push('initial=' + doc.title);
  doc.title = 'UPDATED';
  out.push('updated=' + doc.title);
  // Append a <title> directly under <body>; the head-side title still wins
  // because it is first in tree order.
  const bodyTitle = doc.createElement('title');
  bodyTitle.appendChild(doc.createTextNode('FROM_BODY'));
  doc.body.appendChild(bodyTitle);
  out.push('headStillWins=' + doc.title);
  // Remove the head; the body-side <title> is now the first title in tree
  // order, so the setter should overwrite that existing element.
  const head = doc.getElementsByTagName('head')[0];
  if (head) head.parentNode.removeChild(head);
  doc.title = 'REPLACED_BODY';
  out.push('replacedBody=' + doc.title);
  // Now drop every title element. Setter has no title and no head → no-op.
  const titles = Array.from(doc.getElementsByTagName('title'));
  for (const t of titles) t.parentNode.removeChild(t);
  doc.title = 'SHOULD_NOT_APPLY';
  out.push('afterHeadGone=' + doc.title);
  return out.join('|');
})()
"#,
        )
        .expect("detached document.title spec behavior should evaluate");
    assert_eq!(
        result,
        "initial=ORIG|updated=UPDATED|headStillWins=UPDATED|replacedBody=REPLACED_BODY|afterHeadGone="
    );
}

#[test]
fn detached_native_remove_dispatches_through_runtime_pipeline() {
    let mut vm = new_storage_test_vm("https://detached-remove-runtime-pipeline.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  window.detachedRemovePipelineEvents = [];
  class DetachedRemovePipelineElement extends HTMLElement {
    disconnectedCallback() {
      window.detachedRemovePipelineEvents.push([
        this.isConnected,
        this.parentNode === null,
        doc.body.childNodes.length
      ].join(":"));
    }
  }
  customElements.define("detached-remove-pipeline", DetachedRemovePipelineElement);
  const element = document.createElement("detached-remove-pipeline");
  doc.body.appendChild(element);
  window.detachedRemovePipelineEvents.length = 0;
  doc.body.removeChild(element);
  return [
    JSON.stringify(window.detachedRemovePipelineEvents),
    element.parentNode === null,
    doc.body.childNodes.length
  ].join("|");
})()
"#,
        )
        .expect("detached native remove runtime pipeline timing should evaluate");

    assert_eq!(result, r#"["false:true:0"]|true|0"#);
}

#[test]
fn detached_document_state_accessors_are_declared_on_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-document-state-accessors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const html = parser.parseFromString("<html><body></body></html>", "text/html");
  const xml = parser.parseFromString("<root></root>", "application/xml");
  const htmlProto = Object.getPrototypeOf(html);
  const xmlProto = Object.getPrototypeOf(xml);
  const descriptorOwner = (object, name) => {
    for (let current = object; current; current = Object.getPrototypeOf(current)) {
      if (Object.prototype.hasOwnProperty.call(current, name)) {
        return current;
      }
    }
    return null;
  };
  const shape = (object, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(descriptorOwner(object, name), name);
    return [
      typeof descriptor.get,
      descriptor.get.name,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(",");
  };
  const valueShape = (object, name, expected) => {
    const descriptor = Object.getOwnPropertyDescriptor(object, name);
    return [
      descriptor.value === expected,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ].join(",");
  };
  const htmlKeysBefore = Object.keys(html)
    .filter(name => name === "implementation" || name === "fonts" || name === "location")
    .join(",");
  const xmlKeysBefore = Object.keys(xml)
    .filter(name => name === "implementation" || name === "fonts" || name === "location")
    .join(",");
  const htmlFontsShape = shape(htmlProto, "fonts");
  const xmlFontsShape = shape(xmlProto, "fonts");
  const htmlImplementationShape = shape(htmlProto, "implementation");
  const xmlImplementationShape = shape(xmlProto, "implementation");
  const htmlLocationShape = valueShape(html, "location", null);
  const xmlLocationShape = valueShape(xml, "location", null);
  const htmlImplementationCacheBefore = Object.getOwnPropertyDescriptor(html, "implementation") === undefined;
  const xmlImplementationCacheBefore = Object.getOwnPropertyDescriptor(xml, "implementation") === undefined;
  const htmlImplementation = html.implementation;
  const xmlImplementation = xml.implementation;
  const htmlFonts = html.fonts;
  const xmlFonts = xml.fonts;
  const htmlImplementationCache = Object.getOwnPropertyDescriptor(html, "implementation") === undefined;
  const xmlImplementationCache = Object.getOwnPropertyDescriptor(xml, "implementation") === undefined;

  html.implementation = { marker: "html" };
  xml.implementation = { marker: "xml" };
  html.fonts = { marker: "html-fonts" };
  xml.fonts = { marker: "xml-fonts" };
  html.location = { marker: "html-location" };
  xml.location = { marker: "xml-location" };
  const prototypeSurface = [
    Object.prototype.hasOwnProperty.call(html, "createElement"),
    Object.prototype.hasOwnProperty.call(html, "querySelector"),
    Object.prototype.hasOwnProperty.call(html, "getElementById"),
    Object.prototype.hasOwnProperty.call(html, "fonts"),
    Object.prototype.hasOwnProperty.call(html, "implementation"),
    Object.prototype.hasOwnProperty.call(htmlProto, "createElement"),
    Object.prototype.hasOwnProperty.call(htmlProto, "querySelector"),
    Object.prototype.hasOwnProperty.call(htmlProto, "fonts"),
    descriptorOwner(htmlProto, "fonts") === Document.prototype,
    descriptorOwner(htmlProto, "implementation") === Document.prototype,
    htmlProto === HTMLDocument.prototype,
    xmlProto === XMLDocument.prototype
  ].join(",");

  return [
    htmlFontsShape,
    xmlFontsShape,
    htmlImplementationShape,
    xmlImplementationShape,
    htmlLocationShape,
    xmlLocationShape,
    htmlKeysBefore,
    xmlKeysBefore,
    htmlImplementationCacheBefore,
    xmlImplementationCacheBefore,
    htmlImplementationCache,
    xmlImplementationCache,
    html.implementation === htmlImplementation,
    xml.implementation === xmlImplementation,
    html.fonts === htmlFonts,
    xml.fonts === xmlFonts,
    html.location === null,
    xml.location === null,
    htmlImplementation.createDocumentType("html", "", "").ownerDocument === html,
    xmlImplementation.createDocumentType("html", "", "").ownerDocument === xml,
    Object.prototype.toString.call(htmlFonts),
    Object.prototype.toString.call(xmlFonts),
    prototypeSurface
  ].join("|");
})()
"#,
        )
        .expect("detached document state accessor descriptor probe should evaluate");

    assert_eq!(
        result,
        "function,get fonts,true,true,true|function,get fonts,true,true,true|function,get implementation,true,true,true|function,get implementation,true,true,true|true,false,false,true|true,false,false,true|||true|true|true|true|true|true|true|true|true|true|true|true|[object FontFaceSet]|[object FontFaceSet]|false,false,false,false,false,false,false,false,true,true,true,true"
    );
}

#[test]
fn detached_document_creation_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-document-creation-brand-check.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const html = parser.parseFromString("<html><body></body></html>", "text/html");
  const xml = parser.parseFromString("<root></root>", "application/xml");
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  const htmlElement = Document.prototype.createElement.call(html, "section");
  const htmlElementNs = Document.prototype.createElementNS.call(
    html,
    "http://www.w3.org/1999/xhtml",
    "x:article"
  );
  const xmlElement = Document.prototype.createElement.call(xml, "Mixed");
  const xmlElementNs = Document.prototype.createElementNS.call(xml, "urn:test", "p:item");
  const text = Document.prototype.createTextNode.call(html, "txt");
  const comment = Document.prototype.createComment.call(html, "note");
  const fragment = Document.prototype.createDocumentFragment.call(html);
  const pi = Document.prototype.createProcessingInstruction.call(xml, "pi", "data");
  const attr = Document.prototype.createAttribute.call(html, "DATA-X");
  const nsAttr = Document.prototype.createAttributeNS.call(xml, "urn:test", "p:flag");
  const imported = Document.prototype.importNode.call(html, xmlElementNs, false);
  const adoptedSource = Document.prototype.createElement.call(xml, "adopted");
  const adopted = Document.prototype.adoptNode.call(html, adoptedSource);

  fragment.append(text, comment);

  return JSON.stringify({
    htmlElement: [
      htmlElement.ownerDocument === html,
      htmlElement.tagName,
      htmlElement instanceof HTMLElement,
      own(htmlElement, "tagName")
    ].join(","),
    htmlElementNs: [
      htmlElementNs.ownerDocument === html,
      htmlElementNs.prefix,
      htmlElementNs.localName,
      htmlElementNs.namespaceURI
    ].join(","),
    xmlElement: [
      xmlElement.ownerDocument === xml,
      xmlElement.localName,
      String(xmlElement.namespaceURI)
    ].join(","),
    xmlElementNs: [
      xmlElementNs.ownerDocument === xml,
      xmlElementNs.prefix,
      xmlElementNs.localName,
      xmlElementNs.namespaceURI
    ].join(","),
    characterNodes: [
      text.ownerDocument === html,
      text.data,
      comment.ownerDocument === html,
      comment.data,
      fragment.ownerDocument === html,
      fragment.childNodes.length,
      pi.ownerDocument === xml,
      pi.target
    ].join(","),
    attrs: [
      attr.ownerDocument === html,
      attr.name,
      nsAttr.ownerDocument === xml,
      nsAttr.prefix,
      nsAttr.localName,
      nsAttr.namespaceURI
    ].join(","),
    importAdopt: [
      imported.ownerDocument === html,
      imported.prefix,
      imported.localName,
      adopted === adoptedSource,
      adopted.ownerDocument === html
    ].join(","),
    documentOwn: [
      own(html, "createElement"),
      own(html, "createTextNode"),
      own(html, "createAttribute"),
      own(html, "importNode"),
      own(html, "adoptNode")
    ].join(",")
  });
})()
"#,
        )
        .expect("detached Document prototype creation brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"htmlElement":"true,SECTION,true,false","htmlElementNs":"true,x,article,http://www.w3.org/1999/xhtml","xmlElement":"true,Mixed,null","xmlElementNs":"true,p,item,urn:test","characterNodes":"true,txt,true,note,true,2,true,pi","attrs":"true,data-x,true,p,flag,urn:test","importAdopt":"true,p,item,true,true","documentOwn":"false,false,false,false,false"}"#
    );
}

#[test]
fn detached_document_lifecycle_methods_use_document_prototype_brand_checks() {
    let mut vm = new_storage_test_vm("https://detached-document-lifecycle-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const shape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.value,
      descriptor.value && descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const error = (callback) => {
    try {
      callback();
      return "ok";
    } catch (thrown) {
      return `${thrown.name}:${thrown.code}`;
    }
  };

  const html = document.implementation.createHTMLDocument("");
  const htmlProto = Object.getPrototypeOf(html);
  const openReturn = Document.prototype.open.call(html);
  Document.prototype.write.call(html, "<p id='a'>A</p>");
  Document.prototype.writeln.call(html, "<span id='b'>B</span>");
  const closeReturn = Document.prototype.close.call(html);

  const direct = document.implementation.createHTMLDocument("");
  direct.write("<em>E</em>");
  direct.writeln("<strong>S</strong>");

  const parsed = new DOMParser().parseFromString("<html><body></body></html>", "text/html");
  const parsedOpenReturn = Document.prototype.open.call(parsed);
  parsed.write("<article>parsed</article>");

  const xml = document.implementation.createDocument("urn:test", "root", null);

  return JSON.stringify({
    shapes: ["open", "write", "writeln", "close"].map(shape).join("|"),
    documentOwn: ["open", "write", "writeln", "close"].map((name) => own(document, name)).join(","),
    htmlOwn: ["open", "write", "writeln", "close"].map((name) => own(html, name)).join(","),
    htmlProtoOwn: ["open", "write", "writeln", "close"].map((name) => own(htmlProto, name)).join(","),
    htmlProtoIsStandard: htmlProto === HTMLDocument.prototype,
    openReturn: openReturn === html,
    closeReturn: closeReturn === undefined,
    body: html.body.innerHTML,
    direct: direct.body.innerHTML,
    parsedOpenReturn: parsedOpenReturn === parsed,
    parsed: parsed.body.innerHTML,
    errors: [
      error(() => html.open("/popup", "", "")),
      error(() => Document.prototype.open.call(xml)),
      error(() => Document.prototype.write.call(xml, "x")),
      error(() => Document.prototype.writeln.call(xml, "x")),
      error(() => Document.prototype.close.call(xml))
    ].join("|")
  });
})()
"#,
        )
        .expect("detached Document lifecycle brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"shapes":"true:function:0:true:true|true:function:0:true:true|true:function:0:true:true|true:function:0:true:true","documentOwn":"false,false,false,false","htmlOwn":"false,false,false,false","htmlProtoOwn":"false,false,false,false","htmlProtoIsStandard":true,"openReturn":true,"closeReturn":true,"body":"<p id=\"a\">A</p><span id=\"b\">B</span>\n","direct":"<em>E</em><strong>S</strong>\n","parsedOpenReturn":true,"parsed":"<article>parsed</article>","errors":"InvalidAccessError:15|InvalidStateError:11|InvalidStateError:11|InvalidStateError:11|InvalidStateError:11"}"#
    );
}

#[test]
fn detached_character_data_reads_native_value_after_data_projection_tamper() {
    let mut vm = new_storage_test_vm("https://detached-character-data-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const text = doc.createTextNode("real");
  doc.body.appendChild(text);
  Object.defineProperty(text, "data", {
    value: "fake",
    configurable: true
  });
  return [
    text.data,
    text.nodeValue,
    doc.body.textContent,
    text.isEqualNode(doc.createTextNode("real")),
    text.isEqualNode(doc.createTextNode("fake"))
  ].join("|");
})()
"#,
        )
        .expect("detached character data reads should stay native-backed after data tamper");

    assert_eq!(result, "fake|real|real|true|false");
}
#[test]
fn detached_character_data_clone_reads_native_value_after_data_projection_tamper() {
    let mut vm = new_storage_test_vm("https://detached-character-clone-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const text = doc.createTextNode("real");
  doc.body.appendChild(text);
  Object.defineProperty(text, "data", {
    value: "fake",
    configurable: true
  });
  const shallow = text.cloneNode(false);
  const parent = doc.createElement("div");
  parent.appendChild(text);
  const deep = parent.cloneNode(true);
  return [
    text.data,
    shallow.data,
    shallow.nodeValue,
    deep.firstChild.data,
    deep.firstChild.nodeValue
  ].join("|");
})()
"#,
        )
        .expect("detached character data clone should stay native-backed after data tamper");

    assert_eq!(result, "fake|real|real|real|real");
}
#[test]
fn detached_element_equality_reads_native_attributes_after_method_tamper() {
    let mut vm = new_storage_test_vm("https://detached-attribute-equality-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const left = doc.createElement("div");
  const same = doc.createElement("div");
  const different = doc.createElement("div");
  left.setAttribute("data-real", "one");
  same.setAttribute("data-real", "one");
  different.setAttribute("data-real", "two");
  left.getAttributeNames = () => [];
  left.getAttribute = () => "two";
  return [
    left.getAttributeNames().length,
    left.getAttribute("data-real"),
    left.isEqualNode(same),
    left.isEqualNode(different)
  ].join("|");
})()
"#,
        )
        .expect(
            "detached element equality should stay native-backed after attribute method tamper",
        );

    assert_eq!(result, "0|two|true|false");
}
#[test]
fn detached_html_document_accepts_live_comment_children() {
    let mut vm = new_storage_test_vm("https://detached-document-comment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const left = document.implementation.createHTMLDocument("");
  const right = document.implementation.createHTMLDocument("");
  left.appendChild(document.createComment("data"));
  right.appendChild(document.createComment("data"));
  return [
    left.lastChild.nodeType,
    left.lastChild.data,
    left.isEqualNode(right)
  ].join("|");
})()
"#,
        )
        .expect("detached HTML documents should accept live Comment children");

    assert_eq!(result, "8|data|true");
}
#[test]
fn detached_element_ns_attribute_methods_round_trip() {
    let mut vm = new_storage_test_vm("https://detached-ns-attr.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><div></div></body></html>', 'text/html');
  const el = doc.querySelector('div');
  function probe(callback) {
    try {
      const value = callback();
      return value === null ? "null" : String(value);
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  el.setAttributeNS("urn:moli:test", "lm:flag", "on");
  el.setAttributeNS(null, "data-local", "local");
  const stages = [
    el.getAttributeNS("urn:moli:test", "flag"),
    el.hasAttributeNS("urn:moli:test", "flag"),
    el.getAttributeNS(null, "data-local"),
    el.hasAttributeNS("", "data-local"),
    el.getAttribute("lm:flag"),
    probe(() => el.setAttributeNS("urn:moli:test", "bogus name", "v")),
    probe(() => el.setAttributeNS(null, "lm:bad", "v"))
  ];
  el.removeAttributeNS("urn:moli:test", "flag");
  el.removeAttributeNS(null, "data-local");
  stages.push(el.hasAttributeNS("urn:moli:test", "flag"));
  stages.push(el.hasAttributeNS(null, "data-local"));
  return stages.join("|");
})()
"#,
        )
        .expect("detached Element NS attribute methods should evaluate");

    assert_eq!(
        result,
        "on|true|local|true|on|throw:InvalidCharacterError|throw:NamespaceError|false|false"
    );
}
#[test]
fn detached_html_document_all_matches_chromium_htmldda_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const div = doc.createElement("div");
              div.id = "probe";
              doc.body.appendChild(div);
              const allDescriptor = Object.getOwnPropertyDescriptor(Document.prototype, "all");
              return JSON.stringify({
                ownAll: Object.prototype.hasOwnProperty.call(doc, "all"),
                protoGetter: typeof allDescriptor?.get,
                protoGetterTag: Object.prototype.toString.call(allDescriptor.get.call(doc)),
                allType: typeof doc.all,
                loose: doc.all == undefined,
                strict: doc.all === undefined,
                bool: !!doc.all,
                string: String(doc.all),
                tag: Object.prototype.toString.call(doc.all),
                ctorDirect: doc.all.constructor && doc.all.constructor.name,
                calledType: typeof doc.all(),
                calledNull: doc.all() === null,
                callByIndex: doc.all(doc.all.length - 1) === div,
                namedHit: doc.all("probe") === div,
                itemMethodNull: doc.all.item(999) === null,
                namedMethodNull: doc.all.namedItem("missing") === null
              });
            })()
            "#,
        )
        .expect("detached HTMLDocument.all probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownAll":false,"protoGetter":"function","protoGetterTag":"[object HTMLAllCollection]","allType":"undefined","loose":true,"strict":false,"bool":false,"string":"[object HTMLAllCollection]","tag":"[object HTMLAllCollection]","ctorDirect":"HTMLAllCollection","calledType":"object","calledNull":true,"callByIndex":true,"namedHit":true,"itemMethodNull":true,"namedMethodNull":true}"#
    );
}

#[test]
fn detached_document_all_declared_members_ignore_public_data_spoofing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const div = doc.createElement("div");
              div.id = "probe";
              doc.body.appendChild(div);
              const all = doc.all;
              const summarize = name => {
                const descriptor = Object.getOwnPropertyDescriptor(all, name);
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const summarizePrototype = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  HTMLAllCollection.prototype,
                  name
                );
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const beforeNames = Object.getOwnPropertyNames(all).includes("data");
              all.data = {
                items: [],
                named: { probe: null }
              };
              return [
                summarize("length"),
                summarize("item"),
                summarize("namedItem"),
                summarizePrototype(Symbol.iterator),
                Object.prototype.hasOwnProperty.call(all, Symbol.iterator),
                beforeNames,
                Object.prototype.hasOwnProperty.call(all, "data"),
                all.data && Array.isArray(all.data.items),
                all.item(all.length - 1) === div,
                all.namedItem("probe") === div,
                all("probe") === div,
                typeof all[Symbol.iterator]
              ].join("|");
            })()
            "#,
        )
        .expect("detached document.all declared surface spoofing probe should evaluate");

    assert_eq!(
        result,
        "true:false:true:false:number|true:false:true:true:function|true:false:true:true:function|true:false:true:true:function|false|false|true|true|true|true|true|function"
    );
}

#[test]
fn detached_collection_declared_iterators_ignore_public_data_spoofing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const div = doc.createElement("div");
              div.id = "probe";
              doc.body.appendChild(div);
              const nodeList = doc.querySelectorAll("div");
              const collection = doc.getElementsByTagName("div");
              const summarizePrototype = (object, key) => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  Object.getPrototypeOf(object),
                  key
                );
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value
                ].join(":");
              };
              const beforeNames = [
                Object.getOwnPropertyNames(nodeList).includes("data"),
                Object.getOwnPropertyNames(collection).includes("data")
              ].join(":");
              nodeList.data = { items: [] };
              collection.data = { items: [], named: { probe: null } };
              return [
                summarizePrototype(nodeList, Symbol.iterator),
                summarizePrototype(collection, Symbol.iterator),
                Object.prototype.hasOwnProperty.call(nodeList, Symbol.iterator),
                Object.prototype.hasOwnProperty.call(collection, Symbol.iterator),
                Object.getPrototypeOf(nodeList) === NodeList.prototype,
                Object.getPrototypeOf(collection) === HTMLCollection.prototype,
                beforeNames,
                Object.prototype.hasOwnProperty.call(nodeList, "data"),
                Object.prototype.hasOwnProperty.call(collection, "data"),
                Array.from(nodeList)[0] === div,
                Array.from(collection)[0] === div,
                nodeList.item(0) === div,
                collection.item(0) === div,
                collection.namedItem("probe") === div
              ].join("|");
            })()
            "#,
        )
        .expect("detached collection declared iterator spoofing probe should evaluate");

    assert_eq!(
        result,
        "true:false:true:true:function|true:false:true:true:function|false|false|true|true|false:false|true|true|true|true|true|true|true"
    );
}

#[test]
fn detached_html_document_all_includes_default_document_tree() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              return JSON.stringify({
                allType: typeof doc.all,
                loose: doc.all == undefined,
                strict: doc.all === undefined,
                calledNull: doc.all() === null,
                calledLength: doc.all.length
              });
            })()
            "#,
        )
        .expect("empty detached HTMLDocument.all probe should evaluate");

    assert_eq!(
        result,
        r#"{"allType":"undefined","loose":true,"strict":false,"calledNull":true,"calledLength":4}"#
    );
}
#[test]
fn detached_html_elements_expose_click_method() {
    let mut vm = new_storage_test_vm("https://detached-element-click.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const link = doc.createElement("a");
              let clicks = 0;
              link.addEventListener("click", event => {
                clicks += event.isTrusted ? 1 : 10;
              });
              link.click();
              return `${typeof link.click}|${clicks}`;
            })()
            "#,
        )
        .expect("detached HTML elements should expose synthetic click");

    assert_eq!(result, "function|10");
}
#[test]
fn detached_html_document_body_class_list_matches_domtokenlist_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const body = doc.body;
              body.className = "outer highlight";
              const list = body.classList;
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const stable = list === body.classList;
              const seen = [];
              const thisArg = { marker: "detached" };
              const initial = {
                tag: Object.prototype.toString.call(list),
                containsHighlight: list.contains("highlight"),
                containsMissing: list.contains("missing"),
                containsEmpty: list.contains(""),
                item0: list.item(0),
                item1: list.item(1),
                itemSymbol: probe(() => list.item(Symbol())),
                length: list.length,
                stable
              };
              list.forEach(function(value, index, owner) {
                seen.push(`${this.marker}:${value}:${index}:${owner === list}`);
              }, thisArg);
              list.remove("outer");
              list.add("processed");
              const replaced = list.replace("highlight", "done");
              return JSON.stringify({
                initial,
                seen,
                replaced,
                finalClassName: body.className,
                finalValue: list.value,
                containsDone: list.contains("done"),
                containsOuter: list.contains("outer"),
                toggledMissing: list.toggle("missing", false),
                containsSymbol: probe(() => list.contains(Symbol())),
                toggleSymbol: probe(() => list.toggle(Symbol())),
                replaceMissing: probe(() => list.replace("done")),
                forEachMissing: probe(() => list.forEach())
              });
            })()
            "#,
        )
        .expect("detached HTMLDocument body.classList should behave like DOMTokenList");

    assert_eq!(
        result,
        r#"{"initial":{"tag":"[object DOMTokenList]","containsHighlight":true,"containsMissing":false,"containsEmpty":false,"item0":"outer","item1":"highlight","itemSymbol":"throw:TypeError","length":2,"stable":true},"seen":["detached:outer:0:true","detached:highlight:1:true"],"replaced":true,"finalClassName":"done processed","finalValue":"done processed","containsDone":true,"containsOuter":false,"toggledMissing":false,"containsSymbol":"throw:TypeError","toggleSymbol":"throw:TypeError","replaceMissing":"throw:TypeError","forEachMissing":"throw:TypeError"}"#
    );
}
#[test]
fn detached_html_document_class_list_internal_slots_are_not_visible_to_js() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const body = doc.body;
              const list = body.classList;
              return JSON.stringify({
                bodyOwnNamesHasCacheSlot: Object.getOwnPropertyNames(body).includes("__moliDetachedClassList"),
                listOwnNamesHasTargetSlot: Object.getOwnPropertyNames(list).includes("__moliDetachedClassListTarget"),
                bodyOwnKeysHasCacheSlot: Reflect.ownKeys(body).includes("__moliDetachedClassList"),
                listOwnKeysHasTargetSlot: Reflect.ownKeys(list).includes("__moliDetachedClassListTarget"),
                bodyHasCacheSlot: "__moliDetachedClassList" in body,
                listHasTargetSlot: "__moliDetachedClassListTarget" in list,
                bodyCacheValueType: typeof body.__moliDetachedClassList,
                listTargetValueType: typeof list.__moliDetachedClassListTarget
              });
            })()
            "#,
        )
        .expect("detached classList private slots should stay hidden from JS inspection");

    assert_eq!(
        result,
        r#"{"bodyOwnNamesHasCacheSlot":false,"listOwnNamesHasTargetSlot":false,"bodyOwnKeysHasCacheSlot":false,"listOwnKeysHasTargetSlot":false,"bodyHasCacheSlot":false,"listHasTargetSlot":false,"bodyCacheValueType":"undefined","listTargetValueType":"undefined"}"#
    );
}
#[test]
fn detached_domparser_node_adoption_matches_chromium_parent_and_connected_semantics() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement('html'));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement('body'));
              }
              const doc = new DOMParser().parseFromString(
                '<html><body><main><span id="child">x</span></main></body></html>',
                'text/html'
              );
              const node = doc.getElementById('child');
              const host = document.body || document.documentElement || document;
              const before = {
                parentIsMain: node.parentNode === doc.querySelector('main'),
                parentTag: node.parentNode && (node.parentNode.tagName || node.parentNode.nodeName),
                isConnected: node.isConnected,
                ownerDocumentIsDetached: node.ownerDocument === doc,
                hostContains: host.contains(node)
              };
              host.appendChild(node);
              const after = {
                parentIsHost: node.parentNode === host,
                parentTag: node.parentNode && (node.parentNode.tagName || node.parentNode.nodeName),
                isConnected: node.isConnected,
                ownerDocumentIsLive: node.ownerDocument === document,
                hostContains: host.contains(node)
              };
              return JSON.stringify({ before, after });
            })()
            "#,
        )
        .expect("detached DOMParser node adoption should return a probe result");

    assert_eq!(
        result,
        r#"{"before":{"parentIsMain":true,"parentTag":"MAIN","isConnected":true,"ownerDocumentIsDetached":true,"hostContains":false},"after":{"parentIsHost":true,"parentTag":"BODY","isConnected":true,"ownerDocumentIsLive":true,"hostContains":true}}"#
    );
}
#[test]
fn detached_document_append_child_returns_materialized_child() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createDocument(null, "", null);
              const liveComment = document.createComment("before");
              const inserted = doc.appendChild(liveComment);
              return JSON.stringify({
                returnedIsStored: inserted === doc.firstChild,
                ownerIsDetached: inserted.ownerDocument === doc,
                originalAdopted: liveComment.ownerDocument === doc,
                childCount: doc.childNodes.length
              });
            })()
            "#,
        )
        .expect("detached appendChild return probe should evaluate");

    assert_eq!(
        result,
        r#"{"returnedIsStored":true,"ownerIsDetached":true,"originalAdopted":true,"childCount":1}"#
    );
}
#[test]
fn detached_domparser_adopted_nodes_follow_live_tree_for_children_text_and_mutation_methods() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement('html'));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement('body'));
              }
              const doc = new DOMParser().parseFromString(
                '<html><body><main><span id="child">x</span></main></body></html>',
                'text/html'
              );
              const root = doc.querySelector('main');
              const heldChild = root.firstChild;
              document.body.appendChild(root);

              const liveRoot = document.body.firstChild;
              liveRoot.appendChild(document.createTextNode('y'));

              const beforeRemoval = {
                firstChildIsHeld: root.firstChild === heldChild,
                childParentIsForeignRoot: heldChild.parentNode === root,
                childNodesLength: root.childNodes.length,
                lastChildType: root.lastChild && root.lastChild.nodeType,
                textContent: root.textContent,
                containsHeldChild: root.contains(heldChild)
              };

              const removed = root.removeChild(heldChild);

              const afterRemoval = {
                removedIsHeld: removed === heldChild,
                removedParentIsNull: heldChild.parentNode === null,
                childNodesLength: root.childNodes.length,
                firstChildType: root.firstChild && root.firstChild.nodeType,
                textContent: root.textContent,
                liveBodyText: document.body.firstChild && document.body.firstChild.textContent
              };

              return JSON.stringify({ beforeRemoval, afterRemoval });
            })()
            "#,
        )
        .expect("adopted DOMParser nodes should keep tracking the live subtree");

    assert_eq!(
        result,
        r#"{"beforeRemoval":{"firstChildIsHeld":true,"childParentIsForeignRoot":true,"childNodesLength":2,"lastChildType":3,"textContent":"xy","containsHeldChild":true},"afterRemoval":{"removedIsHeld":true,"removedParentIsNull":true,"childNodesLength":1,"firstChildType":3,"textContent":"y","liveBodyText":"y"}}"#
    );
}
