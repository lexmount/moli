use super::*;

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
fn insert_adjacent_html_enforces_sibling_context_rules() {
    let mut vm = new_storage_test_vm("https://insert-adjacent-sibling-context.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'missing';
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  };
  const sources = ['', 'text', '<!--comment-->', '<div></div>'];
  const positions = ['beforebegin', 'afterend'];
  const detached = document.createElement('div');
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const detachedErrors = positions.flatMap(position =>
    sources.map(source => probe(() => detached.insertAdjacentHTML(position, source)))
  );
  const documentErrors = positions.flatMap(position =>
    sources.map(source => probe(() => root.insertAdjacentHTML(position, source)))
  );

  while (root.firstChild) {
    root.removeChild(root.firstChild);
  }
  root.insertAdjacentHTML(
    'afterbegin',
    '<head id="inside-head"></head><body id="inside-body"></body>'
  );
  const preservedInnerHtmlContext =
    document.head?.id === 'inside-head' &&
    document.body?.id === 'inside-body' &&
    root.firstChild === document.head &&
    root.lastChild === document.body;
  const head = document.head ||
    root.insertBefore(document.createElement('head'), root.firstChild);
  const body = document.body || root.appendChild(document.createElement('body'));
  head.insertAdjacentHTML('beforebegin', '<p id="before-head"></p>');
  body.insertAdjacentHTML('afterend', '<p id="after-body"></p>');
  const beforeHead = document.getElementById('before-head');
  const afterBody = document.getElementById('after-body');

  return JSON.stringify({
    detachedErrors,
    documentErrors,
    preservedInnerHtmlContext,
    counts: [
      document.getElementsByTagName('html').length,
      document.getElementsByTagName('head').length,
      document.getElementsByTagName('body').length
    ],
    placement: [
      beforeHead.nextSibling === head,
      body.nextSibling === afterBody,
      beforeHead.parentNode === root,
      afterBody.parentNode === root
    ]
  });
})()
"#,
        )
        .expect("insertAdjacentHTML sibling context rules should evaluate");

    assert_eq!(
        result,
        r#"{"detachedErrors":["NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7"],"documentErrors":["NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7","NoModificationAllowedError:7"],"preservedInnerHtmlContext":true,"counts":[1,1,1],"placement":[true,true,true,true]}"#
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
fn markup_setters_apply_legacy_null_to_empty_string_conversion() {
    let mut vm = new_storage_test_vm("https://markup-null-string-conversion.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const inner = document.createElement('div');
  inner.innerHTML = '<span>old</span>';
  inner.innerHTML = null;

  const outerParent = document.createElement('div');
  const outer = outerParent.appendChild(document.createElement('span'));
  outer.outerHTML = null;

  const undefinedValue = document.createElement('div');
  undefinedValue.innerHTML = undefined;

  const objectValue = document.createElement('div');
  objectValue.innerHTML = { toString() { return 'converted'; } };

  const shadow = document.createElement('div').attachShadow({ mode: 'open' });
  shadow.innerHTML = '<span>old</span>';
  shadow.innerHTML = null;

  let symbolError = 'missing';
  try {
    inner.innerHTML = Symbol('markup');
  } catch (error) {
    symbolError = error.name;
  }

  return JSON.stringify([
    inner.innerHTML,
    inner.textContent,
    outerParent.innerHTML,
    undefinedValue.innerHTML,
    objectValue.innerHTML,
    shadow.innerHTML,
    symbolError
  ]);
})()
"#,
        )
        .expect("markup WebIDL string conversion should evaluate");

    assert_eq!(
        result,
        r#"["","","","undefined","converted","","TypeError"]"#
    );
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
fn child_document_base_uri_tracks_connected_base_href_mutations() {
    let mut vm = new_storage_test_vm("https://child-base-uri.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  const base = childDocument.body.appendChild(childDocument.createElement('base'));
  const before = childDocument.baseURI;
  base.href = 'sub/';
  const image = childDocument.createElement('img');
  image.src = 'asset.png';
  return JSON.stringify({
    before,
    after: childDocument.baseURI,
    image: image.src,
    ownsBaseUri: Object.prototype.hasOwnProperty.call(childDocument, 'baseURI')
  });
})()
"#,
        )
        .expect("child document base URL should remain live");

    assert_eq!(
        result,
        r#"{"before":"https://child-base-uri.test/path/page.html","after":"https://child-base-uri.test/path/sub/","image":"https://child-base-uri.test/path/sub/asset.png","ownsBaseUri":false}"#
    );
}
