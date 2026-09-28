use super::*;

#[test]
fn selection_and_gamepad_handlers_dispatch_in_each_window() {
    let mut vm = new_storage_test_vm("https://event-handler-surface.test/");
    let result = vm
        .eval(include_str!("surface.js"))
        .expect("selection and gamepad handler regression should evaluate");
    assert_eq!(result, "true");
}

#[test]
fn selection_handlers_respect_windowless_scripting_state() {
    let mut vm = new_parsed_test_vm(
        "https://selection-handler-attributes.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const detached = document.implementation.createHTMLDocument('');
  for (const d of [document, detached]) {
    for (const [namespace, tag] of [
      ['http://www.w3.org/1999/xhtml', 'div'],
      ['http://www.w3.org/2000/svg', 'svg'],
      ['http://www.w3.org/1998/Math/MathML', 'math']
    ]) {
      const element = d.createElementNS(namespace, tag);
      if (tag === 'math' && !(element instanceof MathMLElement)) throw new Error('MathML interface');
      element.setAttribute('onselectstart', 'this.setAttribute("called", event.type); return false');
      if (d === detached) {
        if (element.onselectstart !== null ||
            !element.dispatchEvent(new Event('selectstart', {cancelable:true})) ||
            element.hasAttribute('called')) throw new Error(tag + ': windowless content handler');
        element.onselectstart = function(event) {
          this.setAttribute('called', event.type);
          return false;
        };
      }
      const event = new Event('selectstart', {cancelable:true});
      if (element.dispatchEvent(event) !== false ||
          element.getAttribute('called') !== 'selectstart' ||
          typeof element.onselectstart !== 'function') throw new Error(tag + ': dispatch');
      element.removeAttribute('onselectstart');
      if (element.onselectstart !== null ||
          !element.dispatchEvent(new Event('selectstart', {cancelable:true}))) throw new Error(tag + ': removed');
    }
  }
  return true;
})()
"#,
        )
        .expect("detached selectstart content handlers should evaluate");
    assert_eq!(result, "true");
}
