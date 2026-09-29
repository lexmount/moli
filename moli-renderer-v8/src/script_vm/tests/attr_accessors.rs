use super::*;

#[test]
fn attr_accessors_share_branded_prototypes_and_node_value_operations() {
    let mut vm = new_parsed_test_vm(
        "https://attr-accessors.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("attr_accessors.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__attrAccessorFailures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn attr_value_setters_follow_the_attribute_moved_by_the_default_policy() {
    let mut vm = new_parsed_test_vm(
        "https://attr-default-policy-reentry.test/",
        "<!doctype html><body><iframe id=child></iframe></body>",
    );
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);
    let result = vm
        .eval(
            r#"
(() => {
  let mutation;
  const policyInputs = [];
  const trusted = trustedTypes.createPolicy('initial', {createScript: value => value});
  trustedTypes.createPolicy('default', {
    createScript(value) {
      const pending = mutation;
      mutation = undefined;
      if (!pending) return value;
      policyInputs.push(value);
      pending();
      return 'safe-' + value;
    }
  });
  const documents = [document, document.getElementById('child').contentDocument,
    document.implementation.createHTMLDocument(''),
    document.implementation.createDocument(null, 'root')];
  const results = [];
  for (const targetDocument of documents) {
    for (const member of ['value', 'nodeValue', 'textContent']) {
      const source = document.createElement('button');
      const target = targetDocument.createElementNS('http://www.w3.org/1999/xhtml', 'button');
      source.setAttribute('onclick', trusted.createScript('initial'));
      const attr = source.getAttributeNode('onclick');
      mutation = () => {
        source.removeAttributeNode(attr);
        target.setAttributeNode(attr);
      };
      attr[member] = 'input';
      results.push([source.hasAttribute('onclick'), attr.ownerElement === target,
        attr.ownerDocument === targetDocument, target.getAttributeNode('onclick') === attr,
        attr.value, target.getAttribute('onclick')]);
    }
  }
  return JSON.stringify({results, policyInputs});
})()
"#,
        )
        .expect("default policy attribute movement should evaluate");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "results": vec![serde_json::json!([false, true, true, true, "safe-input", "safe-input"]); 12],
            "policyInputs": vec!["input"; 12],
        })
    );
}
