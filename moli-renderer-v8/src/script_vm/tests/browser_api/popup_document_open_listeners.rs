use super::*;

#[test]
fn popup_document_open_clears_shadow_tree_listeners_but_keeps_detached_nodes() {
    let mut vm = new_storage_test_vm("https://popup-document-open-listeners.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document, calls = [];
  try {
    const host = d.body.appendChild(d.createElement('div'));
    const shadow = host.attachShadow({mode:'closed'});
    const nested = shadow.appendChild(d.createElement('div'));
    const inner = nested.attachShadow({mode:'open'});
    const leaf = inner.appendChild(d.createElement('span'));
    const detached = d.createElement('button');
    const template = d.body.appendChild(d.createElement('template'));
    const content = template.content.appendChild(d.createElement('button'));
    const nodes = [d, d.documentElement, d.head, d.body, host, shadow, nested, inner, leaf, template];
    for (let i = 0; i < nodes.length; ++i) {
      nodes[i].addEventListener('click', () => calls.push('tree' + i));
    }
    leaf.onclick = () => calls.push('old-handler');
    for (const [node, label] of [[detached, 'detached'], [content, 'template']]) {
      node.addEventListener('click', () => calls.push(label));
      node.onclick = () => calls.push(label + '-handler');
    }
    d.open();
    for (const node of [...nodes, detached, content]) node.dispatchEvent(new Event('click'));
    return JSON.stringify({calls:calls.sort(), leafHandler:leaf.onclick === null});
  } finally { w.close(); }
})()
"#,
        )
        .expect("popup document.open should erase only its shadow-including document tree");
    assert_eq!(
        result,
        r#"{"calls":["detached","detached-handler","template","template-handler"],"leafHandler":true}"#
    );
}

#[test]
fn popup_document_open_clears_its_window_without_clearing_other_windows() {
    let mut vm = new_storage_test_vm("https://popup-document-open-window-listeners.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), other = open(), d = w.document, calls = [];
  function root() { calls.push('opener'); }
  window.addEventListener('click', root);
  try {
    other.addEventListener('click', () => calls.push('other'));
    other.document.addEventListener('click', () => calls.push('other-document'));
    w.addEventListener('click', () => calls.push('old-window'));
    w.onclick = () => calls.push('old-handler');
    d.addEventListener('click', () => calls.push('old-document'));
    const body = d.body;
    body.onfocus = () => calls.push('old-focus');
    other.document.open.call(d);
    for (const target of [w, d, other, other.document, window]) {
      target.dispatchEvent(new Event('click'));
    }
    w.dispatchEvent(new Event('focus'));
    return JSON.stringify({calls, click:w.onclick === null,
      focus:w.onfocus === null, bodyFocus:body.onfocus === null});
  } finally { window.removeEventListener('click', root); w.close(); other.close(); }
})()
"#,
        )
        .expect("borrowed popup open should clear the receiver's Window only");
    assert_eq!(
        result,
        r#"{"calls":["other","other-document","opener"],"click":true,"focus":true,"bodyFocus":true}"#
    );
}

#[test]
fn popup_document_open_deactivates_content_handlers_and_bypasses_public_setters() {
    let mut vm = new_storage_test_vm("https://popup-document-open-content-handlers.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document, calls = [];
  try {
    const body = d.body;
    const compiled = body.appendChild(d.createElement('button'));
    const pending = body.appendChild(d.createElement('button'));
    const source = 'this.setAttribute("fired", "yes")';
    compiled.setAttribute('onclick', source);
    compiled.onclick = () => calls.push('old-compiled-handler');
    pending.setAttribute('onclick', source);
    const wasCompiled = typeof compiled.onclick === 'function';
    body.setAttribute('onfocus', 'window.retiredHandlerRan = true');
    d.onclick = () => calls.push('old-document');
    w.onclick = () => calls.push('old-window');
    let sets = 0;
    for (const target of [d, w]) {
      Object.defineProperty(target, 'onclick', {configurable:true, set() {
        ++sets;
        throw new Error('author setter');
      }});
    }
    d.open();
    for (const target of [compiled, pending, d, w]) target.dispatchEvent(new Event('click'));
    w.dispatchEvent(new Event('focus'));
    return JSON.stringify({calls, sets, wasCompiled,
      attributesKept:compiled.getAttribute('onclick') === source && pending.getAttribute('onclick') === source,
      handlersCleared:compiled.onclick === null && pending.onclick === null && w.onfocus === null,
      didNotRun:!compiled.hasAttribute('fired') && !pending.hasAttribute('fired') && w.retiredHandlerRan === undefined});
  } finally { w.close(); }
})()
"#,
        )
        .expect("popup open should erase native handlers without invoking page setters");
    assert_eq!(
        result,
        r#"{"calls":[],"sets":0,"wasCompiled":true,"attributesKept":true,"handlersCleared":true,"didNotRun":true}"#
    );
}
