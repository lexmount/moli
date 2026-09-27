use super::*;

#[test]
fn range_insert_node_updates_collapsed_end_boundary_after_native_insert() {
    let mut vm = new_storage_test_vm("https://range-insert-node-collapsed-end.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  host.append('ab');
  (document.body || document.documentElement || document).appendChild(host);
  const text = host.firstChild;
  const range = document.createRange();
  range.setStart(text, 1);
  range.setEnd(text, 1);
  const marker = document.createElement('span');
  range.insertNode(marker);
  return [
    host.childNodes.length,
    host.childNodes[0].data,
    host.childNodes[1] === marker,
    host.childNodes[2].data,
    range.startContainer === text,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode collapsed text insertion probe should evaluate");

    assert_eq!(result, "3|a|true|b|true|1|true|2");
}

#[test]
fn range_insert_node_counts_document_fragment_children_for_collapsed_end() {
    let mut vm = new_storage_test_vm("https://range-insert-node-fragment-offset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  host.append(document.createElement('a'), document.createElement('d'));
  (document.body || document.documentElement || document).appendChild(host);
  const fragment = document.createDocumentFragment();
  fragment.append(document.createElement('b'), document.createElement('c'));
  const range = document.createRange();
  range.setStart(host, 1);
  range.setEnd(host, 1);
  range.insertNode(fragment);
  return [
    Array.from(host.childNodes, node => node.localName).join(''),
    fragment.childNodes.length,
    range.startContainer === host,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode document fragment offset probe should evaluate");

    assert_eq!(result, "abcd|0|true|1|true|3");
}

#[test]
fn range_insert_node_validates_before_splitting_text_for_document_rules() {
    let mut vm = new_storage_test_vm("https://range-insert-node-validation-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  host.append('abc');
  (document.body || document.documentElement || document).appendChild(host);
  const text = host.firstChild;
  const range = document.createRange();
  range.setStart(text, 1);
  range.setEnd(text, 1);
  const doctype = document.implementation.createDocumentType('html', '', '');
  let thrown = null;
  try {
    range.insertNode(doctype);
  } catch (error) {
    thrown = error;
  }
  return [
    thrown && thrown.name,
    text.data,
    text.parentNode === host,
    host.childNodes.length,
    range.startContainer === text,
    range.startOffset,
    range.endContainer === text,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode document validation order probe should evaluate");

    assert_eq!(result, "HierarchyRequestError|abc|true|1|true|1|true|1");
}

#[test]
fn range_insert_node_splits_cdata_start_container() {
    let mut vm = new_storage_test_vm("https://range-insert-node-cdata.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xml = document.implementation.createDocument(null, 'root');
  const cdata = xml.createCDATASection('abcd');
  xml.documentElement.appendChild(cdata);
  const range = xml.createRange();
  range.setStart(cdata, 2);
  range.setEnd(cdata, 2);
  const marker = xml.createElement('marker');
  range.insertNode(marker);
  return [
    xml.documentElement.childNodes.length,
    xml.documentElement.childNodes[0].data,
    xml.documentElement.childNodes[1] === marker,
    xml.documentElement.childNodes[2].data,
    range.startContainer === cdata,
    range.startOffset,
    range.endContainer === xml.documentElement,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode CDATA insertion probe should evaluate");

    assert_eq!(result, "3|ab|true|cd|true|2|true|2");
}

#[test]
fn range_insert_node_move_keeps_non_collapsed_boundary_after_moved_child() {
    let mut vm = new_storage_test_vm("https://range-insert-node-move-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('p');
  const text = document.createTextNode('abc');
  host.appendChild(text);
  (document.body || document.documentElement || document).appendChild(host);

  const range = document.createRange();
  range.setStart(host, 0);
  range.setEnd(host, 1);
  range.insertNode(text);

  return [
    host.childNodes.length,
    host.firstChild === text,
    range.startContainer === host,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode moving covered child should preserve the end boundary");

    assert_eq!(result, "1|true|true|0|true|1");
}

#[test]
fn range_insert_node_sets_end_after_move_collapses_current_range() {
    let mut vm = new_storage_test_vm("https://range-insert-node-current-collapse.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('p');
  const text = document.createTextNode('abcdefg');
  host.appendChild(text);
  (document.body || document.documentElement || document).appendChild(host);

  const range = document.createRange();
  range.setStart(host, 0);
  range.setEnd(text, 7);
  range.insertNode(text);

  return [
    host.childNodes.length,
    host.firstChild === text,
    range.startContainer === host,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode should repair the end after moving the end container");

    assert_eq!(result, "1|true|true|0|true|1");
}

#[test]
fn range_insert_node_move_comment_keeps_boundary_after_original_position() {
    let mut vm = new_storage_test_vm("https://range-insert-node-comment-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  for (let index = 0; index < 6; index++) {
    host.appendChild(document.createElement('p'));
  }
  const comment = document.createComment('Alphabet soup?');
  host.appendChild(comment);
  (document.body || document.documentElement || document).appendChild(host);

  const range = document.createRange();
  range.setStart(host, 0);
  range.setEnd(comment, 5);
  range.insertNode(comment);

  return [
    host.childNodes.length,
    host.firstChild === comment,
    range.startContainer === host,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode should keep moved comment boundary after original position");

    assert_eq!(result, "7|true|true|0|true|7");
}

#[test]
fn range_insert_node_move_foreign_text_keeps_boundary_after_original_position() {
    let mut vm = new_storage_test_vm("https://range-insert-node-foreign-text-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const foreignDoc = document.implementation.createHTMLDocument('');
  const first = foreignDoc.createElement('p');
  const second = foreignDoc.createElement('p');
  const text = foreignDoc.createTextNode('I admit that I harbor doubts about whether we really need so many things to test, but it is too late to stop now.');
  foreignDoc.body.appendChild(first);
  foreignDoc.body.appendChild(second);
  foreignDoc.body.appendChild(text);

  const range = foreignDoc.createRange();
  range.setStart(foreignDoc.body, 0);
  range.setEnd(text, 36);
  range.insertNode(text);

  return [
    foreignDoc.body.childNodes.length,
    foreignDoc.body.firstChild === text,
    range.startContainer === foreignDoc.body,
    range.startOffset,
    range.endContainer === foreignDoc.body,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode should keep moved foreign text boundary after original position");

    assert_eq!(result, "3|true|true|0|true|3");
}

#[test]
fn range_insert_node_rejects_document_doctype_self_move_before_reference_adjustment() {
    let mut vm = new_storage_test_vm("https://range-insert-node-doctype-self-move.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('');
  const doctype = doc.doctype;
  const range = doc.createRange();
  range.setStart(doc, 0);
  range.setEnd(doc, 1);

  let thrown = null;
  try {
    range.insertNode(doctype);
  } catch (error) {
    thrown = error;
  }

  return [
    !!doctype,
    thrown && thrown.name,
    thrown && thrown.code,
    doc.childNodes.length,
    range.endContainer === doc,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode should reject moving a document doctype before itself");

    assert_eq!(result, "true|HierarchyRequestError|3|2|true|1");
}

#[test]
fn range_insert_node_allows_comment_before_foreign_document_element() {
    let mut vm = new_storage_test_vm("https://range-insert-node-foreign-document-comment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probeDoc = document.implementation.createHTMLDocument('');
  const plainComment = document.createComment('plain');
  let plainThrown = null;
  try {
    probeDoc.insertBefore(plainComment, probeDoc.documentElement);
  } catch (error) {
    plainThrown = error.name;
  }

  const foreignDoc = document.implementation.createHTMLDocument('');
  const comment = document.createComment('Alphabet soup?');
  (document.body || document.documentElement || document).appendChild(comment);
  const range = foreignDoc.createRange();
  range.setStart(foreignDoc, 1);
  range.setEnd(foreignDoc, 1);
  let rangeThrown = null;
  try {
    range.insertNode(comment);
  } catch (error) {
    rangeThrown = error.name;
  }

  const xmlDoc = document.implementation.createDocument(null, null,
    document.implementation.createDocumentType('qorflesnorf', 'abcde', 'x'));
  const xmlElement = xmlDoc.createElement('root');
  xmlDoc.appendChild(xmlElement);
  const xmlComment = document.createComment('xml');
  const xmlRange = xmlDoc.createRange();
  xmlRange.setStart(xmlDoc, 1);
  xmlRange.setEnd(xmlDoc, 1);
  let xmlThrown = null;
  try {
    xmlRange.insertNode(xmlComment);
  } catch (error) {
    xmlThrown = error.name;
  }

  return [
    plainThrown,
    probeDoc.childNodes[1] === plainComment,
    rangeThrown,
    foreignDoc.childNodes[1] === comment,
    comment.ownerDocument === foreignDoc,
    range.startContainer === foreignDoc,
    range.startOffset,
    range.endContainer === foreignDoc,
    range.endOffset,
    xmlThrown,
    xmlDoc.childNodes[1] === xmlComment,
    xmlComment.ownerDocument === xmlDoc,
    xmlRange.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode should allow comments before a document element");

    assert_eq!(result, "|true||true|true|true|1|true|2||true|true|2");
}

#[test]
fn range_surround_contents_rejects_partially_selected_element_before_mutation() {
    let mut vm = new_storage_test_vm("https://range-surround-partial-invalid.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  const first = document.createElement('p');
  first.textContent = 'abc';
  const second = document.createElement('p');
  second.textContent = 'def';
  host.append(first, second);
  (document.body || document.documentElement || document).appendChild(host);
  const wrapper = document.createElement('section');
  wrapper.appendChild(document.createElement('old'));
  const range = document.createRange();
  range.setStart(first.firstChild, 1);
  range.setEnd(host, 2);

  let thrown = null;
  try {
    range.surroundContents(wrapper);
  } catch (error) {
    thrown = error;
  }

  return [
    thrown && thrown.name,
    thrown && thrown.code,
    host.childNodes.length,
    host.firstChild === first,
    first.firstChild.data,
    wrapper.firstChild && wrapper.firstChild.localName,
    range.startContainer === first.firstChild,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.surroundContents should validate partial non-Text selection first");

    assert_eq!(result, "InvalidStateError|11|2|true|abc|old|true|1|true|2");
}

#[test]
fn range_surround_contents_replaces_new_parent_children_and_selects_wrapper() {
    let mut vm = new_storage_test_vm("https://range-surround-success.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  const first = document.createElement('p');
  first.id = 'a';
  first.textContent = 'ab';
  const second = document.createElement('p');
  second.id = 'b';
  second.textContent = 'cd';
  host.append(first, second);
  (document.body || document.documentElement || document).appendChild(host);
  const old = document.createElement('old');
  const wrapper = document.createElement('section');
  wrapper.appendChild(old);
  const range = document.createRange();
  range.setStart(host, 0);
  range.setEnd(host, 2);

  range.surroundContents(wrapper);

  return [
    host.childNodes.length,
    host.firstChild === wrapper,
    Array.from(wrapper.childNodes, node => node.id).join(','),
    old.parentNode === null,
    range.startContainer === host,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.surroundContents should wrap extracted contents and select new parent");

    assert_eq!(result, "1|true|a,b|true|true|0|true|1");
}

#[test]
fn range_surround_contents_text_new_parent_fails_after_insert_step() {
    let mut vm = new_storage_test_vm("https://range-surround-text-parent-failure.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  const child = document.createElement('p');
  child.textContent = 'abc';
  host.appendChild(child);
  (document.body || document.documentElement || document).appendChild(host);
  const wrapper = document.createTextNode('wrapper');
  const range = document.createRange();
  range.setStart(host, 0);
  range.setEnd(host, 1);

  let thrown = null;
  try {
    range.surroundContents(wrapper);
  } catch (error) {
    thrown = error;
  }

  return [
    thrown && thrown.name,
    thrown && thrown.code,
    host.childNodes.length,
    host.firstChild === wrapper,
    child.parentNode && child.parentNode.nodeType,
    range.startContainer === host,
    range.startOffset,
    range.endContainer === host,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.surroundContents should preserve spec mutation order when append fails");

    assert_eq!(result, "HierarchyRequestError|3|1|true|11|true|0|true|1");
}

#[test]
fn range_surround_contents_uses_boundary_after_clearing_new_parent_children() {
    let mut vm = new_storage_test_vm("https://range-surround-ancestor-parent.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('div');
  const child = document.createElement('p');
  child.textContent = 'abc';
  wrapper.appendChild(child);
  const range = document.createRange();
  range.setStart(child.firstChild, 0);
  range.setEnd(child.firstChild, 0);

  let thrown = null;
  try {
    range.surroundContents(wrapper);
  } catch (error) {
    thrown = error;
  }

  return [
    thrown && thrown.name,
    thrown && thrown.code,
    wrapper.childNodes.length,
    child.parentNode === null,
    range.startContainer === wrapper,
    range.startOffset,
    range.endContainer === wrapper,
    range.endOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.surroundContents should use the live boundary after clearing newParent");

    assert_eq!(result, "HierarchyRequestError|3|0|true|true|0|true|0");
}

#[test]
fn replace_child_self_updates_live_range_boundaries_like_remove_then_insert() {
    let mut vm = new_storage_test_vm("https://replace-child-self-range.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement('div');
  parent.append(document.createElement('a'), document.createElement('b'), document.createElement('c'));
  (document.body || document.documentElement || document).appendChild(parent);
  const oldChild = parent.childNodes[1];
  oldChild.textContent = 'bc';

  const parentRange = document.createRange();
  parentRange.setStart(parent, 1);
  parentRange.setEnd(parent, 2);

  const childRange = document.createRange();
  childRange.setStart(oldChild.firstChild, 0);
  childRange.setEnd(oldChild.firstChild, 1);

  const returned = parent.replaceChild(oldChild, oldChild);
  return [
    returned === oldChild,
    parent.childNodes.length,
    parent.childNodes[1] === oldChild,
    parentRange.startContainer === parent,
    parentRange.startOffset,
    parentRange.endContainer === parent,
    parentRange.endOffset,
    childRange.startContainer === parent,
    childRange.startOffset,
    childRange.endContainer === parent,
    childRange.endOffset
  ].join('|');
})()
"#,
        )
        .expect("replaceChild(oldChild, oldChild) should update ranges like remove then insert");

    assert_eq!(result, "true|3|true|true|1|true|1|true|1|true|1");
}

#[test]
fn child_document_stream_slots_ignore_page_tampering() {
    let mut vm = new_storage_test_vm("https://child-stream-slot-tamper.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(iframe);
  const doc = iframe.contentDocument;
  doc.open();
  doc.write('<script>window.__count = (window.__count || 0) + 1;</script>');
  doc.__lmChildDocumentExecutedScriptOffset = 0;
  doc.__lmChildDocumentPendingWrite = '<p id="evil"></p>';
  doc.write('<p id="safe"></p>');
  return [
    iframe.contentWindow.__count,
    !!doc.getElementById('safe'),
    !!doc.getElementById('evil'),
    Object.prototype.propertyIsEnumerable.call(doc, '__lmChildDocumentPendingWrite'),
    Object.prototype.propertyIsEnumerable.call(doc, '__lmChildDocumentExecutedScriptOffset')
  ].join('|');
})()
"#,
        )
        .expect("child document stream internals should ignore page-owned slots");

    assert_eq!(result, "1|true|false|true|true");
}
