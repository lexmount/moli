use super::*;

#[test]
fn dom_api_known_pseudo_element_selectors_with_after_part_pseudo_classes_return_empty() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = (target, selector) => {
                try {
                  return String(target.querySelector(selector));
                } catch (error) {
                  return `throw:${error && error.name}:${error && error.message}`;
                }
              };
              const parsed = new DOMParser().parseFromString("<main></main>", "text/html");
              return JSON.stringify({
                documentHover: probe(document, "::part(label):hover"),
                documentLang: probe(document, "::part(label):lang(en)"),
                detachedHover: probe(parsed, "::part(label):hover"),
                invalidStructural: probe(document, "::part(label):first-child")
              });
            })()
            "#,
        )
        .expect("DOM API pseudo-element selector probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["documentHover"], "null");
    assert_eq!(result["documentLang"], "null");
    assert_eq!(result["detachedHover"], "null");
    assert!(
        result["invalidStructural"]
            .as_str()
            .is_some_and(|value| value.starts_with("throw:SyntaxError:"))
    );
}
#[test]
fn dom_core_prototype_accessors_brand_check_live_and_detached_receivers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const outcome = callback => {
                try {
                  const value = callback();
                  return `ok:${value === null ? "null" : String(value)}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const getter = (prototype, name, receiver) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.get.call(receiver));
              };
              const setter = (prototype, name, receiver, value) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.set.call(receiver, value));
              };

              const parsed = new DOMParser().parseFromString(
                '<!doctype html PUBLIC "pub" "sys"><html><body><div id="det"><span>b</span></div></body></html>',
                "text/html"
              );
              const detached = parsed.getElementById("det");
              const live = document.createElement("section");
              const piDoc = document.implementation.createDocument("", "root");
              const pi = piDoc.createProcessingInstruction("xml-stylesheet", 'href="x"');
              piDoc.appendChild(pi);

              return JSON.stringify({
                liveTagName: getter(Element.prototype, "tagName", live),
                detachedTagName: getter(Element.prototype, "tagName", detached),
                detachedLocalName: getter(Element.prototype, "localName", detached),
                detachedNamespaceURI: getter(Element.prototype, "namespaceURI", detached),
                detachedPrefix: getter(Element.prototype, "prefix", detached),
                detachedInnerHTML: getter(Element.prototype, "innerHTML", detached),
                detachedOuterHTML: getter(Element.prototype, "outerHTML", detached),
                setDetachedInnerHTML: setter(Element.prototype, "innerHTML", detached, "<em>c</em>"),
                detachedInnerHTMLAfterSet: detached.innerHTML,
                fakeTagName: getter(Element.prototype, "tagName", {}),
                documentTagName: getter(Element.prototype, "tagName", document),
                fakeLocalName: getter(Element.prototype, "localName", {}),
                fakeNamespaceURI: getter(Element.prototype, "namespaceURI", {}),
                fakePrefix: getter(Element.prototype, "prefix", {}),
                fakeInnerHTML: getter(Element.prototype, "innerHTML", {}),
                documentInnerHTML: getter(Element.prototype, "innerHTML", document),
                fakeOuterHTML: getter(Element.prototype, "outerHTML", {}),
                documentOuterHTML: getter(Element.prototype, "outerHTML", document),
                fakeInnerHTMLSet: setter(Element.prototype, "innerHTML", {}, "x"),
                documentOuterHTMLSet: setter(Element.prototype, "outerHTML", document, "x"),
                detachedPublicId: getter(DocumentType.prototype, "publicId", parsed.doctype),
                detachedSystemId: getter(DocumentType.prototype, "systemId", parsed.doctype),
                fakePublicId: getter(DocumentType.prototype, "publicId", {}),
                documentPublicId: getter(DocumentType.prototype, "publicId", document),
                piTarget: getter(ProcessingInstruction.prototype, "target", pi),
                fakePiTarget: getter(ProcessingInstruction.prototype, "target", {}),
                textPiTarget: getter(ProcessingInstruction.prototype, "target", document.createTextNode("x"))
              });
            })()
            "#,
        )
        .expect("DOM core accessor brand-check probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["liveTagName"], "ok:SECTION");
    assert_eq!(result["detachedTagName"], "ok:DIV");
    assert_eq!(result["detachedLocalName"], "ok:div");
    assert_eq!(
        result["detachedNamespaceURI"],
        "ok:http://www.w3.org/1999/xhtml"
    );
    assert_eq!(result["detachedPrefix"], "ok:null");
    assert_eq!(result["detachedInnerHTML"], "ok:<span>b</span>");
    assert_eq!(
        result["detachedOuterHTML"],
        "ok:<div id=\"det\"><span>b</span></div>"
    );
    assert_eq!(result["setDetachedInnerHTML"], "ok:undefined");
    assert_eq!(result["detachedInnerHTMLAfterSet"], "<em>c</em>");
    assert_eq!(result["detachedPublicId"], "ok:pub");
    assert_eq!(result["detachedSystemId"], "ok:sys");
    assert_eq!(result["piTarget"], "ok:xml-stylesheet");
    for key in [
        "fakeTagName",
        "documentTagName",
        "fakeLocalName",
        "fakeNamespaceURI",
        "fakePrefix",
        "fakeInnerHTML",
        "documentInnerHTML",
        "fakeOuterHTML",
        "documentOuterHTML",
        "fakeInnerHTMLSet",
        "documentOuterHTMLSet",
        "fakePublicId",
        "documentPublicId",
        "fakePiTarget",
        "textPiTarget",
    ] {
        assert_eq!(result[key], "throw:TypeError", "{key}");
    }
}
#[test]
fn node_and_mixin_prototype_members_brand_check_live_and_detached_receivers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const outcome = callback => {
                try {
                  const value = callback();
                  return `ok:${value === null ? "null" : String(value)}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const getter = (prototype, name, receiver) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.get.call(receiver));
              };
              const setter = (prototype, name, receiver, value) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.set.call(receiver, value));
              };
              const method = (prototype, name, receiver, ...args) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.value.call(receiver, ...args));
              };

              const parsed = new DOMParser().parseFromString(
                '<!doctype html><html><body><div id="parent"><em id="prev"></em><span id="child">t</span></div></body></html>',
                "text/html"
              );
              const detachedParent = parsed.getElementById("parent");
              const detachedChild = parsed.getElementById("child");
              const liveParent = document.createElement("div");
              liveParent.append(document.createElement("b"), document.createTextNode("x"));
              const liveText = document.createTextNode("x");
              const liveChild = document.createElement("span");
              liveParent.append(liveChild);
              const fragment = document.createDocumentFragment();
              fragment.append(document.createElement("i"));
              const docWithDoctype = document.implementation.createDocument(null, "root", null);
              const doctype = document.implementation.createDocumentType("root", "", "");
              docWithDoctype.insertBefore(doctype, docWithDoctype.documentElement);

              const nodeGetterNames = [
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
              const nodeMethodArgs = {
                appendChild: [liveText],
                insertBefore: [liveText, null],
                removeChild: [liveText],
                replaceChild: [liveText, liveText],
                cloneNode: [],
                contains: [liveText],
                hasChildNodes: [],
                isSameNode: [liveText],
                isEqualNode: [liveText],
                compareDocumentPosition: [liveText],
                getRootNode: [],
                lookupPrefix: ["urn:x"],
                lookupNamespaceURI: [null],
                isDefaultNamespace: [null],
                normalize: []
              };
              const ownNames = [
                "children",
                "firstElementChild",
                "lastElementChild",
                "childElementCount",
                "append",
                "prepend",
                "replaceChildren",
                "querySelector",
                "querySelectorAll",
                "before",
                "after",
                "replaceWith",
                "remove",
                "previousElementSibling",
                "nextElementSibling"
              ];
              return JSON.stringify({
                nodeTypeLive: getter(Node.prototype, "nodeType", liveParent),
                nodeTypeDetached: getter(Node.prototype, "nodeType", detachedChild),
                nodeTextDetached: getter(Node.prototype, "textContent", detachedChild),
                setDetachedText: setter(Node.prototype, "textContent", detachedChild, "det"),
                detachedTextAfterSet: detachedChild.textContent,
                containsDetached: method(Node.prototype, "contains", detachedParent, detachedChild),
                hasDetachedChildren: method(Node.prototype, "hasChildNodes", detachedParent),
                cloneDetachedType: outcome(() => Object.getOwnPropertyDescriptor(Node.prototype, "cloneNode").value.call(detachedChild, true).nodeType),
                childrenLiveLength: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "children").get.call(liveParent).length),
                childrenDetachedLength: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "children").get.call(detachedParent).length),
                fragmentChildElementCount: getter(DocumentFragment.prototype, "childElementCount", fragment),
                queryDetached: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "querySelector").value.call(detachedParent, "#child").id),
                appendLive: method(Element.prototype, "append", liveParent, "tail"),
                beforeDetached: method(Element.prototype, "before", detachedChild, "lead"),
                prevElementDetached: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "previousElementSibling").get.call(detachedChild).id),
                fakeNodeGetters: Object.fromEntries(nodeGetterNames.map(name => [name, getter(Node.prototype, name, {})])),
                fakeNodeValueSet: setter(Node.prototype, "nodeValue", {}, "x"),
                fakeTextContentSet: setter(Node.prototype, "textContent", {}, "x"),
                fakeNodeMethods: Object.fromEntries(Object.entries(nodeMethodArgs).map(([name, args]) => [name, method(Node.prototype, name, {}, ...args)])),
                parentGetterOnText: getter(Element.prototype, "children", liveText),
                parentCountOnText: getter(Element.prototype, "childElementCount", liveText),
                parentMethodOnText: method(Element.prototype, "append", liveText, "x"),
                queryOnText: method(Element.prototype, "querySelector", liveText, "*"),
                childMethodOnDocument: method(Element.prototype, "remove", document),
                nonDocumentTypePrevOnDoctype: getter(Element.prototype, "previousElementSibling", doctype),
                nonDocumentTypeNextOnDoctype: getter(Element.prototype, "nextElementSibling", doctype),
                fakePreviousElementSibling: getter(Element.prototype, "previousElementSibling", {}),
                ownSurfaceClean: [liveParent, detachedParent, liveText, detachedChild].every(target =>
                  ownNames.every(name => !Object.prototype.hasOwnProperty.call(target, name))
                ),
                prototypeOwners: [
                  Object.prototype.hasOwnProperty.call(Element.prototype, "children"),
                  Object.prototype.hasOwnProperty.call(Document.prototype, "children"),
                  Object.prototype.hasOwnProperty.call(DocumentFragment.prototype, "children"),
                  Object.prototype.hasOwnProperty.call(Element.prototype, "append"),
                  Object.prototype.hasOwnProperty.call(Element.prototype, "before"),
                  Object.prototype.hasOwnProperty.call(CharacterData.prototype, "before"),
                  Object.prototype.hasOwnProperty.call(Element.prototype, "previousElementSibling"),
                  Object.prototype.hasOwnProperty.call(CharacterData.prototype, "previousElementSibling")
                ].every(Boolean)
              });
            })()
            "##,
        )
        .expect("Node and mixin prototype brand-check probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["nodeTypeLive"], "ok:1");
    assert_eq!(result["nodeTypeDetached"], "ok:1");
    assert_eq!(result["nodeTextDetached"], "ok:t");
    assert_eq!(result["setDetachedText"], "ok:undefined");
    assert_eq!(result["detachedTextAfterSet"], "det");
    assert_eq!(result["containsDetached"], "ok:true");
    assert_eq!(result["hasDetachedChildren"], "ok:true");
    assert_eq!(result["cloneDetachedType"], "ok:1");
    assert_eq!(result["childrenLiveLength"], "ok:2");
    assert_eq!(result["childrenDetachedLength"], "ok:2");
    assert_eq!(result["fragmentChildElementCount"], "ok:1");
    assert_eq!(result["queryDetached"], "ok:child");
    assert_eq!(result["appendLive"], "ok:undefined");
    assert_eq!(result["beforeDetached"], "ok:undefined");
    assert_eq!(result["prevElementDetached"], "ok:prev");
    assert_eq!(result["fakeNodeValueSet"], "throw:TypeError");
    assert_eq!(result["fakeTextContentSet"], "throw:TypeError");
    for name in [
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
        "textContent",
    ] {
        assert_eq!(result["fakeNodeGetters"][name], "throw:TypeError", "{name}");
    }
    for name in [
        "appendChild",
        "insertBefore",
        "removeChild",
        "replaceChild",
        "cloneNode",
        "contains",
        "hasChildNodes",
        "isSameNode",
        "isEqualNode",
        "compareDocumentPosition",
        "getRootNode",
        "lookupPrefix",
        "lookupNamespaceURI",
        "isDefaultNamespace",
        "normalize",
    ] {
        assert_eq!(result["fakeNodeMethods"][name], "throw:TypeError", "{name}");
    }
    for key in [
        "parentGetterOnText",
        "parentCountOnText",
        "parentMethodOnText",
        "queryOnText",
        "childMethodOnDocument",
        "nonDocumentTypePrevOnDoctype",
        "nonDocumentTypeNextOnDoctype",
        "fakePreviousElementSibling",
    ] {
        assert_eq!(result[key], "throw:TypeError", "{key}");
    }
    assert_eq!(result["ownSurfaceClean"], true);
    assert_eq!(result["prototypeOwners"], true);
}
#[test]
fn element_prototype_members_brand_check_live_and_detached_receivers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const outcome = callback => {
                try {
                  const value = callback();
                  if (value === null) return "ok:null";
                  if (value === undefined) return "ok:undefined";
                  if (typeof value === "object") return `ok:${Object.prototype.toString.call(value)}`;
                  return `ok:${String(value)}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const getter = (prototype, name, receiver) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.get.call(receiver));
              };
              const setter = (prototype, name, receiver, value) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.set.call(receiver, value));
              };
              const method = (prototype, name, receiver, ...args) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return outcome(() => descriptor.value.call(receiver, ...args));
              };

              const parsed = new DOMParser().parseFromString(
                '<!doctype html><html><body><div id="detached" class="box"><span class="child" name="n"></span></div></body></html>',
                "text/html"
              );
              const detached = parsed.getElementById("detached");
              const live = document.createElement("div");
              live.id = "live";
              live.className = "box";
              live.innerHTML = '<span class="child" name="n"></span>';
              const fragment = document.createDocumentFragment();
              fragment.appendChild(document.createElement("span"));
              const shadowHost = document.createElement("section");
              const shadowRoot = shadowHost.attachShadow({ mode: "open" });
              shadowRoot.innerHTML = "<span></span>";
              const text = document.createTextNode("x");
              const attr = document.createAttribute("data-new");
              attr.value = "new";
              const fake = {};

              const getterNames = [
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
                "customElementRegistry"
              ];
              const setterArgs = {
                id: "x",
                className: "x",
                innerHTML: "<b>x</b>",
                outerHTML: "<section></section>",
                classList: "x",
                part: "x"
              };
              const methodArgs = {
                getBoundingClientRect: [],
                getClientRects: [],
                hasAttribute: ["id"],
                hasAttributeNS: [null, "id"],
                hasAttributes: [],
                getAttributeNames: [],
                getAttribute: ["id"],
                getAttributeNS: [null, "id"],
                setAttribute: ["data-x", "1"],
                setAttributeNS: [null, "data-y", "2"],
                removeAttribute: ["data-x"],
                removeAttributeNS: [null, "data-y"],
                toggleAttribute: ["hidden"],
                getAttributeNode: ["id"],
                getAttributeNodeNS: [null, "id"],
                setAttributeNode: [attr],
                removeAttributeNode: [attr],
                matches: ["div.box"],
                closest: ["div"],
                getElementsByTagName: ["span"],
                getElementsByTagNameNS: ["*", "*"],
                getElementsByClassName: ["child"],
                getElementsByName: ["n"],
                getHTML: [],
                setHTMLUnsafe: ["<i></i>"]
              };

              return JSON.stringify({
                idLive: getter(Element.prototype, "id", live),
                idDetached: getter(Element.prototype, "id", detached),
                tagDetached: getter(Element.prototype, "tagName", detached),
                innerDetached: getter(Element.prototype, "innerHTML", detached),
                getAttributeLive: method(Element.prototype, "getAttribute", live, "id"),
                getAttributeDetached: method(Element.prototype, "getAttribute", detached, "id"),
                rectLive: method(Element.prototype, "getBoundingClientRect", live),
                matchesDetached: method(Element.prototype, "matches", detached, "div.box"),
                closestDetached: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "closest").value.call(detached.querySelector("span"), "div").id),
                tagCollectionLive: outcome(() => Object.getOwnPropertyDescriptor(Element.prototype, "getElementsByTagName").value.call(live, "span").length),
                tagCollectionDocument: outcome(() => Object.getOwnPropertyDescriptor(Document.prototype, "getElementsByTagName").value.call(document, "body").length),
                tagCollectionFragment: outcome(() => Object.getOwnPropertyDescriptor(DocumentFragment.prototype, "getElementsByTagName").value.call(fragment, "span").length),
                tagCollectionShadowRoot: outcome(() => Object.getOwnPropertyDescriptor(ShadowRoot.prototype, "getElementsByTagName").value.call(shadowRoot, "span").length),
                setHTMLUnsafeLive: method(Element.prototype, "setHTMLUnsafe", live, "<em></em>"),
                getHTMLLive: method(Element.prototype, "getHTML", live),
                documentCustomRegistry: getter(Document.prototype, "customElementRegistry", document),
                shadowRootCustomRegistry: getter(ShadowRoot.prototype, "customElementRegistry", shadowRoot),
                fakeGetters: Object.fromEntries(getterNames.map(name => [name, getter(Element.prototype, name, fake)])),
                textGetters: Object.fromEntries(getterNames.map(name => [name, getter(Element.prototype, name, text)])),
                fakeSetters: Object.fromEntries(Object.entries(setterArgs).map(([name, value]) => [name, setter(Element.prototype, name, fake, value)])),
                textSetters: Object.fromEntries(Object.entries(setterArgs).map(([name, value]) => [name, setter(Element.prototype, name, text, value)])),
                fakeMethods: Object.fromEntries(Object.entries(methodArgs).map(([name, args]) => [name, method(Element.prototype, name, fake, ...args)])),
                textMethods: Object.fromEntries(Object.entries(methodArgs).map(([name, args]) => [name, method(Element.prototype, name, text, ...args)])),
                ownSurfaceClean: [live, detached].every(target =>
                  getterNames.every(name => !Object.prototype.hasOwnProperty.call(target, name)) &&
                  Object.keys(methodArgs).every(name => !Object.prototype.hasOwnProperty.call(target, name))
                )
              });
            })()
            "##,
        )
        .expect("Element prototype brand-check probe should evaluate");

    let result: serde_json::Value =
        serde_json::from_str(&result).expect("probe result should be JSON");
    assert_eq!(result["idLive"], "ok:live");
    assert_eq!(result["idDetached"], "ok:detached");
    assert_eq!(result["tagDetached"], "ok:DIV");
    assert_eq!(
        result["innerDetached"],
        r#"ok:<span class="child" name="n"></span>"#
    );
    assert_eq!(result["getAttributeLive"], "ok:live");
    assert_eq!(result["getAttributeDetached"], "ok:detached");
    assert_eq!(result["rectLive"], "ok:[object DOMRect]");
    assert_eq!(result["matchesDetached"], "ok:true");
    assert_eq!(result["closestDetached"], "ok:detached");
    assert_eq!(result["tagCollectionLive"], "ok:1");
    assert_eq!(result["tagCollectionDocument"], "ok:0");
    assert_eq!(result["tagCollectionFragment"], "ok:1");
    assert_eq!(result["tagCollectionShadowRoot"], "ok:1");
    assert_eq!(result["setHTMLUnsafeLive"], "ok:undefined");
    assert_eq!(result["getHTMLLive"], "ok:<em></em>");
    assert_eq!(
        result["documentCustomRegistry"],
        "ok:[object CustomElementRegistry]"
    );
    assert_eq!(
        result["shadowRootCustomRegistry"],
        "ok:[object CustomElementRegistry]"
    );
    assert_eq!(result["ownSurfaceClean"], true);
    for name in [
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
        "customElementRegistry",
    ] {
        assert_eq!(result["fakeGetters"][name], "throw:TypeError", "{name}");
        assert_eq!(result["textGetters"][name], "throw:TypeError", "{name}");
    }
    for name in [
        "id",
        "className",
        "innerHTML",
        "outerHTML",
        "classList",
        "part",
    ] {
        assert_eq!(result["fakeSetters"][name], "throw:TypeError", "{name}");
        assert_eq!(result["textSetters"][name], "throw:TypeError", "{name}");
    }
    for name in [
        "getBoundingClientRect",
        "getClientRects",
        "hasAttribute",
        "hasAttributeNS",
        "hasAttributes",
        "getAttributeNames",
        "getAttribute",
        "getAttributeNS",
        "setAttribute",
        "setAttributeNS",
        "removeAttribute",
        "removeAttributeNS",
        "toggleAttribute",
        "getAttributeNode",
        "getAttributeNodeNS",
        "setAttributeNode",
        "removeAttributeNode",
        "matches",
        "closest",
        "getElementsByTagName",
        "getElementsByTagNameNS",
        "getElementsByClassName",
        "getElementsByName",
        "getHTML",
        "setHTMLUnsafe",
    ] {
        assert_eq!(result["fakeMethods"][name], "throw:TypeError", "{name}");
        assert_eq!(result["textMethods"][name], "throw:TypeError", "{name}");
    }
}
#[test]
fn domexception_surface_matches_upstream_wpt_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const exception = new DOMException("message", "InvalidCharacterError");
              const messageDesc = Object.getOwnPropertyDescriptor(DOMException.prototype, "message");
              const nameDesc = Object.getOwnPropertyDescriptor(DOMException.prototype, "name");
              const codeDesc = Object.getOwnPropertyDescriptor(DOMException.prototype, "code");
              exception.message = "spoof";
              let getterBrandCheck = false;
              let prototypeToStringBrandCheck = false;
              try {
                messageDesc.get.call({});
              } catch (error) {
                getterBrandCheck = error instanceof TypeError;
              }
              try {
                DOMException.prototype.toString();
              } catch (error) {
                prototypeToStringBrandCheck = error instanceof TypeError;
              }
              return JSON.stringify({
                protoExtendsError: Object.getPrototypeOf(DOMException.prototype) === Error.prototype,
                ownMessage: Object.prototype.hasOwnProperty.call(exception, "message"),
                ownName: Object.prototype.hasOwnProperty.call(exception, "name"),
                ownCode: Object.prototype.hasOwnProperty.call(exception, "code"),
                ownMessageDescriptor: Object.getOwnPropertyDescriptor(exception, "message") ?? null,
                ownNameDescriptor: Object.getOwnPropertyDescriptor(exception, "name") ?? null,
                ownCodeDescriptor: Object.getOwnPropertyDescriptor(exception, "code") ?? null,
                messageGetterType: typeof messageDesc?.get,
                messageGetterName: messageDesc?.get?.name,
                messageGetterLength: messageDesc?.get?.length,
                messageSetterType: typeof messageDesc?.set,
                messageEnumerable: !!messageDesc?.enumerable,
                messageConfigurable: !!messageDesc?.configurable,
                nameGetterType: typeof nameDesc?.get,
                nameGetterName: nameDesc?.get?.name,
                nameGetterLength: nameDesc?.get?.length,
                nameSetterType: typeof nameDesc?.set,
                nameEnumerable: !!nameDesc?.enumerable,
                nameConfigurable: !!nameDesc?.configurable,
                codeGetterType: typeof codeDesc?.get,
                codeGetterName: codeDesc?.get?.name,
                codeGetterLength: codeDesc?.get?.length,
                codeSetterType: typeof codeDesc?.set,
                codeEnumerable: !!codeDesc?.enumerable,
                codeConfigurable: !!codeDesc?.configurable,
                messageAssignOwn: Object.prototype.hasOwnProperty.call(exception, "message"),
                messageAfterAssign: exception.message,
                ownToString: Object.prototype.hasOwnProperty.call(exception, "toString"),
                protoOwnToString: Object.prototype.hasOwnProperty.call(DOMException.prototype, "toString"),
                stringified: exception.toString(),
                getterBrandCheck,
                prototypeToStringBrandCheck
              });
            })()
            "#,
        )
        .expect("DOMException surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"protoExtendsError":true,"ownMessage":false,"ownName":false,"ownCode":false,"ownMessageDescriptor":null,"ownNameDescriptor":null,"ownCodeDescriptor":null,"messageGetterType":"function","messageGetterName":"get message","messageGetterLength":0,"messageSetterType":"undefined","messageEnumerable":true,"messageConfigurable":true,"nameGetterType":"function","nameGetterName":"get name","nameGetterLength":0,"nameSetterType":"undefined","nameEnumerable":true,"nameConfigurable":true,"codeGetterType":"function","codeGetterName":"get code","codeGetterLength":0,"codeSetterType":"undefined","codeEnumerable":true,"codeConfigurable":true,"messageAssignOwn":false,"messageAfterAssign":"message","ownToString":false,"protoOwnToString":false,"stringified":"InvalidCharacterError: message","getterBrandCheck":true,"prototypeToStringBrandCheck":true}"#
    );
}
#[test]
fn meter_progress_numeric_setters_parse_webidl_values() {
    let mut vm = new_storage_test_vm("https://forms-meter-progress-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const meter = document.createElement('meter');
              const progress = document.createElement('progress');
              let meterValueCalls = 0;
              let progressValueCalls = 0;
              let progressMaxCalls = 0;
              function probe(callback) {
                try {
                  return String(callback());
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              meter.value = {
                valueOf() {
                  meterValueCalls += 1;
                  return '0.75';
                }
              };
              const meterValueObject = `${meter.value}:${meterValueCalls}`;
              const meterInfinity = probe(() => { meter.value = Infinity; });
              const meterSymbol = probe(() => { meter.min = Symbol('min'); });
              const meterThrowing = probe(() => {
                meter.max = {
                  valueOf() {
                    throw new RangeError('max');
                  }
                };
              });
              meter.low = null;
              const meterNull = meter.low;
              progress.value = {
                valueOf() {
                  progressValueCalls += 1;
                  return '0.4';
                }
              };
              const progressValueObject = `${progress.value}:${progressValueCalls}`;
              progress.max = {
                valueOf() {
                  progressMaxCalls += 1;
                  return '4.5';
                }
              };
              const progressMaxObject = `${progress.max}:${progressMaxCalls}`;
              const progressZero = probe(() => { progress.max = 0; return progress.max; });
              const progressBadString = probe(() => { progress.value = 'bad'; });
              const progressSymbol = probe(() => { progress.max = Symbol('max'); });
              const progressThrowing = probe(() => {
                progress.value = {
                  valueOf() {
                    throw new RangeError('value');
                  }
                };
              });
              return [
                meterValueObject,
                meterInfinity,
                meterSymbol,
                meterThrowing,
                meterNull,
                progressValueObject,
                progressMaxObject,
                progressZero,
                progressBadString,
                progressSymbol,
                progressThrowing
              ].join('|');
            })()
            "#,
        )
        .expect("meter/progress numeric setters should parse WebIDL values");

    assert_eq!(
        result,
        "0.75:1|throw:TypeError|throw:TypeError|throw:RangeError|0|0.4:1|4.5:1|4.5|throw:TypeError|throw:TypeError|throw:RangeError"
    );
}
#[test]
fn non_document_wrappers_do_not_leak_document_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const div = document.createElement("div");
              const text = document.createTextNode("x");
              return [
                typeof div.createElement,
                "createElement" in div,
                typeof div.all,
                "all" in div,
                typeof text.createElement,
                "all" in text
              ].join("|");
            })()
            "#,
        )
        .expect("non-document wrappers should not leak document surface");

    assert_eq!(result, "undefined|false|undefined|false|undefined|false");
}
#[test]
fn live_script_elements_preserve_htmlscriptelement_brand_across_access_paths() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head><script id=\"probe\"></script></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const byId = document.getElementById("probe");
              const byScripts = document.scripts[0];
              const byAll = document.all("probe");
              return [
                byId.toString(),
                Object.prototype.toString.call(byId),
                byScripts.toString(),
                Object.prototype.toString.call(byScripts),
                byAll.toString(),
                Object.prototype.toString.call(byAll),
                byId instanceof HTMLScriptElement,
                byScripts instanceof HTMLScriptElement,
                byAll instanceof HTMLScriptElement
              ].join("|");
            })()
            "#,
        )
        .expect("live script brand probe should evaluate");

    assert_eq!(
        result,
        "[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|[object HTMLScriptElement]|true|true|true"
    );
}
#[test]
fn live_dom_collections_report_browser_to_string_tags() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div></div><span></span></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => JSON.stringify({
              htmlCollectionTag: Object.prototype.toString.call(document.getElementsByTagName("div")),
              htmlCollectionCtor: document.getElementsByTagName("div").constructor?.name ?? null,
              queryNodeListTag: Object.prototype.toString.call(document.querySelectorAll("div,span")),
              queryNodeListCtor: document.querySelectorAll("div,span").constructor?.name ?? null,
              childNodeListTag: Object.prototype.toString.call(document.body.childNodes),
              childNodeListCtor: document.body.childNodes.constructor?.name ?? null
            }))()
            "#,
        )
        .expect("live collection brand probe should evaluate");

    assert_eq!(
        result,
        r#"{"htmlCollectionTag":"[object HTMLCollection]","htmlCollectionCtor":"HTMLCollection","queryNodeListTag":"[object NodeList]","queryNodeListCtor":"NodeList","childNodeListTag":"[object NodeList]","childNodeListCtor":"NodeList"}"#
    );
}
#[test]
fn live_element_matches_methods_live_on_element_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><div id=\"probe\" class=\"hit\"></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const el = document.getElementById("probe");
              let invalid = null;
              try {
                el.matches(":not(");
              } catch (error) {
                invalid = error.name;
              }
              return [
                el.matches("div.hit"),
                el.webkitMatchesSelector("body > div"),
                Object.prototype.hasOwnProperty.call(el, "matches"),
                Object.prototype.hasOwnProperty.call(el, "webkitMatchesSelector"),
                typeof Element.prototype.matches,
                typeof Element.prototype.webkitMatchesSelector,
                Element.prototype.matches.length,
                Element.prototype.webkitMatchesSelector.length,
                invalid
              ].join("|");
            })()
            "#,
        )
        .expect("Element.matches prototype probe should evaluate");

    assert_eq!(
        result,
        "true|true|false|false|function|function|1|1|SyntaxError"
    );
}
#[test]
fn legacy_platform_objects_use_webidl_property_descriptors_and_key_order() {
    let mut vm = new_storage_test_vm("https://legacy-platform-object.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = (object, key) => {
    const value = Object.getOwnPropertyDescriptor(object, key);
    return value && [value.writable, value.enumerable, value.configurable].join(',');
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const element = document.createElement('div');
  element.className = 'foo';
  const classList = element.classList;
  const classListDefine = throwsTypeError(() => {
    Object.defineProperty(classList, '1', { value: 'bar' });
  });

  const dataList = document.createElement('datalist');
  const namedOption = document.createElement('option');
  namedOption.id = 'named';
  dataList.append(namedOption);
  const collection = dataList.options;
  collection.expando = 1;

  const select = document.createElement('select');
  const option = document.createElement('option');
  Object.defineProperty(select, '0', {
    value: option,
    writable: false,
    enumerable: false,
    configurable: false
  });

  const dataset = element.dataset;
  Object.defineProperty(dataset, 'entry', {
    value: 'value',
    writable: false,
    enumerable: false,
    configurable: false
  });
  const datasetAccessor = throwsTypeError(() => {
    Object.defineProperty(dataset, 'other', { get() { return 'wrong'; } });
  });

  const attributeElement = document.createElement('div');
  const attributes = attributeElement.attributes;
  attributes.first = 1;
  const symbol = Symbol('marker');
  attributes[symbol] = 2;
  attributes.second = 3;
  attributeElement.setAttribute('id', 'target');
  attributeElement.setAttribute('title', 'title');

  return JSON.stringify({
    classList: descriptor(classList, '0'),
    classListDefine,
    collectionNamed: descriptor(collection, 'named'),
    collectionExpando: descriptor(collection, 'expando'),
    select: descriptor(select, '0'),
    selectKeys: Object.keys(select),
    dataset: descriptor(dataset, 'entry'),
    datasetAccessor,
    attributeKeys: Reflect.ownKeys(attributes).map(key =>
      typeof key === 'symbol' ? key.toString() : key)
  });
})()
"#,
        )
        .expect("legacy platform object property probe should evaluate");

    assert_eq!(
        result,
        r#"{"classList":"false,true,true","classListDefine":true,"collectionNamed":"false,false,true","collectionExpando":"true,true,true","select":"true,true,true","selectKeys":["0"],"dataset":"true,true,true","datasetAccessor":true,"attributeKeys":["0","1","id","title","first","second","Symbol(marker)"]}"#
    );
}
#[test]
fn access_key_label_matches_chromium_single_key_surface() {
    let mut vm = new_storage_test_vm("https://access-key-label.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    'accessKeyLabel'
  );
  const label = value => {
    const button = document.createElement('button');
    if (value !== null) button.setAttribute('accesskey', value);
    return button.accessKeyLabel;
  };
  const incompatible = (() => {
    try {
      descriptor.get.call(document.createElementNS('urn:test', 'button'));
      return 'none';
    } catch (error) {
      return error.name;
    }
  })();

  return JSON.stringify({
    descriptor: [descriptor.set, descriptor.enumerable, descriptor.configurable],
    valid: label('b'),
    missing: label(null),
    empty: label(''),
    multiple: label('s 0'),
    nonBmp: label('\u{1F600}'),
    incompatible
  });
})()
"#,
        )
        .expect("accessKeyLabel surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":[null,true,true],"valid":"Alt+b","missing":"","empty":"","multiple":"","nonBmp":"","incompatible":"TypeError"}"#
    );
}
#[test]
fn named_node_map_uses_receiver_host_when_global_bridge_is_missing() {
    assert_named_node_map_uses_receiver_host("delete globalThis.__moliNativeBridge");
}

#[test]
fn named_node_map_uses_receiver_host_when_global_bridge_is_replaced() {
    assert_named_node_map_uses_receiver_host("globalThis.__moliNativeBridge = {}; true");
}

#[test]
fn named_node_map_uses_receiver_host_without_reading_global_bridge_accessor() {
    assert_named_node_map_uses_receiver_host(
        r#"Object.defineProperty(globalThis, '__moliNativeBridge', {
          configurable: true,
          get() { throw new Error('the public bridge accessor must not run'); }
        }); true"#,
    );
}

fn assert_named_node_map_uses_receiver_host(bridge_change: &str) {
    let mut vm = new_storage_test_vm("https://named-node-map-receiver-host.test/");
    vm.eval(
        r#"
globalThis.liveElement = document.createElement('div');
liveElement.setAttribute('data-real', 'one');
globalThis.detachedElement = new DOMParser()
  .parseFromString('<div data-real="one"></div>', 'text/html')
  .querySelector('div');
"#,
    )
    .expect("live and detached elements should be created before the bridge changes");
    assert_eq!(
        vm.eval(bridge_change)
            .expect("the public bridge should be removable or replaceable"),
        "true"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = element => {
    // The first access must construct a wrapper without the public bridge.
    const attributes = element.attributes;
    const attribute = attributes.item(0);
    // Mutate through Attr so detached Element's JS bridge shim is not involved.
    attribute.value = 'two';
    return {
      tag: Object.prototype.toString.call(attributes),
      prototype: Object.getPrototypeOf(attributes) === NamedNodeMap.prototype,
      sameWrapper: attributes === element.attributes,
      sameAttr: attribute === attributes.item(0),
      length: attributes.length,
      indexedValue: attributes[0].value,
      namedValue: attributes.getNamedItem('data-real').value,
      ownerElement: attribute.ownerElement === element
    };
  };
  return JSON.stringify([probe(liveElement), probe(detachedElement)]);
})()
"#,
        )
        .expect("NamedNodeMap should use the validated receiver host without the public bridge");
    let expected = serde_json::json!({
        "tag": "[object NamedNodeMap]",
        "prototype": true,
        "sameWrapper": true,
        "sameAttr": true,
        "length": 1,
        "indexedValue": "two",
        "namedValue": "two",
        "ownerElement": true
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).expect("probe result should be JSON"),
        serde_json::json!([expected.clone(), expected])
    );
}

#[test]
fn named_node_map_accessor_expandos_remain_writable_through_setters() {
    let mut vm = new_storage_test_vm("https://named-node-map-accessor-expando.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const attributes = document.createElement('div').attributes;
  const symbol = Symbol('accessor');
  const writes = [];
  let stringValue = 'string-initial';
  let symbolValue = 'symbol-initial';

  Object.defineProperty(attributes, 'accessor', {
    get() { return stringValue; },
    set(value) {
      stringValue = value;
      writes.push(`string:${value}`);
    },
    enumerable: true,
    configurable: true
  });
  Object.defineProperty(attributes, symbol, {
    get() { return symbolValue; },
    set(value) {
      symbolValue = value;
      writes.push(`symbol:${value}`);
    },
    enumerable: false,
    configurable: true
  });

  attributes.accessor = 'sloppy';
  attributes[symbol] = 'sloppy';
  let stringStrictError = null;
  let symbolStrictError = null;
  try {
    (() => {
      'use strict';
      attributes.accessor = 'strict';
    })();
  } catch (error) {
    stringStrictError = error.name;
  }
  try {
    (() => {
      'use strict';
      attributes[symbol] = 'strict';
    })();
  } catch (error) {
    symbolStrictError = error.name;
  }

  const stringDescriptor = Object.getOwnPropertyDescriptor(attributes, 'accessor');
  const symbolDescriptor = Object.getOwnPropertyDescriptor(attributes, symbol);
  return JSON.stringify({
    stringValue: attributes.accessor,
    symbolValue: attributes[symbol],
    writes,
    stringStrictError,
    symbolStrictError,
    stringDescriptor: [
      typeof stringDescriptor.get,
      typeof stringDescriptor.set,
      stringDescriptor.enumerable,
      stringDescriptor.configurable,
      'writable' in stringDescriptor
    ],
    symbolDescriptor: [
      typeof symbolDescriptor.get,
      typeof symbolDescriptor.set,
      symbolDescriptor.enumerable,
      symbolDescriptor.configurable,
      'writable' in symbolDescriptor
    ]
  });
})()
"#,
        )
        .expect("NamedNodeMap accessor expando probe should evaluate");

    assert_eq!(
        result,
        r#"{"stringValue":"strict","symbolValue":"strict","writes":["string:sloppy","symbol:sloppy","string:strict","symbol:strict"],"stringStrictError":null,"symbolStrictError":null,"stringDescriptor":["function","function",true,true,false],"symbolDescriptor":["function","function",false,true,false]}"#
    );
}
#[test]
fn live_named_node_map_cache_uses_private_slot_and_ignores_public_spoofing() {
    let mut vm = new_storage_test_vm("https://named-node-map-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const element = document.createElement('div');
  body.appendChild(element);
  element.setAttribute('data-real', 'one');
  element.setAttribute('title', 'initial');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliNamedNodeMap'))
    .sort()
    .join(',');
  const attrs = element.attributes;
  const afterCacheNames = internalNames(element);
  const mapOwnNames = internalNames(attrs);
  const fakeMap = {
    length: 99,
    getNamedItem() {
      return { name: 'data-real', value: 'fake' };
    }
  };
  Element.prototype.__moliNamedNodeMapCache = fakeMap;
  element.__moliNamedNodeMapCache = fakeMap;
  const spoofedOwnNames = internalNames(element);
  const afterSpoof = element.attributes;
  element.setAttribute('data-real', 'two');
  return JSON.stringify({
    afterCacheNames,
    mapOwnNames,
    spoofedOwnNames,
    sameWrapper: afterSpoof === attrs,
    length: afterSpoof.length,
    namedValue: afterSpoof.getNamedItem('data-real').value,
    indexedName: afterSpoof[0].name,
    indexedValue: afterSpoof[0].value
  });
})()
"#,
        )
        .expect("live NamedNodeMap cache should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"afterCacheNames":"","mapOwnNames":"","spoofedOwnNames":"__moliNamedNodeMapCache","sameWrapper":true,"length":2,"namedValue":"two","indexedName":"data-real","indexedValue":"two"}"#
    );
}
#[test]
fn basefont_uses_html_element_interface_across_creation_paths() {
    let mut vm = new_storage_test_vm("https://basefont-interface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const namespace = "http://www.w3.org/1999/xhtml";
  const elements = [
    document.createElementNS(namespace, "basefont"),
    new DOMParser().parseFromString("<basefont>", "text/html").querySelector("basefont"),
    document.createElement("BASEFONT")
  ];
  return elements.map(element => [
    Object.prototype.toString.call(element),
    element instanceof HTMLElement,
    element instanceof HTMLUnknownElement
  ].join(":")).join("|");
})()
"#,
        )
        .expect("basefont interface probe should evaluate");

    assert_eq!(
        result,
        "[object HTMLElement]:true:false|[object HTMLElement]:true:false|[object HTMLElement]:true:false"
    );
}
#[test]
fn tag_name_live_collection_freezes_document_htmlness() {
    let mut vm = new_storage_test_vm("https://tag-name-htmlness.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const XHTML_NS = "http://www.w3.org/1999/xhtml";
  const parent = document.createElement("div");
  const child1 = document.createElementNS(XHTML_NS, "a");
  child1.textContent = "xhtml:a";
  const child2 = document.createElementNS(XHTML_NS, "A");
  child2.textContent = "xhtml:A";
  const child3 = document.createElementNS("", "a");
  child3.textContent = "a";
  const child4 = document.createElementNS("", "A");
  child4.textContent = "A";
  parent.append(child1, child2, child3, child4);

  const before = parent.getElementsByTagName("A");
  const xml = document.implementation.createDocument(null, "root");
  xml.documentElement.appendChild(parent);
  const after = parent.getElementsByTagName("A");
  parent.append(child1, child2, child3, child4);

  const text = list => Array.from(list).map(node => node.textContent).join(",");
  return [
    text(before),
    text(after),
    before === after,
    text(parent.getElementsByTagName("A"))
  ].join("|");
})()
"#,
        )
        .expect("tag name HTMLness probe should evaluate");

    assert_eq!(result, "xhtml:a,A|xhtml:A,A|false|xhtml:A,A");
}
#[test]
fn tag_name_live_collections_track_create_clone_import_and_adopt_identities() {
    let mut vm = new_storage_test_vm("https://tag-name-query-identities.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const XHTML_NS = "http://www.w3.org/1999/xhtml";
  const byQualifiedName = document.getElementsByTagName("P:QUERY-NODE");
  const byNamespaceLocalName = document.getElementsByTagNameNS(XHTML_NS, "query-node");
  const container = document.createElement("main");
  document.appendChild(container);

  const created = document.createElementNS(XHTML_NS, "p:query-node");
  created.id = "created";
  container.appendChild(created);

  const cloned = created.cloneNode(false);
  cloned.id = "cloned";
  container.appendChild(cloned);

  const xml = document.implementation.createDocument(XHTML_NS, "x:query-node", null);
  const imported = document.importNode(xml.documentElement, false);
  imported.id = "imported";
  container.appendChild(imported);

  const other = document.implementation.createHTMLDocument("");
  const adopted = other.createElementNS(XHTML_NS, "p:query-node");
  adopted.id = "adopted";
  other.body.appendChild(adopted);
  document.adoptNode(adopted);
  container.appendChild(adopted);

  const ids = collection => Array.from(collection).map(node => node.id).join(",");
  return [
    ids(byQualifiedName),
    ids(byNamespaceLocalName),
    document.getElementsByTagName("x:query-node")[0] === imported,
    document.getElementsByTagNameNS(XHTML_NS, "QUERY-NODE").length,
    container.getElementsByTagNameNS(XHTML_NS, "query-node").length
  ].join("|");
})()
"#,
        )
        .expect("tag-name identity lifecycle probe should evaluate");

    assert_eq!(
        result,
        "created,cloned,adopted|created,cloned,imported,adopted|true|0|4"
    );
}
#[test]
fn attr_to_string_method_matches_declared_surface() {
    let mut vm = new_storage_test_vm("https://attr-to-string-declared-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const attr = document.createAttribute("data-value");
  attr.value = "one";
  const descriptor = Object.getOwnPropertyDescriptor(Attr.prototype, "toString");
  return JSON.stringify({
    descriptor: [
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable,
      descriptor.writable
    ],
    enumerableKeys: Object.keys(Attr.prototype).includes("toString"),
    ownNamesInclude: Object.getOwnPropertyNames(Attr.prototype).includes("toString"),
    attrString: attr.toString(),
    objectString: Object.prototype.toString.call(attr),
    fakeReceiverString: Attr.prototype.toString.call({})
  });
})()
"#,
        )
        .expect("Attr.prototype.toString descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":["function","toString",0,false,true,true],"enumerableKeys":false,"ownNamesInclude":true,"attrString":"[object Attr]","objectString":"[object Attr]","fakeReceiverString":"[object Attr]"}"#
    );
}
#[test]
fn dom_mixin_members_have_webidl_unscopables() {
    let mut vm = new_storage_test_vm("https://dom-unscopables.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const expected = new Map([
    [CharacterData, ["after", "before", "remove", "replaceWith"]],
    [Document, ["append", "fullscreen", "prepend", "replaceChildren"]],
    [DocumentFragment, ["append", "prepend", "replaceChildren"]],
    [DocumentType, ["after", "before", "remove", "replaceWith"]],
    [Element, ["after", "append", "before", "prepend", "remove", "replaceChildren", "replaceWith", "slot"]]
  ]);

  for (const [constructor, names] of expected) {
    const prototype = constructor.prototype;
    const descriptor = Object.getOwnPropertyDescriptor(prototype, Symbol.unscopables);
    assert(!!descriptor, `${constructor.name} @@unscopables descriptor`);
    assert(descriptor.value === prototype[Symbol.unscopables], `${constructor.name} value`);
    assert(descriptor.writable === false, `${constructor.name} writable`);
    assert(descriptor.enumerable === false, `${constructor.name} enumerable`);
    assert(descriptor.configurable === true, `${constructor.name} configurable`);
    const unscopables = descriptor.value;
    assert(Object.getPrototypeOf(unscopables) === null, `${constructor.name} null prototype`);
    assert(Object.getOwnPropertySymbols(unscopables).length === 0, `${constructor.name} symbol keys`);
    assert(
      Object.getOwnPropertyNames(unscopables).sort().join("|") === [...names].sort().join("|"),
      `${constructor.name} exact names`
    );
    for (const name of names) {
      const property = Object.getOwnPropertyDescriptor(unscopables, name);
      assert(property.value === true, `${constructor.name}.${name} value`);
      assert(property.writable === true, `${constructor.name}.${name} writable`);
      assert(property.enumerable === true, `${constructor.name}.${name} enumerable`);
      assert(property.configurable === true, `${constructor.name}.${name} configurable`);
    }
  }

  window.prepend = "global-prepend";
  window.append = "global-append";
  const element = document.createElement("div");
  element.setAttribute(
    "onclick",
    "globalThis.__domUnscopableResolution = [prepend, append];"
  );
  element.dispatchEvent(new Event("click"));
  assert(
    globalThis.__domUnscopableResolution.join("|") === "global-prepend|global-append",
    "Document ParentNode methods must not shadow globals after Element scope"
  );
  delete globalThis.__domUnscopableResolution;
  delete window.prepend;
  delete window.append;
  return "ok";
})()
"#,
        )
        .expect("DOM unscopables probe should evaluate");

    assert_eq!(result, "ok");
}
#[test]
fn node_mixin_methods_install_unscopables_on_each_interface() {
    let mut vm = new_storage_test_vm("https://node-mixin-unscopables.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parentNodeNames = ["prepend", "append", "replaceChildren"];
  const childNodeNames = ["before", "after", "replaceWith", "remove"];
  const hasUnscopables = (prototype, names) => {
    const unscopables = prototype[Symbol.unscopables];
    return Object.getPrototypeOf(unscopables) === null &&
      names.every(name => unscopables[name] === true);
  };
  return [
    hasUnscopables(Document.prototype, parentNodeNames),
    hasUnscopables(DocumentFragment.prototype, parentNodeNames),
    hasUnscopables(Element.prototype, [...parentNodeNames, ...childNodeNames]),
    hasUnscopables(DocumentType.prototype, childNodeNames),
    hasUnscopables(CharacterData.prototype, childNodeNames)
  ].join("|");
})()
"#,
        )
        .expect("node mixin unscopables probe should evaluate");

    assert_eq!(result, "true|true|true|true|true");
}
#[test]
fn named_node_map_lookup_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://namednodemap-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.value ? value.value : String(value);
    } catch (error) {
      return 'throw:' + error.name;
    }
  };
  const el = document.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  el.setAttribute("data-x", "x");
  el.setAttributeNS("urn:moli:test", "lm:flag", "on");
  const attrs = el.attributes;
  const detached = new DOMParser().parseFromString(
    '<html><body><div data-y="y" lm:flag="detached"></div></body></html>',
    'text/html'
  ).querySelector('div').attributes;
  return [
    attrs.getNamedItem({ toString() { return "data-x"; } }).value,
    attrs.getNamedItemNS(
      { toString() { return "urn:moli:test"; } },
      { toString() { return "flag"; } }
    ).value,
    attrs.removeNamedItem({ toString() { return "data-x"; } }).value,
    attrs.getNamedItem("data-x") === null,
    probe(() => attrs.getNamedItem()),
    probe(() => attrs.getNamedItem(Symbol("name"))),
    probe(() => attrs.getNamedItemNS("urn:moli:test")),
    probe(() => attrs.getNamedItemNS(Symbol("namespace"), "flag")),
    probe(() => attrs.removeNamedItem()),
    probe(() => attrs.removeNamedItemNS("urn:moli:test")),
    detached.getNamedItem({ toString() { return "data-y"; } }).value,
    detached.removeNamedItem({ toString() { return "data-y"; } }).value,
    probe(() => detached.getNamedItem()),
    probe(() => detached.getNamedItemNS(null)),
    probe(() => detached.removeNamedItem(Symbol("name")))
  ].join("|");
})()
"#,
        )
        .expect("NamedNodeMap WebIDL lookup argument probe should evaluate");

    assert_eq!(
        result,
        "x|on|x|true|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|y|y|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn dom_implementation_factory_arguments_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://domimplementation-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const impl = document.implementation;
              const probe = callback => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };
              const htmlNull = impl.createHTMLDocument(null);
              const xmlUndefinedNamespace = impl.createDocument(undefined, "root", undefined);
              const xmlUndefinedQualifiedName = impl.createDocument(undefined, undefined, undefined);
              const xmlNullQualifiedName = impl.createDocument(null, null, null);
              const xmlDoc = impl.createDocument(null, "root", null);
              const htmlDoc = impl.createHTMLDocument("");
              const relaxedNameDoc = impl.createDocument(null, "f}oo", null);
              const multiColonDoc = impl.createDocument("http://example.com/", "f:o:o", null);
              const numericPrefixDoc = impl.createDocument("http://example.com/", "0:a", null);
              const originalDefineProperty = Object.defineProperty;
              Object.defineProperty = () => {
                throw new Error("page-tampered defineProperty should not run");
              };
              const definePropertyTamperDoc = impl.createDocument(null, "tamper", null);
              Object.defineProperty = originalDefineProperty;
              return [
                probe(() => impl.createDocumentType()),
                probe(() => impl.createDocumentType("html", Symbol("public"), "")),
                probe(() => impl.createDocumentType("html", {
                  toString() {
                    throw new RangeError("public");
                  }
                }, "")),
                impl.createDocumentType("test:root", "", "").name,
                impl.createDocumentType("", "", "").name,
                impl.createDocumentType("1bad", "", "").name,
                impl.createDocumentType("a:b:c", "", "").name,
                probe(() => impl.createDocumentType("bad name", "", "")),
                probe(() => impl.createDocumentType("bad>", "", "")),
                probe(() => impl.createHTMLDocument(Symbol("title"))),
                probe(() => impl.createHTMLDocument({
                  toString() {
                    throw new RangeError("title");
                  }
                })),
                htmlNull.title,
                probe(() => impl.createDocument()),
                probe(() => impl.createDocument(Symbol("namespace"), "root", null)),
                probe(() => impl.createDocument(null, Symbol("qualifiedName"), null)),
                probe(() => impl.createDocument(null, {
                  toString() {
                    throw new RangeError("qualifiedName");
                  }
                }, null)),
                probe(() => impl.createDocument(null, "root", Symbol("doctype"))),
                xmlUndefinedNamespace.documentElement.localName,
                String(xmlUndefinedNamespace.documentElement.namespaceURI),
                xmlUndefinedQualifiedName.documentElement.localName,
                String(xmlNullQualifiedName.documentElement),
                Object.getPrototypeOf(xmlDoc) === XMLDocument.prototype,
                xmlDoc instanceof XMLDocument,
                typeof xmlDoc.createElement,
                typeof xmlDoc.appendChild,
                xmlDoc.createElement("child").ownerDocument === xmlDoc,
                Object.getPrototypeOf(htmlDoc) === HTMLDocument.prototype,
                htmlDoc instanceof HTMLDocument,
                relaxedNameDoc.documentElement.localName,
                multiColonDoc.documentElement.prefix,
                multiColonDoc.documentElement.localName,
                numericPrefixDoc.documentElement.prefix,
                numericPrefixDoc.documentElement.localName,
                typeof definePropertyTamperDoc.createElement,
                typeof definePropertyTamperDoc.appendChild,
                definePropertyTamperDoc.createElement("child").ownerDocument === definePropertyTamperDoc,
                probe(() => impl.createDocument(null, ":foo", null)),
                probe(() => impl.createDocument(null, "f:o:o", null)),
                probe(() => impl.createDocument("http://example.com/", "a:0", null))
              ].join("|");
            })()
            "#,
        )
        .expect("DOMImplementation WebIDL argument probe should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|RangeError|test:root||1bad|a:b:c|InvalidCharacterError|InvalidCharacterError|TypeError|RangeError|null|TypeError|TypeError|TypeError|RangeError|TypeError|root|null|undefined|null|true|true|function|function|true|true|true|f}oo|f|o:o|0|a|function|function|true|InvalidCharacterError|NamespaceError|InvalidCharacterError"
    );
}
#[test]
fn dom_implementation_prototype_methods_are_declared_with_expected_descriptors() {
    let mut vm = new_storage_test_vm("https://domimplementation-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const names = [
                "hasFeature",
                "createDocumentType",
                "createHTMLDocument",
                "createDocument"
              ];
              const shape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  DOMImplementation.prototype,
                  name
                );
                return [
                  typeof descriptor.value,
                  descriptor.value.name,
                  descriptor.value.length,
                  descriptor.enumerable,
                  descriptor.writable,
                  descriptor.configurable
                ].join(",");
              };
              const implementation = document.implementation;
              return [
                Object.getPrototypeOf(implementation) === DOMImplementation.prototype,
                Object.prototype.hasOwnProperty.call(implementation, "hasFeature"),
                Object.keys(DOMImplementation.prototype)
                  .filter(name => names.includes(name))
                  .join(","),
                names.map(shape).join("|"),
                implementation.hasFeature("unused", "unused"),
                implementation.createDocumentType("html", "", "").name,
                implementation.createHTMLDocument("Title").title,
                implementation.createDocument(null, "root", null).documentElement.localName
              ].join(";");
            })()
            "#,
        )
        .expect("DOMImplementation prototype method descriptor probe should evaluate");

    assert_eq!(
        result,
        "true;false;hasFeature,createDocumentType,createHTMLDocument,createDocument;function,hasFeature,0,true,true,true|function,createDocumentType,3,true,true,true|function,createHTMLDocument,0,true,true,true|function,createDocument,2,true,true,true;true;html;Title;root"
    );
}
#[test]
fn document_implementation_accessor_is_declared_without_public_spoofing() {
    let mut vm = new_storage_test_vm("https://document-implementation-accessor.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(
                Document.prototype,
                "implementation"
              );
              const descriptorOwner = (object, name) => {
                for (let current = object; current; current = Object.getPrototypeOf(current)) {
                  if (Object.prototype.hasOwnProperty.call(current, name)) {
                    return current;
                  }
                }
                return null;
              };
              const descriptorOnChain = (object, name) =>
                Object.getOwnPropertyDescriptor(descriptorOwner(object, name), name);
              const liveImplementation = document.implementation;
              const detached = liveImplementation.createHTMLDocument("");
              const detachedImplementation = detached.implementation;
              const detachedDescriptor = Object.getOwnPropertyDescriptor(
                detached,
                "implementation"
              );
              const detachedPrototypeDescriptor = descriptorOnChain(
                Object.getPrototypeOf(detached),
                "implementation"
              );

              document.implementation = { marker: "document" };
              detached.implementation = { marker: "detached" };

              return [
                typeof descriptor.get,
                descriptor.get.name,
                descriptor.set === undefined,
                descriptor.enumerable,
                descriptor.configurable,
                Object.keys(Document.prototype).includes("implementation"),
                Object.prototype.hasOwnProperty.call(document, "implementation"),
                Object.prototype.hasOwnProperty.call(detached, "implementation"),
                detachedDescriptor === undefined,
                descriptorOwner(Object.getPrototypeOf(detached), "implementation") === Document.prototype,
                typeof detachedPrototypeDescriptor.get,
                detachedPrototypeDescriptor.get.name,
                detachedPrototypeDescriptor.set === undefined,
                detachedPrototypeDescriptor.enumerable,
                detachedPrototypeDescriptor.configurable,
                document.implementation === liveImplementation,
                detached.implementation === detachedImplementation,
                detachedImplementation !== liveImplementation,
                liveImplementation.createDocumentType("html", "", "").ownerDocument === document,
                detachedImplementation.createDocumentType("html", "", "").ownerDocument === detached
              ].join("|");
            })()
            "#,
        )
        .expect("Document implementation accessor descriptor probe should evaluate");

    assert_eq!(
        result,
        "function|get implementation|true|true|true|true|false|false|true|true|function|get implementation|true|true|true|true|true|true|true|true"
    );
}
#[test]
fn dom_implementation_create_document_preserves_xml_metadata_and_doctype_owner() {
    let mut vm = new_parsed_test_vm(
        "https://domimplementation-create-document.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const impl = document.implementation;
              const xhtml = impl.createDocument("http://www.w3.org/1999/xhtml", "", null);
              const svg = impl.createDocument("http://www.w3.org/2000/svg", "", null);
              const sourceDoctype = document.doctype;
              const doc = impl.createDocument(null, null, sourceDoctype);
              return [
                xhtml.contentType,
                svg.contentType,
                doc.documentElement === null,
                doc.childNodes.length,
                doc.doctype === sourceDoctype,
                doc.doctype.ownerDocument === doc
              ].join("|");
            })()
            "#,
        )
        .expect("createDocument should preserve metadata and adopt doctype owner");

    assert_eq!(
        result,
        "application/xhtml+xml|image/svg+xml|true|1|true|true"
    );
}
#[test]
fn dom_implementation_create_document_accepts_detached_doctype_after_prototype_tampering() {
    let mut vm = new_parsed_test_vm(
        "https://domimplementation-detached-doctype-brand.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const doctype = document.implementation.createDocumentType("qorflesnorf", "abcde", "x\"y");
              const ownerDocumentGetter = Object.getOwnPropertyDescriptor(Node.prototype, "ownerDocument").get;
              const wasInstance = doctype instanceof DocumentType;
              Object.setPrototypeOf(doctype, null);
              const isInstanceAfterTamper = doctype instanceof DocumentType;
              const doc = document.implementation.createDocument(null, null, doctype);
              return [
                wasInstance,
                isInstanceAfterTamper,
                doc.doctype === doctype,
                doc.childNodes.length,
                doc.doctype.ownerDocument === doc,
                ownerDocumentGetter.call(doc.doctype) === doc
              ].join("|");
            })()
            "#,
        )
        .expect("createDocument should accept detached DocumentType by internal state");

    assert_eq!(result, "true|false|true|1|false|true");
}
#[test]
fn dom_implementation_and_detached_iterator_internal_slots_ignore_public_spoofing() {
    let mut vm = new_storage_test_vm("https://domimplementation-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const singletonSlot = "__moliDOMImplementationSingleton";
              const ownerSlot = "__moliDOMImplementationOwnerDocument";
              const detachedCacheSlot = "__moliDetachedDOMImplementation";
              const iteratorNodesSlot = "__moliDetachedNodeIteratorNodes";
              const iteratorIndexSlot = "__moliDetachedNodeIteratorIndex";
              const descriptorOwner = (object, name) => {
                for (let current = object; current; current = Object.getPrototypeOf(current)) {
                  if (Object.prototype.hasOwnProperty.call(current, name)) {
                    return current;
                  }
                }
                return null;
              };
              const descriptorOnChain = (object, name) =>
                Object.getOwnPropertyDescriptor(descriptorOwner(object, name), name);

              const liveImpl = document.implementation;
              const detached = liveImpl.createHTMLDocument("");
              const detachedImpl = detached.implementation;
              const fakeDoc = liveImpl.createHTMLDocument("fake");

              const singletonOwnBefore =
                Object.getOwnPropertyNames(window).includes(singletonSlot);
              const liveOwnBefore = Object.getOwnPropertyNames(liveImpl).includes(ownerSlot);
              const detachedImplOwnBefore =
                Object.getOwnPropertyNames(detachedImpl).includes(ownerSlot);
              const detachedDocOwnBefore =
                Object.getOwnPropertyNames(detached).includes(detachedCacheSlot);

              window[singletonSlot] = { marker: "public" };
              liveImpl[ownerSlot] = fakeDoc;
              detachedImpl[ownerSlot] = fakeDoc;
              const liveDoctype = liveImpl.createDocumentType("html", "", "");
              const detachedDoctype = detachedImpl.createDocumentType("html", "", "");

              detached[detachedCacheSlot] = { marker: "public" };

              const main = detached.createElement("main");
              const span = detached.createElement("span");
              main.appendChild(span);
              detached.body.appendChild(main);
              const iterator = detached.createNodeIterator(detached.body, 1);
              const nextDescriptor = descriptorOnChain(iterator, "nextNode");
              const tagDescriptor = descriptorOnChain(iterator, Symbol.toStringTag);
              const iteratorOwnBefore = Object.getOwnPropertyNames(iterator)
                .filter(name => name === iteratorNodesSlot || name === iteratorIndexSlot)
                .sort()
                .join(",");
              iterator[iteratorNodesSlot] = [];
              iterator[iteratorIndexSlot] = 99;
              const first = iterator.nextNode();
              const second = iterator.nextNode();

              return [
                singletonOwnBefore,
                liveOwnBefore,
                detachedImplOwnBefore,
                detachedDocOwnBefore,
                window[singletonSlot].marker,
                document.implementation === liveImpl,
                liveImpl[ownerSlot] === fakeDoc,
                detachedImpl[ownerSlot] === fakeDoc,
                detached[detachedCacheSlot].marker,
                liveDoctype.ownerDocument === document,
                detachedDoctype.ownerDocument === detached,
                iteratorOwnBefore,
                Object.prototype.toString.call(iterator),
                descriptorOwner(iterator, "nextNode") === NodeIterator.prototype,
                [
                  nextDescriptor.enumerable,
                  nextDescriptor.writable,
                  nextDescriptor.configurable,
                  nextDescriptor.value.name,
                  nextDescriptor.value.length
                ].join(","),
                [
                  tagDescriptor.enumerable,
                  tagDescriptor.writable,
                  tagDescriptor.configurable,
                  tagDescriptor.value
                ].join(","),
                iterator[iteratorIndexSlot],
                first && first.nodeName,
                second && second.nodeName
              ].join("|");
            })()
            "#,
        )
        .expect("internal slot reflection and spoofing probe should evaluate");

    assert_eq!(
        result,
        "false|false|false|false|public|true|true|true|public|true|true||[object NodeIterator]|true|true,true,true,nextNode,0|false,false,true,NodeIterator|99|BODY|MAIN"
    );
}
#[test]
fn detached_xml_document_clone_preserves_document_interface() {
    let mut vm = new_storage_test_vm("https://xml-document-clone-node.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const empty = document.implementation.createDocument("namespace", "");
  const emptyClone = empty.cloneNode(true);
  const xhtml = document.implementation.createDocument(
    "http://www.w3.org/1999/xhtml",
    "html",
    null
  );
  const xhtmlClone = xhtml.cloneNode(true);
  return [
    empty.constructor === XMLDocument,
    emptyClone.constructor === XMLDocument,
    Object.prototype.toString.call(emptyClone),
    emptyClone.documentElement === null,
    xhtml.constructor === XMLDocument,
    xhtml.documentElement.localName,
    xhtmlClone.constructor === XMLDocument,
    Object.prototype.toString.call(xhtmlClone),
    xhtmlClone.documentElement.localName,
    xhtmlClone.documentElement.namespaceURI
  ].join("|");
})()
"#,
        )
        .expect("XMLDocument cloneNode brand probe should evaluate");

    assert_eq!(
        result,
        "true|true|[object XMLDocument]|true|true|html|true|[object XMLDocument]|html|http://www.w3.org/1999/xhtml"
    );
}
#[test]
fn document_construction_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://document-construction-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const probe = callback => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };
              const detached = document.implementation.createHTMLDocument("");
              const parsed = new DOMParser().parseFromString(
                "<html><body></body></html>",
                "text/html"
              );
              const parsedXml = new DOMParser().parseFromString(
                "<root/>",
                "application/xml"
              );
              const svgElement = document.createElementNS(
                { toString() { return SVG_NS; } },
                { toString() { return "svg:g"; } }
              );
              const pi = document.createProcessingInstruction(
                "xml-stylesheet",
                "href='style.css'"
              );
              const attr = document.createAttribute("data-value");
              attr.value = null;
              const attrNull = attr.value;
              const attrSymbol = probe(() => { attr.value = Symbol("attr"); });
              attr.nodeValue = { toString() { return "node-value"; } };
              const attrNodeObject = attr.value;
              const attrThrowing = probe(() => {
                attr.textContent = { toString() { throw new Error("attr-text"); } };
              });
              return [
                document.createElement({ toString() { return "ARTICLE"; } }).localName,
                document.createElement(undefined).localName,
                probe(() => document.createElement()),
                probe(() => document.createElement(Symbol("name"))),
                probe(() => document.createElement("1bad")),
                svgElement.namespaceURI + ":" + svgElement.localName,
                probe(() => document.createElementNS()),
                probe(() => document.createElementNS(null)),
                probe(() => document.createElementNS(Symbol("namespace"), "x")),
                probe(() => document.createElementNS(null, "p:root")),
                probe(() => document.createElementNS("urn:not-xml", "xml:root")),
                document.createTextNode({ toString() { return "text"; } }).data,
                probe(() => document.createTextNode()),
                document.createComment(null).data,
                probe(() => document.createComment(Symbol("comment"))),
                pi.target + ":" + pi.data,
                probe(() => document.createProcessingInstruction("xml-stylesheet")),
                probe(() => document.createProcessingInstruction("1bad", Symbol("data"))),
                probe(() => document.createCDATASection()),
                probe(() => document.createCDATASection(Symbol("data"))),
                probe(() => document.createCDATASection("data")),
                document.createAttribute({ toString() { return "DATA-X"; } }).name,
                probe(() => document.createAttribute()),
                probe(() => document.createAttribute(Symbol("attr"))),
                attrNull,
                attrSymbol,
                attrNodeObject,
                attrThrowing,
                document.createAttributeNS(null, { toString() { return "data-z"; } }).localName,
                probe(() => document.createAttributeNS(null)),
                detached.createElement({ toString() { return "section"; } }).localName,
                probe(() => detached.createElement()),
                probe(() => detached.createElement("1bad")),
                detached.createElementNS(null, undefined).localName,
                probe(() => detached.createElementNS(null)),
                probe(() => detached.createElementNS(null, "p:root")),
                detached.createTextNode({ toString() { return "detached"; } }).data,
                probe(() => detached.createTextNode()),
                probe(() => detached.createCDATASection()),
                probe(() => detached.createCDATASection("data")),
                detached.createAttribute({ toString() { return "DATA-X"; } }).name,
                probe(() => detached.createAttribute("bad name")),
                parsed.createElement({ toString() { return "main"; } }).localName,
                probe(() => parsed.createElement()),
                parsed.createAttribute({ toString() { return "DATA-X"; } }).name,
                probe(() => parsed.createAttribute("bad name")),
                parsed.createTextNode({ toString() { return "parsed"; } }).data,
                probe(() => parsed.createTextNode()),
                parsed.createComment(null).data,
                probe(() => parsed.createComment()),
                probe(() => parsed.createElement("1bad")),
                parsed.createElementNS(SVG_NS, "svg:g").namespaceURI + ":" +
                  parsed.createElementNS(SVG_NS, "svg:g").localName,
                probe(() => parsed.createElementNS()),
                probe(() => parsed.createElementNS(null, "p:root")),
                parsed.createProcessingInstruction(
                  "xml-stylesheet",
                  "href='parsed.css'"
                ).target,
                probe(() => parsed.createProcessingInstruction("xml-stylesheet")),
                probe(() => parsed.createProcessingInstruction("1bad", "x")),
                parsedXml.createCDATASection("parsed-data").data,
                probe(() => parsedXml.createCDATASection()),
                probe(() => parsedXml.createCDATASection("bad ]]> data")),
                probe(() => parsed.createCDATASection("data")),
                probe(() => parsed.createCDATASection(Symbol("data")))
              ].join("|");
            })()
            "#,
        )
        .expect("Document construction WebIDL argument probe should evaluate");

    assert_eq!(
        result,
        "article|undefined|TypeError|TypeError|InvalidCharacterError|http://www.w3.org/2000/svg:g|TypeError|TypeError|TypeError|NamespaceError|NamespaceError|text|TypeError|null|TypeError|xml-stylesheet:href='style.css'|TypeError|TypeError|TypeError|TypeError|NotSupportedError|data-x|TypeError|TypeError|null|TypeError|node-value|Error|data-z|TypeError|section|TypeError|InvalidCharacterError|undefined|TypeError|NamespaceError|detached|TypeError|TypeError|NotSupportedError|data-x|InvalidCharacterError|main|TypeError|data-x|InvalidCharacterError|parsed|TypeError|null|TypeError|InvalidCharacterError|http://www.w3.org/2000/svg:g|TypeError|NamespaceError|xml-stylesheet|TypeError|InvalidCharacterError|parsed-data|TypeError|InvalidCharacterError|NotSupportedError|TypeError"
    );
}
#[test]
fn detached_xml_create_element_ns_uses_namespace_interface_despite_prefix() {
    let mut vm = new_storage_test_vm("https://detached-prefixed-interface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const XHTML_NS = "http://www.w3.org/1999/xhtml";
              const SVG_NS = "http://www.w3.org/2000/svg";
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const xml = document.implementation.createDocument("foo", null);
              const html = xml.createElementNS(XHTML_NS, "html:span");
              const svg = xml.createElementNS(SVG_NS, "svg:g");
              return [
                html instanceof HTMLElement,
                html instanceof HTMLSpanElement,
                Object.prototype.toString.call(html),
                html.localName,
                html.prefix,
                html.nodeName,
                probe(() => html.attachInternals()),
                svg instanceof SVGElement,
                svg instanceof SVGGElement,
                Object.prototype.toString.call(svg),
                svg.localName,
                svg.prefix,
                svg.nodeName
              ].join("|");
            })()
            "#,
        )
        .expect("detached XML createElementNS prefixed interface probe should evaluate");

    assert_eq!(
        result,
        "true|true|[object HTMLSpanElement]|span|html|html:span|NotSupportedError|true|true|[object SVGGElement]|g|svg|svg:g"
    );
}
#[test]
fn cssom_linkstyle_math_and_svg_element_surfaces_match_idlharness() {
    let mut vm = new_storage_test_vm("https://cssom-idl-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const MATH_NS = "http://www.w3.org/1998/Math/MathML";
              const svg = document.createElementNS(SVG_NS, "svg");
              const svgStyle = document.createElementNS(SVG_NS, "style");
              svgStyle.setAttribute("type", "text/css; charset=utf-8");
              svgStyle.textContent = "svg { color: green; }";
              svg.append(svgStyle);
              const body = document.body || document.appendChild(document.createElement("body"));
              body.append(svg);

              const math = document.createElementNS(MATH_NS, "math");
              const pi = document.createProcessingInstruction(
                "xml-stylesheet",
                "href='data:text/css,' type='text/css; charset=utf-8'"
              );

              return [
                typeof SVGStyleElement,
                svgStyle instanceof SVGStyleElement,
                "sheet" in SVGStyleElement.prototype,
                svgStyle.sheet instanceof CSSStyleSheet,
                typeof MathMLElement,
                math instanceof MathMLElement,
                math.style instanceof CSSStyleProperties,
                "sheet" in ProcessingInstruction.prototype,
                pi.sheet === null
              ].join("|");
            })()
            "#,
        )
        .expect("CSSOM idlharness surface probe should evaluate");

    assert_eq!(
        result,
        "function|true|true|false|function|true|true|true|true"
    );
}
#[test]
fn svg_script_element_uses_its_declared_surface_and_shared_async_state() {
    let mut vm = new_storage_test_vm("https://svg-script-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const script = document.createElementNS(SVG_NS, "script");
              const typeDescriptor = Object.getOwnPropertyDescriptor(
                SVGScriptElement.prototype,
                "type"
              );
              const asyncDescriptor = Object.getOwnPropertyDescriptor(
                SVGScriptElement.prototype,
                "async"
              );
              const initiallyAsync = script.async;
              script.async = false;
              const explicitlySynchronous = [script.async, script.hasAttribute("async")];
              script.async = true;
              script.type = "module";

              return JSON.stringify({
                constructorType: typeof SVGScriptElement,
                declaredPrototype: Object.getPrototypeOf(script) === SVGScriptElement.prototype,
                svgInheritance: script instanceof SVGElement,
                scriptBrand: script instanceof SVGScriptElement,
                tag: Object.prototype.toString.call(script),
                typeEnumerable: typeDescriptor.enumerable,
                asyncEnumerable: asyncDescriptor.enumerable,
                initiallyAsync,
                explicitlySynchronous,
                asyncAfterSet: script.async,
                asyncAttributeAfterSet: script.hasAttribute("async"),
                typeAfterSet: script.type,
                typeAttributeAfterSet: script.getAttribute("type")
              });
            })()
            "#,
        )
        .expect("SVGScriptElement surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructorType":"function","declaredPrototype":true,"svgInheritance":true,"scriptBrand":true,"tag":"[object SVGScriptElement]","typeEnumerable":true,"asyncEnumerable":true,"initiallyAsync":true,"explicitlySynchronous":[false,false],"asyncAfterSet":true,"asyncAttributeAfterSet":true,"typeAfterSet":"module","typeAttributeAfterSet":"module"}"#
    );
}
#[test]
fn svg_list_objects_keep_declared_brand_and_members() {
    let mut vm = new_storage_test_vm("https://svg-list-objects.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const text = document.createElementNS(SVG_NS, "text");
              text.setAttribute("x", "10 20");
              text.setAttribute("rotate", "15 30");
              const group = document.createElementNS(SVG_NS, "g");
              const svg = document.createElementNS(SVG_NS, "svg");
              const lengthList = text.x;
              const numberList = text.rotate;
              const transformList = group.transform;
              const transform = svg.createSVGTransform();
              transformList.baseVal.initialize(transform);

              return [
                lengthList instanceof SVGAnimatedLengthList,
                Object.prototype.toString.call(lengthList),
                lengthList.baseVal instanceof SVGLengthList,
                Object.prototype.toString.call(lengthList.baseVal),
                lengthList.baseVal.numberOfItems,
                lengthList.baseVal.getItem(0) instanceof SVGLength,
                lengthList.baseVal.getItem(0).value,
                lengthList.animVal instanceof SVGLengthList,
                numberList instanceof SVGAnimatedNumberList,
                Object.prototype.toString.call(numberList),
                numberList.baseVal instanceof SVGNumberList,
                Object.prototype.toString.call(numberList.baseVal),
                numberList.baseVal.numberOfItems,
                numberList.baseVal.getItem(0).value,
                transform instanceof SVGTransform,
                Object.prototype.toString.call(transform),
                transform.type,
                transform.angle,
                transform.matrix instanceof SVGMatrix,
                Object.prototype.toString.call(transform.matrix),
                transform.matrix.a,
                transform.matrix.d,
                transform.matrix.e,
                transform.matrix.f,
                transformList instanceof SVGAnimatedTransformList,
                Object.prototype.toString.call(transformList),
                transformList.baseVal instanceof SVGTransformList,
                Object.prototype.toString.call(transformList.baseVal),
                transformList.baseVal.numberOfItems,
                transformList.baseVal.getItem(0) instanceof SVGTransform
              ].join("|");
            })()
            "#,
        )
        .expect("SVG list object probe should evaluate");

    assert_eq!(
        result,
        "true|[object SVGAnimatedLengthList]|true|[object SVGLengthList]|2|true|10|true|true|[object SVGAnimatedNumberList]|true|[object SVGNumberList]|2|15|true|[object SVGTransform]|1|0|true|[object SVGMatrix]|1|1|0|0|true|[object SVGAnimatedTransformList]|true|[object SVGTransformList]|1|true"
    );
}
#[test]
fn svg_list_matrix_and_transform_declared_methods_keep_descriptors() {
    let mut vm = new_storage_test_vm("https://svg-method-descriptors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const text = document.createElementNS(SVG_NS, "text");
              text.setAttribute("x", "10 20");
              text.setAttribute("rotate", "15 30");
              const group = document.createElementNS(SVG_NS, "g");
              const svg = document.createElementNS(SVG_NS, "svg");
              const lengthList = text.x.baseVal;
              const numberList = text.rotate.baseVal;
              const transformList = group.transform.baseVal;
              const transform = svg.createSVGTransform();
              const matrix = transform.matrix;
              function methodDescriptor(object, name) {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  descriptor.enumerable,
                  descriptor.writable,
                  descriptor.configurable,
                  descriptor.value.name,
                  descriptor.value.length
                ].join(",");
              }
              return JSON.stringify({
                lengthGetItem: methodDescriptor(lengthList, "getItem"),
                lengthAppendItem: methodDescriptor(lengthList, "appendItem"),
                numberInsertItemBefore: methodDescriptor(numberList, "insertItemBefore"),
                transformListCreate: methodDescriptor(transformList, "createSVGTransformFromMatrix"),
                transformListConsolidate: methodDescriptor(transformList, "consolidate"),
                transformSetMatrix: methodDescriptor(transform, "setMatrix"),
                transformSetRotate: methodDescriptor(transform, "setRotate"),
                transformSetSkewX: methodDescriptor(transform, "setSkewX"),
                matrixScaleNonUniform: methodDescriptor(matrix, "scaleNonUniform"),
                matrixRotateFromVector: methodDescriptor(matrix, "rotateFromVector"),
                matrixFlipX: methodDescriptor(matrix, "flipX"),
                transformOwnMethods: Object.getOwnPropertyNames(transform)
                  .filter(name => ["setMatrix", "setRotate", "setScale", "setSkewX", "setSkewY", "setTranslate"].includes(name))
                  .sort(),
                matrixOwnMethods: Object.getOwnPropertyNames(matrix)
                  .filter(name => ["flipX", "flipY", "inverse", "multiply", "rotate", "rotateFromVector", "scale", "scaleNonUniform", "skewX", "skewY", "translate"].includes(name))
                  .sort()
              });
            })()
            "#,
        )
        .expect("SVG method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"lengthGetItem":"false,true,true,getItem,1","lengthAppendItem":"false,true,true,appendItem,1","numberInsertItemBefore":"false,true,true,insertItemBefore,2","transformListCreate":"false,true,true,createSVGTransformFromMatrix,0","transformListConsolidate":"false,true,true,consolidate,0","transformSetMatrix":"false,true,true,setMatrix,0","transformSetRotate":"false,true,true,setRotate,3","transformSetSkewX":"false,true,true,setSkewX,1","matrixScaleNonUniform":"false,true,true,scaleNonUniform,2","matrixRotateFromVector":"false,true,true,rotateFromVector,2","matrixFlipX":"false,true,true,flipX,0","transformOwnMethods":["setMatrix","setRotate","setScale","setSkewX","setSkewY","setTranslate"],"matrixOwnMethods":["flipX","flipY","inverse","multiply","rotate","rotateFromVector","scale","scaleNonUniform","skewX","skewY","translate"]}"#
    );
}
#[test]
fn dom_token_list_uses_array_iteration_methods() {
    let mut vm = new_storage_test_vm("https://dom-token-list-iterators.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const element = document.createElement("div");
              element.className = "c b a";
              const list = element.classList;
              const seen = [];
              list.forEach((value, index, receiver) => {
                seen.push(`${index}=${value}:${receiver === list}`);
              });
              return JSON.stringify({
                iteratorIdentity: list[Symbol.iterator] === Array.prototype[Symbol.iterator],
                keysIdentity: list.keys === Array.prototype.keys,
                valuesIdentity: list.values === Array.prototype.values,
                entriesIdentity: list.entries === Array.prototype.entries,
                forEachIdentity: list.forEach === Array.prototype.forEach,
                keys: Array.from(list.keys()).join(","),
                values: Array.from(list.values()).join(","),
                entries: Array.from(list.entries()).map(pair => pair.join("=")).join(","),
                seen: seen.join(",")
              });
            })()
            "#,
        )
        .expect("DOMTokenList Array iterator bindings should evaluate");

    assert_eq!(
        result,
        r#"{"iteratorIdentity":true,"keysIdentity":true,"valuesIdentity":true,"entriesIdentity":true,"forEachIdentity":true,"keys":"0,1,2","values":"c,b,a","entries":"0=c,1=b,2=a","seen":"0=c:true,1=b:true,2=a:true"}"#
    );
}
#[test]
fn indexed_webapi_iterators_ignore_public_array_tampering() {
    let mut vm = new_storage_test_vm("https://webapi-intrinsic-iterators.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const ArrayConstructor = Array;
              const prototype = ArrayConstructor.prototype;
              const originals = {
                entries: prototype.entries,
                keys: prototype.keys,
                values: prototype.values,
                forEach: prototype.forEach,
                iterator: prototype[Symbol.iterator]
              };
              const poisoned = function poisonedArrayMethod() {
                throw new Error("public Array.prototype method was observed");
              };
              const failures = [];
              const valueIterableInterfaces = [
                "HTMLCollection",
                "HTMLFormControlsCollection",
                "HTMLOptionsCollection",
                "RadioNodeList",
                "HTMLAllCollection",
                "HTMLFormElement",
                "HTMLSelectElement",
                "FileList",
                "NamedNodeMap",
                "CSSStyleDeclaration",
                "StyleSheetList",
                "CSSRuleList",
                "MediaList",
                "CSSKeyframesRule",
                "DataTransferItemList",
                "Plugin",
                "PluginArray",
                "MimeTypeArray",
                "TextTrackList",
                "TextTrackCueList",
                "TouchList"
              ];
              const checkDescriptor = (interfaceName, member, expected, enumerable) => {
                const constructor = globalThis[interfaceName];
                if (typeof constructor !== "function") {
                  failures.push(`${interfaceName}:constructor`);
                  return;
                }
                const descriptor = Object.getOwnPropertyDescriptor(
                  constructor.prototype,
                  member
                );
                const label = typeof member === "symbol" ? "@@iterator" : member;
                if (!descriptor) {
                  failures.push(`${interfaceName}.${label}:missing`);
                  return;
                }
                if (descriptor.value !== expected) {
                  failures.push(`${interfaceName}.${label}:identity`);
                }
                if (
                  descriptor.enumerable !== enumerable ||
                  descriptor.writable !== true ||
                  descriptor.configurable !== true
                ) {
                  failures.push(`${interfaceName}.${label}:descriptor`);
                }
              };

              prototype.entries = poisoned;
              prototype.keys = poisoned;
              prototype.values = poisoned;
              prototype.forEach = poisoned;
              prototype[Symbol.iterator] = poisoned;
              globalThis.Array = undefined;
              try {
                const iterableInterfaces = ["NodeList", "DOMTokenList"];
                for (let index = 0; index < iterableInterfaces.length; index += 1) {
                  const interfaceName = iterableInterfaces[index];
                  checkDescriptor(interfaceName, "entries", originals.entries, true);
                  checkDescriptor(interfaceName, "keys", originals.keys, true);
                  checkDescriptor(interfaceName, "values", originals.values, true);
                  checkDescriptor(interfaceName, "forEach", originals.forEach, true);
                  checkDescriptor(
                    interfaceName,
                    Symbol.iterator,
                    originals.values,
                    false
                  );
                }
                for (
                  let index = 0;
                  index < valueIterableInterfaces.length;
                  index += 1
                ) {
                  const interfaceName = valueIterableInterfaces[index];
                  checkDescriptor(
                    interfaceName,
                    Symbol.iterator,
                    originals.values,
                    false
                  );
                }
                if (Object.hasOwn(FileList.prototype, "values")) {
                  failures.push("FileList.values:unexpected");
                }
              } finally {
                globalThis.Array = ArrayConstructor;
                prototype.entries = originals.entries;
                prototype.keys = originals.keys;
                prototype.values = originals.values;
                prototype.forEach = originals.forEach;
                prototype[Symbol.iterator] = originals.iterator;
              }
              return failures.join("|");
            })()
            "#,
        )
        .expect("indexed WebAPI templates should use V8 Array primordials");

    assert_eq!(result, "");
}
#[test]
fn html_link_rel_list_exposes_supported_tokens() {
    let mut vm = new_storage_test_vm("https://link-rel-list.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const link = document.createElement("link");
              const list = link.relList;
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              link.rel = "preload stylesheet preload";
              const beforeAdd = {
                tag: Object.prototype.toString.call(list),
                stable: list === link.relList,
                supportsType: typeof list.supports,
                supportsPreload: list.supports("preload"),
                supportsModulepreload: list.supports("modulepreload"),
                supportsUnknown: list.supports("unknown"),
                supportsEmpty: list.supports(""),
                supportsMissing: probe(() => list.supports()),
                length: list.length,
                item0: list.item(0),
                item1: list.item(1),
                containsPreload: list.contains("preload"),
                value: list.value,
                stringValue: String(list)
              };
              list.add("prefetch");
              const afterAddRel = link.rel;
              link.relList = "preconnect";
              return JSON.stringify({
                beforeAdd,
                afterAddRel,
                afterSetterRel: link.rel,
                afterSetterLength: list.length,
                afterSetterContainsPreconnect: list.contains("preconnect")
              });
            })()
            "#,
        )
        .expect("HTMLLinkElement.relList should expose DOMTokenList supported-token behavior");

    assert_eq!(
        result,
        r#"{"beforeAdd":{"tag":"[object DOMTokenList]","stable":true,"supportsType":"function","supportsPreload":true,"supportsModulepreload":true,"supportsUnknown":false,"supportsEmpty":false,"supportsMissing":"throw:TypeError","length":2,"item0":"preload","item1":"stylesheet","containsPreload":true,"value":"preload stylesheet preload","stringValue":"preload stylesheet preload"},"afterAddRel":"preload stylesheet prefetch","afterSetterRel":"preconnect","afterSetterLength":1,"afterSetterContainsPreconnect":true}"#
    );
}
#[test]
fn html_link_as_does_not_trim_enumerated_keywords() {
    let mut vm = new_storage_test_vm("https://link-as.test/");
    let result = vm
        .eval(
            r#"(() => {
              for (const doc of [document, document.implementation.createHTMLDocument('')]) {
                const link = doc.createElement('link');
                for (const value of [' fetch', 'fetch ', '\tscript\n', '\u00a0style', 'image\u00a0']) {
                  link.as = value;
                  if (link.as !== '' || link.getAttribute('as') !== value)
                    throw new Error('invalid enumerated keyword: ' + JSON.stringify(value));
                }
                link.as = 'sCrIpT';
                if (link.as !== 'script' || link.getAttribute('as') !== 'sCrIpT')
                  throw new Error('ASCII case folding must preserve the content attribute');
              }
              return 'pass';
            })()"#,
        )
        .expect("link as keyword reflection should evaluate");
    assert_eq!(result, "pass");
}
#[test]
fn element_part_exposes_dom_token_list() {
    let mut vm = new_storage_test_vm("https://element-part-list.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const element = document.createElement("div");
              const list = element.part;
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const initial = {
                tag: Object.prototype.toString.call(list),
                stable: list === element.part,
                length: list.length,
                value: list.value,
                item0: list.item(0)
              };
              element.setAttribute("part", "alpha beta alpha");
              const afterAttribute = {
                length: list.length,
                item0: list.item(0),
                item1: list.item(1),
                containsAlpha: list.contains("alpha"),
                value: list.value
              };
              list.remove("alpha");
              const afterRemove = element.getAttribute("part");
              list.add("gamma");
              const afterAdd = element.getAttribute("part");
              element.part = "delta";
              const afterSetter = {
                attribute: element.getAttribute("part"),
                length: list.length,
                containsDelta: list.contains("delta")
              };
              const invalid = probe(() => list.add("bad token"));
              const supports = probe(() => list.supports("alpha"));
              return JSON.stringify({ initial, afterAttribute, afterRemove, afterAdd, afterSetter, invalid, supports });
            })()
            "#,
        )
        .expect("Element.part DOMTokenList should evaluate");

    assert_eq!(
        result,
        r#"{"initial":{"tag":"[object DOMTokenList]","stable":true,"length":0,"value":"","item0":null},"afterAttribute":{"length":2,"item0":"alpha","item1":"beta","containsAlpha":true,"value":"alpha beta alpha"},"afterRemove":"beta","afterAdd":"beta gamma","afterSetter":{"attribute":"delta","length":1,"containsDelta":true},"invalid":"throw:InvalidCharacterError","supports":"throw:TypeError"}"#
    );
}
#[test]
fn dom_token_list_value_setters_apply_webidl_domstring_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const live = document.createElement("div");
              live.classList.value = null;
              const liveNull = live.className;
              live.classList.value = undefined;
              const liveUndefined = live.className;
              live.classList.value = { toString() { return "alpha beta"; } };
              const liveObject = live.className;
              const liveSymbol = probe(() => { live.classList.value = Symbol("class"); });
              const liveAfterSymbol = live.className;
              const liveThrow = probe(() => {
                live.classList.value = { toString() { throw new RangeError("boom"); } };
              });
              const liveAfterThrow = live.className;

              const detached = document.implementation.createHTMLDocument("").body;
              detached.classList.value = null;
              const detachedNull = detached.className;
              detached.classList.value = undefined;
              const detachedUndefined = detached.className;
              detached.classList.value = { toString() { return "detached value"; } };
              const detachedObject = detached.className;
              const detachedSymbol = probe(() => { detached.classList.value = Symbol("class"); });
              const detachedAfterSymbol = detached.className;

              live.classList = null;
              const elementNull = live.className;
              live.classList = undefined;
              const elementUndefined = live.className;
              live.classList = { toString() { return "element setter"; } };
              const elementObject = live.className;
              const elementSymbol = probe(() => { live.classList = Symbol("class"); });
              const elementAfterSymbol = live.className;

              return JSON.stringify({
                liveNull,
                liveUndefined,
                liveObject,
                liveSymbol,
                liveAfterSymbol,
                liveThrow,
                liveAfterThrow,
                detachedNull,
                detachedUndefined,
                detachedObject,
                detachedSymbol,
                detachedAfterSymbol,
                elementNull,
                elementUndefined,
                elementObject,
                elementSymbol,
                elementAfterSymbol
              });
            })()
            "#,
        )
        .expect("DOMTokenList value setters should apply WebIDL DOMString conversion");

    assert_eq!(
        result,
        r#"{"liveNull":"null","liveUndefined":"undefined","liveObject":"alpha beta","liveSymbol":"TypeError","liveAfterSymbol":"alpha beta","liveThrow":"RangeError","liveAfterThrow":"alpha beta","detachedNull":"null","detachedUndefined":"undefined","detachedObject":"detached value","detachedSymbol":"TypeError","detachedAfterSymbol":"detached value","elementNull":"null","elementUndefined":"undefined","elementObject":"element setter","elementSymbol":"TypeError","elementAfterSymbol":"element setter"}"#
    );
}
#[test]
fn detached_plain_document_all_matches_chromium_htmldda_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = new Document();
              const allDescriptor = Object.getOwnPropertyDescriptor(Document.prototype, "all");
              return JSON.stringify({
                ownAll: Object.prototype.hasOwnProperty.call(doc, "all"),
                protoGetter: typeof allDescriptor?.get,
                allType: typeof doc.all,
                loose: doc.all == undefined,
                strict: doc.all === undefined,
                bool: !!doc.all,
                string: String(doc.all),
                tag: Object.prototype.toString.call(doc.all),
                length: doc.all.length,
                noArgNull: doc.all() === null,
                itemZeroNull: doc.all(0) === null,
                itemMissNull: doc.all(999) === null,
                itemMethodNull: doc.all.item(0) === null,
                namedMethodNull: doc.all.namedItem("missing") === null
              });
            })()
            "#,
        )
        .expect("plain detached Document probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownAll":false,"protoGetter":"function","allType":"undefined","loose":true,"strict":false,"bool":false,"string":"[object HTMLAllCollection]","tag":"[object HTMLAllCollection]","length":0,"noArgNull":true,"itemZeroNull":true,"itemMissNull":true,"itemMethodNull":true,"namedMethodNull":true}"#
    );
}

#[test]
fn svg_value_lists_enforce_item_types_indices_and_read_only_anim_values() {
    let mut vm = new_storage_test_vm("https://svg-value-list-semantics.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const errorName = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error.name;
                }
              };
              const SVG_NS = "http://www.w3.org/2000/svg";
              const text = document.createElementNS(SVG_NS, "text");
              const svg = document.createElementNS(SVG_NS, "svg");
              text.setAttribute("x", "10 20");
              text.setAttribute("rotate", "15 30");

              const lengths = text.x;
              const lengthBase = lengths.baseVal;
              const lengthAnim = lengths.animVal;
              const numbers = text.rotate;
              const numberBase = numbers.baseVal;
              const numberAnim = numbers.animVal;
              const invalidItems = [30, "invalid", text, null];

              for (const item of invalidItems) {
                assert(errorName(() => lengthBase.initialize(item)) === "TypeError",
                  "SVGLengthList.initialize item type");
                assert(errorName(() => lengthBase.insertItemBefore(item, 0)) === "TypeError",
                  "SVGLengthList.insertItemBefore item type");
                assert(errorName(() => lengthBase.replaceItem(item, 0)) === "TypeError",
                  "SVGLengthList.replaceItem item type");
                assert(errorName(() => lengthBase.appendItem(item)) === "TypeError",
                  "SVGLengthList.appendItem item type");
                assert(errorName(() => { lengthBase[0] = item; }) === "TypeError",
                  "SVGLengthList indexed setter item type");
              }

              const length = svg.createSVGLength();
              length.value = 42;
              lengthBase[0] = length;
              assert(lengthBase[0] === length, "SVGLengthList indexed getter");
              assert(text.getAttribute("x") === "42 20", "SVGLengthList indexed reflection");

              const number = svg.createSVGNumber();
              number.value = 7;
              numberBase[1] = number;
              assert(numberBase[1] === number, "SVGNumberList indexed getter");
              assert(text.getAttribute("rotate") === "15 7", "SVGNumberList indexed reflection");
              assert(errorName(() => lengthBase.appendItem(number)) === "TypeError",
                "SVGLengthList rejects SVGNumber");
              assert(errorName(() => numberBase.appendItem(length)) === "TypeError",
                "SVGNumberList rejects SVGLength");

              text.setAttribute("x", "1 2 3");
              assert(lengthBase.length === 3 && lengthBase[2].value === 3,
                "saved baseVal resynchronizes");
              assert(text.x.animVal.length === 3 && text.x.animVal[2].value === 3,
                "animVal resynchronizes after direct baseVal access");

              assert(errorName(() => lengthAnim.clear()) === "NoModificationAllowedError",
                "SVGLengthList animVal clear");
              assert(errorName(() => { lengthAnim[0] = length; }) === "NoModificationAllowedError",
                "SVGLengthList animVal indexed setter");
              assert(errorName(() => numberAnim.appendItem(number)) === "NoModificationAllowedError",
                "SVGNumberList animVal appendItem");
              assert(errorName(() => SVGLengthList.prototype.clear.call({})) === "TypeError",
                "SVGLengthList receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG value list semantics probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_point_lists_are_live_mutable_and_clear_invalid_content() {
    let mut vm = new_storage_test_vm("https://svg-point-list-semantics.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const errorName = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error.name;
                }
              };
              const ns = "http://www.w3.org/2000/svg";
              const polygon = document.createElementNS(ns, "polygon");
              const polyline = document.createElementNS(ns, "polyline");
              const svg = document.createElementNS(ns, "svg");

              assert(typeof SVGPointList === "function", "constructor exposed");
              assert(errorName(() => new SVGPointList()) === "TypeError",
                "illegal constructor");
              for (const [constructor, element] of [
                [SVGPolygonElement, polygon],
                [SVGPolylineElement, polyline],
              ]) {
                const pointsDescriptor = Object.getOwnPropertyDescriptor(
                  constructor.prototype,
                  "points",
                );
                const animatedDescriptor = Object.getOwnPropertyDescriptor(
                  constructor.prototype,
                  "animatedPoints",
                );
                assert(typeof pointsDescriptor.get === "function" &&
                  pointsDescriptor.set === undefined, `${constructor.name}.points descriptor`);
                assert(typeof animatedDescriptor.get === "function" &&
                  animatedDescriptor.set === undefined,
                  `${constructor.name}.animatedPoints descriptor`);
                assert(pointsDescriptor.enumerable && pointsDescriptor.configurable &&
                  animatedDescriptor.enumerable && animatedDescriptor.configurable,
                  `${constructor.name} descriptor flags`);
                assert(element.points instanceof SVGPointList &&
                  element.animatedPoints instanceof SVGPointList,
                  `${constructor.name} point list interfaces`);
                assert(element.points === element.points &&
                  element.animatedPoints === element.animatedPoints &&
                  element.points !== element.animatedPoints,
                  `${constructor.name} SameObject lists`);
              }

              polygon.setAttribute("points", "0,0 100,0 100,100 0,100");
              const points = polygon.points;
              const animatedPoints = polygon.animatedPoints;
              assert(Object.prototype.toString.call(points) === "[object SVGPointList]",
                "point list tag");
              assert(points.length === 4 && points.numberOfItems === 4,
                "valid content points");
              assert(points.getItem(1) === points[1] && points[1] instanceof DOMPoint &&
                points[1] instanceof SVGPoint && points[1].x === 100 && points[1].y === 0,
                "indexed point identity and values");

              polygon.setAttribute("points", "0,0 100,0 INVALID");
              assert(points.numberOfItems === 0,
                "invalid token clears the whole point list");
              polygon.setAttribute("points", "0,0 100,0 20");
              assert(points.numberOfItems === 2,
                "missing final y coordinate truncates the point list");
              polygon.setAttribute("points", "0,0 100,0 20,");
              assert(points.numberOfItems === 2,
                "trailing comma with missing y truncates the point list");

              polygon.setAttribute("points", "0,0 10,20");
              const first = points[0];
              first.x = 2;
              assert(polygon.getAttribute("points") === "2 0 10 20",
                "point coordinate mutation reflects to content");

              const point = svg.createSVGPoint();
              point.x = 5;
              point.y = 6;
              points.clear();
              assert(points.length === 0 && polygon.getAttribute("points") === "",
                "clear reflects an empty list");
              assert(points.initialize(point) === point && points[0] === point,
                "initialize keeps point identity");
              assert(polygon.getAttribute("points") === "5 6", "initialize reflection");
              point.x = 7;
              assert(polygon.getAttribute("points") === "7 6", "owned point stays live");

              const second = svg.createSVGPoint();
              second.x = 8;
              second.y = 9;
              assert(points.appendItem(second) === second && points.length === 2,
                "appendItem");
              const third = svg.createSVGPoint();
              third.x = 10;
              third.y = 11;
              assert(points.insertItemBefore(third, 1) === third && points.length === 3,
                "insertItemBefore");
              const replacement = svg.createSVGPoint();
              replacement.x = 12;
              replacement.y = 13;
              assert(points.replaceItem(replacement, 0) === replacement &&
                points[0] === replacement,
                "replaceItem");
              assert(points.removeItem(1) === third && points.length === 2,
                "removeItem");
              points[0] = point;
              assert(points[0] === point, "indexed setter");

              for (const invalid of [1, "point", polygon, null]) {
                assert(errorName(() => points.appendItem(invalid)) === "TypeError",
                  "point item type enforcement");
              }
              assert(errorName(() => points.getItem(99)) === "IndexSizeError",
                "getItem bounds");

              polygon.setAttribute("points", "1,2 3,4");
              assert(animatedPoints.length === 2 && animatedPoints[1].x === 3,
                "animatedPoints live synchronization");
              assert(errorName(() => animatedPoints.clear()) === "NoModificationAllowedError",
                "animatedPoints list is read-only");
              assert(errorName(() => { animatedPoints[0].x = 9; }) ===
                "NoModificationAllowedError", "animated point is read-only");
              assert(errorName(() => SVGPointList.prototype.clear.call({})) === "TypeError",
                "point list receiver brand");

              const pointsDescriptor = Object.getOwnPropertyDescriptor(
                SVGPolygonElement.prototype,
                "points",
              );
              assert(errorName(() => pointsDescriptor.get.call(svg)) === "TypeError",
                "animated points receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG point list semantics probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn element_heading_reflections_drive_flat_tree_heading_matching() {
    let mut vm = new_storage_test_vm("https://heading-offset.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = name => Object.getOwnPropertyDescriptor(Element.prototype, name);
              const outcome = callback => {
                try {
                  return `ok:${String(callback())}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };

              const parent = document.createElement("div");
              const heading = document.createElement("h1");
              parent.append(heading);
              const mount = document.body || document.documentElement ||
                document.appendChild(document.createElement("html"));
              mount.append(parent);
              const initial = [heading.headingOffset, heading.headingReset, heading.matches(":heading(1)")];
              parent.headingOffset = 3;
              const parentOffset = [parent.getAttribute("headingoffset"), heading.matches(":heading(4)")];
              heading.headingReset = true;
              const reset = [heading.hasAttribute("headingreset"), heading.matches(":heading(1)")];
              heading.headingReset = false;
              heading.headingOffset = 20;
              const clamped = [heading.getAttribute("headingoffset"), heading.headingOffset, heading.matches(":heading(9)")];

              const host = document.createElement("section");
              host.headingOffset = 1;
              const root = host.attachShadow({ mode: "open" });
              const container = document.createElement("div");
              container.headingOffset = 1;
              const slot = document.createElement("slot");
              container.append(slot);
              root.append(container);
              const slotted = document.createElement("h2");
              host.append(slotted);
              mount.append(host);

              const modalParent = document.createElement("div");
              modalParent.headingOffset = 8;
              const modal = document.createElement("dialog");
              const modalHeading = document.createElement("h1");
              modal.append(modalHeading);
              modalParent.append(modal);
              mount.append(modalParent);
              const modalBefore = modal.headingReset;
              modal.showModal();
              const modalState = [modalBefore, modal.headingReset, modalHeading.matches(":heading(1)")];
              modal.close();

              return JSON.stringify({
                owner: [
                  Object.prototype.hasOwnProperty.call(Element.prototype, "headingOffset"),
                  Object.prototype.hasOwnProperty.call(HTMLElement.prototype, "headingOffset"),
                  descriptor("headingOffset").enumerable,
                  descriptor("headingReset").enumerable
                ],
                initial,
                parentOffset,
                reset,
                clamped,
                slotted: slotted.matches(":heading(4)"),
                modalState,
                badGetter: outcome(() => descriptor("headingOffset").get.call({})),
                badSetter: outcome(() => descriptor("headingReset").set.call({}, true))
              });
            })()
            "#,
        )
        .expect("heading reflection and selector probe should evaluate");

    assert_eq!(
        result,
        r#"{"owner":[true,false,true,true],"initial":[0,false,true],"parentOffset":["3",true],"reset":[true,true],"clamped":["20",8,true],"slotted":true,"modalState":[false,true,true],"badGetter":"throw:TypeError","badSetter":"throw:TypeError"}"#
    );
}

#[test]
fn reflected_dom_token_list_attributes_are_live_same_object_and_owner_scoped() {
    let mut vm = new_storage_test_vm("https://reflected-token-lists.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const svg = "http://www.w3.org/2000/svg";
              const detached = document.implementation.createHTMLDocument("");
              const descriptors = [
                [HTMLIFrameElement.prototype, "sandbox"],
                [HTMLLinkElement.prototype, "sizes"],
                [HTMLOutputElement.prototype, "htmlFor"],
                [SVGAElement.prototype, "relList"]
              ];
              for (const [prototype, name] of descriptors) {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} PutForwards setter`);
                assert(descriptor.enumerable && descriptor.configurable, `${name} descriptor flags`);
              }

              const noSupportedTokens = list => {
                try {
                  list.supports("anything");
                  return false;
                } catch (error) {
                  return error && error.name === "TypeError";
                }
              };
              const exercise = ownerDocument => {
                const iframe = ownerDocument.createElement("iframe");
                const sandbox = iframe.sandbox;
                assert(Object.prototype.toString.call(sandbox) === "[object DOMTokenList]", "sandbox type");
                assert(sandbox === iframe.sandbox, "sandbox SameObject");
                iframe.sandbox = "allow-scripts allow-forms allow-scripts";
                assert(sandbox.length === 2 && sandbox.contains("allow-forms"), "sandbox tokens");
                assert(iframe.getAttribute("sandbox") === "allow-scripts allow-forms allow-scripts", "sandbox PutForwards");
                assert(sandbox.supports("ALLOW-SCRIPTS"), "sandbox supports ASCII case-insensitively");
                assert(sandbox.supports("allow-storage-access-by-user-activation"), "sandbox storage-access token");
                assert(!sandbox.supports("unknown"), "sandbox rejects unknown token");
                sandbox.add("allow-popups");
                assert(iframe.getAttribute("sandbox") === "allow-scripts allow-forms allow-popups", "sandbox mutation reflects");

                const output = ownerDocument.createElement("output");
                const htmlFor = output.htmlFor;
                assert(Object.prototype.toString.call(htmlFor) === "[object DOMTokenList]", "htmlFor type");
                assert(htmlFor === output.htmlFor, "htmlFor SameObject");
                output.htmlFor = "first second first";
                htmlFor.add("third");
                assert(output.getAttribute("for") === "first second third", "htmlFor reflects for");
                assert(noSupportedTokens(htmlFor), "htmlFor has no supported tokens");

                const link = ownerDocument.createElement("link");
                const sizes = link.sizes;
                assert(Object.prototype.toString.call(sizes) === "[object DOMTokenList]", "sizes type");
                assert(sizes === link.sizes, "sizes SameObject");
                assert(sizes !== link.relList, "link token lists have distinct identity");
                link.sizes = "16x16 32x32 16x16";
                sizes.remove("16x16");
                assert(link.getAttribute("sizes") === "32x32", "sizes mutation reflects");
                assert(noSupportedTokens(sizes), "sizes has no supported tokens");

                const anchor = ownerDocument.createElementNS(svg, "a");
                const relList = anchor.relList;
                assert(Object.prototype.toString.call(relList) === "[object DOMTokenList]", "SVG relList type");
                assert(relList === anchor.relList, "SVG relList SameObject");
                anchor.relList = "noopener noreferrer";
                relList.add("opener");
                assert(anchor.getAttribute("rel") === "noopener noreferrer opener", "SVG relList reflects rel");
                assert(relList.supports("NOOPENER"), "SVG relList supported tokens");
              };

              exercise(document);
              exercise(detached);

              const div = document.createElement("div");
              for (const name of ["htmlFor", "sandbox", "sizes", "relList"]) {
                assert(div[name] === undefined, `div.${name} should be undefined`);
              }
              assert(document.createElementNS(svg, "link").sizes === undefined, "SVG link.sizes");
              assert(document.createElementNS(svg, "output").htmlFor === undefined, "SVG output.htmlFor");
              assert(document.createElementNS(svg, "iframe").sandbox === undefined, "SVG iframe.sandbox");
              assert(document.createElement("svg").relList === undefined, "HTML svg.relList");
              return "ok";
            })()
            "#,
        )
        .expect("reflected DOMTokenList attributes should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_svg_element_deselect_all_clears_the_owner_document_selection() {
    let mut vm = new_parsed_test_vm(
        "https://svg-deselect-all.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const svgNamespace = "http://www.w3.org/2000/svg";
              const outer = document.createElementNS(svgNamespace, "svg");
              const inner = document.createElementNS(svgNamespace, "svg");
              const svgText = document.createElementNS(svgNamespace, "text");
              const htmlText = document.createElement("p");
              svgText.textContent = "SVG selection";
              htmlText.textContent = "HTML selection";
              inner.appendChild(svgText);
              outer.appendChild(inner);
              document.body.append(outer, htmlText);

              const descriptor = Object.getOwnPropertyDescriptor(
                SVGSVGElement.prototype,
                "deselectAll"
              );
              assert(!!descriptor, "deselectAll descriptor");
              assert(typeof descriptor.value === "function", "deselectAll function");
              assert(descriptor.value.name === "deselectAll", "deselectAll name");
              assert(descriptor.value.length === 0, "deselectAll length");
              assert(descriptor.enumerable && descriptor.writable && descriptor.configurable,
                "deselectAll descriptor flags");

              const selection = window.getSelection();
              const select = node => {
                const range = document.createRange();
                range.selectNodeContents(node);
                selection.removeAllRanges();
                selection.addRange(range);
                assert(selection.rangeCount === 1, "selection precondition");
              };

              outer.deselectAll();
              assert(selection.rangeCount === 0 && selection.isCollapsed,
                "empty selection stays empty");

              select(svgText);
              outer.deselectAll();
              assert(selection.rangeCount === 0 && selection.isCollapsed,
                "outer svg clears SVG selection");

              select(htmlText);
              inner.deselectAll();
              assert(selection.rangeCount === 0 && selection.isCollapsed,
                "inner svg clears selection outside its subtree");

              select(svgText);
              const originalDocumentGetSelection = document.getSelection;
              const originalRemoveAllRanges = Selection.prototype.removeAllRanges;
              document.getSelection = () => { throw new Error("observable getSelection call"); };
              Selection.prototype.removeAllRanges = () => {
                throw new Error("observable removeAllRanges call");
              };
              Object.defineProperty(outer, "ownerDocument", {
                value: null,
                configurable: true
              });
              Object.defineProperty(document, "defaultView", {
                value: null,
                configurable: true
              });
              try {
                outer.deselectAll();
              } finally {
                delete outer.ownerDocument;
                delete document.defaultView;
                document.getSelection = originalDocumentGetSelection;
                Selection.prototype.removeAllRanges = originalRemoveAllRanges;
              }
              assert(selection.rangeCount === 0, "deselectAll uses internal selection state");

              let borrowed = "returned";
              try {
                descriptor.value.call(document.createElement("div"));
              } catch (error) {
                borrowed = error && error.name;
              }
              assert(borrowed === "TypeError", "deselectAll receiver brand");

              const detachedDocument = document.implementation.createHTMLDocument("");
              const detachedSvg = detachedDocument.createElementNS(svgNamespace, "svg");
              detachedSvg.deselectAll();
              return "ok";
            })()
            "#,
        )
        .expect("SVGSVGElement.deselectAll should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_href_animated_string_prefers_href_and_falls_back_to_xlink_href() {
    let mut vm = new_storage_test_vm("https://svg-href-reflection.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const svg = "http://www.w3.org/2000/svg";
              const xlink = "http://www.w3.org/1999/xlink";
              const unrelated = "https://namespaced-href.test/";
              const exercise = ownerDocument => {
                const anchor = ownerDocument.createElementNS(svg, "a");
                const href = anchor.href;
                assert(href === anchor.href, "href is SameObject");

                anchor.setAttributeNS(xlink, "href", "xlink-unprefixed");
                assert(href.baseVal === "xlink-unprefixed", "unprefixed XLink fallback");
                assert(href.animVal === "xlink-unprefixed", "XLink animVal fallback");
                href.baseVal = "xlink-updated";
                assert(anchor.getAttributeNS(xlink, "href") === "xlink-updated",
                  "baseVal updates the XLink attribute");
                assert(!anchor.hasAttributeNS(null, "href"),
                  "XLink update does not create href");

                anchor.setAttributeNS(null, "href", "preferred");
                assert(href.baseVal === "preferred", "href wins regardless of insertion order");
                href.baseVal = "preferred-updated";
                assert(anchor.getAttributeNS(null, "href") === "preferred-updated",
                  "baseVal updates preferred href");
                assert(anchor.getAttributeNS(xlink, "href") === "xlink-updated",
                  "preferred href leaves XLink unchanged");

                anchor.removeAttributeNS(null, "href");
                assert(href.baseVal === "xlink-updated", "removing href restores fallback");
                anchor.removeAttributeNS(xlink, "href");
                assert(href.baseVal === "" && href.animVal === "", "removing fallback resets values");

                anchor.setAttributeNS(xlink, "xlink:href", "xlink-prefixed");
                assert(href.baseVal === "xlink-prefixed", "prefixed XLink fallback");
                href.baseVal = "xlink-prefixed-updated";
                assert(anchor.getAttributeNS(xlink, "href") === "xlink-prefixed-updated",
                  "baseVal updates prefixed XLink attribute");
                assert(anchor.getAttributeNames().includes("xlink:href"),
                  "baseVal preserves the XLink prefix");

                anchor.removeAttributeNS(xlink, "href");
                anchor.setAttributeNS(unrelated, "href", "unrelated");
                assert(href.baseVal === "", "unrelated namespaced href is ignored");
                href.baseVal = "created";
                assert(anchor.getAttributeNS(null, "href") === "created",
                  "baseVal creates an unnamespaced href");
                assert(anchor.getAttributeNS(unrelated, "href") === "unrelated",
                  "baseVal leaves unrelated namespaced href unchanged");

                anchor.removeAttributeNS(null, "href");
                anchor.setAttributeNS(xlink, "xlink:href", "side-effect-fallback");
                href.baseVal = {
                  toString() {
                    anchor.setAttributeNS(null, "href", "created-during-conversion");
                    return "converted";
                  }
                };
                assert(anchor.getAttributeNS(null, "href") === "converted",
                  "baseVal chooses its backing attribute after value conversion");
                assert(anchor.getAttributeNS(xlink, "href") === "side-effect-fallback",
                  "conversion-created href leaves XLink fallback unchanged");
              };

              exercise(document);
              exercise(document.implementation.createHTMLDocument(""));
              return "ok";
            })()
            "#,
        )
        .expect("SVG href reflection should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_value_lists_enforce_item_types_indices_and_read_only_anim_values() {
    let mut vm = new_storage_test_vm("https://svg-value-list-semantics.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const errorName = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error.name;
                }
              };
              const SVG_NS = "http://www.w3.org/2000/svg";
              const text = document.createElementNS(SVG_NS, "text");
              const svg = document.createElementNS(SVG_NS, "svg");
              text.setAttribute("x", "10 20");
              text.setAttribute("rotate", "15 30");

              const lengths = text.x;
              const lengthBase = lengths.baseVal;
              const lengthAnim = lengths.animVal;
              const numbers = text.rotate;
              const numberBase = numbers.baseVal;
              const numberAnim = numbers.animVal;
              const invalidItems = [30, "invalid", text, null];

              for (const item of invalidItems) {
                assert(errorName(() => lengthBase.initialize(item)) === "TypeError",
                  "SVGLengthList.initialize item type");
                assert(errorName(() => lengthBase.insertItemBefore(item, 0)) === "TypeError",
                  "SVGLengthList.insertItemBefore item type");
                assert(errorName(() => lengthBase.replaceItem(item, 0)) === "TypeError",
                  "SVGLengthList.replaceItem item type");
                assert(errorName(() => lengthBase.appendItem(item)) === "TypeError",
                  "SVGLengthList.appendItem item type");
                assert(errorName(() => { lengthBase[0] = item; }) === "TypeError",
                  "SVGLengthList indexed setter item type");
              }

              const length = svg.createSVGLength();
              length.value = 42;
              lengthBase[0] = length;
              assert(lengthBase[0] === length, "SVGLengthList indexed getter");
              assert(text.getAttribute("x") === "42 20", "SVGLengthList indexed reflection");

              const number = svg.createSVGNumber();
              number.value = 7;
              numberBase[1] = number;
              assert(numberBase[1] === number, "SVGNumberList indexed getter");
              assert(text.getAttribute("rotate") === "15 7", "SVGNumberList indexed reflection");
              assert(errorName(() => lengthBase.appendItem(number)) === "TypeError",
                "SVGLengthList rejects SVGNumber");
              assert(errorName(() => numberBase.appendItem(length)) === "TypeError",
                "SVGNumberList rejects SVGLength");

              text.setAttribute("x", "1 2 3");
              assert(lengthBase.length === 3 && lengthBase[2].value === 3,
                "saved baseVal resynchronizes");
              assert(text.x.animVal.length === 3 && text.x.animVal[2].value === 3,
                "animVal resynchronizes after direct baseVal access");

              assert(errorName(() => lengthAnim.clear()) === "NoModificationAllowedError",
                "SVGLengthList animVal clear");
              assert(errorName(() => { lengthAnim[0] = length; }) === "NoModificationAllowedError",
                "SVGLengthList animVal indexed setter");
              assert(errorName(() => numberAnim.appendItem(number)) === "NoModificationAllowedError",
                "SVGNumberList animVal appendItem");
              assert(errorName(() => SVGLengthList.prototype.clear.call({})) === "TypeError",
                "SVGLengthList receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG value list semantics probe should evaluate");

    assert_eq!(result, "ok");
}
