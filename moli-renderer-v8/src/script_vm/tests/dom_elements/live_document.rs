use super::*;

#[test]
fn document_compat_mode_reflects_parser_quirks_mode() {
    let cases = [
        (
            "<!doctype html><html><head></head><body></body></html>",
            "CSS1Compat",
        ),
        (
            "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\"><html><head></head><body></body></html>",
            "CSS1Compat",
        ),
        ("<title>quirks</title><body></body>", "BackCompat"),
    ];

    for (markup, expected) in cases {
        let mut vm = new_parsed_test_vm("https://compat-mode.test/", markup);
        assert_eq!(
            vm.eval("document.compatMode")
                .expect("document.compatMode should evaluate"),
            expected
        );
    }
}

#[test]
fn zhihu_probe_live_document_shape_matches_chromium_branding() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              const all = Object.getOwnPropertyDescriptor(Document.prototype, "all");
              return [
                typeof Document,
                typeof HTMLDocument,
                document.constructor && document.constructor.name,
                Object.prototype.toString.call(document),
                Object.prototype.hasOwnProperty.call(document, "createElement"),
                Object.prototype.hasOwnProperty.call(document, "all"),
                typeof Document.prototype.createElement,
                typeof Document.prototype.getElementById,
                typeof all?.get,
                "createElement" in div,
                "all" in div
              ].join("|");
            })()
            "#,
        )
        .expect("document branding probe should evaluate");

    assert_eq!(
        result,
        "function|function|HTMLDocument|[object HTMLDocument]|false|false|function|function|function|false|false"
    );
}

#[test]
fn prototype_template_migration_matches_chromium_descriptor_and_realm_probe() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const functionField = (value, key) =>
                typeof value === "function" ? value[key] : null;
              const descriptor = (owner, key) => {
                const value = Object.getOwnPropertyDescriptor(owner.prototype, key);
                return {
                  kind: Object.hasOwn(value, "value") ? "data" : "accessor",
                  enumerable: value.enumerable,
                  configurable: value.configurable,
                  writable: Object.hasOwn(value, "writable") ? value.writable : null,
                  valueName: functionField(value.value, "name"),
                  valueLength: functionField(value.value, "length"),
                  getName: functionField(value.get, "name"),
                  getLength: functionField(value.get, "length"),
                  setName: functionField(value.set, "name"),
                  setLength: functionField(value.set, "length")
                };
              };
              const area = document.createElement("area");
              const anchor = document.createElement("a");
              const frame = document.body.appendChild(document.createElement("iframe"));
              const child = frame.contentWindow;
              return JSON.stringify({
                anchorToString: descriptor(HTMLAnchorElement, "toString"),
                areaToString: descriptor(HTMLAreaElement, "toString"),
                areaHref: descriptor(HTMLAreaElement, "href"),
                areaRel: descriptor(HTMLAreaElement, "rel"),
                nodeTextContent: descriptor(Node, "textContent"),
                documentURL: descriptor(Document, "URL"),
                documentOnclick: descriptor(Document, "onclick"),
                elementId: descriptor(Element, "id"),
                instanceOwn: {
                  anchorToString: Object.hasOwn(anchor, "toString"),
                  areaHref: Object.hasOwn(area, "href")
                },
                childRealm: {
                  constructorsDistinct: child.HTMLAreaElement !== HTMLAreaElement,
                  prototypesDistinct:
                    child.HTMLAreaElement.prototype !== HTMLAreaElement.prototype,
                  toStringDistinct:
                    child.HTMLAreaElement.prototype.toString !==
                    HTMLAreaElement.prototype.toString,
                  toStringUsesChildFunctionPrototype:
                    Object.getPrototypeOf(child.HTMLAreaElement.prototype.toString) ===
                    child.Function.prototype
                }
              });
            })()
            "#,
        )
        .expect("prototype template Chromium comparison probe should evaluate");

    assert_eq!(
        result,
        r#"{"anchorToString":{"kind":"data","enumerable":true,"configurable":true,"writable":true,"valueName":"toString","valueLength":0,"getName":null,"getLength":null,"setName":null,"setLength":null},"areaToString":{"kind":"data","enumerable":true,"configurable":true,"writable":true,"valueName":"toString","valueLength":0,"getName":null,"getLength":null,"setName":null,"setLength":null},"areaHref":{"kind":"accessor","enumerable":true,"configurable":true,"writable":null,"valueName":null,"valueLength":null,"getName":"get href","getLength":0,"setName":"set href","setLength":1},"areaRel":{"kind":"accessor","enumerable":true,"configurable":true,"writable":null,"valueName":null,"valueLength":null,"getName":"get rel","getLength":0,"setName":"set rel","setLength":1},"nodeTextContent":{"kind":"accessor","enumerable":true,"configurable":true,"writable":null,"valueName":null,"valueLength":null,"getName":"get textContent","getLength":0,"setName":"set textContent","setLength":1},"documentURL":{"kind":"accessor","enumerable":true,"configurable":true,"writable":null,"valueName":null,"valueLength":null,"getName":"get URL","getLength":0,"setName":null,"setLength":null},"documentOnclick":{"kind":"accessor","enumerable":true,"configurable":true,"writable":null,"valueName":null,"valueLength":null,"getName":"get onclick","getLength":0,"setName":"set onclick","setLength":1},"elementId":{"kind":"accessor","enumerable":true,"configurable":true,"writable":null,"valueName":null,"valueLength":null,"getName":"get id","getLength":0,"setName":"set id","setLength":1},"instanceOwn":{"anchorToString":false,"areaHref":false},"childRealm":{"constructorsDistinct":true,"prototypesDistinct":true,"toStringDistinct":true,"toStringUsesChildFunctionPrototype":true}}"#
    );
}

#[test]
fn node_prototype_methods_support_shadydom_native_copy() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const summarize = (name) => {
                const descriptor = Object.getOwnPropertyDescriptor(Node.prototype, name);
                return [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const inheritedShape = [
                "appendChild",
                "insertBefore",
                "removeChild",
                "replaceChild",
                "cloneNode",
                "contains",
                "hasChildNodes",
                "compareDocumentPosition",
                "getRootNode",
                "normalize"
              ].map((name) => [
                name,
                Object.prototype.hasOwnProperty.call(document, name),
                Object.prototype.hasOwnProperty.call(document.documentElement, name),
                typeof Node.prototype[name],
                typeof document[name],
                typeof document.documentElement[name]
              ].join(":")).join("|");
              const container = document.createElement("div");
              const first = document.createElement("span");
              const second = document.createElement("em");
              container.appendChild(first);
              container.insertBefore(second, first);
              const removed = container.removeChild(first);
              const replacement = document.createElement("strong");
              const replaced = container.replaceChild(replacement, second);
              const containsDescriptor = Object.getOwnPropertyDescriptor(Node.prototype, "contains");
              const cloneDescriptor = Object.getOwnPropertyDescriptor(Node.prototype, "cloneNode");
              Object.defineProperty(Node.prototype, "__shady_native_contains", containsDescriptor);
              Object.defineProperty(Node.prototype, "__shady_native_cloneNode", cloneDescriptor);
              const clone = document.documentElement.__shady_native_cloneNode(false);
              return JSON.stringify({
                containsShape: summarize("contains"),
                cloneShape: summarize("cloneNode"),
                documentNativeContains: typeof document.__shady_native_contains,
                elementNativeCloneNode: typeof document.documentElement.__shady_native_cloneNode,
                containsResult: document.__shady_native_contains(document.documentElement),
                cloneNodeName: clone && clone.nodeName,
                inheritedShape,
                mutationResult: [
                  removed.nodeName,
                  replaced.nodeName,
                  container.firstChild && container.firstChild.nodeName,
                  container.childNodes.length
                ].join(":"),
                documentProtoParentIsNodeProto: Object.getPrototypeOf(Document.prototype) === Node.prototype
              });
            })()
            "#,
        )
        .expect("ShadyDOM native-copy probe should evaluate");

    assert_eq!(
        result,
        r#"{"containsShape":"true:function:contains:1:true:true:true","cloneShape":"true:function:cloneNode:0:true:true:true","documentNativeContains":"function","elementNativeCloneNode":"function","containsResult":true,"cloneNodeName":"HTML","inheritedShape":"appendChild:false:false:function:function:function|insertBefore:false:false:function:function:function|removeChild:false:false:function:function:function|replaceChild:false:false:function:function:function|cloneNode:false:false:function:function:function|contains:false:false:function:function:function|hasChildNodes:false:false:function:function:function|compareDocumentPosition:false:false:function:function:function|getRootNode:false:false:function:function:function|normalize:false:false:function:function:function","mutationResult":"SPAN:EM:STRONG:1","documentProtoParentIsNodeProto":true}"#
    );
}

#[test]
fn node_core_accessors_live_on_node_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/path/page.html",
        "<!doctype html><html><head></head><body><main>old</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const main = document.querySelector("main");
              const text = main.firstChild;
              const comment = document.createComment("note");
              const detached = document.createElement("aside");
              const names = [
                "nodeType",
                "nodeName",
                "nodeValue",
                "isConnected",
                "ownerDocument",
                "baseURI",
                "parentNode",
                "parentElement",
                "childNodes",
                "firstChild",
                "lastChild",
                "previousSibling",
                "nextSibling",
                "textContent"
              ];
              const accessorShape = (name) => {
                const descriptor = Object.getOwnPropertyDescriptor(Node.prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.get,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };
              const own = (object) =>
                names.filter((name) => Object.prototype.hasOwnProperty.call(object, name));

              const before = {
                documentNodeType: document.nodeType,
                documentNodeName: document.nodeName,
                documentOwnerDocument: document.ownerDocument,
                documentBaseURI: document.baseURI,
                elementNodeType: main.nodeType,
                elementNodeName: main.nodeName,
                elementOwnerDocument: main.ownerDocument === document,
                elementParentNode: main.parentNode === document.body,
                elementParentElement: main.parentElement === document.body,
                elementChildNodes: main.childNodes.length,
                elementFirstChild: main.firstChild === text,
                elementLastChild: main.lastChild === text,
                textParentNode: text.parentNode === main,
                textPreviousSibling: text.previousSibling,
                textNextSibling: text.nextSibling,
                textNodeType: text.nodeType,
                textNodeName: text.nodeName,
                textNodeValue: text.nodeValue,
                textContent: text.textContent,
                mainConnected: main.isConnected,
                detachedConnected: detached.isConnected,
                commentNodeValue: comment.nodeValue
              };

              text.nodeValue = "beta";
              const afterNodeValueSetter = {
                textNodeValue: text.nodeValue,
                textContent: text.textContent,
                mainTextContent: main.textContent
              };

              text.textContent = null;
              const afterTextContentNullSetter = {
                textNodeValue: text.nodeValue,
                textContent: text.textContent,
                mainTextContent: main.textContent
              };

              main.nodeValue = "ignored";
              const afterElementNodeValueSetter = main.nodeValue;

              return JSON.stringify({
                descriptors: names.map(accessorShape),
                own: {
                  document: own(document),
                  element: own(main),
                  text: own(text),
                  comment: own(comment),
                  detached: own(detached)
                },
                before,
                afterNodeValueSetter,
                afterTextContentNullSetter,
                afterElementNodeValueSetter
              });
            })()
            "#,
        )
        .expect("Node core accessor prototype probe should evaluate");

    assert_eq!(
        result,
        r##"{"descriptors":["nodeType:true:function:undefined:true:true","nodeName:true:function:undefined:true:true","nodeValue:true:function:function:true:true","isConnected:true:function:undefined:true:true","ownerDocument:true:function:undefined:true:true","baseURI:true:function:undefined:true:true","parentNode:true:function:undefined:true:true","parentElement:true:function:undefined:true:true","childNodes:true:function:undefined:true:true","firstChild:true:function:undefined:true:true","lastChild:true:function:undefined:true:true","previousSibling:true:function:undefined:true:true","nextSibling:true:function:undefined:true:true","textContent:true:function:function:true:true"],"own":{"document":[],"element":[],"text":[],"comment":[],"detached":[]},"before":{"documentNodeType":9,"documentNodeName":"#document","documentOwnerDocument":null,"documentBaseURI":"https://example.com/path/page.html","elementNodeType":1,"elementNodeName":"MAIN","elementOwnerDocument":true,"elementParentNode":true,"elementParentElement":true,"elementChildNodes":1,"elementFirstChild":true,"elementLastChild":true,"textParentNode":true,"textPreviousSibling":null,"textNextSibling":null,"textNodeType":3,"textNodeName":"#text","textNodeValue":"old","textContent":"old","mainConnected":true,"detachedConnected":false,"commentNodeValue":"note"},"afterNodeValueSetter":{"textNodeValue":"beta","textContent":"beta","mainTextContent":"beta"},"afterTextContentNullSetter":{"textNodeValue":"","textContent":"","mainTextContent":""},"afterElementNodeValueSetter":null}"##
    );
}

#[test]
fn dom_mixin_members_live_on_standard_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><main><span></span></main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const main = document.querySelector("main");
              const span = main.firstChild;
              const text = document.createTextNode("text");
              main.appendChild(text);
              const doctype = document.implementation.createDocumentType("html", "", "");
              const fragment = document.createDocumentFragment();
              fragment.appendChild(document.createElement("section"));
              const documentTypeMoveBeforeError = (() => {
                try {
                  const doc = document.implementation.createHTMLDocument("title");
                  const doctype = doc.childNodes[0].cloneNode();
                  doc.documentElement.remove();
                  doc.moveBefore(doctype, null);
                  return "none";
                } catch (error) {
                  return [
                    error && error.name,
                    error && error.code,
                    error instanceof DOMException
                  ].join(":");
                }
              })();

              const methodShape = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const accessorShape = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  !!descriptor,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };
              const own = (object, names) =>
                names.map((name) => `${name}:${Object.prototype.hasOwnProperty.call(object, name)}`).join("|");

              const fragmentHit = fragment.querySelector("section");
              return JSON.stringify({
                parentMethods: [
                  methodShape(Document.prototype, "append"),
                  methodShape(DocumentFragment.prototype, "prepend"),
                  methodShape(Element.prototype, "replaceChildren"),
                  methodShape(Element.prototype, "querySelector"),
                  methodShape(Document.prototype, "moveBefore")
                ],
                parentAccessors: [
                  accessorShape(Document.prototype, "children"),
                  accessorShape(DocumentFragment.prototype, "firstElementChild"),
                  accessorShape(Element.prototype, "childElementCount")
                ],
                childMethods: [
                  methodShape(Element.prototype, "before"),
                  methodShape(CharacterData.prototype, "after"),
                  methodShape(DocumentType.prototype, "replaceWith"),
                  methodShape(Element.prototype, "remove")
                ],
                nonDocumentTypeChildAccessors: [
                  accessorShape(Element.prototype, "previousElementSibling"),
                  accessorShape(CharacterData.prototype, "nextElementSibling")
                ],
                ownShapes: {
                  document: own(document, ["append", "querySelector", "children", "before", "previousElementSibling"]),
                  fragment: own(fragment, ["append", "querySelector", "children", "before", "previousElementSibling"]),
                  element: own(main, ["append", "querySelector", "children", "before", "previousElementSibling"]),
                  text: own(text, ["append", "querySelector", "children", "before", "previousElementSibling"]),
                  doctype: own(doctype, ["append", "querySelector", "children", "before", "previousElementSibling"])
                },
                availability: {
                  documentParent: [typeof document.append, typeof document.querySelector, typeof document.children],
                  fragmentParent: [typeof fragment.append, typeof fragment.querySelector, typeof fragment.children],
                  elementParent: [typeof main.append, typeof main.querySelector, typeof main.children],
                  textChild: [typeof text.before, typeof text.after, typeof text.remove],
                  doctypeChild: [typeof doctype.before, typeof doctype.after, typeof doctype.remove],
                  excluded: [
                    typeof text.append,
                    typeof text.querySelector,
                    typeof text.children,
                    typeof document.before,
                    typeof document.previousElementSibling,
                    typeof fragment.before,
                    typeof fragment.previousElementSibling,
                    typeof doctype.previousElementSibling
                  ]
                },
                behavior: {
                  documentQuery: document.querySelector("main") === main,
                  fragmentNodeType: fragment.nodeType,
                  fragmentChildNodes: fragment.childNodes.length,
                  fragmentChildren: fragment.children.length,
                  fragmentFirstChild: fragment.firstChild && fragment.firstChild.nodeName,
                  fragmentQuery: fragmentHit && fragmentHit.nodeName,
                  fragmentQueryAll: fragment.querySelectorAll("section").length,
                  elementChildren: main.children.length,
                  textNextElementSibling: text.nextElementSibling,
                  spanNextElementSibling: span.nextElementSibling === null,
                  documentTypeMoveBeforeError
                }
              });
            })()
            "#,
        )
        .expect("DOM mixin prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"parentMethods":["true:function:append:0:true:true:true","true:function:prepend:0:true:true:true","true:function:replaceChildren:0:true:true:true","true:function:querySelector:1:true:true:true","true:function:moveBefore:2:true:true:true"],"parentAccessors":["true:function:get children:true:true","true:function:get firstElementChild:true:true","true:function:get childElementCount:true:true"],"childMethods":["true:function:before:0:true:true:true","true:function:after:0:true:true:true","true:function:replaceWith:0:true:true:true","true:function:remove:0:true:true:true"],"nonDocumentTypeChildAccessors":["true:function:get previousElementSibling:true:true","true:function:get nextElementSibling:true:true"],"ownShapes":{"document":"append:false|querySelector:false|children:false|before:false|previousElementSibling:false","fragment":"append:false|querySelector:false|children:false|before:false|previousElementSibling:false","element":"append:false|querySelector:false|children:false|before:false|previousElementSibling:false","text":"append:false|querySelector:false|children:false|before:false|previousElementSibling:false","doctype":"append:false|querySelector:false|children:false|before:false|previousElementSibling:false"},"availability":{"documentParent":["function","function","object"],"fragmentParent":["function","function","object"],"elementParent":["function","function","object"],"textChild":["function","function","function"],"doctypeChild":["function","function","function"],"excluded":["undefined","undefined","undefined","undefined","undefined","undefined","undefined","undefined"]},"behavior":{"documentQuery":true,"fragmentNodeType":11,"fragmentChildNodes":1,"fragmentChildren":1,"fragmentFirstChild":"SECTION","fragmentQuery":"SECTION","fragmentQueryAll":1,"elementChildren":1,"textNextElementSibling":null,"spanNextElementSibling":true,"documentTypeMoveBeforeError":"HierarchyRequestError:3:true"}}"#
    );
}

#[test]
fn dom_owner_accessors_live_on_standard_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/path/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              div.id = "alpha";
              div.className = "one two";
              div.classList = "three four";
              div.part = "badge primary";
              div.slot = "named-slot";
              div.innerHTML = "<span>a</span>";
              div.firstChild.outerHTML = "<em>b</em>";
              const shadowHost = document.createElement("section");
              const shadowRoot = shadowHost.attachShadow({ mode: "open" });
              shadowRoot.innerHTML = "<i>s</i>";
              const doctype = document.implementation.createDocumentType("html", "pub", "sys");
              const pi = document.createProcessingInstruction("xml-stylesheet", "href='a.css'");
              const text = document.createTextNode("text");
              const fragment = document.createDocumentFragment();
              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg:g");

              const accessorShape = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                const flag = (value) => value === undefined ? "undefined" : String(value);
                return [
                  !!descriptor,
                  typeof descriptor?.get,
                  typeof descriptor?.set,
                  flag(descriptor?.enumerable),
                  flag(descriptor?.configurable)
                ].join(":");
              };
              const own = (object, names) =>
                names.map((name) => `${name}:${Object.prototype.hasOwnProperty.call(object, name)}`).join("|");

              return JSON.stringify({
                descriptors: {
                  element: [
                    accessorShape(Element.prototype, "id"),
                    accessorShape(Element.prototype, "className"),
                    accessorShape(Element.prototype, "tagName"),
                    accessorShape(Element.prototype, "localName"),
                    accessorShape(Element.prototype, "namespaceURI"),
                    accessorShape(Element.prototype, "prefix"),
                    accessorShape(Element.prototype, "innerHTML"),
                    accessorShape(Element.prototype, "outerHTML"),
                    accessorShape(Element.prototype, "classList"),
                    accessorShape(Element.prototype, "part"),
                    accessorShape(Element.prototype, "attributes"),
                    accessorShape(Element.prototype, "shadowRoot"),
                    accessorShape(Element.prototype, "slot"),
                    accessorShape(Element.prototype, "assignedSlot")
                  ],
                  shadowRoot: [
                    accessorShape(ShadowRoot.prototype, "innerHTML"),
                    accessorShape(ShadowRoot.prototype, "outerHTML")
                  ],
                  documentType: [
                    accessorShape(DocumentType.prototype, "name"),
                    accessorShape(DocumentType.prototype, "publicId"),
                    accessorShape(DocumentType.prototype, "systemId")
                  ],
                  processingInstruction: [
                    accessorShape(ProcessingInstruction.prototype, "target")
                  ],
                  document: [
                    accessorShape(Document.prototype, "defaultView")
                  ]
                },
                own: {
                  element: own(div, [
                    "id",
                    "className",
                    "tagName",
                    "localName",
                    "namespaceURI",
                    "prefix",
                    "innerHTML",
                    "outerHTML",
                    "classList",
                    "part",
                    "attributes",
                    "shadowRoot",
                    "slot",
                    "assignedSlot"
                  ]),
                  shadowRoot: own(shadowRoot, ["innerHTML", "outerHTML"]),
                  documentType: own(doctype, ["name", "publicId", "systemId"]),
                  processingInstruction: own(pi, ["target"]),
                  document: own(document, ["defaultView"]),
                  parentWindow: [
                    "parentWindow" in document,
                    typeof document.parentWindow,
                    Object.getOwnPropertyDescriptor(Document.prototype, "parentWindow") === undefined
                  ]
                },
                availability: {
                  text: [typeof text.tagName, typeof text.innerHTML, typeof text.defaultView, typeof text.target],
                  fragment: ["innerHTML" in fragment, typeof fragment.innerHTML],
                  specialized: [
                    Object.prototype.hasOwnProperty.call(HTMLElement.prototype, "innerHTML"),
                    Object.prototype.hasOwnProperty.call(HTMLDivElement.prototype, "tagName"),
                    Object.prototype.hasOwnProperty.call(HTMLElement.prototype, "classList"),
                    Object.prototype.hasOwnProperty.call(HTMLDivElement.prototype, "id"),
                    div.innerHTML,
                    shadowRoot.innerHTML
                  ]
                },
                behavior: {
                  divNames: [div.tagName, div.localName, div.namespaceURI, div.prefix],
                  svgNames: [svg.localName, svg.namespaceURI, svg.prefix],
                  html: [div.innerHTML, div.outerHTML],
                  elementCore: [
                    div.id,
                    div.className,
                    div.classList.value,
                    div.part.value,
                    div.slot,
                    div.attributes.length,
                    div.shadowRoot
                  ],
                  doctype: [doctype.name, doctype.publicId, doctype.systemId],
                  piTarget: pi.target,
                  documentView: [document.defaultView === window]
                }
              });
            })()
            "#,
        )
        .expect("DOM owner accessor prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":{"element":["true:function:function:true:true","true:function:function:true:true","true:function:undefined:true:true","true:function:undefined:true:true","true:function:undefined:true:true","true:function:undefined:true:true","true:function:function:true:true","true:function:function:true:true","true:function:function:true:true","true:function:function:true:true","true:function:undefined:true:true","true:function:undefined:true:true","true:function:function:true:true","true:function:undefined:true:true"],"shadowRoot":["true:function:function:true:true","false:undefined:undefined:undefined:undefined"],"documentType":["true:function:undefined:true:true","true:function:undefined:true:true","true:function:undefined:true:true"],"processingInstruction":["true:function:undefined:true:true"],"document":["true:function:undefined:true:true"]},"own":{"element":"id:false|className:false|tagName:false|localName:false|namespaceURI:false|prefix:false|innerHTML:false|outerHTML:false|classList:false|part:false|attributes:false|shadowRoot:false|slot:false|assignedSlot:false","shadowRoot":"innerHTML:false|outerHTML:false","documentType":"name:false|publicId:false|systemId:false","processingInstruction":"target:false","document":"defaultView:false","parentWindow":[false,"undefined",true]},"availability":{"text":["undefined","undefined","undefined","undefined"],"fragment":[false,"undefined"],"specialized":[false,false,false,false,"<em>b</em>","<i>s</i>"]},"behavior":{"divNames":["DIV","div","http://www.w3.org/1999/xhtml",null],"svgNames":["g","http://www.w3.org/2000/svg","svg"],"html":["<em>b</em>","<div id=\"alpha\" class=\"three four\" part=\"badge primary\" slot=\"named-slot\"><em>b</em></div>"],"elementCore":["alpha","three four","three four","badge primary","named-slot",4,null],"doctype":["html","pub","sys"],"piTarget":"xml-stylesheet","documentView":[true]}}"#
    );
}

#[test]
fn document_active_element_uses_document_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, "activeElement");
              assert(!!descriptor, "activeElement descriptor");
              assert(typeof descriptor.get === "function", "activeElement getter");
              assert(descriptor.set === undefined, "activeElement setter");
              assert(descriptor.enumerable === true, "activeElement enumerable");
              assert(descriptor.configurable === true, "activeElement configurable");
              assert(!own(document, "activeElement"), "document activeElement should not be own");

              const input = document.createElement("input");
              document.body.append(input);
              input.focus();
              assert(document.activeElement === input, "focused activeElement");

              const detachedDoc = document.implementation.createHTMLDocument("");
              assert(!own(detachedDoc, "activeElement"), "detached document activeElement should not be own");
              assert("activeElement" in detachedDoc, "detached document activeElement inherited");
              return "ok";
            })()
            "#,
        )
        .expect("document activeElement prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn htmlelement_standard_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/path/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const accessor = (prototype, name, hasSetter = true) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on ${prototype.constructor?.name || "prototype"}`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);

              const htmlNames = [
                "title",
                "lang",
                "autocapitalize",
                "translate",
                "dir",
                "hidden",
                "accessKey",
                "draggable",
                "spellcheck",
                "contentEditable",
                "enterKeyHint",
                "isContentEditable",
                "inputMode",
                "innerText",
                "outerText",
                "popover"
              ];
              const htmlReadonly = new Set(["isContentEditable"]);
              for (const name of htmlNames) {
                accessor(HTMLElement.prototype, name, !htmlReadonly.has(name));
                assert(!own(Element.prototype, name), `${name} duplicated on Element.prototype`);
                assert(!own(HTMLDivElement.prototype, name), `${name} duplicated on HTMLDivElement.prototype`);
              }

              const mixinNames = ["autofocus", "tabIndex"];
              for (const prototype of [HTMLElement.prototype, SVGElement.prototype, MathMLElement.prototype]) {
                for (const name of mixinNames) {
                  accessor(prototype, name);
                }
              }

              const div = document.createElement("div");
              for (const name of htmlNames.concat(mixinNames)) {
                assert(!own(div, name), `${name} should not be own on div`);
              }

              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
              for (const name of mixinNames) {
                assert(!own(svg, name), `${name} should not be own on svg`);
              }

              div.title = "hello";
              div.lang = "en-US";
              div.autocapitalize = "WORDS";
              div.translate = false;
              div.dir = "RTL";
              div.hidden = true;
              div.accessKey = "x";
              div.draggable = true;
              div.spellcheck = false;
              div.contentEditable = "plaintext-only";
              div.enterKeyHint = "Go";
              div.inputMode = "NUMERIC";
              div.innerText = "hello text";
              div.popover = "hint";
              div.autofocus = true;
              div.tabIndex = 5;

              assert(div.title === "hello", "title behavior");
              assert(div.lang === "en-US", "lang behavior");
              assert(div.autocapitalize === "words" && div.getAttribute("autocapitalize") === "WORDS", "autocapitalize behavior");
              assert(div.translate === false && div.getAttribute("translate") === "no", "translate behavior");
              assert(div.dir === "rtl", "dir behavior");
              assert(div.hidden === true && div.hasAttribute("hidden"), "hidden behavior");
              assert(div.accessKey === "x", "accessKey behavior");
              assert(div.draggable === true && div.getAttribute("draggable") === "true", "draggable behavior");
              assert(div.spellcheck === false && div.getAttribute("spellcheck") === "false", "spellcheck behavior");
              assert(div.contentEditable === "plaintext-only" && div.isContentEditable === true, "contentEditable behavior");
              assert(div.enterKeyHint === "go", "enterKeyHint behavior");
              assert(div.inputMode === "numeric", "inputMode behavior");
              assert(div.innerText === "hello text" && div.outerText === "hello text", "innerText/outerText behavior");
              assert(div.popover === "hint", "popover behavior");
              assert(div.autofocus === true && div.tabIndex === 5, "HTMLOrForeignElement behavior");

              const holder = document.createElement("section");
              const para = document.createElement("p");
              para.textContent = "old";
              holder.appendChild(para);
              para.outerText = "new";
              assert(holder.textContent === "new" && holder.firstChild.nodeType === Node.TEXT_NODE, "outerText setter behavior");

              svg.tabIndex = 9;
              svg.autofocus = true;
              assert(svg.tabIndex === 9 && svg.getAttribute("tabindex") === "9", "svg tabIndex behavior");
              assert(svg.autofocus === true && svg.hasAttribute("autofocus"), "svg autofocus behavior");
              return "ok";
            })()
            "#,
        )
        .expect("HTMLElement standard accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn document_state_and_collection_accessors_live_on_document_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/path/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (name, hasSetter = false) => {
                const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
                assert(!!descriptor, `${name} descriptor`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
                return descriptor;
              };

              const names = [
                "fonts",
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
                assert(!own(document, name), `${name} should not be own before use`);
              }

              document.body.innerHTML = [
                "<form id='f'></form>",
                "<img id='i'>",
                "<script id='s'></script>",
                "<a id='href' href='/x'></a>",
                "<a id='named' name='anchor'></a>",
                "<embed id='e'>"
              ].join("");

              const fonts = document.fonts;
              document.domain = "example.com";
              for (const name of names) {
                assert(!own(document, name), `${name} should not become own`);
              }

              const xml = document.implementation.createDocument("urn:test", "root", null);
              assert(!own(xml, "images"), "xml images should not be own");
              assert(xml.images === undefined, "xml images value");
              assert(xml.hidden === false, "xml hidden value");
              assert(xml.visibilityState === "visible", "xml visibility value");

              return [
                Object.prototype.toString.call(fonts),
                document.currentScript === null,
                document.hidden,
                document.visibilityState,
                document.prerendering,
                document.domain,
                document.scrollingElement === document.documentElement,
                document.forms.length,
                document.images.length,
                document.scripts.length,
                document.links.length,
                document.anchors.length,
                document.embeds.length,
                document.plugins.length,
                document.applets.length
              ].join("|");
            })()
            "#,
        )
        .expect("Document state and collection accessor prototype probe should evaluate");

    assert_eq!(
        result,
        "[object FontFaceSet]|true|false|visible|false|example.com|true|1|1|1|1|1|1|1|0"
    );
}

#[test]
fn geometry_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/path/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name, hasSetter) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const elementGeometry = [
                ["clientWidth", false],
                ["clientHeight", false],
                ["clientTop", false],
                ["clientLeft", false],
                ["scrollWidth", false],
                ["scrollHeight", false],
                ["scrollTop", true],
                ["scrollLeft", true]
              ];
              const htmlGeometry = [
                ["offsetWidth", false],
                ["offsetHeight", false],
                ["offsetParent", false],
                ["offsetTop", false],
                ["offsetLeft", false]
              ];

              for (const [name, hasSetter] of elementGeometry) {
                accessor(Element.prototype, name, hasSetter);
                assert(!own(HTMLElement.prototype, name), `${name} duplicated on HTMLElement.prototype`);
                assert(!own(HTMLDivElement.prototype, name), `${name} duplicated on HTMLDivElement.prototype`);
                assert(!own(SVGElement.prototype, name), `${name} duplicated on SVGElement.prototype`);
              }
              for (const [name, hasSetter] of htmlGeometry) {
                accessor(HTMLElement.prototype, name, hasSetter);
                assert(!own(Element.prototype, name), `${name} duplicated on Element.prototype`);
                assert(!own(HTMLDivElement.prototype, name), `${name} duplicated on HTMLDivElement.prototype`);
                assert(!own(SVGElement.prototype, name), `${name} duplicated on SVGElement.prototype`);
              }

              const div = document.createElement("div");
              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
              document.body.append(div, svg);
              for (const [name] of elementGeometry.concat(htmlGeometry)) {
                assert(!own(div, name), `${name} should not be own on div`);
              }
              for (const [name] of elementGeometry) {
                assert(!own(svg, name), `${name} should not be own on svg`);
              }

              div.scrollTop = 12;
              div.scrollLeft = 7;
              assert(div.scrollTop === 0 && div.scrollLeft === 0, "non-scrollable element stays at zero");
              assert(Number.isInteger(div.clientWidth), "clientWidth behavior");
              assert(Number.isInteger(div.clientHeight), "clientHeight behavior");
              assert(Number.isInteger(div.scrollWidth), "scrollWidth behavior");
              assert(Number.isInteger(div.scrollHeight), "scrollHeight behavior");
              assert(Number.isInteger(div.offsetWidth), "offsetWidth behavior");
              assert(Number.isInteger(div.offsetHeight), "offsetHeight behavior");
              assert(Number.isInteger(div.offsetTop), "offsetTop behavior");
              assert(Number.isInteger(div.offsetLeft), "offsetLeft behavior");
              assert(div.offsetParent === null || div.offsetParent instanceof Element, "offsetParent behavior");
              assert(Number.isInteger(svg.clientWidth), "svg clientWidth behavior");
              assert(typeof svg.offsetWidth === "undefined", "svg should not expose HTMLElement offsets");
              const detachedDocument = new DOMParser().parseFromString("<div></div>", "text/html");
              const detachedDiv = detachedDocument.querySelector("div");
              for (const [name] of elementGeometry.concat(htmlGeometry)) {
                assert(!own(detachedDiv, name), `${name} should not be own on detached div`);
              }
              assert(Number.isInteger(detachedDiv.clientWidth), "detached clientWidth behavior");
              assert(Number.isInteger(detachedDiv.scrollWidth), "detached scrollWidth behavior");
              assert(Number.isInteger(detachedDiv.offsetWidth), "detached offsetWidth behavior");
              assert(Number.isInteger(detachedDiv.offsetTop), "detached offsetTop behavior");
              return "ok";
            })()
            "#,
        )
        .expect("geometry accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_specialized_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://svg-specialized-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on ${prototype.constructor?.name || "prototype"}`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(descriptor.set === undefined, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
                return descriptor.get;
              };
              const assertNoOwn = (object, names, label) => {
                for (const name of names) {
                  assert(!own(object, name), `${label}.${name} should not be own before use`);
                  assert(!Object.keys(object).includes(name), `${label}.${name} should not be enumerable own`);
                }
              };
              const assertStaysInherited = (object, names, label) => {
                for (const name of names) {
                  const before = object[name];
                  assert(!own(object, name), `${label}.${name} should not be own after read`);
                  assert(delete object[name], `${label}.${name} delete`);
                  object[name] = "shadow";
                  assert(object[name] === before, `${label}.${name} after assignment`);
                  assert(!own(object, name), `${label}.${name} should not become own`);
                }
              };

              const ns = "http://www.w3.org/2000/svg";
              const rect = document.createElementNS(ns, "rect");
              const path = document.createElementNS(ns, "path");
              const text = document.createElementNS(ns, "text");
              const pattern = document.createElementNS(ns, "pattern");
              const linear = document.createElementNS(ns, "linearGradient");
              const radial = document.createElementNS(ns, "radialGradient");

              const transform = accessor(SVGGraphicsElement.prototype, "transform");
              const pathLength = accessor(SVGGeometryElement.prototype, "pathLength");
              const textLength = accessor(SVGTextContentElement.prototype, "textLength");
              const lengthAdjust = accessor(SVGTextContentElement.prototype, "lengthAdjust");
              const textX = accessor(SVGTextPositioningElement.prototype, "x");
              for (const name of ["y", "dx", "dy", "rotate"]) {
                accessor(SVGTextPositioningElement.prototype, name);
              }
              const patternTransform = accessor(SVGPatternElement.prototype, "patternTransform");
              const gradientTransform = accessor(SVGGradientElement.prototype, "gradientTransform");
              const rectX = accessor(SVGRectElement.prototype, "x");
              for (const name of ["y", "width", "height", "rx", "ry"]) {
                accessor(SVGRectElement.prototype, name);
              }

              assert(!own(SVGRectElement.prototype, "pathLength"), "pathLength inherited by SVGRectElement");
              assert(!own(SVGRectElement.prototype, "transform"), "transform inherited by SVGRectElement");
              assert(!own(SVGTextElement.prototype, "x"), "x inherited by SVGTextElement");
              assert(!own(SVGLinearGradientElement.prototype, "gradientTransform"),
                "gradientTransform inherited by SVGLinearGradientElement");

              assertNoOwn(rect, ["x", "y", "width", "height", "rx", "ry", "pathLength", "transform"], "rect");
              assertNoOwn(path, ["pathLength", "transform"], "path");
              assertNoOwn(text, ["textLength", "lengthAdjust", "x", "y", "dx", "dy", "rotate", "transform"], "text");
              assertNoOwn(pattern, ["patternTransform"], "pattern");
              assertNoOwn(linear, ["gradientTransform"], "linearGradient");
              assertNoOwn(radial, ["gradientTransform"], "radialGradient");

              rect.setAttribute("x", "13");
              path.setAttribute("pathLength", "7");
              text.setAttribute("x", "1 2");
              pattern.setAttribute("patternTransform", "translate(3)");
              linear.setAttribute("gradientTransform", "scale(2)");

              assert(rectX.call(rect) === rect.x, "rect x getter identity");
              assert(pathLength.call(path) === path.pathLength, "pathLength getter identity");
              assert(transform.call(rect) === rect.transform, "transform getter identity");
              assert(textLength.call(text) === text.textLength, "textLength getter identity");
              assert(lengthAdjust.call(text) === text.lengthAdjust, "lengthAdjust getter identity");
              assert(textX.call(text) === text.x, "text x getter identity");
              assert(patternTransform.call(pattern) === pattern.patternTransform, "patternTransform getter identity");
              assert(gradientTransform.call(linear) === linear.gradientTransform, "gradientTransform getter identity");

              assert(rect.x.baseVal.value === 13, "rect x reflects attribute");
              assert(path.pathLength.baseVal === 7, "pathLength reflects attribute");
              assert(text.x.baseVal.numberOfItems === 2, "text x reflects list");
              assert(pattern.patternTransform.baseVal.numberOfItems === 1, "pattern transform reflects list");
              assert(linear.gradientTransform.baseVal.numberOfItems === 1, "linear gradient transform reflects list");
              assert(radial.gradientTransform.baseVal.numberOfItems === 0, "radial gradient default transform");

              assertStaysInherited(rect, ["x", "y", "width", "height", "rx", "ry", "pathLength", "transform"], "rect");
              assertStaysInherited(text, ["textLength", "lengthAdjust", "x", "y", "dx", "dy", "rotate"], "text");
              assertStaysInherited(pattern, ["patternTransform"], "pattern");
              assertStaysInherited(linear, ["gradientTransform"], "linearGradient");
              return "ok";
            })()
            "#,
        )
        .expect("SVG specialized accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn element_methods_and_dataset_live_on_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) {
                  throw new Error(message);
                }
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const method = (prototype, name, length, enumerable) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.value === "function", `${name} method`);
                assert(descriptor.value.length === length, `${name} length`);
                assert(descriptor.enumerable === enumerable, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
                assert(descriptor.writable === true, `${name} writable`);
              };
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(descriptor.set === undefined, `${name} readonly`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const elementMethods = new Map([
                ["getBoundingClientRect", 0],
                ["getClientRects", 0],
                ["hasAttribute", 1],
                ["hasAttributeNS", 2],
                ["getAttribute", 1],
                ["getAttributeNS", 2],
                ["setAttribute", 2],
                ["setAttributeNS", 3],
                ["removeAttribute", 1],
                ["removeAttributeNS", 2],
                ["closest", 1],
                ["getElementsByTagName", 1],
                ["getElementsByTagNameNS", 2],
                ["getElementsByClassName", 1],
                ["getAttributeNames", 0],
                ["hasAttributes", 0],
                ["toggleAttribute", 1]
              ]);
              for (const [name, length] of elementMethods) {
                method(Element.prototype, name, length, true);
              }
              method(Element.prototype, "getElementsByName", 1, false);

              const extendedMethods = new Map([
                ["getAttributeNode", 1],
                ["getAttributeNodeNS", 2],
                ["setAttributeNode", 1],
                ["setAttributeNodeNS", 1],
                ["removeAttributeNode", 1],
                ["insertAdjacentElement", 2],
                ["insertAdjacentText", 2],
                ["insertAdjacentHTML", 2],
                ["__moliInsertAdjacentNode", 0]
              ]);
              for (const [name, length] of extendedMethods) {
                method(Element.prototype, name, length, true);
              }

              const actionMethods = [
                "focus",
                "blur",
                "click",
                "showPopover",
                "hidePopover",
                "togglePopover"
              ];
              for (const name of actionMethods) {
                method(HTMLElement.prototype, name, 0, true);
                assert(!own(Element.prototype, name), `${name} should not live on Element.prototype`);
              }
              method(HTMLElement.prototype, "scrollIntoViewIfNeeded", 0, false);

              for (const prototype of [HTMLElement.prototype, SVGElement.prototype, MathMLElement.prototype]) {
                accessor(prototype, "dataset");
              }
              assert(!own(Element.prototype, "dataset"), "dataset duplicated on Element.prototype");

              const host = document.createElement("section");
              host.innerHTML = '<p id="child" class="item" name="field"></p>';
              const child = host.firstElementChild;

              for (const name of elementMethods.keys()) {
                assert(!own(child, name), `${name} should not be own on child`);
              }
              for (const name of Array.from(extendedMethods.keys()).concat(actionMethods, ["getElementsByName", "scrollIntoViewIfNeeded", "dataset"])) {
                assert(!own(child, name), `${name} should not be own on child`);
              }

              child.setAttribute("data-token", "one");
              child.setAttributeNS("urn:test", "t:flag", "yes");
              assert(child.getAttribute("data-token") === "one", "getAttribute behavior");
              assert(child.getAttributeNS("urn:test", "flag") === "yes", "getAttributeNS behavior");
              assert(child.hasAttribute("data-token") && child.hasAttributeNS("urn:test", "flag"), "hasAttribute behavior");
              assert(child.getAttributeNames().includes("data-token"), "getAttributeNames behavior");
              assert(child.hasAttributes(), "hasAttributes behavior");
              assert(child.toggleAttribute("hidden") === true && child.hasAttribute("hidden"), "toggleAttribute behavior");
              child.removeAttribute("hidden");
              child.removeAttributeNS("urn:test", "flag");
              assert(!child.hasAttribute("hidden") && !child.hasAttributeNS("urn:test", "flag"), "removeAttribute behavior");
              assert(host.getElementsByTagName("p").length === 1, "getElementsByTagName behavior");
              assert(host.getElementsByClassName("item").length === 1, "getElementsByClassName behavior");
              assert(host.getElementsByName("field").length === 1, "getElementsByName behavior");
              assert(child.closest("section") === host, "closest behavior");

              const attr = document.createAttribute("data-node");
              attr.value = "node";
              child.setAttributeNode(attr);
              assert(child.getAttributeNode("data-node") === attr, "attributeNode behavior");
              child.insertAdjacentText("beforeend", "txt");
              child.insertAdjacentHTML("beforeend", "<span></span>");
              assert(child.textContent === "txt" && child.lastElementChild.localName === "span", "insertAdjacent behavior");

              child.dataset.fooBar = "baz";
              assert(child.getAttribute("data-foo-bar") === "baz", "html dataset behavior");
              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
              assert(!own(svg, "dataset"), "dataset should not be own on svg");
              svg.dataset.iconName = "home";
              assert(svg.getAttribute("data-icon-name") === "home", "svg dataset behavior");

              const fragment = document.createDocumentFragment();
              const fragmentChild = document.createElement("a");
              fragmentChild.className = "fragment-link";
              fragmentChild.setAttribute("name", "fragment-name");
              fragmentChild.id = "fragment-id";
              fragment.appendChild(fragmentChild);
              method(Document.prototype, "getElementById", 1, false);
              method(DocumentFragment.prototype, "getElementById", 1, false);
              assert(!own(document, "getElementById"), "getElementById should not be own on document");
              assert(!own(fragment, "getElementById"), "getElementById should not be own on fragment");
              method(DocumentFragment.prototype, "getElementsByTagName", 1, false);
              method(ShadowRoot.prototype, "getElementsByTagName", 1, false);
              assert(!own(fragment, "getElementsByTagName"), "getElementsByTagName should not be own on fragment");
              assert(fragment.getElementById("fragment-id") === fragmentChild, "fragment getElementById behavior");
              assert(fragment.getElementsByTagName("a").length === 1, "fragment getElementsByTagName behavior");
              assert(fragment.getElementsByClassName("fragment-link").length === 1, "fragment getElementsByClassName behavior");
              assert(fragment.getElementsByName("fragment-name").length === 1, "fragment getElementsByName behavior");

              const shadowHost = document.createElement("div");
              const shadowRoot = shadowHost.attachShadow({ mode: "open" });
              shadowRoot.innerHTML = '<a id="shadow-id" class="shadow-link" name="shadow-name"></a>';
              assert(!own(shadowRoot, "getElementById"), "getElementById should not be own on shadow root");
              assert(!own(shadowRoot, "getElementsByTagName"), "getElementsByTagName should not be own on shadow root");
              assert(shadowRoot.getElementById("shadow-id")?.localName === "a", "shadow getElementById behavior");
              assert(shadowRoot.getElementsByTagName("a").length === 1, "shadow getElementsByTagName behavior");
              assert(shadowRoot.getElementsByClassName("shadow-link").length === 1, "shadow getElementsByClassName behavior");
              assert(shadowRoot.getElementsByName("shadow-name").length === 1, "shadow getElementsByName behavior");

              return "ok";
            })()
            "#,
        )
        .expect("Element methods and dataset prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn specialized_element_methods_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><form id='f'><input name='q'><select><option>a</option></select><table><tbody><tr></tr></tbody></table></form></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const method = (prototype, name, length, enumerable = true) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.value === "function", `${name} function`);
                assert(descriptor.value.length === length, `${name} length`);
                assert(descriptor.enumerable === enumerable, `${name} enumerable`);
                assert(descriptor.writable === true, `${name} writable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(descriptor.set === undefined, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              for (const [name, length] of [
                ["requestSubmit", 1],
                ["submit", 0],
                ["reset", 0],
                ["checkValidity", 0],
                ["reportValidity", 0]
              ]) {
                method(HTMLFormElement.prototype, name, length);
              }
              for (const [name, length] of [
                ["play", 0],
                ["pause", 0],
                ["load", 0],
                ["canPlayType", 1],
                ["addTextTrack", 1]
              ]) {
                method(HTMLMediaElement.prototype, name, length);
                assert(!own(HTMLAudioElement.prototype, name), `${name} duplicated on audio prototype`);
                assert(!own(HTMLVideoElement.prototype, name), `${name} duplicated on video prototype`);
              }
              method(HTMLImageElement.prototype, "decode", 0);
              for (const [name, length] of [
                ["showPicker", 0],
                ["stepUp", 0],
                ["stepDown", 0]
              ]) {
                method(HTMLInputElement.prototype, name, length);
              }
              for (const prototype of [HTMLInputElement.prototype, HTMLTextAreaElement.prototype]) {
                for (const [name, length] of [
                  ["setSelectionRange", 2],
                  ["setRangeText", 1],
                  ["select", 0]
                ]) {
                  method(prototype, name, length);
                }
              }
              for (const [name, length] of [
                ["add", 1],
                ["item", 1],
                ["namedItem", 1],
                ["remove", 0]
              ]) {
                method(HTMLSelectElement.prototype, name, length);
              }
              for (const prototype of [
                HTMLButtonElement.prototype,
                HTMLInputElement.prototype,
                HTMLMeterElement.prototype,
                HTMLOutputElement.prototype,
                HTMLProgressElement.prototype,
                HTMLSelectElement.prototype,
                HTMLTextAreaElement.prototype
              ]) {
                accessor(prototype, "labels");
              }
              for (const prototype of [
                HTMLButtonElement.prototype,
                HTMLFieldSetElement.prototype,
                HTMLInputElement.prototype,
                HTMLObjectElement.prototype,
                HTMLOutputElement.prototype,
                HTMLSelectElement.prototype,
                HTMLTextAreaElement.prototype
              ]) {
                for (const name of ["validity", "validationMessage", "willValidate"]) {
                  accessor(prototype, name);
                }
                for (const [name, length] of [
                  ["checkValidity", 0],
                  ["reportValidity", 0],
                  ["setCustomValidity", 1]
                ]) {
                  method(prototype, name, length);
                }
              }
              for (const [name, length] of [
                ["insertRow", 0],
                ["deleteRow", 1]
              ]) {
                method(HTMLTableSectionElement.prototype, name, length);
              }
              for (const [name, length] of [
                ["insertCell", 0],
                ["deleteCell", 1]
              ]) {
                method(HTMLTableRowElement.prototype, name, length);
              }

              const form = document.querySelector("form");
              const button = document.createElement("button");
              const fieldset = document.createElement("fieldset");
              const input = document.querySelector("input");
              const objectElement = document.createElement("object");
              const output = document.createElement("output");
              const textarea = document.createElement("textarea");
              const meter = document.createElement("meter");
              const progress = document.createElement("progress");
              const select = document.querySelector("select");
              const tbody = document.querySelector("tbody");
              const row = document.querySelector("tr");
              const image = document.createElement("img");
              const audio = document.createElement("audio");
              const video = document.createElement("video");
              form.append(button, fieldset, objectElement, output, textarea, meter, progress);

              for (const [object, names] of [
                [form, ["requestSubmit", "submit", "reset", "checkValidity", "reportValidity"]],
                [button, ["labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [fieldset, ["validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [input, ["showPicker", "stepUp", "stepDown", "setSelectionRange", "setRangeText", "select", "labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [objectElement, ["validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [output, ["labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [textarea, ["setSelectionRange", "setRangeText", "select", "labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [meter, ["labels"]],
                [progress, ["labels"]],
                [select, ["add", "item", "namedItem", "remove", "labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [tbody, ["insertRow", "deleteRow"]],
                [row, ["insertCell", "deleteCell"]],
                [image, ["decode"]],
                [audio, ["play", "pause", "load", "canPlayType", "addTextTrack"]],
                [video, ["play", "pause", "load", "canPlayType", "addTextTrack"]]
              ]) {
                for (const name of names) {
                  assert(!own(object, name), `${name} should not be own on instance`);
                }
              }

              input.type = "number";
              input.value = "2";
              input.stepUp();
              assert(input.value === "3", "input stepUp behavior");
              textarea.value = "abcd";
              textarea.setSelectionRange(1, 3);
              textarea.setRangeText("XY");
              assert(textarea.value === "aXYd", "text control behavior");
              const option = document.createElement("option");
              option.value = "b";
              option.text = "b";
              select.add(option);
              assert(select.item(1) === option && select.namedItem("q") === null, "select methods behavior");
              select.remove(1);
              assert(select.length === 1, "select remove behavior");
              const detachedSelectDocument = new DOMParser().parseFromString(
                "<select><option id='first' value='a'>A</option></select>",
                "text/html"
              );
              const detachedSelect = detachedSelectDocument.querySelector("select");
              for (const name of ["length", "options", "selectedOptions", "selectedIndex", "value", "add", "item", "namedItem", "remove"]) {
                assert(!own(detachedSelect, name), `${name} should not be own on detached select`);
              }
              const detachedOption = detachedSelectDocument.createElement("option");
              detachedOption.id = "second";
              detachedOption.setAttribute("value", "b");
              detachedOption.text = "B";
              detachedSelect.add(detachedOption);
              detachedSelect.value = "b";
              assert(detachedSelect.length === 2, "detached select length behavior");
              assert(detachedSelect.options.length === 2, "detached select options behavior");
              const detachedItem = detachedSelect.item(1);
              assert(detachedItem?.id === "second" && detachedItem?.value === "b", "detached select item behavior");
              assert(detachedSelect.namedItem("second")?.value === "b", "detached select namedItem behavior");
              assert(detachedSelect.selectedIndex === 1 && detachedSelect.selectedOptions.length === 1, "detached select selected behavior");
              detachedSelect.remove(0);
              assert(detachedSelect.length === 1 && detachedSelect.item(0)?.id === "second", "detached select remove behavior");
              const insertedRow = tbody.insertRow();
              assert(insertedRow.parentNode === tbody, "section insertRow behavior");
              tbody.deleteRow(1);
              const cell = row.insertCell();
              assert(cell.parentNode === row, "row insertCell behavior");
              row.deleteCell(0);
              assert(row.cells.length === 0, "row deleteCell behavior");
              assert(form.checkValidity() === true && input.checkValidity() === true, "validation behavior");
              assert(input.validity.valid === true && input.validationMessage === "" && input.willValidate === true, "validation accessor behavior");
              assert(input.labels.length === 0 && meter.labels.length === 0, "labels behavior");
              assert(typeof audio.canPlayType("audio/mpeg") === "string" && typeof image.decode().then === "function", "media/image behavior");
              return "ok";
            })()
            "#,
        )
        .expect("specialized element method prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_specialized_element_surfaces_are_inherited() {
    let mut vm = new_parsed_test_vm(
        "https://detached-specialized-owner-prototypes.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const probeInherited = (element, names, label) => {
    for (const name of names) {
      assert(name in element, `${label}.${name} missing`);
      assert(!own(element, name), `${label}.${name} should not be own before access`);
      void element[name];
      assert(!own(element, name), `${label}.${name} should not be own after access`);
    }
  };

  const doc = new DOMParser().parseFromString(`
    <!doctype html>
    <html>
      <head>
        <base id="base">
        <link id="link">
        <meta id="meta">
        <title id="title">Title</title>
      </head>
      <body id="body">
        <form id="form">
          <button id="button"></button>
          <fieldset id="fieldset"><legend id="legend"></legend><input id="field"></fieldset>
          <input id="input" list="choices">
          <datalist id="choices"><option id="data-option"></option></datalist>
          <select id="select"><option id="option">Choice</option></select>
          <textarea id="textarea">Text</textarea>
          <output id="output"></output>
          <object id="object"><param id="param"></object>
          <meter id="meter"></meter>
          <progress id="progress"></progress>
        </form>
        <a id="anchor"></a>
        <area id="area">
        <audio id="audio"></audio>
        <blockquote id="blockquote"></blockquote>
        <data id="data"></data>
        <del id="del"></del>
        <details id="details"></details>
        <dir id="dir"></dir>
        <dl id="dl"></dl>
        <embed id="embed">
        <font id="font"></font>
        <frame id="frame">
        <iframe id="iframe"></iframe>
        <hr id="hr">
        <img id="image">
        <ins id="ins"></ins>
        <label id="label" for="input"></label>
        <li id="li"></li>
        <map id="map"></map>
        <marquee id="marquee"></marquee>
        <menu id="menu"></menu>
        <ol id="ol"></ol>
        <optgroup id="optgroup"></optgroup>
        <q id="q"></q>
        <slot id="slot"></slot>
        <source id="source">
        <table id="table"><tbody id="tbody"><tr id="row"><td id="cell"></td></tr></tbody></table>
        <template id="template"><span></span></template>
        <time id="time"></time>
        <track id="track">
        <ul id="ul"></ul>
        <video id="video"></video>
      </body>
    </html>
  `, "text/html");
  const id = name => doc.getElementById(name);
  const detachedFrame = id("frame") || doc.createElement("frame");

  const cases = [
    [doc.documentElement, ["version"], "html"],
    [id("body"), ["onload", "text", "link", "vLink", "aLink", "background"], "body"],
    [id("anchor"), ["href", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "target", "download", "rel", "relList", "name", "text"], "anchor"],
    [id("area"), ["href", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "target", "download", "rel", "relList", "alt"], "area"],
    [id("audio"), ["preload", "play", "pause", "load", "canPlayType", "addTextTrack"], "audio"],
    [id("base"), ["target"], "base"],
    [id("blockquote"), ["cite"], "blockquote"],
    [id("button"), ["disabled", "form", "formAction", "formEnctype", "formMethod", "formNoValidate", "formTarget", "labels", "name", "type", "validity", "validationMessage", "value", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "button"],
    [id("data"), ["value"], "data"],
    [id("del"), ["cite", "dateTime"], "del"],
    [id("details"), ["name", "open"], "details"],
    [id("dir"), ["compact"], "dir"],
    [id("dl"), ["compact"], "dl"],
    [id("embed"), ["name"], "embed"],
    [id("fieldset"), ["disabled", "elements", "form", "name", "type", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "fieldset"],
    [id("font"), ["color"], "font"],
    [id("form"), ["acceptCharset", "action", "autocomplete", "elements", "encoding", "enctype", "length", "method", "name", "noValidate", "rel", "relList", "target", "requestSubmit", "submit", "reset", "checkValidity", "reportValidity"], "form"],
    [detachedFrame, ["frameBorder", "longDesc", "marginHeight", "marginWidth", "name", "scrolling"], "frame"],
    [id("hr"), ["color", "noShade"], "hr"],
    [id("iframe"), ["contentDocument", "contentWindow", "frameBorder", "longDesc", "marginHeight", "marginWidth", "name", "scrolling", "src", "srcdoc"], "iframe"],
    [id("image"), ["alt", "border", "decode", "decoding", "height", "hspace", "longDesc", "lowsrc", "name", "src", "srcset", "useMap", "vspace", "width"], "image"],
    [id("input"), ["accept", "alt", "autocomplete", "checked", "defaultChecked", "defaultValue", "dirName", "disabled", "files", "form", "formAction", "formEnctype", "formMethod", "formNoValidate", "formTarget", "height", "indeterminate", "labels", "list", "max", "maxLength", "min", "minLength", "multiple", "name", "pattern", "placeholder", "readOnly", "required", "size", "src", "step", "type", "validity", "validationMessage", "value", "valueAsDate", "valueAsNumber", "willValidate", "width", "checkValidity", "reportValidity", "setCustomValidity", "select", "setRangeText", "setSelectionRange", "showPicker", "stepDown", "stepUp"], "input"],
    [id("ins"), ["cite", "dateTime"], "ins"],
    [id("label"), ["control", "form", "htmlFor"], "label"],
    [id("legend"), ["form"], "legend"],
    [id("li"), ["value"], "li"],
    [id("link"), ["media", "rel", "relList", "target"], "link"],
    [id("map"), ["name"], "map"],
    [id("marquee"), ["bgColor", "hspace", "vspace"], "marquee"],
    [id("menu"), ["compact"], "menu"],
    [id("meta"), ["content", "httpEquiv", "media", "name"], "meta"],
    [id("meter"), ["high", "labels", "low", "max", "min", "optimum", "value"], "meter"],
    [id("object"), ["archive", "border", "code", "codeBase", "codeType", "data", "declare", "form", "hspace", "name", "standby", "type", "useMap", "validity", "validationMessage", "vspace", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "object"],
    [id("ol"), ["compact", "reversed", "start", "type"], "ol"],
    [id("optgroup"), ["disabled", "label"], "optgroup"],
    [id("option"), ["defaultSelected", "disabled", "form", "index", "label", "selected", "text", "value"], "option"],
    [id("output"), ["defaultValue", "form", "labels", "name", "type", "validity", "validationMessage", "value", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "output"],
    [id("param"), ["name", "type", "value", "valueType"], "param"],
    [id("progress"), ["labels", "max", "position", "value"], "progress"],
    [id("q"), ["cite"], "q"],
    [id("select"), ["autocomplete", "disabled", "form", "labels", "length", "multiple", "name", "options", "required", "selectedIndex", "selectedOptions", "size", "validity", "validationMessage", "value", "willValidate", "add", "checkValidity", "item", "namedItem", "remove", "reportValidity", "setCustomValidity"], "select"],
    [id("slot"), ["name"], "slot"],
    [id("source"), ["media", "srcset"], "source"],
    [id("table"), ["bgColor", "border", "caption", "rows", "tBodies", "tFoot", "tHead"], "table"],
    [id("tbody"), ["rows"], "tbody"],
    [id("row"), ["bgColor", "cells", "rowIndex", "sectionRowIndex"], "row"],
    [id("cell"), ["bgColor", "cellIndex", "colSpan", "rowSpan"], "cell"],
    [id("template"), ["content"], "template"],
    [id("textarea"), ["autocomplete", "cols", "defaultValue", "dirName", "disabled", "form", "labels", "maxLength", "minLength", "name", "placeholder", "readOnly", "required", "rows", "textLength", "type", "validity", "validationMessage", "value", "willValidate", "wrap", "checkValidity", "reportValidity", "select", "setCustomValidity", "setRangeText", "setSelectionRange"], "textarea"],
    [id("time"), ["dateTime"], "time"],
    [id("track"), ["label"], "track"],
    [id("ul"), ["compact"], "ul"],
    [id("video"), ["preload", "play", "pause", "load", "canPlayType", "addTextTrack"], "video"]
  ];

  for (const [element, names, label] of cases) {
    probeInherited(element, names, label);
  }
  return "ok";
})()
"#,
        )
        .expect("detached specialized owner prototype inventory should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_form_and_target_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name, hasSetter = true) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
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

              const formNames = [
                ["action", true],
                ["acceptCharset", true],
                ["autocomplete", true],
                ["enctype", true],
                ["encoding", true],
                ["elements", false],
                ["length", false],
                ["method", true],
                ["name", true],
                ["noValidate", true],
                ["target", true]
              ];
              for (const [name, hasSetter] of formNames) {
                accessor(HTMLFormElement.prototype, name, hasSetter);
              }

              const form = document.createElement("form");
              form.innerHTML = "<input name='q'><button name='go'></button>";
              document.body.append(form);
              for (const name of [
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
              ]) {
                assert(!own(form, name), `${name} should not be own on form`);
              }
              form.action = "/submit";
              form.acceptCharset = "utf-8";
              form.autocomplete = "off";
              form.enctype = "multipart/form-data";
              assert(form.enctype === "multipart/form-data", "form enctype behavior");
              form.encoding = "text/plain";
              form.method = "post";
              form.name = "search";
              form.noValidate = true;
              form.target = "_blank";
              assert(form.action === "https://example.com/submit", "form action behavior");
              assert(form.acceptCharset === "utf-8", "form acceptCharset behavior");
              assert(form.encoding === "text/plain", "form encoding behavior");
              assert(form.method === "post", "form method behavior");
              assert(form.name === "search", "form name behavior");
              assert(form.noValidate === true, "form noValidate behavior");
              assert(form.target === "_blank", "form target behavior");
              assert(form.elements.length === 2 && form.length === 2, "form collection behavior");

              const associationHost = document.createElement("form");
              associationHost.id = "owner";
              associationHost.innerHTML = `
                <button></button>
                <fieldset></fieldset>
                <input>
                <object></object>
                <output></output>
                <select></select>
                <textarea></textarea>
              `;
              document.body.append(associationHost);
              const formAssociatedOwners = [
                [HTMLButtonElement.prototype, associationHost.querySelector("button"), "button"],
                [HTMLFieldSetElement.prototype, associationHost.querySelector("fieldset"), "fieldset"],
                [HTMLInputElement.prototype, associationHost.querySelector("input"), "input"],
                [HTMLObjectElement.prototype, associationHost.querySelector("object"), "object"],
                [HTMLOutputElement.prototype, associationHost.querySelector("output"), "output"],
                [HTMLSelectElement.prototype, associationHost.querySelector("select"), "select"],
                [HTMLTextAreaElement.prototype, associationHost.querySelector("textarea"), "textarea"]
              ];
              for (const [prototype, element, label] of formAssociatedOwners) {
                accessor(prototype, "form", false);
                assert(!own(element, "form"), `${label} form should not be own`);
                assert(element.form === associationHost, `${label} form owner`);
                element.form = null;
                assert(!own(element, "form"), `${label} form assignment should not create own`);
                assert(element.form === associationHost, `${label} form after assignment`);
                assert(delete element.form, `${label} form delete`);
                assert(!own(element, "form"), `${label} form after delete`);
              }

              const explicitLabel = document.createElement("label");
              const explicitInput = document.createElement("input");
              const implicitLabel = document.createElement("label");
              const implicitInput = document.createElement("textarea");
              explicitInput.id = "label-target";
              explicitLabel.htmlFor = "label-target";
              implicitLabel.append("implicit", implicitInput);
              associationHost.append(explicitLabel, explicitInput, implicitLabel);
              accessor(HTMLLabelElement.prototype, "htmlFor", true);
              accessor(HTMLLabelElement.prototype, "control", false);
              accessor(HTMLLabelElement.prototype, "form", false);
              for (const label of [explicitLabel, implicitLabel]) {
                assert(!own(label, "htmlFor"), "label htmlFor should not be own");
                assert(!own(label, "control"), "label control should not be own");
                assert(!own(label, "form"), "label form should not be own");
              }
              assert(explicitLabel.htmlFor === "label-target", "label htmlFor behavior");
              assert(explicitLabel.control === explicitInput, "explicit label control");
              assert(implicitLabel.control === implicitInput, "implicit label control");
              assert(explicitLabel.form === associationHost, "explicit label form");
              assert(implicitLabel.form === associationHost, "implicit label form");
              explicitLabel.control = null;
              explicitLabel.form = null;
              assert(!own(explicitLabel, "control"), "label control assignment should not create own");
              assert(!own(explicitLabel, "form"), "label form assignment should not create own");
              assert(explicitLabel.control === explicitInput, "label control after assignment");
              assert(explicitLabel.form === associationHost, "label form after assignment");

              const autocompleteOwners = [
                [HTMLInputElement.prototype, document.createElement("input"), "input"],
                [HTMLSelectElement.prototype, document.createElement("select"), "select"],
                [HTMLTextAreaElement.prototype, document.createElement("textarea"), "textarea"]
              ];
              for (const [prototype, element, label] of autocompleteOwners) {
                accessor(prototype, "autocomplete", true);
                assert(!own(element, "autocomplete"), `${label} autocomplete should not be own`);
                element.autocomplete = " NAME\t";
                assert(element.getAttribute("autocomplete") === " NAME\t", `${label} autocomplete setter reflection`);
                assert(element.autocomplete === "name", `${label} autocomplete canonical getter`);
              }

              const targetOwners = [
                [HTMLAnchorElement.prototype, document.createElement("a"), "anchor"],
                [HTMLAreaElement.prototype, document.createElement("area"), "area"],
                [HTMLBaseElement.prototype, document.createElement("base"), "base"],
                [HTMLLinkElement.prototype, document.createElement("link"), "link"],
                [HTMLFormElement.prototype, form, "form"]
              ];
              const div = document.createElement("div");
              const text = document.createTextNode("x");
              const targetDescriptors = targetOwners.map(([prototype, element, label]) => [
                accessor(prototype, "target", true),
                element,
                label
              ]);
              for (const [descriptor, element, label] of targetDescriptors) {
                assert(!own(element, "target"), `${label} target should not be own`);
                descriptor.set.call(element, `${label}-target`);
                assert(element.target === `${label}-target`, `${label} target behavior`);
                assert(descriptor.get.call(element) === `${label}-target`, `${label} target direct getter`);
                assert(!own(element, "target"), `${label} target should stay inherited`);
                for (const receiver of [{}, text, div]) {
                  assert(throwsTypeError(() => descriptor.get.call(receiver)), `${label} target getter receiver`);
                  assert(throwsTypeError(() => descriptor.set.call(receiver, "bad")), `${label} target setter receiver`);
                }
                for (const [, otherElement, otherLabel] of targetDescriptors) {
                  if (otherElement === element) continue;
                  assert(throwsTypeError(() => descriptor.get.call(otherElement)), `${label} getter rejects ${otherLabel}`);
                  assert(throwsTypeError(() => descriptor.set.call(otherElement, "bad")), `${label} setter rejects ${otherLabel}`);
                }
              }
              assert(Object.getOwnPropertyDescriptor(HTMLElement.prototype, "form") === undefined, "HTMLElement form absent");
              assert(!("form" in div), "plain HTMLElement form absent");
              assert(Object.getOwnPropertyDescriptor(HTMLElement.prototype, "target") === undefined, "HTMLElement target absent");
              assert(!("target" in div), "plain HTMLElement target absent");
              assert(!("autocomplete" in div), "plain HTMLElement autocomplete absent");
              return "ok";
            })()
            "#,
        )
        .expect("form and target accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_rel_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const cases = [
                [HTMLAnchorElement.prototype, document.createElement("a"), "anchor"],
                [HTMLAreaElement.prototype, document.createElement("area"), "area"],
                [HTMLFormElement.prototype, document.createElement("form"), "form"],
                [HTMLLinkElement.prototype, document.createElement("link"), "link"]
              ];
              for (const [prototype] of cases) {
                accessor(prototype, "rel");
                accessor(prototype, "relList");
              }
              assert(!own(HTMLElement.prototype, "rel"), "rel should not be on HTMLElement.prototype");
              assert(!own(HTMLElement.prototype, "relList"), "relList should not be on HTMLElement.prototype");
              const div = document.createElement("div");
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
        .expect("rel owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn legacy_boolean_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const compactOwners = [
                [HTMLDirectoryElement.prototype, document.createElement("dir"), "dir"],
                [HTMLDListElement.prototype, document.createElement("dl"), "dl"],
                [HTMLMenuElement.prototype, document.createElement("menu"), "menu"],
                [HTMLOListElement.prototype, document.createElement("ol"), "ol"],
                [HTMLUListElement.prototype, document.createElement("ul"), "ul"]
              ];
              for (const [prototype] of compactOwners) {
                accessor(prototype, "compact");
              }
              accessor(HTMLHRElement.prototype, "noShade");
              assert(!own(HTMLElement.prototype, "compact"), "compact should not be on HTMLElement.prototype");
              assert(!own(HTMLElement.prototype, "noShade"), "noShade should not be on HTMLElement.prototype");
              const div = document.createElement("div");
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

              const hr = document.createElement("hr");
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
        .expect("legacy boolean owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_name_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const owners = [
                [HTMLAnchorElement.prototype, document.createElement("a"), "anchor"],
                [HTMLButtonElement.prototype, document.createElement("button"), "button"],
                [HTMLDetailsElement.prototype, document.createElement("details"), "details"],
                [HTMLEmbedElement.prototype, document.createElement("embed"), "embed"],
                [HTMLFieldSetElement.prototype, document.createElement("fieldset"), "fieldset"],
                [HTMLFormElement.prototype, document.createElement("form"), "form"],
                [HTMLFrameElement.prototype, document.createElement("frame"), "frame"],
                [HTMLIFrameElement.prototype, document.createElement("iframe"), "iframe"],
                [HTMLImageElement.prototype, document.createElement("img"), "image"],
                [HTMLInputElement.prototype, document.createElement("input"), "input"],
                [HTMLMapElement.prototype, document.createElement("map"), "map"],
                [HTMLMetaElement.prototype, document.createElement("meta"), "meta"],
                [HTMLObjectElement.prototype, document.createElement("object"), "object"],
                [HTMLOutputElement.prototype, document.createElement("output"), "output"],
                [HTMLParamElement.prototype, document.createElement("param"), "param"],
                [HTMLSelectElement.prototype, document.createElement("select"), "select"],
                [HTMLSlotElement.prototype, document.createElement("slot"), "slot"],
                [HTMLTextAreaElement.prototype, document.createElement("textarea"), "textarea"]
              ];

              for (const [prototype, element, label] of owners) {
                accessor(prototype, "name");
                assert(!own(element, "name"), `${label} name should not be own`);
                element.name = `${label}-name`;
                assert(element.name === `${label}-name`, `${label} name behavior`);
                assert(element.getAttribute("name") === `${label}-name`, `${label} name reflection`);
              }

              const div = document.createElement("div");
              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
              assert(Object.getOwnPropertyDescriptor(Element.prototype, "name") === undefined, "Element name absent");
              assert(Object.getOwnPropertyDescriptor(HTMLElement.prototype, "name") === undefined, "HTMLElement name absent");
              assert(!("name" in div), "plain HTMLElement name absent");
              assert(!("name" in svg), "SVGElement name absent");
              return "ok";
            })()
            "#,
        )
        .expect("HTML name accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_value_accessor_live_on_owner_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://button-value-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptor = Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, "value");
  assert(!!descriptor, "HTMLButtonElement.value descriptor missing");
  assert(typeof descriptor.get === "function", "value getter");
  assert(typeof descriptor.set === "function", "value setter");
  assert(descriptor.enumerable === true, "value enumerable");
  assert(descriptor.configurable === true, "value configurable");
  assert(!own(HTMLElement.prototype, "value"), "value should not live on HTMLElement");
  assert(!("value" in document.createElement("div")), "value should not be on div");

  const button = document.createElement("button");
  document.body.append(button);
  assert(!own(button, "value"), "button.value should not be own before set");
  button.value = "go";
  assert(button.value === "go", "button.value getter");
  assert(button.getAttribute("value") === "go", "button value attr");
  assert(!own(button, "value"), "button.value should not be own after set");
  assert(delete button.value, "delete button.value");
  assert(!own(button, "value"), "button.value should stay inherited");
  assert(button.value === "go", "button.value after delete");
  return "ok";
})()
"#,
        )
        .expect("button value owner prototype accessor should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn form_control_value_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://form-control-values-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  accessor(HTMLInputElement.prototype, "value", true);
  accessor(HTMLInputElement.prototype, "defaultValue", true);
  accessor(HTMLTextAreaElement.prototype, "value", true);
  accessor(HTMLTextAreaElement.prototype, "defaultValue", true);
  accessor(HTMLOutputElement.prototype, "value", true);
  accessor(HTMLOutputElement.prototype, "defaultValue", true);
  accessor(HTMLOptionElement.prototype, "value", true);
  accessor(HTMLOptionElement.prototype, "text", true);
  accessor(HTMLOptionElement.prototype, "defaultSelected", true);
  accessor(HTMLOptionElement.prototype, "disabled", true);
  accessor(HTMLOptionElement.prototype, "form", false);
  accessor(HTMLOptionElement.prototype, "index", false);
  assert(!Object.getOwnPropertyDescriptor(HTMLOptionElement.prototype, "name"), "HTMLOptionElement.name should be absent");
  accessor(HTMLOptionElement.prototype, "selected", true);
  accessor(HTMLSelectElement.prototype, "length", true);
  accessor(HTMLSelectElement.prototype, "options", false);
  accessor(HTMLSelectElement.prototype, "selectedOptions", false);
  accessor(HTMLSelectElement.prototype, "selectedIndex", true);
  accessor(HTMLSelectElement.prototype, "value", true);
  accessor(HTMLSelectElement.prototype, "disabled", true);
  accessor(HTMLSelectElement.prototype, "multiple", true);
  accessor(HTMLSelectElement.prototype, "required", true);
  accessor(HTMLSelectElement.prototype, "size", true);

  const input = document.createElement("input");
  const textarea = document.createElement("textarea");
  const output = document.createElement("output");
  const form = document.createElement("form");
  const select = document.createElement("select");
  const option = document.createElement("option");
  select.append(option);
  form.append(select);
  document.body.append(input, textarea, output, form);

  for (const [element, names] of [
    [input, ["value", "defaultValue"]],
    [textarea, ["value", "defaultValue"]],
    [output, ["value", "defaultValue"]],
    [option, ["value", "text", "defaultSelected", "disabled", "form", "index", "selected"]],
    [select, ["length", "options", "selectedOptions", "selectedIndex", "value", "disabled", "multiple", "required", "size"]]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
    }
  }

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
  select.selectedIndex = 0;
  select.length = 2;

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
  assert(option.form === form, "option form");
  assert(option.index === 0, "option index");
  assert(option.selected === true, "option selected");
  assert(select.value === "choice", "select value");
  assert(select.selectedIndex === 0, "select selectedIndex");
  assert(select.length === 2, "select length");
  assert(select.disabled === true && select.hasAttribute("disabled"), "select disabled");
  assert(select.multiple === true && select.hasAttribute("multiple"), "select multiple");
  assert(select.required === true && select.hasAttribute("required"), "select required");
  assert(select.size === 4, "select size");

  for (const [element, names] of [
    [input, ["value", "defaultValue"]],
    [textarea, ["value", "defaultValue"]],
    [output, ["value", "defaultValue"]],
    [option, ["value", "text", "defaultSelected", "disabled", "form", "index", "selected"]],
    [select, ["length", "options", "selectedOptions", "selectedIndex", "value", "disabled", "multiple", "required", "size"]]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }

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
  assert(option.form === form, "option form after delete");
  assert(option.index === 0, "option index after delete");
  assert(option.selected === true, "option selected after delete");
  assert(select.value === "choice", "select value after delete");
  assert(select.selectedIndex === 0, "select selectedIndex after delete");
  assert(select.length === 2, "select length after delete");
  assert(select.disabled === true, "select disabled after delete");
  assert(select.multiple === true, "select multiple after delete");
  assert(select.required === true, "select required after delete");
  assert(select.size === 4, "select size after delete");
  return "ok";
})()
"#,
        )
        .expect("form control value owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn select_element_members_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://select-receiver-brand.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const makeOption = (id, value) => {
    const option = document.createElement("option");
    option.id = id;
    option.value = value;
    option.text = value;
    return option;
  };

  const select = document.createElement("select");
  const first = makeOption("first", "a");
  const second = makeOption("second", "b");
  select.append(first, second);
  document.body.append(select);

  const input = document.createElement("input");
  const option = document.createElement("option");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
    ["item", [0], value => value === first],
    ["namedItem", ["first"], value => value === first],
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
        .expect("select receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn option_element_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://option-receiver-brand.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const form = document.createElement("form");
  const select = document.createElement("select");
  const option = document.createElement("option");
  select.append(option);
  form.append(select);
  document.body.append(form);

  const input = document.createElement("input");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("option receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn input_element_accessors_live_on_owner_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://input-accessor-owner-prototypes.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const input = document.createElement("input");
  const datalist = document.createElement("datalist");
  datalist.id = "choices";
  document.body.append(input, datalist);
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
  assert(input.formAction === "https://input-accessor-owner-prototypes.test/submit", "formAction");
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
  assert(input.src === "https://input-accessor-owner-prototypes.test/button.png", "src");
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

  const fileInput = document.createElement("input");
  fileInput.type = "file";
  document.body.append(fileInput);
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
"#,
        )
        .expect("input owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn input_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://input-submit-overrides-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("input submitter override receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn input_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://input-reflected-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const input = document.createElement("input");
  const textarea = document.createElement("textarea");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("input reflected receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn simple_control_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://simple-control-owner-prototypes.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const form = document.createElement("form");
  const fieldset = document.createElement("fieldset");
  const legend = document.createElement("legend");
  const input = document.createElement("input");
  const datalist = document.createElement("datalist");
  const option = document.createElement("option");
  const output = document.createElement("output");
  const meter = document.createElement("meter");
  const progress = document.createElement("progress");
  fieldset.append(legend, input);
  datalist.append(option);
  form.append(fieldset, datalist, output, meter, progress);
  document.body.append(form);

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
"#,
        )
        .expect("simple control owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn simple_control_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://simple-control-receiver-brand.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const form = document.createElement("form");
  const fieldset = document.createElement("fieldset");
  const input = document.createElement("input");
  const meter = document.createElement("meter");
  const progress = document.createElement("progress");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
  fieldset.append(input);
  form.append(fieldset, meter, progress);
  document.body.append(form);

  const badReceivers = [{}, text, div, input, document.createElement("button")];
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
        .expect("simple control receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_and_textarea_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://button-textarea-owner-prototypes.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const form = document.createElement("form");
  const button = document.createElement("button");
  const target = document.createElement("div");
  const textarea = document.createElement("textarea");
  assert(!("required" in button), "button required should not be an IDL property");
  assert(!Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, "required"),
         "HTMLButtonElement.required descriptor should be absent");
  button.setAttribute("required", false);
  assert(button.getAttribute("required") === "false", "button required=false attribute text");
  button.setAttribute("required", true);
  assert(button.getAttribute("required") === "true", "button required=true attribute text");
  target.id = "target";
  form.append(button, textarea);
  document.body.append(target, form);

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
  assert(button.formAction === new URL("/submit", document.URL).href, "button formAction");
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
"#,
        )
        .expect("button/textarea owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://button-submit-overrides-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("button submitter override receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://button-reflected-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("button reflected receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn textarea_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://textarea-reflected-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const textarea = document.createElement("textarea");
  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("textarea reflected receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn object_param_and_data_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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

              const object = document.createElement("object");
              const param = document.createElement("param");
              const data = document.createElement("data");
              const div = document.createElement("div");
              document.body.append(object, param, data, div);

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

              object.data = "https://assets.example/plugin.bin";
              object.type = "application/x-test";
              object.archive = "archive.jar";
              object.code = "Applet";
              object.codeBase = "https://assets.example/classes/";
              object.codeType = "application/java";
              object.declare = true;
              object.standby = "Loading";
              assert(object.data === "https://assets.example/plugin.bin", "object data");
              assert(object.type === "application/x-test", "object type");
              assert(object.archive === "archive.jar", "object archive");
              assert(object.code === "Applet", "object code");
              assert(object.codeBase === "https://assets.example/classes/", "object codeBase");
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
              assert(object.data === "https://assets.example/plugin.bin", "object data after delete");
              assert(object.declare === true, "object declare after delete");
              assert(param.valueType === "data", "param valueType after delete");
              assert(data.value === "data-value", "data value after delete");
              return "ok";
            })()
            "#,
        )
        .expect("object/param/data owner prototype accessor probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_media_quote_mod_time_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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

              const html = document.documentElement;
              const audio = document.createElement("audio");
              const video = document.createElement("video");
              const q = document.createElement("q");
              const blockquote = document.createElement("blockquote");
              const ins = document.createElement("ins");
              const del = document.createElement("del");
              const time = document.createElement("time");
              const div = document.createElement("div");
              document.body.append(audio, video, q, blockquote, ins, del, time, div);

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
              q.cite = "refs/q.html";
              blockquote.cite = "refs/quote.html";
              ins.cite = "refs/ins.html";
              ins.dateTime = "2026-06-19";
              del.cite = "refs/del.html";
              del.dateTime = "2026-06-20";
              time.dateTime = "2026-06-21";

              assert(html.version === "4.01" && html.getAttribute("version") === "4.01", "html version");
              assert(audio.preload === "metadata" && audio.getAttribute("preload") === "metadata", "audio preload");
              assert(video.preload === "none" && video.getAttribute("preload") === "none", "video preload");
              audio.preload = "invalid";
              assert(audio.preload === "auto" && audio.getAttribute("preload") === "invalid", "audio invalid preload");
              assert(q.cite === "https://example.com/base/refs/q.html", "q cite URL");
              assert(blockquote.cite === "https://example.com/base/refs/quote.html", "blockquote cite URL");
              assert(ins.cite === "https://example.com/base/refs/ins.html", "ins cite URL");
              assert(ins.dateTime === "2026-06-19", "ins dateTime");
              assert(del.cite === "https://example.com/base/refs/del.html", "del cite URL");
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
              assert(q.cite === "https://example.com/base/refs/q.html", "q cite after delete");
              assert(ins.dateTime === "2026-06-19", "ins dateTime after delete");
              assert(time.dateTime === "2026-06-21", "time dateTime after delete");
              return "ok";
            })()
            "#,
        )
        .expect("HTML/media/quote/mod/time owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn label_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
              assert(!("label" in document.createElement("div")), "div should not expose label");
              assert(!("label" in document.createElement("select")), "select should not expose label");

              const optgroup = document.createElement("optgroup");
              const option = document.createElement("option");
              const track = document.createElement("track");
              option.textContent = "Fallback";
              document.body.append(optgroup, option, track);

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
        .expect("label owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn frame_legacy_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
              }
              accessor(HTMLImageElement.prototype, "longDesc");
              for (const name of ["scrolling", "frameBorder", "marginHeight", "marginWidth"]) {
                assert(!(name in document.createElement("img")), `${name} should not be on img`);
              }

              const frame = document.createElement("frame");
              const iframe = document.createElement("iframe");
              const img = document.createElement("img");
              document.body.append(frame, iframe, img);

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
        .expect("frame legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn resource_legacy_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };
              const absentFromPlainHtml = (name) => {
                assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
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
                absentFromPlainHtml(name);
              }

              const area = document.createElement("area");
              const image = document.createElement("img");
              const source = document.createElement("source");
              const object = document.createElement("object");
              document.body.append(area, image, source, object);

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
        .expect("resource legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn legacy_dimension_and_color_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
              }

              const table = document.createElement("table");
              const row = document.createElement("tr");
              const cell = document.createElement("td");
              const image = document.createElement("img");
              const object = document.createElement("object");
              const hr = document.createElement("hr");
              const font = document.createElement("font");
              const marquee = document.createElement("marquee");
              document.body.append(table, row, cell, image, object, hr, font, marquee);

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
        .expect("legacy dimension and color owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn body_legacy_accessors_live_on_owner_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://body-legacy-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
                assert(!own(document.body, name), `${name} should not be own before set`);
              }
              for (const name of bodyOnlyNames) {
                assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
                assert(!(name in document.createElement("div")), `${name} should not be on div`);
              }

              const handler = () => "body-load";
              document.body.onload = handler;
              assert(window.onload === handler, "body.onload setter syncs window.onload");
              assert(document.body.onload === handler, "body.onload getter reads window.onload");

              document.body.text = "black";
              document.body.link = "#111111";
              document.body.vLink = "#222222";
              document.body.aLink = "#333333";
              document.body.background = "paper.png";
              assert(document.body.text === "black" && document.body.getAttribute("text") === "black", "body text");
              assert(document.body.link === "#111111" && document.body.getAttribute("link") === "#111111", "body link");
              assert(document.body.vLink === "#222222" && document.body.getAttribute("vlink") === "#222222", "body vLink");
              assert(document.body.aLink === "#333333" && document.body.getAttribute("alink") === "#333333", "body aLink");
              assert(document.body.background === "paper.png" && document.body.getAttribute("background") === "paper.png", "body background");

              for (const name of names) {
                assert(!own(document.body, name), `${name} should not be own after set`);
                assert(delete document.body[name], `${name} delete`);
                assert(!own(document.body, name), `${name} should stay inherited`);
              }
              assert(document.body.onload === handler, "body.onload after delete");
              assert(document.body.text === "black", "body text after delete");
              window.onload = null;
              return "ok";
            })()
            "##,
        )
        .expect("body legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn global_event_handler_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://global-event-handler-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
              const submit = accessor(HTMLElement.prototype, "onsubmit");
              const load = accessor(HTMLElement.prototype, "onload");
              assert(HTMLElement.prototype.onclick === null, "HTMLElement.prototype.onclick default");
              assert(HTMLElement.prototype.onsubmit === null, "HTMLElement.prototype.onsubmit default");
              assert(HTMLElement.prototype.onload === null, "HTMLElement.prototype.onload default");
              assert(Object.getOwnPropertyDescriptor(HTMLBodyElement.prototype, "onload").get !== load.get,
                "HTMLBodyElement.onload should keep body/window override");

              const div = document.createElement("div");
              const other = document.createElement("div");
              const form = document.createElement("form");
              for (const [element, name] of [[div, "onclick"], [other, "onclick"], [form, "onsubmit"]]) {
                assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
              }

              function handler() {}
              div.onclick = handler;
              other.onclick = "not a function";
              form.onsubmit = handler;
              assert(div.onclick === handler, "div.onclick handler");
              assert(other.onclick === null, "non-function onclick becomes null");
              assert(form.onsubmit === handler, "form.onsubmit handler");
              for (const [element, name] of [[div, "onclick"], [other, "onclick"], [form, "onsubmit"]]) {
                assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
                assert(delete element[name], `${element.localName}.${name} delete`);
                assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
              }
              assert(div.onclick === handler, "div.onclick after delete");
              assert(form.onsubmit === handler, "form.onsubmit after delete");

              if (typeof SVGElement === "function") {
                accessor(SVGElement.prototype, "onclick");
                const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
                assert(!own(svg, "onclick"), "svg.onclick should not be own before set");
                svg.onclick = handler;
                assert(svg.onclick === handler, "svg.onclick handler");
                assert(!own(svg, "onclick"), "svg.onclick should not be own after set");
              }

              return "ok";
            })()
            "##,
        )
        .expect("global event handler owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn global_drag_event_handlers_cover_window_document_and_elements() {
    let mut vm = new_parsed_test_vm(
        "https://global-drag-event-handlers.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const names = [
    'ondragstart', 'ondrag', 'ondragover', 'ondragenter',
    'ondragleave', 'ondrop', 'ondragend'
  ];
  const div = document.createElement('div');
  document.body.appendChild(div);
  const describe = (owner, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return !!descriptor &&
      typeof descriptor.get === 'function' &&
      typeof descriptor.set === 'function' &&
      descriptor.enumerable && descriptor.configurable;
  };
  const surfaces = names.every(name =>
    name in window && name in document && name in div &&
    describe(window, name) &&
    describe(Document.prototype, name) &&
    describe(HTMLElement.prototype, name) &&
    window[name] === null && document[name] === null && div[name] === null
  );

  const calls = [];
  window.ondrag = event => calls.push(`window:${event.type}`);
  document.ondrag = event => calls.push(`document:${event.type}`);
  div.ondrag = event => calls.push(`element:${event.type}`);
  window.dispatchEvent(new Event('drag'));
  document.dispatchEvent(new Event('drag'));
  div.dispatchEvent(new Event('drag'));

  window.ondrag = {};
  document.ondrag = undefined;
  div.ondrag = null;
  return JSON.stringify({
    surfaces,
    calls,
    cleared: [window.ondrag, document.ondrag, div.ondrag]
  });
})()
"#,
        )
        .expect("GlobalEventHandlers drag surface should evaluate");

    assert_eq!(
        result,
        r#"{"surfaces":true,"calls":["window:drag","document:drag","element:drag"],"cleared":[null,null,null]}"#
    );
}

#[test]
fn media_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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
              assert(!("media" in document.createElement("div")), "media should not be on div");

              const link = document.createElement("link");
              const source = document.createElement("source");
              const style = document.createElement("style");
              const meta = document.createElement("meta");
              document.head.append(link, style, meta);
              document.body.append(source);

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
        .expect("media owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_media_element_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://media-receiver-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const audio = document.createElement("audio");
  const video = document.createElement("video");
  const div = document.createElement("div");
  const img = document.createElement("img");
  const source = document.createElement("source");
  const text = document.createTextNode("x");
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
        .expect("HTML media element receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn specialized_structural_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name, hasSetter = true) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const templateNames = [
                ["content", false],
                ["shadowRootMode", true],
                ["shadowRootDelegatesFocus", true],
                ["shadowRootClonable", true],
                ["shadowRootSerializable", true],
                ["shadowRootCustomElementRegistry", true],
                ["shadowRootSlotAssignment", true],
                ["shadowRootAdoptedStyleSheets", true]
              ];
              for (const [name, hasSetter] of templateNames) {
                accessor(HTMLTemplateElement.prototype, name, hasSetter);
              }
              const template = document.createElement("template");
              for (const [name] of templateNames) {
                assert(!own(template, name), `${name} should not be own on template`);
              }
              template.innerHTML = "<span>inside</span>";
              template.shadowRootMode = "open";
              template.shadowRootDelegatesFocus = true;
              template.shadowRootClonable = true;
              template.shadowRootSerializable = true;
              template.shadowRootSlotAssignment = "manual";
              template.shadowRootAdoptedStyleSheets = "[]";
              assert(template.content.firstChild.localName === "span", "template content behavior");
              assert(template.shadowRootMode === "open", "template shadowRootMode behavior");
              assert(template.shadowRootDelegatesFocus === true, "template delegates behavior");
              assert(template.shadowRootClonable === true, "template clonable behavior");
              assert(template.shadowRootSerializable === true, "template serializable behavior");
              assert(template.shadowRootSlotAssignment === "manual", "template slotAssignment behavior");
              assert(template.shadowRootAdoptedStyleSheets === "[]", "template adopted sheets behavior");

              const shadowRootNames = [
                ["host", false],
                ["mode", false],
                ["delegatesFocus", false],
                ["slotAssignment", false],
                ["clonable", false],
                ["serializable", false],
                ["referenceTarget", true],
                ["activeElement", false]
              ];
              for (const [name, hasSetter] of shadowRootNames) {
                accessor(ShadowRoot.prototype, name, hasSetter);
              }
              const shadowHost = document.createElement("section");
              document.body.append(shadowHost);
              const shadowRoot = shadowHost.attachShadow({
                mode: "open",
                delegatesFocus: true,
                slotAssignment: "manual",
                clonable: true,
                serializable: true,
                referenceTarget: "target-id"
              });
              for (const [name] of shadowRootNames) {
                assert(!own(shadowRoot, name), `${name} should not be own on shadow root`);
              }
              assert(shadowRoot.host === shadowHost, "shadow root host behavior");
              assert(shadowRoot.mode === "open", "shadow root mode behavior");
              assert(shadowRoot.delegatesFocus === true, "shadow root delegates behavior");
              assert(shadowRoot.slotAssignment === "manual", "shadow root slotAssignment behavior");
              assert(shadowRoot.clonable === true, "shadow root clonable behavior");
              assert(shadowRoot.serializable === true, "shadow root serializable behavior");
              assert(shadowRoot.referenceTarget === "target-id", "shadow root referenceTarget init");
              shadowRoot.referenceTarget = null;
              assert(shadowRoot.referenceTarget === null, "shadow root referenceTarget null setter");
              shadowRoot.referenceTarget = true;
              assert(shadowRoot.referenceTarget === "true", "shadow root referenceTarget string setter");
              assert(shadowRoot.activeElement === null, "shadow root activeElement default");

              const tableNames = [
                ["caption", true],
                ["tHead", true],
                ["tFoot", true],
                ["rows", false],
                ["tBodies", false]
              ];
              for (const [name, hasSetter] of tableNames) {
                accessor(HTMLTableElement.prototype, name, hasSetter);
              }
              const table = document.createElement("table");
              table.innerHTML = "<caption>old</caption><thead></thead><tbody><tr></tr></tbody><tfoot></tfoot>";
              for (const [name] of tableNames) {
                assert(!own(table, name), `${name} should not be own on table`);
              }
              assert(table.caption.textContent === "old", "table caption getter");
              assert(table.tHead.localName === "thead", "table tHead getter");
              assert(table.tFoot.localName === "tfoot", "table tFoot getter");
              assert(table.rows.length === 1 && table.tBodies.length === 1, "table collections");
              const caption = document.createElement("caption");
              caption.textContent = "new";
              table.caption = caption;
              assert(table.caption === caption && table.firstElementChild === caption, "table caption setter");

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
                assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
              }
              const tableProbe = document.createElement("table");
              const section = document.createElement("tbody");
              const row = document.createElement("tr");
              const cell = document.createElement("td");
              tableProbe.append(section);
              section.append(row);
              row.append(cell);
              for (const [element, names] of [
                [section, ["rows"]],
                [row, ["rowIndex", "sectionRowIndex", "cells"]],
                [cell, ["colSpan", "rowSpan", "cellIndex"]]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${name} should not be own before access`);
                }
              }
              assert(section.rows.length === 1, "section rows");
              assert(row.rowIndex === 0, "rowIndex");
              assert(row.sectionRowIndex === 0, "sectionRowIndex");
              assert(row.cells.length === 1, "row cells");
              assert(cell.cellIndex === 0, "cellIndex");
              cell.colSpan = 7;
              cell.rowSpan = 0;
              assert(cell.colSpan === 7 && cell.getAttribute("colspan") === "7", "colSpan behavior");
              assert(cell.rowSpan === 0 && cell.getAttribute("rowspan") === "0", "rowSpan behavior");
              for (const [element, names] of [
                [section, ["rows"]],
                [row, ["rowIndex", "sectionRowIndex", "cells"]],
                [cell, ["colSpan", "rowSpan", "cellIndex"]]
              ]) {
                for (const name of names) {
                  assert(!own(element, name), `${name} should not be own after access`);
                  assert(delete element[name], `${name} delete`);
                  assert(!own(element, name), `${name} should stay inherited`);
                }
              }

              const simpleCases = [
                [HTMLLIElement.prototype, "value", document.createElement("li"), 7, "7", "value"],
                [HTMLOListElement.prototype, "start", document.createElement("ol"), 3, "3", "start"],
                [HTMLOListElement.prototype, "reversed", document.createElement("ol"), true, "", "reversed"],
                [HTMLOListElement.prototype, "type", document.createElement("ol"), "A", "A", "type"],
                [HTMLOptGroupElement.prototype, "disabled", document.createElement("optgroup"), true, "", "disabled"],
                [HTMLDetailsElement.prototype, "open", document.createElement("details"), true, "", "open"],
                [HTMLMetaElement.prototype, "content", document.createElement("meta"), "width=device-width", "width=device-width", "content"],
                [HTMLMetaElement.prototype, "httpEquiv", document.createElement("meta"), "refresh", "refresh", "http-equiv"],
                [HTMLTitleElement.prototype, "text", document.createElement("title"), "Page Title", "Page Title", null]
              ];
              for (const [prototype, name, element, value, expected, attribute] of simpleCases) {
                accessor(prototype, name);
                assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
                assert(!own(element, name), `${name} should not be own before set`);
                element[name] = value;
                assert(element[name] === value || element[name] === expected, `${name} getter`);
                if (attribute === null) {
                  assert(element.textContent === expected, `${name} text content`);
                } else {
                  assert(element.getAttribute(attribute) === expected, `${name} attribute`);
                }
                assert(!own(element, name), `${name} should not be own after set`);
                assert(delete element[name], `${name} delete`);
                assert(!own(element, name), `${name} should stay inherited`);
                assert(element[name] === value || element[name] === expected, `${name} after delete`);
              }
              return "ok";
            })()
            "#,
        )
        .expect("specialized structural accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn title_text_uses_only_direct_text_node_children() {
    let mut vm = new_parsed_test_vm(
        "https://title-text-children.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const title = document.createElement("title");
  title.append(
    document.createComment("COMMENT"),
    document.createTextNode("DIRECT"),
    Object.assign(document.createElement("span"), { textContent: "NESTED" })
  );
  if (title.text !== "DIRECT") throw new Error(`title.text: ${title.text}`);
  if (title.textContent !== "DIRECTNESTED") throw new Error(`title.textContent: ${title.textContent}`);

  title.text = "replacement";
  if (title.childNodes.length !== 1) throw new Error("title.text setter child count");
  if (title.firstChild.nodeType !== Node.TEXT_NODE) throw new Error("title.text setter child type");
  if (title.text !== "replacement") throw new Error("title.text setter value");
  return "ok";
})()
"#,
        )
        .expect("title.text should use child text content rather than descendant text content");

    assert_eq!(result, "ok");
}

#[test]
fn fetch_priority_reflection_is_shared_by_supported_elements() {
    let mut vm = new_parsed_test_vm(
        "https://fetch-priority-reflection.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const cases = [
    [HTMLImageElement.prototype, document.createElement("img"), "img"],
    [HTMLLinkElement.prototype, document.createElement("link"), "link"],
    [HTMLScriptElement.prototype, document.createElement("script"), "script"]
  ];

  for (const [prototype, element, label] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "fetchPriority");
    assert(!!descriptor, `${label} descriptor`);
    assert(typeof descriptor.get === "function", `${label} getter`);
    assert(typeof descriptor.set === "function", `${label} setter`);
    assert(descriptor.get.call(element) === "auto", `${label} missing default`);

    element.setAttribute("fetchpriority", "HIGH");
    assert(descriptor.get.call(element) === "high", `${label} high canonicalization`);
    descriptor.set.call(element, "LOW");
    assert(element.getAttribute("fetchpriority") === "LOW", `${label} setter reflection`);
    assert(descriptor.get.call(element) === "low", `${label} low canonicalization`);
    descriptor.set.call(element, "invalid");
    assert(element.getAttribute("fetchpriority") === "invalid", `${label} invalid reflection`);
    assert(descriptor.get.call(element) === "auto", `${label} invalid default`);
    assert(throwsTypeError(() => descriptor.set.call(element, Symbol())), `${label} symbol setter`);

    const invalidReceivers = [
      {},
      document.createTextNode("x"),
      document.createElement("div"),
      ...cases.filter(([, candidate]) => candidate !== element).map(([, candidate]) => candidate)
    ];
    for (const receiver of invalidReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${label} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, "high")), `${label} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("fetchPriority should reflect the shared limited-known-value enumeration");

    assert_eq!(result, "ok");
}

#[test]
fn simple_specialized_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://simple-specialized-receiver-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const li = document.createElement("li");
  const ol = document.createElement("ol");
  const optgroup = document.createElement("optgroup");
  const details = document.createElement("details");
  const meta = document.createElement("meta");
  const title = document.createElement("title");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("simple specialized receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_table_structural_members_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://table-receiver-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const table = document.createElement("table");
  const caption = document.createElement("caption");
  const thead = document.createElement("thead");
  const tfoot = document.createElement("tfoot");
  const tbody = document.createElement("tbody");
  const row = document.createElement("tr");
  const td = document.createElement("td");
  const th = document.createElement("th");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
  const methodTable = document.createElement("table");
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
    const section = document.createElement("tbody");
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
    const methodRow = document.createElement("tr");
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
        .expect("HTML table structural receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn table_head_placement_uses_element_children_as_the_reference() {
    let mut vm = new_parsed_test_vm(
        "https://table-head-placement.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const buildTable = () => {
    const table = document.createElement("table");
    table.append(
      document.createTextNode("leading"),
      document.createElement("caption"),
      document.createComment("between"),
      document.createElement("colgroup"),
      document.createTextNode("before body"),
      document.createElement("tbody")
    );
    return table;
  };

  const createdTable = buildTable();
  const createdHead = createdTable.createTHead();

  const assignedTable = buildTable();
  const assignedHead = document.createElement("thead");
  assignedTable.tHead = assignedHead;

  return [
    [...createdTable.children].map(element => element.localName).join(","),
    createdHead.previousElementSibling.localName,
    createdHead.nextElementSibling.localName,
    [...assignedTable.children].map(element => element.localName).join(","),
    assignedHead.previousElementSibling.localName,
    assignedHead.nextElementSibling.localName
  ].join("|");
})()
"#,
        )
        .expect("table head placement probe should evaluate");

    assert_eq!(
        result,
        "caption,colgroup,thead,tbody|colgroup|tbody|caption,colgroup,thead,tbody|colgroup|tbody"
    );
}

#[test]
fn document_storage_access_api_minimal_surface_matches_idlharness() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        globalThis.__storageAccessApiProbe = "pending";
        (async () => {
          const has = Document.prototype.hasStorageAccess;
          const request = Document.prototype.requestStorageAccess;
          const hasDesc = Object.getOwnPropertyDescriptor(Document.prototype, "hasStorageAccess");
          const requestDesc = Object.getOwnPropertyDescriptor(Document.prototype, "requestStorageAccess");
          const promiseOutcome = async (fn, receiver) => {
            let value;
            try {
              value = fn.call(receiver);
            } catch (error) {
              return `throw:${error && error.name}`;
            }
            const isPromise = value instanceof Promise;
            try {
              await value;
              return `resolved:${isPromise}`;
            } catch (error) {
              return `rejected:${isPromise}:${error && error.name}`;
            }
          };
          const hasAccess = await document.hasStorageAccess();
          const requestResult = await document.requestStorageAccess();
          return {
            hasType: typeof has,
            hasName: has.name,
            hasLength: has.length,
            hasWritable: !!hasDesc.writable,
            hasEnumerable: !!hasDesc.enumerable,
            hasConfigurable: !!hasDesc.configurable,
            requestType: typeof request,
            requestName: request.name,
            requestLength: request.length,
            requestWritable: !!requestDesc.writable,
            requestEnumerable: !!requestDesc.enumerable,
            requestConfigurable: !!requestDesc.configurable,
            hasAccess,
            requestUndefined: requestResult === undefined,
            nullReceiver: await promiseOutcome(has, null),
            objectReceiver: await promiseOutcome(request, {})
          };
        })().then(
          value => { globalThis.__storageAccessApiProbe = JSON.stringify(value); },
          error => { globalThis.__storageAccessApiProbe = `error:${error && error.message}`; }
        );
        "#,
        None,
    )
    .expect("storage access api probe should schedule");

    let result = vm
        .eval("String(globalThis.__storageAccessApiProbe)")
        .expect("storage access api probe should evaluate");

    assert_eq!(
        result,
        r#"{"hasType":"function","hasName":"hasStorageAccess","hasLength":0,"hasWritable":true,"hasEnumerable":true,"hasConfigurable":true,"requestType":"function","requestName":"requestStorageAccess","requestLength":0,"requestWritable":true,"requestEnumerable":true,"requestConfigurable":true,"hasAccess":true,"requestUndefined":true,"nullReceiver":"rejected:true:TypeError","objectReceiver":"rejected:true:TypeError"}"#
    );
}

#[test]
fn document_has_storage_access_is_false_for_insecure_context() {
    let mut vm = new_storage_test_vm("http://example.com/");

    vm.exec(
        r#"
        globalThis.__insecureStorageAccessProbe = "pending";
        document.hasStorageAccess().then(
          value => { globalThis.__insecureStorageAccessProbe = String(value); },
          error => { globalThis.__insecureStorageAccessProbe = `error:${error && error.name}`; }
        );
        "#,
        None,
    )
    .expect("insecure storage access probe should schedule");

    let result = vm
        .eval("String(globalThis.__insecureStorageAccessProbe)")
        .expect("insecure storage access probe should settle");

    assert_eq!(result, "false");
}

#[test]
fn document_request_storage_access_rejects_insecure_context() {
    let mut vm = new_storage_test_vm("http://example.com/");

    vm.exec(
        r#"
        globalThis.__insecureRequestStorageAccessProbe = "pending";
        document.requestStorageAccess().then(
          () => { globalThis.__insecureRequestStorageAccessProbe = "resolved"; },
          error => {
            globalThis.__insecureRequestStorageAccessProbe =
              `${error && error.name}:${error instanceof DOMException}`;
          }
        );
        "#,
        None,
    )
    .expect("insecure requestStorageAccess probe should schedule");

    let result = vm
        .eval("String(globalThis.__insecureRequestStorageAccessProbe)")
        .expect("insecure requestStorageAccess probe should settle");

    assert_eq!(result, "NotAllowedError:true");
}

#[test]
fn document_storage_access_rejects_non_fully_active_documents() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        globalThis.__inactiveStorageAccessProbe = "pending";
        (async () => {
          const rejectionName = async (promise) => {
            try {
              await promise;
              return "resolved";
            } catch (error) {
              return `${error && error.name}:${error instanceof DOMException}`;
            }
          };
          const xml = document.implementation.createDocument("", null);
          const html = document.implementation.createHTMLDocument("");
          return [
            await rejectionName(xml.hasStorageAccess()),
            await rejectionName(xml.requestStorageAccess()),
            await rejectionName(html.hasStorageAccess()),
            await rejectionName(html.requestStorageAccess())
          ].join("|");
        })().then(
          value => { globalThis.__inactiveStorageAccessProbe = value; },
          error => { globalThis.__inactiveStorageAccessProbe = `error:${error && error.name}`; }
        );
        "#,
        None,
    )
    .expect("inactive storage access probe should schedule");

    let result = vm
        .eval("String(globalThis.__inactiveStorageAccessProbe)")
        .expect("inactive storage access probe should settle");

    assert_eq!(
        result,
        "InvalidStateError:true|InvalidStateError:true|InvalidStateError:true|InvalidStateError:true"
    );
}

#[test]
fn live_document_cookie_descriptor_matches_chromium_prototype_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Object.getPrototypeOf(document);
              const proto2 = proto && Object.getPrototypeOf(proto);
              const summarize = (obj, key) => {
                const d = obj && Object.getOwnPropertyDescriptor(obj, key);
                if (!d) return null;
                return {
                  enumerable: !!d.enumerable,
                  configurable: !!d.configurable,
                  writable: Object.prototype.hasOwnProperty.call(d, "writable") ? !!d.writable : null,
                  hasGetter: typeof d.get === "function",
                  hasSetter: typeof d.set === "function",
                  valueType: Object.prototype.hasOwnProperty.call(d, "value") ? typeof d.value : null
                };
              };
              return JSON.stringify({
                ctor: document.constructor && document.constructor.name,
                ownCookie: Object.prototype.hasOwnProperty.call(document, "cookie"),
                documentCookie: summarize(document, "cookie"),
                protoName: proto && proto.constructor && proto.constructor.name,
                protoCookie: summarize(proto, "cookie"),
                proto2Name: proto2 && proto2.constructor && proto2.constructor.name,
                proto2Cookie: summarize(proto2, "cookie"),
                documentProtoCookie: typeof Document !== "undefined" ? summarize(Document.prototype, "cookie") : null,
                htmlDocumentProtoCookie: typeof HTMLDocument !== "undefined" ? summarize(HTMLDocument.prototype, "cookie") : null
              });
            })()
            "#,
        )
        .expect("document cookie descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"ctor":"HTMLDocument","ownCookie":false,"documentCookie":null,"protoName":"HTMLDocument","protoCookie":null,"proto2Name":"Document","proto2Cookie":{"enumerable":true,"configurable":true,"writable":null,"hasGetter":true,"hasSetter":true,"valueType":null},"documentProtoCookie":{"enumerable":true,"configurable":true,"writable":null,"hasGetter":true,"hasSetter":true,"valueType":null},"htmlDocumentProtoCookie":null}"#
    );
}
#[test]
fn live_document_cookie_getter_returns_string_when_cookie_store_is_empty() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => JSON.stringify({
              type: typeof document.cookie,
              value: document.cookie,
              missingMatch: document.cookie.match(/missing_cookie/)
            }))()
            "#,
        )
        .expect("document cookie empty getter probe should evaluate");

    assert_eq!(
        result,
        r#"{"type":"string","value":"","missingMatch":null}"#
    );
}
#[test]
fn live_document_cookie_write_uses_prototype_accessor_without_creating_own_property() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              document.cookie = "probe_cookie=ok; Path=/";
              return JSON.stringify({
                cookieIncludesProbe: document.cookie.includes("probe_cookie=ok"),
                ownCookie: Object.prototype.hasOwnProperty.call(document, "cookie"),
                ownCookieDesc: Object.getOwnPropertyDescriptor(document, "cookie") ?? null,
                protoCookieGetterType: typeof Object.getOwnPropertyDescriptor(Document.prototype, "cookie")?.get,
                protoCookieSetterType: typeof Object.getOwnPropertyDescriptor(Document.prototype, "cookie")?.set
              });
            })()
            "#,
        )
        .expect("document cookie write probe should evaluate");

    assert_eq!(
        result,
        r#"{"cookieIncludesProbe":true,"ownCookie":false,"ownCookieDesc":null,"protoCookieGetterType":"function","protoCookieSetterType":"function"}"#
    );
}
#[test]
fn live_document_all_matches_chromium_htmldda_surface() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div id=\"probe\"></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => JSON.stringify({
              type: typeof document.all,
              loose: document.all == undefined,
              strict: document.all === undefined,
              bool: !!document.all,
              tag: Object.prototype.toString.call(document.all),
              string: String(document.all),
              ctorDirect: document.all.constructor && document.all.constructor.name,
              noArgType: typeof document.all(),
              noArgNull: document.all() === null,
              item0: document.all(0)?.tagName ?? null,
              item999Null: document.all(999) === null,
              namedHit: document.all("probe") === document.getElementById("probe"),
              itemMethodNull: document.all.item(999) === null,
              namedMethodNull: document.all.namedItem('missing') === null
            }))()
            "#,
        )
        .expect("live document.all probe should evaluate");

    assert_eq!(
        result,
        r#"{"type":"undefined","loose":true,"strict":false,"bool":false,"tag":"[object HTMLAllCollection]","string":"[object HTMLAllCollection]","ctorDirect":"HTMLAllCollection","noArgType":"object","noArgNull":true,"item0":"HTML","item999Null":true,"namedHit":true,"itemMethodNull":true,"namedMethodNull":true}"#
    );
}

#[test]
fn live_html_collection_enforces_brand_and_legacy_named_property_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><p id=\"named\"></p></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const collection = document.getElementsByTagName("p");
              const element = document.getElementById("named");
              const derived = Object.create(collection);
              const detachedCollection = new DOMParser()
                .parseFromString("<html><body><span></span></body></html>", "text/html")
                .getElementsByTagName("span");
              const lengthGetter = Object.getOwnPropertyDescriptor(
                HTMLCollection.prototype,
                "length"
              ).get;
              let derivedLengthError = null;
              try {
                lengthGetter.call(derived);
              } catch (error) {
                derivedLengthError = error.name;
              }
              derived.named = "derived expando";
              collection.named = "ignored";
              let strictSetError = null;
              try {
                (() => {
                  "use strict";
                  collection.named = "ignored";
                })();
              } catch (error) {
                strictSetError = error.name;
              }
              collection.unsupported = "collection expando";
              const namedDescriptor = Object.getOwnPropertyDescriptor(collection, "named");
              return JSON.stringify({
                inheritedNamed: Object.getPrototypeOf(derived).named === element,
                derivedOwnNamed: Object.hasOwn(derived, "named"),
                derivedNamed: derived.named,
                collectionNamedPreserved: collection.named === element,
                strictSetError,
                unsupportedExpando: collection.unsupported,
                namedDescriptor: {
                  writable: namedDescriptor.writable,
                  enumerable: namedDescriptor.enumerable,
                  configurable: namedDescriptor.configurable
                },
                collectionLength: lengthGetter.call(collection),
                detachedCollectionLength: lengthGetter.call(detachedCollection),
                derivedLengthError
              });
            })()
            "#,
        )
        .expect("HTMLCollection legacy platform object semantics should evaluate");

    assert_eq!(
        result,
        r#"{"inheritedNamed":true,"derivedOwnNamed":true,"derivedNamed":"derived expando","collectionNamedPreserved":true,"strictSetError":"TypeError","unsupportedExpando":"collection expando","namedDescriptor":{"writable":false,"enumerable":false,"configurable":true},"collectionLength":1,"detachedCollectionLength":1,"derivedLengthError":"TypeError"}"#
    );
}

#[test]
fn html_collection_prototype_members_follow_webidl_enumeration() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><body><form id=\"first\"></form><form id=\"second\"></form></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const forms = document.forms;
              const descriptorEnumerability = ["length", "item", "namedItem"].map(
                name => Object.getOwnPropertyDescriptor(
                  HTMLCollection.prototype,
                  name
                ).enumerable
              );
              const iteratorEnumerable = Object.getOwnPropertyDescriptor(
                HTMLCollection.prototype,
                Symbol.iterator
              ).enumerable;
              const enumerated = [];
              for (const name in forms) {
                enumerated.push(name);
              }
              return JSON.stringify({
                descriptorEnumerability,
                iteratorEnumerable,
                indices: enumerated.splice(0, 2),
                prototypeMembers: enumerated.sort()
              });
            })()
            "#,
        )
        .expect("HTMLCollection enumeration checks should evaluate");

    assert_eq!(
        result,
        r#"{"descriptorEnumerability":[true,true,true],"iteratorEnumerable":false,"indices":["0","1"],"prototypeMembers":["item","length","namedItem"]}"#
    );
}

#[test]
fn live_html_collection_iterator_observes_mutations_between_steps() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><body><div id=\"host\"><span id=\"initial\"></span></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.getElementById("host");
              const collection = host.getElementsByTagName("span");
              const iterator = collection[Symbol.iterator]();

              const replacement = document.createElement("span");
              replacement.id = "replacement";
              document.getElementById("initial").replaceWith(replacement);
              const first = iterator.next();

              const appended = document.createElement("span");
              appended.id = "appended";
              host.appendChild(appended);
              const second = iterator.next();

              appended.remove();
              const third = iterator.next();

              return JSON.stringify({
                first: [first.done, first.value && first.value.id],
                second: [second.done, second.value && second.value.id],
                thirdDone: third.done,
                finalIds: Array.from(collection, element => element.id)
              });
            })()
            "#,
        )
        .expect("live HTMLCollection iterator mutation checks should evaluate");

    assert_eq!(
        result,
        r#"{"first":[false,"replacement"],"second":[false,"appended"],"thirdDone":true,"finalIds":["replacement"]}"#
    );
}

#[test]
fn live_document_all_declared_members_ignore_public_data_spoofing() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div id=\"probe\"></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const all = document.all;
              const probe = document.getElementById("probe");
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
                all.item(0) && all.item(0).tagName,
                all.namedItem("probe") === probe,
                typeof all[Symbol.iterator]
              ].join("|");
            })()
            "#,
        )
        .expect("document.all declared surface spoofing probe should evaluate");

    assert_eq!(
        result,
        "true:false:true:false:number|true:false:true:true:function|true:false:true:true:function|true:false:true:true:function|false|false|true|true|HTML|true|function"
    );
}

#[test]
fn document_write_parses_variadic_webidl_strings() {
    let mut vm = new_storage_test_vm("https://document-write-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error && error.name;
                }
              };
              document.write(
                "<main id='",
                { toString() { return "written"; } },
                "'>value:",
                null,
                "</main>"
              );
              document.close();
              return [
                document.getElementById("written")?.textContent,
                probe(() => document.write(Symbol("chunk"))),
                document.getElementById("written")?.textContent
              ].join("|");
            })()
            "#,
        )
        .expect("Document.write WebIDL variadic argument probe should evaluate");

    assert_eq!(result, "value:null|TypeError|value:null");
}

#[test]
fn document_open_preserves_document_identity_and_detaches_the_replaced_tree() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-identity.test/",
        "<!doctype html><html><body><main id=\"old\">old text</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const oldDocument = document;
              const oldNode = document.getElementById("old");
              const oldBody = document.body;
              const oldText = oldNode.firstChild;
              const oldClassList = oldNode.classList;
              const oldDataset = oldNode.dataset;
              const oldStyle = oldNode.style;
              const oldDocumentMains = document.getElementsByTagName("main");
              const oldBodyChildren = oldBody.children;
              const oldShadowHost = document.createElement("section");
              oldBody.append(oldShadowHost);
              const oldShadow = oldShadowHost.attachShadow({ mode: "open" });
              oldShadow.innerHTML = "<span>shadow text</span>";
              const oldShadowChild = oldShadow.firstChild;
              const preDetached = document.createElement("aside");
              const listenerRuns = {
                node: 0,
                document: 0,
                window: 0,
                handler: 0,
                preDetachedNode: 0,
                preDetachedHandler: 0
              };
              oldNode.addEventListener("click", () => listenerRuns.node++);
              oldNode.onclick = () => listenerRuns.handler++;
              preDetached.addEventListener("click", () => listenerRuns.preDetachedNode++);
              preDetached.onclick = () => listenerRuns.preDetachedHandler++;
              document.addEventListener("replacement-probe", () => listenerRuns.document++);
              window.addEventListener("replacement-probe", () => listenerRuns.window++);

              document.open();
              document.write("<!doctype html><html><body><main id='new'>new text</main></body></html>");
              document.close();

              oldNode.dispatchEvent(new Event("click"));
              preDetached.dispatchEvent(new Event("click"));
              document.dispatchEvent(new Event("replacement-probe", { bubbles: true }));
              window.dispatchEvent(new Event("replacement-probe"));

              return JSON.stringify({
                sameDocument: document === oldDocument,
                oldNodeConnected: oldNode.isConnected,
                oldNodeText: oldNode.textContent,
                oldNodeOwnerPreserved: oldNode.ownerDocument === document,
                oldNodeParentPreserved: oldNode.parentNode === oldBody,
                oldTextIdentityPreserved: oldNode.firstChild === oldText,
                oldClassListIdentityPreserved: oldNode.classList === oldClassList,
                oldDatasetIdentityPreserved: oldNode.dataset === oldDataset,
                oldStyleIdentityPreserved: oldNode.style === oldStyle,
                documentCollectionIdentityPreserved:
                  document.getElementsByTagName("main") === oldDocumentMains,
                documentCollectionTracksReplacement:
                  Array.from(oldDocumentMains, node => node.id).join(","),
                detachedCollectionIdentityPreserved:
                  oldBody.children === oldBodyChildren,
                detachedCollectionKeepsOldTree:
                  Array.from(oldBodyChildren, node => node.id || node.localName).join(","),
                oldNodeStillMatches: oldNode.matches('#old'),
                oldBodyConnected: oldBody.isConnected,
                oldShadowIdentityPreserved: oldShadowHost.shadowRoot === oldShadow,
                oldShadowChildIdentityPreserved: oldShadow.firstChild === oldShadowChild,
                oldShadowText: oldShadowChild.textContent,
                oldShadowConnected: oldShadow.isConnected,
                listenerRuns,
                oldLookupMissing: document.getElementById("old") === null,
                newText: document.getElementById("new")?.textContent
              });
            })()
            "#,
        )
        .expect("document.open identity and detached-tree probe should evaluate");

    assert_eq!(
        result,
        r#"{"sameDocument":true,"oldNodeConnected":false,"oldNodeText":"old text","oldNodeOwnerPreserved":true,"oldNodeParentPreserved":true,"oldTextIdentityPreserved":true,"oldClassListIdentityPreserved":true,"oldDatasetIdentityPreserved":true,"oldStyleIdentityPreserved":true,"documentCollectionIdentityPreserved":true,"documentCollectionTracksReplacement":"new","detachedCollectionIdentityPreserved":true,"detachedCollectionKeepsOldTree":"old,section","oldNodeStillMatches":true,"oldBodyConnected":false,"oldShadowIdentityPreserved":true,"oldShadowChildIdentityPreserved":true,"oldShadowText":"shadow text","oldShadowConnected":false,"listenerRuns":{"node":0,"document":0,"window":0,"handler":0,"preDetachedNode":1,"preDetachedHandler":1},"oldLookupMissing":true,"newText":"new text"}"#
    );
}

// Ported from WPT opening-the-input-stream/active.window.js and
// mutation-observer.window.js. Document.open() performs the all-children
// removal synchronously; the replacement parser must not pre-create an HTML
// shell before it consumes input.
#[test]
fn document_open_immediately_empties_document_and_reports_one_removal_batch() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-active.test/",
        "<html><body><main id=old>old</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const oldDocument = document;
              const oldHtml = document.documentElement;
              const observer = new MutationObserver(() => {});
              observer.observe(document, { childList: true, subtree: true });

              const returned = document.open();
              const firstRecords = observer.takeRecords().map(record => ({
                target: record.target.nodeName,
                added: Array.from(record.addedNodes, node => node.nodeName),
                removed: Array.from(record.removedNodes, node => node.nodeName),
                removedOldHtml: record.removedNodes[0] === oldHtml,
              }));
              const firstState = {
                returnedSameDocument: returned === oldDocument,
                childCount: document.childNodes.length,
                documentElementIsNull: document.documentElement === null,
                bodyIsNull: document.body === null,
                readyState: document.readyState,
                firstRecords,
              };

              document.open();
              const secondRecords = observer.takeRecords().length;
              const secondChildCount = document.childNodes.length;
              document.close();

              return JSON.stringify({
                ...firstState,
                secondRecords,
                secondChildCount,
              });
            })()
            "#,
        )
        .expect("document.open active-state probe should evaluate");

    assert_eq!(
        result,
        r##"{"returnedSameDocument":true,"childCount":0,"documentElementIsNull":true,"bodyIsNull":true,"readyState":"loading","firstRecords":[{"target":"#document","added":[],"removed":["HTML"],"removedOldHtml":true}],"secondRecords":0,"secondChildCount":0}"##
    );
}

// Ported from WPT opening-the-input-stream/active.window.js. A script-created
// document parser mutates the live Document as each write is consumed; waiting
// until close() and swapping in a completed foreign tree is observably wrong.
#[test]
fn document_write_incrementally_updates_the_live_document_before_close() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-incremental.test/",
        "<html><body><main>old</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              const afterOpen = document.childNodes.length;
              document.write('<!doctype html>');
              const afterDoctypeWrite = {
                childCount: document.childNodes.length,
                firstType: document.firstChild?.nodeType,
                firstName: document.firstChild?.nodeName,
              };
              document.close();
              const afterClose = {
                childCount: document.childNodes.length,
                hasHtml: document.documentElement?.nodeName,
              };

              document.write();
              const afterImplicitOpen = document.childNodes.length;
              document.write('<!doctype html>');
              const afterSecondDoctypeWrite = document.childNodes.length;
              document.close();
              return JSON.stringify({
                afterOpen,
                afterDoctypeWrite,
                afterClose,
                afterImplicitOpen,
                afterSecondDoctypeWrite,
                finalChildCount: document.childNodes.length,
              });
            })()
            "#,
        )
        .expect("incremental document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"afterOpen":0,"afterDoctypeWrite":{"childCount":1,"firstType":10,"firstName":"html"},"afterClose":{"childCount":2,"hasHtml":"HTML"},"afterImplicitOpen":0,"afterSecondDoctypeWrite":1,"finalChildCount":2}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/003.html,
// 004.html, 009.html, and 015.html. Each write continues the same tokenizer;
// it must not parse the calls as independent fragments.
#[test]
fn document_write_keeps_tokenizer_state_across_split_tag_and_attribute_input() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-split-tag.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<');
              document.write('i id');
              document.write("='test'");
              document.write(" class='a'>Filler");
              document.write(' Text</');
              document.write('i>');
              const element = document.body.firstChild;
              const snapshot = {
                name: element.localName,
                id: element.id,
                className: element.className,
                text: element.textContent,
                childCount: document.body.childNodes.length,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("split tag and attribute document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"name":"i","id":"test","className":"a","text":"Filler Text","childCount":1}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/018.html,
// 042.html, and 044-046.html. These tokenizer states intentionally span
// separate document.write calls.
#[test]
fn document_write_keeps_comment_character_reference_and_rcdata_state_across_calls() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-split-tokenizer-states.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<body><!');
              document.write('--com');
              document.write('ment-->');
              document.write('<span>&not');
              document.write('in;abc</span>');
              document.write('<textarea><span>');
              document.write('Filler</span></text');
              document.write('area>');
              const nodes = document.body.childNodes;
              const snapshot = {
                commentType: nodes[0].nodeType,
                comment: nodes[0].data,
                entity: nodes[1].textContent,
                textareaText: nodes[2].textContent,
                textareaChildren: nodes[2].childNodes.length,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("split tokenizer state document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"commentType":8,"comment":"comment","entity":"∉abc","textareaText":"<span>Filler</span>","textareaChildren":1}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/034-036.html.
#[test]
fn document_write_keeps_foreign_content_cdata_state_across_calls() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-split-cdata.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<body><svg><!');
              document.write('[CDATA[Filler');
              document.write(' Text]]></svg>');
              const svg = document.body.firstChild;
              const snapshot = {
                name: svg.localName,
                namespace: svg.namespaceURI,
                text: svg.textContent,
                childType: svg.firstChild.nodeType,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("split foreign-content CDATA document.write probe should evaluate");

    assert_eq!(
        result,
        r##"{"name":"svg","namespace":"http://www.w3.org/2000/svg","text":"Filler Text","childType":3}"##
    );
}

// Ported from WPT dynamic-markup-insertion/document-write/051.html. CRLF
// preprocessing must retain the pending CR across parser input chunks.
#[test]
fn document_write_normalizes_newlines_across_call_boundaries() {
    let mut vm = new_parsed_test_vm(
        "https://document-write-newline-boundary.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write('<body>');
              document.write('\r');
              document.write('\nA');
              document.write('\rB');
              document.close();
              return JSON.stringify(document.body.textContent);
            })()
            "#,
        )
        .expect("cross-call newline normalization probe should evaluate");

    assert_eq!(result, r##""\nA\nB""##);
}

// Ported from WPT opening-the-input-stream/quirks.window.js. open() itself
// resets the mode synchronously; the tokenizer changes it only after a full
// doctype token has been consumed, even when the token spans writes.
#[test]
fn document_open_resets_compat_mode_and_parser_updates_it_incrementally() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-quirks.test/",
        "<html><body>quirks</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const modes = [document.compatMode];
              document.open();
              modes.push(document.compatMode);
              document.write('<!doctype html public');
              modes.push(document.compatMode);
              document.write(' "-//IETF//DTD HTML 3//"');
              modes.push(document.compatMode);
              document.write('>');
              modes.push(document.compatMode);
              document.close();
              modes.push(document.compatMode);

              document.open();
              modes.push(document.compatMode);
              document.write('<!doctype html');
              modes.push(document.compatMode);
              document.write('>');
              modes.push(document.compatMode);
              document.close();
              modes.push(document.compatMode);
              return modes.join('|');
            })()
            "#,
        )
        .expect("document.open quirks-mode probe should evaluate");

    assert_eq!(
        result,
        "BackCompat|CSS1Compat|CSS1Compat|CSS1Compat|BackCompat|BackCompat|CSS1Compat|CSS1Compat|CSS1Compat|CSS1Compat"
    );
}

// Ported from WPT opening-the-input-stream/custom-element.window.js. The
// dynamic-markup counter is a parser construction guard, not a blanket custom
// element construction guard.
#[test]
fn document_open_is_allowed_in_create_element_custom_element_constructor() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-create-element-constructor.test/",
        "<!doctype html><html><body><main>old</main></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              let returnedDocument = null;
              let errorName = null;
              class OpenFromConstructor extends HTMLElement {
                constructor() {
                  super();
                  try {
                    returnedDocument = document.open();
                  } catch (error) {
                    errorName = error.name;
                  }
                }
              }
              customElements.define('x-open-from-constructor', OpenFromConstructor);
              const element = document.createElement('x-open-from-constructor');
              const snapshot = {
                errorName,
                returnedSameDocument: returnedDocument === document,
                constructed: element instanceof OpenFromConstructor,
                childCountAfterOpen: document.childNodes.length,
              };
              document.close();
              return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("createElement custom element document.open probe should evaluate");

    assert_eq!(
        result,
        r##"{"errorName":null,"returnedSameDocument":true,"constructed":true,"childCountAfterOpen":0}"##
    );
}

// Ported from WPT opening-the-input-stream/tasks.window.js. Document-owned
// parser/script continuations are invalidated by open(), but an ordinary task
// already queued on the associated Window remains queued.
#[tokio::test]
async fn document_open_keeps_already_queued_window_timer_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://document-open-keeps-window-task.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              globalThis.__documentOpenTaskLog = [];
              setTimeout(() => {
                __documentOpenTaskLog.push(document.getElementById('replacement')?.textContent);
              }, 0);
              document.open();
              document.write('<main id=replacement>new document</main>');
              document.close();
              return __documentOpenTaskLog.length;
            })()
            "#,
        )
        .expect("document.open queued timer setup should evaluate"),
        "0"
    );

    for _ in 0..4 {
        let _ = vm
            .run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("the exact timer body should advance the pre-open Window timer");
        if vm
            .eval("__documentOpenTaskLog.length")
            .expect("document.open task count should evaluate")
            == "1"
        {
            break;
        }
    }

    assert_eq!(
        vm.eval("__documentOpenTaskLog.join('|')")
            .expect("document.open task result should evaluate"),
        "new document"
    );
}

#[test]
fn document_open_coalesces_doctype_and_element_removal_into_one_record() {
    let mut vm = new_parsed_test_vm(
        "https://document-open-remove-all.test/",
        "<!doctype html><html><body>old</body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const oldChildren = Array.from(document.childNodes);
              const observer = new MutationObserver(() => {});
              observer.observe(document, { childList: true });
              document.open();
              const records = observer.takeRecords();
              const result = {
                oldChildCount: oldChildren.length,
                recordCount: records.length,
                removedCount: records[0]?.removedNodes.length,
                exactIdentityAndOrder: Array.from(
                  records[0]?.removedNodes || [],
                  (node, index) => node === oldChildren[index]
                ).every(Boolean),
                addedCount: records[0]?.addedNodes.length,
                childCount: document.childNodes.length,
              };
              document.close();
              return JSON.stringify(result);
            })()
            "#,
        )
        .expect("document.open all-children mutation probe should evaluate");

    assert_eq!(
        result,
        r#"{"oldChildCount":2,"recordCount":1,"removedCount":2,"exactIdentityAndOrder":true,"addedCount":0,"childCount":0}"#
    );
}

#[test]
fn document_writeln_uses_document_prototype_and_appends_newline() {
    let mut vm = new_storage_test_vm("https://document-writeln-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error && error.name;
                }
              };
              const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, "writeln");
              document.open();
              document.writeln(
                "<main id='",
                { toString() { return "line"; } },
                "'>value</main>"
              );
              document.close();
              return [
                typeof descriptor.value,
                descriptor.value.length,
                descriptor.enumerable,
                descriptor.configurable,
                Object.prototype.hasOwnProperty.call(document, "writeln"),
                document.getElementById("line")?.textContent,
                JSON.stringify(document.body.innerHTML),
                probe(() => document.writeln(Symbol("chunk"))),
                document.getElementById("line")?.textContent
              ].join("|");
            })()
            "#,
        )
        .expect("Document.writeln WebIDL variadic argument probe should evaluate");

    assert_eq!(
        result,
        "function|0|true|true|false|value|\"<main id=\\\"line\\\">value</main>\\n\"|TypeError|value"
    );
}

#[test]
fn document_write_replacement_style_sources_drive_has_invalidation() {
    let mut vm = new_storage_test_vm("https://document-write-style-source.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write(`
                <!doctype html>
                <style>body:has(.marker) .target { color: rgb(1, 2, 3); }</style>
                <main><div id="target" class="target"></div></main>
              `);
              document.close();
              const target = document.getElementById("target");
              const before = getComputedStyle(target).color;
              const marker = document.createElement("span");
              marker.className = "marker";
              document.body.appendChild(marker);
              return `${before}|${getComputedStyle(target).color}`;
            })()
            "#,
        )
        .expect("document.write replacement style invalidation probe should evaluate");

    assert_eq!(result, "rgb(0, 0, 0)|rgb(1, 2, 3)");
}

#[test]
fn inline_style_has_not_any_link_invalidates_on_plain_child_insertion() {
    let mut vm = new_storage_test_vm("https://link-pseudo-has-invalidation.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              document.open();
              document.write(`
                <!doctype html>
                <style>
                  #parent { color: rgb(0, 0, 255); }
                  #grandparent { color: rgb(0, 0, 255); }
                  #parent:has(> :not(:link)) { color: rgb(128, 128, 128); }
                  #parent:has(> :link) { color: rgb(0, 128, 0); }
                  #parent:has(> :visited) { color: rgb(255, 0, 0); }
                  #grandparent:has(:not(:any-link)) { color: rgb(128, 128, 128); }
                  #grandparent:has(:any-link) { color: rgb(0, 128, 0); }
                </style>
                <div id="grandparent"></div>
              `);
              document.close();
              const grandparent = document.getElementById("grandparent");
              const before = getComputedStyle(grandparent).color;
              const parent = document.createElement("div");
              parent.id = "parent";
              grandparent.appendChild(parent);
              return [
                before,
                getComputedStyle(grandparent).color,
                getComputedStyle(parent).color
              ].join("|");
            })()
            "#,
        )
        .expect(":not(:any-link) invalidation probe should evaluate");

    assert_eq!(result, "rgb(0, 0, 255)|rgb(128, 128, 128)|rgb(0, 0, 255)");
}
