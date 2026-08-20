use super::*;

fn run_large_stack_dom_test<F>(thread_name: &'static str, test: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(test)
        .expect("large-stack DOM test thread should spawn")
        .join()
        .expect("large-stack DOM test thread should finish");
}

#[test]
fn child_document_create_text_and_comment_nodes_work() {
    let mut vm = new_storage_test_vm("https://child-window-detached-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<body></body>';
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  const text = doc.createTextNode('hello');
  const comment = doc.createComment('note');
  doc.body.appendChild(text);
  doc.body.appendChild(comment);
  return [
    typeof doc.createTextNode,
    text.nodeType,
    text.nodeName,
    text.data,
    text.parentNode === doc.body,
    typeof doc.createComment,
    comment.nodeType,
    comment.nodeName,
    comment.data,
    doc.body.childNodes.length
  ].join('|');
})()
"#,
        )
        .expect("detached child document text/comment nodes should work");

    assert_eq!(
        result,
        "function|3|#text|hello|true|function|8|#comment|note|2"
    );
}
#[test]
fn detached_character_data_accessors_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://detached-character-data-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const text = doc.createTextNode("seed");
  const comment = doc.createComment("note");
  const probe = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return "throw:" + error.name;
    }
  };
  text.data = { toString() { return "updated"; } };
  const dataSymbol = probe(() => { text.data = Symbol("data"); });
  text.nodeValue = { toString() { return "node-value"; } };
  const nodeSymbol = probe(() => { comment.nodeValue = Symbol("nodeValue"); });
  comment.data = null;
  return [
    text.data,
    text.nodeValue,
    dataSymbol,
    comment.data,
    nodeSymbol
  ].join("|");
})()
"#,
        )
        .expect("detached character data WebIDL accessors should evaluate");

    assert_eq!(
        result,
        "node-value|node-value|throw:TypeError||throw:TypeError"
    );
}
#[test]
fn live_node_replace_child_rejects_non_child_with_not_found_error() {
    let mut vm = new_storage_test_vm("https://live-node-replace-not-found.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement('div');
  const oldChild = document.createElement('span');
  const newChild = document.createElement('b');
  (document.body || document.documentElement || document).appendChild(parent);
  try {
    parent.replaceChild(newChild, oldChild);
    return 'missing';
  } catch (error) {
    return [
      error.name,
      error.code,
      parent.childNodes.length,
      newChild.parentNode === null,
      oldChild.parentNode === null
    ].join('|');
  }
})()
"#,
        )
        .expect("live replaceChild should reject a non-child oldChild");

    assert_eq!(result, "NotFoundError|8|0|true|true");
}
#[test]
fn detached_document_fragment_inserts_children_and_empties_fragment() {
    let mut vm = new_storage_test_vm("https://detached-document-fragment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><p id="end"></p></body></html>',
    'text/html'
  );
  const fragment = doc.createDocumentFragment();
  const first = doc.createElement('a');
  first.id = 'first';
  const text = doc.createTextNode('text');
  const second = doc.createElement('b');
  second.id = 'second';
  fragment.appendChild(first);
  fragment.appendChild(text);
  fragment.appendChild(second);
  const beforeInsertConnected = [
    fragment.isConnected,
    first.isConnected,
    text.isConnected,
    second.isConnected
  ].join(',');
  const returned = doc.body.insertBefore(fragment, doc.getElementById('end'));
  return [
    typeof doc.createDocumentFragment,
    Object.prototype.toString.call(fragment),
    fragment instanceof DocumentFragment,
    fragment.nodeType,
    fragment.nodeName,
    beforeInsertConnected,
    returned === fragment,
    fragment.childNodes.length,
    fragment.children.length,
    fragment.firstChild === null,
    doc.body.childNodes.length,
    doc.body.children.length,
    doc.body.firstChild === first,
    first.nextSibling === text,
    text.nextSibling === second,
    second.nextSibling.id,
    second.previousSibling === text,
    first.parentNode === doc.body,
    second.parentNode === doc.body,
    first.isConnected,
    text.isConnected,
    second.isConnected
  ].join('|');
})()
"#,
        )
        .expect("detached DocumentFragment should insert children and empty itself");

    assert_eq!(
        result,
        "function|[object DocumentFragment]|true|11|#document-fragment|false,false,false,false|true|0|0|true|4|3|true|true|true|end|true|true|true|true|true|true"
    );
}
#[test]
fn document_fragment_constructor_uses_live_document_fragment_semantics() {
    let mut vm = new_storage_test_vm("https://document-fragment-constructor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const constructed = new DocumentFragment();
  const factory = document.createDocumentFragment();

  function run(label, fragment) {
    const node = document.createElement('div');
    node.id = label;
    fragment.appendChild(node);
    const afterFragment = node.parentNode && node.parentNode.nodeName;
    document.body.appendChild(fragment);
    const afterBody = node.parentNode && node.parentNode.nodeName;
    const sameById = document.getElementById(label) === node;
    const contains = document.body.contains(node);
    const removed = node.parentNode.removeChild(node);
    return [
      afterFragment,
      afterBody,
      sameById,
      contains,
      removed === node,
      node.parentNode === null,
      fragment.childNodes.length
    ].join(',');
  }

  return [
    Object.prototype.toString.call(constructed),
    constructed instanceof DocumentFragment,
    run('ctor-fragment-child', constructed),
    run('factory-fragment-child', factory)
  ].join('|');
})()
"#,
        )
        .expect("DocumentFragment constructor should match document factory semantics");

    assert_eq!(
        result,
        "[object DocumentFragment]|true|#document-fragment,BODY,true,true,true,true,0|#document-fragment,BODY,true,true,true,true,0"
    );
}
#[test]
fn document_fragment_constructor_falls_back_cleanly_when_live_getter_throws() {
    let mut vm = new_storage_test_vm("https://document-fragment-constructor-throw.test/");

    let result = vm
        .eval(
            r#"
(() => {
  Object.defineProperty(document, 'createDocumentFragment', {
    configurable: true,
    get() {
      throw new Error('getter boom');
    }
  });
  const fragment = new DocumentFragment();
  const child = document.createElement('span');
  child.id = 'fallback-fragment-child';
  fragment.appendChild(child);
  return [
    Object.prototype.toString.call(fragment),
    fragment instanceof DocumentFragment,
    fragment.nodeType,
    fragment.nodeName,
    fragment.ownerDocument === document,
    fragment.firstChild === child,
    fragment.childNodes.length
  ].join('|');
})()
"#,
        )
        .expect("DocumentFragment constructor fallback should clear thrown getter state");

    assert_eq!(
        result,
        "[object DocumentFragment]|true|11|#document-fragment|true|true|1"
    );
}
#[test]
fn detached_doctype_append_preserves_parent_and_owner_document() {
    let mut vm = new_storage_test_vm("https://detached-doctype-parent.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createDocument(null, "", null);
  const doctype = doc.implementation.createDocumentType("html", "", "");
  const appended = doc.appendChild(doctype);
  const before = [
    appended === doctype,
    doctype.parentNode === doc,
    doctype.ownerDocument === doc,
    String(doctype.parentNode),
    String(doctype.ownerDocument),
    doc.childNodes.length,
    doc.childNodes[0] === doctype,
    String(doc.childNodes[0]),
    doc.firstChild === doctype,
    String(doc.firstChild)
  ].join(",");
  doctype.remove();
  const after = [
    doctype.parentNode === null,
    doctype.ownerDocument === doc,
    String(doctype.parentNode),
    String(doctype.ownerDocument)
  ].join(",");
  return before + "|" + after;
})()
"#,
        )
        .expect("detached doctype parent/owner probe should evaluate");

    assert_eq!(
        result,
        "true,true,true,[object XMLDocument],[object XMLDocument],1,true,[object DocumentType],true,[object DocumentType]|true,true,null,[object XMLDocument]"
    );
}
#[test]
fn detached_node_remove_child_and_remove_refresh_parent_surface() {
    let mut vm = new_storage_test_vm("https://detached-node-remove-child.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><p id="a"></p><b id="b"></b><i id="c"><span id="nested"></span></i></body></html>',
    'text/html'
  );
  const a = doc.getElementById('a');
  const b = doc.getElementById('b');
  const c = doc.getElementById('c');
  const nested = doc.getElementById('nested');
  const removed = doc.body.removeChild(b);
  c.remove();
  return [
    typeof doc.body.removeChild,
    removed === b,
    b.parentNode === null,
    b.previousSibling === null,
    b.nextSibling === null,
    b.isConnected,
    c.parentNode === null,
    c.isConnected,
    nested.isConnected,
    doc.body.childNodes.length,
    doc.body.children.length,
    doc.body.firstChild === a,
    doc.body.lastChild === a,
    a.previousSibling === null,
    a.nextSibling === null,
    doc.body.children.item(0) === a,
    doc.body.children.item(1) === null
  ].join('|');
})()
"#,
        )
        .expect("detached removeChild/remove should refresh parent surface");

    assert_eq!(
        result,
        "function|true|true|true|true|false|true|false|false|1|1|true|true|true|true|true|true"
    );
}
#[test]
fn element_insert_adjacent_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://insert-adjacent-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const root = document.createElement('div');
  const span = document.createElement('span');
  span.textContent = 'element';
  const returned = root.insertAdjacentElement({ toString() { return 'beforeend'; } }, span);
  root.insertAdjacentText({ toString() { return 'beforeend'; } }, {
    toString() {
      return 'text';
    }
  });
  root.insertAdjacentText('beforeend', undefined);
  root.insertAdjacentHTML('beforeend', {
    toString() {
      return '<b id="inserted">bold</b>';
    }
  });
  return [
    returned === span,
    root.textContent,
    root.querySelector('#inserted').textContent,
    probe(() => root.insertAdjacentText()),
    probe(() => root.insertAdjacentText(Symbol(), 'x')),
    probe(() => root.insertAdjacentText({
      toString() {
        throw new RangeError('position');
      }
    }, 'x')),
    probe(() => root.insertAdjacentText('beforeend', Symbol())),
    probe(() => root.insertAdjacentText('sideways', 'x')),
    probe(() => root.insertAdjacentHTML('beforeend')),
    probe(() => root.insertAdjacentHTML('beforeend', Symbol())),
    probe(() => root.insertAdjacentElement('beforeend')),
    probe(() => root.insertAdjacentElement(Symbol(), span))
  ].join('|');
})()
"#,
        )
        .expect("insertAdjacent methods should parse WebIDL arguments");

    assert_eq!(
        result,
        "true|elementtextundefinedbold|bold|throw:TypeError|throw:TypeError|throw:RangeError|throw:TypeError|throw:SyntaxError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn document_fragment_and_shadow_root_get_element_by_id_match_browser_lookup_boundaries() {
    let mut vm = new_storage_test_vm("https://fragment-shadow-get-by-id.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.createElement('div');
  document.body.appendChild(host);

  const fragment = document.createDocumentFragment();
  const outer = document.createElement('section');
  const inner = document.createElement('span');
  outer.id = 'outer';
  inner.id = 'inside-fragment';
  outer.appendChild(inner);
  fragment.appendChild(outer);

  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<div id="shadow-target"><span id="shadow-nested"></span></div>';

  return [
    typeof fragment.getElementById,
    fragment.getElementById('inside-fragment') === inner,
    fragment.getElementById('outer') === outer,
    fragment.getElementById('missing') === null,
    typeof shadow.getElementById,
    shadow.getElementById('shadow-target')?.id,
    shadow.getElementById('shadow-nested')?.id,
    document.getElementById('shadow-target') === null
  ].join('|');
})()
"#,
        )
        .expect("DocumentFragment and ShadowRoot getElementById should resolve subtree ids");

    assert_eq!(
        result,
        "function|true|true|true|function|shadow-target|shadow-nested|true"
    );
}
#[test]
fn detached_html_document_shadow_root_queries_respect_tree_boundaries() {
    let mut vm = new_storage_test_vm("https://detached-shadow-boundaries.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('Test');
  const light = doc.createElement('p');
  light.id = 'test-id';
  light.className = 'test-class';
  doc.body.appendChild(light);

  const shadow = doc.body.attachShadow({ mode: 'open' });
  const shadowP = doc.createElement('p');
  shadowP.id = 'test-id';
  shadowP.className = 'test-class';
  shadow.appendChild(shadowP);

  const closedHost = doc.createElement('div');
  doc.body.appendChild(closedHost);
  const closed = closedHost.attachShadow({ mode: 'closed' });
  const prototypeMethodShape = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      Object.prototype.hasOwnProperty.call(shadow, name),
      !!descriptor,
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length,
      descriptor.writable,
      descriptor.configurable
    ].join(':');
  };
  const prototypeAccessorShape = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      Object.prototype.hasOwnProperty.call(shadow, name),
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable,
      shadow[name] === doc.body
    ].join(':');
  };
  const inheritedMethodShape = name =>
    [
      Object.prototype.hasOwnProperty.call(shadow, name),
      typeof shadow[name]
    ].join(':');

  return [
    shadow instanceof ShadowRoot,
    shadow.parentNode === null,
    shadow.parentElement === null,
    shadow.host === doc.body,
    doc.body.shadowRoot === shadow,
    closedHost.shadowRoot === null,
    closed.host === closedHost,
    doc.querySelector('p') === light,
    doc.querySelector('.test-class') === light,
    doc.querySelector('#test-id') === light,
    doc.querySelectorAll('p').length,
    shadow.querySelector('p') === shadowP,
    shadow.querySelector('.test-class') === shadowP,
    shadow.querySelector('#test-id') === shadowP,
    shadow.querySelectorAll('p').length,
    shadow.getElementById('test-id') === shadowP,
    prototypeAccessorShape(ShadowRoot.prototype, 'host'),
    prototypeMethodShape(ShadowRoot.prototype, 'getSelection'),
    inheritedMethodShape('cloneNode')
  ].join('|');
})()
"#,
        )
        .expect("detached ShadowRoot query boundaries should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|1|true|true|true|1|true|false:function:true:true:true:true|false:true:function:getSelection:0:true:true|false:function"
    );
}
#[test]
fn detached_selector_matching_handles_pseudo_only_and_deep_compounds() {
    let mut vm = new_storage_test_vm("https://detached-selector-compounds.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(`
    <html><body>
      <main>
        <section id="a"><p><span id="target" data-kind="hit"></span></p></section>
        <section id="b"><p><span id="other"></span></p></section>
      </main>
    </body></html>
  `, 'text/html');
  const body = doc.body;
  const target = doc.getElementById('target');
  let reads = [];
  for (const element of [target, doc.getElementById('other')]) {
    for (const property of ['id', 'className', 'localName', 'namespaceURI', 'nodeValue']) {
      Object.defineProperty(element, property, {
        configurable: true,
        get() {
          reads.push(property);
          return property === 'nodeValue' ? 'tampered' : 'wrong';
        }
      });
    }
    element.getAttribute = name => {
      reads.push(`get:${name}`);
      return 'wrong';
    };
  }
  const label = node => node === target ? 'target' : node === doc.getElementById('other') ? 'other' : node.localName;
  return JSON.stringify({
    pseudoFirst: Array.from(body.querySelectorAll(':first-child')).map(label).join(','),
    pseudoEmpty: Array.from(body.querySelectorAll(':empty')).map(label).join(','),
    deepChild: Array.from(body.querySelectorAll('main > section > p > span')).map(label).join(','),
    deepDescendant: Array.from(body.querySelectorAll('main section span')).map(label).join(','),
    complexAncestor: body.querySelector('main > section p > span') === target,
    attr: body.querySelector('[data-kind=hit]') === target,
    reads
  });
})()
"#,
        )
        .expect("detached selectors should handle pseudo-only and deep compounds");

    assert_eq!(
        result,
        r#"{"pseudoFirst":"main,section,p,target,p,other","pseudoEmpty":"target,other","deepChild":"target,other","deepDescendant":"target,other","complexAncestor":true,"attr":true,"reads":[]}"#
    );
}
#[test]
fn detached_node_replace_child_and_clone_node_follow_dom_shape() {
    let mut vm = new_storage_test_vm("https://detached-node-replace-clone.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><p id="old"></p><aside id="tail"></aside></body></html>',
    'text/html'
  );
  const old = doc.getElementById('old');
  const fragment = doc.createDocumentFragment();
  const first = doc.createElement('section');
  first.id = 'first';
  const second = doc.createElement('article');
  second.id = 'second';
  fragment.appendChild(first);
  fragment.appendChild(second);
  const returned = doc.body.replaceChild(fragment, old);

  const source = doc.createElement('div');
  source.id = 'source';
  source.setAttribute('data-x', '1');
  source.appendChild(doc.createTextNode('hello'));
  const shallow = source.cloneNode(false);
  const deep = source.cloneNode(true);

  return [
    typeof doc.body.replaceChild,
    typeof source.cloneNode,
    returned === old,
    old.parentNode === null,
    old.isConnected,
    fragment.childNodes.length,
    doc.body.children.length,
    doc.body.children.item(0) === first,
    doc.body.children.item(1) === second,
    first.previousSibling === null,
    first.nextSibling === second,
    second.previousSibling === first,
    second.nextSibling.id,
    shallow.id,
    shallow.getAttribute('data-x'),
    shallow.childNodes.length,
    shallow.isConnected,
    deep.id,
    deep.getAttribute('data-x'),
    deep.childNodes.length,
    deep.childNodes[0].data,
    deep.isConnected
  ].join('|');
})()
"#,
        )
        .expect("detached replaceChild/cloneNode should follow DOM shape");

    assert_eq!(
        result,
        "function|function|true|true|false|0|3|true|true|true|true|true|tail|source|1|0|false|source|1|1|hello|false"
    );
}
#[test]
fn detached_nodes_expose_owner_document_default_view_and_contains() {
    let mut vm = new_storage_test_vm("https://detached-node-ownership.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><main id="root"><span id="child"></span></main></body></html>',
    'text/html'
  );
  const root = doc.getElementById('root');
  const child = doc.getElementById('child');
  const fragment = doc.createDocumentFragment();
  const created = doc.createElement('section');
  const text = doc.createTextNode('x');
  fragment.appendChild(created);
  created.appendChild(text);
  const deep = root.cloneNode(true);

  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const childDoc = frame.contentDocument;
  const childNode = childDoc.createElement('div');
  childNode.id = 'inside';
  childDoc.body.appendChild(childNode);
  const owns = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const deleteDefaultView = delete childDoc.defaultView;
  const deleteParentWindow = delete childDoc.parentWindow;

  return [
    doc.ownerDocument === null,
    doc.defaultView === null,
    root.ownerDocument === doc,
    child.ownerDocument === doc,
    fragment.ownerDocument === doc,
    created.ownerDocument === doc,
    text.ownerDocument === doc,
    deep.ownerDocument === doc,
    deep.firstChild.ownerDocument === doc,
    childDoc.ownerDocument === null,
    !owns(childDoc, 'defaultView'),
    !owns(childDoc, 'parentWindow'),
    deleteDefaultView,
    deleteParentWindow,
    childDoc.defaultView === frame.contentWindow,
    typeof childDoc.parentWindow === 'undefined',
    childDoc.defaultView.document === childDoc,
    childNode.ownerDocument === childDoc,
    typeof root.contains,
    doc.contains(doc),
    doc.contains(root),
    root.contains(root),
    root.contains(child),
    child.contains(root),
    root.contains(created),
    fragment.contains(created),
    created.contains(text),
    deep.contains(deep.firstChild),
    root.contains(null)
  ].join('|');
})()
"#,
        )
        .expect("detached DOM ownership and contains should be consistent");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|function|true|true|true|true|false|false|true|true|true|false"
    );
}
#[test]
fn detached_html_document_accessors_do_not_cross_shadow_boundary() {
    let mut vm = new_storage_test_vm("https://detached-upper-boundary-accessors.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const hostMarkup = [
    '<head class="host">',
    '<title class="host"></title>',
    '<link class="host" rel="help" href="#">',
    '</head>',
    '<body class="host">',
    '<p class="host"></p>',
    '<a class="host" name="test-name"></a>',
    '<a class="host" href="#"></a>',
    '<area class="host" href="#">',
    '<img class="host" src="#" alt="">',
    '<embed class="host"></embed>',
    '<form class="host"></form>',
    '<script class="host"><' + '/script>',
    '</body>'
  ].join('\n');
  const shadowMarkup = hostMarkup.replaceAll('host', 'shadow');
  const doc = document.implementation.createHTMLDocument('');
  doc.documentElement.innerHTML = hostMarkup;
  doc.documentElement.className = 'host';
  const shadowRoot = doc.body.attachShadow({ mode: 'open' });
  shadowRoot.innerHTML = shadowMarkup;

  doc.getElementsByTagName('title')[0].textContent = 'host title';
  shadowRoot.querySelector('title').textContent = 'shadow title';
  shadowRoot.querySelector('p').id = 'shadow-id';

  function hostCollection(collection) {
    return collection.length > 0 &&
      Array.prototype.every.call(collection, element => element.className === 'host');
  }

  return [
    doc.head.className,
    doc.body.className,
    doc.title,
    hostCollection(doc.images),
    hostCollection(doc.embeds),
    hostCollection(doc.plugins),
    hostCollection(doc.links),
    hostCollection(doc.forms),
    hostCollection(doc.scripts),
    hostCollection(doc.getElementsByName('test-name')),
    hostCollection(doc.anchors),
    hostCollection(doc.all),
    hostCollection(doc.getElementsByTagName('p')),
    doc.getElementsByTagNameNS('http://www.w3.org/1999/xhtml', 'p')[0].className,
    doc.getElementById('shadow-id') === null
  ].join('|');
})()
"##,
        )
        .expect("detached document accessors should respect shadow upper boundary");

    assert_eq!(
        result,
        "host|host|host title|true|true|true|true|true|true|true|true|true|true|host|true"
    );
}
#[test]
fn detached_shadow_label_and_form_idrefs_stay_in_tree_scope() {
    let mut vm = new_storage_test_vm("https://detached-shadow-idrefs.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const d = document.implementation.createHTMLDocument('');
  const host = d.createElement('div');
  d.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });

  const shadowInput = d.createElement('input');
  shadowInput.id = 'control-id';
  shadow.appendChild(shadowInput);
  const outerLabel = d.createElement('label');
  outerLabel.setAttribute('for', 'control-id');
  d.body.appendChild(outerLabel);

  const innerLabel = d.createElement('label');
  innerLabel.setAttribute('for', 'control-id');
  shadow.appendChild(innerLabel);

  const shadowForm = d.createElement('form');
  shadowForm.id = 'form-id';
  shadow.appendChild(shadowForm);
  const outerInput = d.createElement('input');
  outerInput.setAttribute('form', 'form-id');
  d.body.appendChild(outerInput);

  const innerInput = d.createElement('input');
  innerInput.setAttribute('form', 'form-id');
  shadow.appendChild(innerInput);

  return [
    outerLabel.control === null,
    innerLabel.control === shadowInput,
    outerInput.form === null,
    innerInput.form === shadowForm
  ].join('|');
})()
"#,
        )
        .expect("detached shadow idref accessors should respect tree scope");

    assert_eq!(result, "true|true|true|true");
}
#[test]
fn detached_nodes_compare_document_position_matches_basic_dom_shape() {
    let mut vm = new_storage_test_vm("https://detached-node-position.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><main id="root"><span id="first"></span><em id="second"></em></main></body></html>',
    'text/html'
  );
  const root = doc.getElementById('root');
  const first = doc.getElementById('first');
  const second = doc.getElementById('second');
  const created = doc.createElement('section');
  const otherDoc = new DOMParser().parseFromString('<html><body><p id="other"></p></body></html>', 'text/html');
  const other = otherDoc.getElementById('other');
  const fragment = doc.createDocumentFragment();
  const fragmentChild = doc.createElement('b');
  fragment.appendChild(fragmentChild);

  const disconnected = created.compareDocumentPosition(root);
  const crossDocument = root.compareDocumentPosition(other);
  const fragmentDisconnected = fragment.compareDocumentPosition(root);
  root.insertBefore(created, first);
  let typeError = '';
  try {
    root.compareDocumentPosition(null);
  } catch (error) {
    typeError = error && error.name;
  }

  return [
    typeof root.compareDocumentPosition,
    doc.compareDocumentPosition(root),
    root.compareDocumentPosition(doc),
    root.compareDocumentPosition(first),
    first.compareDocumentPosition(root),
    first.compareDocumentPosition(second),
    second.compareDocumentPosition(first),
    root.compareDocumentPosition(root),
    (disconnected & Node.DOCUMENT_POSITION_DISCONNECTED) !== 0,
    (disconnected & Node.DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC) !== 0,
    (crossDocument & Node.DOCUMENT_POSITION_DISCONNECTED) !== 0,
    (crossDocument & Node.DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC) !== 0,
    (fragmentDisconnected & Node.DOCUMENT_POSITION_DISCONNECTED) !== 0,
    created.compareDocumentPosition(first),
    first.compareDocumentPosition(created),
    fragment.compareDocumentPosition(fragmentChild),
    fragmentChild.compareDocumentPosition(fragment),
    typeError
  ].join('|');
})()
"#,
        )
        .expect("detached compareDocumentPosition should cover basic DOM relations");

    assert_eq!(
        result,
        "function|20|10|20|10|4|2|0|true|true|true|true|true|4|2|20|10|TypeError"
    );
}
#[test]
fn detached_nodes_expose_basic_node_relationship_helpers() {
    let mut vm = new_storage_test_vm("https://detached-node-helpers.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = '<html><body><main id="root" data-x="1"><span>text</span><!--c--></main></body></html>';
  const doc = new DOMParser().parseFromString(source, 'text/html');
  const sameDoc = new DOMParser().parseFromString(source, 'text/html');
  const differentDoc = new DOMParser().parseFromString(
    '<html><body><main id="root" data-x="1"><span>different</span><!--c--></main></body></html>',
    'text/html'
  );
  const root = doc.getElementById('root');
  const clone = root.cloneNode(true);
  const shallow = root.cloneNode(false);
  const sameRoot = sameDoc.getElementById('root');
  const differentRoot = differentDoc.getElementById('root');
  const fragment = doc.createDocumentFragment();
  const created = doc.createElement('section');
  const text = doc.createTextNode('x');
  fragment.appendChild(created);
  created.appendChild(text);

  return [
    typeof root.hasChildNodes,
    typeof root.isEqualNode,
    typeof root.getRootNode,
    doc.hasChildNodes(),
    root.hasChildNodes(),
    root.firstChild.hasChildNodes(),
    created.hasChildNodes(),
    text.hasChildNodes(),
    root.isEqualNode(clone),
    root.isEqualNode(shallow),
    root.isEqualNode(sameRoot),
    root.isEqualNode(differentRoot),
    root.isEqualNode(null),
    doc.isEqualNode(sameDoc),
    root.getRootNode() === doc,
    text.getRootNode() === fragment,
    created.getRootNode() === fragment,
    root.firstChild.getRootNode({ composed: true }) === doc
  ].join('|');
})()
"#,
        )
        .expect("detached Node helpers should cover equality, children, and roots");

    assert_eq!(
        result,
        "function|function|function|true|true|true|true|false|true|false|true|false|false|true|true|true|true|true"
    );
}
#[test]
fn mutation_observer_accepts_detached_child_document_nodes() {
    let mut vm = new_storage_test_vm("https://child-window-detached-observer.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const target = frame.contentDocument.createElement('div');
  target.id = 'root';
  frame.contentDocument.body.appendChild(target);
  const observer = new MutationObserver(() => {});
  let status = 'ok';
  try {
    observer.observe(target, { childList: true, subtree: true });
    target.appendChild(frame.contentDocument.createTextNode('hello'));
  } catch (error) {
    status = error && error.message;
  }
  return [
    target instanceof Node,
    target instanceof frame.contentWindow.Node,
    status,
    observer.takeRecords().length
  ].join('|');
})()
"#,
        )
        .expect("MutationObserver should accept detached child document nodes");

    assert_eq!(result, "false|true|ok|1");
}
#[test]
fn mutation_observer_child_list_records_node_moves_and_fragments() {
    let mut vm = new_storage_test_vm("https://mutation-observer-child-list-records.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const label = (node) => node ? (node.nodeType === Node.TEXT_NODE ? node.data : node.nodeName) : '';
  const labels = (nodes) => Array.from(nodes).map(label).join(',');
  const summarize = (records) => records.map((record) => [
    record.type,
    labels(record.addedNodes),
    labels(record.removedNodes),
    label(record.previousSibling),
    label(record.nextSibling)
  ].join(':')).join('|');
  const makeFragment = () => {
    const fragment = document.createDocumentFragment();
    fragment.appendChild(document.createTextNode('11'));
    fragment.appendChild(document.createTextNode('22'));
    return fragment;
  };
  const results = [];

  const fragmentParent = document.createElement('p');
  fragmentParent.appendChild(document.createElement('span'));
  const fragmentParentObserver = new MutationObserver(() => {});
  fragmentParentObserver.observe(fragmentParent, { childList: true });
  fragmentParent.insertBefore(makeFragment(), fragmentParent.firstChild);
  results.push(summarize(fragmentParentObserver.takeRecords()));

  const fragmentTarget = makeFragment();
  const fragmentHost = document.createElement('p');
  const fragmentObserver = new MutationObserver(() => {});
  fragmentObserver.observe(fragmentTarget, { childList: true });
  fragmentHost.appendChild(fragmentTarget);
  results.push(summarize(fragmentObserver.takeRecords()));

  const moveSource = document.createElement('p');
  moveSource.appendChild(document.createElement('span'));
  const moveDestination = document.createElement('p');
  const moveObserver = new MutationObserver(() => {});
  moveObserver.observe(moveSource, { childList: true });
  moveDestination.appendChild(moveSource.firstChild);
  results.push(summarize(moveObserver.takeRecords()));

  const rangeParent = document.createElement('p');
  rangeParent.appendChild(document.createElement('span'));
  rangeParent.appendChild(document.createElement('b'));
  const range = document.createRange();
  range.setStartBefore(rangeParent.firstChild);
  range.setEndAfter(rangeParent.firstChild);
  const rangeObserver = new MutationObserver(() => {});
  rangeObserver.observe(rangeParent, { childList: true });
  range.deleteContents();
  results.push(summarize(rangeObserver.takeRecords()));

  const selfParent = document.createElement('p');
  selfParent.appendChild(document.createElement('span'));
  const selfObserver = new MutationObserver(() => {});
  selfObserver.observe(selfParent, { childList: true });
  selfParent.replaceChild(selfParent.firstChild, selfParent.firstChild);
  results.push(summarize(selfObserver.takeRecords()));

  return results.join('\n');
})()
"#,
        )
        .expect("MutationObserver childList records should evaluate");

    assert_eq!(
        result,
        "childList:11,22:::SPAN\n\
         childList::11,22::\n\
         childList::SPAN::\n\
         childList::SPAN::B\n\
         childList::SPAN::|\
         childList:SPAN:::"
    );
}
#[test]
fn mutation_observer_coalesces_child_node_replace_with_records() {
    let mut vm = new_storage_test_vm("https://mutation-observer-replace-with.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement('p');
  const before = document.createElement('b');
  const replaced = document.createElement('span');
  const after = document.createElement('i');
  parent.append(before, replaced, after);
  const observer = new MutationObserver(() => {});
  observer.observe(parent, { childList: true });
  replaced.replaceWith('x', document.createElement('em'));
  return observer.takeRecords().map((record) => [
    record.type,
    Array.from(record.addedNodes, (node) => node.nodeName).join(','),
    Array.from(record.removedNodes, (node) => node.nodeName).join(','),
    record.previousSibling && record.previousSibling.nodeName,
    record.nextSibling && record.nextSibling.nodeName
  ].join(':')).join('|');
})()
"#,
        )
        .expect("ChildNode.replaceWith MutationObserver record should evaluate");

    assert_eq!(result, "childList:#text,EM:SPAN:B:I");
}
#[test]
fn mutation_observer_reports_normalize_records_in_mutation_order() {
    let mut vm = new_storage_test_vm("https://mutation-observer-normalize-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.createElement('p');
  parent.append(
    document.createTextNode('A'),
    document.createTextNode('-'),
    document.createTextNode('X'),
    document.createElement('em'),
    document.createTextNode('C'),
    document.createTextNode('-'),
    document.createTextNode('tail'),
    document.createTextNode('')
  );
  const observer = new MutationObserver(() => {});
  observer.observe(parent, {
    subtree: true,
    childList: true,
    characterData: true,
    characterDataOldValue: true
  });
  parent.normalize();
  return JSON.stringify(observer.takeRecords().map((record) => [
    record.type,
    record.target.nodeType === Node.TEXT_NODE ? record.target.data : record.target.nodeName,
    record.oldValue,
    Array.from(record.removedNodes, (node) => node.data).join(',')
  ]));
})()
"#,
        )
        .expect("Node.normalize MutationObserver records should evaluate");

    assert_eq!(
        result,
        r#"[["characterData","A-X","A",""],["childList","P",null,"-"],["characterData","A-X","A-",""],["childList","P",null,"X"],["characterData","C-tail","C",""],["childList","P",null,"-"],["characterData","C-tail","C-",""],["childList","P",null,"tail"],["childList","P",null,""]]"#
    );
}
#[test]
fn mutation_observer_records_are_queued_before_inserted_scripts_run() {
    let mut vm = new_storage_test_vm("https://mutation-observer-script-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const main = document.createElement('main');
  document.body.appendChild(main);
  window.__lmMutationObserver = new MutationObserver(() => {});
  window.__lmMutationObserver.observe(main, { childList: true });
  const script = document.createElement('script');
  script.textContent = `
    const records = window.__lmMutationObserver.takeRecords();
    window.__lmMutationRecords = [
      records.length,
      records[0] && records[0].target === document.querySelector('main'),
      records[0] && records[0].addedNodes[0] === document.currentScript
    ].join('|');
  `;
  main.appendChild(script);
  return window.__lmMutationRecords;
})()
"#,
        )
        .expect("inserted script should see its own mutation record");

    assert_eq!(result, "1|true|true");
}
#[test]
fn mutation_observer_coalesces_inner_and_outer_html_replacements() {
    let mut vm = new_storage_test_vm("https://mutation-observer-markup-replace.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const label = (node) => node ? (node.nodeType === Node.TEXT_NODE ? node.data : node.nodeName) : '';
  const labels = (nodes) => Array.from(nodes).map(label).join(',');
  const summarize = (records) => records.map((record) => [
    record.type,
    labels(record.addedNodes),
    labels(record.removedNodes),
    label(record.previousSibling),
    label(record.nextSibling)
  ].join(':')).join('|');

  const inner = document.createElement('p');
  inner.appendChild(document.createTextNode('old'));
  document.body.appendChild(inner);
  const innerObserver = new MutationObserver(() => {});
  innerObserver.observe(inner, { childList: true });
  inner.innerHTML = '<span>new</span><span>text</span>';

  const outer = document.createElement('div');
  outer.appendChild(document.createElement('p'));
  document.body.appendChild(outer);
  const outerObserver = new MutationObserver(() => {});
  outerObserver.observe(outer, { childList: true });
  outer.firstChild.outerHTML = '<em>next</em>';

  return [
    summarize(innerObserver.takeRecords()),
    summarize(outerObserver.takeRecords())
  ].join('\n');
})()
"#,
        )
        .expect("markup replacement mutation records should evaluate");

    assert_eq!(
        result,
        "childList:SPAN,SPAN:old::\n\
         childList:EM:P::"
    );
}
#[test]
fn outer_html_rejects_document_parent_without_mutating_tree() {
    let mut vm = new_storage_test_vm("https://outer-html-document-parent.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  const root = document.documentElement;
  const before = root.outerHTML;
  let errorName = 'missing';
  let errorCode = -1;
  try {
    root.outerHTML = '<html><body><p id="replacement">replacement</p></body></html>';
  } catch (error) {
    errorName = error.name;
    errorCode = error.code;
  }
  return JSON.stringify([
    errorName,
    errorCode,
    document.documentElement === root,
    root.outerHTML === before,
    document.getElementById('replacement') === null
  ]);
})()
"#,
        )
        .expect("document child outerHTML rejection should evaluate");

    assert_eq!(result, r#"["NoModificationAllowedError",7,true,true,true]"#);
}
#[test]
fn mutation_observer_reports_local_name_and_namespace_for_attribute_ns() {
    let mut vm = new_storage_test_vm("https://mutation-observer-attribute-name.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  const observer = new MutationObserver(() => {});
  observer.observe(element, { attributes: true, attributeOldValue: true });
  element.setAttributeNS("urn:moli:test", "lm:flag", "on");
  const created = observer.takeRecords()[0];
  element.removeAttributeNS("urn:moli:test", "flag");
  return JSON.stringify([created, ...observer.takeRecords()].map((record) => ({
    type: record.type,
    attributeName: record.attributeName,
    attributeNamespace: record.attributeNamespace,
    oldValue: record.oldValue
  })));
})()
"#,
        )
        .expect("namespaced attribute removals should report local name and namespace");

    assert_eq!(
        result,
        r#"[{"type":"attributes","attributeName":"flag","attributeNamespace":"urn:moli:test","oldValue":null},{"type":"attributes","attributeName":"flag","attributeNamespace":"urn:moli:test","oldValue":"on"}]"#
    );
}
#[test]
fn detached_child_document_anchor_resolves_url_properties() {
    let mut vm = new_storage_test_vm("https://child-window-anchor.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const laterParentBase = document.createElement('base');
  laterParentBase.href = 'https://later-parent-base.test/';
  (document.head || document.documentElement || document).appendChild(laterParentBase);
  const anchor = frame.contentDocument.createElement('a');
  anchor.setAttribute('href', '/item?id=1#frag');
  return [
    frame.contentDocument.URL,
    frame.contentDocument.baseURI,
    Object.prototype.toString.call(anchor),
    anchor instanceof HTMLAnchorElement,
    anchor instanceof frame.contentWindow.HTMLAnchorElement,
    Object.getPrototypeOf(anchor) === frame.contentWindow.HTMLAnchorElement.prototype,
    anchor.href,
    anchor.protocol,
    anchor.host,
    anchor.hostname,
    anchor.port,
    anchor.pathname,
    anchor.search,
    anchor.hash,
    anchor.pathname.charAt(0)
  ].join('|');
})()
"#,
        )
        .expect("detached child document anchors should expose URL properties");

    assert_eq!(
        result,
        "about:blank|https://child-window-anchor.test/path/page.html|[object HTMLAnchorElement]|false|true|true|https://child-window-anchor.test/item?id=1#frag|https:|child-window-anchor.test|child-window-anchor.test||/item|?id=1|#frag|/"
    );
}
#[test]
fn detached_html_elements_use_specific_prototypes_for_common_tags() {
    let mut vm = new_storage_test_vm("https://detached-html-element-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const expected = new Map([
    ['a', 'HTMLAnchorElement'],
    ['img', 'HTMLImageElement'],
    ['form', 'HTMLFormElement'],
    ['input', 'HTMLInputElement'],
    ['button', 'HTMLButtonElement'],
    ['script', 'HTMLScriptElement'],
    ['iframe', 'HTMLIFrameElement'],
    ['canvas', 'HTMLCanvasElement'],
    ['textarea', 'HTMLTextAreaElement'],
    ['select', 'HTMLSelectElement'],
    ['option', 'HTMLOptionElement'],
    ['section', 'HTMLElement']
  ]);
  const doc = new DOMParser().parseFromString(
    '<html><body><img id="parsed"><form id="form"><input id="input"></form></body></html>',
    'text/html'
  );
  const created = [];
  for (const [tag, ctorName] of expected) {
    const element = doc.createElement(tag);
    const ctor = globalThis[ctorName];
    created.push([
      tag,
      Object.prototype.toString.call(element),
      ctor && element instanceof ctor,
      element instanceof HTMLElement,
      element.constructor && element.constructor.name
    ].join(','));
  }
  const parsedImg = doc.getElementById('parsed');
  const parsedInput = doc.getElementById('input');
  const probeDiv = doc.createElement('div');
  probeDiv.innerHTML = '<span class="x"></span>';
  const probeProto = Object.getPrototypeOf(probeDiv);
  const probeProtoParent = Object.getPrototypeOf(probeProto);
  const hasOwn = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const methodShape = [
    hasOwn(probeDiv, 'appendChild'),
    hasOwn(probeDiv, 'querySelector'),
    hasOwn(probeDiv, 'getAttribute'),
    hasOwn(probeDiv, 'matches'),
    probeProto === HTMLDivElement.prototype,
    probeProtoParent === HTMLElement.prototype,
    typeof probeDiv.appendChild,
    probeDiv.querySelector('.x').tagName
  ].join(',');
  const xmlDoc = new DOMParser().parseFromString('<input/>', 'application/xml');
  return [
    created.join(';'),
    Object.prototype.toString.call(parsedImg),
    parsedImg instanceof HTMLImageElement,
    Object.prototype.toString.call(parsedInput),
    parsedInput instanceof HTMLInputElement,
    Object.prototype.toString.call(xmlDoc.documentElement),
    xmlDoc.documentElement instanceof Element,
    xmlDoc.documentElement instanceof HTMLInputElement,
    methodShape
  ].join('|');
})()
"#,
        )
        .expect("detached HTML elements should use common specialized prototypes");

    assert_eq!(
        result,
        "a,[object HTMLAnchorElement],true,true,HTMLAnchorElement;img,[object HTMLImageElement],true,true,HTMLImageElement;form,[object HTMLFormElement],true,true,HTMLFormElement;input,[object HTMLInputElement],true,true,HTMLInputElement;button,[object HTMLButtonElement],true,true,HTMLButtonElement;script,[object HTMLScriptElement],true,true,HTMLScriptElement;iframe,[object HTMLIFrameElement],true,true,HTMLIFrameElement;canvas,[object HTMLCanvasElement],true,true,HTMLCanvasElement;textarea,[object HTMLTextAreaElement],true,true,HTMLTextAreaElement;select,[object HTMLSelectElement],true,true,HTMLSelectElement;option,[object HTMLOptionElement],true,true,HTMLOptionElement;section,[object HTMLElement],true,true,HTMLElement|[object HTMLImageElement]|true|[object HTMLInputElement]|true|[object Element]|true|false|false,false,false,false,true,true,function,SPAN"
    );
}
#[test]
fn live_html_create_element_matches_replay_brands_for_standard_tags() {
    let mut vm = new_storage_test_vm("https://live-html-element-replay-brands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const expected = new Map([
    ['html', 'HTMLHtmlElement'],
    ['head', 'HTMLHeadElement'],
    ['body', 'HTMLBodyElement'],
    ['input', 'HTMLInputElement'],
    ['select', 'HTMLSelectElement'],
    ['option', 'HTMLOptionElement'],
    ['fieldset', 'HTMLFieldSetElement'],
    ['meta', 'HTMLMetaElement'],
    ['title', 'HTMLTitleElement'],
    ['span', 'HTMLSpanElement'],
    ['p', 'HTMLParagraphElement'],
    ['area', 'HTMLAreaElement'],
    ['base', 'HTMLBaseElement'],
    ['br', 'HTMLBRElement'],
    ['data', 'HTMLDataElement'],
    ['datalist', 'HTMLDataListElement'],
    ['map', 'HTMLMapElement'],
    ['object', 'HTMLObjectElement'],
    ['output', 'HTMLOutputElement'],
    ['progress', 'HTMLProgressElement'],
    ['table', 'HTMLTableElement'],
    ['caption', 'HTMLTableCaptionElement'],
    ['col', 'HTMLTableColElement'],
    ['tbody', 'HTMLTableSectionElement'],
    ['tr', 'HTMLTableRowElement'],
    ['slot', 'HTMLSlotElement'],
    ['source', 'HTMLSourceElement'],
    ['del', 'HTMLModElement'],
    ['pre', 'HTMLPreElement'],
    ['frame', 'HTMLFrameElement'],
    ['frameset', 'HTMLFrameSetElement'],
    ['font', 'HTMLFontElement'],
    ['marquee', 'HTMLMarqueeElement'],
    ['meter', 'HTMLMeterElement'],
    ['ul', 'HTMLUListElement'],
    ['section', 'HTMLElement']
  ]);
  const out = [];
  for (const [tag, ctorName] of expected) {
    const element = document.createElement(tag);
    const ctor = globalThis[ctorName];
    out.push([
      tag,
      element.constructor && element.constructor.name,
      Object.prototype.toString.call(element),
      !!ctor && element instanceof ctor,
      element instanceof HTMLElement
    ].join(':'));
  }
  return out.join('|');
})()
"#,
        )
        .expect("live createElement should expose Chromium-like brands");

    assert_eq!(
        result,
        "html:HTMLHtmlElement:[object HTMLHtmlElement]:true:true|head:HTMLHeadElement:[object HTMLHeadElement]:true:true|body:HTMLBodyElement:[object HTMLBodyElement]:true:true|input:HTMLInputElement:[object HTMLInputElement]:true:true|select:HTMLSelectElement:[object HTMLSelectElement]:true:true|option:HTMLOptionElement:[object HTMLOptionElement]:true:true|fieldset:HTMLFieldSetElement:[object HTMLFieldSetElement]:true:true|meta:HTMLMetaElement:[object HTMLMetaElement]:true:true|title:HTMLTitleElement:[object HTMLTitleElement]:true:true|span:HTMLSpanElement:[object HTMLSpanElement]:true:true|p:HTMLParagraphElement:[object HTMLParagraphElement]:true:true|area:HTMLAreaElement:[object HTMLAreaElement]:true:true|base:HTMLBaseElement:[object HTMLBaseElement]:true:true|br:HTMLBRElement:[object HTMLBRElement]:true:true|data:HTMLDataElement:[object HTMLDataElement]:true:true|datalist:HTMLDataListElement:[object HTMLDataListElement]:true:true|map:HTMLMapElement:[object HTMLMapElement]:true:true|object:HTMLObjectElement:[object HTMLObjectElement]:true:true|output:HTMLOutputElement:[object HTMLOutputElement]:true:true|progress:HTMLProgressElement:[object HTMLProgressElement]:true:true|table:HTMLTableElement:[object HTMLTableElement]:true:true|caption:HTMLTableCaptionElement:[object HTMLTableCaptionElement]:true:true|col:HTMLTableColElement:[object HTMLTableColElement]:true:true|tbody:HTMLTableSectionElement:[object HTMLTableSectionElement]:true:true|tr:HTMLTableRowElement:[object HTMLTableRowElement]:true:true|slot:HTMLSlotElement:[object HTMLSlotElement]:true:true|source:HTMLSourceElement:[object HTMLSourceElement]:true:true|del:HTMLModElement:[object HTMLModElement]:true:true|pre:HTMLPreElement:[object HTMLPreElement]:true:true|frame:HTMLFrameElement:[object HTMLFrameElement]:true:true|frameset:HTMLFrameSetElement:[object HTMLFrameSetElement]:true:true|font:HTMLFontElement:[object HTMLFontElement]:true:true|marquee:HTMLMarqueeElement:[object HTMLMarqueeElement]:true:true|meter:HTMLMeterElement:[object HTMLMeterElement]:true:true|ul:HTMLUListElement:[object HTMLUListElement]:true:true|section:HTMLElement:[object HTMLElement]:true:true"
    );
}

#[test]
fn parsed_html_template_content_is_exposed_through_live_wrapper() {
    let mut vm = new_parsed_test_vm(
        "https://live-template-content.test/",
        "<!doctype html><html><body><template id=t>Hello<span id=inner>world</span></template></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const template = document.getElementById('t');
  const content = template.content;
  const childNodes = Array.prototype.map.call(content.childNodes, (node) => {
    return [node.nodeType, node.nodeName, node.nodeValue || node.localName].join(':');
  });
  const descriptor = Object.getOwnPropertyDescriptor(HTMLTemplateElement.prototype, 'content');
  return [
    template.constructor && template.constructor.name,
    Object.prototype.toString.call(template),
    template instanceof HTMLTemplateElement,
    content instanceof DocumentFragment,
    Object.prototype.toString.call(content),
    content.childNodes.length,
    childNodes.join(','),
    content.querySelector('#inner').textContent,
    typeof descriptor.get,
    descriptor.enumerable,
    descriptor.configurable
  ].join('|');
})()
"#,
        )
        .expect("parsed template content should be visible through live wrapper");

    assert_eq!(
        result,
        "HTMLTemplateElement|[object HTMLTemplateElement]|true|true|[object DocumentFragment]|2|3:#text:Hello,1:SPAN:span|world|function|true|true"
    );
}

#[test]
fn parsed_xml_xhtml_template_preserves_interface_and_content() {
    let mut vm = new_storage_test_vm("https://xml-template-content.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xml = new DOMParser().parseFromString(
    "<template xmlns='http://www.w3.org/1999/xhtml'><test/></template>",
    "text/xml"
  );
  const template = xml.documentElement;
  return [
    template.constructor && template.constructor.name,
    Object.prototype.toString.call(template),
    template instanceof HTMLTemplateElement,
    template.childElementCount,
    template.content instanceof DocumentFragment,
    template.content.firstChild.localName,
  ].join('|');
})()
            "#,
        )
        .expect("XML XHTML template content should be exposed");

    assert_eq!(
        result,
        "HTMLTemplateElement|[object HTMLTemplateElement]|true|0|true|test"
    );
}

#[test]
fn xml_serializer_synthesizes_required_namespace_declarations() {
    let mut vm = new_storage_test_vm("https://xml-serializer-namespaces.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xml = document.implementation.createDocument('urn:catalog', 'c:catalog', null);
  const root = xml.documentElement;
  root.setAttributeNS('http://www.w3.org/2000/xmlns/', 'xmlns:m', 'urn:meta');
  const item = xml.createElementNS('urn:catalog', 'c:item');
  item.setAttributeNS('urn:meta', 'm:code', 'code-7');
  item.append(xml.createTextNode('alpha & beta'));
  root.append(item);

  const serialized = new XMLSerializer().serializeToString(xml);
  const reparsed = new DOMParser().parseFromString(serialized, 'application/xml');
  return [
    serialized,
    reparsed.documentElement.namespaceURI,
    reparsed.documentElement.getAttributeNS(
      'http://www.w3.org/2000/xmlns/',
      'c'
    ),
    reparsed.getElementsByTagNameNS('urn:catalog', 'item').length,
    reparsed.getElementsByTagNameNS('urn:catalog', 'item')[0]
      .getAttributeNS('urn:meta', 'code')
  ].join('|');
})()
"#,
        )
        .expect("XMLSerializer namespace projection should evaluate");

    assert_eq!(
        result,
        concat!(
            "<c:catalog xmlns:c=\"urn:catalog\" xmlns:m=\"urn:meta\">",
            "<c:item m:code=\"code-7\">alpha &amp; beta</c:item>",
            "</c:catalog>|urn:catalog|urn:catalog|1|code-7"
        )
    );
}

#[test]
fn xml_serializer_matches_chromium_for_empty_elements_attrs_and_parser_errors() {
    let mut vm = new_storage_test_vm("https://xml-serializer-node-kinds.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const serializer = new XMLSerializer();
  const htmlVoid = document.createElement('br');
  const htmlVoidWithChild = document.createElement('br');
  htmlVoidWithChild.append('child');
  const xml = document.implementation.createDocument(null, 'root');
  const owner = document.createElement('div');
  owner.setAttribute('data-value', 'a<&">\t\n\r');
  const attribute = owner.getAttributeNode('data-value');
  const emptyXml = new DOMParser().parseFromString('', 'text/xml');
  const emptyError = emptyXml.getElementsByTagName('parsererror')[0];
  const emptySerialized = serializer.serializeToString(emptyXml);
  const partialXml = new DOMParser().parseFromString(
    '<catalog><item></catalog>',
    'application/xml'
  );
  const partialError = partialXml.getElementsByTagName('parsererror')[0];
  const partialSerialized = serializer.serializeToString(partialXml);

  return [
    serializer.serializeToString(htmlVoid) ===
      '<br xmlns="http://www.w3.org/1999/xhtml" />',
    serializer.serializeToString(htmlVoidWithChild) ===
      '<br xmlns="http://www.w3.org/1999/xhtml">child</br>',
    serializer.serializeToString(xml.documentElement) === '<root/>',
    serializer.serializeToString(attribute) ===
      'a&lt;&amp;&quot;&gt;&#9;&#10;&#13;',
    emptyXml.documentElement.localName === 'html',
    emptyXml.documentElement.namespaceURI === 'http://www.w3.org/1999/xhtml',
    emptyXml.documentElement.getAttribute('xmlns') === null,
    emptyError.getAttributeNames().join(',') === 'style',
    emptySerialized.startsWith(
      '<html xmlns="http://www.w3.org/1999/xhtml"><body><parsererror style='
    ),
    !emptySerialized.includes('<parsererror xmlns='),
    partialError.getAttribute('xmlns') === null,
    partialSerialized.startsWith(
      '<catalog><parsererror xmlns="http://www.w3.org/1999/xhtml" style='
    )
  ].join('|');
})()
"#,
        )
        .expect("XMLSerializer node-kind projection should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn dom_parser_xml_errors_preserve_the_partial_document_root() {
    let mut vm = new_storage_test_vm("https://dom-parser-partial-xml-error.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parsed = new DOMParser().parseFromString(
    '<catalog><item></catalog>',
    'application/xml'
  );
  const errors = parsed.getElementsByTagName('parsererror');
  return [
    parsed.documentElement.localName,
    errors.length,
    errors[0].parentNode === parsed.documentElement,
    errors[0].namespaceURI,
    errors[0].nextElementSibling.localName,
    errors[0].querySelectorAll('h3').length
  ].join('|');
})()
"#,
        )
        .expect("DOMParser partial XML error tree should evaluate");

    assert_eq!(result, "catalog|1|true|http://www.w3.org/1999/xhtml|item|2");
}

#[test]
fn xhtml_element_interface_survives_move_through_xml_document() {
    let mut vm = new_storage_test_vm("https://xhtml-xml-document-move.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const xml = document.implementation.createDocument(
    "http://www.w3.org/1999/xhtml",
    "html"
  );
  const style = document.createElement("style");
  style.setAttribute("nonce", "allowme");
  const initialInterface = Object.getPrototypeOf(style) === HTMLStyleElement.prototype;

  xml.documentElement.appendChild(style);
  const xmlInterface = Object.getPrototypeOf(style) === HTMLStyleElement.prototype;
  const xmlNonce = style.nonce;

  body.appendChild(style);
  return [
    initialInterface,
    xmlInterface,
    xmlNonce,
    Object.getPrototypeOf(style) === HTMLStyleElement.prototype,
    style.nonce,
    style.getAttribute("nonce")
  ].join("|");
})()
            "#,
        )
        .expect("XHTML element interface should survive XML document adoption");

    assert_eq!(result, "true|true|allowme|true|allowme|");
}

#[test]
fn child_content_document_template_uses_child_realm_template_surface() {
    let mut vm = new_storage_test_vm("https://child-template-content.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  doc.open();
  doc.write('<!doctype html><html><body><template id=t>Hello<span id=inner>world</span></template></body></html>');
  doc.close();
  const template = doc.getElementById('t');
  const content = template.content;
  return [
    template.constructor === frame.contentWindow.HTMLTemplateElement,
    template instanceof frame.contentWindow.HTMLTemplateElement,
    Object.prototype.toString.call(template),
    content instanceof frame.contentWindow.DocumentFragment,
    Object.prototype.toString.call(content),
    content.childNodes.length,
    content.querySelector('#inner').textContent,
    typeof Object.getOwnPropertyDescriptor(frame.contentWindow.HTMLTemplateElement.prototype, 'content').get
  ].join('|');
})()
"#,
        )
        .expect("child template content should be visible through child realm wrapper");

    assert_eq!(
        result,
        "true|true|[object HTMLTemplateElement]|true|[object DocumentFragment]|2|world|function"
    );
}

#[test]
fn live_html_create_element_uses_html_unknown_element_for_observed_unknown_tags() {
    let mut vm = new_storage_test_vm("https://live-html-unknown-element-brands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const tags = ['applet', 'bgsound', 'blink', 'content', 'decorator', 'element', 'image', 'isindex', 'menuitem', 'shadow', 'spacer'];
  return tags.map((tag) => {
    const element = document.createElement(tag);
    return [
      tag,
      element.constructor && element.constructor.name,
      Object.prototype.toString.call(element),
      element instanceof HTMLUnknownElement,
      element instanceof HTMLElement
    ].join(':');
  }).join('|');
})()
"#,
        )
        .expect("unknown replay tags should brand as HTMLUnknownElement");

    assert_eq!(
        result,
        "applet:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|bgsound:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|blink:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|content:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|decorator:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|element:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|image:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|isindex:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|menuitem:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|shadow:HTMLUnknownElement:[object HTMLUnknownElement]:true:true|spacer:HTMLUnknownElement:[object HTMLUnknownElement]:true:true"
    );
}
#[test]
fn detached_html_elements_reflect_common_attributes() {
    let mut vm = new_storage_test_vm("https://detached-reflected-attrs.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body>' +
      '<input id="input" name="token" value="seed" checked disabled>' +
      '<img id="img" name="hero" src="img/a.png">' +
      '<script id="script" src="/app.js"></script>' +
      '<iframe id="frame" name="child" src="child.html"></iframe>' +
      '<button id="button" name="go" value="yes" disabled></button>' +
      '<textarea id="textarea" name="bio"></textarea>' +
      '<select id="select" name="choice"><option id="option" value="a" disabled>A</option></select>' +
      '<section id="section"></section>' +
    '</body></html>',
    'text/html'
  );
  const input = doc.getElementById('input');
  const img = doc.getElementById('img');
  const script = doc.getElementById('script');
  const frame = doc.getElementById('frame');
  const button = doc.getElementById('button');
  const textarea = doc.getElementById('textarea');
  const select = doc.getElementById('select');
  const option = doc.getElementById('option');
  const section = doc.getElementById('section');
  const created = doc.createElement('input');
  created.name = 'created-name';
  created.value = 42;
  created.checked = true;
  created.disabled = true;
  input.checked = false;
  input.disabled = false;
  img.src = '/asset.png';
  button.disabled = false;
  textarea.value = 'typed';
  select.value = 'b';
  option.disabled = false;
  return [
    input.name,
    input.value,
    input.checked,
    input.disabled,
    input.getAttribute('checked') === null,
    input.getAttribute('disabled') === null,
    created.name,
    created.value,
    created.checked,
    created.disabled,
    created.getAttribute('checked'),
    created.getAttribute('disabled'),
    img.name,
    img.getAttribute('src'),
    img.src,
    script.getAttribute('src'),
    script.src,
    frame.name,
    frame.getAttribute('src'),
    frame.src,
    button.name,
    button.value,
    button.disabled,
    button.getAttribute('disabled') === null,
    textarea.name,
    textarea.value,
    textarea.getAttribute('value'),
    select.name,
    select.value,
    select.getAttribute('value'),
    option.value,
    option.disabled,
    option.getAttribute('disabled') === null,
    'name' in section,
    typeof section.value,
    typeof section.checked,
    typeof section.src
  ].join('|');
})()
"#,
        )
        .expect("detached HTML elements should reflect common attributes");

    assert_eq!(
        result,
        "token|seed|false|false|false|true|created-name|42|true|true|||hero|/asset.png|https://detached-reflected-attrs.test/asset.png|/app.js|https://detached-reflected-attrs.test/app.js|child|child.html|https://detached-reflected-attrs.test/path/child.html|go|yes|false|true|bio|typed||choice|||a|false|true|false|undefined|undefined|undefined"
    );
}
#[test]
fn live_element_name_accessor_is_writable() {
    let mut vm = new_storage_test_vm("https://live-name-attr.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const input = document.createElement('input');
  input.name = 'live-token';
  return [input.name, input.getAttribute('name')].join('|');
})()
"#,
        )
        .expect("live element name accessor should evaluate");

    assert_eq!(result, "live-token|live-token");
}

#[test]
fn option_name_assignment_is_an_expando_and_does_not_reflect_the_content_attribute() {
    let mut vm = new_storage_test_vm("https://option-name.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const option = document.createElement('option');
  option.setAttribute('name', 'content-name');
  const before = option.name;
  option.name = 'expando-name';
  const descriptor = Object.getOwnPropertyDescriptor(option, 'name');
  return JSON.stringify({
    prototypeHasName: Object.prototype.hasOwnProperty.call(HTMLOptionElement.prototype, 'name'),
    before: before === undefined ? 'undefined' : before,
    expando: option.name,
    attribute: option.getAttribute('name'),
    descriptor: [
      descriptor.value,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ]
  });
})()
"#,
        )
        .expect("option name expando probe should evaluate");

    assert_eq!(
        result,
        r#"{"prototypeHasName":false,"before":"undefined","expando":"expando-name","attribute":"content-name","descriptor":["expando-name",true,true,true]}"#
    );
}

#[test]
fn detached_child_document_elements_expose_focus_and_blur() {
    let mut vm = new_storage_test_vm("https://child-window-detached-focus.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<body></body>';
  (document.body || document.documentElement || document).appendChild(frame);
  const input = frame.contentDocument.createElement('input');
  let status = 'ok';
  try {
    input.focus();
    input.blur();
  } catch (error) {
    status = error && error.message;
  }
  return [
    input instanceof HTMLElement,
    input instanceof frame.contentWindow.HTMLElement,
    input instanceof frame.contentWindow.HTMLInputElement,
    Object.getPrototypeOf(input) === frame.contentWindow.HTMLInputElement.prototype,
    typeof input.focus,
    typeof input.blur,
    status
  ].join('|');
})()
"#,
        )
        .expect("detached child document elements should expose focus/blur");

    assert_eq!(result, "false|true|true|true|function|function|ok");
}
#[test]
fn dom_parser_detached_elements_dispatch_local_events() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-events.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><div id="parent"><button id="button"></button></div></body></html>',
    'text/html'
  );
  const parent = doc.getElementById('parent');
  const button = doc.getElementById('button');
  const order = [];
  parent.addEventListener('click', (event) => {
    order.push('capture:' + (event.target === button) + ':' + (event.currentTarget === parent));
  }, true);
  button.addEventListener('click', (event) => {
    order.push('target:' + (event.target === button) + ':' + (event.currentTarget === button) + ':' + (event.composedPath()[0] === button));
  });
  parent.addEventListener('click', () => order.push('bubble'));
  button.click();
  return order.join('|');
})()
"#,
        )
        .expect("DOMParser detached elements should dispatch local click events");

    assert_eq!(result, "capture:true:true|target:true:true:true|bubble");
}
#[test]
fn dom_parser_detached_iframe_load_ignores_tampered_non_elements() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-iframe-load-guard.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const text = doc.createTextNode('not an iframe');
  Object.defineProperty(text, 'localName', { value: 'iframe' });
  let textLoads = 0;
  text.addEventListener('load', () => ++textLoads);
  doc.body.appendChild(text);

  const iframe = doc.createElement('iframe');
  let iframeLoads = 0;
  iframe.addEventListener('load', () => ++iframeLoads);
  doc.body.appendChild(iframe);
  return `${textLoads}|${iframeLoads}`;
})()
"#,
        )
        .expect("DOMParser detached iframe load should guard node type");

    assert_eq!(result, "0|1");
}

#[test]
fn dom_parser_detached_iframe_window_declares_own_methods() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-window-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const iframe = doc.createElement('iframe');
  doc.body.appendChild(iframe);
  const win = iframe.contentWindow;
  const shape = name => {
    const descriptor = Object.getOwnPropertyDescriptor(win, name);
    const value = descriptor && descriptor.value;
    return [
      typeof value,
      value && value.name,
      value && value.length,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.configurable,
      descriptor && descriptor.writable,
      /\[native code\]/.test(String(value))
    ].join(':');
  };
  return [
    Object.prototype.toString.call(win),
    shape('postMessage'),
    shape('open'),
    shape('blur'),
    shape('find'),
    shape('stop'),
    shape('print'),
    win.find('needle'),
    String(win.blur()),
    String(win.print())
  ].join('|');
})()
"#,
        )
        .expect("DOMParser detached iframe window methods should evaluate");

    assert_eq!(
        result,
        "[object Window]|function:postMessage:1:false:true:true:true|function:open:0:false:true:true:true|function:blur:0:false:true:true:true|function:find:0:false:true:true:true|function:stop:0:false:true:true:true|function:print:0:false:true:true:true|false|undefined|undefined"
    );
}

#[test]
fn dom_parser_detached_iframe_adopted_node_loads_after_insert() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-iframe-import-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sourceDoc = new DOMParser().parseFromString(
    '<html><body><iframe></iframe></body></html>',
    'text/html'
  );
  const targetDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const source = sourceDoc.querySelector('iframe');
  let sourceListenerLoads = 0;
  const handlerTargets = [];
  source.onload = function() {
    handlerTargets.push(this.ownerDocument === targetDoc ? 'imported' : 'source');
  };
  source.addEventListener('load', () => ++sourceListenerLoads);
  targetDoc.body.appendChild(source);
  const imported = targetDoc.querySelector('iframe');

  return [
    !!imported,
    sourceListenerLoads,
    handlerTargets.join(','),
    source.contentDocument === imported.contentDocument,
    source.contentWindow === imported.contentWindow
  ].join('|');
})()
"#,
        )
        .expect("DOMParser detached iframe import source sync should evaluate");

    assert_eq!(result, "true|1|imported|true|true");
}
#[test]
fn dom_parser_detached_event_target_handles_remove_once_prevent_default_and_exceptions() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-events-edge.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><div id="target"></div></body></html>', 'text/html');
  const target = doc.getElementById('target');
  const out = [];
  function removed() { out.push('removed'); }
  target.addEventListener('custom', removed);
  target.removeEventListener('custom', removed);
  target.addEventListener('custom', { handleEvent() { out.push('object-once'); } }, { once: true });
  target.dispatchEvent(new Event('custom'));
  target.dispatchEvent(new Event('custom'));
  target.addEventListener('cancelable', (event) => event.preventDefault());
  const allowed = target.dispatchEvent(new Event('cancelable', { cancelable: true }));
  target.addEventListener('boom', () => { throw new Error('detached listener boom'); });
  target.addEventListener('boom', () => out.push('after-throw'));
  const boomAllowed = target.dispatchEvent(new Event('boom'));
  return [out.join(','), allowed, boomAllowed].join('|');
})()
"#,
        )
        .expect("DOMParser detached event dispatch should match EventTarget edge behavior");

    assert_eq!(result, "object-once,after-throw|false|true");
}
#[test]
fn constructed_event_target_exposes_composed_path_during_dispatch() {
    let mut vm = new_storage_test_vm("https://event-target-constructible.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = new EventTarget();
  const event = new Event('custom');
  const observed = [];
  target.addEventListener('custom', (event) => {
    observed.push(event.target === target);
    observed.push(event.currentTarget === target);
    observed.push(event.composedPath().length);
    observed.push(event.composedPath()[0] === target);
    event.initEvent('mutated', true, true);
    observed.push(event.type);
    observed.push(event.bubbles);
    observed.push(event.cancelable);
  }, { once: true });
  const allowed = target.dispatchEvent(event);
  observed.push(allowed);
  observed.push(event.currentTarget === null);
  observed.push(event.composedPath().length);
  target.dispatchEvent(event);
  return observed.join('|');
})()
"#,
        )
        .expect("constructed EventTarget should expose composedPath while dispatching");

    assert_eq!(result, "true|true|1|true|custom|false|false|true|true|0");
}
#[test]
fn dom_parser_detached_event_handler_properties_dispatch_through_local_events() {
    let mut vm = new_storage_test_vm("https://dom-parser-detached-handler-properties.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body><button id="target"></button></body></html>', 'text/html');
  const target = doc.getElementById('target');
  const out = [];
  out.push(target.onclick === null);
  target.onclick = function(event) {
    out.push('first:' + (this === target) + ':' + event.type + ':' + (event.currentTarget === target));
  };
  out.push(typeof target.onclick);
  target.click();
  target.onclick = function() { out.push('second'); };
  target.addEventListener('click', () => out.push('listener'));
  target.click();
  target.onclick = null;
  out.push(target.onclick === null);
  target.click();
  return out.join('|');
})()
"#,
        )
        .expect("DOMParser detached event handler properties should dispatch local events");

    assert_eq!(
        result,
        "true|function|first:true:click:true|second|listener|true|listener"
    );
}
#[test]
fn document_point_queries_use_real_paint_order_geometry() {
    let mut vm = new_parsed_test_vm(
        "https://document-point-query.test/path/index.html",
        r#"<html><body>
            <div id="target" style="width:100px;height:50px">target</div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => JSON.stringify({
  element: document.elementFromPoint(10, 10)?.localName ?? null,
  elements: document.elementsFromPoint(10, 10).map(element => element.localName)
}))()
"#,
        )
        .expect("document point query probe should evaluate");

    assert_eq!(
        result,
        r#"{"element":"div","elements":["div","body","html"]}"#
    );
}
#[test]
fn document_point_queries_parse_webidl_coordinates() {
    let mut vm = new_parsed_test_vm(
        "https://document-point-query-webidl.test/path/index.html",
        r#"<html><body>
            <div id="target" style="width:100px;height:50px">target</div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.localName ? value.localName : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const arrayProbe = callback => {
    try {
      return callback().map((element) => element.localName).join(",");
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  return JSON.stringify({
    missingElement: probe(() => document.elementFromPoint()),
    missingElements: arrayProbe(() => document.elementsFromPoint(1)),
    symbolX: probe(() => document.elementFromPoint(Symbol(), 1)),
    symbolY: arrayProbe(() => document.elementsFromPoint(1, Symbol())),
    infinity: probe(() => document.elementFromPoint(Infinity, 1)),
    stringCoordinates: probe(() => document.elementFromPoint("10", "10")),
    objectCoordinates: arrayProbe(() => document.elementsFromPoint(
      { valueOf() { return 10; } },
      { valueOf() { return 10; } }
    ))
  });
})()
"#,
        )
        .expect("document point query WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"missingElement":"throw:TypeError","missingElements":"throw:TypeError","symbolX":"throw:TypeError","symbolY":"throw:TypeError","infinity":"throw:TypeError","stringCoordinates":"div","objectCoordinates":"div,body,html"}"#
    );
}
#[test]
fn shadow_root_point_queries_retarget_real_layout_hits_to_the_tree_scope() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-root-point-query.test/path/index.html",
        r#"<html><head><style>
            html, body { margin: 0; }
            #host, #inside { display: block; width: 100px; height: 50px; }
        </style></head><body><div id="host"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.getElementById('host');
  const shadow = host.attachShadow({ mode: 'closed' });
  shadow.innerHTML = '<span id="inside">text</span>';
  return [
    document.elementFromPoint(1, 1)?.id,
    document.elementsFromPoint(1, 1).map(element => element.id || element.localName).join(','),
    shadow.elementFromPoint(1, 1)?.id,
    shadow.elementsFromPoint(1, 1).map(element => element.id || element.localName).join(',')
  ].join('|');
})()
"#,
        )
        .expect("shadow root point queries should evaluate");

    assert_eq!(result, "host|host,body,html|inside|inside,host,body,html");
}
#[test]
fn xpath_evaluator_constructor_requires_new() {
    let mut vm = new_parsed_test_vm(
        "https://xpath-evaluator-constructor.test/",
        "<html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const evaluator = new XPathEvaluator();
  let bareCall;
  try {
    XPathEvaluator();
    bareCall = 'ok';
  } catch (error) {
    bareCall = error.name;
  }
  const host = document.createElement('div');
  const shadow = host.attachShadow({ mode: 'open' });
  const span = document.createElement('span');
  shadow.appendChild(span);
  document.body.appendChild(host);
  const shadowResult = evaluator.evaluate(
    '//span',
    span,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const parsed = new DOMParser().parseFromString(
    '<root><item id="detached"/></root>',
    'text/xml'
  );
  const detachedItem = parsed.documentElement.firstChild;
  const detachedResult = evaluator.evaluate(
    '//item',
    detachedItem,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  return [
    evaluator instanceof XPathEvaluator,
    Object.getPrototypeOf(evaluator) === XPathEvaluator.prototype,
    Object.prototype.toString.call(evaluator),
    XPathEvaluator.name,
    XPathEvaluator.length,
    bareCall,
    typeof evaluator.evaluate,
    evaluator.evaluate.length,
    shadowResult.singleNodeValue === span,
    detachedResult.singleNodeValue === detachedItem
  ].join('|');
})()
"#,
        )
        .expect("XPathEvaluator constructor probe should evaluate");

    assert_eq!(
        result,
        "true|true|[object XPathEvaluator]|XPathEvaluator|0|TypeError|function|2|true|true"
    );
}

#[test]
fn xpath_evaluator_create_ns_resolver_returns_node_identity() {
    let mut vm = new_parsed_test_vm(
        "https://xpath-evaluator-create-ns-resolver.test/",
        "<!doctype html><html><body>text</body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const evaluator = new XPathEvaluator();
  const fragment = document.createDocumentFragment();
  const attribute = document.createAttribute('data-probe');
  const nodes = [
    document,
    fragment,
    document.doctype,
    document.body,
    document.body.firstChild,
    attribute
  ];
  const errorName = callback => {
    try {
      callback();
      return 'missing';
    } catch (error) {
      return error.name;
    }
  };
  return JSON.stringify({
    identities: nodes.map(node => evaluator.createNSResolver(node) === node),
    elementXml: evaluator.createNSResolver(document.body).lookupNamespaceURI('xml'),
    documentXml: evaluator.createNSResolver(new Document()).lookupNamespaceURI('xml'),
    length: evaluator.createNSResolver.length,
    missing: errorName(() => evaluator.createNSResolver()),
    primitive: errorName(() => evaluator.createNSResolver(1))
  });
})()
"#,
        )
        .expect("XPathEvaluator createNSResolver probe should evaluate");

    assert_eq!(
        result,
        r#"{"identities":[true,true,true,true,true,true],"elementXml":"http://www.w3.org/XML/1998/namespace","documentXml":null,"length":1,"missing":"TypeError","primitive":"TypeError"}"#
    );
}

#[test]
fn document_xpath_queries_parse_webidl_arguments() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-webidl.test/path/index.html",
        r#"<html><body>
            <section><div id="first"></div><div id="second"></div></section>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const ids = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const wrapped = document.evaluate(
    { toString() { return "//div[@id='second']"; } },
    document,
    null,
    { valueOf() { return XPathResult.FIRST_ORDERED_NODE_TYPE; } }
  );
  const existing = document.evaluate(
    "//div[@id='first']",
    document,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const withExisting = document.evaluate(
    "//div[@id='second']",
    document,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE,
    existing
  );
  const parsed = new DOMParser().parseFromString(
    "<main><div id='detached-first'></div><div id='detached-second'></div></main>",
    "text/html"
  );
  const detachedExisting = parsed.evaluate(
    "//div[@id='detached-first']",
    parsed,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const detachedWithExisting = parsed.evaluate(
    "//div[@id='detached-second']",
    parsed,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE,
    detachedExisting
  );
  const liveResolverNode = document.body.firstElementChild;
  const detachedResolverNode = parsed.documentElement;
  return JSON.stringify({
    first: ids.snapshotItem(0).id,
    wrappedExpressionAndType: wrapped.singleNodeValue.id,
    existingResultIgnored: withExisting !== existing,
    existingResultNode: withExisting.singleNodeValue.id,
    detachedExistingResultIgnored: detachedWithExisting !== detachedExisting,
    detachedExistingResultNode: detachedWithExisting.singleNodeValue.id,
    liveResolverIdentity: document.createNSResolver(liveResolverNode) === liveResolverNode,
    detachedResolverIdentity: parsed.createNSResolver(detachedResolverNode) === detachedResolverNode,
    createNSResolverLength: document.createNSResolver.length,
    missingResolverNode: probe(() => document.createNSResolver()),
    primitiveResolverNode: probe(() => document.createNSResolver(1)),
    missingExpression: probe(() => document.evaluate()),
    missingContext: probe(() => document.evaluate("//div")),
    symbolExpression: probe(() => document.evaluate(Symbol(), document)),
    symbolType: probe(() => document.evaluate("//div", document, null, Symbol())),
    primitiveExistingResult: probe(() => document.evaluate("//div", document, null, 0, 1)),
    symbolExistingResult: probe(() => document.evaluate("//div", document, null, 0, Symbol())),
    detachedPrimitiveExistingResult: probe(() => parsed.evaluate("//div", parsed, null, 0, 1)),
    unsupportedType: probe(() => document.evaluate("//div", document, null, 10)),
    missingSnapshotIndex: probe(() => ids.snapshotItem()),
    symbolSnapshotIndex: probe(() => ids.snapshotItem(Symbol())),
    outOfRangeSnapshotItem: ids.snapshotItem(ids.snapshotLength) === null,
    wrappedSnapshotIndex: ids.snapshotItem({ valueOf() { return 1; } }).id
  });
})()
"#,
        )
        .expect("document XPath WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"first":"first","wrappedExpressionAndType":"second","existingResultIgnored":true,"existingResultNode":"second","detachedExistingResultIgnored":true,"detachedExistingResultNode":"detached-second","liveResolverIdentity":true,"detachedResolverIdentity":true,"createNSResolverLength":1,"missingResolverNode":"throw:TypeError","primitiveResolverNode":"throw:TypeError","missingExpression":"throw:TypeError","missingContext":"throw:TypeError","symbolExpression":"throw:TypeError","symbolType":"throw:TypeError","primitiveExistingResult":"throw:TypeError","symbolExistingResult":"throw:TypeError","detachedPrimitiveExistingResult":"throw:TypeError","unsupportedType":"throw:NotSupportedError","missingSnapshotIndex":"throw:TypeError","symbolSnapshotIndex":"throw:TypeError","outOfRangeSnapshotItem":true,"wrappedSnapshotIndex":"second"}"#
    );
}
#[test]
fn document_xpath_evaluates_live_detached_context_nodes() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-live-context.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement("article");
  const span = document.createElement("span");
  span.id = "detached";
  span.setAttribute("data-kind", "target");
  span.textContent = "Detached text";
  host.appendChild(span);

  const nodeResult = document.evaluate(
    ".//span[@data-kind='target']",
    host,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  );
  const stringResult = document.evaluate(
    "string(.//span)",
    host,
    null,
    XPathResult.STRING_TYPE
  );

  return JSON.stringify({
    sameNode: nodeResult.singleNodeValue === span,
    stringValue: stringResult.stringValue
  });
})()
"#,
        )
        .expect("live detached XPath context should evaluate");

    assert_eq!(result, r#"{"sameNode":true,"stringValue":"Detached text"}"#);
}
#[test]
fn document_xpath_live_iterators_track_dom_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-iterator-mutation.test/path/index.html",
        r#"<html><body><div id="first"></div><div id="second"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const iterator = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const first = iterator.iterateNext().id;
  document.body.appendChild(document.createElement("div"));

  const attrIterator = document.evaluate(
    "//div[@id='first']",
    document,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  document.getElementById("first").setAttribute("data-mutated", "yes");

  return JSON.stringify({
    first,
    iteratorInvalid: iterator.invalidIteratorState,
    iteratorAfterTreeMutation: probe(() => iterator.iterateNext()),
    snapshotInvalid: snapshot.invalidIteratorState,
    snapshotLength: snapshot.snapshotLength,
    snapshotSecond: snapshot.snapshotItem(1).id,
    attrIteratorInvalid: attrIterator.invalidIteratorState,
    attrIteratorAfterMutation: probe(() => attrIterator.iterateNext())
  });
})()
"#,
        )
        .expect("live XPath iterators should observe DOM mutations");

    assert_eq!(
        result,
        r#"{"first":"first","iteratorInvalid":true,"iteratorAfterTreeMutation":"throw:InvalidStateError","snapshotInvalid":false,"snapshotLength":2,"snapshotSecond":"second","attrIteratorInvalid":true,"attrIteratorAfterMutation":"throw:InvalidStateError"}"#
    );
}
#[test]
fn document_xpath_detached_iterators_track_domparser_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-detached-iterator-mutation.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const parsed = new DOMParser().parseFromString(
    "<main><div id='first'></div><div id='second'></div></main>",
    "text/html"
  );
  const iterator = parsed.evaluate(
    "//div",
    parsed,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = parsed.evaluate(
    "//div",
    parsed,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const first = iterator.iterateNext().id;
  parsed.body.appendChild(parsed.createElement("div"));

  const attrIterator = parsed.evaluate(
    "//div[@id='first']",
    parsed,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  parsed.getElementById("first").setAttribute("data-mutated", "yes");

  return JSON.stringify({
    first,
    iteratorInvalid: iterator.invalidIteratorState,
    iteratorAfterTreeMutation: probe(() => iterator.iterateNext()),
    snapshotInvalid: snapshot.invalidIteratorState,
    snapshotLength: snapshot.snapshotLength,
    snapshotSecond: snapshot.snapshotItem(1).id,
    attrIteratorInvalid: attrIterator.invalidIteratorState,
    attrIteratorAfterMutation: probe(() => attrIterator.iterateNext())
  });
})()
"#,
        )
        .expect("detached DOMParser XPath iterators should observe DOM mutations");

    assert_eq!(
        result,
        r#"{"first":"first","iteratorInvalid":true,"iteratorAfterTreeMutation":"throw:InvalidStateError","snapshotInvalid":false,"snapshotLength":2,"snapshotSecond":"second","attrIteratorInvalid":true,"attrIteratorAfterMutation":"throw:InvalidStateError"}"#
    );
}
#[test]
fn document_xpath_detached_object_tree_iterators_track_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-object-tree-iterator-mutation.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const doc = document.implementation.createHTMLDocument("");
  const first = doc.createElement("div");
  first.id = "first";
  const second = doc.createElement("div");
  second.id = "second";
  const text = doc.createTextNode("before");
  second.appendChild(text);
  doc.body.appendChild(first);
  doc.body.appendChild(second);

  const treeIterator = doc.evaluate(
    "//div",
    doc,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = doc.evaluate(
    "//div",
    doc,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const firstId = treeIterator.iterateNext().id;
  doc.body.appendChild(doc.createElement("div"));

  const attrIterator = doc.evaluate(
    "//div[@id='first']",
    doc,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  first.setAttribute("data-mutated", "yes");

  const textIterator = doc.evaluate(
    "//div[text()='before']",
    doc,
    null,
    XPathResult.UNORDERED_NODE_ITERATOR_TYPE
  );
  text.data = "after";

  return JSON.stringify({
    firstId,
    treeInvalid: treeIterator.invalidIteratorState,
    treeAfterMutation: probe(() => treeIterator.iterateNext()),
    snapshotInvalid: snapshot.invalidIteratorState,
    snapshotLength: snapshot.snapshotLength,
    snapshotSecond: snapshot.snapshotItem(1).id,
    attrInvalid: attrIterator.invalidIteratorState,
    attrAfterMutation: probe(() => attrIterator.iterateNext()),
    textInvalid: textIterator.invalidIteratorState,
    textAfterMutation: probe(() => textIterator.iterateNext())
  });
})()
"#,
        )
        .expect("detached object-tree XPath iterators should observe DOM mutations");

    assert_eq!(
        result,
        r#"{"firstId":"first","treeInvalid":true,"treeAfterMutation":"throw:InvalidStateError","snapshotInvalid":false,"snapshotLength":2,"snapshotSecond":"second","attrInvalid":true,"attrAfterMutation":"throw:InvalidStateError","textInvalid":true,"textAfterMutation":"throw:InvalidStateError"}"#
    );
}
#[test]
fn document_xpath_result_accessors_are_type_specific() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-result-accessors.test/path/index.html",
        r#"<html><body><div id="first"></div><div id="second"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && value.id ? value.id : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const number = document.evaluate("count(//div)", document, null, XPathResult.NUMBER_TYPE);
  const string = document.evaluate("string(//div[@id='first']/@id)", document, null, XPathResult.STRING_TYPE);
  const boolean = document.evaluate("count(//div) = 2", document, null, XPathResult.BOOLEAN_TYPE);
  const nodeSetBoolean = document.evaluate("//div", document, null, XPathResult.BOOLEAN_TYPE);
  const emptyNodeSetBoolean = document.evaluate("//missing", document, null, XPathResult.BOOLEAN_TYPE);
  const single = document.evaluate("//div[@id='first']", document, null, XPathResult.FIRST_ORDERED_NODE_TYPE);
  const iterator = document.evaluate("//div", document, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE);
  const snapshot = document.evaluate("//div", document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE);
  const nodeSetBooleanInvalidBefore = nodeSetBoolean.invalidIteratorState;
  const nodeSetBooleanIterateBefore = probe(() => nodeSetBoolean.iterateNext());
  document.body.appendChild(document.createElement("div"));

  return JSON.stringify({
    numberValue: number.numberValue,
    stringValue: string.stringValue,
    booleanValue: boolean.booleanValue,
    nodeSetBooleanType: nodeSetBoolean.resultType,
    nodeSetBooleanValue: nodeSetBoolean.booleanValue,
    emptyNodeSetBooleanValue: emptyNodeSetBoolean.booleanValue,
    nodeSetBooleanInvalidBefore,
    nodeSetBooleanInvalidAfter: nodeSetBoolean.invalidIteratorState,
    nodeSetBooleanIterateBefore,
    nodeSetBooleanIterateAfter: probe(() => nodeSetBoolean.iterateNext()),
    singleNodeValue: single.singleNodeValue.id,
    snapshotLength: snapshot.snapshotLength,
    numberStringValue: probe(() => number.stringValue),
    stringNumberValue: probe(() => string.numberValue),
    booleanSingleNodeValue: probe(() => boolean.singleNodeValue),
    singleSnapshotLength: probe(() => single.snapshotLength),
    snapshotIterateNext: probe(() => snapshot.iterateNext()),
    iteratorSnapshotItem: probe(() => iterator.snapshotItem(0)),
    snapshotSingleNodeValue: probe(() => snapshot.singleNodeValue)
  });
})()
"#,
        )
        .expect("XPathResult type-specific accessors should evaluate");

    assert_eq!(
        result,
        r#"{"numberValue":2,"stringValue":"first","booleanValue":true,"nodeSetBooleanType":3,"nodeSetBooleanValue":true,"emptyNodeSetBooleanValue":false,"nodeSetBooleanInvalidBefore":false,"nodeSetBooleanInvalidAfter":false,"nodeSetBooleanIterateBefore":"throw:TypeError","nodeSetBooleanIterateAfter":"throw:TypeError","singleNodeValue":"first","snapshotLength":2,"numberStringValue":"throw:TypeError","stringNumberValue":"throw:TypeError","booleanSingleNodeValue":"throw:TypeError","singleSnapshotLength":"throw:TypeError","snapshotIterateNext":"throw:TypeError","iteratorSnapshotItem":"throw:TypeError","snapshotSingleNodeValue":"throw:TypeError"}"#
    );
}
#[test]
fn document_xpath_result_uses_browser_like_object_shape() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-result-shape.test/path/index.html",
        r#"<html><body><div id="first"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const iterator = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const proto = XPathResult.prototype;
  const descriptorShape = descriptor => [
    typeof descriptor.value,
    descriptor.value.name,
    descriptor.value.length,
    descriptor.enumerable,
    descriptor.configurable,
    descriptor.writable
  ];
  const iterateDescriptor = Object.getOwnPropertyDescriptor(proto, "iterateNext");
  const snapshotDescriptor = Object.getOwnPropertyDescriptor(proto, "snapshotItem");
  const tagDescriptor = Object.getOwnPropertyDescriptor(proto, Symbol.toStringTag);
  return JSON.stringify({
    constructorType: typeof XPathResult,
    constructorName: XPathResult.name,
    constructorLength: XPathResult.length,
    illegalConstructor: probe(() => new XPathResult()),
    tag: Object.prototype.toString.call(iterator),
    instanceofXPathResult: iterator instanceof XPathResult,
    prototypeMatch: Object.getPrototypeOf(iterator) === proto,
    constructorOnPrototype: proto.constructor === XPathResult,
    ownKeys: Object.keys(iterator),
    ownIterateNext: Object.prototype.hasOwnProperty.call(iterator, "iterateNext"),
    ownResultType: Object.prototype.hasOwnProperty.call(iterator, "resultType"),
    resultTypeValue: iterator.resultType,
    invalidIteratorStateValue: iterator.invalidIteratorState,
    prototypeIterateNext: descriptorShape(iterateDescriptor),
    prototypeSnapshotItem: descriptorShape(snapshotDescriptor),
    prototypeToStringTag: [
      tagDescriptor.value,
      tagDescriptor.enumerable,
      tagDescriptor.configurable,
      tagDescriptor.writable
    ],
    prototypeResultTypePresent: "resultType" in proto,
    constructorConstantsEnumerable: Object.keys(XPathResult).includes("ORDERED_NODE_ITERATOR_TYPE"),
    prototypeConstantsEnumerable: Object.keys(proto).includes("ORDERED_NODE_ITERATOR_TYPE"),
    prototypeMethodsEnumerable: [
      Object.keys(proto).includes("iterateNext"),
      Object.keys(proto).includes("snapshotItem")
    ],
    constructorConstant: XPathResult.ORDERED_NODE_ITERATOR_TYPE,
    prototypeConstant: proto.ORDERED_NODE_ITERATOR_TYPE,
    windowEnumerable: Object.keys(window).includes("XPathResult")
  });
})()
"#,
        )
        .expect("XPathResult object shape should evaluate");

    assert_eq!(
        result,
        r#"{"constructorType":"function","constructorName":"XPathResult","constructorLength":0,"illegalConstructor":"throw:TypeError","tag":"[object XPathResult]","instanceofXPathResult":true,"prototypeMatch":true,"constructorOnPrototype":true,"ownKeys":[],"ownIterateNext":false,"ownResultType":false,"resultTypeValue":5,"invalidIteratorStateValue":false,"prototypeIterateNext":["function","iterateNext",0,true,true,true],"prototypeSnapshotItem":["function","snapshotItem",0,true,true,true],"prototypeToStringTag":["XPathResult",false,true,false],"prototypeResultTypePresent":true,"constructorConstantsEnumerable":true,"prototypeConstantsEnumerable":true,"prototypeMethodsEnumerable":[true,true],"constructorConstant":5,"prototypeConstant":5,"windowEnumerable":false}"#
    );
}

#[test]
fn document_xpath_result_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-result-private-slots.test/path/index.html",
        r#"<html><body><div id="first"></div><div id="second"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return callback();
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith("__moliXPath"))
    .sort();
  const iterator = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_ITERATOR_TYPE
  );
  const snapshot = document.evaluate(
    "//div",
    document,
    null,
    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE
  );
  const number = document.evaluate(
    "count(//div)",
    document,
    null,
    XPathResult.NUMBER_TYPE
  );
  const internalNamesBefore = {
    iterator: internalNames(iterator),
    snapshot: internalNames(snapshot),
    number: internalNames(number)
  };
  Object.assign(iterator, {
    __moliXPathType: XPathResult.NUMBER_TYPE,
    __moliXPathNumberValue: 99,
    __moliXPathNodes: [],
    __moliXPathIndex: 99
  });
  Object.assign(snapshot, {
    __moliXPathType: XPathResult.STRING_TYPE,
    __moliXPathStringValue: "spoofed",
    __moliXPathSnapshotLength: 99,
    __moliXPathNodes: []
  });
  Object.assign(number, {
    __moliXPathType: XPathResult.STRING_TYPE,
    __moliXPathStringValue: "spoofed"
  });
  const proto = XPathResult.prototype;
  const fake = {
    __moliXPathType: XPathResult.STRING_TYPE,
    __moliXPathStringValue: "fake",
    __moliXPathSnapshotLength: 9,
    __moliXPathNodes: [document.body],
    __moliXPathIndex: 0
  };
  return JSON.stringify({
    internalNamesBefore,
    iteratorType: iterator.resultType,
    iteratorInvalid: iterator.invalidIteratorState,
    iteratorNext: iterator.iterateNext().id,
    snapshotLength: snapshot.snapshotLength,
    snapshotFirst: snapshot.snapshotItem(0).id,
    numberValue: number.numberValue,
    fakeResultType: Object.getOwnPropertyDescriptor(proto, "resultType").get.call(fake),
    fakeStringValue: probe(() => Object.getOwnPropertyDescriptor(proto, "stringValue").get.call(fake)),
    fakeSnapshotItem: probe(() => proto.snapshotItem.call(fake, 0))
  });
})()
"#,
        )
        .expect("XPathResult private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"internalNamesBefore":{"iterator":[],"snapshot":[],"number":[]},"iteratorType":5,"iteratorInvalid":false,"iteratorNext":"first","snapshotLength":2,"snapshotFirst":"first","numberValue":2,"fakeResultType":0,"fakeStringValue":"throw:TypeError","fakeSnapshotItem":"throw:TypeError"}"#
    );
}

#[test]
fn document_xpath_maps_attribute_node_results() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-attribute-result.test/path/index.html",
        r#"<html><body><section id="root" data-live="yes"></section></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.getElementById("root");
  const liveAttr = document.evaluate(
    "//@data-live",
    document,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const parsed = new DOMParser().parseFromString(
    "<main><p id='p' data-detached='yes'></p></main>",
    "text/html"
  );
  const detachedAttr = parsed.evaluate(
    "//@data-detached",
    parsed,
    null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  return JSON.stringify({
    liveName: liveAttr && liveAttr.name,
    liveValue: liveAttr && liveAttr.value,
    liveOwner: liveAttr && liveAttr.ownerElement === root,
    detachedName: detachedAttr && detachedAttr.name,
    detachedValue: detachedAttr && detachedAttr.value,
    detachedOwner: detachedAttr && detachedAttr.ownerElement === parsed.getElementById("p")
  });
})()
"#,
        )
        .expect("XPath attribute node results should evaluate");

    assert_eq!(
        result,
        r#"{"liveName":"data-live","liveValue":"yes","liveOwner":true,"detachedName":"data-detached","detachedValue":"yes","detachedOwner":true}"#
    );
}

#[test]
fn document_xpath_lang_uses_namespaced_attribute_local_names() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-lang.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parsed = new DOMParser().parseFromString(
    "<root xml:lang='ja'><inherited/><specific xml:lang='en-US'/></root>",
    "text/xml"
  );
  const inherited = parsed.documentElement.firstChild;
  const specific = inherited.nextSibling;
  const evaluateLang = (expression, node) => parsed.evaluate(
    expression,
    node,
    null,
    XPathResult.BOOLEAN_TYPE
  ).booleanValue;

  return JSON.stringify({
    inheritedJapanese: evaluateLang('lang("ja")', inherited),
    specificEnglish: evaluateLang('lang("en")', specific),
    specificOverridesJapanese: evaluateLang('lang("ja")', specific)
  });
})()
"#,
        )
        .expect("XPath lang() should inspect namespaced attribute local names");

    assert_eq!(
        result,
        r#"{"inheritedJapanese":true,"specificEnglish":true,"specificOverridesJapanese":false}"#
    );
}
#[test]
fn document_xpath_uses_namespace_resolver_callbacks() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-namespace-resolver.test/path/index.html",
        r#"<html><body><svg id="liveSvg"><g id="liveGroup"></g></svg></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const svgNs = "http://www.w3.org/2000/svg";
  const liveByFunction = document.evaluate(
    "//svg:svg",
    document,
    prefix => prefix === "svg" ? svgNs : null,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const parsed = new DOMParser().parseFromString(
    "<svg xmlns='http://www.w3.org/2000/svg' id='detachedSvg'><g id='detachedGroup'/></svg>",
    "image/svg+xml"
  );
  const detachedByObject = parsed.evaluate(
    "//svg:g",
    parsed,
    { lookupNamespaceURI(prefix) { return prefix === "svg" ? svgNs : null; } },
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const callError = { kind: "call" };
  const getError = { kind: "get" };
  const coercionError = { kind: "coercion" };
  const reported = [];
  const reportLabels = new Map([
    [callError, "call"],
    [getError, "get"],
    [coercionError, "coercion"]
  ]);
  const onError = event => {
    reported.push(reportLabels.get(event.error) || (event.error && event.error.name));
    event.preventDefault();
  };
  window.addEventListener("error", onError);

  const unresolvedPrefix = probe(() =>
    document.evaluate("//missing:svg", document, () => null)
  );
  const resolverCallException = probe(() =>
    document.evaluate("//svg:svg", document, () => { throw callError; })
  );
  const resolverGetException = probe(() =>
    document.evaluate("//svg:svg", document, {
      get lookupNamespaceURI() { throw getError; }
    })
  );
  const truthyNonCallable = probe(() =>
    document.evaluate("//svg:svg", document, { lookupNamespaceURI: {} })
  );
  const falsyNonCallable = probe(() =>
    document.evaluate("//svg:svg", document, {})
  );
  const undefinedResult = probe(() =>
    document.evaluate("//svg:svg", document, () => undefined)
  );
  const nullResult = probe(() =>
    document.evaluate("//svg:svg", document, () => null)
  );
  const numberResult = probe(() =>
    document.evaluate("//svg:svg", document, () => 0)
  );
  const booleanResult = probe(() =>
    document.evaluate("//svg:svg", document, () => false)
  );
  const symbolResult = probe(() =>
    document.evaluate("//svg:svg", document, () => Symbol())
  );
  const coercionException = probe(() =>
    document.evaluate("//svg:svg", document, () => ({
      toString() { throw coercionError; },
      valueOf() { throw new Error("valueOf must not be called"); }
    }))
  );
  const detachedUnresolvedPrefix = probe(() =>
    parsed.evaluate("//missing:svg", parsed, null)
  );
  const invalidResolver = probe(() =>
    document.evaluate("//svg:svg", document, 1)
  );
  window.removeEventListener("error", onError);

  return JSON.stringify({
    liveId: liveByFunction && liveByFunction.id,
    detachedId: detachedByObject && detachedByObject.id,
    unresolvedPrefix,
    resolverCallException,
    resolverGetException,
    truthyNonCallable,
    falsyNonCallable,
    undefinedResult,
    nullResult,
    numberResult,
    booleanResult,
    symbolResult,
    coercionException,
    detachedUnresolvedPrefix,
    invalidResolver,
    reported
  });
})()
"#,
        )
        .expect("XPath namespace resolver callbacks should evaluate");

    assert_eq!(
        result,
        r#"{"liveId":"liveSvg","detachedId":"detachedGroup","unresolvedPrefix":"throw:NamespaceError","resolverCallException":"throw:NamespaceError","resolverGetException":"throw:NamespaceError","truthyNonCallable":"throw:NamespaceError","falsyNonCallable":"throw:NamespaceError","undefinedResult":"throw:NamespaceError","nullResult":"throw:NamespaceError","numberResult":"ok","booleanResult":"ok","symbolResult":"throw:NamespaceError","coercionException":"throw:NamespaceError","detachedUnresolvedPrefix":"throw:NamespaceError","invalidResolver":"throw:TypeError","reported":["call","get","TypeError","TypeError","TypeError","coercion"]}"#
    );
}

#[test]
fn document_xpath_resolver_uses_webidl_callback_interface_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-resolver-callback-interface.test/",
        r#"<html><body><svg id="target"></svg></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const svgNs = "http://www.w3.org/2000/svg";
  const expression = "//svg:svg";

  let callableThis = "unset";
  let callableCalls = 0;
  let forbiddenOperationGets = 0;
  function callableResolver(prefix) {
    "use strict";
    callableThis = this;
    callableCalls++;
    return prefix === "svg" ? svgNs : null;
  }
  Object.defineProperty(callableResolver, "lookupNamespaceURI", {
    get() {
      forbiddenOperationGets++;
      throw new Error("the callable branch must not look up the operation");
    }
  });
  const callableResult = document.evaluate(
    expression,
    document,
    callableResolver,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  let operationGets = 0;
  let operationCalls = 0;
  let objectReceivers = 0;
  const objectResolver = {
    get lookupNamespaceURI() {
      operationGets++;
      return function(prefix) {
        operationCalls++;
        objectReceivers += this === objectResolver;
        return prefix === "svg" ? svgNs : null;
      };
    }
  };
  document.evaluate(expression, document, objectResolver);
  document.evaluate(expression, document, objectResolver);

  const replaceableResolver = {
    lookupNamespaceURI() {
      return null;
    }
  };
  let beforeReplacement;
  try {
    document.evaluate(expression, document, replaceableResolver);
    beforeReplacement = "ok";
  } catch (error) {
    beforeReplacement = error.name;
  }
  replaceableResolver.lookupNamespaceURI = () => svgNs;
  const afterReplacement = document.evaluate(
    expression,
    document,
    replaceableResolver,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  const revocable = Proxy.revocable(() => svgNs, {});
  const proxyBeforeRevoke = document.evaluate(
    expression,
    document,
    revocable.proxy,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;
  revocable.revoke();
  let reportedRevokedTypeError = false;
  const onError = event => {
    reportedRevokedTypeError = event.error instanceof TypeError;
    event.preventDefault();
  };
  window.addEventListener("error", onError);
  let revokedResult;
  try {
    document.evaluate(expression, document, revocable.proxy);
    revokedResult = "ok";
  } catch (error) {
    revokedResult = error.name;
  }
  window.removeEventListener("error", onError);

  return JSON.stringify({
    callableId: callableResult && callableResult.id,
    callableThisIsUndefined: callableThis === undefined,
    callableCalls,
    forbiddenOperationGets,
    operationGets,
    operationCalls,
    objectReceivers,
    beforeReplacement,
    afterReplacementId: afterReplacement && afterReplacement.id,
    proxyBeforeRevokeId: proxyBeforeRevoke && proxyBeforeRevoke.id,
    revokedResult,
    reportedRevokedTypeError
  });
})()
"#,
        )
        .expect("XPath callback-interface invocation semantics should evaluate");

    assert_eq!(
        result,
        r#"{"callableId":"target","callableThisIsUndefined":true,"callableCalls":1,"forbiddenOperationGets":0,"operationGets":2,"operationCalls":2,"objectReceivers":2,"beforeReplacement":"NamespaceError","afterReplacementId":"target","proxyBeforeRevokeId":"target","revokedResult":"NamespaceError","reportedRevokedTypeError":true}"#
    );
}

#[test]
fn document_xpath_resolver_uses_callback_relevant_realm() {
    let mut vm = new_parsed_test_vm(
        "https://document-xpath-resolver-realm.test/",
        r#"<html><body><svg id="target"></svg></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__xpathResolverRealmFrame = iframe;
  return "ready";
})()
"#,
    )
    .expect("cross-Realm XPath resolver setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const other = globalThis.__xpathResolverRealmFrame.contentWindow;
  const expression = "//svg:svg";
  const missingOperation = new other.Object();
  let reported = null;
  const onError = event => {
    reported = {
      relevantTypeError:
        event.error instanceof other.TypeError &&
        !(event.error instanceof TypeError),
      targetIsResolverWindow: event.currentTarget === other
    };
    event.preventDefault();
  };
  other.addEventListener("error", onError);
  let evaluationFailure;
  try {
    document.evaluate(expression, document, missingOperation);
    evaluationFailure = "ok";
  } catch (error) {
    evaluationFailure = {
      name: error.name,
      evaluatorRealm:
        error instanceof DOMException &&
        !(error instanceof other.DOMException)
    };
  }
  other.removeEventListener("error", onError);

  globalThis.__xpathResolverExpectedRealm = other;
  globalThis.__xpathResolverCallFacts = [];
  const crossRealmCallable = other.Function(
    "prefix",
    `"use strict";
     parent.__xpathResolverCallFacts = [
       this === undefined,
       globalThis === parent.__xpathResolverExpectedRealm,
       prefix
     ];
     return "http://www.w3.org/2000/svg";`
  );
  const resolved = document.evaluate(
    expression,
    document,
    crossRealmCallable,
    XPathResult.FIRST_ORDERED_NODE_TYPE
  ).singleNodeValue;

  return JSON.stringify({
    evaluationFailure,
    reported,
    resolvedId: resolved && resolved.id,
    callFacts: globalThis.__xpathResolverCallFacts
  });
})()
"#,
        )
        .expect("cross-Realm XPath resolver invocation should evaluate");

    assert_eq!(
        result,
        r#"{"evaluationFailure":{"name":"NamespaceError","evaluatorRealm":true},"reported":{"relevantTypeError":true,"targetIsResolverWindow":true},"resolvedId":"target","callFacts":[true,true,"svg"]}"#
    );
}

#[test]
fn dom_selector_queries_parse_webidl_strings() {
    let mut vm = new_parsed_test_vm(
        "https://dom-selector-webidl.test/path/index.html",
        r#"<html><body>
            <section id="root">
              <div id="target" class="alpha beta" name="box"><span class="alpha"></span></div>
              <input id="field" name="field">
            </section>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r##"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      if (value && value.id) return value.id;
      if (value && typeof value.length === "number") return `length:${value.length}`;
      return String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const target = document.getElementById("target");
  const parsed = new DOMParser().parseFromString(
    "<main><p id='p' class='x'><span class='x'></span></p></main>",
    "text/html"
  );
  const detachedP = parsed.getElementById("p");
  return JSON.stringify({
    idObject: document.getElementById({ toString() { return "target"; } }).id,
    selectorObject: document.querySelector({ toString() { return "#target"; } }).id,
    selectorAllObject: document.querySelectorAll({ toString() { return ".alpha"; } }).length,
    matchesObject: target.matches({ toString() { return ".alpha"; } }),
    closestObject: target.closest({ toString() { return "section"; } }).id,
    tagObject: document.getElementsByTagName({ toString() { return "div"; } }).length,
    classObject: document.getElementsByClassName({ toString() { return "alpha"; } }).length,
    nameObject: document.getElementsByName({ toString() { return "field"; } })[0].id,
    tagNsObject: document.getElementsByTagNameNS(
      { toString() { return "*"; } },
      { toString() { return "div"; } }
    ).length,
    missingId: probe(() => document.getElementById()),
    symbolId: probe(() => document.getElementById(Symbol())),
    missingSelector: probe(() => document.querySelector()),
    symbolSelectorAll: probe(() => document.querySelectorAll(Symbol())),
    missingMatches: probe(() => target.matches()),
    symbolClosest: probe(() => target.closest(Symbol())),
    missingTag: probe(() => document.getElementsByTagName()),
    symbolClass: probe(() => document.getElementsByClassName(Symbol())),
    symbolName: probe(() => document.getElementsByName(Symbol())),
    missingNsLocal: probe(() => document.getElementsByTagNameNS("*")),
    symbolNs: probe(() => document.getElementsByTagNameNS(Symbol(), "div")),
    detachedIdObject: parsed.getElementById({ toString() { return "p"; } }).id,
    detachedSelectorObject: parsed.querySelector({ toString() { return "#p"; } }).id,
    detachedSelectorAllObject: detachedP.querySelectorAll({ toString() { return ".x"; } }).length,
    detachedTagObject: detachedP.getElementsByTagName({ toString() { return "span"; } }).length,
    detachedMissingId: probe(() => parsed.getElementById()),
    detachedSymbolSelector: probe(() => parsed.querySelector(Symbol())),
    detachedMissingSelectorAll: probe(() => detachedP.querySelectorAll()),
    detachedSymbolTag: probe(() => detachedP.getElementsByTagName(Symbol()))
  });
})()
"##,
        )
        .expect("DOM selector WebIDL probe should evaluate");

    assert_eq!(
        result,
        r#"{"idObject":"target","selectorObject":"target","selectorAllObject":2,"matchesObject":true,"closestObject":"root","tagObject":1,"classObject":2,"nameObject":"field","tagNsObject":1,"missingId":"throw:TypeError","symbolId":"throw:TypeError","missingSelector":"throw:TypeError","symbolSelectorAll":"throw:TypeError","missingMatches":"throw:TypeError","symbolClosest":"throw:TypeError","missingTag":"throw:TypeError","symbolClass":"throw:TypeError","symbolName":"throw:TypeError","missingNsLocal":"throw:TypeError","symbolNs":"throw:TypeError","detachedIdObject":"p","detachedSelectorObject":"p","detachedSelectorAllObject":1,"detachedTagObject":1,"detachedMissingId":"throw:TypeError","detachedSymbolSelector":"throw:TypeError","detachedMissingSelectorAll":"throw:TypeError","detachedSymbolTag":"throw:TypeError"}"#
    );
}
#[test]
fn reflected_id_lone_surrogates_use_the_lossy_dom_string_boundary() {
    let mut vm = new_parsed_test_vm(
        "https://dom-selector-surrogate-escapes.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r##"
(() => {
  const container = document.createElement("div");
  document.body.appendChild(container);

  const replacementFirst = document.createElement("span");
  replacementFirst.id = "\u{fffd}surrogateFirst";
  container.appendChild(replacementFirst);

  const surrogateFirst = document.createElement("span");
  surrogateFirst.id = "\ud83dsurrogateFirst";
  container.appendChild(surrogateFirst);

  const replacementSecond = document.createElement("span");
  replacementSecond.id = "surrogateSecond\u{fffd}";
  container.appendChild(replacementSecond);

  const surrogateSecond = document.createElement("span");
  surrogateSecond.id = "surrogateSecond\udd11";
  container.appendChild(surrogateSecond);

  return JSON.stringify({
    highProperty: surrogateFirst.id,
    highAttribute: surrogateFirst.getAttribute("id"),
    highMatchesReplacementEscape: surrogateFirst.matches("#\\d83d surrogateFirst"),
    escapedHighFindsFirstReplacement: container.querySelector("#\\d83d surrogateFirst") === replacementFirst,
    lowProperty: surrogateSecond.id,
    lowAttribute: surrogateSecond.getAttribute("id"),
    lowMatchesReplacementEscape: surrogateSecond.matches("#surrogateSecond\\dd11"),
    escapedLowFindsFirstReplacement: container.querySelector("#surrogateSecond\\dd11") === replacementSecond
  });
})()
"##,
        )
        .expect("surrogate selector probe should evaluate");

    assert_eq!(
        result,
        r#"{"highProperty":"�surrogateFirst","highAttribute":"�surrogateFirst","highMatchesReplacementEscape":true,"escapedHighFindsFirstReplacement":true,"lowProperty":"surrogateSecond�","lowAttribute":"surrogateSecond�","lowMatchesReplacementEscape":true,"escapedLowFindsFirstReplacement":true}"#
    );
}
#[test]
fn get_elements_by_tag_name_ns_matches_null_namespace_elements() {
    let mut vm = new_parsed_test_vm(
        "https://tag-name-ns-null.test/path/index.html",
        r#"<html><body><section id="root"></section></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.getElementById("root");
  const nullNs = document.createElementNS(null, "widget");
  nullNs.id = "nullNs";
  const htmlWidget = document.createElement("widget");
  htmlWidget.id = "htmlWidget";
  root.append(nullNs, htmlWidget);

  const xml = document.implementation.createDocument(null, "root");
  const detachedNullNs = xml.createElementNS(null, "widget");
  detachedNullNs.setAttribute("id", "detachedNullNs");
  const detachedHtmlNs = xml.createElementNS("http://www.w3.org/1999/xhtml", "widget");
  detachedHtmlNs.setAttribute("id", "detachedHtmlNs");
  xml.documentElement.append(detachedNullNs, detachedHtmlNs);

  const ids = collection => Array.from(collection).map(node => node.id).join(",");
  return JSON.stringify({
    liveNullNs: ids(document.getElementsByTagNameNS(null, "widget")),
    liveEmptyNs: ids(document.getElementsByTagNameNS("", "widget")),
    liveWildcardNs: ids(document.getElementsByTagNameNS("*", "widget")),
    liveElementNullNs: ids(root.getElementsByTagNameNS(null, "widget")),
    liveCaseSensitive: document.getElementsByTagNameNS(null, "WIDGET").length,
    detachedNullNs: ids(xml.getElementsByTagNameNS(null, "widget")),
    detachedEmptyNs: ids(xml.getElementsByTagNameNS("", "widget")),
    detachedWildcardNs: ids(xml.getElementsByTagNameNS("*", "widget")),
    detachedElementNullNs: ids(xml.documentElement.getElementsByTagNameNS(null, "widget")),
    detachedCaseSensitive: xml.getElementsByTagNameNS(null, "WIDGET").length
  });
})()
"#,
        )
        .expect("null namespace getElementsByTagNameNS probe should evaluate");

    assert_eq!(
        result,
        r#"{"liveNullNs":"nullNs","liveEmptyNs":"nullNs","liveWildcardNs":"nullNs,htmlWidget","liveElementNullNs":"nullNs","liveCaseSensitive":0,"detachedNullNs":"detachedNullNs","detachedEmptyNs":"detachedNullNs","detachedWildcardNs":"detachedNullNs,detachedHtmlNs","detachedElementNullNs":"detachedNullNs","detachedCaseSensitive":0}"#
    );
}
#[test]
fn file_constructor_preserves_declared_metadata_slots() {
    let mut vm = new_storage_test_vm("https://file-metadata-declaration.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptorReport = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      typeof descriptor?.set,
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(":");
  };
  const defaultFile = new File(["a"], "default.txt");
  const explicitFile = new File([new Uint8Array([104, 105])], "note.txt", {
    type: "text/plain",
    lastModified: 7
  });
  const weirdFile = new File(["x"], "weird.txt", { lastModified: Infinity });
  const dt = new DataTransfer();
  const item = dt.items.add(explicitFile);
  const roundTrip = item && item.getAsFile && item.getAsFile();
  const nameDescriptor = Object.getOwnPropertyDescriptor(File.prototype, "name");
  const lastModifiedDescriptor = Object.getOwnPropertyDescriptor(File.prototype, "lastModified");
  const lengthDescriptor = Object.getOwnPropertyDescriptor(FileList.prototype, "length");
  const fileInternalNames = Object.getOwnPropertyNames(explicitFile)
    .filter(name => name.startsWith("__lmFile"))
    .sort();
  const fileListInternalNames = Object.getOwnPropertyNames(dt.files)
    .filter(name => name.startsWith("__lmFile"))
    .sort();
  const fakeFile = {
    __lmFileName: "spoofed.txt",
    __lmFileLastModified: 99
  };
  const fakeFileList = { __lmFileListLength: 99 };
  return JSON.stringify({
    fileDescriptors: [
      descriptorReport(File.prototype, "name"),
      descriptorReport(File.prototype, "lastModified")
    ],
    fileListDescriptors: [
      descriptorReport(FileList.prototype, "length")
    ],
    fileInternalNames,
    fileListInternalNames,
    defaultName: defaultFile.name,
    defaultLastModifiedFinite: Number.isFinite(defaultFile.lastModified),
    explicitName: explicitFile.name,
    explicitLastModified: explicitFile.lastModified,
    explicitType: explicitFile.type,
    roundTripName: roundTrip && roundTrip.name,
    roundTripLastModified: roundTrip && roundTrip.lastModified,
    roundTripType: roundTrip && roundTrip.type,
    tag: Object.prototype.toString.call(explicitFile),
    ctor: explicitFile instanceof File,
    weirdLastModifiedFinite: Number.isFinite(weirdFile.lastModified),
    fakeName: nameDescriptor.get.call(fakeFile),
    fakeLastModifiedFinite: Number.isFinite(lastModifiedDescriptor.get.call(fakeFile)),
    fakeLength: lengthDescriptor.get.call(fakeFileList)
  });
})()
"#,
        )
        .expect("File metadata declaration should preserve script-visible slots");

    assert_eq!(
        result,
        r#"{"fileDescriptors":["name:function:get name:0:undefined:true:true","lastModified:function:get lastModified:0:undefined:true:true"],"fileListDescriptors":["length:function:get length:0:undefined:true:true"],"fileInternalNames":[],"fileListInternalNames":[],"defaultName":"default.txt","defaultLastModifiedFinite":true,"explicitName":"note.txt","explicitLastModified":7,"explicitType":"text/plain","roundTripName":"note.txt","roundTripLastModified":7,"roundTripType":"text/plain","tag":"[object File]","ctor":true,"weirdLastModifiedFinite":true,"fakeName":"","fakeLastModifiedFinite":true,"fakeLength":0}"#
    );
}
#[test]
fn data_transfer_and_input_files_support_playwright_style_upload_assignment() {
    let mut vm = new_storage_test_vm("https://input-files-upload.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const input = document.createElement('input');
  input.type = 'file';
  host.appendChild(input);

  const seen = [];
  input.addEventListener('input', () => {
    seen.push(`input:${input.files.length}:${input.files[0].name}:${input.value}`);
  });
  input.addEventListener('change', () => {
    seen.push(`change:${input.files[0].type}:${input.files[0].lastModified}`);
  });

  const dt = new DataTransfer();
  const file = new File([new Uint8Array([104, 105])], 'note.txt', {
    type: 'text/plain',
    lastModified: 7
  });
  dt.items.add(file);
  input.files = dt.files;
  input.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));

  return [
    typeof DataTransfer,
    dt.items.length,
    Object.prototype.toString.call(dt.files),
    input.files.length,
    input.files[0].name,
    input.files[0].type,
    input.files[0].lastModified,
    input.value,
    typeof input.files[Symbol.iterator],
    Array.from(input.files).map(file => file.name).join(','),
    seen.join(',')
  ].join('|');
})()
"#,
        )
        .expect("DataTransfer-backed file assignment should succeed");

    assert_eq!(
        result,
        "function|1|[object FileList]|1|note.txt|text/plain|7|C:\\fakepath\\note.txt|function|note.txt|input:1:note.txt:C:\\fakepath\\note.txt,change:text/plain:7"
    );
}

#[test]
fn input_files_nullable_setter_preserves_current_file_list() {
    let mut vm = new_storage_test_vm("https://input-files-nullable-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const input = document.createElement('input');
  input.type = 'file';
  const transfer = new DataTransfer();
  transfer.items.add(new File(['data'], 'note.txt'));
  input.files = transfer.files;
  const files = input.files;

  input.files = null;
  const afterNull = input.files === files && input.files[0].name;
  input.files = undefined;
  const afterUndefined = input.files === files && input.files[0].name;

  const assignmentError = value => {
    try {
      input.files = value;
      return 'none';
    } catch (error) {
      return error.name;
    }
  };
  return [
    afterNull,
    afterUndefined,
    assignmentError([]),
    assignmentError([new File([], 'other.txt')])
  ].join('|');
})()
"#,
        )
        .expect("nullable input files setter probe should evaluate");

    assert_eq!(result, "note.txt|note.txt|TypeError|TypeError");
}

#[test]
fn input_file_list_cache_refreshes_after_external_file_selection_replacement() {
    let mut vm = new_storage_test_vm("https://input-files-external-replace.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const input = document.createElement('input');
  input.id = 'upload';
  input.type = 'file';
  host.appendChild(input);
  globalThis.__emptyFiles = input.files;
  return [input.files.length, input.files === globalThis.__emptyFiles].join('|');
})()
"#,
        )
        .expect("initial file input cache probe should run");
    assert_eq!(initial, "0|true");

    let upload = vm
        .document_runtime
        .get_element_by_id("upload")
        .expect("upload input should exist");
    assert!(
        vm.set_file_input_files(
            upload,
            vec![crate::dom::native::SelectedFile {
                bytes: b"alpha".to_vec(),
                mime_type: "text/plain".to_owned(),
                name: "first.txt".to_owned(),
                last_modified: 1.0,
            }],
            false,
        )
        .expect("first external file selection should run")
    );
    let first = vm
        .eval(
            r#"
(() => {
  const input = document.getElementById('upload');
  globalThis.__firstFiles = input.files;
  return [
    input.files.length,
    input.files[0].name,
    input.files === globalThis.__emptyFiles,
    input.files === globalThis.__firstFiles
  ].join('|');
})()
"#,
        )
        .expect("first file input cache probe should run");
    assert_eq!(first, "1|first.txt|false|true");

    assert!(
        vm.set_file_input_files(
            upload,
            vec![crate::dom::native::SelectedFile {
                bytes: b"bravo".to_vec(),
                mime_type: "text/plain".to_owned(),
                name: "second.txt".to_owned(),
                last_modified: 2.0,
            }],
            false,
        )
        .expect("second external file selection should run")
    );
    let second = vm
        .eval(
            r#"
(() => {
  const input = document.getElementById('upload');
  const files = input.files;
  return [
    globalThis.__firstFiles[0].name,
    files.length,
    files[0].name,
    files === globalThis.__firstFiles,
    input.files === files
  ].join('|');
})()
"#,
        )
        .expect("second file input cache probe should run");
    assert_eq!(second, "first.txt|1|second.txt|false|true");
}
#[test]
fn data_transfer_item_list_add_returns_item_with_get_as_file() {
    let mut vm = new_storage_test_vm("https://data-transfer-item-add.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const dt = new DataTransfer();
  const file = new File(['hi'], 'note.txt', { type: 'text/plain', lastModified: 7 });
  const item = dt.items.add(file);
  const roundTrip = item && item.getAsFile && item.getAsFile();
  return [
    item !== null,
    item && item.kind,
    item && item.type,
    roundTrip && roundTrip.name,
    roundTrip && roundTrip.type,
    roundTrip && roundTrip.lastModified
  ].join('|');
})()
"#,
        )
        .expect("DataTransferItemList.add return value should evaluate");

    assert_eq!(result, "true|file|text/plain|note.txt|text/plain|7");
}

#[test]
fn constructed_data_transfer_defaults_to_none_effects() {
    let mut vm = new_storage_test_vm("https://data-transfer-constructor-defaults.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  return [transfer.dropEffect, transfer.effectAllowed].join('|');
})()
"#,
        )
        .expect("constructed DataTransfer defaults should evaluate");

    assert_eq!(result, "none|none");
}

#[test]
fn data_transfer_item_removals_disable_existing_wrappers() {
    let mut vm = new_storage_test_vm("https://data-transfer-item-disabled-mode.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const removedTransfer = new DataTransfer();
  const removedItem = removedTransfer.items.add(
    new File(['file'], 'removed.txt', { type: 'text/plain' })
  );
  removedTransfer.items.remove(0);

  const clearedTransfer = new DataTransfer();
  const clearedItem = clearedTransfer.items.add('cleared', 'text/plain');
  clearedTransfer.items.clear();
  let clearedCallbackCalled = false;
  clearedItem.getAsString(() => { clearedCallbackCalled = true; });

  const clearDataTransfer = new DataTransfer();
  const clearDataItem = clearDataTransfer.items.add('clear-data', 'text/plain');
  clearDataTransfer.clearData('text/plain');
  let clearDataCallbackCalled = false;
  clearDataItem.getAsString(() => { clearDataCallbackCalled = true; });

  return JSON.stringify({
    removed: [
      removedItem.kind,
      removedItem.type,
      removedItem.getAsFile() === null,
      removedItem.webkitGetAsEntry() === null,
      removedTransfer.items.length
    ],
    cleared: [
      clearedItem.kind,
      clearedItem.type,
      clearedCallbackCalled,
      clearedTransfer.items.length
    ],
    clearData: [
      clearDataItem.kind,
      clearDataItem.type,
      clearDataCallbackCalled,
      clearDataTransfer.items.length
    ]
  });
})()
"#,
        )
        .expect("removed DataTransferItem wrappers should enter disabled mode");

    assert_eq!(
        result,
        r#"{"removed":["","",true,true,0],"cleared":["","",false,0],"clearData":["","",false,0]}"#
    );
}
#[test]
fn data_transfer_item_and_list_use_stable_interface_wrappers() {
    let mut vm = new_storage_test_vm("https://data-transfer-item-wrapper.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const dt = new DataTransfer();
  const file = new File(['hi'], 'note.txt', { type: 'text/plain', lastModified: 7 });
  const item = dt.items.add(file);
  const indexed = dt.items[0];
  const fromItem = dt.items.item(0);
  return [
    item instanceof DataTransferItem,
    dt.items instanceof DataTransferItemList,
    indexed === item,
    fromItem === item,
    indexed && indexed.getAsFile && indexed.getAsFile().name,
    dt.items.length
  ].join('|');
})()
"#,
        )
        .expect("DataTransferItem wrappers should evaluate");

    assert_eq!(result, "true|true|true|true|note.txt|1");
}

#[test]
fn data_transfer_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://data-transfer-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const accessorDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    const setter = descriptor && descriptor.set;
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      typeof setter,
      setter ? setter.name : 'none',
      setter ? setter.length : 'none',
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(':');
  };
  const dt = new DataTransfer();
  dt.setData('text/plain', 'alpha');
  const file = new File(['hi'], 'note.txt', { type: 'text/plain', lastModified: 7 });
  const item = dt.items.add(file);
  const entry = item.webkitGetAsEntry();
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__lmDataTransfer') || name.startsWith('__lmFileSystem'))
    .sort();
  const ownNamesBefore = {
    dataTransfer: internalNames(dt),
    itemList: internalNames(dt.items),
    item: internalNames(item),
    entry: internalNames(entry)
  };

  DataTransfer.prototype.__lmDataTransferItems = dt.items;
  DataTransfer.prototype.__lmDataTransferTypes = ['prototype/type'];
  DataTransfer.prototype.__lmDataTransferDropEffect = 'copy';
  DataTransfer.prototype.__lmDataTransferEffectAllowed = 'all';
  DataTransferItemList.prototype.__lmDataTransferItemArray = [item, item];
  DataTransferItem.prototype.__lmDataTransferItemKind = 'string';
  DataTransferItem.prototype.__lmDataTransferItemType = 'prototype/type';
  DataTransferItem.prototype.__lmDataTransferItemFile = new File(['bad'], 'bad.txt');
  FileSystemEntry.prototype.__lmFileSystemEntryName = 'prototype.txt';
  FileSystemEntry.prototype.__lmFileSystemEntryFullPath = '/prototype.txt';
  FileSystemEntry.prototype.__lmFileSystemEntryIsFile = false;
  FileSystemEntry.prototype.__lmFileSystemEntryIsDirectory = true;

  const fakeDataTransfer = Object.create(DataTransfer.prototype);
  const fakeList = Object.create(DataTransferItemList.prototype);
  const fakeItem = Object.create(DataTransferItem.prototype);
  const fakeEntry = Object.create(FileSystemEntry.prototype);
  const entryNameGetter = Object.getOwnPropertyDescriptor(FileSystemEntry.prototype, 'name').get;
  const entryPathGetter = Object.getOwnPropertyDescriptor(FileSystemEntry.prototype, 'fullPath').get;
  const descriptors = {
    dataTransfer: [
      accessorDescriptor(DataTransfer.prototype, 'files'),
      accessorDescriptor(DataTransfer.prototype, 'items'),
      accessorDescriptor(DataTransfer.prototype, 'types'),
      accessorDescriptor(DataTransfer.prototype, 'dropEffect'),
      accessorDescriptor(DataTransfer.prototype, 'effectAllowed')
    ],
    itemList: [
      accessorDescriptor(DataTransferItemList.prototype, 'length')
    ],
    item: [
      accessorDescriptor(DataTransferItem.prototype, 'kind'),
      accessorDescriptor(DataTransferItem.prototype, 'type')
    ],
    entry: [
      accessorDescriptor(FileSystemEntry.prototype, 'filesystem'),
      accessorDescriptor(FileSystemEntry.prototype, 'fullPath'),
      accessorDescriptor(FileSystemEntry.prototype, 'isDirectory'),
      accessorDescriptor(FileSystemEntry.prototype, 'isFile'),
      accessorDescriptor(FileSystemEntry.prototype, 'name')
    ]
  };

  dt.__lmDataTransferItems = fakeList;
  dt.__lmDataTransferTypes = ['own/type'];
  dt.__lmDataTransferDropEffect = 'move';
  dt.__lmDataTransferEffectAllowed = 'copyMove';
  dt.items.__lmDataTransferItemArray = [item, item, item];
  dt.items.__lmDataTransferItemListIndexedLength = 9;
  item.__lmDataTransferItemKind = 'string';
  item.__lmDataTransferItemType = 'own/type';
  item.__lmDataTransferItemFile = new File(['bad'], 'bad.txt');
  entry.__lmFileSystemEntryName = 'own.txt';
  entry.__lmFileSystemEntryFullPath = '/own.txt';
  entry.__lmFileSystemEntryIsFile = false;
  entry.__lmFileSystemEntryIsDirectory = true;

  return JSON.stringify({
    ownNamesBefore,
    descriptors,
    real: [
      dt.getData('text/plain'),
      dt.types.join(','),
      dt.dropEffect,
      dt.effectAllowed,
      dt.items.length,
      item.kind,
      item.type,
      item.getAsFile().name,
      entry.name,
      entry.fullPath,
      entry.isFile,
      entry.isDirectory
    ].join('|'),
    fake: [
      DataTransfer.prototype.getData.call(fakeDataTransfer, 'text/plain'),
      fakeDataTransfer.types === undefined ? 'undefined' : fakeDataTransfer.types.join(','),
      fakeDataTransfer.dropEffect,
      fakeDataTransfer.effectAllowed,
      fakeList.length,
      fakeItem.kind,
      fakeItem.type,
      fakeItem.getAsFile(),
      entryNameGetter.call(fakeEntry),
      entryPathGetter.call(fakeEntry)
    ].map(value => value === null ? 'null' : String(value)).join('|')
  });
})()
"#,
        )
        .expect("DataTransfer declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"ownNamesBefore":{"dataTransfer":[],"itemList":[],"item":[],"entry":[]},"descriptors":{"dataTransfer":["files:function:get files:0:undefined:none:none:true:true","items:function:get items:0:undefined:none:none:true:true","types:function:get types:0:undefined:none:none:true:true","dropEffect:function:get dropEffect:0:function:set dropEffect:1:true:true","effectAllowed:function:get effectAllowed:0:function:set effectAllowed:1:true:true"],"itemList":["length:function:get length:0:undefined:none:none:true:true"],"item":["kind:function:get kind:0:undefined:none:none:true:true","type:function:get type:0:undefined:none:none:true:true"],"entry":["filesystem:function:get filesystem:0:undefined:none:none:true:true","fullPath:function:get fullPath:0:undefined:none:none:true:true","isDirectory:function:get isDirectory:0:undefined:none:none:true:true","isFile:function:get isFile:0:undefined:none:none:true:true","name:function:get name:0:undefined:none:none:true:true"]},"real":"alpha|text/plain,Files|none|none|2|file|text/plain|note.txt|note.txt|/note.txt|true|false","fake":"|undefined|none|uninitialized|0|||null||"}"#
    );
}

#[test]
fn data_transfer_directory_entries_use_private_slots_for_reflection_and_spoofing() {
    use crate::runtime::{RendererDragData, RendererDraggedDirectory, RendererDraggedFile};

    let mut vm = new_parsed_test_vm(
        "https://data-transfer-directory-slots.test/",
        r#"<html><body><div id="drop" style="width: 100px; height: 100px">drop</div></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  const target = document.getElementById('drop');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__lmDataTransfer') || name.startsWith('__lmFileSystem'))
    .sort();
  window.__directoryDropReport = 'missing';
  target.addEventListener('drop', event => {
    const item = event.dataTransfer.items[0];
    const entry = item.webkitGetAsEntry();
    const reader = entry.createReader();

    const ownNamesBefore = {
      dataTransfer: internalNames(event.dataTransfer),
      itemList: internalNames(event.dataTransfer.items),
      item: internalNames(item),
      entry: internalNames(entry),
      reader: internalNames(reader)
    };

    event.dataTransfer.__lmDataTransferItems = null;
    event.dataTransfer.__lmDataTransferTypes = ['own/type'];
    event.dataTransfer.items.__lmDataTransferItemArray = [];
    item.__lmDataTransferItemKind = 'string';
    item.__lmDataTransferItemType = 'own/type';
    entry.__lmFileSystemEntryName = 'own';
    entry.__lmFileSystemEntryFullPath = '/own';
    entry.__lmFileSystemEntryIsDirectory = false;
    entry.__lmFileSystemEntryIsFile = true;
    entry.__lmFileSystemDirectoryEntryEntries = [];
    reader.__lmFileSystemDirectoryReaderEntries = [];
    reader.__lmFileSystemDirectoryReaderOffset = 0;

    const reader2 = entry.createReader();

    window.__directoryDropReport = JSON.stringify({
      ownNamesBefore,
      transfer: [
        event.dataTransfer.types.join(','),
        event.dataTransfer.items.length,
        item.kind,
        item.type,
        entry.name,
        entry.fullPath,
        entry.isDirectory,
        entry.isFile
      ].join('|'),
      freshReaderOwnNames: internalNames(reader2)
    });
  });
})()
"#,
    )
    .expect("directory drop listener setup should evaluate");

    let drag_data = RendererDragData {
        items: Vec::new(),
        files: Vec::new(),
        directories: vec![RendererDraggedDirectory {
            name: "docs".to_owned(),
            files: vec![RendererDraggedFile {
                bytes: b"hello".to_vec(),
                mime_type: "text/plain".to_owned(),
                name: "child.txt".to_owned(),
                last_modified: 7.0,
            }],
            directories: vec![RendererDraggedDirectory {
                name: "nested".to_owned(),
                files: Vec::new(),
                directories: Vec::new(),
            }],
        }],
        drag_operations_mask: 1,
    };
    vm.dispatch_drag_event_at_point(10.0, 10.0, "drop", drag_data, 0)
        .expect("directory drop should dispatch");

    let result = vm
        .eval("window.__directoryDropReport")
        .expect("directory drop report should be readable");

    assert_eq!(
        result,
        r#"{"ownNamesBefore":{"dataTransfer":[],"itemList":[],"item":[],"entry":[],"reader":[]},"transfer":"Files|1|file||docs|/docs|true|false","freshReaderOwnNames":[]}"#
    );
}

#[test]
fn data_transfer_string_surface_and_drag_event_constructor_work() {
    let mut vm = new_storage_test_vm("https://drag-event-data-transfer.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const dt = new DataTransfer();
  dt.setData('text', 'alpha');
  const htmlItem = dt.items.add('<b>beta</b>', 'text/html');
  let stringPayload = '';
  htmlItem.getAsString(value => {
    stringPayload = value;
  });
  const event = new DragEvent('drop', {
    dataTransfer: dt,
    clientX: 12,
    clientY: 34
  });
  const typesBeforeClear = dt.types.join(',');
  dt.clearData('text/html');
  return [
    typeof DragEvent,
    event instanceof DragEvent,
    event instanceof MouseEvent,
    event.dataTransfer === dt,
    dt.getData('text/plain'),
    typesBeforeClear,
    dt.types.join(','),
    htmlItem.kind,
    htmlItem.type,
    stringPayload,
    dt.dropEffect,
    dt.effectAllowed,
    event.clientX,
    event.clientY,
    dt.items.length
  ].join('|');
})()
"#,
        )
        .expect("DragEvent/DataTransfer string surface should evaluate");

    assert_eq!(
        result,
        "function|true|true|true|alpha|text/plain,text/html|text/plain||||none|none|12|34|1"
    );
}

#[test]
fn drag_event_data_transfer_init_enforces_nullable_interface_conversion() {
    let mut vm = new_storage_test_vm("https://drag-event-data-transfer-conversion.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const outcome = dataTransfer => {
    try {
      new DragEvent('drop', { dataTransfer });
      return 'accepted';
    } catch (error) {
      return error.name;
    }
  };
  const transfer = new DataTransfer();
  const fakeTransfer = Object.create(DataTransfer.prototype);
  let getterError = 'missing';
  try {
    new DragEvent('drop', {
      get dataTransfer() {
        throw new RangeError('sentinel');
      }
    });
  } catch (error) {
    getterError = `${error.name}:${error.message}`;
  }
  return [
    new DragEvent('drop').dataTransfer === null,
    new DragEvent('drop', { dataTransfer: null }).dataTransfer === null,
    new DragEvent('drop', { dataTransfer: undefined }).dataTransfer === null,
    new DragEvent('drop', { dataTransfer: transfer }).dataTransfer === transfer,
    outcome({}),
    outcome(fakeTransfer),
    outcome(1),
    getterError
  ].join('|');
})()
"#,
        )
        .expect("DragEvent dataTransfer conversion should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|TypeError|TypeError|TypeError|RangeError:sentinel"
    );
}

#[test]
fn data_transfer_file_list_reference_tracks_item_mutations() {
    let mut vm = new_storage_test_vm("https://data-transfer-live-files.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  const files = transfer.files;
  const first = new File(["first"], "first.txt");
  const second = new File(["second"], "second.txt");
  transfer.items.add(first);
  transfer.items.add(second);
  const afterAdd = [
    transfer.files === files,
    files.length,
    files[0] === first,
    files.item(1) === second
  ].join(":");
  transfer.items.remove(0);
  const afterRemove = [
    transfer.files === files,
    files.length,
    files[0] === second,
    files[1] === undefined,
    files.item(1) === null
  ].join(":");
  return `${afterAdd}|${afterRemove}`;
})()
"#,
        )
        .expect("live DataTransfer FileList probe should evaluate");

    assert_eq!(result, "true:2:true:true|true:1:true:true:true");
}

#[test]
fn mouse_dragstart_bubbles_to_window_once() {
    let mut vm = new_parsed_test_vm(
        "https://dragstart-bubbles-once.test/",
        r#"<html><body><div id="drag" draggable="true">drag</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__dragStarts = 0;
  window.__dragStartLog = [];
  window.addEventListener('dragstart', event => {
    window.__dragStarts += 1;
    window.__dragStartLog.push([
      event.type,
      event.currentTarget === window,
      !!event.dataTransfer
    ].join(':'));
  });
})()
"#,
    )
    .expect("dragstart listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("mousemove should start drag");

    let result = vm
        .eval(
            r#"
(() => [window.__dragStarts, window.__dragStartLog.join('|')].join('|'))()
"#,
        )
        .expect("dragstart log should evaluate");
    assert_eq!(result, "1|dragstart:true:true");
}

#[test]
fn mouse_drop_requires_prevented_dragover() {
    let mut vm = new_parsed_test_vm(
        "https://drop-requires-prevented-dragover.test/",
        r#"<html><body><div id="drag" draggable="true">drag</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__allowDrop = false;
  window.__dragLog = [];
  window.__dropCount = 0;
  const drag = document.getElementById('drag');
  drag.addEventListener('dragstart', () => {
    window.__dragLog.push('dragstart');
  });
  drag.addEventListener('dragover', event => {
    window.__dragLog.push(`dragover:${window.__allowDrop}`);
    if (window.__allowDrop) {
      event.preventDefault();
    }
  });
  drag.addEventListener('drop', () => {
    window.__dropCount += 1;
    window.__dragLog.push('drop');
  });
})()
"#,
    )
    .expect("drag listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("first mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("first mousemove should start drag");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("first mouseup should finish drag");

    let first_result = vm
        .eval(
            r#"
(() => [window.__dropCount, window.__dragLog.join('|')].join('|'))()
"#,
        )
        .expect("first drag log should evaluate");
    assert_eq!(first_result, "0|dragstart|dragover:false");

    vm.eval("window.__allowDrop = true")
        .expect("drop permission flag should update");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("second mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("second mousemove should start drag");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("second mouseup should finish drag");

    let second_result = vm
        .eval(
            r#"
(() => [window.__dropCount, window.__dragLog.join('|')].join('|'))()
"#,
        )
        .expect("second drag log should evaluate");
    assert_eq!(
        second_result,
        "1|dragstart|dragover:false|dragstart|dragover:true|drop"
    );
}

#[test]
fn mouse_dragstart_prevent_default_cancels_drag_session() {
    let mut vm = new_parsed_test_vm(
        "https://dragstart-prevent-default-cancels.test/",
        r#"<html><body><div id="drag" draggable="true">drag</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__dragStarts = 0;
  window.__dragOvers = 0;
  window.__drops = 0;
  const drag = document.getElementById('drag');
  drag.addEventListener('dragstart', event => {
    window.__dragStarts += 1;
    event.preventDefault();
  });
  drag.addEventListener('dragover', event => {
    window.__dragOvers += 1;
    event.preventDefault();
  });
  drag.addEventListener('drop', () => {
    window.__drops += 1;
  });
})()
"#,
    )
    .expect("drag cancel listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("mousemove should attempt drag");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("mouseup should dispatch");

    let result = vm
        .eval(
            r#"
(() => [window.__dragStarts, window.__dragOvers, window.__drops].join('|'))()
"#,
        )
        .expect("drag cancel result should evaluate");
    assert_eq!(result, "1|0|0");
}

#[test]
fn mouse_dispatch_emits_pointer_event_properties() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-event-properties.test/",
        r#"<html><body><button id="target">tap</button></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__pointerLog = [];
  for (const type of ['pointerdown', 'pointerup']) {
    window.addEventListener(type, event => {
      window.__pointerLog.push([
        event.type,
        event.pointerType,
        event.pressure,
        event.tangentialPressure,
        event.tiltX,
        event.tiltY,
        event.twist,
        event.clientX,
        event.clientY,
        event.button,
        event.buttons
      ].join(':'));
    });
  }
})()
"#,
    )
    .expect("pointer listener setup should evaluate");

    vm.dispatch_mouse_event_at_point_with_pointer(
        20.0,
        20.0,
        "mousedown",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties {
            pointer_id: 1,
            pointer_type: "pen".to_owned(),
            pressure: 0.75,
            tangential_pressure: -0.25,
            tilt_x: 12.0,
            tilt_y: -8.0,
            twist: 45.0,
        },
    )
    .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point_with_pointer(
        20.0,
        20.0,
        "mouseup",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties {
            pointer_id: 1,
            pointer_type: "pen".to_owned(),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            twist: 0.0,
        },
    )
    .expect("mouseup should dispatch pointerup");

    let result = vm
        .eval("window.__pointerLog.join('|')")
        .expect("pointer log should evaluate");
    assert_eq!(
        result,
        "pointerdown:pen:0.75:-0.25:12:-8:45:20:20:0:1|pointerup:pen:0:0:0:0:0:20:20:0:0"
    );
}

#[test]
fn canceled_pointerdown_suppresses_compat_mouse_events_but_keeps_click() {
    let mut vm = new_parsed_test_vm(
        "https://pointerdown-suppresses-compat-mouse.test/",
        r#"<html><body><button id="target">tap</button></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target = document.getElementById('target');
  window.__compatLog = [];
  for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'click']) {
    target.addEventListener(type, event => {
      window.__compatLog.push(event.type);
      if (event.type === 'pointerdown') {
        event.preventDefault();
      }
    });
  }
})()
"#,
    )
    .expect("compat suppression listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should dispatch pointerup and click");

    let result = vm
        .eval("window.__compatLog.join('|')")
        .expect("compat log should evaluate");
    assert_eq!(result, "pointerdown|pointerup|click");
}

#[test]
fn pointer_capture_routes_mouse_pointer_until_pointerup() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-routes-mouse.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__captureStarted = false;
  window.__captureLog = [];
  for (const target of [target0, target1]) {
    for (const type of ['pointerdown', 'gotpointercapture', 'pointermove', 'pointerup', 'lostpointercapture']) {
      target.addEventListener(type, event => {
        if (event.type === 'pointermove' && !window.__captureStarted) {
          return;
        }
        window.__captureLog.push(`${event.type}@${target.id}`);
        if (event.type === 'pointermove' && target === target0) {
          window.__captureLog.push(`activeHas:${target0.hasPointerCapture(event.pointerId)}`);
        }
        if (event.type === 'pointerdown' && target === target0) {
          window.__captureStarted = true;
          target0.setPointerCapture(event.pointerId);
          window.__captureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
        }
      });
    }
  }
})()
"#,
    )
    .expect("pointer capture listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("captured pointermove should dispatch to capture target");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("captured pointerup should release capture");

    let result = vm
        .eval("window.__captureLog.join('|')")
        .expect("capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@target0|has:true|gotpointercapture@target0|pointermove@target0|activeHas:true|pointerup@target0|lostpointercapture@target0"
    );
}

#[test]
fn pointer_capture_lost_dispatches_before_compat_mouseup() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-lost-before-mouseup.test/",
        r#"<html><body><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  window.__captureMouseupOrder = [];
  for (const type of ['pointerdown', 'gotpointercapture', 'pointerup', 'lostpointercapture', 'mouseup']) {
    target0.addEventListener(type, event => {
      window.__captureMouseupOrder.push(event.type);
      if (event.type === 'pointerdown') {
        target0.setPointerCapture(event.pointerId);
      }
    });
  }
})()
"#,
    )
    .expect("pointer capture mouseup order listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should dispatch pointerup, lostpointercapture, mouseup");

    let result = vm
        .eval("window.__captureMouseupOrder.join('|')")
        .expect("capture mouseup order log should evaluate");
    assert_eq!(
        result,
        "pointerdown|gotpointercapture|pointerup|lostpointercapture|mouseup"
    );
}

#[test]
fn pointer_capture_mouse_events_preserve_modifiers() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-mouse-modifiers.test/",
        r#"<html><body><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  window.__captureModifierLog = [];
  for (const type of ['pointerdown', 'gotpointercapture', 'pointerup', 'lostpointercapture']) {
    target0.addEventListener(type, event => {
      window.__captureModifierLog.push(`${event.type}:${event.ctrlKey}:${event.shiftKey}:${event.altKey}:${event.metaKey}`);
      if (event.type === 'pointerdown') {
        target0.setPointerCapture(event.pointerId);
      }
    });
  }
})()
"#,
    )
    .expect("pointer capture modifier listener setup should evaluate");

    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        10.0,
        11.0,
        "mousedown",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties::default(),
        10,
    )
    .expect("mousedown should dispatch pointerdown with modifiers");
    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        10.0,
        11.0,
        "mouseup",
        0,
        None,
        0,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties::default(),
        10,
    )
    .expect("mouseup should dispatch pointerup and capture release with modifiers");

    let result = vm
        .eval("window.__captureModifierLog.join('|')")
        .expect("capture modifier log should evaluate");
    assert_eq!(
        result,
        "pointerdown:true:true:false:false|gotpointercapture:true:true:false:false|pointerup:true:true:false:false|lostpointercapture:true:true:false:false"
    );
}

#[test]
fn touch_pointer_implicit_capture_routes_until_pointerup() {
    let mut vm = new_parsed_test_vm(
        "https://touch-pointer-implicit-capture.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__touchImplicitCaptureLog = [];
  for (const target of [target0, target1]) {
    for (const type of ['pointerdown', 'gotpointercapture', 'pointermove', 'pointerup', 'lostpointercapture']) {
      target.addEventListener(type, event => {
        window.__touchImplicitCaptureLog.push(`${event.type}@${target.id}:${event.pointerType}`);
        if (event.type === 'pointerdown' && target === target0) {
          window.__touchImplicitCaptureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
        }
      });
    }
  }
})()
"#,
    )
    .expect("touch implicit capture listener setup should evaluate");

    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchstart", false)
        .expect("touchstart should dispatch pointerdown with implicit capture");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchmove", false)
        .expect("touchmove should route to implicit capture target");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchend", false)
        .expect("touchend should release implicit capture");

    let result = vm
        .eval("window.__touchImplicitCaptureLog.join('|')")
        .expect("touch implicit capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@target0:touch|has:true|gotpointercapture@target0:touch|pointermove@target0:touch|pointerup@target0:touch|lostpointercapture@target0:touch"
    );
}

#[test]
fn touch_pointer_capture_lost_dispatches_before_touchend() {
    let mut vm = new_parsed_test_vm(
        "https://touch-pointer-lost-before-touchend.test/",
        r#"<html><body><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  window.__touchEndOrder = [];
  for (const type of ['pointerdown', 'gotpointercapture', 'pointerup', 'lostpointercapture', 'touchend']) {
    target0.addEventListener(type, event => {
      window.__touchEndOrder.push(event.type);
    });
  }
})()
"#,
    )
    .expect("touch capture touchend order listener setup should evaluate");

    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchstart", false)
        .expect("touchstart should dispatch pointerdown");
    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchend", false)
        .expect("touchend should dispatch pointerup, lostpointercapture, touchend");

    let result = vm
        .eval("window.__touchEndOrder.join('|')")
        .expect("touchend order log should evaluate");
    assert_eq!(
        result,
        "pointerdown|gotpointercapture|pointerup|lostpointercapture|touchend"
    );
}

#[test]
fn touch_pointer_capture_can_route_to_explicit_capture_target() {
    let mut vm = new_parsed_test_vm(
        "https://touch-pointer-explicit-capture.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__touchExplicitCaptureLog = [];
  button.addEventListener('pointerdown', event => {
    window.__touchExplicitCaptureLog.push(`pointerdown@button:${event.pointerType}`);
    target0.setPointerCapture(event.pointerId);
    window.__touchExplicitCaptureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
  });
  button.addEventListener('pointermove', () => {
    window.__touchExplicitCaptureLog.push('pointermove@button');
  });
  target0.addEventListener('gotpointercapture', event => {
    window.__touchExplicitCaptureLog.push(`gotpointercapture@target0:${event.pointerType}`);
  });
  target0.addEventListener('pointermove', event => {
    window.__touchExplicitCaptureLog.push(`pointermove@target0:${event.pointerType}`);
  });
  target0.addEventListener('pointerup', event => {
    window.__touchExplicitCaptureLog.push(`pointerup@target0:${event.pointerType}`);
  });
  target0.addEventListener('lostpointercapture', event => {
    window.__touchExplicitCaptureLog.push(`lostpointercapture@target0:${event.pointerType}`);
  });
})()
"#,
    )
    .expect("touch explicit capture listener setup should evaluate");

    vm.dispatch_touch_event_at_point(10.0, 11.0, "touchstart", false)
        .expect("touchstart should dispatch pointerdown");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchmove", false)
        .expect("touchmove should dispatch to explicit capture target");
    vm.dispatch_touch_event_at_point(10.0, 35.0, "touchend", false)
        .expect("touchend should release explicit capture");

    let result = vm
        .eval("window.__touchExplicitCaptureLog.join('|')")
        .expect("touch explicit capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button:touch|has:true|gotpointercapture@target0:touch|pointermove@target0:touch|pointerup@target0:touch|lostpointercapture@target0:touch"
    );
}

#[test]
fn pointer_raw_update_dispatches_after_pointer_boundary_before_mouse_boundary() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-raw-update-order.test/",
        r#"<html><body><div id="init">init</div><div id="target">target</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target = document.getElementById('target');
  window.__rawUpdateOrder = [`exposed:${'onpointerrawupdate' in target}`];
  function log(event) {
    window.__rawUpdateOrder.push(event.type);
  }
  for (const type of ['pointerover', 'pointerenter', 'pointerrawupdate', 'pointermove', 'mouseover', 'mouseenter']) {
    target.addEventListener(type, log);
  }
  target.addEventListener('pointerrawupdate', () => {
    target.removeEventListener('mouseover', log);
    target.removeEventListener('mouseenter', log);
  }, { once: true });
})()
"#,
    )
    .expect("pointer raw update listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("initial mousemove should establish previous hover target");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("second mousemove should dispatch raw update before mouse boundary");

    let result = vm
        .eval("window.__rawUpdateOrder.join('|')")
        .expect("raw update order log should evaluate");
    assert_eq!(
        result,
        "exposed:true|pointerover|pointerenter|pointerrawupdate|pointermove"
    );
}

#[test]
fn pointer_raw_update_flushes_capture_before_pointermove() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-raw-update-capture.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__rawUpdateCaptureLog = [];
  target0.addEventListener('pointerdown', event => {
    window.__rawUpdateCaptureLog.push('pointerdown@target0');
    target0.setPointerCapture(event.pointerId);
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__rawUpdateCaptureLog.push('gotpointercapture@target0');
  });
  target0.addEventListener('pointerrawupdate', event => {
    window.__rawUpdateCaptureLog.push('pointerrawupdate@target0');
    target0.releasePointerCapture(event.pointerId);
  });
  target0.addEventListener('lostpointercapture', () => {
    window.__rawUpdateCaptureLog.push('lostpointercapture@target0');
  });
  target0.addEventListener('pointermove', () => {
    window.__rawUpdateCaptureLog.push('pointermove@target0');
  });
  target1.addEventListener('pointermove', () => {
    window.__rawUpdateCaptureLog.push('pointermove@target1');
  });
})()
"#,
    )
    .expect("pointer raw update capture listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set pending capture");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("mousemove should dispatch raw update before pointermove");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should complete pointer stream");

    let result = vm
        .eval("window.__rawUpdateCaptureLog.join('|')")
        .expect("raw update capture log should evaluate");
    assert_eq!(
        result,
        "pointerdown@target0|gotpointercapture@target0|pointerrawupdate@target0|lostpointercapture@target0|pointermove@target1"
    );
}

#[test]
fn release_pointer_capture_clears_pending_capture_before_got_event() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-release-pending.test/",
        r#"<html><body><div id="target0">first</div><div id="target1">second</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__releaseCaptureStarted = false;
  window.__releaseCaptureLog = [];
  try {
    target0.setPointerCapture(1);
  } catch (error) {
    window.__releaseCaptureLog.push(`inactive:${error.name}`);
  }
  for (const target of [target0, target1]) {
    for (const type of ['pointerdown', 'gotpointercapture', 'pointermove', 'pointerup', 'lostpointercapture']) {
      target.addEventListener(type, event => {
        if (event.type === 'pointermove' && !window.__releaseCaptureStarted) {
          return;
        }
        window.__releaseCaptureLog.push(`${event.type}@${target.id}`);
        if (event.type === 'pointerdown' && target === target0) {
          window.__releaseCaptureStarted = true;
          target0.setPointerCapture(event.pointerId);
          target0.releasePointerCapture(event.pointerId);
          window.__releaseCaptureLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
        }
      });
    }
  }
})()
"#,
    )
    .expect("release capture listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch pointerdown");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("uncaptured pointermove should dispatch to hit target");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("uncaptured pointerup should dispatch to hit target");

    let result = vm
        .eval("window.__releaseCaptureLog.join('|')")
        .expect("release capture log should evaluate");
    assert_eq!(
        result,
        "inactive:NotFoundError|pointerdown@target0|has:false|pointermove@target1|pointerup@target1"
    );
}

#[test]
fn removing_got_pointer_capture_target_dispatches_lost_on_document() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-got-removal.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__captureRemovalLog = [];
  button.addEventListener('pointerdown', event => {
    window.__captureRemovalLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
  });
  button.addEventListener('pointerup', () => {
    window.__captureRemovalLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__captureRemovalLog.push('gotpointercapture@target0');
    target0.remove();
  });
  target0.addEventListener('lostpointercapture', () => {
    window.__captureRemovalLog.push('lostpointercapture@target0');
  });
  target0.addEventListener('pointerup', () => {
    window.__captureRemovalLog.push('pointerup@target0');
  });
  document.addEventListener('lostpointercapture', event => {
    if (event.target === document) {
      window.__captureRemovalLog.push('lostpointercapture@document');
    }
  });
})()
"#,
    )
    .expect("capture removal listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set pending pointer capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should process removed capture target");

    let result = vm
        .eval("window.__captureRemovalLog.join('|')")
        .expect("capture removal log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button|gotpointercapture@target0|lostpointercapture@document|pointerup@button"
    );
}

#[test]
fn lost_pointer_capture_can_remove_pending_target_before_got() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-lost-removes-pending.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture0</div><div id="target1">capture1</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  const target1 = document.getElementById('target1');
  window.__pendingRemovalLog = [];
  button.addEventListener('pointerdown', event => {
    window.__pendingRemovalLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
  });
  button.addEventListener('pointerup', () => {
    window.__pendingRemovalLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__pendingRemovalLog.push('gotpointercapture@target0');
  });
  target0.addEventListener('pointermove', event => {
    window.__pendingRemovalLog.push('pointermove@target0');
    target1.setPointerCapture(event.pointerId);
  });
  target0.addEventListener('lostpointercapture', () => {
    window.__pendingRemovalLog.push('lostpointercapture@target0');
    target1.remove();
  });
  target1.addEventListener('gotpointercapture', () => {
    window.__pendingRemovalLog.push('gotpointercapture@target1');
  });
})()
"#,
    )
    .expect("pending capture removal listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set first pending capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("mousemove should dispatch to first capture target");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should skip removed second capture target");

    let result = vm
        .eval("window.__pendingRemovalLog.join('|')")
        .expect("pending capture removal log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button|gotpointercapture@target0|pointermove@target0|lostpointercapture@target0|pointerup@button"
    );
}

#[test]
fn removed_pending_pointer_capture_target_is_cleared_immediately() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-pending-removal-hook.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__pendingHookLog = [];
  button.addEventListener('pointerdown', event => {
    window.__pendingHookLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
    target0.remove();
    window.__pendingHookLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
  });
  button.addEventListener('pointerup', () => {
    window.__pendingHookLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__pendingHookLog.push('gotpointercapture@target0');
  });
  document.addEventListener('lostpointercapture', event => {
    if (event.target === document) {
      window.__pendingHookLog.push('lostpointercapture@document');
    }
  });
})()
"#,
    )
    .expect("pending capture hook listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set then clear pending pointer capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should not capture removed pending target");

    let result = vm
        .eval("window.__pendingHookLog.join('|')")
        .expect("pending hook log should evaluate");
    assert_eq!(result, "pointerdown@button|has:false|pointerup@button");
}

#[test]
fn removed_active_pointer_capture_target_loses_capture_on_next_event() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-capture-active-removal-hook.test/",
        r#"<html><body><div id="button">button</div><div id="target0">capture</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  const button = document.getElementById('button');
  const target0 = document.getElementById('target0');
  window.__activeHookLog = [];
  button.addEventListener('pointerdown', event => {
    window.__activeHookLog.push('pointerdown@button');
    target0.setPointerCapture(event.pointerId);
  });
  button.addEventListener('pointerup', () => {
    window.__activeHookLog.push('pointerup@button');
  });
  target0.addEventListener('gotpointercapture', () => {
    window.__activeHookLog.push('gotpointercapture@target0');
  });
  target0.addEventListener('pointermove', event => {
    window.__activeHookLog.push('pointermove@target0');
    target0.remove();
    window.__activeHookLog.push(`has:${target0.hasPointerCapture(event.pointerId)}`);
  });
  target0.addEventListener('pointerup', () => {
    window.__activeHookLog.push('pointerup@target0');
  });
  document.addEventListener('lostpointercapture', event => {
    if (event.target === document) {
      window.__activeHookLog.push('lostpointercapture@document');
    }
  });
})()
"#,
    )
    .expect("active capture hook listener setup should evaluate");

    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should set pending pointer capture");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mousemove", -1, None, 0.0, 0.0)
        .expect("mousemove should dispatch to active capture target");
    vm.dispatch_mouse_event_at_point(10.0, 11.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("mouseup should release disconnected capture target on document");

    let result = vm
        .eval("window.__activeHookLog.join('|')")
        .expect("active hook log should evaluate");
    assert_eq!(
        result,
        "pointerdown@button|gotpointercapture@target0|pointermove@target0|has:false|lostpointercapture@document|pointerup@button"
    );
}

#[test]
fn mouse_hover_dispatches_pointer_boundary_before_mouse_boundary() {
    let mut vm = new_parsed_test_vm(
        "https://pointer-boundary-order.test/",
        r#"<html><body><div id="a">a</div><div id="b">b</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__boundaryLog = [];
  for (const id of ['a', 'b']) {
    const target = document.getElementById(id);
    for (const type of [
      'pointerover', 'pointerenter', 'pointerout', 'pointerleave', 'pointermove',
      'mouseover', 'mouseenter', 'mouseout', 'mouseleave', 'mousemove'
    ]) {
      target.addEventListener(type, event => {
        window.__boundaryLog.push([
          event.type,
          id,
          event.pointerType || '',
          event.relatedTarget ? event.relatedTarget.id : '',
          event.bubbles
        ].join(':'));
      });
    }
  }
})()
"#,
    )
    .expect("boundary listener setup should evaluate");

    let pointer = crate::runtime::RendererPointerEventProperties {
        pointer_id: 1,
        pointer_type: "pen".to_owned(),
        pressure: 0.0,
        tangential_pressure: 0.0,
        tilt_x: 0.0,
        tilt_y: 0.0,
        twist: 0.0,
    };
    vm.dispatch_mouse_event_at_point_with_pointer(
        10.0,
        10.0,
        "mousemove",
        0,
        Some(0),
        0,
        0.0,
        0.0,
        pointer.clone(),
    )
    .expect("first mousemove should dispatch boundary events");
    vm.dispatch_mouse_event_at_point_with_pointer(
        10.0,
        34.0,
        "mousemove",
        0,
        Some(0),
        0,
        0.0,
        0.0,
        pointer,
    )
    .expect("second mousemove should dispatch boundary events");

    let result = vm
        .eval("window.__boundaryLog.join('|')")
        .expect("boundary log should evaluate");
    assert_eq!(
        result,
        "pointerover:a:pen::true|pointerenter:a:pen::false|mouseover:a:::true|mouseenter:a:::true|pointermove:a:pen::true|mousemove:a:::true|pointerout:a:pen:b:true|pointerleave:a:pen:b:false|pointerover:b:pen:a:true|pointerenter:b:pen:a:false|mouseout:a::b:true|mouseleave:a::b:true|mouseover:b::a:true|mouseenter:b::a:true|pointermove:b:pen::true|mousemove:b:::true"
    );
}

#[test]
fn mouse_hover_persists_stylo_state_and_reflows_dropdown_before_next_hit_test() {
    let mut vm = new_parsed_test_vm(
        "https://hover-dropdown.test/",
        r#"
<!doctype html>
<style>
  html, body { margin: 0; padding: 0; }
  #menu, #trigger, #submenu, #outside { width: 120px; }
  #trigger, #submenu, #outside { display: block; box-sizing: border-box; height: 24px; }
  #submenu { display: none; }
  #menu:hover #submenu { display: block; }
</style>
<nav id="menu">
  <button id="trigger">menu</button>
  <button id="submenu">child</button>
</nav>
<button id="outside">outside</button>
"#,
    );
    vm.eval(
        r#"
(() => {
  const menu = document.getElementById('menu');
  const trigger = document.getElementById('trigger');
  const submenu = document.getElementById('submenu');
  const outside = document.getElementById('outside');
  window.__submenuClicks = 0;
  window.__duringHover = '';
  window.__afterLeave = '';
  trigger.addEventListener('mousemove', () => {
    window.__duringHover = [
      trigger.matches(':hover'),
      menu.matches(':hover'),
      document.body.matches(':hover'),
      document.documentElement.matches(':hover'),
      getComputedStyle(submenu).display
    ].join('|');
  });
  submenu.addEventListener('click', () => window.__submenuClicks++);
  outside.addEventListener('mousemove', () => {
    window.__afterLeave = [
      trigger.matches(':hover'),
      menu.matches(':hover'),
      submenu.matches(':hover'),
      outside.matches(':hover'),
      getComputedStyle(submenu).display
    ].join('|');
  });
})()
"#,
    )
    .expect("hover dropdown listeners should install");

    assert_eq!(
        vm.eval(
            "[document.getElementById('menu').matches(':hover'), getComputedStyle(document.getElementById('submenu')).display].join('|')"
        )
        .expect("initial hover state should evaluate"),
        "false|none"
    );

    vm.dispatch_mouse_event_at_point(10.0, 10.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("mousemove should establish hover state");
    assert_eq!(
        vm.eval("window.__duringHover")
            .expect("in-event hover state should evaluate"),
        "true|true|true|true|block"
    );

    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mousedown", 0, Some(1), 0.0, 0.0)
        .expect("mousedown should hit the newly displayed submenu");
    vm.dispatch_mouse_event_at_point(10.0, 35.0, "mouseup", 0, Some(0), 0.0, 0.0)
        .expect("mouseup should activate the newly displayed submenu");
    assert_eq!(
        vm.eval("String(window.__submenuClicks)")
            .expect("submenu click count should evaluate"),
        "1"
    );

    vm.dispatch_mouse_event_at_point(10.0, 60.0, "mousemove", -1, Some(0), 0.0, 0.0)
        .expect("mousemove outside the menu should clear its hover chain");
    assert_eq!(
        vm.eval("window.__afterLeave")
            .expect("post-hover state should evaluate"),
        "false|false|false|true|none"
    );
}

#[test]
fn detached_document_tag_collection_exposes_named_item() {
    let mut vm = new_storage_test_vm("https://detached-document-named-item.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><head><meta name="greyEnv" content="prod"><meta id="by-id" content="id"></head></html>',
    'text/html'
  );
  const metas = doc.getElementsByTagName('meta');
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return 'throw:' + error.name;
    }
  };
  return [
    Object.prototype.toString.call(metas),
    typeof metas.namedItem,
    Object.hasOwn(metas, 'length'),
    Object.hasOwn(metas, 'item'),
    Object.hasOwn(metas, 'namedItem'),
    Object.hasOwn(metas, Symbol.iterator),
    Object.getOwnPropertyDescriptor(HTMLCollection.prototype, 'length').get.call(metas),
    metas.namedItem({ toString() { return 'greyEnv'; } }) && metas.namedItem('greyEnv').getAttribute('content'),
    metas.namedItem('by-id') && metas.namedItem('by-id').getAttribute('content'),
    metas.namedItem('missing') === null,
    probe(() => metas.namedItem(undefined)),
    probe(() => metas.namedItem()),
    probe(() => metas.namedItem(Symbol('name')))
  ].join('|');
})()
"#,
        )
        .expect("detached document tag collection namedItem should be available");

    assert_eq!(
        result,
        "[object HTMLCollection]|function|false|false|false|false|2|prod|id|true|null|throw:TypeError|throw:TypeError"
    );
}
#[test]
fn detached_document_insert_updates_collection_and_sibling_surface() {
    let mut vm = new_storage_test_vm("https://detached-dom-surface-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><p id="a" name="alpha"></p><span id="b"></span></body></html>',
    'text/html'
  );
  const inserted = doc.createElement('section');
  inserted.id = 'mid';
  inserted.setAttribute('name', 'middle');
  doc.body.insertBefore(inserted, doc.getElementById('b'));
  const children = doc.body.children;
  return [
    Object.prototype.toString.call(children),
    children.length,
    children.item(0) === doc.getElementById('a'),
    children.item(1) === inserted,
    children.item(99) === null,
    children.namedItem('middle') === inserted,
    children.namedItem('mid') === inserted,
    inserted.previousSibling.id,
    inserted.nextSibling.id,
    doc.body.firstChild.id,
    doc.body.lastChild.id,
    doc.body.childNodes.length
  ].join('|');
})()
"#,
        )
        .expect("detached DOM insertion should refresh collection and sibling surface");

    assert_eq!(
        result,
        "[object HTMLCollection]|3|true|true|true|true|true|a|b|a|b|3"
    );
}
#[test]
fn detached_document_node_mutation_methods_refresh_document_surface() {
    let mut vm = new_storage_test_vm("https://detached-document-node-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<!doctype html><html><head><title>x</title></head><body><p id="a"></p></body></html>',
    'text/html'
  );
  const before = [
    typeof doc.removeChild,
    typeof doc.appendChild,
    typeof doc.insertBefore,
    typeof doc.replaceChild,
    doc.doctype && doc.doctype.nodeType,
    doc.documentElement && doc.documentElement.nodeName,
    doc.body && doc.body.nodeName
  ].join(',');

  const originalRoot = doc.documentElement;
  const doctype = doc.doctype;
  const removed = doc.removeChild(originalRoot);
  const afterRemove = [
    removed === originalRoot,
    doc.documentElement === null,
    doc.body === null,
    doc.firstChild === doctype,
    doc.lastChild === doctype,
    originalRoot.parentNode === null,
    originalRoot.isConnected === false
  ].join(',');

  const replacement = originalRoot.cloneNode(true);
  doc.appendChild(replacement);
  const afterAppend = [
    doc.documentElement === replacement,
    doc.body === replacement.querySelector('body'),
    doc.head === replacement.querySelector('head'),
    doc.firstChild === doctype,
    doc.lastChild === replacement,
    replacement.parentNode === doc,
    replacement.isConnected === true,
    doc.childNodes.length
  ].join(',');

  const html2 = doc.createElement('html');
  doc.replaceChild(html2, replacement);
  const afterReplace = [
    doc.documentElement === html2,
    doc.lastChild === html2,
    replacement.parentNode === null,
    html2.parentNode === doc
  ].join(',');

  return `${before}|${afterRemove}|${afterAppend}|${afterReplace}`;
})()
"#,
        )
        .expect("detached document node mutation methods should refresh document surface");

    assert_eq!(
        result,
        "function,function,function,function,10,HTML,BODY|true,true,true,true,true,true,true|true,true,true,true,true,true,true,2|true,true,true,true"
    );
}
#[test]
fn detached_document_can_append_dom_parser_snapshot_clone() {
    let mut vm = new_storage_test_vm("https://detached-document-dom-parser-clone.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const referenceDoc = document.implementation.createHTMLDocument("");
  referenceDoc.removeChild(referenceDoc.documentElement);
  const snapshot = new DOMParser().parseFromString(
    '<!doctype html><html><head><title>x</title></head><body><p id="a"></p></body></html>',
    'text/html'
  );
  const snapshotClone = snapshot.documentElement.cloneNode(true);
  referenceDoc.appendChild(snapshotClone);
  const childDoc = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body></body></html>',
    'text/html'
  );
  childDoc.removeChild(childDoc.documentElement);
  const referenceClone = referenceDoc.documentElement.cloneNode(true);
  const appendedToChild = childDoc.appendChild(referenceClone);
  return [
    referenceDoc.documentElement && referenceDoc.documentElement.nodeName,
    referenceDoc.body && referenceDoc.body.nodeName,
    referenceDoc.getElementById('a') && referenceDoc.getElementById('a').nodeName,
    referenceDoc.documentElement === snapshotClone,
    snapshotClone.parentNode === null,
    snapshot.documentElement.parentNode === snapshot,
    referenceClone.nodeType,
    referenceClone.nodeName,
    referenceClone.localName,
    referenceClone.namespaceURI,
    referenceClone.childNodes && referenceClone.childNodes.length,
    typeof referenceClone.getAttributeNames,
    appendedToChild && appendedToChild.nodeType,
    appendedToChild && appendedToChild.__lmDomParserId === undefined,
    childDoc.documentElement && childDoc.documentElement.nodeName,
    childDoc.body && childDoc.body.nodeName,
    childDoc.getElementById('a') && childDoc.getElementById('a').nodeName,
    childDoc.documentElement === appendedToChild,
    referenceClone.parentNode === childDoc,
    referenceDoc.documentElement.parentNode === referenceDoc
  ].join('|');
})()
"#,
        )
        .expect("detached document should import DOMParser snapshot clones");

    assert_eq!(
        result,
        "HTML|BODY|P|true|false|true|1|HTML|html|http://www.w3.org/1999/xhtml|2|function|1|true|HTML|BODY|P|true|true|true"
    );
}
#[test]
fn dom_parser_import_uses_native_children_after_child_nodes_tamper() {
    let mut vm = new_storage_test_vm("https://dom-parser-import-native-children.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = new DOMParser().parseFromString(
    '<!doctype html><html><body><section id="source"><span id="real">ok</span></section></body></html>',
    'text/html'
  );
  const section = source.getElementById('source');
  const real = source.getElementById('real');
  const fake = source.createElement('fake-node');
  fake.id = 'fake';
  const projected = section.childNodes;
  projected[0] = fake;
  projected.length = 1;
  source.__lmDomParserId = 999999;
  source.__lmDomParserNode = 999999;
  section.__lmDomParserId = 999999;
  section.__lmDomParserNode = 999999;
  real.__lmDomParserId = 999999;
  real.__lmDomParserNode = 999999;
  const realAfterTamper = source.getElementById('real');

  const target = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body></body></html>',
    'text/html'
  );
  const imported = target.body.appendChild(section);
  return [
    imported.nodeName,
    imported.ownerDocument === target,
    imported.childNodes.length,
    imported.firstChild && imported.firstChild.nodeName,
    imported.firstChild && imported.firstChild.id,
    imported.textContent,
    target.getElementById('real') === imported.firstChild,
    realAfterTamper === real,
    realAfterTamper.parentNode === section,
    target.getElementById('fake') === null,
    fake.parentNode === null
  ].join('|');
})()
"#,
        )
        .expect("DOMParser import should use native source children after childNodes tamper");

    assert_eq!(
        result,
        "SECTION|true|1|SPAN|real|ok|true|true|true|true|true"
    );
}

#[test]
fn dom_parser_native_query_selector_all_declares_node_list_shell() {
    let mut vm = new_storage_test_vm("https://dom-parser-native-node-list-shell.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = new DOMParser().parseFromString(
    '<!doctype html><html><body><section id="root"><span id="a"></span><span id="b"></span></section></body></html>',
    'text/html'
  );
  const root = source.getElementById('root');
  const list = root.querySelectorAll('span');
  const methodShape = (prototype, key) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
    return [
      !!descriptor,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.configurable,
      descriptor && descriptor.writable,
      descriptor && typeof descriptor.value,
      descriptor && descriptor.value.name,
      descriptor && descriptor.value.length
    ].join(':');
  };
  const beforeDataName = Object.getOwnPropertyNames(list).includes('data');
  list.data = { items: [] };
  return [
    Object.prototype.toString.call(list),
    list.constructor && list.constructor.name,
    Object.getPrototypeOf(list) === NodeList.prototype,
    list.length,
    list[0].id,
    list[1].id,
    list[2] === undefined,
    list.item(0).id,
    list.item(1).id,
    list.item(2) === null,
    Array.from(list).map(node => node.id).join(','),
    Object.hasOwn(list, 'length'),
    Object.hasOwn(list, 'item'),
    Object.hasOwn(list, Symbol.iterator),
    methodShape(NodeList.prototype, 'item'),
    methodShape(NodeList.prototype, Symbol.iterator).split(':').slice(0, 5).join(':'),
    NodeList.prototype[Symbol.iterator] === Array.prototype.values,
    beforeDataName,
    Object.prototype.hasOwnProperty.call(list, 'data'),
    list.item(0).id,
    Array.from(list).map(node => node.id).join(',')
  ].join('|');
})()
"#,
        )
        .expect("DOMParser native NodeList shell should be declared");

    assert_eq!(
        result,
        "[object NodeList]|NodeList|true|2|a|b|true|a|b|true|a,b|false|false|false|true:false:true:true:function:item:1|true:false:true:true:function|true|false|true|a|a,b"
    );
}

#[test]
fn dom_parser_import_uses_native_attributes_after_attribute_method_tamper() {
    let mut vm = new_storage_test_vm("https://dom-parser-import-native-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = new DOMParser().parseFromString(
    '<!doctype html><html><body><section id="source" data-real="native"></section></body></html>',
    'text/html'
  );
  const section = source.getElementById('source');
  section.setAttributeNS('urn:attr', 'a:flag', 'value');
  section.getAttributeNames = () => ['data-fake'];
  section.getAttribute = () => 'tampered';

  const target = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body></body></html>',
    'text/html'
  );
  const imported = target.body.appendChild(section);
  return [
    imported.getAttribute('id'),
    imported.getAttribute('data-real'),
    imported.getAttribute('data-fake'),
    imported.getAttributeNS('urn:attr', 'flag'),
    imported.hasAttributeNS('urn:attr', 'flag'),
    imported.getAttributeNames().join(',')
  ].join('|');
})()
"#,
        )
        .expect("DOMParser import should use native source attributes after method tamper");

    assert_eq!(result, "source|native||value|true|id,data-real,a:flag");
}

#[test]
fn parent_node_prepend_can_reuse_the_existing_first_child() {
    let mut vm = new_storage_test_vm("https://parent-node-prepend-existing-first.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  const first = document.createElement('span');
  const second = document.createElement('em');
  first.id = 'first';
  second.id = 'second';
  host.append(first, second);
  (document.body || document.documentElement || document).appendChild(host);

  const firstResult = host.prepend(first);
  const afterFirst = Array.from(host.children, child => child.id).join(',');
  const secondResult = host.prepend(second);
  const afterSecond = Array.from(host.children, child => child.id).join(',');
  host.prepend(second);

  return [
    firstResult === undefined,
    afterFirst,
    secondResult === undefined,
    afterSecond,
    Array.from(host.children, child => child.id).join(',')
  ].join('|');
})()
"#,
        )
        .expect("ParentNode.prepend should move an existing reference child");

    assert_eq!(result, "true|first,second|true|second,first|second,first");
}

#[test]
fn detached_document_parent_node_append_and_prepend_match_child_fixture_needs() {
    let mut vm = new_storage_test_vm("https://detached-document-parent-node-append.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<!doctype html><html><head></head><body><p id="host"></p></body></html>',
    'text/html'
  );
  const host = doc.getElementById('host');
  const child = doc.createElement('span');
  child.id = 'local';
  child.setAttribute('data-kind', 'fixture');
  const textHost = doc.createElement('p');
  textHost.textContent = 'seed';
  host.append(child, 'tail');
  host.prepend('head');

  const foreignDoc = document.implementation.createHTMLDocument('');
  const foreign = foreignDoc.createElement('em');
  foreign.id = 'foreign';
  foreign.append('copy');
  host.append(foreign);

  return [
    typeof host.append,
    typeof host.prepend,
    host.childNodes.length,
    host.firstChild.nodeValue,
    host.childNodes[1] === child,
    host.childNodes[2].nodeValue,
    host.lastChild.nodeName,
    host.lastChild.id,
    host.lastChild.textContent,
    foreign.parentNode === null,
    doc.getElementById('foreign') === host.lastChild,
    textHost.firstChild && textHost.firstChild.nodeValue,
    textHost.childNodes.length,
    child.attributes.length,
    child.attributes[0] && child.attributes[0].name,
    child.attributes[0] && child.attributes[0].value,
    child.attributes['data-kind'] && child.attributes['data-kind'].value,
    Object.prototype.toString.call(child.attributes)
  ].join('|');
})()
"#,
        )
        .expect("detached document append/prepend should support strings and foreign clones");

    assert_eq!(
        result,
        "function|function|4|head|true|tail|EM|foreign|copy|false|true|seed|1|2|id|local|fixture|[object NamedNodeMap]"
    );
}
#[test]
fn child_content_document_element_attributes_are_indexed() {
    let mut vm = new_storage_test_vm("https://child-content-document-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(iframe);
  const doc = iframe.contentDocument;
  const node = doc.createElement('p');
  node.id = 'child';
  node.setAttribute('data-kind', 'fixture');
  const referenceDoc = document.implementation.createHTMLDocument('');
  referenceDoc.removeChild(referenceDoc.documentElement);
  doc.body.setAttribute('onload', 'run()');
  const sourceClone = doc.documentElement.cloneNode(true);
  const sourceCloneBody = sourceClone.querySelector('body');
  const sourceCloneBodyAttributes = [
    sourceCloneBody.attributes.length,
    sourceCloneBody.attributes[0] && sourceCloneBody.attributes[0].name,
    sourceCloneBody.attributes[0] && sourceCloneBody.attributes[0].value,
    sourceCloneBody.attributes.item(0) && sourceCloneBody.attributes.item(0).name,
    sourceCloneBody.attributes.getNamedItem('onload') && sourceCloneBody.attributes.getNamedItem('onload').value
  ].join(',');
  referenceDoc.appendChild(sourceClone);
  doc.removeChild(doc.documentElement);
  doc.appendChild(referenceDoc.documentElement.cloneNode(true));
  const restored = doc.createElement('p');
  restored.id = 'restored';
  restored.setAttribute('data-kind', 'clone-path');
  return [
    node.attributes.length,
    node.attributes[0] && node.attributes[0].name,
    node.attributes[0] && node.attributes[0].value,
    node.attributes['data-kind'] && node.attributes['data-kind'].value,
    Object.prototype.toString.call(node.attributes),
    restored.attributes.length,
    restored.attributes[0] && restored.attributes[0].name,
    restored.attributes[0] && restored.attributes[0].value,
    restored.attributes['data-kind'] && restored.attributes['data-kind'].value,
    doc.body.attributes.length,
    doc.body.attributes[0] && doc.body.attributes[0].name,
    doc.body.attributes[0] && doc.body.attributes[0].value,
    sourceCloneBodyAttributes
  ].join('|');
})()
"#,
        )
        .expect("child contentDocument element attributes should be indexed");

    assert_eq!(
        result,
        "2|id|child|fixture|[object NamedNodeMap]|2|id|restored|clone-path|1|onload|run()|1,onload,run(),onload,run()"
    );
}
#[test]
fn detached_html_document_created_elements_persist_direct_attributes() {
    let mut vm = new_storage_test_vm("https://detached-create-html-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('');
  const p = doc.createElement('p');
  p.setAttribute('data-kind', 'local');
  p.id = 'host';
  doc.body.setAttribute('onload', 'run()');
  const a = doc.createElement('a');
  a.href = 'http://example.org/?ä';
  const svg = doc.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 10 10');
  return [
    p.getAttribute('data-kind'),
    p.hasAttribute('data-kind'),
    p.attributes.length,
    p.attributes[0] && p.attributes[0].name,
    p.attributes[0] && p.attributes[0].value,
    p.attributes.getNamedItem('data-kind') && p.attributes.getNamedItem('data-kind').value,
    p.id,
    p.getAttribute('id'),
    doc.body.getAttribute('onload'),
    doc.body.attributes[0] && doc.body.attributes[0].name,
    a.getAttribute('href'),
    a.href,
    svg.getAttribute('viewBox'),
    svg.getAttribute('viewbox')
  ].join('|');
})()
"#,
        )
        .expect("detached createHTMLDocument elements should persist direct attributes");

    assert_eq!(
        result,
        "local|true|2|data-kind|local|local|host|host|run()|onload|http://example.org/?ä|http://example.org/?%C3%A4|0 0 10 10|"
    );
}
#[test]
fn constructed_document_xhtml_anchor_reflects_url_href() {
    let mut vm = new_storage_test_vm("https://constructed-document-anchor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new Document();
  const a = doc.createElementNS('http://www.w3.org/1999/xhtml', 'a');
  a.href = 'http://example.org/?ä';
  return [
    a.constructor === HTMLAnchorElement,
    a instanceof HTMLAnchorElement,
    a.getAttribute('href'),
    a.href
  ].join('|');
})()
"#,
        )
        .expect("constructed Document XHTML anchors should reflect URL href");

    assert_eq!(
        result,
        "true|true|http://example.org/?ä|http://example.org/?%C3%A4"
    );
}
#[test]
fn detached_html_document_namespaced_attributes_import_with_metadata() {
    let mut vm = new_storage_test_vm("https://detached-create-html-ns-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('Title');
  doc.body.setAttributeNS('http://example.com/', 'p:name', 'value');
  doc.body.removeAttribute('p:name');
  const removedByQualifiedName = [
    doc.body.getAttribute('p:name') === null,
    doc.body.getAttributeNS('http://example.com/', 'name') === null,
    doc.body.getAttributeNodeNS('http://example.com/', 'name') === null
  ].join(',');
  doc.body.setAttributeNS('http://example.com/', 'p:name', 'value');
  const originalAttr = doc.body.getAttributeNodeNS('http://example.com/', 'name');
  const imported = document.importNode(originalAttr, true);
  const beforeRemove = [
    removedByQualifiedName,
    doc.body.getAttribute('p:name'),
    doc.body.getAttributeNS('http://example.com/', 'name'),
    doc.body.hasAttributeNS('http://example.com/', 'name'),
    originalAttr && originalAttr.name,
    originalAttr && originalAttr.prefix,
    originalAttr && originalAttr.namespaceURI,
    originalAttr && originalAttr.localName,
    imported && imported.prefix,
    imported && imported.namespaceURI,
    imported && imported.localName
  ].join('|');
  doc.body.removeAttributeNS('http://example.com/', 'name');
  return [
    beforeRemove,
    doc.body.getAttribute('p:name') === null,
    doc.body.getAttributeNS('http://example.com/', 'name') === null,
    doc.body.getAttributeNodeNS('http://example.com/', 'name') === null
  ].join('|');
})()
"#,
        )
        .expect("detached createHTMLDocument NS attributes should import");

    assert_eq!(
        result,
        "true,true,true|value|value|true|p:name|p|http://example.com/|name|p|http://example.com/|name|true|true|true"
    );
}

#[test]
fn live_attr_object_cache_uses_private_slot_and_ignores_public_spoofing() {
    let mut vm = new_storage_test_vm("https://attr-cache-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const element = document.createElement('div');
  body.appendChild(element);
  element.setAttribute('data-real', 'one');
  element.setAttributeNS('urn:attr-cache', 'p:flag', 'ns-one');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliAttrObjectCache'))
    .sort()
    .join(',');
  const named = element.getAttributeNode('data-real');
  const namespaced = element.getAttributeNodeNS('urn:attr-cache', 'flag');
  const afterCacheNames = internalNames(element);
  const fakeCache = Object.create(null);
  fakeCache['data-real'] = {
    name: 'data-real',
    value: 'fake',
    ownerElement: null,
    namespaceURI: null,
    localName: 'data-real'
  };
  Element.prototype.__moliAttrObjectCache = fakeCache;
  element.__moliAttrObjectCache = fakeCache;
  const spoofedOwnNames = internalNames(element);
  const namedAfterSpoof = element.getAttributeNode('data-real');
  const namespacedAfterSpoof = element.getAttributeNodeNS('urn:attr-cache', 'flag');
  element.removeAttribute('data-real');
  element.removeAttributeNS('urn:attr-cache', 'flag');
  return JSON.stringify({
    afterCacheNames,
    spoofedOwnNames,
    sameNamed: namedAfterSpoof === named,
    sameNamespaced: namespacedAfterSpoof === namespaced,
    namedValue: namedAfterSpoof && namedAfterSpoof.value,
    namespacedValue: namespacedAfterSpoof && namespacedAfterSpoof.value,
    namedDetached: named.ownerElement === null && named.value === 'one',
    namespacedDetached: namespaced.ownerElement === null && namespaced.value === 'ns-one',
    namedRemoved: element.getAttributeNode('data-real') === null,
    namespacedRemoved: element.getAttributeNodeNS('urn:attr-cache', 'flag') === null
  });
})()
"#,
        )
        .expect("live Attr cache should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"afterCacheNames":"","spoofedOwnNames":"__moliAttrObjectCache","sameNamed":true,"sameNamespaced":true,"namedValue":"one","namespacedValue":"ns-one","namedDetached":true,"namespacedDetached":true,"namedRemoved":true,"namespacedRemoved":true}"#
    );
}

#[test]
fn detached_document_adopts_live_namespaced_attributes() {
    let mut vm = new_storage_test_vm("https://detached-adopt-live-ns-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createDocument('urn:doc', 'root', null);
  const source = document.createElementNS('urn:source', 's:item');
  source.setAttribute('data-real', 'native');
  source.setAttributeNS('urn:attr', 'a:flag', 'value');
  const realGetAttribute = source.getAttribute;
  const realGetAttributeNames = source.getAttributeNames;
  source.getAttributeNames = () => ['data-fake'];
  source.getAttribute = () => 'tampered';
  const adopted = doc.documentElement.appendChild(source);
  const attr = adopted.getAttributeNodeNS('urn:attr', 'flag');
  return [
    realGetAttribute.call(adopted, 'data-real'),
    realGetAttribute.call(adopted, 'data-fake'),
    realGetAttribute.call(adopted, 'a:flag'),
    adopted.getAttributeNS('urn:attr', 'flag'),
    adopted.hasAttributeNS('urn:attr', 'flag'),
    realGetAttributeNames.call(adopted).join(','),
    attr && attr.name,
    attr && attr.prefix,
    attr && attr.namespaceURI,
    attr && attr.localName,
    attr && attr.value
  ].join('|');
})()
"#,
        )
        .expect("detached documents should preserve adopted live namespaced attributes");

    assert_eq!(
        result,
        "native||value|value|true|data-real,a:flag|flag||urn:attr|flag|value"
    );
}
#[test]
fn detached_element_clone_preserves_namespaced_attributes() {
    let mut vm = new_storage_test_vm("https://detached-clone-ns-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createDocument('urn:doc', 'root', null);
  const source = doc.createElementNS('urn:source', 's:item');
  source.setAttributeNS('urn:attr', 'a:flag', 'value');
  const clone = source.cloneNode(false);
  const attr = clone.getAttributeNodeNS('urn:attr', 'flag');
  return [
    clone.getAttributeNames().join(','),
    clone.getAttribute('a:flag'),
    clone.getAttributeNS('urn:attr', 'flag'),
    clone.hasAttributeNS('urn:attr', 'flag'),
    attr && attr.name,
    attr && attr.prefix,
    attr && attr.namespaceURI,
    attr && attr.localName,
    attr && attr.value
  ].join('|');
})()
"#,
        )
        .expect("detached cloneNode should preserve namespaced attributes");

    assert_eq!(
        result,
        "a:flag|value|value|true|a:flag|a|urn:attr|flag|value"
    );
}
#[test]
fn detached_html_template_content_uses_separate_owner_document() {
    let mut vm = new_storage_test_vm("https://detached-template-content-owner.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptorShape = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(",");
  };
  const doc = document.implementation.createHTMLDocument('');
  const template = doc.createElement('template');
  doc.body.appendChild(template);
  template.content.appendChild(doc.createElement('span'));
  const templateContent = template.content;
  const templateContentDeleteResult = delete template.content;
  template.content = doc.createElement('div');
  DOMParser = function() {
    throw new Error('page-tampered DOMParser should not run');
  };
  doc.body.innerHTML = '<template><div>some text</div></template>';
  const parsedTemplate = doc.querySelector('template');
  const parsedContent = parsedTemplate && parsedTemplate.content;
  const parsedContentDeleteResult = parsedTemplate && delete parsedTemplate.content;
  if (parsedTemplate) {
    parsedTemplate.content = doc.createElement('span');
  }
  return [
    template.content.ownerDocument !== doc,
    template.content.ownerDocument.defaultView === null,
    template.content.firstChild.ownerDocument === template.content.ownerDocument,
    template.content.firstChild.localName,
    template.ownerDocument === doc,
    doc.body.childNodes.length,
    doc.body.innerHTML,
    parsedTemplate !== null,
    parsedTemplate && parsedTemplate.content.ownerDocument !== doc,
    parsedTemplate && parsedTemplate.content.ownerDocument.defaultView === null,
    descriptorShape(HTMLTemplateElement.prototype, 'content'),
    Object.prototype.hasOwnProperty.call(template, 'content'),
    template.content === templateContent,
    templateContentDeleteResult,
    Object.keys(template).includes('content'),
    parsedTemplate && descriptorShape(HTMLTemplateElement.prototype, 'content'),
    parsedTemplate && Object.prototype.hasOwnProperty.call(parsedTemplate, 'content'),
    parsedTemplate && parsedTemplate.content === parsedContent,
    parsedContentDeleteResult,
    parsedTemplate && Object.keys(parsedTemplate).includes('content')
  ].join('|');
})()
"#,
        )
        .expect("detached template content owner should evaluate");

    assert_eq!(
        result,
        "true|true|true|span|true|1|<template><div>some text</div></template>|true|true|true|function,true,true,true|false|true|true|false|function,true,true,true|false|true|true|false"
    );
}

#[test]
fn detached_html_image_decode_uses_prototype_method() {
    let mut vm = new_storage_test_vm("https://detached-image-decode-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument('');
  const image = doc.createElement('img');
  const descriptor = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, 'decode');
  return [
    typeof image.decode,
    descriptor.value === image.decode,
    descriptor.value.name,
    descriptor.value.length,
    descriptor.enumerable,
    descriptor.writable,
    descriptor.configurable,
    Object.keys(HTMLImageElement.prototype).includes('decode'),
    Object.keys(image).includes('decode'),
    Object.prototype.hasOwnProperty.call(image, 'decode'),
    typeof image.decode().then
  ].join('|');
})()
"#,
        )
        .expect("detached image decode surface should evaluate");

    assert_eq!(
        result,
        "function|true|decode|0|true|true|true|true|false|false|function"
    );
}
#[test]
fn frameset_inner_html_ignores_parser_inserted_template() {
    let mut vm = new_storage_test_vm("https://frameset-template-fragment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frameset = document.createElement('frameset');
  frameset.innerHTML = '<template>some text</template>';
  const parsedTemplate = frameset.querySelector('template');
  const template = document.createElement('template');
  frameset.appendChild(template);
  const detached = document.implementation.createHTMLDocument('');
  const detachedFrameset = detached.createElement('frameset');
  detachedFrameset.innerHTML = '<template>some text</template>';
  const detachedParsedTemplate = detachedFrameset.querySelector('template');
  detachedFrameset.appendChild(detached.createElement('template'));
  return [
    parsedTemplate === null,
    frameset.querySelectorAll('template').length,
    detachedParsedTemplate === null,
    detachedFrameset.querySelectorAll('template').length
  ].join('|');
})()
"#,
        )
        .expect("frameset innerHTML template handling should evaluate");

    assert_eq!(result, "true|1|true|1");
}
#[test]
fn html_reflection_regression_slice_matches_wpt_expectations() {
    let mut vm = new_storage_test_vm("https://reflection-regression-slice.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.createElement('form');
  form.acceptCharset = 'utf-8';
  form.setAttribute('autocomplete', 'OFF');
  const formAutocomplete = form.autocomplete;

  const hr = document.createElement('hr');
  hr.color = 'red';
  hr.noShade = true;
  hr.size = '4';

  const script = document.createElement('script');
  script.crossOrigin = undefined;
  const scriptMissing = script.getAttribute('crossorigin') === null && script.crossOrigin === null;
  script.setAttribute('crossorigin', 'invalid');
  const scriptInvalid = script.crossOrigin;
  script.setAttribute('src', '');

  const img = document.createElement('img');
  img.crossOrigin = undefined;
  const imgMissing = img.getAttribute('crossorigin') === null && img.crossOrigin === null;
  img.setAttribute('crossorigin', '');
  const imgEmpty = img.crossOrigin;
  img.setAttribute('src', '');
  img.isMap = true;
  img.width = 2147483648;

  const mod = document.createElement('ins');
  mod.setAttribute('cite', ' foo ');

  const a = document.createElement('a');
  a.href = '';

  return [
    form.getAttribute('accept-charset'),
    form.acceptCharset,
    formAutocomplete,
    hr.getAttribute('color'),
    hr.color,
    hr.noShade,
    hr.size,
    scriptMissing,
    scriptInvalid,
    script.src === location.href,
    imgMissing,
    imgEmpty,
    img.isMap,
    img.getAttribute('width'),
    mod.cite,
    a.protocol,
    a.host,
    a.pathname,
    img.src === location.href
  ].join('|');
})()
"#,
        )
        .expect("HTML reflection regression slice should evaluate");

    assert_eq!(
        result,
        "utf-8|utf-8|off|red|red|true|4|true|anonymous|true|true|anonymous|true|0|https://reflection-regression-slice.test/path/foo|https:|reflection-regression-slice.test|/path/page.html|true"
    );
}
#[test]
fn range_insert_node_rejects_ancestor_without_splitting_text() {
    let mut vm = new_storage_test_vm("https://range-insert-node-hierarchy.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const p = document.createElement('p');
  p.textContent = 'abc';
  (document.body || document.documentElement || document).appendChild(p);
  const text = p.firstChild;
  const range = document.createRange();
  range.setStart(text, 0);
  range.setEnd(text, 0);
  Node.prototype.contains = () => false;
  let thrown = null;
  try {
    range.insertNode(p);
  } catch (e) {
    thrown = e;
  }
  return [
    thrown && thrown.name,
    thrown && thrown.code,
    p.firstChild === text,
    text.data,
    p.childNodes.length,
    range.startContainer === text,
    range.startOffset
  ].join('|');
})()
"#,
        )
        .expect("Range.insertNode should reject ancestor insertion before text split");

    assert_eq!(result, "HierarchyRequestError|3|true|abc|1|true|0");
}

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
#[test]
fn detached_dom_parser_tree_root_stops_on_tampered_parent_cycle() {
    let mut vm = new_storage_test_vm("https://detached-dom-parent-cycle.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><section id="a"><span id="b"></span></section></body></html>',
    'text/html'
  );
  const a = doc.getElementById('a');
  const b = doc.getElementById('b');
  Object.defineProperty(a, 'parentNode', {
    configurable: true,
    get() {
      return a;
    }
  });
  const found = a.querySelector('#b');
  return `${found === b}:${found && found.id}`;
})()
"#,
        )
        .expect("DOMParser canonical node lookup should not spin on parent cycles");

    assert_eq!(result, "true:b");
}
#[test]
fn dom_parser_html_elements_expose_inner_html() {
    let mut vm = new_storage_test_vm("https://dom-parser-inner-html.test/");

    let result = vm
            .eval(
                r#"
(() => {
  const doc = new DOMParser().parseFromString('<h1>Title</h1><p><strong>Body</strong></p>', 'text/html');
  const h1 = doc.querySelector('h1');
  return [
    Object.prototype.toString.call(doc),
    typeof doc.body.innerHTML,
    doc.body.innerHTML,
    h1.innerHTML,
    h1.textContent,
  ].join('|');
})()
"#,
            )
            .expect("dom parser html innerHTML should be readable");

    assert_eq!(
        result,
        "[object HTMLDocument]|string|<h1>Title</h1><p><strong>Body</strong></p>|Title|Title"
    );
}
#[test]
fn dom_parser_inner_text_is_html_only() {
    let mut vm = new_storage_test_vm("https://dom-parser-inner-text-scope.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const html = parser.parseFromString('<div id="html">\n  HTML  <span>Text</span>\n</div>', 'text/html').getElementById('html');
  const xml = parser.parseFromString('<root><child>XML</child></root>', 'text/xml').documentElement;
  const svg = parser.parseFromString('<svg xmlns="http://www.w3.org/2000/svg"><text>SVG</text></svg>', 'image/svg+xml').documentElement;
  return [
    'innerText' in html,
    html.innerText,
    'innerText' in xml,
    typeof xml.innerText,
    'innerText' in svg,
    typeof svg.innerText
  ].join('|');
})()
"#,
        )
        .expect("DOMParser detached innerText scope should evaluate");

    assert_eq!(
        result,
        "true|\n  HTML  Text\n|false|undefined|false|undefined"
    );
}

#[test]
fn live_inner_text_applies_inline_text_transform() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-transform.test/",
        r#"<!doctype html><html><body>
          <a id="upper" style="text-transform: uppercase">link<br>text</a>
          <a id="lower" style="text-transform: lowercase">LINK TEXT</a>
          <a id="cap" style="text-transform: capitalize">link text</a>
          <div id="inherit" style="text-transform: uppercase">outer <span>inner</span></div>
          <div id="reset" style="text-transform: uppercase">outer <span style="text-transform: none">inner</span></div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
[
  document.getElementById('upper').innerText,
  document.getElementById('lower').innerText,
  document.getElementById('cap').innerText,
  document.getElementById('inherit').innerText,
  document.getElementById('reset').innerText
].join('|')
"#,
        )
        .expect("innerText text-transform cases should evaluate");

    assert_eq!(
        result,
        "LINK\nTEXT|link text|Link Text|OUTER INNER|OUTER inner"
    );
}

#[test]
fn live_inner_text_preserves_breaks_and_private_use_text_during_normalization() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-single-pass.test/",
        r#"<!doctype html><html><body>
          <div id="target" style="text-transform: uppercase">
            alpha <span>betaß</span><br>
            <br><span style="text-transform: none">  end </span>
          </div>
        </body></html>"#,
    );

    let result = vm
        .eval("document.getElementById('target').innerText")
        .expect("single-pass innerText should evaluate");

    assert_eq!(result, "ALPHA BETASS\n\n\u{E000} end");
}

#[test]
fn inner_text_matches_chromium_structural_and_white_space_rules() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-structure.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const targets = [];
  const read = (html) => {
    const host = document.createElement('div');
    host.innerHTML = html;
    document.body.append(host);
    targets.push(host.querySelector('#x'));
    return targets.length - 1;
  };
  const readReplacedWithChild = (tag) => {
    const element = document.createElement(tag);
    element.append('abc');
    document.body.append(element);
    targets.push(element);
    return targets.length - 1;
  };
  const cases = {
    inline: read('<div id="x">a<span>b</span>c</div>'),
    nestedBlock: read('<div id="x">a<div>b</div>c</div>'),
    cssBlock: read('<div id="x"><span style="display:block">a</span><span style="display:block">b</span></div>'),
    paragraph: read('<div id="x">a<p>b</p>c</div>'),
    adjacentParagraphs: read('<div id="x"><p>a</p><p>b</p></div>'),
    emptyBlocks: read('<div id="x">a<div></div><div>b</div><div></div>c</div>'),
    inlineBlock: read('<div id="x">a<span style="display:inline-block">b</span>c</div>'),
    emptyInlineBlock: read('<div id="x">abc <span style="display:inline-block"></span> def</div>'),
    spacedInlineBlock: read('<div id="x">abc <span style="display:inline-block"> def </span> ghi</div>'),
    tightInlineBlock: read('<div id="x">123<span style="display:inline-block"> abc </span>def</div>'),
    imageLeadingSpace: read('<div id="x"><img> abc</div>'),
    imageTrailingSpace: read('<div id="x">abc <img></div>'),
    imageChild: readReplacedWithChild('img'),
    inputChild: readReplacedWithChild('input'),
    atomicBlockPair: read('<div id="x"><span style="display:inline-block"><div>a</div><div>b<br></div></span> <span style="display:inline-block"><div> <div>c</div><div>d</div> </div></span></div>'),
    atomicBlockPairNoBr: read('<div id="x"><span style="display:inline-block"><div>a</div><div>b</div></span> <span style="display:inline-block"><div> <div>c</div><div>d</div> </div></span></div>'),
    blockOfAtomicPairs: read('<div id="x"><div><span style="display:inline-block"><div>a</div><div>b<br></div></span> <span style="display:inline-block"><div> <div>c</div><div>d</div> </div></span></div>\n<div><span style="display:inline-block"><div>e</div></span></div></div>'),
    blockWhitespace: read('<div id="x"><div>a</div> <div>b</div></div>'),
    atomicWhitespace: read('<div id="x"><span style="display:inline-block">a</span> <span style="display:inline-block">b</span></div>'),
    flex: read('<div id="x">a<span style="display:flex">b</span>c</div>'),
    flexItems: read('<div id="x" style="display:flex"><span>a</span><span>b</span></div>'),
    gridItems: read('<div id="x" style="display:grid"><span>a</span><span>b</span></div>'),
    inlineFlexItems: read('<div id="x">x<span style="display:inline-flex"><span>a</span><span>b</span></span>y</div>'),
    inlineGridItems: read('<div id="x">x<span style="display:inline-grid"><span>a</span><span>b</span></span>y</div>'),
    normal: read('<div id="x" style="white-space:normal">  a \t b\n c  </div>'),
    pre: read('<div id="x" style="white-space:pre">  a \t b\n c  </div>'),
    preWrap: read('<div id="x" style="white-space:pre-wrap">  a \t b\n c  </div>'),
    preLine: read('<div id="x" style="white-space:pre-line">  a \t b\n c  </div>'),
    breakSpaces: read('<div id="x" style="white-space:break-spaces">  a \t b\n c  </div>'),
    inheritedPre: read('<div id="x" style="white-space:pre"> a <span> b\n c </span> d </div>'),
    overriddenNormal: read('<div id="x" style="white-space:pre"> a <span style="white-space:normal">  b\n c  </span> d </div>'),
    preElement: read('<pre id="x">  a \t b\n c  </pre>'),
    hr: read('<div id="x">a<hr><hr>b</div>'),
    rootBr: read('<br id="x">'),
    rubyRp: read('<div id="x"><ruby>abc<rp>(</rp><rt>def</rt><rp>)</rp></ruby></div>'),
    loneRp: read('<div id="x"><rp>abc</rp></div>'),
    renderedRp: read('<div id="x"><rp style="display:block">abc</rp>def</div>'),
    renderedScript: read('<div id="x">a<script style="display:block">b</script>c</div>'),
    renderedStyle: read('<div id="x">a<style style="display:block">b</style>c</div>'),
    textarea: read('<div id="x">a<textarea>b</textarea>c</div>'),
    canvas: read('<div id="x">a<canvas>b</canvas>c</div>'),
    svgStop: read('<div id="x"><svg><stop>abc</stop></svg></div>')
  };
  return JSON.stringify(Object.fromEntries(
    Object.entries(cases).map(([name, index]) => [name, targets[index].innerText])
  ));
})()
"#,
        )
        .expect("Chromium-shaped structural innerText cases should evaluate");

    assert_eq!(
        result,
        r#"{"inline":"abc","nestedBlock":"a\nb\nc","cssBlock":"a\nb","paragraph":"a\n\nb\n\nc","adjacentParagraphs":"a\n\nb","emptyBlocks":"a\nb\nc","inlineBlock":"abc","emptyInlineBlock":"abc  def","spacedInlineBlock":"abc def ghi","tightInlineBlock":"123abcdef","imageLeadingSpace":" abc","imageTrailingSpace":"abc ","imageChild":"","inputChild":"","atomicBlockPair":"a\nb\n\n \nc\nd","atomicBlockPairNoBr":"a\nb\n \nc\nd","blockOfAtomicPairs":"a\nb\n\n \nc\nd\ne","blockWhitespace":"a\nb","atomicWhitespace":"a b","flex":"a\nb\nc","flexItems":"a\nb","gridItems":"a\nb","inlineFlexItems":"x\na\nb\ny","inlineGridItems":"x\na\nb\ny","normal":"a b c","pre":"  a \t b\n c  ","preWrap":"  a \t b\n c  ","preLine":"a b\nc","breakSpaces":"  a \t b\n c  ","inheritedPre":" a  b\n c  d ","overriddenNormal":" a  b c  d ","preElement":"  a \t b\n c  ","hr":"a\nb","rootBr":"","rubyRp":"abcdef","loneRp":"","renderedRp":"abc\ndef","renderedScript":"a\nb\nc","renderedStyle":"a\nb\nc","textarea":"ac","canvas":"ac","svgStop":""}"#
    );
}

#[test]
fn inner_text_matches_chromium_table_and_select_rules() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-table-select.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const targets = [];
  const read = (html) => {
    const host = document.createElement('div');
    host.innerHTML = html;
    document.body.append(host);
    targets.push(host.querySelector('#x'));
    return targets.length - 1;
  };
  const cases = {
    table: read('<table id="x"><tbody><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></tbody></table>'),
    hiddenCell: read('<table id="x"><tbody><tr><td>a</td><td style="display:none">x</td><td>b</td></tr></tbody></table>'),
    preservedWhitespace: read('<div id="x"><table style="white-space:pre">  <tbody>  <tr>  <td>a</td>  </tr>  </tbody>  </table></div>'),
    visibilityHiddenFirst: read('<table id="x"><tbody><tr><td style="visibility:hidden">x</td><td>b</td></tr></tbody></table>'),
    visibilityHiddenMiddle: read('<table id="x"><tbody><tr><td>a</td><td style="visibility:hidden">x</td><td>b</td></tr></tbody></table>'),
    visibilityHiddenLast: read('<table id="x"><tbody><tr><td>a</td><td style="visibility:hidden">x</td></tr></tbody></table>'),
    hiddenFirstRow: read('<table id="x"><tbody><tr style="visibility:hidden"><td>x</td></tr><tr><td>b</td></tr></tbody></table>'),
    hiddenMiddleRow: read('<table id="x"><tbody><tr><td>a</td></tr><tr style="visibility:hidden"><td>x</td></tr><tr><td>b</td></tr></tbody></table>'),
    hiddenLastRow: read('<table id="x"><tbody><tr><td>a</td></tr><tr style="visibility:hidden"><td>x</td></tr></tbody></table>'),
    cssTable: read('<div id="x"><div style="display:table"><span style="display:table-cell">a</span>\n<span style="display:table-cell">b</span></div></div>'),
    cssInlineTable: read('<div id="x"><div style="display:inline-table"><span style="display:table-cell">a</span>\n<span style="display:table-cell">b</span></div></div>'),
    rowRoot: read('<table><tbody><tr id="x"><td>a</td><td>b</td></tr><tr><td>c</td></tr></tbody></table>'),
    inlineTable: read('<div id="x">a<table style="display:inline-table"><tbody><tr><td>x</td><td>y</td></tr><tr><td>z</td></tr></tbody></table>b</div>'),
    select: read('<div id="x">a<select><option>one</option><option>two</option></select>b</div>'),
    option: read('<option id="x">  one <span> two </span> </option>'),
    optgroup: read('<div id="x">a<select><optgroup label="g"><option>one</option><option>two</option></optgroup></select>b</div>'),
    emptyOptgroup: read('<div id="x">a<select><optgroup label="g"></optgroup></select>b</div>'),
    outsideOptgroup: read('<div id="x">a<optgroup>ignored</optgroup>bc</div>'),
    outsideOption: read('<div id="x">a<option>one</option>bc</div>')
  };
  return JSON.stringify(Object.fromEntries(
    Object.entries(cases).map(([name, index]) => [name, targets[index].innerText])
  ));
})()
"#,
        )
        .expect("Chromium-shaped table/select innerText cases should evaluate");

    assert_eq!(
        result,
        r#"{"table":"a\tb\nc\td","hiddenCell":"a\tb","preservedWhitespace":"a","visibilityHiddenFirst":"b","visibilityHiddenMiddle":"a\tb","visibilityHiddenLast":"a\t","hiddenFirstRow":"b","hiddenMiddleRow":"a\nb","hiddenLastRow":"a\n","cssTable":"a\tb","cssInlineTable":"a\tb","rowRoot":"a\tb","inlineTable":"ax\ty\nzb","select":"a\none\ntwo\nb","option":"one two","optgroup":"a\none\ntwo\nb","emptyOptgroup":"a\nb","outsideOptgroup":"a\nignored\nbc","outsideOption":"a\none\nbc"}"#
    );
}

#[test]
fn inner_text_projects_closed_details_rendered_subtree() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-details.test/",
        r#"<!doctype html><html><body>
          <details id="target"><summary><span id="summary-child">first</span></summary><summary id="second">second</summary><div id="hidden-child">details</div></details>
          <details id="no-summary"><div>details</div></details>
          <details id="nested" open><summary>outer</summary><details><summary>inner</summary><div>hidden</div></details><div>tail</div></details>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const hidden = document.getElementById('hidden-child');
  const values = {
    closed: target.innerText,
    summaryChild: document.getElementById('summary-child').innerText,
    secondSummary: document.getElementById('second').innerText,
    hiddenChild: hidden.innerText,
    hiddenCheckVisibility: hidden.checkVisibility(),
    noSummary: document.getElementById('no-summary').innerText,
    nested: document.getElementById('nested').innerText
  };
  target.open = true;
  values.open = target.innerText;
  values.openHiddenChild = hidden.innerText;
  target.open = false;
  values.reclosed = target.innerText;
  return JSON.stringify(values);
})()
"#,
        )
        .expect("closed details rendered-subtree cases should evaluate");

    assert_eq!(
        result,
        r#"{"closed":"first","summaryChild":"first","secondSummary":"","hiddenChild":"","hiddenCheckVisibility":false,"noSummary":"","nested":"outer\ninner\ntail","open":"first\nsecond\ndetails","openHiddenChild":"details","reclosed":"first"}"#
    );
}

#[test]
fn check_visibility_and_inner_text_use_computed_rendered_state() {
    let mut vm = new_parsed_test_vm(
        "https://rendered-state.test/",
        r#"<!doctype html><html><head><style>
          .hidden { display: none; }
          .upper { text-transform: uppercase; }
          #visibility-hidden { visibility: hidden; }
          #visibility-child { visibility: visible; }
          #transparent { opacity: 0; }
          #contents { display: contents; }
          #under-content-hidden { display: none; }
          #outer-display-none { display: none; }
        </style></head><body>
          <div id="card"><span class="upper">visible</span><span class="hidden">leak</span></div>
          <div id="hidden-root" class="hidden">hidden root</div>
          <div id="visibility-hidden">hidden visibility<span id="visibility-child">visible child</span></div>
          <div id="transparent">transparent</div>
          <div id="contents"><span>contents</span></div>
          <div id="content-hidden" style="content-visibility: hidden !important; content-visibility: visible">content hidden</div>
          <div id="outer-content-hidden" style="content-visibility: hidden"><span id="content-hidden-child">hidden child</span><span id="under-content-hidden">under content hidden</span></div>
          <div id="outer-display-none"><span id="under-display-none" style="content-visibility: hidden">under display none</span></div>
          <div id="shadow-host"><span>assigned</span><span slot="missing">unassigned leak</span></div>
          <div id="dynamic">dynamic</div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const get = (id) => document.getElementById(id);
  const detached = new DOMParser().parseFromString(
    '<div id="x">\n  HTML  <span>Text</span>\n</div>',
    'text/html'
  ).getElementById('x');
  const detachedContentHidden = document.createElement('div');
  detachedContentHidden.setAttribute('style', 'content-visibility: hidden');
  detachedContentHidden.innerHTML = 'detached <span>content hidden</span>';
  const shadowHost = get('shadow-host');
  shadowHost.attachShadow({mode: 'open'}).innerHTML = 'shadow text<slot></slot>';
  const unassignedShadowChild = shadowHost.querySelector('[slot="missing"]');
  const dynamic = get('dynamic');
  const dynamicStates = [dynamic.checkVisibility()];
  dynamic.classList.add('hidden');
  dynamicStates.push(dynamic.checkVisibility());
  dynamic.classList.remove('hidden');
  dynamicStates.push(dynamic.checkVisibility());

  return JSON.stringify({
    shape: [typeof Element.prototype.checkVisibility, Element.prototype.checkVisibility.length],
    cardInnerText: get('card').innerText,
    hiddenRootInnerText: get('hidden-root').innerText,
    visibilityInnerText: get('visibility-hidden').innerText,
    detachedInnerText: detached.innerText,
    detachedContentHiddenInnerText: detachedContentHidden.innerText,
    contentHiddenInnerText: get('content-hidden').innerText,
    underContentHiddenInnerText: get('under-content-hidden').innerText,
    underDisplayNoneInnerText: get('under-display-none').innerText,
    shadowHostInnerText: shadowHost.innerText,
    checks: [
      get('card').checkVisibility(),
      get('hidden-root').checkVisibility(),
      get('visibility-hidden').checkVisibility(),
      get('visibility-hidden').checkVisibility({checkVisibilityCSS: true}),
      get('visibility-child').checkVisibility({visibilityProperty: true}),
      get('transparent').checkVisibility(),
      get('transparent').checkVisibility({checkOpacity: true}),
      get('contents').checkVisibility(),
      get('content-hidden').checkVisibility(),
      get('content-hidden-child').checkVisibility(),
      detachedContentHidden.checkVisibility(),
      unassignedShadowChild.checkVisibility()
    ],
    dynamicStates
  });
})()
"#,
        )
        .expect("computed rendered-state surfaces should evaluate");

    assert_eq!(
        result,
        r#"{"shape":["function",0],"cardInnerText":"VISIBLE","hiddenRootInnerText":"hidden root","visibilityInnerText":"visible child","detachedInnerText":"\n  HTML  Text\n","detachedContentHiddenInnerText":"detached content hidden","contentHiddenInnerText":"","underContentHiddenInnerText":"","underDisplayNoneInnerText":"under display none","shadowHostInnerText":"assigned","checks":[true,false,true,false,true,true,false,false,true,false,false,false],"dynamicStates":[true,false,true]}"#
    );
}

#[test]
fn content_visibility_only_locks_chromium_eligible_boxes() {
    let mut vm = new_parsed_test_vm(
        "https://content-visibility-applicability.test/",
        r#"<!doctype html><html><body>
          <span id="inline" style="content-visibility: hidden">inline visible</span>
          <span id="atomic" style="display: inline-block; content-visibility: hidden">atomic hidden</span>
          <div id="block" style="content-visibility: hidden">block hidden</div>
          <table id="table" style="content-visibility: hidden"><tbody><tr><td>table visible</td></tr></tbody></table>
          <table><tbody><tr id="row" style="content-visibility: hidden"><td>row visible</td></tr></tbody></table>
          <table><tbody><tr><td id="cell" style="content-visibility: hidden">cell hidden</td></tr></tbody></table>
          <table><caption id="caption" style="content-visibility: hidden">caption visible</caption><tbody><tr><td>caption body</td></tr></tbody></table>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const text = (id) => document.getElementById(id).innerText;
  return JSON.stringify({
    inline: text('inline'),
    atomic: text('atomic'),
    block: text('block'),
    table: text('table'),
    row: text('row'),
    cell: text('cell'),
    caption: text('caption')
  });
})()
"#,
        )
        .expect("content-visibility applicability cases should evaluate");

    assert_eq!(
        result,
        r#"{"inline":"inline visible","atomic":"","block":"","table":"table visible","row":"row visible","cell":"","caption":"caption visible"}"#
    );
}

#[test]
fn rendered_style_facts_refresh_after_synchronous_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://rendered-style-mutation.test/",
        r#"<!doctype html><html><head><style>
          .hidden { display: none; }
          .upper { text-transform: uppercase; }
          .transparent { opacity: 0; }
          .invisible { visibility: hidden; }
        </style></head><body>
          <div id="target"><span>fresh value</span></div>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const values = [target.innerText, target.checkVisibility()];
  target.className = 'upper';
  values.push(target.innerText);
  target.className = 'transparent';
  values.push(target.checkVisibility({opacityProperty: true}));
  target.className = 'invisible';
  values.push(target.innerText, target.checkVisibility({visibilityProperty: true}));
  target.className = 'hidden';
  values.push(target.innerText, target.checkVisibility());
  target.className = '';
  target.hidden = true;
  values.push(target.innerText, target.checkVisibility());
  target.hidden = false;
  target.setAttribute('style', 'content-visibility: hidden');
  values.push(target.innerText, target.checkVisibility());
  target.setAttribute('style', 'text-transform: lowercase');
  values.push(target.innerText, target.checkVisibility());
  return JSON.stringify(values);
})()
"#,
        )
        .expect("rendered style reads should observe every synchronous mutation");

    assert_eq!(
        result,
        r#"["fresh value",true,"FRESH VALUE",false,"",false,"fresh value",false,"fresh value",false,"",true,"fresh value",true]"#
    );
}

#[test]
fn inner_text_reuses_prepared_style_inputs_within_each_synchronous_read() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-prepared-style-inputs.test/",
        r#"<!doctype html><html><head><style>
          .upper { text-transform: uppercase; }
        </style></head><body><div id="target"></div></body></html>"#,
    );
    vm.eval(
        r#"
const target = document.getElementById('target');
for (let index = 0; index < 128; index++) {
  const child = document.createElement('span');
  child.textContent = 'a';
  target.appendChild(child);
}
"#,
    )
    .expect("innerText prepared-input fixture should initialize");

    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_before = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_before = vm.layout_pass_observability_for_test();
    let first = vm
        .eval(
            "(() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("first innerText read should evaluate");
    let builds_after_first = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_first = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_after_first = vm.layout_pass_observability_for_test();

    let repeated = vm
        .eval(
            "(() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("repeated innerText read should evaluate");
    let builds_after_repeated = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_repeated = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_after_repeated = vm.layout_pass_observability_for_test();

    let second = vm
        .eval(
            "target.className = 'upper'; (() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("mutated innerText read should evaluate");
    let builds_after_second = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_second = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_after_second = vm.layout_pass_observability_for_test();

    let stylesheet_mutation = vm
        .eval(
            "document.querySelector('style').textContent = '.upper { text-transform: lowercase; }'; (() => { const text = target.innerText; return [text.length, text[0], text[127]].join('|'); })()",
        )
        .expect("stylesheet-mutated innerText read should evaluate");
    let builds_after_stylesheet_mutation = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_stylesheet_mutation = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_after_stylesheet_mutation = vm.layout_pass_observability_for_test();

    assert_eq!(first, "128|a|a");
    assert_eq!(repeated, "128|a|a");
    assert_eq!(second, "128|A|A");
    assert_eq!(stylesheet_mutation, "128|a|a");
    assert!(
        !layout_before.0
            && !layout_after_first.0
            && !layout_after_repeated.0
            && !layout_after_second.0
            && !layout_after_stylesheet_mutation.0
    );
    assert_eq!(layout_after_first.1, layout_before.1 + 1);
    assert_eq!(layout_after_repeated.1, layout_after_first.1);
    assert_eq!(layout_after_second.1, layout_after_first.1);
    assert_eq!(layout_after_stylesheet_mutation.1, layout_after_first.1);
    assert_eq!(
        builds_after_first.saturating_sub(builds_before),
        1,
        "one innerText traversal should prepare one document input"
    );
    assert_eq!(
        builds_after_repeated.saturating_sub(builds_after_first),
        0,
        "an unchanged generation should reuse its document input across getters"
    );
    assert_eq!(
        builds_after_second.saturating_sub(builds_after_repeated),
        1,
        "a later innerText read must prepare one fresh input after mutation"
    );
    assert_eq!(
        builds_after_stylesheet_mutation.saturating_sub(builds_after_second),
        1,
        "a stylesheet mutation must invalidate the cross-getter document input"
    );
    assert_eq!(
        key_builds_after_first.saturating_sub(key_builds_before),
        1,
        "one traversal must hash one retained-system key, not one key per descendant"
    );
    assert_eq!(
        key_builds_after_repeated.saturating_sub(key_builds_after_first),
        0,
        "an unchanged generation should reuse the prepared retained-system key"
    );
    assert_eq!(
        key_builds_after_second.saturating_sub(key_builds_after_repeated),
        1,
        "a style mutation must prepare one fresh retained-system key"
    );
    assert_eq!(
        key_builds_after_stylesheet_mutation.saturating_sub(key_builds_after_second),
        1,
        "a stylesheet mutation must prepare one fresh retained-system key"
    );
}

#[test]
fn inner_text_new_sources_wait_for_a_fresh_paint_layout() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-latest-layout.test/",
        "<!doctype html><html><body><div id=target><span>a</span></div></body></html>",
    );
    let passes_before = vm.layout_pass_observability_for_test().1;
    let cache_before = vm.layout_snapshot_cache_observability_for_test();

    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("the cold innerText read should evaluate"),
        "a"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 1);

    assert_eq!(
        vm.eval(
            "const added = document.createElement('span'); added.textContent = 'b'; target.append(added); target.innerText",
        )
        .expect("the warm innerText read should evaluate"),
        "a",
        "a text source absent from the latest frozen layout tree remains unrendered until refresh"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 1);

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))
        .expect("fresh paint layout should succeed")
        .expect("the fixture should have a layout root");
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 2);
    assert_eq!(
        vm.eval("target.innerText")
            .expect("innerText should read the refreshed geometry snapshot"),
        "ab"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, passes_before + 2);

    let cache_after = vm.layout_snapshot_cache_observability_for_test();
    assert_eq!(cache_after.0, cache_before.0 + 2);
    assert_eq!(cache_after.1, cache_before.1 + 1);
    assert_eq!(cache_after.2, cache_before.2 + 2);
}

#[test]
fn inner_text_document_input_cache_tracks_emulated_media_changes() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-emulated-media-inputs.test/",
        r#"<!doctype html><html><head><style>
          @media print { #target { text-transform: uppercase; } }
        </style></head><body><div id="target">mixed</div></body></html>"#,
    );

    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_before = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("screen innerText read should evaluate"),
        "mixed"
    );
    let builds_after_screen = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_screen = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("print innerText read should evaluate"),
        "MIXED"
    );
    let builds_after_print = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_print = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("restored screen innerText read should evaluate"),
        "mixed"
    );
    let builds_after_restore = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_restore = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    assert_eq!(builds_after_screen.saturating_sub(builds_before), 1);
    assert_eq!(builds_after_print.saturating_sub(builds_after_screen), 1);
    assert_eq!(builds_after_restore.saturating_sub(builds_after_print), 1);
    assert_eq!(key_builds_after_screen.saturating_sub(key_builds_before), 1);
    assert_eq!(
        key_builds_after_print.saturating_sub(key_builds_after_screen),
        1
    );
    assert_eq!(
        key_builds_after_restore.saturating_sub(key_builds_after_print),
        1
    );
}

#[test]
fn inner_text_document_input_cache_tracks_document_url_changes() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-document-url-inputs.test/start/index.html",
        r#"<!doctype html><html><head>
          <base href="https://cdn.test/stable-base/">
          <style>#target { text-transform: uppercase; }</style>
        </head><body><div id="target">mixed</div></body></html>"#,
    );

    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("initial innerText read should evaluate"),
        "MIXED"
    );
    let builds_after_initial = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_initial = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    assert_eq!(
        vm.eval(
            "history.pushState(null, '', '/next/path'); document.getElementById('target').innerText"
        )
        .expect("innerText after same-document URL mutation should evaluate"),
        "MIXED"
    );
    let builds_after_url_change = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_url_change = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    assert_eq!(
        vm.eval("document.getElementById('target').innerText")
            .expect("repeated innerText after URL mutation should evaluate"),
        "MIXED"
    );
    let builds_after_repeated = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_repeated = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    assert_eq!(
        builds_after_url_change.saturating_sub(builds_after_initial),
        1
    );
    assert_eq!(
        key_builds_after_url_change.saturating_sub(key_builds_after_initial),
        1,
        "a path-changing history mutation must not reuse a prepared key for the old document URL"
    );
    assert_eq!(
        builds_after_repeated.saturating_sub(builds_after_url_change),
        0
    );
    assert_eq!(
        key_builds_after_repeated.saturating_sub(key_builds_after_url_change),
        0
    );
}

#[test]
fn inner_text_reuses_one_document_style_world_across_shadow_scopes() {
    let mut vm = new_parsed_test_vm(
        "https://inner-text-shadow-style-inputs.test/",
        r#"<!doctype html><html><body>
          <div id="host"><span>assigned</span><span slot="missing">hidden</span></div>
        </body></html>"#,
    );
    vm.eval(
        r#"
const host = document.getElementById('host');
host.attachShadow({mode: 'open'}).innerHTML =
  '<style>::slotted(span) { text-transform: uppercase; }</style><slot></slot>';
"#,
    )
    .expect("shadow innerText prepared-input fixture should initialize");
    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_before = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_before = vm.layout_pass_observability_for_test();

    let first_text = vm
        .eval("host.innerText")
        .expect("shadow innerText read should evaluate");
    let builds_after_first = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_first = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_after_first = vm.layout_pass_observability_for_test();
    let second_text = vm
        .eval("host.innerText")
        .expect("repeated shadow innerText read should evaluate");
    let builds_after_second = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_second = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();
    let layout_after_second = vm.layout_pass_observability_for_test();

    assert_eq!(first_text, "ASSIGNED");
    assert_eq!(second_text, "ASSIGNED");
    assert!(!layout_before.0 && !layout_after_first.0 && !layout_after_second.0);
    assert_eq!(layout_after_first.1, layout_before.1 + 1);
    assert_eq!(layout_after_second.1, layout_after_first.1);
    assert_eq!(
        builds_after_first.saturating_sub(builds_before),
        1,
        "layout and rendered-text collection must share one document TreeScope universe"
    );
    assert_eq!(
        builds_after_second.saturating_sub(builds_after_first),
        0,
        "an unchanged document TreeScope universe must reuse its prepared input"
    );
    assert_eq!(
        key_builds_after_first.saturating_sub(key_builds_before),
        1,
        "one observation must build one retained-system key for the document"
    );
    assert_eq!(
        key_builds_after_second.saturating_sub(key_builds_after_first),
        0,
        "the rendered-text collector must reuse the unchanged document key"
    );
}

#[test]
fn paint_layout_reuses_one_style_world_across_many_empty_shadow_roots() {
    let mut vm = new_parsed_test_vm(
        "https://paint-empty-shadow-style-world.test/",
        r#"<!doctype html><html><head><style>
          body { color: rgb(1, 2, 3); }
        </style></head><body><main id="light">light</main></body></html>"#,
    );
    vm.eval(
        r#"
const body = document.body;
for (let index = 0; index < 64; index += 1) {
  const host = document.createElement('section');
  body.appendChild(host);
  const shadow = host.attachShadow({mode: index % 2 === 0 ? 'open' : 'closed'});
  const target = document.createElement('span');
  target.textContent = `shadow-${index}`;
  shadow.appendChild(target);
  if (index % 8 === 0) {
    const nestedHost = document.createElement('article');
    shadow.appendChild(nestedHost);
    nestedHost.attachShadow({mode: 'open'}).append(`nested-${index}`);
  }
}
"#,
    )
    .expect("empty shadow-root paint fixture should initialize");

    let document = vm.document_handle_for_test();
    let rebuilds_before = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_before = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("first empty shadow-root paint layout should succeed")
        .expect("the fixture should have a layout root");
    let rebuilds_after_first =
        vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let builds_after_first = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let key_builds_after_first = vm
        ._context_host
        .borrow()
        .stylo_style_system_key_builds_for_test();

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("second empty shadow-root paint layout should succeed")
        .expect("the fixture should have a layout root");

    assert_eq!(
        rebuilds_after_first.saturating_sub(rebuilds_before),
        1,
        "entering and leaving empty shadow roots must not replace the retained style system",
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds_after_first,
        "an unchanged fresh paint layout must retain the same document style system",
    );
    assert_eq!(builds_after_first.saturating_sub(builds_before), 1);
    assert_eq!(key_builds_after_first.saturating_sub(key_builds_before), 1);
    assert_eq!(
        vm._context_host
            .borrow()
            .stylo_computed_style_input_builds_for_test(),
        builds_after_first,
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .stylo_style_system_key_builds_for_test(),
        key_builds_after_first,
    );
}

#[test]
fn document_import_node_clones_dom_parser_svg_snapshot() {
    let mut vm = new_parsed_test_vm(
        "https://dom-parser-svg-import.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
(() => {
  const parsed = new DOMParser().parseFromString(
    "<symbol xmlns='http://www.w3.org/2000/svg' id='icon' viewBox='0 0 1 1'><path d='M0 0h1v1'/></symbol>",
    "image/svg+xml"
  );
  const imported = document.importNode(parsed.documentElement, true);
  document.body.appendChild(imported);
  return [
    imported.namespaceURI,
    imported.localName,
    imported.getAttribute("id"),
    imported.firstChild && imported.firstChild.localName,
    document.querySelector("#icon path").getAttribute("d")
  ].join("|");
})()
"##,
        )
        .expect("DOMParser SVG snapshot import should evaluate");

    assert_eq!(
        result,
        "http://www.w3.org/2000/svg|symbol|icon|path|M0 0h1v1"
    );
}
#[test]
fn dom_parser_parse_from_string_parses_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://dom-parser-webidl-args.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function probe(callback) {
    try {
      const value = callback();
      return value && value.documentElement
        ? value.documentElement.tagName
        : String(value);
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  const parser = new DOMParser();
  const sourceObject = {
    toString() {
      return '<html><body><section id="from-object"></section></body></html>';
    }
  };
  return JSON.stringify({
    html: parser.parseFromString('<html><body><p></p></body></html>', 'text/html').documentElement.tagName,
    xml: parser.parseFromString('<root></root>', 'application/xml').documentElement.tagName,
    objectSource: parser.parseFromString(sourceObject, 'text/html').getElementById('from-object').tagName,
    nullSource: parser.parseFromString(null, 'text/html').body.textContent,
    missingSource: probe(() => parser.parseFromString()),
    missingType: probe(() => parser.parseFromString('<root></root>')),
    invalidType: probe(() => parser.parseFromString('<root></root>', 'TEXT/html')),
    symbolSource: probe(() => parser.parseFromString(Symbol(), 'text/html')),
    symbolType: probe(() => parser.parseFromString('<root></root>', Symbol())),
    throwingSource: probe(() => parser.parseFromString({
      toString() { throw new Error('source failed'); }
    }, 'text/html'))
  });
})()
"#,
        )
        .expect("DOMParser.parseFromString WebIDL argument probe should run");

    assert_eq!(
        result,
        r#"{"html":"HTML","xml":"root","objectSource":"SECTION","nullSource":"null","missingSource":"throw:TypeError","missingType":"throw:TypeError","invalidType":"throw:TypeError","symbolSource":"throw:TypeError","symbolType":"throw:TypeError","throwingSource":"throw:Error"}"#
    );
}

#[test]
fn dom_parser_prototype_parse_from_string_is_declared_operation() {
    let mut vm = new_storage_test_vm("https://dom-parser-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parser = new DOMParser();
  const descriptor = Object.getOwnPropertyDescriptor(DOMParser.prototype, 'parseFromString');
  const html = parser.parseFromString(
    '<html><body><main id="root"></main></body></html>',
    'text/html'
  );
  const xml = DOMParser.prototype.parseFromString.call(
    parser,
    '<root><child /></root>',
    'application/xml'
  );
  return JSON.stringify({
    descriptor: [
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(':'),
    own: Object.hasOwn(parser, 'parseFromString'),
    enumerable: Object.keys(DOMParser.prototype).join(','),
    behavior: [
      html.getElementById('root').tagName,
      xml.documentElement.tagName,
      parser instanceof DOMParser
    ].join(':')
  });
})()
"#,
        )
        .expect("DOMParser prototype method descriptor probe should run");

    assert_eq!(
        result,
        r#"{"descriptor":"function:parseFromString:2:true:true:true","own":false,"enumerable":"parseFromString","behavior":"MAIN:root:true"}"#
    );
}

#[test]
fn dom_parser_xml_text_content_excludes_processing_instruction_descendants() {
    let mut vm = new_storage_test_vm("https://dom-parser-xml-text-content.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    "<root><?start name=p?>Unchanged<!--ignored--><![CDATA[ kept ]]></root>",
    "application/xml"
  );
  const root = doc.documentElement;
  return JSON.stringify({
    rootText: root.textContent,
    piText: root.firstChild.textContent,
    commentText: root.childNodes[2].textContent
  });
})()
"#,
        )
        .expect("DOMParser XML textContent processing instruction probe should run");

    assert_eq!(
        result,
        r#"{"rootText":"Unchanged kept ","piText":"name=p","commentText":"ignored"}"#
    );
}
#[test]
fn dom_parser_inner_html_supports_html_sanitizer_roundtrip_path() {
    let mut vm = new_storage_test_vm("https://dom-parser-sanitizer-path.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function sanitizeLikePage(html) {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    return doc.body.innerHTML;
  }
  return [
    sanitizeLikePage('Plain title'),
    sanitizeLikePage('<p>A</p><p><strong>B</strong></p>'),
    sanitizeLikePage('<img src="x.png" img_width="10"><p>caption</p>'),
  ].join('|');
})()
"#,
        )
        .expect("dom parser body.innerHTML should support sanitizer roundtrips");

    assert_eq!(
        result,
        "Plain title|<p>A</p><p><strong>B</strong></p>|<img src=\"x.png\" img_width=\"10\"><p>caption</p>"
    );
}
#[test]
fn dom_parser_body_and_elements_expose_class_list_for_sanitizer_walks() {
    let mut vm = new_storage_test_vm("https://dom-parser-class-list.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body class="article-body"><pre class="highlight language-js">const x = 1;</pre></body></html>',
    'text/html'
  );
  const body = doc.body;
  const pre = body.querySelector('pre');
  return JSON.stringify({
    bodyTag: Object.prototype.toString.call(body.classList),
    bodyContains: body.classList.contains('article-body'),
    preContainsHighlight: pre.classList.contains('highlight'),
    preContainsLanguage: pre.classList.contains('language-js'),
    stable: body.classList === body.classList,
    preStable: pre.classList === pre.classList,
    item0: pre.classList.item(0),
    item1: pre.classList.item(1),
    length: pre.classList.length
  });
})()
"#,
        )
        .expect("DOMParser detached body.classList should stay available for sanitizer walks");

    assert_eq!(
        result,
        r#"{"bodyTag":"[object DOMTokenList]","bodyContains":true,"preContainsHighlight":true,"preContainsLanguage":true,"stable":true,"preStable":true,"item0":"highlight","item1":"language-js","length":2}"#
    );
}
#[test]
fn dom_parser_elements_expose_dataset_has_attribute_and_element_traversal() {
    let mut vm = new_storage_test_vm("https://dom-parser-dataset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body data-root="article">' +
      'text<!--gap-->' +
      '<pre id="code" data-eeimg="1" data-tex="inline"></pre>' +
      '<figure id="target"></figure>' +
      '<span id="tail"></span>' +
    '</body></html>',
    'text/html'
  );
  const body = doc.body;
  const pre = doc.getElementById('code');
  const figure = doc.getElementById('target');
  const dataset = pre.dataset;
  dataset.imageSrc = 'https://img.test/a.png';
  dataset.imageStatus = 'ready';
  return JSON.stringify({
    bodyHasRoot: body.hasAttribute('data-root'),
    bodyMissing: body.hasAttribute('data-missing'),
    datasetTag: Object.prototype.toString.call(dataset),
    stable: dataset === pre.dataset,
    eeimg: dataset.eeimg,
    tex: dataset.tex,
    imageSrc: dataset.imageSrc,
    attrImageSrc: pre.getAttribute('data-image-src'),
    keys: Object.keys(dataset).sort(),
    firstElementChild: body.firstElementChild && body.firstElementChild.id,
    lastElementChild: body.lastElementChild && body.lastElementChild.id,
    childElementCount: body.childElementCount,
    preNext: pre.nextElementSibling && pre.nextElementSibling.id,
    figurePrev: figure.previousElementSibling && figure.previousElementSibling.id,
    tailPrev: doc.getElementById('tail').previousElementSibling && doc.getElementById('tail').previousElementSibling.id
  });
})()
"#,
        )
        .expect("DOMParser detached elements should expose dataset and element traversal");

    assert_eq!(
        result,
        r#"{"bodyHasRoot":true,"bodyMissing":false,"datasetTag":"[object DOMStringMap]","stable":true,"eeimg":"1","tex":"inline","imageSrc":"https://img.test/a.png","attrImageSrc":"https://img.test/a.png","keys":["eeimg","imageSrc","imageStatus","tex"],"firstElementChild":"code","lastElementChild":"tail","childElementCount":3,"preNext":"target","figurePrev":"code","tailPrev":"target"}"#
    );
}
#[test]
fn child_shadow_root_legacy_wpt_attributes_and_methods() {
    let mut vm = new_storage_test_vm("https://child-shadow-root-legacy-wpt.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentWindow.document;
  const host = doc.createElement('div');
  doc.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const input = doc.createElement('input');
  shadow.appendChild(input);
  input.focus();
  const activeTag = shadow.activeElement && shadow.activeElement.tagName;

  const span = doc.createElement('span');
  span.innerHTML = 'Some text';
  shadow.appendChild(span);
  const innerBefore = shadow.innerHTML.toLowerCase();
  shadow.innerHTML = '<input type="text" id="inputId"><div id="divId">new text</div>';
  const innerAfter = shadow.innerHTML.toLowerCase();

  const styleHost = doc.createElement('div');
  doc.body.appendChild(styleHost);
  const styleRoot = styleHost.attachShadow({ mode: 'open' });
  const emptyStyleLength = styleRoot.styleSheets.length;
  styleRoot.appendChild(doc.createElement('style'));
  const styleLength = styleRoot.styleSheets.length;

  const selectionHost = doc.createElement('div');
  doc.body.appendChild(selectionHost);
  const selectionRoot = selectionHost.attachShadow({ mode: 'open' });
  const selected = doc.createElement('span');
  selected.innerHTML = 'Some text';
  selectionRoot.appendChild(selected);
  const range = doc.createRange();
  range.setStart(selected.firstChild, 0);
  range.setEnd(selected.firstChild, 3);
  const selection = selectionRoot.getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
  const selectedText = selectionRoot.getSelection().toString();

  let cloneError = '';
  try {
    selectionRoot.cloneNode();
  } catch (error) {
    cloneError = error.name + ':' + error.code;
  }

  return [
    activeTag,
    innerBefore,
    innerAfter,
    shadow.querySelector('#inputId') && shadow.querySelector('#inputId').id,
    emptyStyleLength,
    styleLength,
    selectedText,
    cloneError
  ].join('|');
})()
"#,
        )
        .expect("child ShadowRoot legacy WPT surface should evaluate");

    assert_eq!(
        result,
        "INPUT|<input><span>some text</span>|<input type=\"text\" id=\"inputid\"><div id=\"divid\">new text</div>|inputId|0|1|Som|NotSupportedError:9"
    );
}
