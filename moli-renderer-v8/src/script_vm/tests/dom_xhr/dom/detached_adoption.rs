use super::*;

#[test]
fn dom_parser_append_is_atomic_when_later_argument_is_invalid() {
    let mut vm = new_storage_test_vm("https://dom-parser-append-atomic.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<!doctype html><html><body></body></html>', 'text/html');
  try {
    doc.body.append('before', document);
  } catch (e) {
  }
  return [
    doc.body.childNodes.length,
    doc.body.textContent
  ].join('|');
})()
"#,
        )
        .expect("DOMParser append should not partially mutate on later invalid args");

    assert_eq!(result, "0|");
}

#[test]
fn detached_document_head_body_only_match_document_element_children() {
    let mut vm = new_storage_test_vm("https://detached-head-body-direct-children.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<!doctype html><html><head></head><body></body></html>', 'text/html');
  doc.removeChild(doc.documentElement);
  const html = doc.createElement('html');
  const body = doc.createElement('body');
  const section = doc.createElement('section');
  const nestedHead = doc.createElement('head');
  section.appendChild(nestedHead);
  body.appendChild(section);
  html.appendChild(body);
  doc.appendChild(html);
  return [
    doc.head === null,
    nestedHead.localName,
    doc.body.localName
  ].join('|');
})()
"#,
        )
        .expect("detached document head/body should only use direct html children");

    assert_eq!(result, "true|head|body");
}

#[test]
fn child_content_document_exposes_document_node_mutation_methods() {
    let mut vm = new_storage_test_vm("https://child-content-document-node-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(iframe);
  const doc = iframe.contentDocument;
  const root = doc.documentElement;
  const before = [
    typeof doc.removeChild,
    typeof doc.appendChild,
    typeof doc.insertBefore,
    typeof doc.replaceChild,
    !!root
  ].join(',');

  const removed = doc.removeChild(root);
  const afterRemove = [
    removed === root,
    doc.documentElement === null,
    doc.body === null,
    root.parentNode === null,
    root.isConnected === false
  ].join(',');

  const replacement = root.cloneNode(true);
  doc.appendChild(replacement);
  const afterAppend = [
    doc.documentElement === replacement,
    doc.body === replacement.querySelector('body'),
    doc.lastChild === replacement,
    replacement.parentNode === doc,
    replacement.isConnected === true
  ].join(',');

  return `${before}|${afterRemove}|${afterAppend}`;
})()
"#,
        )
        .expect("child contentDocument should expose document node mutation methods");

    assert_eq!(
        result,
        "function,function,function,function,true|true,true,true,true,true|true,true,true,true,true"
    );
}

#[test]
fn child_window_exports_script_globals_even_when_script_throws() {
    let mut vm = new_storage_test_vm("https://child-script-throw-export.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(iframe);
  const win = iframe.contentWindow;
  const doc = iframe.contentDocument;
  doc.open();
  doc.write('<script>function keptFunction() { return this === window ? 7 : 0; } var keptVar = 3; throw new Error("boom");</script>');
  doc.close();
  return [
    typeof win.keptFunction,
    win.keptFunction(),
    win.keptVar
  ].join('|');
})()
"#,
        )
        .expect("child script globals should be exported before a runtime throw escapes");

    assert_eq!(result, "function|7|3");
}

#[test]
fn detached_dom_parser_nodes_keep_relationship_state_after_node_prototype_migration() {
    let mut vm = new_storage_test_vm("https://detached-dom-node-prototype-shadowing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><p id="a"></p><span id="b"></span></body></html>',
    'text/html'
  );
  const body = doc.body;
  const a = doc.getElementById('a');
  const b = doc.getElementById('b');
  const div = doc.createElement('div');

  const before = [
    Object.prototype.hasOwnProperty.call(body, 'childNodes'),
    Object.prototype.hasOwnProperty.call(body, 'firstChild'),
    Object.prototype.hasOwnProperty.call(body, 'lastChild'),
    Object.prototype.hasOwnProperty.call(a, 'parentNode'),
    Object.prototype.hasOwnProperty.call(a, 'nextSibling'),
    Object.prototype.hasOwnProperty.call(b, 'previousSibling'),
    Object.prototype.toString.call(body.childNodes),
    Array.from(body.childNodes).map(node => node.id).join(','),
    Object.prototype.toString.call(div.childNodes),
    Array.from(div.childNodes).length,
    body.firstChild === a,
    body.lastChild === b,
    a.parentNode === body,
    a.nextSibling === b,
    b.previousSibling === a
  ].join('|');

  body.insertBefore(div, b);
  const text = doc.createTextNode('x');
  div.appendChild(text);

  const after = [
    Array.from(body.childNodes).map(node => node.id || node.nodeName).join(','),
    body.firstChild === a,
    body.lastChild === b,
    div.parentNode === body,
    Object.prototype.toString.call(div.childNodes),
    Array.from(div.childNodes).length,
    div.firstChild === text,
    div.lastChild === text,
    text.parentNode === div
  ].join('|');

  return `${before}||${after}`;
})()
"#,
        )
        .expect("detached DOMParser nodes should retain own relationship state");

    assert_eq!(
        result,
        "false|false|false|false|false|false|[object NodeList]|a,b|[object NodeList]|0|true|true|true|true|true||a,DIV,b|true|true|true|[object NodeList]|1|true|true|true"
    );
}

#[test]
fn detached_document_rejects_overdeep_live_subtree_adoption() {
    run_large_stack_dom_test("detached-adopt-depth", || {
        let mut vm = new_storage_test_vm("https://detached-adopt-depth.test/");

        let result = vm
            .eval(
                r#"
(() => {
  const detached = document.implementation.createDocument(null, 'container', null);
  const root = document.createElement('div');
  let cursor = root;
  for (let i = 0; i < 514; i++) {
    const child = document.createElement('div');
    cursor.appendChild(child);
    cursor = child;
  }
  try {
    detached.documentElement.appendChild(root);
    return `inserted:${detached.documentElement.childNodes.length}`;
  } catch (error) {
    return `${error.name}:${detached.documentElement.childNodes.length}`;
  }
})()
"#,
            )
            .expect("overdeep detached live subtree adoption should return a bounded result");

        assert_eq!(result, "HierarchyRequestError:0");
    });
}
