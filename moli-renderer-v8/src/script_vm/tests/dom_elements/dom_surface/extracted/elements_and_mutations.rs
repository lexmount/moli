use super::*;

#[test]
fn customized_built_in_constructors_can_extend_specialized_html_elements() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              let directError = null;
              try {
                new HTMLButtonElement();
              } catch (error) {
                directError = error && error.name;
              }

              let constructorCalls = 0;
              class BuiltInButton extends HTMLButtonElement {
                constructor() {
                  super();
                  constructorCalls += 1;
                  this.customized = true;
                }
              }

              customElements.define("wpt-specialized-button", BuiltInButton, { extends: "button" });
              const button = document.createElement("button", { is: "wpt-specialized-button" });
              return [
                directError,
                constructorCalls,
                button instanceof BuiltInButton,
                button instanceof HTMLButtonElement,
                button instanceof HTMLElement,
                button.tagName,
                button.customized === true
              ].join("|");
            })()
            "#,
        )
        .expect("customized built-in specialized constructor probe should evaluate");

    assert_eq!(result, "TypeError|1|true|true|true|BUTTON|true");
}
#[test]
fn custom_element_disabled_features_shadow_blocks_attach_shadow() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = (callback) => {
                try {
                  const value = callback();
                  return value instanceof ShadowRoot ? "shadow" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };

              class MyCustom extends HTMLElement {}
              customElements.define("my-custom", MyCustom);
              const autonomous = probe(() =>
                document.createElement("my-custom").attachShadow({ mode: "open" })
              );
              const undefinedAutonomous = probe(() =>
                document.createElement("undefined-custom").attachShadow({ mode: "open" })
              );

              class ShadowDisabledElement extends HTMLElement {
                static get disabledFeatures() { return ["shadow"]; }
              }
              const beforeDefinitionHost = document.createElement("shadow-disabled-element");
              const beforeDefinition = probe(() =>
                beforeDefinitionHost.attachShadow({ mode: "closed" })
              );
              const beforeDefinitionDuplicate = probe(() =>
                beforeDefinitionHost.attachShadow({ mode: "closed" })
              );
              customElements.define("shadow-disabled-element", ShadowDisabledElement);
              const afterDefinitionDuplicate = probe(() =>
                beforeDefinitionHost.attachShadow({ mode: "closed" })
              );
              const afterDefinitionNew = probe(() =>
                document.createElement("shadow-disabled-element").attachShadow({ mode: "closed" })
              );

              class ShadowDisabledHeadingElement extends HTMLHeadingElement {
                static get disabledFeatures() { return ["shadow"]; }
              }
              const builtInHost = document.createElement("h2", {
                is: "shadow-disabled-heading-element"
              });
              const builtInBeforeDefinition = probe(() =>
                builtInHost.attachShadow({ mode: "closed" })
              );
              const builtInBeforeDefinitionDuplicate = probe(() =>
                builtInHost.attachShadow({ mode: "closed" })
              );
              const builtInCreatedBeforeDefinition = document.createElement("h2", {
                is: "shadow-disabled-heading-element"
              });
              customElements.define(
                "shadow-disabled-heading-element",
                ShadowDisabledHeadingElement,
                { extends: "h2" }
              );
              const builtInAfterDefinitionDuplicate = probe(() =>
                builtInHost.attachShadow({ mode: "closed" })
              );
              const builtInAfterDefinitionNew = probe(() =>
                document.createElement("h2", {
                  is: "shadow-disabled-heading-element"
                }).attachShadow({ mode: "closed" })
              );
              const builtInCreatedBeforeDefinitionLaterAttach = probe(() =>
                builtInCreatedBeforeDefinition.attachShadow({ mode: "closed" })
              );

              class CapitalShadowDisabledElement extends HTMLElement {
                static get disabledFeatures() { return ["SHADOW"]; }
              }
              customElements.define(
                "capital-shadow-disabled-element",
                CapitalShadowDisabledElement
              );
              const capitalShadow = probe(() =>
                document.createElement("capital-shadow-disabled-element")
                  .attachShadow({ mode: "open" })
              );

              class MyInput extends HTMLInputElement {}
              customElements.define("my-input", MyInput, { extends: "input" });
              const inputBuiltin = probe(() =>
                document.createElement("input", { is: "my-input" })
                  .attachShadow({ mode: "open" })
              );

              return [
                autonomous,
                undefinedAutonomous,
                beforeDefinition,
                beforeDefinitionDuplicate,
                afterDefinitionDuplicate,
                afterDefinitionNew,
                builtInBeforeDefinition,
                builtInBeforeDefinitionDuplicate,
                builtInAfterDefinitionDuplicate,
                builtInAfterDefinitionNew,
                builtInCreatedBeforeDefinitionLaterAttach,
                capitalShadow,
                inputBuiltin
              ].join("|");
            })()
            "#,
        )
        .expect("custom element disabledFeatures shadow probe should evaluate");

    assert_eq!(
        result,
        "shadow|shadow|shadow|NotSupportedError|NotSupportedError|NotSupportedError|shadow|NotSupportedError|NotSupportedError|NotSupportedError|NotSupportedError|shadow|NotSupportedError"
    );
}
#[test]
fn exec_command_insert_text_reuses_adjacent_text_at_element_boundaries() {
    let mut vm = new_storage_test_vm("https://exec-command-insert-text-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("div");
  host.contentEditable = "true";
  (document.body || document.documentElement || document).appendChild(host);
  host.focus();
  const selection = getSelection();

  host.innerHTML = "<img>foo<img>";
  selection.collapse(host, 1);
  const insertedBefore = document.execCommand("insertText", false, "x");
  const beforeText = host.childNodes[1];
  const before = [
    insertedBefore,
    host.childNodes.length,
    beforeText.data,
    selection.anchorNode === beforeText,
    selection.anchorOffset
  ].join(":");

  host.innerHTML = "<img>foo<img>";
  selection.collapse(host, 2);
  const insertedAfter = document.execCommand("insertText", false, "x");
  const afterText = host.childNodes[1];
  const after = [
    insertedAfter,
    host.childNodes.length,
    afterText.data,
    selection.anchorNode === afterText,
    selection.anchorOffset
  ].join(":");

  return `${before}|${after}`;
})()
"#,
        )
        .expect("execCommand boundary insertText probe should evaluate");

    assert_eq!(result, "true:3:xfoo:true:1|true:3:foox:true:4");
}
#[test]
fn live_elements_expose_named_node_map_attributes_surface() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body data-x=\"1\" hidden dir=\"rtl\"></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const attrs = document.body.attributes;
              const parts = [];
              for (let i = 0; i < attrs.length; i++) {
                parts.push(`${attrs[i].name}=${attrs[i].value}`);
              }
              return JSON.stringify({
                hasAttributesSurface: "attributes" in document.body,
                tag: Object.prototype.toString.call(attrs),
                ctor: attrs.constructor?.name ?? null,
                length: attrs.length,
                parts: parts.join("|"),
                protoEnumerable: Object.getOwnPropertyDescriptor(Element.prototype, "attributes")?.enumerable ?? null,
                protoConfigurable: Object.getOwnPropertyDescriptor(Element.prototype, "attributes")?.configurable ?? null,
                protoGetterType: typeof Object.getOwnPropertyDescriptor(Element.prototype, "attributes")?.get
              });
            })()
            "#,
        )
        .expect("element attributes surface should evaluate");

    assert_eq!(
        result,
        r#"{"hasAttributesSurface":true,"tag":"[object NamedNodeMap]","ctor":"NamedNodeMap","length":3,"parts":"data-x=1|hidden=|dir=rtl","protoEnumerable":true,"protoConfigurable":true,"protoGetterType":"function"}"#
    );
}
#[test]
fn dataset_writes_and_deletes_only_null_namespace_attributes() {
    let mut vm = new_storage_test_vm("https://dataset-namespace.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.setAttributeNS('urn:first', 'data-my-custom-attr', 'first');
  element.setAttributeNS('urn:second', 'data-my-custom-attr', 'second');

  element.dataset.myCustomAttr = 'third';
  const afterSet = Array.from(element.attributes, attribute => [
    attribute.namespaceURI,
    attribute.name,
    attribute.value
  ]);
  const nullNamespaceValue = element.getAttributeNS(null, 'data-my-custom-attr');

  delete element.dataset.myCustomAttr;
  const afterDelete = Array.from(element.attributes, attribute => [
    attribute.namespaceURI,
    attribute.name,
    attribute.value
  ]);

  return JSON.stringify({ afterSet, nullNamespaceValue, afterDelete });
})()
"#,
        )
        .expect("dataset namespace-isolation probe should evaluate");

    assert_eq!(
        result,
        r#"{"afterSet":[["urn:first","data-my-custom-attr","first"],["urn:second","data-my-custom-attr","second"],[null,"data-my-custom-attr","third"]],"nullNamespaceValue":"third","afterDelete":[["urn:first","data-my-custom-attr","first"],["urn:second","data-my-custom-attr","second"]]}"#
    );
}
#[test]
fn detached_dom_parser_elements_expose_named_node_map_attributes_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = new DOMParser().parseFromString(
                '<!doctype html><html><body data-x="1" hidden dir="rtl"></body></html>',
                'text/html'
              );
              const attrs = doc.body.attributes;
              const parts = [];
              for (let i = 0; i < attrs.length; i++) {
                parts.push(`${attrs[i].name}=${attrs[i].value}`);
              }
              return JSON.stringify({
                hasAttributesSurface: "attributes" in doc.body,
                tag: Object.prototype.toString.call(attrs),
                ctor: attrs.constructor?.name ?? null,
                length: attrs.length,
                parts: parts.join("|")
              });
            })()
            "#,
        )
        .expect("detached element attributes surface should evaluate");

    assert_eq!(
        result,
        r#"{"hasAttributesSurface":true,"tag":"[object NamedNodeMap]","ctor":"NamedNodeMap","length":3,"parts":"data-x=1|hidden=|dir=rtl"}"#
    );
}
#[test]
fn live_element_attribute_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://element-attribute-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const el = document.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  function probe(callback) {
    try {
      return callback();
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  el.setAttribute(null, undefined);
  el.setAttributeNS(null, "data-local", "local");
  el.setAttributeNS(undefined, "data-undefined-ns", "undefined-ns");
  el.setAttributeNS("urn:moli:test", "lm:flag", "on");
  const attrNode = el.getAttributeNode({ toString() { return "null"; } });
  const attrNodeNs = el.getAttributeNodeNS(
    { toString() { return "urn:moli:test"; } },
    { toString() { return "flag"; } }
  );
  const toggledNull = el.toggleAttribute("data-toggle", null);
  const toggledUndefined = el.toggleAttribute("data-toggle", undefined);
  const beforeRemove = [
    el.getAttribute("null"),
    el.getAttributeNS(null, "data-local"),
    el.getAttributeNS(undefined, "data-local"),
    el.getAttributeNS(null, "data-undefined-ns"),
    el.hasAttributeNS("urn:moli:test", "flag"),
    attrNode && attrNode.value,
    attrNodeNs && attrNodeNs.value,
    toggledNull,
    toggledUndefined,
    el.hasAttribute("data-toggle")
  ].join(",");
  el.removeAttributeNS("urn:moli:test", "flag");
  return [
    beforeRemove,
    el.hasAttributeNS("urn:moli:test", "flag"),
    probe(() => el.getAttribute()),
    probe(() => el.getAttributeNode()),
    probe(() => el.getAttributeNodeNS(undefined)),
    probe(() => el.getAttributeNS(undefined)),
    probe(() => el.getAttributeNode(Symbol())),
    probe(() => el.setAttribute("x", Symbol())),
    probe(() => el.getAttribute(Symbol()))
  ].join("|");
})()
"#,
        )
        .expect("live Element attribute WebIDL args should evaluate");

    assert_eq!(
        result,
        "undefined,local,local,undefined-ns,true,undefined,on,false,true,true|false|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn live_element_attribute_name_validation_matches_chromium() {
    let mut vm = new_storage_test_vm("https://element-attribute-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
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
    const el = document.createElement("div");
    return probe(() => el.setAttribute(name, "v")) === "undefined" &&
      el.hasAttribute(name) &&
      el.getAttribute(name) === "v";
  });
  const toggleAllowed = allowed.every(name => {
    const el = document.createElement("div");
    return probe(() => el.toggleAttribute(name)) === "true" &&
      el.hasAttribute(name);
  });
  const createAllowed = allowed.every(name =>
    probe(() => document.createAttribute(name).name.length === name.length) === "true"
  );
  const nsAllowed = [
    "@slotchange$lit$",
    "1name",
    "a:0",
    "0:a",
    "a:b:c"
  ].every(name => {
    const el = document.createElement("div");
    return probe(() => el.setAttributeNS("urn:test", name, "v")) === "undefined";
  });
  const invalidSet = invalid.map(name =>
    probe(() => document.createElement("div").setAttribute(name, "v"))
  ).join(",");
  const invalidToggle = invalid.map(name =>
    probe(() => document.createElement("div").toggleAttribute(name))
  ).join(",");
  const invalidCreate = invalid.map(name =>
    probe(() => document.createAttribute(name))
  ).join(",");
  const invalidRemove = invalid.map(name => {
    const el = document.createElement("div");
    el.setAttribute("data-ok", "1");
    return probe(() => el.removeAttribute(name)) + ":" + el.getAttribute("data-ok");
  }).join(",");
  return [
    setAllowed,
    toggleAllowed,
    createAllowed,
    nsAllowed,
    invalidSet,
    invalidToggle,
    invalidCreate,
    invalidRemove
  ].join("|");
})()
"#,
        )
        .expect("live attribute name validation should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError,throw:InvalidCharacterError|undefined:1,undefined:1,undefined:1,undefined:1,undefined:1,undefined:1"
    );
}
#[test]
fn element_attribute_names_and_attr_nodes_match_dom_edge_cases() {
    let mut vm = new_storage_test_vm("https://element-attribute-edge-cases.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xml = document.implementation.createDocument(null, "");
  const weirdNames = ["xml:lang", "_name.1", "俄语"];
  const weird = weirdNames.map(name => {
    const attr = xml.createAttribute(name);
    return `${attr.name}:${attr.specified}`;
  }).join(",");

  const foreign = document.createElementNS("http://www.example.com", "foo");
  foreign.setAttribute("A", "test");
  const foreignParts = [
    foreign.hasAttribute("A"),
    foreign.hasAttributeNS("", "A"),
    foreign.getAttributeNS(null, "A")
  ].join(",");

  const div = document.createElement("div");
  const attr1 = document.createAttributeNS("ns1", "p1:name");
  attr1.value = "value1";
  const attr2 = document.createAttributeNS("ns2", "p2:name");
  attr2.value = "value2";
  div.setAttributeNode(attr1);
  div.setAttributeNodeNS(attr2);

  const caseEl = document.createElement("div");
  const lower = document.createAttributeNS("ns1", "foobar");
  const upper = document.createAttributeNS("ns1", "FOOBAR");
  caseEl.setAttributeNode(lower);
  const old = caseEl.setAttributeNode(upper);

  const htmlInXml = xml.createElementNS("http://www.w3.org/1999/xhtml", "div");
  htmlInXml.setAttributeNS("foo", "A:B", "");
  htmlInXml.setAttributeNS("", "I", "");

  return [
    weird,
    foreignParts,
    div.getAttributeNodeNS("ns1", "name").value,
    div.getAttributeNodeNS("ns2", "name").value,
    old === null,
    lower.ownerElement === caseEl,
    upper.ownerElement === caseEl,
    Object.getOwnPropertyNames(htmlInXml.attributes).join(",")
  ].join("|");
})()
"#,
        )
        .expect("attribute edge case probe should evaluate");

    assert_eq!(
        result,
        "xml:lang:true,_name.1:true,俄语:true|true,true,test|value1|value2|true|true|true|0,1,A:B,I"
    );
}
#[test]
fn html_unknown_element_brand_survives_clone_node_custom_prototype() {
    let mut vm = new_storage_test_vm("https://html-unknown-clone-node.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const unknown = document.createElement("unknown");
  const clone = unknown.cloneNode();
  const proto = Object.create(HTMLElement.prototype);
  const customProtoNode = document.createElement("hi");
  Object.setPrototypeOf(customProtoNode, proto);
  const customProtoClone = customProtoNode.cloneNode(true);
  const autonomous = document.createElement("x-foo");
  return [
    unknown instanceof HTMLUnknownElement,
    Object.prototype.toString.call(unknown),
    clone instanceof HTMLUnknownElement,
    Object.prototype.toString.call(clone),
    proto.isPrototypeOf(customProtoNode),
    proto.isPrototypeOf(customProtoClone),
    customProtoClone instanceof HTMLUnknownElement,
    Object.prototype.toString.call(customProtoClone),
    autonomous instanceof HTMLElement,
    autonomous instanceof HTMLUnknownElement
  ].join("|");
})()
"#,
        )
        .expect("HTMLUnknownElement cloneNode probe should evaluate");

    assert_eq!(
        result,
        "true|[object HTMLUnknownElement]|true|[object HTMLUnknownElement]|true|false|true|[object HTMLUnknownElement]|true|false"
    );
}
#[test]
fn detached_insert_adjacent_methods_match_element_surface() {
    let mut vm = new_storage_test_vm("https://detached-insert-adjacent.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("div");
  const child = doc.createElement("span");
  const loose = doc.createElement("p");
  doc.body.appendChild(host);
  function probe(callback) {
    try {
      const value = callback();
      return value === null ? "null" : value === undefined ? "undefined" : String(value);
    } catch (error) {
      return error && error.name;
    }
  }
  const returned = host.insertAdjacentElement("beforeend", child);
  host.insertAdjacentText("afterbegin", "text");
  return [
    returned === child,
    host.firstChild.textContent,
    host.lastChild.localName,
    probe(() => loose.insertAdjacentElement("beforebegin", doc.createElement("b"))),
    probe(() => doc.documentElement.insertAdjacentElement("beforebegin", doc.createElement("b"))),
    probe(() => doc.documentElement.insertAdjacentText("beforebegin", "x")),
    probe(() => host.insertAdjacentElement("sideways", doc.createElement("i"))),
    probe(() => host.insertAdjacentElement("beforeend", doc.doctype))
  ].join("|");
})()
"#,
        )
        .expect("detached insertAdjacent methods should evaluate");

    assert_eq!(
        result,
        "true|text|span|null|HierarchyRequestError|HierarchyRequestError|SyntaxError|TypeError"
    );
}
#[test]
fn attr_nodes_inherit_node_but_tree_mutation_rejects_them() {
    let mut vm = new_storage_test_vm("https://attr-node-tree-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement("div");
  const child = document.createElement("span");
  const attr = document.createAttribute("data-value");
  parent.appendChild(child);
  const outcome = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  };

  return [
    attr instanceof Attr,
    attr instanceof Node,
    Attr.prototype instanceof Node,
    Object.getPrototypeOf(Attr.prototype) === Node.prototype,
    outcome(() => parent.appendChild(attr)),
    outcome(() => parent.insertBefore(attr, child)),
    outcome(() => parent.replaceChild(attr, child)),
    outcome(() => parent.append(attr)),
    outcome(() => child.before(attr)),
    outcome(() => parent.removeChild(attr)),
    outcome(() => parent.insertBefore(document.createTextNode("x"), attr)),
    parent.firstChild === child,
    parent.childNodes.length
  ].join("|");
})()
"#,
        )
        .expect("Attr Node inheritance and tree mutation probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|HierarchyRequestError:3|HierarchyRequestError:3|HierarchyRequestError:3|HierarchyRequestError:3|HierarchyRequestError:3|NotFoundError:8|NotFoundError:8|true|1"
    );
}
#[test]
fn attr_clone_node_copies_attribute_metadata_without_owner() {
    let mut vm = new_storage_test_vm("https://attr-clone-node.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methodShape = (owner, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      Object.prototype.hasOwnProperty.call(owner, name),
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable,
      descriptor.writable
    ].join(':');
  };
  const attr = document.createAttribute("data-value");
  attr.value = "one";
  const attrClone = attr.cloneNode(true);

  const namespaced = document.createAttributeNS("urn:moli:test", "lm:flag");
  namespaced.value = "two";
  const namespacedClone = namespaced.cloneNode(false);

  const element = document.createElement("div");
  const live = document.createAttributeNS("urn:moli:live", "lm:item");
  live.value = "before";
  element.setAttributeNodeNS(live);
  element.setAttributeNS("urn:moli:live", "lm:item", "after");
  const liveClone = live.cloneNode();

  return JSON.stringify({
    attr: [
      attrClone !== attr,
      attrClone instanceof Attr,
      Object.prototype.toString.call(attrClone),
      attrClone.nodeType,
      attrClone.name,
      attrClone.localName,
      attrClone.value,
      attrClone.ownerElement === null,
      typeof attrClone.cloneNode,
      attrClone.cloneNode().value
    ],
    namespaced: [
      namespacedClone !== namespaced,
      namespacedClone.name,
      namespacedClone.localName,
      namespacedClone.prefix,
      namespacedClone.namespaceURI,
      namespacedClone.value,
      namespacedClone.ownerElement === null
    ],
    live: [
      live.ownerElement === element,
      live.value,
      liveClone !== live,
      liveClone.ownerElement === null,
      liveClone.name,
      liveClone.localName,
      liveClone.prefix,
      liveClone.namespaceURI,
      liveClone.value
    ],
    methods: [
      methodShape(Node.prototype, "isSameNode"),
      methodShape(attr, "cloneNode"),
      methodShape(attr, "lookupNamespaceURI")
    ],
    methodInheritance: [
      Object.hasOwn(attr, "isSameNode"),
      Object.hasOwn(attrClone, "isSameNode"),
      attr.isSameNode === Node.prototype.isSameNode,
      attrClone.isSameNode === Node.prototype.isSameNode
    ],
    methodBehavior: [
      attr.isSameNode(attr),
      attr.isSameNode(attrClone),
      live.lookupNamespaceURI("xml")
    ]
  });
})()
"#,
        )
        .expect("Attr cloneNode probe should evaluate");

    assert_eq!(
        result,
        r#"{"attr":[true,true,"[object Attr]",2,"data-value","data-value","one",true,"function","one"],"namespaced":[true,"lm:flag","flag","lm","urn:moli:test","two",true],"live":[true,"after",true,true,"lm:item","item","lm","urn:moli:live","after"],"methods":["true:function:isSameNode:1:true:true:true","true:function:cloneNode:0:false:true:true","true:function:lookupNamespaceURI:0:false:true:true"],"methodInheritance":[false,false,true,true],"methodBehavior":[true,false,"http://www.w3.org/XML/1998/namespace"]}"#
    );
}
#[test]
fn live_node_wrappers_use_intrinsic_prototypes_after_public_constructor_replacement() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/",
        "<!doctype html><html><body><span data-cp=41></span></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const intrinsicPrototype = HTMLSpanElement.prototype;
  Object.defineProperty(globalThis, "HTMLSpanElement", {
    configurable: true,
    value: undefined,
  });
  const span = document.querySelector("span");
  return `${span.dataset.cp}|${Object.getPrototypeOf(span) === intrinsicPrototype}`;
})()
"#,
        )
        .expect("live node wrapper should use the realm's intrinsic prototype");

    assert_eq!(result, "41|true");
}
#[test]
fn live_html_get_attribute_node_normalizes_cached_attr_name() {
    let mut vm = new_storage_test_vm("https://live-attribute-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const el = document.createElement("div");
  el.setAttribute("DATA-CASE", "v");
  const first = el.getAttributeNode("DATA-CASE");
  const second = el.getAttributeNode("data-case");
  return [
    el.getAttribute("data-case"),
    el.getAttribute("DATA-CASE"),
    first && first.name,
    first === second,
    el.getAttributeNames().join(",")
  ].join("|");
})()
"#,
        )
        .expect("live HTML getAttributeNode cache should evaluate");

    assert_eq!(result, "v|v|data-case|true|data-case");
}
#[test]
fn svg_style_element_declares_and_reflects_its_own_attributes() {
    let mut vm = new_storage_test_vm("https://svg-style-reflection.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const SVG_NS = "http://www.w3.org/2000/svg";
              const style = document.createElementNS(SVG_NS, "style");
              const prototype = SVGStyleElement.prototype;
              const accessorShape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  !!descriptor,
                  typeof descriptor?.get,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };
              const throwsTypeError = callback => {
                try {
                  callback();
                  return false;
                } catch (error) {
                  return error instanceof TypeError;
                }
              };

              const missing = [style.media, style.title, style.type];
              style.media = "screen";
              style.title = "theme";
              style.type = "text/example";
              const reflected = [
                style.getAttribute("media"),
                style.getAttribute("title"),
                style.getAttribute("type")
              ];
              style.setAttribute("media", "print");
              style.setAttribute("title", "alternate");
              style.setAttribute("type", "text/css");

              return JSON.stringify({
                accessors: ["media", "title", "type", "disabled"].map(accessorShape),
                missing,
                reflected,
                attributes: [style.media, style.title, style.type],
                disabled: style.disabled,
                incompatible: [
                  throwsTypeError(() => Reflect.get(prototype, "media", {})),
                  throwsTypeError(() => Reflect.set(prototype, "type", "text/css", {})),
                  throwsTypeError(() => Reflect.get(prototype, "disabled", {}))
                ]
              });
            })()
            "#,
        )
        .expect("SVGStyleElement reflection should evaluate");

    assert_eq!(
        result,
        r#"{"accessors":["true:function:function:true:true","true:function:function:true:true","true:function:function:true:true","true:function:function:true:true"],"missing":["","",""],"reflected":["screen","theme","text/example"],"attributes":["print","alternate","text/css"],"disabled":false,"incompatible":[true,true,true]}"#,
    );
}
#[test]
fn dom_token_list_replace_matches_order_and_validation_edges() {
    let mut vm = new_storage_test_vm("https://dom-token-list-replace.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const element = document.createElement("div");
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              element.className = "c b a";
              const existing = element.classList.replace("c", "a");
              const existingClass = element.className;
              element.className = "a a a  b";
              const observer = new MutationObserver(() => {});
              observer.observe(element, { attributes: true });
              const same = element.classList.replace("a", "a");
              const sameMutations = observer.takeRecords().length;
              observer.disconnect();
              const sameClass = element.className;
              const validation = probe(() => element.classList.replace(" ", ""));
              return JSON.stringify({ existing, existingClass, same, sameMutations, sameClass, validation });
            })()
            "#,
        )
        .expect("DOMTokenList.replace edge cases should evaluate");

    assert_eq!(
        result,
        r#"{"existing":true,"existingClass":"a b","same":true,"sameMutations":1,"sameClass":"a b","validation":"throw:SyntaxError"}"#
    );
}
#[test]
fn html_link_as_reflects_attribute() {
    let mut vm = new_storage_test_vm("https://link-as.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const link = document.createElement("link");
              const beforeOwn = Object.prototype.hasOwnProperty.call(link, "as");
              link.as = "json";
              const afterSet = {
                as: link.as,
                attr: link.getAttribute("as"),
                own: Object.prototype.hasOwnProperty.call(link, "as")
              };
              link.setAttribute("as", "text");
              const cases = {
                "Image": "image",
                "images": "",
                "scripT": "script",
                "style": "style",
                "": "",
                "foNt": "font",
                "foobar": "",
                "video": "video",
                "audio": "audio",
                "track": "track",
                "fetch": "fetch",
                "json": "json",
                "text": "text"
              };
              const reflected = {};
              for (const key of Object.keys(cases)) {
                link.as = key;
                reflected[key] = link.as;
              }
              return JSON.stringify({
                beforeOwn,
                descriptorOwner: Object.prototype.hasOwnProperty.call(HTMLLinkElement.prototype, "as"),
                afterSet,
                afterAttr: link.as,
                reflected
              });
            })()
            "#,
        )
        .expect("HTMLLinkElement.as should reflect the as content attribute");

    assert_eq!(
        result,
        r#"{"beforeOwn":false,"descriptorOwner":true,"afterSet":{"as":"json","attr":"json","own":false},"afterAttr":"text","reflected":{"Image":"image","images":"","scripT":"script","style":"style","":"","foNt":"font","foobar":"","video":"video","audio":"audio","track":"track","fetch":"fetch","json":"json","text":"text"}}"#
    );
}
#[test]
fn html_link_integrity_reflects_attribute_on_live_and_detached_documents() {
    let mut vm = new_storage_test_vm("https://link-integrity.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const descriptor = Object.getOwnPropertyDescriptor(HTMLLinkElement.prototype, "integrity");
  assert(descriptor && typeof descriptor.get === "function" &&
    typeof descriptor.set === "function", "prototype accessor");
  assert(descriptor.enumerable && descriptor.configurable, "descriptor flags");
  const documents = [
    document,
    document.implementation.createHTMLDocument(""),
    new DOMParser().parseFromString("<link integrity='sha384-parsed'>", "text/html")
  ];
  assert(documents[2].querySelector("link").integrity === "sha384-parsed", "parsed attribute");
  for (const doc of documents) {
    const link = doc.createElement("link");
    assert(link.integrity === "" && !link.hasAttribute("integrity"), "missing attribute");
    link.integrity = "sha384-element";
    assert(link.getAttribute("integrity") === "sha384-element", "IDL assignment sets attribute");
    assert(!Object.prototype.hasOwnProperty.call(link, "integrity"), "assignment uses prototype");
    link.setAttribute("integrity", "sha256-attribute");
    assert(link.integrity === "sha256-attribute", "content attribute updates getter");
    link.integrity = "";
    assert(link.integrity === "" && link.hasAttribute("integrity"), "explicit empty attribute");
    link.integrity = null;
    assert(link.getAttribute("integrity") === "null", "DOMString conversion");
    link.removeAttribute("integrity");
    assert(link.integrity === "" && !link.hasAttribute("integrity"), "removed attribute");
  }
  const div = document.createElement("div");
  for (const callback of [() => descriptor.get.call(div), () => descriptor.set.call(div, "hash")]) {
    let error;
    try { callback(); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, "incompatible receiver");
  }
  assert(!div.hasAttribute("integrity"), "invalid setter leaves receiver unchanged");
  return "ok";
})()
"#,
        )
        .expect("link integrity should reflect its attribute through the owning prototype");

    assert_eq!(result, "ok");
}
#[test]
fn assigning_top_does_not_replace_legacy_unforgeable_top_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        top = { hacked: true };
        "#,
        None,
    )
    .expect("assigning top should not throw");

    let result = vm
        .eval(
            r#"
            [
                top === globalThis,
                top.hacked === true,
                Object.getOwnPropertyDescriptor(globalThis, "top")?.configurable === false
            ].join("|")
            "#,
        )
        .expect("top alias probe should evaluate");

    assert_eq!(result, "true|false|true");
}
#[test]
fn top_level_var_parent_can_replace_global_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        var parent = document.createElement("div");
        parent.id = "shadow-parent";
        globalThis.__varParentProbe = [parent.id, typeof parent.appendChild].join("|");
        "#,
        None,
    )
    .expect("top-level var parent should replace the global alias");

    let result = vm
        .eval("globalThis.__varParentProbe")
        .expect("var parent probe should evaluate");

    assert_eq!(result, "shadow-parent|function");
}
#[test]
fn assigning_document_does_not_replace_legacy_unforgeable_document_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        document = { hacked: true };
        "#,
        None,
    )
    .expect("assigning document should not throw");

    let result = vm
        .eval(
            r#"
            [
                document === globalThis.document,
                typeof document.createElement,
                document.hacked === true
            ].join("|")
            "#,
        )
        .expect("document alias reassignment probe should evaluate");

    assert_eq!(result, "true|function|false");
}
#[test]
fn contextual_fragment_scripts_run_when_inserted() {
    let mut vm = new_storage_test_vm("https://contextual-fragment-scripts.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__fragmentScriptOrder = [];
  const range = document.createRange();
  const fragment = range.createContextualFragment(
    "<script>__fragmentScriptOrder.push('fragment script')<\/script>"
  );
  __fragmentScriptOrder.push('before append');
  (document.body || document.documentElement || document).appendChild(fragment.firstChild);
  __fragmentScriptOrder.push('after append');
  return __fragmentScriptOrder.join('|');
})()
"#,
        )
        .expect("contextual fragment inline script should evaluate");

    assert_eq!(result, "before append|fragment script|after append");
}
#[test]
fn document_fragment_insert_runs_nested_scripts_in_tree_order() {
    let mut vm = new_storage_html_test_vm("https://document-fragment-script-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__documentFragmentScriptOrder = [];
  const fragment = document.createDocumentFragment();
  const first = document.createElement('script');
  first.textContent = "__documentFragmentScriptOrder.push('first')";
  const container = document.createElement('div');
  const nested = document.createElement('script');
  nested.textContent = "__documentFragmentScriptOrder.push('nested')";
  const last = document.createElement('script');
  last.textContent = "__documentFragmentScriptOrder.push('last')";
  fragment.appendChild(first);
  container.appendChild(nested);
  fragment.appendChild(container);
  fragment.appendChild(last);
  __documentFragmentScriptOrder.push('before append');
  (document.body || document.documentElement || document).appendChild(fragment);
  __documentFragmentScriptOrder.push('after append');
  return __documentFragmentScriptOrder.join('|');
})()
"#,
        )
        .expect("document fragment script order should evaluate");

    assert_eq!(result, "before append|first|nested|last|after append");
}
