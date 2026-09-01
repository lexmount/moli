use super::*;

#[test]
fn query_selector_all_preserves_isolated_world_node_identity() {
    for universal_access in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://query-selector-realms.test/",
            "<!doctype html><html><body></body></html>",
        );
        let context_id = vm
            .create_isolated_world("query-selector-identity", universal_access)
            .expect("isolated query realm should be created");
        let result = vm
            .eval_in_isolated_context(
                context_id,
                r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const fragment = document.createDocumentFragment();
  const detached = document.createElement('section');
  const shadow = document.body.appendChild(document.createElement('div')).attachShadow({mode:'open'});
  for (const root of [document.body, fragment, detached, shadow]) {
    for (let index = 0; index < 1100; ++index) {
      const node = document.createElement('span');
      node.className = index === 0 ? 'hit first' : 'hit';
      root.appendChild(node);
    }
  }
  for (const root of [document, document.body, fragment, detached, shadow]) {
    for (const selector of ['.first', '.hit']) {
      const first = root.querySelector(selector);
      const list = root.querySelectorAll(selector);
      assert(list[0] === first, 'querySelector and querySelectorAll must return the same wrapper');
      assert(Object.getPrototypeOf(list) === NodeList.prototype, 'NodeList belongs to the isolated receiver realm');
      assert(list.item(0) === first && [...list][0] === first, 'item and iteration preserve node identity');
      assert(list !== root.querySelectorAll(selector), 'each query creates a fresh static NodeList');
      const length = list.length;
      first.remove();
      assert(list.length === length && list[0] === first, 'removed nodes remain in the static result');
      root === document ? document.body.prepend(first) : root.prepend(first);
    }
  }
  return 'ok';
})()
"#,
            )
            .expect("isolated queries must not fall back to the main world");
        assert_eq!(result, "ok", "universal_access={universal_access}");
    }
}

#[test]
fn query_selector_all_borrowed_methods_keep_the_receiver_realm() {
    let mut vm = new_parsed_test_vm(
        "https://query-selector-borrowed.test/",
        "<!doctype html><html><body><span class=hit></span></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const frame = document.body.appendChild(document.createElement('iframe'));
  const child = frame.contentWindow;
  child.document.body.appendChild(child.document.createElement('span')).className = 'hit';
  for (const [owner, caller] of [[window, child], [child, window]]) {
    const doc = owner.document;
    const fragment = doc.createDocumentFragment();
    fragment.appendChild(doc.createElement('span')).className = 'hit';
    const shadow = doc.body.appendChild(doc.createElement('div')).attachShadow({mode:'open'});
    shadow.appendChild(doc.createElement('span')).className = 'hit';
    for (const [root, method] of [
      [doc, caller.Document.prototype.querySelectorAll],
      [doc.body, caller.Element.prototype.querySelectorAll],
      [fragment, caller.DocumentFragment.prototype.querySelectorAll],
      [shadow, caller.DocumentFragment.prototype.querySelectorAll],
    ]) {
      const list = method.call(root, '.hit');
      assert(Object.getPrototypeOf(list) === owner.NodeList.prototype, 'result uses receiver realm, not borrowed method realm');
      assert(list[0] === root.querySelector('.hit'), 'borrowed query preserves native node identity');
      assert(list.item(0) === list[0] && [...list][0] === list[0], 'all collection access paths agree');
    }
  }
  return 'ok';
})()
"#,
        )
        .expect("querySelectorAll must retain the receiver realm across borrowed methods");
    assert_eq!(result, "ok");
}
