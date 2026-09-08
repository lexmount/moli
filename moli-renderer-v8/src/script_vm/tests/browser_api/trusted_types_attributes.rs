use super::*;

#[tokio::test]
async fn adopted_attribute_sinks_use_the_node_document_global_and_preserve_exception_realms() {
    let mut vm = new_storage_test_vm("https://adopted-attribute-types.test/");
    vm.eval(r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "tt-frame";
  frame.srcdoc = `<meta http-equiv="Content-Security-Policy" content="require-trusted-types-for 'script'"><body></body>`;
  body.appendChild(frame);
})()
"#).expect("Trusted Types child setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "TT child should commit").await;
    let result = vm.eval(r#"
(() => {
  const child = document.getElementById("tt-frame").contentWindow;
  const childDoc = child.document;
  const secondary = childDoc.implementation.createHTMLDocument("");
  const explicit = trustedTypes.createPolicy("pass", { createScript: value => value });
  const setters = [
    (element, value) => element.setAttribute("onclick", value),
    (element, value) => element.setAttributeNS(null, "onclick", value),
    ...["setAttributeNode", "setAttributeNodeNS", "setNamedItem", "setNamedItemNS"].map(method =>
      (element, value) => {
        const attr = document.createAttribute("onclick");
        attr.value = value;
        return method.startsWith("setNamed") ? element.attributes[method](attr) : element[method](attr);
      }),
    ...["value", "nodeValue", "textContent"].map(property =>
      (element, value) => { element.getAttributeNode("onclick")[property] = value; })
  ];
  globalThis.__attributeReports = { top: [], child: [] };
  // Observe the receiving global independently of the event's DOM target.
  window.addEventListener("securitypolicyviolation", event => __attributeReports.top.push(event.sample));
  child.addEventListener("securitypolicyviolation", event => __attributeReports.child.push(event.sample));
  const moveAndSet = (source, target, set, index) => {
    const element = source.createElement("button");
    element.setAttribute("onclick", explicit.createScript("initial"));
    target.adoptNode(element);
    // Like the adoption WPT, derive the exception realm from the exposed
    // receiver, without assuming that adoption keeps the original wrapper realm.
    const receiver = index < 6 ? element : element.getAttributeNode("onclick");
    const expectedError = new receiver.constructor.constructor(explicit.createScript("return TypeError"))();
    let outcome = "none";
    try { set(element, "input"); } catch (error) {
      outcome = `${error.name}:${error instanceof expectedError}`;
    }
    return [outcome, element.ownerDocument === target, element.getAttribute("onclick")];
  };
  const intoPlain = setters.map((set, index) => moveAndSet(childDoc, document, set, index));
  const intoEnforced = setters.map((set, index) => moveAndSet(document, childDoc, set, index));
  const intoSecondary = setters.map((set, index) => moveAndSet(document, secondary, set, index));
  const stringConversions = [false, true].map(namespaced => {
    const element = document.createElement("button");
    let converted = 0;
    const value = { toString() { converted++; childDoc.adoptNode(element); return "converted-input"; }};
    let blocked = false;
    try {
      if (namespaced) element.setAttributeNS(null, "onclick", value);
      else element.setAttribute("onclick", value);
    } catch (error) { blocked = error instanceof TypeError; }
    return [blocked, converted, element.ownerDocument === childDoc, element.hasAttribute("onclick")];
  });
  const calls = [];
  trustedTypes.createPolicy("default", { createScript: value => { calls.push("top"); return `top-${value}`; }});
  child.trustedTypes.createPolicy("default", { createScript: value => { calls.push("child"); return `child-${value}`; }});
  const defaulted = setters.map((set, index) => moveAndSet(document, childDoc, set, index));
  const secondaryDefaulted = setters.map((set, index) => moveAndSet(document, secondary, set, index));
  return JSON.stringify({ intoPlain, intoEnforced, intoSecondary, stringConversions, defaulted, secondaryDefaulted, calls });
})()
"#).expect("adopted attribute sink checks should evaluate");
    let allowed = vec![serde_json::json!(["none", true, "input"]); 9];
    let blocked = vec![serde_json::json!(["TypeError:true", true, "initial"]); 9];
    let converted = vec![serde_json::json!(["none", true, "child-input"]); 9];
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "intoPlain": allowed, "intoEnforced": blocked, "intoSecondary": blocked,
        "stringConversions": [[true, 1, true, false], [true, 1, true, false]],
        "defaulted": converted, "secondaryDefaulted": converted, "calls": vec!["child"; 18]
        })
    );
    drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([__attributeReports.top.length, __attributeReports.child.length])")
            .unwrap(),
        "[0,20]"
    );
}

#[test]
fn secondary_document_attribute_nodes_enforce_trusted_types_without_retrying() {
    let mut vm = new_storage_test_vm("https://secondary-document-attribute-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);
    let result = vm
        .eval(
            r#"
(() => {
  const secondary = document.implementation.createHTMLDocument("");
  const setters = [
    (element, attr) => element.setAttributeNode(attr),
    (element, attr) => element.setAttributeNodeNS(attr),
    (element, attr) => element.attributes.setNamedItem(attr),
    (element, attr) => element.attributes.setNamedItemNS(attr)
  ];
  const rejected = setters.map(set => {
    const element = secondary.createElement("button");
    const attr = secondary.createAttribute("onclick");
    attr.value = "blocked";
    let rejected = false;
    try { set(element, attr); } catch (error) { rejected = error instanceof TypeError; }
    return [rejected, attr.ownerElement === null, element.hasAttribute("onclick")];
  });
  const calls = [];
  const exception = new RangeError("policy callback");
  trustedTypes.createPolicy("default", { createScript(value) {
    calls.push(value);
    if (value === "throw") throw exception;
    return `safe-${value}`;
  }});
  const accepted = setters.map(set => {
    const element = secondary.createElement("button");
    const attr = secondary.createAttribute("onclick");
    attr.value = "input";
    const old = set(element, attr);
    return [old === null, attr.ownerElement === element,
      element.getAttribute("onclick"), element.getAttributeNode("onclick") === attr];
  });
  const abrupt = setters.map(set => {
    const element = secondary.createElement("button");
    const attr = secondary.createAttribute("onclick");
    attr.value = "throw";
    let preserved = false;
    try { set(element, attr); } catch (error) { preserved = error === exception; }
    return [preserved, attr.ownerElement === null, element.hasAttribute("onclick")];
  });
  return JSON.stringify({ rejected, accepted, abrupt, calls });
})()
"#,
        )
        .expect("secondary document attribute node checks should evaluate");
    assert_eq!(
        result,
        r#"{"rejected":[[true,true,false],[true,true,false],[true,true,false],[true,true,false]],"accepted":[[true,true,"safe-input",true],[true,true,"safe-input",true],[true,true,"safe-input",true],[true,true,"safe-input",true]],"abrupt":[[true,true,false],[true,true,false],[true,true,false],[true,true,false]],"calls":["input","input","input","input","throw","throw","throw","throw"]}"#
    );
}

#[test]
fn attribute_node_lookup_preserves_native_namespaces_and_shared_identity() {
    let mut vm = new_storage_test_vm("https://attribute-node-namespace.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const ns = "http://www.w3.org/1999/xlink";
  const secondary = document.implementation.createHTMLDocument("");
  const results = [];
  for (const doc of [document, secondary]) {
    for (const nsFirst of [false, true]) {
      const element = doc.createElementNS("http://www.w3.org/2000/svg", "script");
      element.setAttributeNS(ns, "xlink:href", "initial");
      element.getAttribute = element.getAttributeNS = () => { throw new Error("overridden getter"); };
      const attr = nsFirst ? element.getAttributeNodeNS(ns, "href")
        : element.getAttributeNode("xlink:href");
      const identity = attr === element.getAttributeNodeNS(ns, "href") &&
        attr === element.getAttributeNode("xlink:href") &&
        attr === element.attributes.item(0) &&
        attr === element.attributes.getNamedItem("xlink:href") &&
        attr === element.attributes.getNamedItemNS(ns, "href");
      const metadata = [attr.name, attr.localName, attr.prefix, attr.namespaceURI];
      delete element.getAttribute;
      delete element.getAttributeNS;
      element.removeAttributeNode(attr);
      element.setAttributeNode(attr);
      attr.value = "changed";
      results.push([metadata, identity, attr.ownerElement === element,
        element.getAttributeNS(ns, "href"), element.attributes.length]);
    }
  }
  return JSON.stringify(results);
})()
"#,
        )
        .expect("attribute node namespace checks should evaluate");
    let expected = serde_json::json!([
        [
            "xlink:href",
            "href",
            "xlink",
            "http://www.w3.org/1999/xlink"
        ],
        true,
        true,
        "changed",
        1
    ]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected, expected, expected])
    );
}

#[test]
fn attribute_node_cache_distinguishes_equal_qualified_names_in_different_namespaces() {
    let mut vm = new_storage_test_vm("https://attribute-node-cache-namespace.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const secondary = document.implementation.createHTMLDocument("");
  return JSON.stringify([document, secondary].map(doc => {
    const element = doc.createElementNS("http://www.w3.org/2000/svg", "g");
    element.setAttributeNS("urn:first", "same:name", "first");
    element.setAttributeNS("urn:second", "same:name", "second");
    const first = element.getAttributeNodeNS("urn:first", "name");
    const second = element.getAttributeNodeNS("urn:second", "name");
    const byName = element.getAttributeNode("same:name");
    const indexed = [element.attributes[0], element.attributes[1]];
    second.value = "updated";
    return [first !== second, byName === first, indexed[0] === first, indexed[1] === second,
      first.namespaceURI, second.namespaceURI, first.value, second.value];
  }));
})()
"#,
        )
        .expect("namespace cache identity checks should evaluate");
    let row = serde_json::json!([
        true,
        true,
        true,
        true,
        "urn:first",
        "urn:second",
        "first",
        "updated"
    ]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([row, row])
    );
}
