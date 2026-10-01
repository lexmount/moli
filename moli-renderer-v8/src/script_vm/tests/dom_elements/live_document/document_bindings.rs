use super::*;

#[test]
fn input_show_picker_enforces_brand_without_rejecting_inherited_child_origin() {
    let mut vm = new_storage_test_vm("https://show-picker-origin.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "show-picker-origin-child";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("showPicker child Realm should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "showPicker inherited-origin child Realm",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = document.getElementById("show-picker-origin-child").contentWindow;
  const childInput = child.document.createElement("input");
  const outcome = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error.name;
    }
  };
  const childOutcome = outcome(() => childInput.showPicker());
  return JSON.stringify({
    inheritedOriginPassedSecurityCheck:
      childOutcome === "ok" || childOutcome === "NotAllowedError",
    elementBrand: outcome(() =>
      HTMLInputElement.prototype.showPicker.call(document.createElement("div"))),
    objectBrand: outcome(() => HTMLInputElement.prototype.showPicker.call({}))
  });
})()
"#,
        )
        .expect("showPicker receiver and inherited origin checks should evaluate");

    assert_eq!(
        result,
        r#"{"inheritedOriginPassedSecurityCheck":true,"elementBrand":"TypeError","objectBrand":"TypeError"}"#
    );
}

#[test]
fn cross_realm_dom_bindings_reject_incompatible_receivers_in_their_own_realm() {
    let mut vm = new_storage_test_vm("https://cross-realm-dom-receivers.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "cross-realm-dom-receivers";
  body.appendChild(frame);
})()
"#,
    )
    .expect("cross-realm receiver child frame should be created");
    materialize_single_child_default_realm_for_test(
        &mut vm,
        "cross-realm DOM receiver child Realm",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const other = document.getElementById("cross-realm-dom-receivers").contentWindow;
  const notElement = Object.create(other.HTMLElement.prototype);
  const notText = Object.create(other.Text.prototype);
  const notDocument = Object.create(other.HTMLDocument.prototype);
  const element = other.document.createElement("button");
  const text = other.document.createTextNode("foo");
  const outcome = callback => {
    try {
      callback();
      return "no throw";
    } catch (error) {
      return [
        error.name,
        error instanceof other.TypeError,
        error instanceof TypeError
      ].join(":");
    }
  };

  return [
    outcome(() => { Object.create(other.document).head; }),
    outcome(() => {
      Object.getOwnPropertyDescriptor(other.HTMLElement.prototype, "title")
        .get.call(notElement);
    }),
    outcome(() => {
      Reflect.get(other.document.createElement("div"), "hidden", notElement);
    }),
    outcome(() => { new Proxy(text, {}).nodeType; }),
    outcome(() => { Object.create(element).innerHTML = ""; }),
    outcome(() => {
      Object.getOwnPropertyDescriptor(other.HTMLElement.prototype, "onclick")
        .set.call(notElement, null);
    }),
    outcome(() => { Reflect.set(new other.Text("foo"), "data", "foo", notText); }),
    outcome(() => { new Proxy(other.document, {}).title = ""; }),
    outcome(() => { Object.create(element).click(); }),
    outcome(() => { other.document.querySelector.call(notDocument, "*"); }),
    outcome(() => { Reflect.apply(text.remove, notText, []); }),
    outcome(() => {
      new Proxy(other.document.createElement("a"), {})
        .addEventListener("foo", () => {});
    })
  ].join("|");
})()
"#,
        )
        .expect("cross-realm receiver brand checks should evaluate");

    assert_eq!(
        result,
        std::iter::repeat_n("TypeError:true:false", 12)
            .collect::<Vec<_>>()
            .join("|")
    );
}

#[test]
fn document_metadata_view_focus_and_state_validate_native_receivers() {
    let mut vm = new_storage_test_vm("https://document-receivers.test/");
    vm.eval(
        r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.id = 'document-receiver-child';
root.appendChild(frame);
"#,
    )
    .expect("receiver test child should be created");
    materialize_single_child_default_realm_for_test(&mut vm, "Document receiver child realm");

    let fixture = include_str!("../../../../../tests/fixtures/document-receivers.js");
    vm.eval(&format!(
        "{fixture}\ndocumentReceiverProbe().then(value => {{ globalThis.documentReceiverResult = value; }}, error => {{ globalThis.documentReceiverResult = {{error: String(error.stack || error)}}; }});"
    )).expect("Document receiver probe should start");
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(documentReceiverResult)").unwrap()).unwrap();
    let checks = result["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("{result}"));
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["pass"] != true)
        .collect();
    assert_eq!(result["state"], "pass", "{failures:?}");
    assert_eq!(checks.len(), 111);
}

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
fn html_element_hidden_reflects_nullable_boolean_number_and_string_union() {
    let mut vm = new_parsed_test_vm(
        "https://hidden-reflection.test/",
        "<!doctype html><div id=target></div>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.getElementById("target");
              const assignments = [
                [false, false, null],
                [true, true, ""],
                ["foo", true, ""],
                ["", false, null],
                ["UnTiL-FoUnD", "until-found", "until-found"],
                [null, false, null],
                [undefined, false, null],
                [1, true, ""],
                [0, false, null],
                [NaN, false, null],
                [{ toString() { return "UNTIL-FOUND"; } }, "until-found", "until-found"]
              ];
              const assignmentResults = assignments.map(([value, expected, attribute]) => {
                target.hidden = value;
                return target.hidden === expected && target.getAttribute("hidden") === attribute;
              });
              target.setAttribute("hidden", "uNtIl-FoUnD");
              return JSON.stringify({
                assignmentResults,
                canonicalGetter: target.hidden
              });
            })()
            "#,
        )
        .expect("HTMLElement.hidden union reflection should evaluate");

    assert_eq!(
        result,
        r#"{"assignmentResults":[true,true,true,true,true,true,true,true,true,true,true],"canonicalGetter":"until-found"}"#
    );
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
              const throwsTypeError = callback => {
                try {
                  callback();
                } catch (error) {
                  return error instanceof TypeError;
                }
                return false;
              };

              const htmlNames = [
                "title",
                "lang",
                "autocapitalize",
                "autocorrect",
                "translate",
                "dir",
                "hidden",
                "inert",
                "accessKey",
                "draggable",
                "spellcheck",
                "writingSuggestions",
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

              const mixinNames = ["focusGroup", "focusGroupStart", "autofocus", "tabIndex"];
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
              div.autocorrect = false;
              div.translate = false;
              div.dir = "RTL";
              div.hidden = true;
              div.inert = true;
              div.accessKey = "x";
              div.draggable = true;
              div.spellcheck = false;
              div.writingSuggestions = false;
              div.contentEditable = "plaintext-only";
              div.enterKeyHint = "Go";
              div.inputMode = "NUMERIC";
              div.innerText = "hello text";
              div.popover = "hint";
              div.focusGroup = "toolbar";
              div.focusGroupStart = true;
              div.autofocus = true;
              div.tabIndex = 5;

              assert(div.title === "hello", "title behavior");
              assert(div.lang === "en-US", "lang behavior");
              assert(div.autocapitalize === "words" && div.getAttribute("autocapitalize") === "WORDS", "autocapitalize behavior");
              assert(div.autocorrect === false && div.getAttribute("autocorrect") === "off", "autocorrect behavior");
              assert(div.translate === false && div.getAttribute("translate") === "no", "translate behavior");
              assert(div.dir === "rtl", "dir behavior");
              assert(div.hidden === true && div.hasAttribute("hidden"), "hidden behavior");
              assert(div.inert === true && div.hasAttribute("inert"), "inert behavior");
              const inertDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "inert");
              assert(throwsTypeError(() => inertDescriptor.get.call(HTMLElement.prototype)), "inert getter brand");
              assert(throwsTypeError(() => inertDescriptor.set.call(HTMLElement.prototype, true)), "inert setter brand");
              assert(div.accessKey === "x", "accessKey behavior");
              assert(div.draggable === true && div.getAttribute("draggable") === "true", "draggable behavior");
              assert(div.spellcheck === false && div.getAttribute("spellcheck") === "false", "spellcheck behavior");
              assert(div.writingSuggestions === "false" && div.getAttribute("writingsuggestions") === "false", "writingSuggestions behavior");
              assert(div.contentEditable === "plaintext-only" && div.isContentEditable === true, "contentEditable behavior");
              assert(div.enterKeyHint === "go", "enterKeyHint behavior");
              assert(div.inputMode === "numeric", "inputMode behavior");
              assert(div.innerText === "hello text" && div.outerText === "hello text", "innerText/outerText behavior");
              assert(div.popover === "hint", "popover behavior");
              assert(
                div.focusGroup === "toolbar" &&
                  div.getAttribute("focusgroup") === "toolbar" &&
                  div.focusGroupStart === true &&
                  div.hasAttribute("focusgroupstart") &&
                  div.autofocus === true &&
                  div.tabIndex === 5,
                "HTMLOrSVGOrMathMLElement behavior"
              );

              const holder = document.createElement("section");
              const para = document.createElement("p");
              para.textContent = "old";
              holder.appendChild(para);
              para.outerText = "new";
              assert(holder.textContent === "new" && holder.firstChild.nodeType === Node.TEXT_NODE, "outerText setter behavior");

              svg.tabIndex = 9;
              svg.autofocus = true;
              svg.focusGroup = "grid";
              svg.focusGroupStart = true;
              assert(svg.tabIndex === 9 && svg.getAttribute("tabindex") === "9", "svg tabIndex behavior");
              assert(svg.autofocus === true && svg.hasAttribute("autofocus"), "svg autofocus behavior");
              assert(svg.focusGroup === "grid" && svg.focusGroupStart === true, "svg focusgroup behavior");
              const math = document.createElementNS("http://www.w3.org/1998/Math/MathML", "math");
              math.focusGroup = "tablist inline";
              math.focusGroupStart = true;
              assert(
                math.focusGroup === "tablist inline" && math.focusGroupStart === true,
                "MathML focusgroup behavior"
              );
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
              assert(xml.images instanceof HTMLCollection && xml.images.length === 0, "xml images value");
              assert(xml.hidden === true, "xml hidden value");
              assert(xml.visibilityState === "hidden", "xml visibility value");

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
fn document_collection_getters_preserve_receiver_realm_and_validate_brand() {
    let mut vm = new_parsed_test_vm(
        "https://document-collection-realm.test/",
        "<!doctype html><html><body><iframe id='child'></iframe></body></html>",
    );
    materialize_single_child_default_realm_for_test(&mut vm, "Document collection child realm");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const child = document.getElementById("child").contentWindow;
  const doc = child.document;
  for (const name of ["forms", "images", "scripts", "links", "anchors", "embeds", "plugins", "applets"]) {
    const getter = Object.getOwnPropertyDescriptor(Document.prototype, name).get;
    const collection = getter.call(doc);
    assert(Object.getPrototypeOf(collection) === child.HTMLCollection.prototype, `${name} realm`);
    assert(collection === doc[name], `${name} SameObject across realms`);
    for (const receiver of [{}, document.createElement("div"), document.createDocumentFragment(), null]) {
      let error;
      try { getter.call(receiver); } catch (caught) { error = caught; }
      assert(error instanceof TypeError, `${name} receiver brand`);
    }
  }
  const embeds = doc.embeds;
  const embed = doc.createElement("embed");
  const foreign = doc.createElementNS("urn:foreign", "embed");
  doc.body.appendChild(foreign);
  assert(embeds.length === 0, "foreign embed excluded");
  doc.body.appendChild(embed);
  assert(embeds.length === 1 && embeds[0] === embed, "live child collection");
  assert(embeds === doc.plugins, "child plugins aliases embeds");
  embed.remove();
  assert(embeds.length === 0, "child collection removal");
  return "ok";
})()
"#,
        )
        .expect("Document collection realm and brand probes should evaluate");

    assert_eq!(result, "ok");
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
fn tab_index_getter_uses_the_spec_default_element_set() {
    let mut vm = new_parsed_test_vm(
        "https://tab-index-defaults.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const svg = "http://www.w3.org/2000/svg";
              const mathml = "http://www.w3.org/1998/Math/MathML";
              const other = "https://tab-index-defaults.test/namespace";
              const exercise = ownerDocument => {
                for (const name of [
                  "a", "area", "button", "frame", "iframe", "input", "object", "select", "textarea"
                ]) {
                  const element = ownerDocument.createElement(name);
                  assert(element.tabIndex === 0, `${name} default`);
                }

                const div = ownerDocument.createElement("div");
                assert(div.tabIndex === -1, "ordinary HTML element default");

                const svgAnchor = ownerDocument.createElementNS(svg, "a");
                const svgGroup = ownerDocument.createElementNS(svg, "g");
                const mathAnchor = ownerDocument.createElementNS(mathml, "a");
                const mathRow = ownerDocument.createElementNS(mathml, "mrow");
                const foreignAnchor = ownerDocument.createElementNS(other, "a");
                assert(svgAnchor.tabIndex === 0, "SVG a default");
                assert(svgGroup.tabIndex === -1, "other SVG element default");
                assert(mathAnchor.tabIndex === 0, "MathML a default");
                assert(mathRow.tabIndex === -1, "other MathML element default");
                assert(foreignAnchor.tabIndex === undefined, "unrelated namespace has no mixin");

                const details = ownerDocument.createElement("details");
                const nonSummary = ownerDocument.createElement("span");
                const firstSummary = ownerDocument.createElement("summary");
                const secondSummary = ownerDocument.createElement("summary");
                details.append(nonSummary, firstSummary, secondSummary);
                assert(firstSummary.tabIndex === 0, "first summary child default");
                assert(secondSummary.tabIndex === -1, "later summary child default");
                assert(ownerDocument.createElement("summary").tabIndex === -1,
                  "orphan summary default");

                firstSummary.remove();
                assert(secondSummary.tabIndex === 0, "summary default follows tree mutation");
                details.append(firstSummary);
                assert(secondSummary.tabIndex === 0 && firstSummary.tabIndex === -1,
                  "first summary is based on current child order");

                const wrapper = ownerDocument.createElement("div");
                const nestedSummary = ownerDocument.createElement("summary");
                wrapper.append(nestedSummary);
                details.prepend(wrapper);
                assert(nestedSummary.tabIndex === -1, "nested summary is not a details summary");

                secondSummary.setAttribute("tabindex", "-7");
                assert(secondSummary.tabIndex === -7, "explicit valid tabindex wins");
                secondSummary.setAttribute("tabindex", "invalid");
                assert(secondSummary.tabIndex === 0, "invalid tabindex uses element default");
                div.setAttribute("tabindex", "invalid");
                assert(div.tabIndex === -1, "invalid tabindex uses ordinary default");
              };

              exercise(document);
              return "ok";
            })()
            "#,
        )
        .expect("tabIndex default getter behavior should evaluate");

    assert_eq!(result, "ok");
}
