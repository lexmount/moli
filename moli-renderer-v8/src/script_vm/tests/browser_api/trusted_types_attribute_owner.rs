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
  ];
  globalThis.__attributeReports = { top: [], child: [] };
  // Observe the receiving global independently of the event's DOM target.
  window.addEventListener("securitypolicyviolation", event => __attributeReports.top.push(event.sample));
  child.addEventListener("securitypolicyviolation", event => __attributeReports.child.push(event.sample));
  const moveAndSet = (source, target, set) => {
    const element = source.createElement("button");
    element.setAttribute("onclick", explicit.createScript("initial"));
    target.adoptNode(element);
    // Like the adoption WPT, derive the exception realm from the exposed
    // receiver, without assuming that adoption keeps the original wrapper realm.
    const receiver = element;
    const expectedError = new receiver.constructor.constructor(explicit.createScript("return TypeError"))();
    let outcome = "none";
    try { set(element, "input"); } catch (error) {
      outcome = `${error.name}:${error instanceof expectedError}`;
    }
    return [outcome, element.ownerDocument === target, element.getAttribute("onclick")];
  };
  const intoPlain = setters.map(set => moveAndSet(childDoc, document, set));
  const intoEnforced = setters.map(set => moveAndSet(document, childDoc, set));
  const intoSecondary = setters.map(set => moveAndSet(document, secondary, set));
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
  const defaulted = setters.map(set => moveAndSet(document, childDoc, set));
  const secondaryDefaulted = setters.map(set => moveAndSet(document, secondary, set));
  return JSON.stringify({ intoPlain, intoEnforced, intoSecondary, stringConversions, defaulted, secondaryDefaulted, calls });
})()
"#).expect("adopted attribute sink checks should evaluate");
    let allowed = vec![serde_json::json!(["none", true, "input"]); 2];
    let blocked = vec![serde_json::json!(["TypeError:true", true, "initial"]); 2];
    let converted = vec![serde_json::json!(["none", true, "child-input"]); 2];
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "intoPlain": allowed, "intoEnforced": blocked, "intoSecondary": blocked,
        "stringConversions": [[true, 1, true, false], [true, 1, true, false]],
        "defaulted": converted, "secondaryDefaulted": converted, "calls": vec!["child"; 4]
        })
    );
    drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify([__attributeReports.top.length, __attributeReports.child.length])")
            .unwrap(),
        "[0,6]"
    );
}
